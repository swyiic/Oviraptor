fn claim_web_pipeline_branch(
    db_path: &Path, scan_id: &str, attempt_number: i64, work_dir: &Path, worker: &Path,
    admission: &mut Option<WebDispatchAdmission>,
) -> Result<NativeBranchGuard,String> {
    if let Some(mut guard) = admission.as_mut().and_then(|frozen| frozen.claimed_guard.take()) {
        if guard.db_path != db_path || guard.scan_id != scan_id || guard.attempt != attempt_number || guard.branch != "web" {
            guard.disarm();
            return Err("web_recovery_owned_guard_mismatch".into());
        }
        Ok(guard)
    } else if let Some(frozen) = admission {
        NativeBranchGuard::claim_with_preflight(db_path,scan_id,attempt_number,"web", |connection| {
            verify_live_web_dispatch_binding_in(connection,scan_id,attempt_number,work_dir,worker,&frozen.home)
        })
    } else { claim_workbench_pipeline_branch(db_path,scan_id,attempt_number,"web") }
}

#[allow(clippy::too_many_arguments)]
fn launch_sentinel_url_pipeline(
    db_path: PathBuf,
    scan_id: String,
    attempt_number: i64,
    worker: PathBuf,
    work_dir: PathBuf,
    targets: Vec<(String, String)>,
    proxies: Vec<(String, String)>,
    no_proxy: String,
    model_environment: ModelRuntimeEnv,
    runtime_path: OsString,
    adaptive: AgentBudgetSettings,
    packet_budget: usize,
    auth_session_path: Option<PathBuf>,
    admission: Option<WebDispatchAdmission>,
    continuation_holder: Option<String>,
) -> Result<(), String> {
    launch_sentinel_url_pipeline_with_tool_configuration(db_path, scan_id, attempt_number, worker, work_dir, targets, proxies, no_proxy, model_environment, runtime_path, adaptive, packet_budget, auth_session_path, admission, None, continuation_holder)
}

