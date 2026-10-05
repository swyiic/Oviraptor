// Explicit operator-only admission for a bound ordinary Web attempt that has
// never acquired dispatch ownership. This is not resume/retry and creates no
// new attempt, policy, file, key, identity, budget or authorization.
fn web_recovery_scope_in(connection: &rusqlite::Connection, scan_id: &str, attempt: i64) -> Result<(), String> {
    if !native_dispatch_attempt_eligible(connection, scan_id, attempt, "web")? {
        return Err("web_recovery_attempt_ineligible".into());
    }
    let eligible: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sentinel_scans s JOIN sentinel_scan_attempts a
         ON a.scan_id=s.id AND a.attempt_number=s.attempt_count
         WHERE s.id=?1 AND s.scan_type='web' AND s.source_path='' AND a.status='scanning'
         AND s.llm_requests=a.llm_requests_start AND s.total_tokens=a.total_tokens_start
         AND s.input_tokens=a.input_tokens_start AND s.output_tokens=a.output_tokens_start
         AND s.cached_tokens=a.cached_tokens_start
         AND (SELECT COUNT(*) FROM native_scan_branches b WHERE b.scan_id=s.id AND b.attempt_number=?2)=1)
         AND EXISTS(SELECT 1 FROM native_web_dispatch_bindings WHERE scan_id=?1 AND attempt_number=?2 AND schema_version=1)
         AND NOT EXISTS(SELECT 1 FROM agent_runs WHERE scan_id=?1 AND attempt_number=?2)
         AND NOT EXISTS(SELECT 1 FROM sentinel_processes WHERE scan_id=?1)
         AND NOT EXISTS(SELECT 1 FROM analyzer_container_receipts WHERE scan_id=?1 AND cleanup_status<>'confirmed')
         AND NOT EXISTS(SELECT 1 FROM agent_gap_followups WHERE scan_id=?1)
         AND NOT EXISTS(SELECT 1 FROM agent_authorization_controls WHERE scan_id=?1 AND attempt_number=?2)",
        params![scan_id,attempt], |row| row.get(0),
    ).map_err(|_| "web_recovery_scope_lookup_failed")?;
    if !eligible { return Err("web_recovery_requires_separate_review".into()); }
    Ok(())
}

// Read-only UI hint, never an authorization. Full HMAC/files/tool/session checks
// run only on the explicit command, not on every status poll.
fn web_recovery_available_in(connection: &rusqlite::Connection, scan_id: &str, attempt: i64) -> bool {
    web_recovery_scope_in(connection,scan_id,attempt).is_ok()
        && connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM native_branch_dispatches WHERE scan_id=?1 AND attempt_number=?2
             AND branch='web' AND claim_id='' AND claimed_at='')",
            params![scan_id,attempt], |row| row.get::<_,bool>(0),
        ).unwrap_or(false)
}

struct OwnedWebRecovery<T> {
    // Held until the caller has handed the guard to its worker.
    _lifecycle: NativeInvocationOwner,
    guard: NativeBranchGuard,
    inputs: T,
}

fn claim_web_recovery<T>(
    db_path: &Path, scan_id: &str, attempt: i64, operator_confirmed: bool,
    mut reconstruct: impl FnMut(&rusqlite::Connection) -> Result<T,String>,
) -> Result<OwnedWebRecovery<T>, String> {
    if !operator_confirmed { return Err("web_recovery_explicit_confirmation_required".into()); }
    validate_web_start_id(scan_id)?;
    if attempt <= 0 || attempt > i64::from(u32::MAX) { return Err("web_recovery_attempt_invalid".into()); }
    let lifecycle = claim_scan_control(db_path,scan_id)?;
    let mut inputs = None;
    let mut guard = NativeBranchGuard::claim_with_preflight(db_path,scan_id,attempt,"web",|connection| {
        web_recovery_scope_in(connection,scan_id,attempt)?;
        inputs = Some(reconstruct(connection)?);
        Ok(())
    })?;
    let Some(inputs) = inputs else {
        // Defensive only: claim_with_preflight calls reconstruct twice. If that
        // contract ever changes, preserve the receipt and do not infer failure.
        guard.disarm();
        return Err("web_recovery_inputs_unconfirmed".into());
    };
    Ok(OwnedWebRecovery { _lifecycle: lifecycle, guard, inputs })
}

struct WebRecoveryInputs {
    work_dir: PathBuf,
    targets: Vec<(String,String)>,
    runtime: AgentWebPipelineRuntime,
    model: ModelRuntimeEnv,
    runtime_path: OsString,
    auth_session_path: Option<PathBuf>,
    admission: WebDispatchAdmission,
}

