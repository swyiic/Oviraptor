enum WorkbenchStartRequest {
    New(Box<WorkbenchScanInput>),
    Retry(String),
}

fn build_workbench_investigation_policy(
    mode: &str, budget: Option<f64>, identities: Vec<String>, skills: &[i64], instruction: &str,
) -> Result<JsonValue,String> {
    let mut policy=build_web_investigation_policy(Some(mode),budget,identities,skills,instruction,"workbench",None)?;
    // Workbench can inherit more than 32 configured skills. Persist the actual
    // selection, not the Web UI selection cap; retries must never lose IDs.
    policy["selectedSkillIds"]=json!(skills);
    Ok(policy)
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkbenchRetryBasis {
    attempt: i64,
    status: String,
    task_path: String,
    policy_digest: String,
    task_digest: String,
    skills_digest: String,
}

fn workbench_retry_digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}",Sha256::digest(bytes))
}

fn read_workbench_retry_task(path: &Path) -> Result<Vec<u8>,String> {
    // Do not let a corrupt task path block on a FIFO or follow a replaced file.
    let mut options=OpenOptions::new();
    options.read(true);
    #[cfg(unix)] {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    let mut file=options.open(path).map_err(|_| "workbench_retry_task_unavailable")?;
    let metadata=file.metadata().map_err(|_| "workbench_retry_task_unavailable")?;
    if !metadata.is_file() || metadata.len()>16*1024*1024 {
        return Err("workbench_retry_task_unsafe_or_oversized".into());
    }
    let mut bytes=Vec::new();
    std::io::Read::take(&mut file,16*1024*1024+1).read_to_end(&mut bytes)
        .map_err(|_| "workbench_retry_task_unavailable")?;
    if bytes.len()>16*1024*1024 { return Err("workbench_retry_task_unsafe_or_oversized".into()); }
    Ok(bytes)
}

fn workbench_selected_skill_ids(
    connection: &rusqlite::Connection, ids: &[i64], inherit_enabled: bool,
) -> Result<Vec<i64>,String> {
    let mut selected=if ids.is_empty() && inherit_enabled {
        let mut statement=connection.prepare("SELECT id FROM agent_skills WHERE enabled=1 ORDER BY id").map_err(|e|e.to_string())?;
        let rows=statement.query_map([],|row|row.get::<_,i64>(0)).map_err(|e|e.to_string())?
            .collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
        rows
    } else { ids.to_vec() };
    if selected.iter().any(|id|*id<=0) { return Err("workbench_skill_selection_invalid".into()); }
    selected.sort_unstable();
    selected.dedup();
    for id in &selected {
        let enabled:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM agent_skills WHERE id=?1 AND enabled=1)",[id],|row|row.get(0))
            .map_err(|_| "workbench_skill_selection_unavailable")?;
        if !enabled { return Err("workbench_selected_skill_missing_or_disabled".into()); }
    }
    Ok(selected)
}

fn workbench_policy_skill_ids(policy: &JsonValue) -> Result<Vec<i64>,String> {
    let ids:Vec<i64>=serde_json::from_value(policy.get("selectedSkillIds").cloned().ok_or("workbench_retry_policy_incomplete")?)
        .map_err(|_| "workbench_retry_policy_invalid")?;
    let mut normalized=ids.clone();
    normalized.sort_unstable(); normalized.dedup();
    if ids!=normalized || ids.iter().any(|id|*id<=0) { return Err("workbench_retry_policy_invalid".into()); }
    Ok(ids)
}

