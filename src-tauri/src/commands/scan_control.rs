#[tauri::command]
pub fn start_strix_workbench_scan(
    state: State<AppState>,
    input: StrixWorkbenchInput,
) -> Result<SentinelScan, String> {
    start_strix_workbench_scan_impl(&state, input, None)
}

#[tauri::command]
pub fn rescan_strix_workbench_scan(
    state: State<AppState>,
    scan_id: String,
) -> Result<SentinelScan, String> {
    let connection = db::open(&state.db_path)?;
    let (project_id, task_name, scan_type, source_path, task_path, status): (i64, String, String, String, String, String) = connection.query_row(
        "SELECT project_id,task_name,scan_type,source_path,task_path,status FROM sentinel_scans WHERE id=?1 AND project_id IS NOT NULL",
        [&scan_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?))
    ).map_err(|_| "原工作台任务不存在或没有项目归属".to_string())?;
    if scan_type == "web" {
        return Err("Web 资产任务请使用原有再次扫描流程".into());
    }
    if matches!(status.as_str(), "scanning" | "pausing") {
        return Err("工作台任务仍在运行，请先暂停".into());
    }
    // Paused workbench tasks use the same in-place attempt ledger as a normal
    // retry. The dedicated resume command dispatches here after checking the
    // scan type; never send a source directory through the Web URL pipeline.
    let payload = fs::read_to_string(task_path)
        .ok()
        .and_then(|text| serde_json::from_str::<JsonValue>(&text).ok())
        .unwrap_or_default();
    let urls = payload
        .get("urls")
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(JsonValue::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let skill_names = payload
        .get("skills")
        .and_then(JsonValue::as_str)
        .unwrap_or("")
        .split('、')
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    let mut skill_ids = Vec::new();
    for name in skill_names {
        if let Ok(id) =
            connection.query_row("SELECT id FROM strix_skills WHERE name=?1", [name], |row| {
                row.get(0)
            })
        {
            skill_ids.push(id);
        }
    }
    let input = StrixWorkbenchInput {
        project_id,
        task_name,
        scan_type,
        urls,
        source_path,
        skill_ids,
        instruction: String::new(),
        scan_mode: payload
            .get("scanMode")
            .and_then(JsonValue::as_str)
            .unwrap_or("deep")
            .to_string(),
        scope_mode: payload
            .get("scopeMode")
            .and_then(JsonValue::as_str)
            .unwrap_or("full")
            .to_string(),
        diff_base: payload
            .get("diffBase")
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .to_string(),
        max_budget_usd: payload.get("maxBudgetUsd").and_then(JsonValue::as_f64),
        environment: value_first(&payload, &["environment"]),
        auth_profile_name: String::new(),
        auth_type: "none".into(),
        auth_header_name: String::new(),
        auth_value: String::new(),
        auth_session_id: value_first(&payload, &["authSessionId"]),
        auth_session_ids: investigation_strings(payload.get("authSessionIds")),
        auth_session_scope_id: String::new(),
        ci_provider: value_first(&payload, &["ciProvider"]),
        repository_url: value_first(&payload, &["repositoryUrl"]),
        branch: value_first(&payload, &["branch"]),
        commit_sha: value_first(&payload, &["commitSha"]),
        build_id: value_first(&payload, &["buildId"]),
        max_critical: payload
            .pointer("/policy/maxCritical")
            .and_then(JsonValue::as_i64)
            .unwrap_or(0),
        max_high: payload
            .pointer("/policy/maxHigh")
            .and_then(JsonValue::as_i64)
            .unwrap_or(5),
        block_release: payload
            .pointer("/policy/blockRelease")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false),
    };
    drop(connection);
    start_strix_workbench_scan_impl(&state, input, Some(scan_id))
}