#[allow(clippy::too_many_arguments)]
fn launch_sentinel_url_pipeline_with_tool_configuration(
    db_path: PathBuf,
    scan_id: String,
    attempt_number: i64,
    worker: PathBuf,
    work_dir: PathBuf,
    targets: Vec<(String, String)>,
    proxies: Vec<(String, String)>,
    no_proxy: String,
    model_environment: ModelRuntimeEnv,
    runtime_path: OsString,
    adaptive: AgentBudgetSettings,
    packet_budget: usize,
    auth_session_path: Option<PathBuf>,
    mut admission: Option<WebDispatchAdmission>,
    tool_configuration: Option<crate::tool_supply::ToolAdministratorConfiguration>,
    continuation_holder: Option<String>,
) -> Result<(), String> {
    thread::Builder::new().name("native-web-pipeline".into()).spawn(move || {
        let _continuation_guard = continuation_holder.as_deref().map(|holder| {
            bind_continuation_write_holder(&scan_id, attempt_number, holder);
            ContinuationWriteGuard
        });
        let claim = claim_web_pipeline_branch(&db_path,&scan_id,attempt_number,&work_dir,&worker,&mut admission);
        let mut branch_guard = match claim {
            Ok(guard) => guard,
            Err(error) => {
                append_runner_log(&work_dir.join("oviraptor-runner.log"), &format!("Web 分支未取得执行权，不启动；未领取的工作台分支按准入结果登记，其他状态不改写：{error}"));
                restore_unlaunched_continuation(&db_path, &scan_id, attempt_number);
                return;
            }
        };
        let (settings, recon_config) = match admission {
            Some(frozen) => (frozen.settings, frozen.recon_config),
            None => match db::open(&db_path).and_then(|connection|agent_policy_settings_for_attempt(&connection,&scan_id,attempt_number)) {
                Ok(settings)=>(settings,frontend_recon_config(&worker)),
                Err(error)=>{
                    append_runner_log(&work_dir.join("oviraptor-runner.log"),&format!("冻结策略不可验证，Web 分支不启动：{error}"));
                    restore_unlaunched_continuation(&db_path, &scan_id, attempt_number);
                    return;
                }
            },
        };
        if !sentinel_scan_is_active(&db_path, &scan_id) {
            restore_unlaunched_continuation(&db_path, &scan_id, attempt_number);
            return;
        }
        let log_path = work_dir.join("oviraptor-runner.log");
        let total_targets = targets.len();
        // Tier 1 deterministic exposure probe (git leak, .env, backups, ...). Runs
        // per target regardless of the model/value gate, so basic exposures are
        // found and recorded as findings even when the page itself exposes no API
        // surface (docs/AGENT_NEST_INTELLIGENT_FLOW_DESIGN.md §10b).
        {
            let checks = crate::scanning::exposure_probe::builtin_checks();
            for (target_url, _company) in &targets {
                let hits = crate::scanning::exposure_probe::probe_exposures(target_url, &checks);
                if !hits.is_empty() {
                    let recorded = crate::scanning::exposure_probe::record_exposures(
                        &db_path, &scan_id, target_url, &hits,
                    );
                    append_runner_log(
                        &log_path,
                        &format!(
                            "暴露探测：{target_url} 命中 {} 项，记入发现 {recorded} 条（{}）",
                            hits.len(),
                            hits.iter()
                                .map(|hit| hit.id.as_str())
                                .collect::<Vec<_>>()
                                .join(",")
                        ),
                    );
                }
            }
        }
        let model_policy = local_model_runtime_policy(&model_environment);
        let packet_budget = if model_environment.deployment == "local" {
            packet_budget.min(model_policy.frontend_packet_budget_bytes)
        } else {
            packet_budget
        };
        if model_environment.deployment == "local" {
            match apply_omlx_local_resource_policy(&model_environment) {
                Ok(Some(summary)) => append_runner_log(&log_path, &summary),
                Ok(None) => append_runner_log(
                    &log_path,
                    "local model resource policy: non-oMLX endpoint; Oviraptor request limits still apply",
                ),
                Err(error) => append_runner_log(
                    &log_path,
                    &format!("oMLX resource policy persisted with live-apply warning: {error}"),
                ),
            }
        }
        append_runner_log(
            &log_path,
            &format!(
                "frontend packet policy: {} KB total budget; evidence, parameters, sensitive clues, and code slices are priority-compacted",
                packet_budget / 1024
            ),
        );
        let producer = launch_frontend_recon_producer(
            db_path.clone(),
            scan_id.clone(),
            attempt_number,
            worker,
            work_dir.clone(),
            targets,
            proxies,
            no_proxy.clone(),
            model_environment.full_power,
            model_environment.deployment == "local",
            runtime_path.clone(),
            adaptive.clone(),
            packet_budget,
            log_path.clone(),
            auth_session_path,
            recon_config,
        );
        let (receiver, frontend_ack) = match producer {
            Ok(producer) => producer,
            Err(error) => {
                append_runner_log(&log_path,&format!("前端线程未取得执行权：{error}"));
                restore_unlaunched_continuation(&db_path, &scan_id, attempt_number);
                return;
            }
        };
        // Local context/memory admission errors retain the completed frontend
        // evidence and remain retryable after the automatic resource policy is
        // adjusted. They are partial results, not target execution failures.
        append_runner_log(&log_path, "agent backend: native (retired backends are read-only)");
        let mut tally = AgentPipelineTally::default();
        for item in receiver {
            if continuation_write_refused(&db_path, &scan_id, attempt_number) {
                append_runner_log(&log_path, "续跑执行权已过期，停止写入状态和扣费");
                return;
            }
            if let Ok(connection) = db::open(&db_path) {
                if let Ok(Some(kind)) = task_binding::claim_queued_execution_control(&connection, &scan_id, attempt_number) {
                    append_runner_log(&log_path, &format!("执行控制通道已领取 {kind}，停止继续写入，不删除任务，不新建额度"));
                    if matches!(kind.as_str(), "pause" | "cancel") {
                        let _ = request_sentinel_pause(&db_path, &scan_id);
                    }
                    return;
                }
                let _ = task_binding::deliver_supplements_for_scan(&connection, &scan_id);
            }
            if !native_web_attempt_current(&db_path, &scan_id, attempt_number) { return; }
            if sentinel_scan_pause_requested(&db_path, &scan_id) {
                append_runner_log(
                    &log_path,
                    "pipeline pause checkpoint reached; waiting for actual worker exit",
                );
                append_runner_log(
                    &log_path,
                    "pause requested; waiting for owned workers to exit; no automatic replay",
                );
                return;
            }
            if !sentinel_scan_is_active(&db_path, &scan_id) {
                return;
            }
            let prepared = match item {
                FrontendQueueItem::Ready(prepared) => prepared,
                FrontendQueueItem::Limited {
                    position,
                    url,
                    reason,
                } => {
                    tally.limited += 1;
                    update_batch_targets(&db_path, &scan_id, std::slice::from_ref(&url), "limited");
                    sentinel_scan_update(
                        &db_path,
                        &scan_id,
                        "scanning",
                        &format!("目标 {position}/{total_targets} · 自动停止 · {reason}"),
                    );
                    continue;
                }
                FrontendQueueItem::Failed {
                    position,
                    url,
                    reason,
                } => {
                    tally.failed += 1;
                    // A recon timeout is unfinished work, not a closed failure.
                    // Keep it resumable so 继续 only reruns this URL.
                    let resumable = reason.contains("硬上限") || reason.contains("时限") || reason.contains("晚到结果");
                    let stored_status = if resumable { "paused" } else { "failed" };
                    if let Ok(connection) = db::open(&db_path) {
                        if authorize_bound_continuation_write(&connection, &scan_id, attempt_number).is_err() {
                            append_runner_log(&log_path, "续跑执行权已过期，停止写入状态和扣费");
                            return;
                        }
                        let _ = connection.execute(
                            "UPDATE sentinel_targets SET status=?1,routing_reason=?2,updated_at=datetime('now','localtime') WHERE scan_id=?3 AND url=?4",
                            params![stored_status, reason, scan_id, url],
                        );
                    }
                    tally.push_detail(format!("{url}：{reason}"));
                    sentinel_scan_update(
                        &db_path,
                        &scan_id,
                        "scanning",
                        &format!("目标 {position}/{total_targets} · 前端探测失败 · {reason}"),
                    );
                    continue;
                }
            };
            // In local-model mode this acknowledges the prepared URL only after
            // its Native Agent work (or routing skip) has finished. The producer then
            // starts the next browser/AST pass without competing for CPU.
            let _frontend_ack = FrontendQueueAck(frontend_ack.as_ref());
            let position = prepared.position;
            let route = &prepared.route;
            if !record_agent_routing_skip(
                &db_path,
                &scan_id,
                route,
                position,
                total_targets,
                &mut tally,
            ) {
                continue;
            }
            // Plan selection/writes belong inside target admission. A repeated
            // target must not change the winner's frozen plan before it owns it.
            let dispatched = run_agent_target_with_tool_configuration(
                &prepared,
                AgentTargetExecution {
                    db_path: &db_path,
                    scan_id: &scan_id,
                    attempt_number,
                    settings: &settings,
                    environment: &model_environment,
                    adaptive: &adaptive,
                    log_path: &log_path,
                },
                tool_configuration.as_ref(),
            );
            let dispatched = match dispatched {
                Ok(dispatched) => dispatched,
                Err(error) => {
                    append_runner_log(&log_path, &format!("目标未取得执行权，不改写已有执行或终态，需检查当前执行记录：{error}"));
                    branch_guard.disarm();
                    return;
                }
            };
            if !native_web_attempt_current(&db_path, &scan_id, attempt_number) { return; }
            if !record_owned_agent_target_outcome(&db_path, &scan_id, route, &dispatched, &mut tally) {
                return;
            }
            if sentinel_scan_pause_requested(&db_path, &scan_id) {
                return;
            }
        }
        if continuation_write_refused(&db_path, &scan_id, attempt_number) {
            append_runner_log(&log_path, "续跑执行权已过期，停止写入终态和扣费");
            return;
        }
        if !native_web_attempt_current(&db_path, &scan_id, attempt_number) { return; }
        if sentinel_scan_pause_requested(&db_path, &scan_id) {
            return;
        }
        let (status, summary) = tally.terminal_summary(total_targets);
        if let Err(error) = finish_native_branch(&db_path, &scan_id, attempt_number, "web", status, &summary, &json!({"targetCount": total_targets})) {
            append_runner_log(&log_path, &format!("Web 分支终态未写入：{error}"));
        }
        if scan_supports_automatic_learning(&db_path, &scan_id) {
            schedule_learning_candidate(
                db_path.clone(),
                scan_id.clone(),
                model_environment.clone(),
            );
        }
    }).map(|_| ()).map_err(|_| "web_dispatch_thread_start_failed_no_automatic_retry".into())
}