fn web_recovery_plan_in(
    connection: &rusqlite::Connection, app_data: &Path, scan_id: &str, attempt: i64,
) -> Result<(PathBuf,Vec<(String,String)>), String> {
    let (work_dir, task_path, raw_plan): (String,String,String) = connection.query_row(
        "SELECT a.work_dir,s.task_path,a.backend_plan_json FROM sentinel_scan_attempts a
         JOIN sentinel_scans s ON s.id=a.scan_id AND s.attempt_count=a.attempt_number
         WHERE s.id=?1 AND a.attempt_number=?2", params![scan_id,attempt],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).map_err(|_| "web_recovery_plan_missing")?;
    let expected = fs::canonicalize(app_data).map_err(|_| "web_recovery_app_data_missing")?
        .join(SCAN_WORK_DIRECTORY).join(scan_id).join(format!("attempt-{attempt:04}"));
    if Path::new(&work_dir) != expected || Path::new(&task_path) != expected.join("task.json") {
        return Err("web_recovery_artifact_provenance_invalid".into());
    }
    let directory = WebBindingDirectory::open(&expected)?;
    for entry in fs::read_dir(&expected).map_err(|_| "web_recovery_directory_unavailable")? {
        let entry = entry.map_err(|_| "web_recovery_directory_unavailable")?;
        if !matches!(entry.file_name().to_str(), Some(".oviraptor-scan-id" | ".web-dispatch-key" |
            "task.json" | "targets.json" | "targets.txt" | "agent-instruction.md" |
            "model-prompt-audit.json" | "auth-sessions.json" | "oviraptor-runner.log")) {
            return Err("web_recovery_unexpected_artifacts_require_review".into());
        }
    }
    // A prior refused launch may have written a diagnostic log. It is not a
    // frozen input, but must not redirect subsequent appends through a link or
    // special file. Apply the same anchored regular-file checks as startup.
    directory.read("oviraptor-runner.log",WEB_DISPATCH_BINDING_LIMIT,true,false)?;
    let mut statement = connection.prepare(
        "SELECT company,url FROM sentinel_targets WHERE scan_id=?1 AND last_attempt_number=?2 ORDER BY id",
    ).map_err(|_| "web_recovery_targets_unavailable")?;
    let targets = statement.query_map(params![scan_id,attempt], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?)))
        .map_err(|_| "web_recovery_targets_unavailable")?.collect::<Result<Vec<_>,_>>()
        .map_err(|_| "web_recovery_targets_unavailable")?;
    if targets.is_empty() || targets.iter().any(|(_,url)| reqwest::Url::parse(url).map_or(true, |parsed|
        !matches!(parsed.scheme(),"http"|"https") || parsed.host_str().is_none()
            || !parsed.username().is_empty() || parsed.password().is_some())) {
        return Err("web_recovery_web_scope_invalid".into());
    }
    let raw: JsonValue = serde_json::from_str(&raw_plan).map_err(|_| "web_recovery_backend_plan_invalid")?;
    let plan = ScanBackendPlan::from_json(&raw).ok_or("web_recovery_backend_plan_invalid")?;
    let urls = targets.iter().map(|(_,url)|url.as_str()).collect::<HashSet<_>>();
    if plan.scan_id != scan_id || plan.attempt_number != attempt || !plan.requires_node || !plan.requires_browser
        || raw["targets"].as_array().map(Vec::len) != Some(plan.targets.len())
        || plan.targets.len() != urls.len() || plan.targets.iter().map(|t|t.url.as_str()).collect::<HashSet<_>>() != urls
        || plan.targets.iter().any(|target|target.backend != AgentBackendKind::Native) {
        return Err("web_recovery_backend_plan_invalid".into());
    }
    let frozen_targets = serde_json::json!(targets.iter().map(|(company,url)|
        serde_json::json!({"company":company,"url":url})).collect::<Vec<_>>());
    let read_json = |name| -> Result<JsonValue,String> {
        let bytes = directory.read(name,WEB_DISPATCH_BINDING_LIMIT,false,false)?.ok_or("web_recovery_artifact_missing")?;
        serde_json::from_slice(&bytes).map_err(|_| "web_recovery_artifact_invalid".into())
    };
    let task = read_json("task.json")?;
    if task["scanId"] != scan_id || task["targets"] != frozen_targets
        || task.pointer("/runtimePolicy/backend").and_then(JsonValue::as_str) != Some("native-agent")
        || read_json("targets.json")? != frozen_targets {
        return Err("web_recovery_artifact_plan_mismatch".into());
    }
    directory.check_identity()?;
    Ok((expected,targets))
}

