// Workbench publication is a single database transaction. Preparation may fail
// before this boundary; no source or Web worker may launch until it commits.
#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkbenchStartRecord {
    scan_id: String,
    project_id: i64,
    project_name: String,
    task_name: String,
    scan_type: String,
    attempt: u32,
    urls: Vec<String>,
    source_path: String,
    scope_mode: String,
    diff_base: String,
    skills: String,
    environment: String,
    auth_profile_name: String,
    auth_type: String,
    auth_session_ids: Vec<String>,
    authenticated: bool,
    ci_provider: String,
    repository_url: String,
    branch: String,
    commit_sha: String,
    build_id: String,
    policy: JsonValue,
    #[serde(default)]
    llm_policy: JsonValue,
    #[serde(default)]
    retry_basis: Option<WorkbenchRetryBasis>,
}

fn workbench_budget(value: Option<f64>) -> Result<Option<f64>, String> {
    if value.is_some_and(|v| !v.is_finite() || v <= 0.0 || v > 10_000.0) {
        return Err("Token 预算对应的美元上限必须为有限数、大于 0 且不超过 10000".into());
    }
    // Throughput settings never erase an operator's explicit cost ceiling.
    Ok(value)
}

// Only synchronous thread-spawn failures enter here: no target operation has
// started in this branch. A running sibling keeps its own execution/outcome.
fn record_workbench_spawn_result(db_path: &Path, scan_id: &str, attempt: i64, branch: &str, result: Result<(),String>) -> Result<(),String> {
    if let Err(error) = result {
        let report=json!({"failurePhase":"thread_spawn","executionStarted":false,"error":error});
        if branch=="source" {
            finish_native_source_branch(db_path,scan_id,attempt,&report)?;
        } else {
            finish_native_branch(db_path,scan_id,attempt,branch,"failed",&format!("分支线程未启动：{error}"),&report)?;
        }
    }
    Ok(())
}

// A successfully spawned worker can still fail admission. Only workbench
// branches with a durable *unclaimed* receipt may be settled as not executed.
// A duplicate caller, a prior claim or an uncertain commit never proves that.
fn claim_workbench_pipeline_branch(
    db_path: &Path, scan_id: &str, attempt: i64, branch: &'static str,
) -> Result<NativeBranchGuard,String> {
    match NativeBranchGuard::claim(db_path,scan_id,attempt,branch) {
        Ok(guard) => Ok(guard),
        Err(error) => {
            match settle_unclaimed_workbench_admission(db_path,scan_id,attempt,branch,&error) {
                Ok(_) => Err(error),
                Err(_) => Err(format!("{error}; workbench_admission_settlement_unconfirmed")),
            }
        }
    }
}