/// Continue queued URLs on the original attempt. Refuses before any status
/// change when the claim ledger, balance, artifacts, worker, or model is unfit.
/// A spawned pipeline uses the saved attempt and does not insert a new allowance.
pub(crate) fn begin_remaining_target_continuation(db_path: &Path, scan_id: &str) -> Result<serde_json::Value, String> {
    let connection = db::open(db_path)?;
    recover_unclaimed_continuation(&connection, scan_id)?;
    let (status, scan_type, attempt): (String, String, i64) = connection.query_row(
        "SELECT status, scan_type, attempt_count FROM sentinel_scans WHERE id=?1",
        [scan_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).map_err(|_| "scan_task_unavailable".to_string())?;
    if scan_type != "web" { return Err("scan_channel_web_only".into()); }
    if !matches!(status.as_str(), "paused" | "partial") {
        return Err(format!("任务当前状态为 {status}，不能续跑剩余目标"));
    }
    let unknown: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_http_request_claims WHERE scan_id=?1 AND attempt_number=?2 AND received_at=''",
        params![scan_id, attempt], |row| row.get(0),
    ).map_err(|_| "claim_ledger_unchecked".to_string())?;
    if unknown > 0 { return Err("unknown_request_not_replayable".into()); }
    let targets = remaining_continuation_targets(&connection, scan_id)?;
    if targets.is_empty() {
        let manual: i64 = connection.query_row(
            "SELECT COUNT(*) FROM sentinel_targets WHERE scan_id=?1 AND status='manual_review'",
            [scan_id], |row| row.get(0),
        ).map_err(|_| "claim_ledger_unchecked".to_string())?;
        if manual > 0 { return Err("continuation_needs_manual_review".into()); }
        return Err("no_remaining_targets".into());
    }
    if let Ok(root) = connection.query_row(
        "SELECT id FROM agent_runs WHERE scan_id=?1 AND attempt_number=?2 AND backend='native' AND role='coordinator' AND parent_run_id IS NULL AND assignment_id='' ORDER BY id LIMIT 1",
        params![scan_id, attempt], |row| row.get::<_, String>(0),
    ) {
        match crate::agent_runtime::multi_agent::budget::root::remaining_child_capacity(&connection, &root) {
            Ok((tokens, requests)) if tokens == Some(0) || requests == Some(0) => return Err("original_request_balance_exhausted".into()),
            Err(_) => return Err("claim_ledger_unchecked".into()),
            Ok(_) => {}
        }
    }
    let work_dir: String = connection.query_row(
        "SELECT work_dir FROM sentinel_scan_attempts WHERE scan_id=?1 AND attempt_number=?2",
        params![scan_id, attempt], |row| row.get(0),
    ).map_err(|_| "continuation_artifacts_missing".to_string())?;
    if work_dir.trim().is_empty() { return Err("continuation_artifacts_missing".into()); }
    let root: String = connection.query_row(
        "SELECT id FROM agent_runs WHERE scan_id=?1 AND attempt_number=?2 AND backend='native' AND role='coordinator' AND parent_run_id IS NULL AND assignment_id='' ORDER BY id LIMIT 1",
        params![scan_id, attempt], |row| row.get(0),
    ).map_err(|_| "original_root_pipeline_unbound".to_string())?;
    let financed: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_root_budget_attempts WHERE root_run_id=?1)",
        [&root], |row| row.get(0),
    ).map_err(|_| "claim_ledger_unchecked".to_string())?;
    if !financed { return Err("original_root_pipeline_unbound".into()); }
    continuation_hold_counts(&connection, &root)?;
    let holder = claim_continuation_holder(&connection, scan_id, attempt)?;
    bind_continuation_write_holder(scan_id, attempt, &holder);
    let _continuation_writer = ContinuationWriteGuard;
    let _ = connection.execute(
        "UPDATE scan_continuation_holders SET prior_status=?1 WHERE scan_id=?2 AND attempt_number=?3 AND holder_id=?4",
        params![status, scan_id, attempt, holder],
    );
    if let Err(error) = reserve_continuation_budget_hold(&connection, &root) {
        release_continuation_holder(&connection, scan_id, attempt, &holder);
        return Err(error);
    }
    let settings = match agent_policy_settings_for_scan(&connection, scan_id) {
        Ok(settings) => settings,
        Err(error) => {
            abandon_unlaunched_continuation(&connection, scan_id, attempt, &holder, &root);
            return Err(error);
        }
    };
    let model = match model_runtime_env(&settings) {
        Ok(model) => model,
        Err(error) => {
            abandon_unlaunched_continuation(&connection, scan_id, attempt, &holder, &root);
            return Err(error);
        }
    };
    let worker = match crate::worker_resources::resolve_frontend_recon_worker_in(Path::new(&work_dir))
        .or_else(|_| crate::worker_resources::resolve_frontend_recon_worker_in(db_path.parent().unwrap_or(db_path)))
    {
        Ok(worker) => worker,
        Err(error) => {
            abandon_unlaunched_continuation(&connection, scan_id, attempt, &holder, &root);
            return Err(error);
        }
    };
    // The same runtime constructor as the original start and the recovery
    // path: proxy pool, no-proxy list, packet budget and the adaptive budget
    // with the attempt's web policy and deployment applied.
    let runtime = match resolve_agent_web_pipeline_runtime_for_attempt(&connection, scan_id, attempt, &model.deployment, worker.clone()) {
        Ok(runtime) => runtime,
        Err(error) => {
            abandon_unlaunched_continuation(&connection, scan_id, attempt, &holder, &root);
            return Err(error);
        }
    };
    let auth_session = match continuation_auth_session(&connection, scan_id, &runtime.web_policy, Path::new(&work_dir)) {
        Ok(auth_session) => auth_session,
        Err(error) => {
            abandon_unlaunched_continuation(&connection, scan_id, attempt, &holder, &root);
            return Err(error);
        }
    };
    let changed = match connection.execute(
        "UPDATE sentinel_scans SET status='scanning', updated_at=datetime('now','localtime') WHERE id=?1 AND status=?2 AND attempt_count=?3",
        params![scan_id, status, attempt],
    ) {
        Ok(changed) => changed,
        Err(error) => {
            abandon_unlaunched_continuation(&connection, scan_id, attempt, &holder, &root);
            return Err(error.to_string());
        }
    };
    if changed != 1 {
        abandon_unlaunched_continuation(&connection, scan_id, attempt, &holder, &root);
        return Err("continuation_state_changed".into());
    }
    let _ = connection.execute_batch("CREATE TABLE IF NOT EXISTS scan_target_continuation_fences(id TEXT PRIMARY KEY, project_id INTEGER NOT NULL, scan_id TEXT NOT NULL, attempt_number INTEGER NOT NULL, queued_urls_json TEXT NOT NULL, launched INTEGER NOT NULL DEFAULT 0, refusal_code TEXT NOT NULL, created_at TEXT NOT NULL DEFAULT (datetime('now')))");
    let _ = connection.execute("ALTER TABLE scan_target_continuation_fences ADD COLUMN prior_status TEXT", []);
    let project_id: i64 = connection.query_row("SELECT project_id FROM sentinel_scans WHERE id=?1", [scan_id], |row| row.get(0)).unwrap_or(0);
    let marked = connection.execute(
        "UPDATE scan_target_continuation_fences SET prior_status=?1 WHERE scan_id=?2 AND attempt_number=?3 AND launched=0",
        params![status, scan_id, attempt],
    ).unwrap_or(0);
    if marked == 0 {
        let _ = connection.execute(
            "INSERT INTO scan_target_continuation_fences(id,project_id,scan_id,attempt_number,queued_urls_json,launched,refusal_code,prior_status) VALUES(?1,?2,?3,?4,'[]',0,'continuation_spawn_pending',?5)",
            params![uuid::Uuid::new_v4().to_string(), project_id, scan_id, attempt, status],
        );
    }
    let revert = |connection: &rusqlite::Connection| {
        let _ = connection.execute(
            "UPDATE sentinel_scans SET status=?1 WHERE id=?2 AND status='scanning' AND attempt_count=?3",
            params![status, scan_id, attempt],
        );
    };
    let runtime_path = crate::worker_resources::sentinel_runtime_path(db_path.parent().unwrap_or(db_path));
    let launched = launch_sentinel_url_pipeline(
        db_path.to_path_buf(), scan_id.to_string(), attempt, worker, PathBuf::from(&work_dir), targets.clone(),
        runtime.proxies, runtime.no_proxy, model, runtime_path, runtime.adaptive, runtime.packet_budget, auth_session, None, Some(holder.clone()),
    );
    if let Err(error) = launched {
        revert(&connection);
        abandon_unlaunched_continuation(&connection, scan_id, attempt, &holder, &root);
        return Err(error);
    }
    if !continuation_claim_observed(db_path, scan_id, attempt) {
        restore_unlaunched_continuation(db_path, scan_id, attempt);
        revert(&connection);
        abandon_unlaunched_continuation(&connection, scan_id, attempt, &holder, &root);
        return Err("continuation_execution_unclaimed".into());
    }
    let commands = task_binding::promote_claimed_continuation(&connection, scan_id, attempt).unwrap_or_default();
    Ok(json!({"scanId": scan_id, "attemptNumber": attempt, "accepted": true, "executionStarted": true, "launched": true, "targetCount": targets.len(), "freshAllowanceMinted": false, "verified": false, "commands": commands}))
}

