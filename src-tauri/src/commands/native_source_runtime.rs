// Publication receipt for source model work. Public task/contract JSON contains
// no model key, endpoint or proxy credentials. A per-attempt private HMAC binds
// those values without turning imported historical JSON into execution rights.
const SOURCE_RUNTIME_KEY_FILE: &str = ".source-runtime-key";

// Apply at task creation as well as at execution/reentry. Historical task
// JSON remains readable, but cannot mint an unfunded source-model attempt.
fn source_model_cost_admission(capped: bool) -> Result<(), String> {
    if capped {
        return Err("source_model_cost_accounting_unavailable".into());
    }
    Ok(())
}

fn source_model_policy(model: &ModelRuntimeEnv) -> JsonValue {
    json!({"model":model.llm,"deployment":model.deployment,"fullPower":model.full_power,
        "promptAuditMode":model.prompt_audit_mode})
}

fn source_runtime_material(
    connection: &rusqlite::Connection, scan_id: &str, attempt: i64,
    directory: &WebBindingDirectory, model: &ModelRuntimeEnv, runtime: &AgentWebPipelineRuntime,
) -> Result<(JsonValue,Vec<u8>),String> {
    if connection.is_autocommit() { return Err("source_runtime_requires_transaction".into()); }
    if !native_source_attempt_active(connection,scan_id,attempt) {
        return Err("source_runtime_attempt_inactive".into());
    }
    let scope=load_workbench_source_scope(connection,scan_id,attempt)?;
    let project_id: i64=connection.query_row(
        "SELECT s.project_id FROM sentinel_scans s JOIN sentinel_scan_attempts a ON a.scan_id=s.id AND a.attempt_number=s.attempt_count
         JOIN projects p ON p.id=s.project_id WHERE s.id=?1 AND s.attempt_count=?2 AND a.work_dir=?3 AND s.task_path=?4 AND p.status='active'",
        params![scan_id,attempt,directory.path.to_string_lossy(),directory.path.join("task.json").to_string_lossy()],|r|r.get(0),
    ).map_err(|_|"source_runtime_work_directory_mismatch")?;
    let bytes=directory.read("task.json",WEB_DISPATCH_BINDING_LIMIT,false,false)?
        .ok_or("source_runtime_task_missing")?;
    let task: JsonValue=serde_json::from_slice(&bytes).map_err(|_|"source_runtime_task_invalid")?;
    if task["scanId"]!=scan_id || task["attempt"]!=attempt || task["sourcePath"]!=scope.source_path
        || task["scanType"]!=scope.scan_type || task["scopeMode"]!=scope.scope_mode
        || task["llmPolicy"]!=source_model_policy(model) || !task["policy"].is_object() {
        return Err("source_runtime_task_mismatch".into());
    }
    let task_scope=WorkbenchSourceScope::new(&scope.source_path,&scope.scan_type,&scope.scope_mode,
        task["diffBase"].as_str().ok_or("source_runtime_task_mismatch")?)?;
    if task_scope!=scope { return Err("source_runtime_task_mismatch".into()); }
    let requested_budget=match &task["maxBudgetUsd"] {
        JsonValue::Null=>None,
        JsonValue::Number(n)=>Some(n.as_f64().ok_or("source_runtime_budget_invalid")?),
        _=>return Err("source_runtime_budget_invalid".into()),
    };
    workbench_budget(requested_budget)?;
    if task["maxBudgetUsd"]!=task["policy"]["maxBudgetUsd"]
        || requested_budget!=runtime.adaptive.max_budget_usd
        || task["scanMode"].as_str()!=Some(runtime.adaptive.max_mode.as_str()) {
        return Err("source_runtime_budget_mismatch".into());
    }
    let settings=sentinel_settings(connection);
    let live_model=model_runtime_env(&settings)?;
    if serde_json::to_value(&live_model).map_err(|_|"source_runtime_model_invalid")?
        !=serde_json::to_value(model).map_err(|_|"source_runtime_model_invalid")? {
        return Err("source_runtime_model_changed_before_publication".into());
    }
    let (effective,_,_)=effective_web_policy(connection,&task["policy"],&settings)?;
    if effective!=runtime.web_policy { return Err("source_runtime_policy_mismatch".into()); }
    let (timeout,tokens,target_requests)=runtime.adaptive.limits(&runtime.adaptive.max_mode);
    let contract=json!({"schemaVersion":1,"scanId":scan_id,"attemptNumber":attempt,"projectId":project_id,
        "surface":"source","scope":scope,"modelPolicy":source_model_policy(model),
        "operatorPolicy":task["policy"],"effectivePolicy":effective,
        "budget":{"mode":runtime.adaptive.max_mode,"timeoutSeconds":timeout,"tokenLimit":tokens,
            "configuredTargetRequestLimit":target_requests,"maxBudgetUsd":requested_budget},
        "skillNames":runtime.skill_names,"skillInstructions":runtime.skill_instructions,
        "targetRequestsGranted":0,"hostActionsGranted":0});
    // Bind only model inputs, not unrelated browser/analyzer settings. No
    // default Web URL, plan, browser identity or tool authority is manufactured.
    #[cfg(unix)] let directory_identity=json!(directory.identity);
    #[cfg(not(unix))] let directory_identity=JsonValue::Null;
    let message=serde_json::to_vec(&json!({"domain":"oviraptor.source-runtime.v1","contract":contract,
        "workDir":directory.path,"directoryIdentity":directory_identity,
        "taskSha256":format!("{:x}",Sha256::digest(&bytes)),"model":model,
        "proxies":runtime.proxies,"noProxy":runtime.no_proxy}))
        .map_err(|_|"source_runtime_serialization_failed")?;
    if message.len() as u64>WEB_DISPATCH_BINDING_LIMIT { return Err("source_runtime_material_oversized".into()); }
    directory.check_identity()?;
    Ok((contract,message))
}