fn settle_unclaimed_workbench_admission(
    db_path: &Path, scan_id: &str, attempt: i64, branch: &str, error: &str,
) -> Result<bool,String> {
    if !matches!(branch,"source"|"web") { return Err("invalid_native_branch".into()); }
    if error.starts_with("native_dispatch_commit_unconfirmed:")
        || error.starts_with("native_invocation_not_owned:") {
        return Ok(false);
    }
    // Claim and settlement take the same OS lock, then the database write lock.
    // Recheck receipts under both, so a competing successful worker wins intact.
    let _owner = match claim_native_invocation(db_path,scan_id,attempt,"branch",branch) {
        Ok(owner) => owner,
        Err(error) if error.starts_with("native_invocation_not_owned:") => return Ok(false),
        Err(error) => return Err(error),
    };
    let mut connection = db::open(db_path)?;
    connection.pragma_update(None,"synchronous","FULL").map_err(|e| e.to_string())?;
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e| e.to_string())?;
    let eligible: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sentinel_scans s
         JOIN sentinel_scan_attempts a ON a.scan_id=s.id AND a.attempt_number=s.attempt_count
         JOIN native_scan_branches b ON b.scan_id=s.id AND b.attempt_number=s.attempt_count
         JOIN native_branch_dispatches d ON d.scan_id=b.scan_id AND d.attempt_number=b.attempt_number AND d.branch=b.branch
         WHERE s.id=?1 AND s.attempt_count=?2 AND s.scan_type IN ('code','greybox','cicd')
         AND s.status='scanning' AND a.status='scanning' AND b.branch=?3 AND b.status='pending'
         AND d.claim_id='' AND d.claimed_at='')
         AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)
         AND NOT EXISTS(SELECT 1 FROM native_web_attempt_closures WHERE scan_id=?1 AND attempt_number=?2)",
        params![scan_id,attempt,branch],|row| row.get(0),
    ).map_err(|e| e.to_string())?;
    if !eligible { return Ok(false); }
    // Do not persist arbitrary error strings: driver errors may contain input
    // data. A fixed code and phase are sufficient for truthful UI/reporting.
    let report = json!({"code":"workbench_admission_rejected","failurePhase":"branch_admission","executionStarted":false});
    let checkpoint = "分支准入失败，未取得执行权；此分支未开始执行";
    let siblings = workbench_admission_siblings(&tx,scan_id,attempt,branch)?;
    if !finish_native_branch_in(&tx,scan_id,attempt,branch,"failed",checkpoint,&report)? {
        return Err("workbench_admission_not_persisted".into());
    }
    // No URL in an unclaimed Web branch was dispatched. Keep the task's target
    // list truthful too; do not leave greybox URLs looking actively scanning.
    if branch == "web" {
        tx.execute("UPDATE sentinel_targets SET status='failed' WHERE scan_id=?1 AND last_attempt_number=?2
            AND url<>(SELECT source_path FROM sentinel_scans WHERE id=?1)",params![scan_id,attempt])
            .map_err(|e| e.to_string())?;
    }
    let targets_settled: bool = tx.query_row(
        "SELECT count(*)>0 AND sum(CASE WHEN status='failed' AND last_attempt_number=?2 THEN 0 ELSE 1 END)=0
         FROM sentinel_targets WHERE scan_id=?1 AND
         ((?3='source' AND url=(SELECT source_path FROM sentinel_scans WHERE id=?1))
          OR (?3='web' AND url<>(SELECT source_path FROM sentinel_scans WHERE id=?1)))",
        params![scan_id,attempt,branch],|row|row.get(0),
    ).map_err(|e|e.to_string())?;
    let persisted: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sentinel_scans s
         JOIN sentinel_scan_attempts a ON a.scan_id=s.id AND a.attempt_number=s.attempt_count
         JOIN native_scan_branches b ON b.scan_id=s.id AND b.attempt_number=s.attempt_count
         JOIN native_branch_dispatches d ON d.scan_id=b.scan_id AND d.attempt_number=b.attempt_number AND d.branch=b.branch
         WHERE s.id=?1 AND s.attempt_count=?2 AND b.branch=?3 AND b.status='failed'
         AND b.checkpoint=?4 AND b.report_json=?5 AND d.claim_id='' AND d.claimed_at=''
         AND a.status=s.status AND a.checkpoint=s.current_checkpoint
         AND s.scan_type IN ('code','greybox','cicd') AND s.status=CASE
             WHEN EXISTS(SELECT 1 FROM native_scan_branches WHERE scan_id=?1 AND attempt_number=?2 AND status='pending') THEN 'scanning'
             WHEN EXISTS(SELECT 1 FROM native_scan_branches WHERE scan_id=?1 AND attempt_number=?2 AND status<>'failed') THEN 'partial'
             ELSE 'failed' END)
         AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)",
        params![scan_id,attempt,branch,checkpoint,report.to_string()],|row| row.get(0),
    ).map_err(|e| e.to_string())?;
    if !persisted { return Err("workbench_admission_branch_postcondition".into()); }
    if !targets_settled { return Err("workbench_admission_target_postcondition".into()); }
    if siblings != workbench_admission_siblings(&tx,scan_id,attempt,branch)? {
        return Err("workbench_admission_sibling_postcondition".into());
    }
    tx.commit().map_err(|error| format!("workbench_admission_commit_unconfirmed:{error}"))?;
    Ok(true)
}