/// The identity file the attempt started with, still equal to the task's
/// current, validated identity material. A changed, expired or missing identity
/// blocks the continuation instead of running it anonymously or as someone else.
fn continuation_auth_session(
    connection: &rusqlite::Connection, scan_id: &str, web_policy: &serde_json::Value, work_dir: &Path,
) -> Result<Option<PathBuf>, String> {
    let project_id: i64 = connection.query_row("SELECT project_id FROM sentinel_scans WHERE id=?1", [scan_id], |row| row.get(0))
        .map_err(|_| "continuation_auth_unchecked".to_string())?;
    let expected = web_dispatch_auth_document(connection, project_id, web_policy)
        .map_err(|_| "continuation_auth_unchecked".to_string())?;
    let path = work_dir.join("auth-sessions.json");
    let actual = match fs::read(&path) {
        Ok(bytes) => Some(serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|_| "continuation_auth_invalid".to_string())?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Err("continuation_auth_unchecked".into()),
    };
    if actual != expected {
        return Err("continuation_auth_changed".into());
    }
    Ok(actual.map(|_| path))
}

pub(crate) fn remaining_continuation_targets(connection: &rusqlite::Connection, scan_id: &str) -> Result<Vec<(String, String)>, String> {
    let mut statement = connection.prepare(
        "SELECT company, url, status FROM sentinel_targets WHERE scan_id=?1 ORDER BY id",
    ).map_err(|_| "claim_ledger_unchecked".to_string())?;
    let rows = statement.query_map([scan_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)))
        .map_err(|_| "claim_ledger_unchecked".to_string())?;
    let mut remaining = Vec::new();
    for row in rows {
        let (company, url, status) = row.map_err(|_| "claim_ledger_unchecked".to_string())?;
        match status.as_str() {
            "queued" | "pending" | "" | "paused" => remaining.push((company, url)),
            "completed" | "recon_only" | "manual_review" | "fuse_excluded" | "cancelled" | "failed" | "limited" => {}
            "scanning" => return Err("continuation_target_still_running".into()),
            _ => return Err("continuation_target_state_unknown".into()),
        }
    }
    Ok(remaining)
}