fn web_recovery_tools_present(descriptor: &JsonValue) -> Result<(),String> {
    for kind in ["nodeCandidates","browserCandidates"] {
        let present = descriptor["toolchain"][kind].as_array().is_some_and(|entries| entries.iter().any(|entry| {
            if entry["state"] != "present" { return false; }
            #[cfg(unix)] { entry.pointer("/stamp/unix/mode").and_then(JsonValue::as_u64).is_some_and(|mode|mode & 0o111 != 0) }
            #[cfg(not(unix))] { true }
        }));
        if !present { return Err(format!("web_recovery_tool_missing:{kind}")); }
    }
    // Presence is not a health/version check. No tool is executed pre-claim and
    // no tool is downloaded or substituted to make this admission pass.
    Ok(())
}

fn reconstruct_web_recovery_in(
    connection: &rusqlite::Connection, app_data: &Path, scan_id: &str, attempt: i64, worker: &Path,
) -> Result<WebRecoveryInputs,String> {
    let (work_dir,targets) = web_recovery_plan_in(connection,app_data,scan_id,attempt)?;
    let home = app_data.parent().unwrap_or(app_data).to_path_buf();
    let settings = sentinel_settings(connection);
    let model = model_runtime_env(&settings)?;
    let runtime = resolve_agent_web_pipeline_runtime(connection,scan_id,&model.deployment,worker.into())?;
    let runtime_path = sentinel_runtime_path(&home);
    let descriptor = web_dispatch_runtime_binding(connection,&runtime,&model,&runtime_path)?;
    verify_web_dispatch_binding_in(connection,scan_id,attempt,&WebBindingDirectory::open(&work_dir)?,&descriptor)?;
    private_web_mode_on(connection,scan_id,attempt)?;
    web_recovery_tools_present(&descriptor)?;
    let recon_config = serde_json::from_value(descriptor["frontendConfig"].clone()).map_err(|_| "web_binding_frontend_config_invalid")?;
    // The binding verifier has already compared current, unexpired, project-
    // scoped identity material with this exact startup file.
    let auth_session_path = if WebBindingDirectory::open(&work_dir)?.read("auth-sessions.json",WEB_DISPATCH_BINDING_LIMIT,true,false)?.is_some() {
        Some(work_dir.join("auth-sessions.json"))
    } else { None };
    Ok(WebRecoveryInputs { work_dir,targets,runtime,model,runtime_path,auth_session_path,
        admission: WebDispatchAdmission { home,settings,recon_config,claimed_guard:None } })
}

#[tauri::command]
pub async fn recover_never_dispatched_web_attempt(
    app: AppHandle, state: State<'_, AppState>, scan_id: String, attempt_number: i64, operator_confirmed: bool,
) -> Result<JsonValue,String> {
    let worker = resolve_frontend_recon_worker(&app)?;
    let db_path = state.db_path.clone();
    let app_data = state.app_data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || recover_web_attempt_impl(
        &db_path,&app_data,&scan_id,attempt_number,operator_confirmed,&worker,
    )).await.map_err(|_| "web_recovery_worker_unconfirmed_no_automatic_retry".to_string())?
}

fn recover_web_attempt_impl(
    db_path: &Path, app_data: &Path, scan_id: &str, attempt_number: i64,
    operator_confirmed: bool, worker: &Path,
) -> Result<JsonValue,String> {
    let OwnedWebRecovery { _lifecycle, guard, mut inputs } = claim_web_recovery(
        db_path,scan_id,attempt_number,operator_confirmed,|connection|
            reconstruct_web_recovery_in(connection,app_data,scan_id,attempt_number,worker),
    )?;
    inputs.admission.claimed_guard = Some(guard);
    launch_sentinel_url_pipeline(db_path.into(),scan_id.into(),attempt_number,
        inputs.runtime.worker,inputs.work_dir,inputs.targets,inputs.runtime.proxies,inputs.runtime.no_proxy,
        inputs.model,inputs.runtime_path,inputs.runtime.adaptive,inputs.runtime.packet_budget,
        inputs.auth_session_path,Some(inputs.admission))?;
    Ok(serde_json::json!({"scanId":scan_id,"attemptNumber":attempt_number,
        "dispatchState":"claimed","executionState":"submitted","automaticReplayAllowed":false}))
}