fn capture_workbench_retry_basis(connection: &rusqlite::Connection,scan_id: &str) -> Result<WorkbenchRetryBasis,String> {
    let (attempt,status,path,policy):(i64,String,String,String)=connection.query_row(
        "SELECT s.attempt_count,s.status,s.task_path,c.policy_json FROM sentinel_scans s JOIN sentinel_scan_contexts c ON c.scan_id=s.id
         WHERE s.id=?1 AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=s.id)",
        [scan_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
    ).map_err(|_| "workbench_retry_scope_unavailable")?;
    let parsed:JsonValue=serde_json::from_str(&policy).map_err(|_| "workbench_retry_policy_invalid")?;
    let ids=workbench_policy_skill_ids(&parsed)?;
    workbench_selected_skill_ids(connection,&ids,false)?;
    let mut skills=Vec::new();
    for id in &ids {
        let entry:(String,String)=connection.query_row("SELECT name,instructions FROM agent_skills WHERE id=?1 AND enabled=1",[id],|row|Ok((row.get(0)?,row.get(1)?)))
            .map_err(|_| "workbench_selected_skill_missing_or_disabled")?;
        skills.push((id,entry));
    }
    Ok(WorkbenchRetryBasis {attempt,status,task_path:path.clone(),policy_digest:workbench_retry_digest(policy.as_bytes()),
        task_digest:workbench_retry_digest(&read_workbench_retry_task(Path::new(&path))?),
        skills_digest:workbench_retry_digest(&serde_json::to_vec(&skills).map_err(|_| "workbench_retry_policy_invalid")?)})
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkbenchSavedTask {
    scan_id:String, project_id:i64, task_name:String, scan_type:String, attempt:i64,
    urls:Vec<String>, source_path:String, scan_mode:String, scope_mode:String, diff_base:String,
    max_budget_usd:Option<f64>, environment:String, auth_session_id:String, auth_session_ids:Vec<String>,
    ci_provider:String, repository_url:String, branch:String, commit_sha:String, build_id:String,
    runtime_policy:JsonValue, policy:JsonValue,
}

fn load_workbench_retry_input(connection: &rusqlite::Connection,scan_id: &str) -> Result<(WorkbenchScanInput,WorkbenchRetryBasis),String> {
    let tx=rusqlite::Transaction::new_unchecked(connection,rusqlite::TransactionBehavior::Deferred).map_err(|_| "workbench_retry_snapshot_unavailable")?;
    let basis=capture_workbench_retry_basis(&tx,scan_id)?;
    if !matches!(basis.status.as_str(),"paused"|"partial"|"completed"|"completed_with_gaps"|"failed"|"cancelled"|"protected_stop"|"interrupted") {
        return Err("workbench_retry_state_ineligible".into());
    }
    let (project,kind,source,name,work,context):(i64,String,String,String,String,String)=tx.query_row(
        "SELECT s.project_id,s.scan_type,s.source_path,s.task_name,a.work_dir,c.policy_json FROM sentinel_scans s
         JOIN sentinel_scan_attempts a ON a.scan_id=s.id AND a.attempt_number=s.attempt_count
         JOIN sentinel_scan_contexts c ON c.scan_id=s.id WHERE s.id=?1",[scan_id],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)),
    ).map_err(|_| "workbench_retry_attempt_unavailable")?;
    if Path::new(&basis.task_path)!=Path::new(&work).join("task.json") { return Err("workbench_retry_task_path_mismatch".into()); }
    let bytes=read_workbench_retry_task(Path::new(&basis.task_path))?;
    if workbench_retry_digest(&bytes)!=basis.task_digest { return Err("workbench_retry_inputs_changed".into()); }
    let raw:JsonValue=serde_json::from_slice(&bytes).map_err(|_| "workbench_retry_task_invalid")?;
    // Option<f64> alone treats a missing key as None; missing is not permission
    // to erase a previously configured budget.
    if raw.get("maxBudgetUsd").is_none() { return Err("workbench_retry_task_incomplete".into()); }
    let saved:WorkbenchSavedTask=serde_json::from_value(raw).map_err(|_| "workbench_retry_task_invalid")?;
    if saved.scan_id!=scan_id || saved.project_id!=project || saved.scan_type!=kind || saved.source_path!=source
        || saved.task_name!=name || saved.attempt!=basis.attempt || !matches!(kind.as_str(),"code"|"greybox"|"cicd")
        || saved.runtime_policy.get("backend").and_then(JsonValue::as_str)!=Some("native-agent") {
        return Err("workbench_retry_task_scope_mismatch".into());
    }
    if !matches!(saved.scan_mode.as_str(),"quick"|"standard"|"deep") || !matches!(saved.scope_mode.as_str(),"auto"|"diff"|"full") {
        return Err("workbench_retry_mode_invalid".into());
    }
    let current:JsonValue=serde_json::from_str(&context).map_err(|_| "workbench_retry_policy_invalid")?;
    for key in ["schemaVersion","policyKind","entryPoint","webModeCeiling","maxBudgetUsd","selectedSkillIds","additionalInstruction","authSessionId","authSessionIds","maxCritical","maxHigh","blockRelease"] {
        let original=saved.policy.get(key).ok_or("workbench_retry_policy_incomplete")?;
        if current.get(key)!=Some(original) { return Err("workbench_retry_policy_mismatch".into()); }
    }
    if saved.policy.get("schemaVersion").and_then(JsonValue::as_i64)!=Some(WEB_INVESTIGATION_POLICY_SCHEMA)
        || saved.policy.get("policyKind").and_then(JsonValue::as_str)!=Some("unified-web-investigation")
        || saved.policy.get("entryPoint").and_then(JsonValue::as_str)!=Some("workbench")
        || saved.policy.get("webModeCeiling").and_then(JsonValue::as_str)!=Some(saved.scan_mode.as_str())
        || saved.policy.get("maxBudgetUsd")!=Some(&json!(saved.max_budget_usd))
        || saved.policy.get("authSessionId")!=Some(&json!(saved.auth_session_id))
        || saved.policy.get("authSessionIds")!=Some(&json!(saved.auth_session_ids)) {
        return Err("workbench_retry_policy_mismatch".into());
    }
    let instruction=saved.policy.get("additionalInstruction").and_then(JsonValue::as_str).ok_or("workbench_retry_policy_invalid")?.to_string();
    if instruction.chars().count()>12_000 { return Err("workbench_retry_policy_invalid".into()); }
    let skill_ids=workbench_policy_skill_ids(&saved.policy)?;
    workbench_selected_skill_ids(&tx,&skill_ids,false)?;
    let max_critical=saved.policy.get("maxCritical").and_then(JsonValue::as_i64).filter(|v|(0..=10_000).contains(v)).ok_or("workbench_retry_policy_invalid")?;
    let max_high=saved.policy.get("maxHigh").and_then(JsonValue::as_i64).filter(|v|(0..=10_000).contains(v)).ok_or("workbench_retry_policy_invalid")?;
    let block_release=saved.policy.get("blockRelease").and_then(JsonValue::as_bool).ok_or("workbench_retry_policy_invalid")?;
    workbench_budget(saved.max_budget_usd)?;
    let mut stmt=tx.prepare("SELECT url FROM sentinel_targets WHERE scan_id=?1 ORDER BY url").map_err(|_| "workbench_retry_targets_unavailable")?;
    let actual=stmt.query_map([scan_id],|r|r.get::<_,String>(0)).map_err(|_| "workbench_retry_targets_unavailable")?
        .collect::<Result<Vec<_>,_>>().map_err(|_| "workbench_retry_targets_unavailable")?;
    drop(stmt);
    let mut expected=saved.urls.clone();
    if !source.is_empty() { expected.push(source.clone()); }
    expected.sort(); expected.dedup();
    if actual!=expected { return Err("workbench_retry_targets_changed".into()); }
    let input=WorkbenchScanInput { project_id:project,task_name:name,scan_type:kind,urls:saved.urls,source_path:source,
        skill_ids,instruction,scan_mode:saved.scan_mode,scope_mode:saved.scope_mode,diff_base:saved.diff_base,max_budget_usd:saved.max_budget_usd,
        environment:saved.environment,auth_profile_name:String::new(),auth_type:"none".into(),auth_header_name:String::new(),auth_value:String::new(),
        auth_session_id:saved.auth_session_id,auth_session_ids:saved.auth_session_ids,auth_session_scope_id:String::new(),
        ci_provider:saved.ci_provider,repository_url:saved.repository_url,branch:saved.branch,commit_sha:saved.commit_sha,build_id:saved.build_id,
        max_critical,max_high,block_release };
    tx.commit().map_err(|_| "workbench_retry_snapshot_unavailable")?;
    Ok((input,basis))
}