pub(crate) fn claim_continuation_holder(connection: &rusqlite::Connection, scan_id: &str, attempt: i64) -> Result<String, String> {
    connection.execute_batch("CREATE TABLE IF NOT EXISTS scan_continuation_holders(scan_id TEXT NOT NULL, attempt_number INTEGER NOT NULL, holder_id TEXT NOT NULL, state TEXT NOT NULL, PRIMARY KEY(scan_id, attempt_number))").map_err(|error| error.to_string())?;
    let _ = connection.execute("ALTER TABLE scan_continuation_holders ADD COLUMN claimed_at TEXT NOT NULL DEFAULT ''", []);
    let _ = connection.execute("ALTER TABLE scan_continuation_holders ADD COLUMN prior_status TEXT NOT NULL DEFAULT ''", []);
    let holder = uuid::Uuid::new_v4().to_string();
    let inserted = connection.execute(
        "INSERT OR IGNORE INTO scan_continuation_holders(scan_id,attempt_number,holder_id,state,claimed_at) VALUES(?1,?2,?3,'claiming',datetime('now'))",
        params![scan_id, attempt, holder],
    ).map_err(|error| error.to_string())?;
    if inserted == 1 { return Ok(holder); }
    let state: String = connection.query_row(
        "SELECT state FROM scan_continuation_holders WHERE scan_id=?1 AND attempt_number=?2",
        params![scan_id, attempt], |row| row.get(0),
    ).map_err(|error| error.to_string())?;
    if state == "released" || unstarted_claim_is_stale(connection, scan_id, attempt) {
        let replaced = connection.execute(
            "UPDATE scan_continuation_holders SET holder_id=?1, state='claiming', claimed_at=datetime('now') WHERE scan_id=?2 AND attempt_number=?3 AND (state='released' OR (state='claiming' AND claimed_at<>'' AND claimed_at<=datetime('now','-5 seconds')))",
            params![holder, scan_id, attempt],
        ).map_err(|error| error.to_string())?;
        if replaced == 1 { return Ok(holder); }
    }
    Err("continuation_holder_conflict".into())
}