fn verify_source_runtime_in(
    connection: &rusqlite::Connection, scan_id: &str, attempt: i64,
    directory: &WebBindingDirectory, model: &ModelRuntimeEnv, runtime: &AgentWebPipelineRuntime,
) -> Result<JsonValue,String> {
    let (schema,stored,tag):(i64,String,Vec<u8>)=connection.query_row(
        "SELECT schema_version,contract_json,binding_tag FROM source_runtime_contracts WHERE scan_id=?1 AND attempt_number=?2",
        params![scan_id,attempt],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).map_err(|_|"source_runtime_receipt_missing")?;
    if schema!=1 || tag.len()!=32 { return Err("source_runtime_receipt_invalid".into()); }
    let key=directory.read(SOURCE_RUNTIME_KEY_FILE,32,false,false)?.ok_or("source_runtime_key_missing")?;
    if key.len()!=32 { return Err("source_runtime_key_invalid".into()); }
    let (contract,message)=source_runtime_material(connection,scan_id,attempt,directory,model,runtime)?;
    if serde_json::from_str::<JsonValue>(&stored).map_err(|_|"source_runtime_receipt_invalid")?!=contract {
        return Err("source_runtime_contract_changed".into());
    }
    aws_lc_rs::hmac::verify(&aws_lc_rs::hmac::Key::new(aws_lc_rs::hmac::HMAC_SHA256,&key),&message,&tag)
        .map_err(|_|"source_runtime_inputs_changed")?;
    Ok(contract)
}

fn register_source_runtime_contract(
    connection: &rusqlite::Connection, record: &WorkbenchStartRecord, work_dir: &Path,
    model: &ModelRuntimeEnv, runtime: &AgentWebPipelineRuntime,
) -> Result<(),String> {
    if record.llm_policy!=source_model_policy(model) { return Err("source_runtime_selection_mismatch".into()); }
    let directory=WebBindingDirectory::open(work_dir)?;
    let attempt=i64::from(record.attempt);
    let (contract,message)=source_runtime_material(connection,&record.scan_id,attempt,&directory,model,runtime)?;
    if contract["operatorPolicy"]!=record.policy { return Err("source_runtime_operator_policy_mismatch".into()); }
    // Allocate once. A leftover key after an uncertain publication is never
    // overwritten or adopted; a fresh attempt must use its own directory.
    let mut key=[0u8;32];
    aws_lc_rs::rand::fill(&mut key).map_err(|_|"source_runtime_entropy_unavailable")?;
    let mut file=directory.open_file(SOURCE_RUNTIME_KEY_FILE,true).map_err(|_|"source_runtime_key_create_failed")?;
    file.write_all(&key).and_then(|()|file.sync_all()).map_err(|_|"source_runtime_key_sync_failed")?;
    directory.read("task.json",WEB_DISPATCH_BINDING_LIMIT,false,true)?;
    directory.sync()?;
    let tag=aws_lc_rs::hmac::sign(&aws_lc_rs::hmac::Key::new(aws_lc_rs::hmac::HMAC_SHA256,&key),&message);
    let changed=connection.execute(
        "INSERT INTO source_runtime_contracts(scan_id,attempt_number,schema_version,contract_json,binding_tag) VALUES(?1,?2,1,?3,?4)",
        params![record.scan_id,attempt,contract.to_string(),tag.as_ref()],
    ).map_err(|_|"source_runtime_registration_failed")?;
    if changed!=1 { return Err("source_runtime_registration_not_persisted".into()); }
    verify_source_runtime_in(connection,&record.scan_id,attempt,&directory,model,runtime)?;
    // The receipt insert may have triggers. Recheck live settings, identities
    // and the unclaimed branch after the final write, not merely the old inputs.
    let current=resolve_agent_web_pipeline_runtime(connection,&record.scan_id,&model.deployment,PathBuf::new())?;
    verify_source_runtime_in(connection,&record.scan_id,attempt,&directory,model,&current)?;
    let unclaimed: bool=connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM native_branch_dispatches WHERE scan_id=?1 AND attempt_number=?2 AND branch='source' AND claim_id='' AND claimed_at='')",
        params![record.scan_id,attempt],|r|r.get(0),
    ).map_err(|_|"source_runtime_dispatch_missing")?;
    if !unclaimed { return Err("source_runtime_dispatch_already_claimed".into()); }
    Ok(())
}