// Preserve the independent sibling's result and dispatch receipt, including
// the absence of a receipt. NULL is not silently treated as an empty claim.
fn workbench_admission_siblings(connection: &rusqlite::Connection,scan_id: &str,attempt: i64,branch: &str) -> Result<Vec<JsonValue>,String> {
    let mut stmt = connection.prepare(
        "SELECT b.branch,b.status,b.checkpoint,b.report_json,d.claim_id,d.claimed_at
         FROM native_scan_branches b LEFT JOIN native_branch_dispatches d
         ON d.scan_id=b.scan_id AND d.attempt_number=b.attempt_number AND d.branch=b.branch
         WHERE b.scan_id=?1 AND b.attempt_number=?2 AND b.branch<>?3 ORDER BY b.branch",
    ).map_err(|e| e.to_string())?;
    let mut rows = stmt.query_map(params![scan_id,attempt,branch],|row| Ok(json!([
        row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,
        row.get::<_,Option<String>>(4)?,row.get::<_,Option<String>>(5)?,
    ]))).map_err(|e| e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())?;
    let mut targets = connection.prepare(
        "SELECT id,url,status,last_attempt_number FROM sentinel_targets WHERE scan_id=?1 AND
         ((?2='source' AND url<>(SELECT source_path FROM sentinel_scans WHERE id=?1))
          OR (?2='web' AND url=(SELECT source_path FROM sentinel_scans WHERE id=?1))) ORDER BY id",
    ).map_err(|e| e.to_string())?;
    let sibling_targets = targets.query_map(params![scan_id,branch],|row| Ok(json!({"target":[
        row.get::<_,i64>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,i64>(3)?,
    ]}))).map_err(|e| e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())?;
    rows.extend(sibling_targets);
    Ok(rows)
}