fn unstarted_claim_is_stale(connection: &rusqlite::Connection, scan_id: &str, attempt: i64) -> bool {
    let status: String = connection.query_row("SELECT status FROM sentinel_scans WHERE id=?1", [scan_id], |row| row.get(0)).unwrap_or_default();
    if status == "scanning" { return false; }
    connection.query_row(
        "SELECT COUNT(*) FROM scan_continuation_holders WHERE scan_id=?1 AND attempt_number=?2 AND state='claiming' AND claimed_at<>'' AND claimed_at<=datetime('now','-5 seconds')",
        params![scan_id, attempt], |row| row.get::<_, i64>(0),
    ).unwrap_or(0) == 1
}

/// Successful counts stay as read, including 0. A failed read is the same
/// ledger block as the other unread claim queries.
fn continuation_hold_counts(connection: &rusqlite::Connection, root: &str) -> Result<(i64, i64), String> {
    let count = |kind: &str| connection.query_row(
        "SELECT COUNT(*) FROM agent_budget_entries WHERE root_run_id=?1 AND source_id='continuation-hold' AND dimension='model_requests' AND kind=?2",
        params![root, kind], |row| row.get(0),
    ).map_err(|_| "claim_ledger_unchecked".to_string());
    Ok((count("reserve")?, count("release")?))
}