fn verify_source_runtime_contract(
    connection: &rusqlite::Connection, scan_id: &str, attempt: i64, work_dir: &Path,
) -> Result<(ModelRuntimeEnv,AgentWebPipelineRuntime,JsonValue),String> {
    let tx=rusqlite::Transaction::new_unchecked(connection,rusqlite::TransactionBehavior::Immediate)
        .map_err(|_|"source_runtime_verification_unavailable")?;
    let model=model_runtime_env(&sentinel_settings(&tx))?;
    let runtime=resolve_agent_web_pipeline_runtime(&tx,scan_id,&model.deployment,PathBuf::new())?;
    let directory=WebBindingDirectory::open(work_dir)?;
    let contract=verify_source_runtime_in(&tx,scan_id,attempt,&directory,&model,&runtime)?;
    tx.commit().map_err(|_|"source_runtime_verification_unconfirmed")?;
    Ok((model,runtime,contract))
}

// Shared by real production-entry regression tests, including the retired-CLI
// PATH trap. Publish actual authority rather than injecting executable rows.
#[cfg(test)]
fn source_runtime_publication_fixture(
    connection: &rusqlite::Connection, scan_id: &str, work_dir: &Path, source: &Path,
    scan_type: &str, scope_mode: &str, diff_base: &str,
) {
    connection.execute("INSERT INTO projects(name) VALUES('Source publication fixture')",[]).unwrap();
    let project_id=connection.last_insert_rowid();
    connection.execute("UPDATE config_profiles SET settings_json=json_set(settings_json,'$.modelProfiles',json(?1),'$.activeModelProfileId','source-fixture') WHERE id=(SELECT id FROM config_profiles ORDER BY is_default DESC,id LIMIT 1)",
        [json!([{"id":"source-fixture","llm":"fixture-source-model","apiKey":"fixture-source-key","apiBase":"http://127.0.0.1:9/v1","deployment":"cloud"}]).to_string()]).unwrap();
    let model=model_runtime_env(&sentinel_settings(connection)).unwrap();
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(work_dir,fs::Permissions::from_mode(0o700)).unwrap();
    }
    let policy=json!({"webModeCeiling":"standard","maxBudgetUsd":null,"maxCritical":0,"maxHigh":0,"blockRelease":true});
    let task=json!({"scanId":scan_id,"projectId":project_id,"projectName":"Source publication fixture",
        "taskName":"Source fixture","scanType":scan_type,"attempt":1,"urls":[],"sourcePath":source,
        "scopeMode":scope_mode,"diffBase":diff_base,"skills":"","environment":"internal","authProfileName":"",
        "authType":"none","authSessionIds":[],"authenticated":false,"ciProvider":"","repositoryUrl":"",
        "branch":"","commitSha":"","buildId":"","policy":policy,"llmPolicy":source_model_policy(&model),
        "scanMode":"standard","maxBudgetUsd":null});
    fs::write(work_dir.join("task.json"),task.to_string()).unwrap();
    let record: WorkbenchStartRecord=serde_json::from_value(task).unwrap();
    let plan=ScanBackendPlan {scan_id:scan_id.into(),attempt_number:1,targets:vec![],requires_node:false,requires_browser:false};
    publish_workbench_start(connection,&record,false,"",work_dir,&plan,PathBuf::new(),&model).unwrap();
}