#[tauri::command]
pub fn confirm_sentinel_scan(
    app: AppHandle,
    state: State<AppState>,
    scan_id: String,
) -> Result<SentinelScan, String> {
    let connection = db::open(&state.db_path)?;
    let (project_id, project_name, status): (Option<i64>, String, String) = connection
        .query_row(
            "SELECT project_id,project_name,status FROM sentinel_scans WHERE id=?1",
            [&scan_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|_| "任务不存在".to_string())?;
    if status != "draft" {
        return Err(format!("任务当前状态为 {}，不能重复确认", status));
    }
    let project_id = project_id.ok_or_else(|| "任务没有本地工作空间归属".to_string())?;
    let project_active: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1 AND status='active')",
            [project_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if !project_active {
        return Err("工作空间已归档或不存在；请先恢复工作空间再确认任务".into());
    }
    let _ = connection.execute(
        "UPDATE sentinel_targets SET status='fuse_excluded',routing_reason='该 URL 位于 Strix 熔断区；移出熔断区后才会恢复自动扫描',updated_at=datetime('now','localtime') WHERE scan_id=?1 AND EXISTS (SELECT 1 FROM sentinel_fuse_zone f WHERE f.project_id=sentinel_targets.project_id AND f.normalized_url=lower(rtrim(trim(sentinel_targets.url),'/')))",
        [&scan_id],
    );
    let mut stmt = connection
        .prepare(SENTINEL_RESUME_TARGETS_SQL)
        .map_err(|e| e.to_string())?;
    let targets = stmt
        .query_map([&scan_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(stmt);
    if targets.is_empty() {
        return Err("任务没有可扫描目标；URL 可能都在 Strix 熔断区".into());
    }
    let settings = sentinel_settings(&connection);
    let mut adaptive = AdaptiveStrixSettings::from_json(&settings);
    let stored_web_policy = connection
        .query_row(
            "SELECT policy_json FROM sentinel_scan_contexts WHERE scan_id=?1",
            [&scan_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .map(json)
        .unwrap_or_else(|| serde_json::json!({"webModeCeiling":"standard"}));
    let (web_policy, skill_names, skill_instructions) =
        effective_web_policy(&connection, &stored_web_policy, &settings)?;
    connection.execute(
        "INSERT INTO sentinel_scan_contexts(scan_id,environment,policy_json) VALUES(?1,'internal',?2) ON CONFLICT(scan_id) DO UPDATE SET policy_json=excluded.policy_json,updated_at=datetime('now','localtime')",
        params![scan_id, web_policy.to_string()],
    ).map_err(|error| error.to_string())?;
    adaptive.apply_web_policy(&web_policy);
    let mut auth_session_ids = investigation_strings(web_policy.get("authSessionIds"));
    let auth_session_id = web_policy.get("authSessionId").and_then(JsonValue::as_str).unwrap_or("").trim().to_string();
    if !auth_session_id.is_empty() {
        auth_session_ids.push(auth_session_id);
    }
    auth_session_ids.sort();
    auth_session_ids.dedup();
    let proxies = approved_strix_proxies(&settings);
    let no_proxy = settings
        .get("noProxy")
        .and_then(JsonValue::as_str)
        .unwrap_or("127.0.0.1,localhost")
        .to_string();
    let home = state
        .app_data_dir
        .parent()
        .unwrap_or(&state.app_data_dir)
        .to_path_buf();
    let strix = resolve_strix_executable(&settings, &home)?;
    let strix_cli = strix_cli_capabilities(&strix)?;
    let strix_environment = strix_runtime_env(&settings, &home)?;
    adaptive.apply_deployment(&strix_environment.deployment);
    let packet_budget = frontend_packet_budget(&settings, &strix_environment.deployment);
    let python = resolve_plain_python(&settings, &home)?;
    let worker = resolve_frontend_recon_worker(&app)?;
    let runtime_path = sentinel_runtime_path(&home);
    let docker = ensure_docker_ready(&home, &runtime_path)?;
    let (startup_idle_timeout, startup_hard_timeout) =
        strix_startup_timeouts(&strix_environment);
    let task_dir = state.app_data_dir.join("sentinel-tasks");
    fs::create_dir_all(&task_dir).map_err(|e| e.to_string())?;
    let task_path = task_dir.join(format!("{}.json", scan_id));
    // This file is an immutable execution plan. Runtime status and checkpoints
    // live only in sentinel_scans/sentinel_scan_attempts; duplicating them here
    // left every completed plan permanently saying `queued` and encouraged
    // accidental reuse of stale state during diagnostics.
    let payload = serde_json::json!({"scanId":scan_id,"projectId":project_id,"projectName":project_name,"targets":targets.iter().map(|(company,url)|serde_json::json!({"company":company,"url":url})).collect::<Vec<_>>(),"frontendReconStrategy":"coverage-led-browser-exploration+evidence-validation","strixQueueOrder":"fifo","effectiveWebPolicy":web_policy.clone(),"skills":skill_names.clone(),"adaptiveRouting":{"enabled":true,"forcedMode":"coverage-led","modeCeiling":adaptive.max_mode.clone(),"maxBudgetUsd":adaptive.max_budget_usd,"quickScore":adaptive.quick_score,"standardScore":adaptive.standard_score,"deepScore":adaptive.deep_score,"quickTimeout":adaptive.quick_timeout,"standardTimeout":adaptive.standard_timeout,"deepTimeout":adaptive.deep_timeout,"quickTokenLimit":adaptive.quick_tokens,"standardTokenLimit":adaptive.standard_tokens,"deepTokenLimit":adaptive.deep_tokens,"quickRequestLimit":adaptive.quick_requests,"standardRequestLimit":adaptive.standard_requests,"deepRequestLimit":adaptive.deep_requests,"noToolTurnLimit":adaptive.no_tool_turn_limit,"startupIdleTimeout":startup_idle_timeout,"startupHardTimeout":startup_hard_timeout},"llmPolicy":{"model":strix_environment.llm,"deployment":strix_environment.deployment,"fullPower":strix_environment.full_power,"promptAuditMode":strix_environment.prompt_audit_mode},"runtimePolicy":strix_runtime_policy(&strix_cli,&strix_environment.image),"authorizedProxyPool":!proxies.is_empty(),"createdAt":chrono::Utc::now().to_rfc3339()});
    fs::write(
        &task_path,
        serde_json::to_vec_pretty(&payload).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let scan_work_root = state.app_data_dir.join("strix-jobs").join(&scan_id);
    fs::create_dir_all(&scan_work_root).map_err(|error| error.to_string())?;
    fs::write(scan_work_root.join(".oviraptor-scan-id"), &scan_id)
        .map_err(|error| error.to_string())?;
    let minimum_attempt = connection
        .query_row(
            "SELECT attempt_count FROM sentinel_scans WHERE id=?1",
            [&scan_id],
            |row| row.get::<_, u32>(0),
        )
        .unwrap_or(0)
        .saturating_add(1);
    let work_dir = next_scan_attempt_work_dir(&scan_work_root, minimum_attempt)?;
    fs::write(work_dir.join(".oviraptor-scan-id"), &scan_id).map_err(|error| error.to_string())?;
    let attempt_number = scan_attempt_number(&work_dir);
    let auth_session_path = if auth_session_ids.is_empty() {
        None
    } else {
        let mut documents = crate::auth_session::distinct_session_documents_for_scan(
            &connection,
            &auth_session_ids,
            project_id,
        )?;
        let document = if documents.len() == 1 {
            documents.remove(0)
        } else {
            serde_json::json!({
                "schemaVersion": 2,
                "kind": "identity-matrix",
                "sessions": documents,
                "comparisonPolicy": "same-target-same-action-plan",
                "identityIsolation": "dedicated-webview-and-distinct-auth-material"
            })
        };
        let path = work_dir.join("auth-sessions.json");
        crate::auth_session::write_session_document(&path, &document)?;
        Some(path)
    };
    let targets_json = work_dir.join("targets.json");
    fs::write(
        &targets_json,
        serde_json::to_vec_pretty(
            &targets
                .iter()
                .map(|(company, url)| serde_json::json!({"company":company,"url":url}))
                .collect::<Vec<_>>(),
        )
        .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let targets_txt = work_dir.join("targets.txt");
    fs::write(
        &targets_txt,
        targets
            .iter()
            .map(|(_, url)| url.replace(['\r', '\n'], ""))
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .map_err(|error| error.to_string())?;
    let instruction_path = work_dir.join("strix-instruction.md");
    let instruction = render_web_investigation_instruction(
        &web_policy,
        &skill_instructions,
        strix_environment.deployment == "local",
    );
    fs::write(&instruction_path, &instruction).map_err(|error| error.to_string())?;
    write_strix_prompt_audit(&work_dir, &instruction, &strix_environment)?;
    connection.execute(
        "UPDATE sentinel_scans SET status='scanning',current_checkpoint=?1,task_path=?2,skill_names=?3,attempt_count=?4,updated_at=datetime('now','localtime') WHERE id=?5",
        params![if strix_environment.full_power { format!("第 {attempt_number} 次执行：{} 个 URL；逐 URL 前端探测后进入 Strix；本地火力全开仅放宽普通 Web，现代前端仍执行定向验证硬上限",targets.len()) } else { format!("第 {attempt_number} 次执行：{} 个 URL；逐 URL 探测后立即进入 Strix FIFO 队列",targets.len()) },task_path.to_string_lossy(), skill_names, attempt_number, scan_id],
    ).map_err(|e| e.to_string())?;
    for (_, url) in &targets {
        connection
            .execute(
                "UPDATE sentinel_targets SET last_attempt_number=?1 WHERE scan_id=?2 AND url=?3",
                params![attempt_number, scan_id, url],
            )
            .map_err(|error| error.to_string())?;
    }
    record_sentinel_attempt_start(&connection, &scan_id, attempt_number as i64, &work_dir)?;
    // Move the previous attempt's model/result checkpoints out of the live
    // result surface before the worker starts. Frontend reconnaissance and
    // investigation evidence remain reusable; old Strix errors/results stay
    // available through the immutable attempt ledger and work directory.
    prepare_latest_strix_attempt(&connection, &scan_id, attempt_number as i64)?;
    let result = sentinel_scan_by_id(&connection, &scan_id)?;
    launch_sentinel_url_pipeline(
        state.db_path.clone(),
        scan_id,
        python,
        worker,
        strix,
        docker,
        work_dir,
        targets,
        instruction_path,
        proxies,
        no_proxy,
        strix_environment,
        runtime_path,
        adaptive,
        packet_budget,
        auth_session_path,
    );
    Ok(result)
}

#[tauri::command]
pub fn pause_sentinel_scan(
    state: State<AppState>,
    scan_id: String,
) -> Result<SentinelScan, String> {
    let connection = db::open(&state.db_path)?;
    let status: String = connection
        .query_row(
            "SELECT status FROM sentinel_scans WHERE id=?1",
            [&scan_id],
            |row| row.get(0),
        )
        .map_err(|_| "任务不存在".to_string())?;
    if !["scanning", "pausing"].contains(&status.as_str()) {
        return Err(format!("任务当前状态为 {status}，不能请求暂停"));
    }
    if status == "scanning" {
        connection
            .execute(
                "UPDATE sentinel_scans SET status='pausing',current_checkpoint='暂停请求已接收；正在停止当前 URL，已写入结果会保留',updated_at=datetime('now','localtime') WHERE id=?1",
                [&scan_id],
            )
            .map_err(|error| error.to_string())?;
    }
    // Stop the active worker immediately; the worker's polling path still
    // performs its normal cleanup and persists any artifacts already written.
    let runner_log = state
        .app_data_dir
        .join("strix-jobs")
        .join(&scan_id)
        .join("oviraptor-runner.log");
    append_runner_log(
        &runner_log,
        "pause requested; stopping registered URL workers",
    );
    let stopped = force_stop_registered_sentinel_processes(&state.db_path, &scan_id);
    append_runner_log(
        &runner_log,
        &format!(
            "pause stop signal sent to {} process(es): {:?}",
            stopped.len(),
            stopped
        ),
    );
    finish_sentinel_pause(
        &state.db_path,
        &scan_id,
        "已暂停；当前 URL 的前端解析与 Strix 测试均已停止，恢复后从该 URL 重新进入队列",
    );
    append_runner_log(
        &runner_log,
        "pipeline state is paused; active URL workers stopped and queued URLs retained",
    );
    sentinel_scan_by_id(&connection, &scan_id)
}

#[tauri::command]
pub fn resume_sentinel_scan(
    app: AppHandle,
    state: State<AppState>,
    scan_id: String,
) -> Result<SentinelScan, String> {
    let db_path = state.db_path.clone();
    let connection = db::open(&db_path)?;
    let (status, scan_type): (String, String) = connection
        .query_row(
            "SELECT status,scan_type FROM sentinel_scans WHERE id=?1",
            [&scan_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| "任务不存在".to_string())?;
    if status != "paused" {
        return Err(format!("任务当前状态为 {status}，不能恢复"));
    }
    if scan_type != "web" {
        drop(connection);
        return rescan_strix_workbench_scan(state, scan_id);
    }
    let remaining: i64 = connection
        .query_row(SENTINEL_RESUME_COUNT_SQL, [&scan_id], |row| row.get(0))
        .unwrap_or(0);
    if remaining == 0 {
        return Err("没有尚未完成的 URL 可恢复".into());
    }
    connection
        .execute(
            "UPDATE sentinel_scans SET status='draft',current_checkpoint=?1,updated_at=datetime('now','localtime') WHERE id=?2",
            params![format!("准备恢复剩余 {remaining} 个 URL"), scan_id],
        )
        .map_err(|error| error.to_string())?;
    connection
        .execute(
            "INSERT INTO app_settings(key,value) VALUES(?1,'resume') ON CONFLICT(key) DO UPDATE SET value='resume'",
            [format!("sentinel-next-attempt-mode:{scan_id}")],
        )
        .map_err(|error| error.to_string())?;
    drop(connection);
    match confirm_sentinel_scan(app, state, scan_id.clone()) {
        Ok(scan) => Ok(scan),
        Err(error) => {
            let _ = db::open(&db_path).and_then(|connection| {
                connection
                    .execute(
                        "UPDATE sentinel_scans SET status='paused',current_checkpoint=?1,updated_at=datetime('now','localtime') WHERE id=?2",
                        params![format!("恢复失败：{error}"), scan_id],
                    )
                    .map(|_| ())
                    .map_err(|db_error| db_error.to_string())
                    .and_then(|_| {
                        connection
                            .execute(
                                "DELETE FROM app_settings WHERE key=?1",
                                [format!("sentinel-next-attempt-mode:{scan_id}")],
                            )
                            .map(|_| ())
                            .map_err(|db_error| db_error.to_string())
                    })
            });
            Err(error)
        }
    }
}

#[tauri::command]
pub fn cancel_sentinel_scan(state: State<AppState>, scan_id: String) -> Result<(), String> {
    let connection = db::open(&state.db_path)?;
    let (status, path): (String, String) = connection
        .query_row(
            "SELECT status,task_path FROM sentinel_scans WHERE id=?1",
            [&scan_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| "任务不存在".to_string())?;
    if status != "draft" {
        return Err("只有未确认任务可以删除；已确认任务请保留审计记录".into());
    }
    if !path.is_empty() {
        let _ = fs::remove_file(path);
    }
    connection
        .execute(
            "DELETE FROM browser_auth_sessions WHERE owner_scan_id=?1",
            [&scan_id],
        )
        .map_err(|error| error.to_string())?;
    connection
        .execute("DELETE FROM sentinel_scans WHERE id=?1", [&scan_id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn request_stop_sentinel_process(process_id: i64) {
    if process_id <= 0 {
        return;
    }
    if cfg!(target_os = "windows") {
        let _ = Command::new("taskkill")
            .args(["/PID", &process_id.to_string(), "/T"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    } else {
        let pid = process_id.to_string();
        let process_group = format!("-{pid}");
        let _ = Command::new("kill")
            .args(["-TERM", "--", &process_group])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("pkill")
            .args(["-TERM", "-P", &pid])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("kill")
            .args(["-TERM", &pid])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn force_stop_sentinel_process(process_id: i64) {
    if process_id <= 0 {
        return;
    }
    request_stop_sentinel_process(process_id);
    thread::sleep(Duration::from_millis(250));
    if cfg!(target_os = "windows") {
        let _ = Command::new("taskkill")
            .args(["/PID", &process_id.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    } else {
        let pid = process_id.to_string();
        let process_group = format!("-{pid}");
        let _ = Command::new("kill")
            .args(["-KILL", "--", &process_group])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("pkill")
            .args(["-KILL", "-P", &pid])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("kill")
            .args(["-KILL", &pid])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn graceful_stop_sentinel_process(child: &mut std::process::Child, process_id: i64) {
    request_stop_sentinel_process(process_id);
    let deadline = Instant::now() + Duration::from_secs(4);
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Err(_) => return,
            Ok(None) => thread::sleep(Duration::from_millis(100)),
        }
    }
    force_stop_sentinel_process(process_id);
    let _ = child.wait();
}

#[tauri::command]
pub fn delete_sentinel_scan(state: State<AppState>, scan_id: String) -> Result<(), String> {
    if scan_id.contains('/') || scan_id.contains('\\') || scan_id.contains("..") {
        return Err("任务 ID 非法".into());
    }
    let connection = db::open(&state.db_path)?;
    let (_status, task_path): (String, String) = connection
        .query_row(
            "SELECT status,task_path FROM sentinel_scans WHERE id=?1",
            [&scan_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| "任务不存在".to_string())?;
    // 先写删除标记，避免 Agent 结果目录暂时被占用时，下次同步又把任务复活。
    connection
        .execute(
            "INSERT INTO sentinel_deleted_scans(scan_id) VALUES(?1) ON CONFLICT(scan_id) DO UPDATE SET deleted_at=datetime('now','localtime')",
            [&scan_id],
        )
        .map_err(|error| error.to_string())?;
    let process_ids = {
        let mut statement = connection
            .prepare("SELECT process_id FROM sentinel_processes WHERE scan_id=?1")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([&scan_id], |row| row.get::<_, i64>(0))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        rows
    };
    for process_id in process_ids {
        force_stop_sentinel_process(process_id);
    }
    let mut cleanup_warnings = Vec::new();
    if !task_path.trim().is_empty() {
        let path = std::path::PathBuf::from(&task_path);
        if path.is_file() {
            if let Err(error) = fs::remove_file(&path) {
                cleanup_warnings.push(format!("{}：{}", path.display(), error));
            }
        }
    }
    let default_task_path = state
        .app_data_dir
        .join("sentinel-tasks")
        .join(format!("{}.json", scan_id));
    if default_task_path.is_file() {
        if let Err(error) = fs::remove_file(&default_task_path) {
            cleanup_warnings.push(format!("{}：{}", default_task_path.display(), error));
        }
    }
    let result_dir = state
        .app_data_dir
        .parent()
        .unwrap_or(&state.app_data_dir)
        .join(".trae-cn/scan-results")
        .join(&scan_id);
    if result_dir.is_dir() {
        if let Err(error) = fs::remove_dir_all(&result_dir) {
            cleanup_warnings.push(format!("{}：{}", result_dir.display(), error));
        }
    }
    let strix_job_dir = state.app_data_dir.join("strix-jobs").join(&scan_id);
    if strix_job_dir.is_dir() {
        if let Err(error) = fs::remove_dir_all(&strix_job_dir) {
            cleanup_warnings.push(format!("{}：{}", strix_job_dir.display(), error));
        }
    }
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    // Historical releases modeled retries as child scans. Those children own
    // copied targets/checkpoints and must remain standalone if the old parent
    // is deleted; otherwise their next retry points at a missing scan.
    transaction
        .execute(
            "UPDATE sentinel_scans SET previous_scan_id='' WHERE previous_scan_id=?1",
            [&scan_id],
        )
        .map_err(|error| error.to_string())?;
    // 显式清理，兼容旧数据库中没有 ON DELETE CASCADE 的表结构。
    for table in [
        "sentinel_validations",
        "sentinel_opportunities",
        "sentinel_findings",
        "sentinel_checkpoints",
        "sentinel_targets",
        "sentinel_processes",
    ] {
        transaction
            .execute(&format!("DELETE FROM {table} WHERE scan_id=?1"), [&scan_id])
            .map_err(|error| error.to_string())?;
    }
    transaction
        .execute(
            "DELETE FROM browser_auth_sessions WHERE owner_scan_id=?1",
            [&scan_id],
        )
        .map_err(|error| error.to_string())?;
    let deleted = transaction
        .execute("DELETE FROM sentinel_scans WHERE id=?1", [&scan_id])
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    if deleted == 0 {
        return Err("任务不存在或已经删除".into());
    }
    // 文件占用不会阻断数据库删除；删除标记会阻止残留目录再次入库。
    if !cleanup_warnings.is_empty() {
        eprintln!(
            "Sentinel 删除完成，以下残留文件待系统释放后清理：{}",
            cleanup_warnings.join("；")
        );
    }
    Ok(())
}

pub(crate) fn list_sentinel_scans_inner(
    db_path: &Path,
    project_id: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<SentinelScan>, String> {
    let connection = db::open(db_path)?;
    let mut s = connection
        .prepare(&format!(
            "SELECT {SENTINEL_SCAN_COLUMNS} FROM sentinel_scans WHERE (?1 IS NULL OR project_id=?1) ORDER BY updated_at DESC,id DESC LIMIT ?2"
        ))
        .map_err(|e| e.to_string())?;
    let rows = s
        .query_map(
            params![project_id, limit.unwrap_or(300).clamp(20, 2000)],
            sentinel_scan_row,
        )
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string());
    rows
}

#[tauri::command]
pub async fn list_sentinel_scans(
    state: State<'_, AppState>,
    project_id: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<SentinelScan>, String> {
    let db_path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        list_sentinel_scans_inner(&db_path, project_id, limit)
    })
    .await
    .map_err(|error| format!("Strix 任务列表读取线程失败：{error}"))?
}

#[tauri::command]
pub fn list_sentinel_scan_attempts(
    state: State<AppState>,
    scan_id: String,
) -> Result<Vec<SentinelScanAttempt>, String> {
    let connection = db::open(&state.db_path)?;
    sync_sentinel_attempt(&connection, &scan_id);
    let mut statement = connection
        .prepare(
            "SELECT scan_id,attempt_number,execution_mode,status,stage,checkpoint,stop_reason,work_dir,llm_requests_delta,input_tokens_delta,output_tokens_delta,cached_tokens_delta,total_tokens_delta,started_at,finished_at,updated_at FROM sentinel_scan_attempts WHERE scan_id=?1 ORDER BY attempt_number DESC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([scan_id], |row| {
            Ok(SentinelScanAttempt {
                scan_id: row.get(0)?,
                attempt_number: row.get(1)?,
                execution_mode: row.get(2)?,
                status: row.get(3)?,
                stage: row.get(4)?,
                checkpoint: row.get(5)?,
                stop_reason: row.get(6)?,
                work_dir: row.get(7)?,
                llm_requests: row.get(8)?,
                input_tokens: row.get(9)?,
                output_tokens: row.get(10)?,
                cached_tokens: row.get(11)?,
                total_tokens: row.get(12)?,
                started_at: row.get(13)?,
                finished_at: row.get(14)?,
                updated_at: row.get(15)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

#[tauri::command]
pub async fn list_sentinel_vulnerability_scan_ids(
    state: State<'_, AppState>,
    project_id: Option<i64>,
) -> Result<Vec<String>, String> {
    let connection = db::open(&state.db_path)?;
    let mut statement = connection
        .prepare(
            "SELECT DISTINCT f.scan_id FROM sentinel_findings f JOIN sentinel_scans s ON s.id=f.scan_id WHERE f.kind='vulnerability' AND (?1 IS NULL OR s.project_id=?1) ORDER BY s.updated_at DESC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([project_id], |row| row.get(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

fn get_sentinel_runner_log_inner(
    db_path: &Path,
    app_data_dir: &Path,
    scan_id: String,
    limit: Option<usize>,
) -> Result<Vec<String>, String> {
    let scan_id = scan_id.trim();
    if scan_id.is_empty()
        || scan_id
            .chars()
            .any(|character| matches!(character, '/' | '\\' | ':'))
        || scan_id.contains("..")
    {
        return Err("invalid scan id".into());
    }
    let connection = db::open(db_path)?;
    let exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1)",
            [scan_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if !exists {
        return Err("scan not found".into());
    }
    let path = app_data_dir
        .join("strix-jobs")
        .join(scan_id)
        .join("oviraptor-runner.log");
    Ok(strix_runner_log_tail(
        &path,
        limit.unwrap_or(300).clamp(1, 1000),
    ))
}

#[tauri::command]
pub async fn get_sentinel_runner_log(
    state: State<'_, AppState>,
    scan_id: String,
    limit: Option<usize>,
) -> Result<Vec<String>, String> {
    let db_path = state.db_path.clone();
    let app_data_dir = state.app_data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        get_sentinel_runner_log_inner(&db_path, &app_data_dir, scan_id, limit)
    })
    .await
    .map_err(|error| format!("Strix 日志读取线程失败：{error}"))?
}

fn search_sentinel_scan_ids_inner(db_path: &Path, search: String) -> Result<Vec<String>, String> {
    let needle = format!("%{}%", search.trim());
    if search.trim().is_empty() {
        return Ok(Vec::new());
    }
    let connection = db::open(db_path)?;
    let mut statement = connection
        .prepare(
            "SELECT DISTINCT s.id FROM sentinel_scans s LEFT JOIN sentinel_targets t ON t.scan_id=s.id LEFT JOIN sentinel_findings f ON f.scan_id=s.id WHERE s.project_name LIKE ?1 OR s.task_name LIKE ?1 OR s.source_path LIKE ?1 OR s.id LIKE ?1 OR t.company LIKE ?1 OR t.url LIKE ?1 OR f.target_url LIKE ?1 ORDER BY s.updated_at DESC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([needle], |row| row.get(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

#[tauri::command]
pub async fn search_sentinel_scan_ids(
    state: State<'_, AppState>,
    search: String,
) -> Result<Vec<String>, String> {
    let db_path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || search_sentinel_scan_ids_inner(&db_path, search))
        .await
        .map_err(|error| format!("Strix 搜索线程失败：{error}"))?
}

#[tauri::command]
pub async fn list_sentinel_targets(
    state: State<'_, AppState>,
    project_id: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<SentinelTarget>, String> {
    let connection = db::open(&state.db_path)?;
    // Aggregate URL history once. The previous correlated COUNT re-scanned the
    // entire target table for every row and degraded quadratically.
    let mut s = connection.prepare(
        "WITH filtered AS (
           SELECT *,lower(rtrim(trim(url),'/')) AS normalized_url
           FROM sentinel_targets WHERE (?1 IS NULL OR project_id=?1)
         ), history AS (
           SELECT project_id,normalized_url,COUNT(DISTINCT scan_id) AS scan_count
           FROM filtered GROUP BY project_id,normalized_url
         )
         SELECT t.id,t.project_id,t.scan_id,t.company,t.url,t.status,t.value_score,t.scan_mode,t.routing_reason,t.last_attempt_number,t.created_at,t.updated_at,COALESCE(h.scan_count,0)
         FROM filtered t LEFT JOIN history h ON h.project_id=t.project_id AND h.normalized_url=t.normalized_url
         ORDER BY t.updated_at DESC,t.id DESC LIMIT ?2"
    ).map_err(|e| e.to_string())?;
    let rows = s
        .query_map(
            params![project_id, limit.unwrap_or(5000).clamp(100, 20_000)],
            |r| {
                Ok(SentinelTarget {
                    id: r.get(0)?,
                    project_id: r.get(1)?,
                    scan_id: r.get(2)?,
                    company: r.get(3)?,
                    url: r.get(4)?,
                    status: r.get(5)?,
                    value_score: r.get(6)?,
                    scan_mode: r.get(7)?,
                    routing_reason: r.get(8)?,
                    last_attempt_number: r.get(9)?,
                    created_at: r.get(10)?,
                    updated_at: r.get(11)?,
                    scan_count: r.get(12)?,
                })
            },
        )
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string());
    rows
}

#[tauri::command]
pub fn list_sentinel_fuse_zone(
    state: State<AppState>,
    project_id: Option<i64>,
) -> Result<Vec<SentinelFuseEntry>, String> {
    let connection = db::open(&state.db_path)?;
    let mut statement = connection.prepare(
        "SELECT id,project_id,asset_id,company,url,source_scan_id,reason,verdict,note,evidence,archived,created_at,updated_at FROM sentinel_fuse_zone WHERE (?1 IS NULL OR project_id=?1) ORDER BY archived,updated_at DESC,id DESC",
    ).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([project_id], |row| {
            Ok(SentinelFuseEntry {
                id: row.get(0)?,
                project_id: row.get(1)?,
                asset_id: row.get(2)?,
                company: row.get(3)?,
                url: row.get(4)?,
                source_scan_id: row.get(5)?,
                reason: row.get(6)?,
                verdict: row.get(7)?,
                note: row.get(8)?,
                evidence: row.get(9)?,
                archived: row.get(10)?,
                created_at: row.get(11)?,
                updated_at: row.get(12)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

#[tauri::command]
pub fn save_sentinel_fuse_review(
    state: State<AppState>,
    input: SentinelFuseReviewInput,
) -> Result<(), String> {
    let verdict = input.verdict.trim();
    if ![
        "pending",
        "manual_verified",
        "needs_followup",
        "not_reproducible",
    ]
    .contains(&verdict)
    {
        return Err("熔断区人工结论无效".into());
    }
    let connection = db::open(&state.db_path)?;
    let changed = connection.execute(
        "UPDATE sentinel_fuse_zone SET verdict=?1,note=?2,evidence=?3,archived=?4,updated_at=datetime('now','localtime') WHERE id=?5",
        params![verdict, input.note.trim(), input.evidence.trim(), input.archived as i64, input.id],
    ).map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("熔断记录不存在".into());
    }
    Ok(())
}

#[tauri::command]
pub fn remove_sentinel_fuse_entry(
    app: AppHandle,
    state: State<AppState>,
    entry_id: i64,
) -> Result<SentinelScan, String> {
    let connection = db::open(&state.db_path)?;
    let (project_id, company, url, source_scan_id): (i64, String, String, String) = connection
        .query_row(
            "SELECT z.project_id,z.company,z.url,z.source_scan_id FROM sentinel_fuse_zone z JOIN projects p ON p.id=z.project_id AND p.status='active' WHERE z.id=?1",
            [entry_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|_| "熔断记录不存在，或工作空间已归档；请先恢复工作空间再重试".to_string())?;
    let (project_name, scan_type, status): (String, String, String) = connection
        .query_row(
            "SELECT project_name,scan_type,status FROM sentinel_scans WHERE id=?1",
            [&source_scan_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|_| "熔断来源任务不存在，无法自动重试".to_string())?;
    if scan_type != "web" {
        return Err("熔断 URL 只能回到原 Web 任务继续执行".into());
    }
    if matches!(status.as_str(), "scanning" | "pausing") {
        return Err("来源任务仍在运行，请先暂停后再移出熔断区".into());
    }
    if status == "paused" {
        return Err("来源任务已暂停，请先继续或结束该任务再移出熔断区".into());
    }
    // Keep compatibility with historical retry children that may still be
    // active, but never create another child. New retries always continue the
    // source task in place.
    let existing = connection
        .query_row(
            "SELECT s.id,s.status FROM sentinel_scans s JOIN sentinel_targets t ON t.scan_id=s.id WHERE s.previous_scan_id=?1 AND t.url=?2 AND s.status IN ('draft','queued','scanning','pausing') ORDER BY s.updated_at DESC LIMIT 1",
            params![source_scan_id, url],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if let Some((scan_id, status)) = existing {
        connection
            .execute("DELETE FROM sentinel_fuse_zone WHERE id=?1", [entry_id])
            .map_err(|error| error.to_string())?;
        let scan = sentinel_scan_by_id(&connection, &scan_id)?;
        drop(connection);
        return if status == "draft" {
            confirm_sentinel_scan(app, state, scan_id)
        } else {
            Ok(scan)
        };
    }
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    transaction
        .execute("DELETE FROM sentinel_fuse_zone WHERE id=?1", [entry_id])
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "INSERT INTO sentinel_targets(project_id,scan_id,company,url,status) VALUES(?1,?2,?3,?4,'queued') ON CONFLICT(project_id,scan_id,url) DO UPDATE SET company=excluded.company,status='queued',value_score=0,scan_mode='',routing_reason='',updated_at=datetime('now','localtime')",
            params![project_id, source_scan_id, company, url],
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "UPDATE sentinel_scans SET project_name=?1,status='draft',current_checkpoint='已移出熔断区；正在当前任务中复用前端证据并自动重试',previous_scan_id='',updated_at=datetime('now','localtime') WHERE id=?2",
            params![project_name, source_scan_id],
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "DELETE FROM sentinel_processes WHERE scan_id=?1",
            [&source_scan_id],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    drop(connection);
    confirm_sentinel_scan(app, state, source_scan_id)
}

#[tauri::command]
pub async fn list_sentinel_checkpoints(
    state: State<'_, AppState>,
    scan_id: String,
) -> Result<Vec<SentinelCheckpoint>, String> {
    let connection = db::open(&state.db_path)?;
    let mut stmt = connection.prepare("SELECT scan_id,url,stage,raw_json,updated_at FROM sentinel_checkpoints WHERE scan_id=?1 ORDER BY updated_at DESC").map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([scan_id], |r| {
            Ok(SentinelCheckpoint {
                scan_id: r.get(0)?,
                url: r.get(1)?,
                stage: r.get(2)?,
                raw_json: r.get(3)?,
                updated_at: r.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

#[tauri::command]
pub async fn list_sentinel_findings(
    state: State<'_, AppState>,
    scan_id: String,
    kind: Option<String>,
) -> Result<Vec<SentinelFinding>, String> {
    let connection = db::open(&state.db_path)?;
    let inventory_record: Option<String> = connection
        .query_row(
            "SELECT record_json FROM sentinel_findings WHERE scan_id=?1 AND kind='source_inventory' ORDER BY id DESC LIMIT 1",
            [&scan_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let inventory_needs_refresh = inventory_record
        .as_deref()
        .and_then(|record| serde_json::from_str::<JsonValue>(record).ok())
        .and_then(|record| record.get("lineStats").cloned())
        .is_none();
    if inventory_needs_refresh {
        let source: Option<(String, String)> = connection
            .query_row(
                "SELECT scan_type,source_path FROM sentinel_scans WHERE id=?1",
                [&scan_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if let Some((scan_type, source_path)) = source {
            if scan_type != "web" && Path::new(&source_path).is_dir() {
                insert_source_inventory(&connection, &scan_id, &source_path)?;
            }
        }
    }
    let mut stmt = connection.prepare("SELECT id,scan_id,target_url,stage,kind,record_key,title,severity,record_json,updated_at FROM sentinel_findings WHERE scan_id=?1 AND (?2 IS NULL OR kind=?2) ORDER BY stage,kind,id").map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![scan_id, kind], |r| {
            Ok(SentinelFinding {
                id: r.get(0)?,
                scan_id: r.get(1)?,
                target_url: r.get(2)?,
                stage: r.get(3)?,
                kind: r.get(4)?,
                record_key: r.get(5)?,
                title: r.get(6)?,
                severity: r.get(7)?,
                record_json: r.get(8)?,
                updated_at: r.get(9)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

#[tauri::command]
pub async fn list_sentinel_opportunities(
    state: State<'_, AppState>,
    project_id: Option<i64>,
    scan_id: Option<String>,
    status: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<SentinelOpportunity>, String> {
    let connection = db::open(&state.db_path)?;
    let limit = limit.unwrap_or(500).clamp(1, 5000);
    let query_limit = limit.saturating_mul(4).min(5000);
    let mut statement = connection.prepare(
        "SELECT id,project_id,scan_id,target_url,opportunity_key,category,title,score,status,confidence,why_json,evidence_json,recommended_action_json,source,record_json,first_seen,last_seen FROM sentinel_opportunities WHERE (?1 IS NULL OR project_id=?1) AND (?2 IS NULL OR scan_id=?2) AND (?3 IS NULL OR status=?3) ORDER BY CASE status WHEN 'ready' THEN 0 WHEN 'in_progress' THEN 1 WHEN 'queued' THEN 2 ELSE 3 END,score DESC,last_seen DESC,id DESC LIMIT ?4",
    ).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![project_id, scan_id, status, query_limit], |row| {
            Ok(SentinelOpportunity {
                id: row.get(0)?,
                project_id: row.get(1)?,
                scan_id: row.get(2)?,
                target_url: row.get(3)?,
                opportunity_key: row.get(4)?,
                category: row.get(5)?,
                title: row.get(6)?,
                score: row.get(7)?,
                status: row.get(8)?,
                confidence: row.get(9)?,
                why: json(row.get(10)?),
                evidence: json(row.get(11)?),
                recommended_action: json(row.get(12)?),
                source: row.get(13)?,
                record: json(row.get(14)?),
                first_seen: row.get(15)?,
                last_seen: row.get(16)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let mut grouped: Vec<SentinelOpportunity> = Vec::new();
    let mut indexes: HashMap<String, usize> = HashMap::new();
    for mut row in rows {
        let method = value_first(&row.record, &["method", "httpMethod"]).to_ascii_uppercase();
        let endpoint = value_first(
            &row.record,
            &["normalizedPath", "endpoint", "url", "path"],
        );
        let key = format!(
            "{}|{}|{}|{}|{}",
            row.scan_id,
            row.target_url,
            row.category.to_ascii_lowercase(),
            method,
            normalized_investigation_path(&endpoint)
        );
        if let Some(index) = indexes.get(&key).copied() {
            let current = &mut grouped[index];
            current.score = current.score.max(row.score);
            merge_opportunity_record(&mut current.record, &row.record);
            merge_json_array(&mut current.why, &row.why);
            merge_json_array(&mut current.evidence, &row.evidence);
            if opportunity_status_rank(&row.status) > opportunity_status_rank(&current.status) {
                current.status = std::mem::take(&mut row.status);
                current.id = row.id;
                current.opportunity_key = std::mem::take(&mut row.opportunity_key);
            }
        } else {
            indexes.insert(key, grouped.len());
            grouped.push(row);
        }
    }
    grouped.truncate(limit as usize);
    Ok(grouped)
}

fn opportunity_status_rank(status: &str) -> u8 {
    match status {
        "validated" => 7,
        "in_progress" => 6,
        "ready" => 5,
        "queued" => 4,
        "needs_more_evidence" => 3,
        "blocked_by_authorization" => 2,
        _ => 1,
    }
}

fn merge_json_array(target: &mut JsonValue, source: &JsonValue) {
    let mut values = target.as_array().cloned().unwrap_or_default();
    let mut seen = values
        .iter()
        .map(JsonValue::to_string)
        .collect::<HashSet<_>>();
    for item in source.as_array().into_iter().flatten() {
        if seen.insert(item.to_string()) {
            values.push(item.clone());
        }
    }
    *target = JsonValue::Array(values);
}

fn merge_opportunity_record(target: &mut JsonValue, source: &JsonValue) {
    let Some(target_object) = target.as_object_mut() else {
        return;
    };
    for field in ["identityKeys", "identityScopeKeys"] {
        let mut merged = target_object
            .get(field)
            .and_then(JsonValue::as_array)
            .cloned()
            .unwrap_or_default();
        let mut seen = merged
            .iter()
            .map(JsonValue::to_string)
            .collect::<HashSet<_>>();
        for value in source
            .get(field)
            .and_then(JsonValue::as_array)
            .into_iter()
            .flatten()
        {
            if seen.insert(value.to_string()) {
                merged.push(value.clone());
            }
        }
        target_object.insert(field.into(), JsonValue::Array(merged));
    }
    let mut runs = target_object
        .get("identityRuns")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    for incoming in source
        .get("identityRuns")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
    {
        let identity = value_first(incoming, &["identityKey"]);
        if let Some(existing) = runs
            .iter_mut()
            .find(|value| value_first(value, &["identityKey"]) == identity)
        {
            let existing_observed =
                existing.get("observed").and_then(JsonValue::as_bool) == Some(true);
            let incoming_observed =
                incoming.get("observed").and_then(JsonValue::as_bool) == Some(true);
            if incoming_observed && !existing_observed {
                *existing = incoming.clone();
            }
        } else {
            runs.push(incoming.clone());
        }
    }
    target_object.insert("identityRuns".into(), JsonValue::Array(runs));
}

#[tauri::command]
pub fn update_sentinel_opportunity_status(
    state: State<AppState>,
    opportunity_id: i64,
    status: String,
) -> Result<(), String> {
    let status = status.trim().to_ascii_lowercase();
    if ![
        "queued",
        "ready",
        "in_progress",
        "validated",
        "dismissed",
        "exhausted",
        "needs_more_evidence",
        "blocked_by_authorization",
        "closed",
    ]
    .contains(&status.as_str())
    {
        return Err("不支持的机会状态".into());
    }
    let connection = db::open(&state.db_path)?;
    let (record, scan_id, target_url, category) = connection
        .query_row(
            "SELECT record_json,scan_id,target_url,category FROM sentinel_opportunities WHERE id=?1",
            [opportunity_id],
            |row| {
                Ok((
                    json(row.get::<_, String>(0)?),
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .map_err(|_| "机会记录不存在".to_string())?;
    if matches!(status.as_str(), "ready" | "in_progress") {
        let (eligible, reason) = opportunity_agent_readiness(&record);
        if !eligible {
            return Err(format!(
                "该线索尚缺少可复现请求契约或新鲜响应，不能进入 Strix 验证队列：{reason}"
            ));
        }
    }
    let method = value_first(&record, &["method", "httpMethod"]).to_ascii_uppercase();
    let normalized_path = value_first(
        &record,
        &["normalizedPath", "endpoint", "url", "path"],
    );
    let normalized_path = normalized_investigation_path(&normalized_path).to_ascii_lowercase();
    let changed = connection
        .execute(
            "UPDATE sentinel_opportunities SET status=?1,last_seen=datetime('now','localtime') WHERE scan_id=?2 AND target_url=?3 AND lower(category)=lower(?4) AND upper(COALESCE(json_extract(record_json,'$.method'),''))=?5 AND lower(COALESCE(json_extract(record_json,'$.normalizedPath'),''))=?6",
            params![status, scan_id, target_url, category, method, normalized_path],
        )
        .map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("机会记录不存在".into());
    }
    // Keep terminal/manual opportunity actions visible in the investigation
    // graph and Action Center. Updating only the opportunity row made a card
    // appear to disappear without leaving an auditable next step.
    if matches!(status.as_str(), "validated" | "dismissed" | "exhausted" | "needs_more_evidence" | "blocked_by_authorization") {
        connection.execute(
            "INSERT INTO investigation_actions(project_id,scan_id,target_url,action_key,state_key,action_type,label,outcome,value_score,protocol_json) SELECT project_id,scan_id,target_url,?1,'opportunity-status','opportunity_follow_up',?2,?3,?4,?5 FROM sentinel_opportunities WHERE id=?6 ON CONFLICT(scan_id,target_url,action_key) DO UPDATE SET label=excluded.label,outcome=excluded.outcome,value_score=excluded.value_score,protocol_json=excluded.protocol_json,updated_at=datetime('now','localtime')",
            params![
                format!("opportunity-status:{}", opportunity_id),
                match status.as_str() {
                    "validated" => "验证已完成 · 查看结论与证据",
                    "dismissed" => "已排除 · 保留排除依据",
                    "exhausted" => "无新增证据 · 停止重复验证",
                    "needs_more_evidence" => "需要更多证据 · 继续同请求核查",
                    "blocked_by_authorization" => "旧授权停点 · 已交由自动策略收口",
                    _ => "机会状态已更新",
                },
                status.clone(),
                match status.as_str() { "validated" => 95, "needs_more_evidence" => 70, "blocked_by_authorization" => 40, _ => 20 },
                serde_json::json!({"opportunityId": opportunity_id, "status": status}).to_string(),
                opportunity_id,
            ],
        ).map_err(|error| error.to_string())?;
        connection.execute(
            "UPDATE sentinel_opportunities SET record_json=json_set(CASE WHEN json_valid(record_json) THEN record_json ELSE '{}' END,'$.lastOpportunityStatus',?1,'$.lastOpportunityStatusAt',datetime('now','localtime')) WHERE id=?2",
            params![status, opportunity_id],
        ).map_err(|error| error.to_string())?;
    }
    Ok(())
}