/// One request hold on the original root. `remaining_child_capacity` already
/// counts this reserve. It is not a new allowance, and a later usage write
/// releases it instead of adding a second one.
pub(crate) fn reserve_continuation_budget_hold(connection: &rusqlite::Connection, root: &str) -> Result<(), String> {
    crate::agent_runtime::continuation_writer::guard_budget_append(connection, root)?;
    let (reserved, released) = continuation_hold_counts(connection, root)?;
    if reserved > released { return Ok(()); }
    let lease: String = connection.query_row(
        "SELECT id FROM agent_root_budget_attempts WHERE root_run_id=?1 ORDER BY id LIMIT 1",
        [root], |row| row.get(0),
    ).map_err(|_| "claim_ledger_unchecked".to_string())?;
    match crate::agent_runtime::multi_agent::budget::root::remaining_child_capacity(connection, root) {
        Ok((_, Some(0))) => return Err("original_request_balance_exhausted".into()),
        Ok(_) => {}
        Err(_) => return Err("claim_ledger_unchecked".into()),
    }
    connection.execute(
        "INSERT INTO agent_budget_entries(entry_id,root_run_id,assignment_id,lease_attempt_id,dimension,kind,amount,idempotency_key,source_id) VALUES(?1,?2,'',?3,'model_requests','reserve',1,?4,'continuation-hold')",
        params![uuid::Uuid::new_v4().to_string(), root, lease, format!("continuation-hold:{root}:model_requests:{reserved}")],
    ).map_err(|error| error.to_string())?;
    Ok(())
}

pub(crate) fn release_continuation_budget_hold(connection: &rusqlite::Connection, root: &str) -> Result<(), String> {
    crate::agent_runtime::continuation_writer::guard_budget_append(connection, root)?;
    let (reserved, released) = continuation_hold_counts(connection, root)?;
    if reserved <= released { return Ok(()); }
    let lease: String = connection.query_row(
        "SELECT lease_attempt_id FROM agent_budget_entries WHERE root_run_id=?1 AND source_id='continuation-hold' AND dimension='model_requests' AND kind='reserve' ORDER BY rowid DESC LIMIT 1",
        [root], |row| row.get(0),
    ).map_err(|error| error.to_string())?;
    connection.execute(
        "INSERT INTO agent_budget_entries(entry_id,root_run_id,assignment_id,lease_attempt_id,dimension,kind,amount,idempotency_key,source_id) VALUES(?1,?2,'',?3,'model_requests','release',1,?4,'continuation-hold')",
        params![uuid::Uuid::new_v4().to_string(), root, lease, format!("continuation-hold-release:{root}:model_requests:{released}")],
    ).map_err(|error| error.to_string())?;
    Ok(())
}

pub(crate) fn release_continuation_budget_hold_for_scan(connection: &rusqlite::Connection, scan_id: &str, attempt: i64) {
    let Ok(root) = connection.query_row(
        "SELECT id FROM agent_runs WHERE scan_id=?1 AND attempt_number=?2 AND backend='native' AND role='coordinator' AND parent_run_id IS NULL AND assignment_id='' ORDER BY id LIMIT 1",
        params![scan_id, attempt], |row| row.get::<_, String>(0),
    ) else { return; };
    let _ = release_continuation_budget_hold(connection, &root);
}

fn abandon_unlaunched_continuation(connection: &rusqlite::Connection, scan_id: &str, attempt: i64, holder: &str, root: &str) {
    let _ = release_continuation_budget_hold(connection, root);
    release_continuation_holder(connection, scan_id, attempt, holder);
}

pub(crate) fn release_continuation_holder(connection: &rusqlite::Connection, scan_id: &str, attempt: i64, holder: &str) {
    let _ = connection.execute(
        "UPDATE scan_continuation_holders SET state='released' WHERE scan_id=?1 AND attempt_number=?2 AND holder_id=?3",
        params![scan_id, attempt, holder],
    );
}

pub(crate) fn reject_stale_continuation_write(connection: &rusqlite::Connection, scan_id: &str, attempt: i64, holder: &str) -> Result<(), String> {
    let current: (String, String) = connection.query_row(
        "SELECT holder_id, state FROM scan_continuation_holders WHERE scan_id=?1 AND attempt_number=?2",
        params![scan_id, attempt], |row| Ok((row.get(0)?, row.get(1)?)),
    ).map_err(|_| "continuation_holder_stale".to_string())?;
    if current.0 != holder || current.1 == "released" { return Err("continuation_holder_stale".into()); }
    Ok(())
}

struct ContinuationWriteGuard;

impl Drop for ContinuationWriteGuard {
    fn drop(&mut self) { crate::agent_runtime::continuation_writer::clear(); }
}

pub(crate) fn bind_continuation_write_holder(scan_id: &str, attempt: i64, holder: &str) {
    crate::agent_runtime::continuation_writer::bind(scan_id, attempt, holder);
}


fn continuation_holder_for_write(scan_id: &str, attempt: i64) -> Option<String> {
    crate::agent_runtime::continuation_writer::bound().and_then(|(id, bound_attempt, holder)| {
        (id == scan_id && bound_attempt == attempt).then_some(holder)
    })
}