#[allow(clippy::too_many_arguments)]
fn publish_workbench_start(
    connection: &rusqlite::Connection,
    record: &WorkbenchStartRecord,
    reuse: bool,
    draft_scope: &str,
    work_dir: &Path,
    plan: &ScanBackendPlan,
    worker: PathBuf,
    model: &ModelRuntimeEnv,
) -> Result<(SentinelScan, AgentWebPipelineRuntime), String> {
    let tx = rusqlite::Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let r = record;
    let task_path = work_dir.join("task.json");
    let active: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1 AND name=?2 AND status='active') AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?3)",
        params![r.project_id,r.project_name,r.scan_id], |row| row.get(0)).map_err(|e| e.to_string())?;
    if !active { return Err("workbench_start_scope_unavailable".into()); }
    if !matches!(r.scan_type.as_str(), "code" | "greybox" | "cicd") || r.attempt == 0
        || plan.scan_id != r.scan_id || plan.attempt_number != i64::from(r.attempt)
        || plan.targets.iter().any(|t| t.backend != AgentBackendKind::Native)
        || plan.targets.iter().map(|t| &t.url).collect::<Vec<_>>() != r.urls.iter().collect::<Vec<_>>()
    { return Err("workbench_start_plan_mismatch".into()); }
    let mut expected_targets = r.urls.clone();
    if !r.source_path.is_empty() { expected_targets.push(r.source_path.clone()); }
    expected_targets.sort();
    expected_targets.dedup();
    if expected_targets.is_empty() { return Err("workbench_start_empty_targets".into()); }
    let checkpoint = format!("Agent 协作台任务正在执行第 {} 次尝试", r.attempt);
    if reuse {
        let expected = r.retry_basis.as_ref().ok_or("workbench_retry_basis_missing")?;
        if &capture_workbench_retry_basis(&tx,&r.scan_id)? != expected {
            return Err("workbench_retry_inputs_changed".into());
        }
        let prior: (i64,String,String,String,i64) = tx.query_row("SELECT project_id,scan_type,source_path,status,attempt_count FROM sentinel_scans WHERE id=?1",
            [&r.scan_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?))).map_err(|e| e.to_string())?;
        if prior.0 != r.project_id || prior.1 != r.scan_type || prior.2 != r.source_path
            || !matches!(prior.3.as_str(), "paused"|"partial"|"completed"|"completed_with_gaps"|"failed"|"cancelled"|"protected_stop"|"interrupted")
            || prior.4 >= i64::from(r.attempt)
        { return Err("workbench_retry_scope_changed".into()); }
        let mut stmt = tx.prepare("SELECT url FROM sentinel_targets WHERE scan_id=?1 ORDER BY url").map_err(|e| e.to_string())?;
        let old = stmt.query_map([&r.scan_id], |row| row.get::<_,String>(0)).map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())?;
        if old != expected_targets { return Err("workbench_retry_targets_changed".into()); }
        let changed = tx.execute("UPDATE sentinel_scans SET status='scanning',current_checkpoint=?1,task_path=?2,previous_scan_id='',task_name=?3,skill_names=?4,attempt_count=?5,updated_at=datetime('now','localtime') WHERE id=?6 AND attempt_count=?7 AND status=?8",
            params![checkpoint,task_path.to_string_lossy(),r.task_name,r.skills,r.attempt,r.scan_id,prior.4,prior.3]).map_err(|e| e.to_string())?;
        if changed != 1 { return Err("workbench_retry_not_persisted".into()); }
    } else {
        let changed = tx.execute("INSERT INTO sentinel_scans(id,project_id,project_name,status,current_checkpoint,task_path,scan_type,task_name,source_path,skill_names,attempt_count) VALUES(?1,?2,?3,'scanning',?4,?5,?6,?7,?8,?9,?10)",
            params![r.scan_id,r.project_id,r.project_name,checkpoint,task_path.to_string_lossy(),r.scan_type,r.task_name,r.source_path,r.skills,r.attempt]).map_err(|e| e.to_string())?;
        if changed != 1 { return Err("workbench_scan_not_persisted".into()); }
        crate::auth_session::bind_draft_sessions_to_scan(&tx,&r.auth_session_ids,r.project_id,draft_scope,&r.scan_id)?;
    }
    // Never let the general attempt upsert overwrite a prior attempt's ledger.
    let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sentinel_scan_attempts WHERE scan_id=?1 AND attempt_number>=?2)",
        params![r.scan_id,r.attempt], |row| row.get(0)).map_err(|e| e.to_string())?;
    if exists { return Err("workbench_attempt_already_exists".into()); }
    let backend = plan.as_json().to_string();
    tx.execute("INSERT INTO sentinel_checkpoints(scan_id,url,stage,raw_json) VALUES(?1,'','scan_backend_plan',?2) ON CONFLICT(scan_id,url,stage) DO UPDATE SET raw_json=excluded.raw_json,updated_at=datetime('now','localtime')",
        params![r.scan_id,backend]).map_err(|e| e.to_string())?;
    record_sentinel_attempt_start(&tx,&r.scan_id,r.attempt.into(),work_dir)?;
    let source_scope = if r.source_path.is_empty() { None } else {
        let scope = WorkbenchSourceScope::from_record(r)?;
        scope.verify_task_file(r,work_dir)?;
        store_workbench_source_scope(&tx,&r.scan_id,r.attempt.into(),&scope)?;
        Some(scope)
    };
    if r.scan_type == "cicd" {
        store_workbench_ci_policy(&tx,&r.scan_id,r.attempt.into(),&r.policy)?;
    }
    tx.execute("INSERT INTO sentinel_scan_contexts(scan_id,environment,auth_profile_name,auth_type,authenticated,ci_provider,repository_url,branch,commit_sha,build_id,policy_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11) ON CONFLICT(scan_id) DO UPDATE SET environment=excluded.environment,auth_profile_name=excluded.auth_profile_name,auth_type=excluded.auth_type,authenticated=excluded.authenticated,ci_provider=excluded.ci_provider,repository_url=excluded.repository_url,branch=excluded.branch,commit_sha=excluded.commit_sha,build_id=excluded.build_id,policy_json=excluded.policy_json,gate_status='',gate_reason='',updated_at=datetime('now','localtime')",
        params![r.scan_id,r.environment,r.auth_profile_name,r.auth_type,r.authenticated,r.ci_provider,r.repository_url,r.branch,r.commit_sha,r.build_id,r.policy.to_string()]).map_err(|e| e.to_string())?;
    for target in &expected_targets {
        let fused: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sentinel_fuse_zone WHERE project_id=?1 AND archived=0 AND normalized_url=lower(rtrim(trim(?2),'/')))",
            params![r.project_id,target], |row| row.get(0)).map_err(|e| e.to_string())?;
        if fused { return Err("workbench_target_fuse_requires_review".into()); }
        if reuse {
            tx.execute("UPDATE sentinel_targets SET status='scanning',last_attempt_number=?3,updated_at=datetime('now','localtime') WHERE scan_id=?1 AND url=?2",params![r.scan_id,target,r.attempt]).map_err(|e| e.to_string())?;
        } else {
            tx.execute("INSERT INTO sentinel_targets(project_id,scan_id,company,url,status,last_attempt_number) VALUES(?1,?2,?3,?4,'scanning',?5)",params![r.project_id,r.scan_id,r.project_name,target,r.attempt]).map_err(|e| e.to_string())?;
        }
    }
    insert_source_inventory(&tx,&r.scan_id,&r.source_path)?;
    let runtime = resolve_agent_web_pipeline_runtime(&tx,&r.scan_id,&model.deployment,worker)?;
    tx.execute("UPDATE sentinel_scan_contexts SET policy_json=?1,gate_status='not_evaluated' WHERE scan_id=?2",params![runtime.web_policy.to_string(),r.scan_id]).map_err(|e| e.to_string())?;
    let mut branches = Vec::new();
    if !r.source_path.is_empty() { branches.push("source"); }
    if !r.urls.is_empty() { branches.push("web"); }
    register_native_branches_in(&tx,&r.scan_id,r.attempt.into(),&branches)?;
    if !r.source_path.is_empty() {
        register_source_runtime_contract(&tx,r,work_dir,model,&runtime)?;
    }
    // Check postconditions after all writes/triggers, including ignored inserts.
    let valid: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1 AND project_id=?2 AND project_name=?3 AND scan_type=?4 AND source_path=?5 AND status='scanning' AND attempt_count=?6 AND task_path=?7 AND task_name=?8 AND skill_names=?9 AND current_checkpoint=?10)",
        params![r.scan_id,r.project_id,r.project_name,r.scan_type,r.source_path,r.attempt,task_path.to_string_lossy(),r.task_name,r.skills,checkpoint], |row| row.get(0)).map_err(|e| e.to_string())?;
    let attempt_valid: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sentinel_scan_attempts WHERE scan_id=?1 AND attempt_number=?2 AND status='scanning' AND work_dir=?3 AND backend_plan_json=?4 AND finished_at='' AND stop_reason='') AND EXISTS(SELECT 1 FROM sentinel_checkpoints WHERE scan_id=?1 AND url='' AND stage='scan_backend_plan' AND raw_json=?4)",
        params![r.scan_id,r.attempt,work_dir.to_string_lossy(),backend], |row| row.get(0)).map_err(|e| e.to_string())?;
    let context_valid: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sentinel_scan_contexts WHERE scan_id=?1 AND environment=?2 AND auth_profile_name=?3 AND auth_type=?4 AND authenticated=?5 AND ci_provider=?6 AND repository_url=?7 AND branch=?8 AND commit_sha=?9 AND build_id=?10 AND policy_json=?11 AND gate_status='not_evaluated' AND gate_reason='')",
        params![r.scan_id,r.environment,r.auth_profile_name,r.auth_type,r.authenticated,r.ci_provider,r.repository_url,r.branch,r.commit_sha,r.build_id,runtime.web_policy.to_string()], |row| row.get(0)).map_err(|e| e.to_string())?;
    if !valid { return Err("workbench_scan_postcondition".into()); }
    if !attempt_valid { return Err("workbench_attempt_postcondition".into()); }
    if !context_valid { return Err("workbench_context_postcondition".into()); }
    let mut stmt = tx.prepare("SELECT url,project_id,company,status,last_attempt_number FROM sentinel_targets WHERE scan_id=?1 ORDER BY url").map_err(|e| e.to_string())?;
    let actual = stmt.query_map([&r.scan_id], |row| Ok((row.get::<_,String>(0)?,row.get::<_,i64>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,i64>(4)?)))
        .map_err(|e| e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())?;
    drop(stmt);
    let expected = expected_targets.into_iter().map(|url| (url,r.project_id,r.project_name.clone(),"scanning".to_string(),i64::from(r.attempt))).collect::<Vec<_>>();
    if actual != expected { return Err("workbench_targets_postcondition".into()); }
    for id in &r.auth_session_ids {
        let bound: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM browser_auth_sessions WHERE id=?1 AND project_id=?2 AND owner_scan_id=?3 AND draft_scope_id='')",params![id,r.project_id,r.scan_id], |row| row.get(0)).map_err(|e| e.to_string())?;
        if !bound { return Err("workbench_identity_postcondition".into()); }
    }
    // Recheck after all writes/triggers: expired/replaced authentication during
    // dependency preparation must not publish a runnable stale-credential task.
    verify_workbench_browser_auth_publication(&tx,r,work_dir)?;
    if let Some(scope) = source_scope {
        scope.verify_task_file(r,work_dir)?;
        verify_workbench_source_scope(&tx,&r.scan_id,r.attempt.into(),&scope)?;
    }
    if r.scan_type == "cicd" && load_workbench_ci_policy(&tx,&r.scan_id,r.attempt.into())?
        != GatePolicy::from_workbench(&r.policy)? {
        return Err("workbench_ci_policy_postcondition".into());
    }
    if !r.source_path.is_empty() {
        let directory=WebBindingDirectory::open(work_dir)?;
        verify_source_runtime_in(&tx,&r.scan_id,r.attempt.into(),&directory,model,&runtime)?;
    }
    let result = sentinel_scan_by_id(&tx,&r.scan_id)?;
    tx.commit().map_err(|e| format!("workbench_start_commit_unconfirmed:{e}"))?;
    Ok((result,runtime))
}