fn authorize_bound_continuation_write(connection: &rusqlite::Connection, scan_id: &str, attempt: i64) -> Result<(), String> {
    if let Some((id, bound_attempt, holder)) = crate::agent_runtime::continuation_writer::bound() {
        if id != scan_id { return Ok(()); }
        if bound_attempt != attempt { return Err("continuation_holder_stale".into()); }
        return reject_stale_continuation_write(connection, scan_id, attempt, &holder);
    }
    let claiming: i64 = connection.query_row(
        "SELECT COUNT(*) FROM scan_continuation_holders WHERE scan_id=?1 AND attempt_number=?2 AND state='claiming'",
        params![scan_id, attempt], |row| row.get(0),
    ).unwrap_or(0);
    if claiming > 0 { return Err("continuation_holder_stale".into()); }
    Ok(())
}

fn continuation_write_refused(db_path: &Path, scan_id: &str, attempt: i64) -> bool {
    db::open(db_path).is_ok_and(|connection| authorize_bound_continuation_write(&connection, scan_id, attempt).is_err())
}

/// A continuation that set scanning and then died before the worker claimed the
/// fence is not a running scan. Put the saved prior status back so the same
/// attempt can be claimed again. A launched fence or a fresh claim stays.
pub(crate) fn recover_unclaimed_continuation(connection: &rusqlite::Connection, scan_id: &str) -> Result<(), String> {
    let _ = connection.execute("ALTER TABLE scan_continuation_holders ADD COLUMN prior_status TEXT NOT NULL DEFAULT ''", []);
    let Ok((status, attempt)) = connection.query_row(
        "SELECT status, attempt_count FROM sentinel_scans WHERE id=?1",
        [scan_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
    ) else { return Ok(()); };
    if status != "scanning" { return Ok(()); }
    let launched: i64 = connection.query_row(
        "SELECT COUNT(*) FROM scan_target_continuation_fences WHERE scan_id=?1 AND attempt_number=?2 AND launched=1",
        params![scan_id, attempt], |row| row.get(0),
    ).unwrap_or(0);
    if launched > 0 { return Ok(()); }
    let holder_prior: String = connection.query_row(
        "SELECT prior_status FROM scan_continuation_holders WHERE scan_id=?1 AND attempt_number=?2 AND state='claiming' AND claimed_at<>'' AND claimed_at<=datetime('now','-5 seconds')",
        params![scan_id, attempt], |row| row.get(0),
    ).unwrap_or_default();
    let fence_prior: String = connection.query_row(
        "SELECT prior_status FROM scan_target_continuation_fences WHERE scan_id=?1 AND attempt_number=?2 AND launched=0 AND COALESCE(prior_status,'')<>'' ORDER BY created_at DESC LIMIT 1",
        params![scan_id, attempt], |row| row.get(0),
    ).unwrap_or_default();
    let prior = if fence_prior.trim().is_empty() { holder_prior } else { fence_prior };
    if !matches!(prior.as_str(), "paused" | "partial") { return Ok(()); }
    connection.execute(
        "UPDATE sentinel_scans SET status=?1, updated_at=datetime('now','localtime') WHERE id=?2 AND status='scanning' AND attempt_count=?3",
        params![prior, scan_id, attempt],
    ).map_err(|error| error.to_string())?;
    Ok(())
}

fn continuation_claim_observed(db_path: &Path, scan_id: &str, attempt: i64) -> bool {
    let started = std::time::Instant::now();
    while started.elapsed() < std::time::Duration::from_millis(1500) {
        if let Ok(connection) = db::open(db_path) {
            let launched: i64 = connection.query_row(
                "SELECT launched FROM scan_target_continuation_fences WHERE scan_id=?1 AND attempt_number=?2 ORDER BY created_at DESC LIMIT 1",
                params![scan_id, attempt], |row| row.get(0),
            ).unwrap_or(0);
            if launched == 1 { return true; }
            let status: String = connection.query_row(
                "SELECT status FROM sentinel_scans WHERE id=?1", [scan_id], |row| row.get(0),
            ).unwrap_or_default();
            if status != "scanning" { return false; }
        }
        thread::sleep(std::time::Duration::from_millis(20));
    }
    false
}

fn restore_unlaunched_continuation(db_path: &Path, scan_id: &str, attempt: i64) {
    let Ok(connection) = db::open(db_path) else { return };
    let Ok(prior) = connection.query_row(
        "SELECT prior_status FROM scan_target_continuation_fences WHERE scan_id=?1 AND attempt_number=?2 AND launched=0 AND prior_status<>'' ORDER BY created_at DESC LIMIT 1",
        params![scan_id, attempt], |row| row.get::<_, String>(0),
    ) else { return };
    let _ = connection.execute(
        "UPDATE sentinel_scans SET status=?1 WHERE id=?2 AND status='scanning' AND attempt_count=?3",
        params![prior, scan_id, attempt],
    );
}
