/// The Strix adapter. It keeps only process, mount and artifact handling; the
/// plan, budgets and terminal interpretation belong to Oviraptor.
struct StrixAgentBackend<'a> {
    db_path: &'a Path,
    scan_id: &'a str,
    strix: &'a str,
    docker: &'a Path,
    instruction_path: &'a Path,
    no_proxy: &'a str,
    environment: &'a StrixRuntimeEnv,
    runtime_path: &'a OsString,
    adaptive: &'a AdaptiveStrixSettings,
    position: usize,
    total: usize,
    log_path: &'a Path,
}

impl AgentBackend for StrixAgentBackend<'_> {
    fn kind(&self) -> AgentBackendKind {
        AgentBackendKind::Strix
    }

    fn execute(&self, context: &AgentRunContext) -> AgentTargetOutcome {
        if self.strix.trim().is_empty() {
            return AgentTargetOutcome::Failed(AgentStop::new(
                AGENT_STOP_CONFIGURATION,
                "本机没有可用的 Strix；原生 Agent 后端不依赖它，请在运行环境页安装 Strix 后才能使用 Strix 后端",
            ));
        }
        run_adaptive_strix_with_provider_retry(
            self.db_path,
            self.scan_id,
            self.strix,
            self.docker,
            &context.target_dir,
            &context.route,
            self.instruction_path,
            context.proxy.as_deref(),
            self.no_proxy,
            self.environment,
            self.runtime_path,
            self.adaptive,
            self.position,
            self.total,
            self.log_path,
        )
    }
}

/// §11: everything the task detail "执行计划" panel shows, from the plan and
/// checkpoint rows. Raw request/response JSON stays in the existing viewer.
#[tauri::command]
pub fn get_agent_target_execution(
    state: State<'_, AppState>,
    scan_id: String,
    url: String,
) -> Result<JsonValue, String> {
    let connection = db::open(&state.db_path)?;
    let plan = read_agent_checkpoint(&state.db_path, &scan_id, &url, "agent_execution_plan");
    let native = NativeAgentState::read(&state.db_path, &scan_id, &url);
    let (target_status, target_mode): (String, String) = connection
        .query_row(
            "SELECT status,scan_mode FROM sentinel_targets WHERE scan_id=?1 AND url=?2",
            params![scan_id, url],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap_or_else(|_| ("queued".to_string(), String::new()));
    let coverage: String = connection
        .query_row(
            "SELECT record_json FROM sentinel_findings WHERE scan_id=?1 AND target_url=?2 AND stage=?3 AND record_key='coverage' ORDER BY updated_at DESC LIMIT 1",
            params![scan_id, url, AGENT_COVERAGE_STAGE],
            |row| row.get(0),
        )
        .unwrap_or_default();
    let confirmed: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sentinel_findings WHERE scan_id=?1 AND target_url=?2 AND stage=?3 AND kind='vulnerability'",
            params![scan_id, url, AGENT_VULNERABILITY_STAGE],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let required: Vec<&str> = agent_required_families();
    let covered: Vec<String> = native
        .as_ref()
        .map(|value| {
            let mut rows = value.covered_families.clone();
            rows.sort();
            rows
        })
        .unwrap_or_default();
    let usage_from_plan = plan.get("budgets").cloned().unwrap_or(serde_json::json!({}));
    Ok(serde_json::json!({
        "backend": plan.get("backend").cloned().unwrap_or_else(|| serde_json::json!("strix")),
        "mode": plan.get("mode").cloned().unwrap_or(serde_json::json!(target_mode)),
        "surface": plan.get("surface"),
        "timeoutSeconds": plan.get("timeoutSeconds"),
        "budgets": usage_from_plan,
        "hardLimits": {
            "hardTotalTokens": plan.pointer("/budgets/hardTotalTokens"),
            "hardModelRequests": plan.pointer("/budgets/hardModelRequests"),
            "maxTurns": plan.pointer("/budgets/maxTurns"),
            "noProgressWindow": plan.pointer("/stopping/noProgressWindow")
        },
        "coverage": {
            "required": required,
            "requiredLabels": required.iter().map(|family| agent_coverage_family_label(family)).collect::<Vec<_>>(),
            "covered": covered,
            "completedRatio": if required.is_empty() { 0.0 } else { covered.len() as f64 / required.len() as f64 },
            "ledger": json(coverage),
            "ledgerReported": native.is_some(),
            "confirmedFindings": confirmed
        },
        "runtime": {
            "turns": native.as_ref().map(|value| value.turns).unwrap_or(0),
            "noProgressStreak": native.as_ref().map(|value| value.no_progress_streak).unwrap_or(0),
            "currentAction": native.as_ref().and_then(|value| value.pending_queue.first().cloned()),
            "progressSignature": native.as_ref().map(|value| value.progress_signature.clone()).unwrap_or_default(),
            "lastExpansionReason": native.as_ref().map(|value| value.last_expansion_reason.clone()).unwrap_or_default(),
            "budgetUsage": native.as_ref().map(|value| value.budget_usage.clone()).unwrap_or(serde_json::json!({})),
            "tokenUsage": native.as_ref().map(|value| value.token_usage.as_json()).unwrap_or_else(|| serde_json::json!({})),
            "terminalReason": native.as_ref().map(|value| value.terminal_reason.clone()).unwrap_or_default()
        },
        "targetStatus": target_status,
        "targetStatusText": agent_terminal_status_label(&target_status),
    }))
}

fn agent_terminal_status_label(status: &str) -> String {
    match status {
        "completed" => "已完成",
        "partial" | "paused" | "completed_with_gaps" => "待补充验证",
        "limited" | "protected_stop" => "已熔断",
        "failed" => "执行失败",
        "recon_only" => "仅完成确定性侦察",
        "manual_review" => "转人工分析",
        "resume_incompatible" => "续跑状态不兼容，需要重新执行",
        "persistence_failure" => "本地记录失败，已停止以避免重复消耗",
        "cancelled" => "已取消",
        "scanning" => "正在执行",
        other => other,
    }
    .to_string()
}

/// Code, CI and source scans still need a sandboxed full-stack engine; the
/// native web agent only owns HTTP/JS targets with a deterministic evidence
/// bundle.
fn agent_native_eligible(scan_type: &str, source_path: &str, urls: &[String]) -> bool {
    // Source, CI and greybox-with-source runs stay on the sandboxed engine; only
    // a URL-only web target set is owned by the native agent.
    source_path.trim().is_empty() && !urls.is_empty() && matches!(scan_type, "web" | "greybox")
}

/// Runtime inputs the shared web pipeline needs. Both the Asset task entry and
/// the Strix workbench entry build them here so there is only one set of
/// budget, proxy and worker resolution rules.
struct AgentWebPipelineRuntime {
    worker: PathBuf,
    proxies: Vec<(String, String)>,
    no_proxy: String,
    adaptive: AdaptiveStrixSettings,
    packet_budget: usize,
    web_policy: JsonValue,
    skill_names: String,
    skill_instructions: String,
}

fn agent_web_pipeline_runtime(
    app: &tauri::AppHandle,
    connection: &rusqlite::Connection,
    scan_id: &str,
    deployment: &str,
) -> Result<AgentWebPipelineRuntime, String> {
    let settings = sentinel_settings(connection);
    let mut adaptive = AdaptiveStrixSettings::from_json(&settings);
    let stored_web_policy = connection
        .query_row(
            "SELECT policy_json FROM sentinel_scan_contexts WHERE scan_id=?1",
            [scan_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .map(json)
        .unwrap_or_else(|| serde_json::json!({"webModeCeiling": "standard"}));
    let (web_policy, skill_names, skill_instructions) =
        effective_web_policy(connection, &stored_web_policy, &settings)?;
    connection
        .execute(
            "INSERT INTO sentinel_scan_contexts(scan_id,environment,policy_json) VALUES(?1,'internal',?2) ON CONFLICT(scan_id) DO UPDATE SET policy_json=excluded.policy_json,updated_at=datetime('now','localtime')",
            params![scan_id, web_policy.to_string()],
        )
        .map_err(|error| error.to_string())?;
    adaptive.apply_web_policy(&web_policy);
    adaptive.apply_deployment(deployment);
    Ok(AgentWebPipelineRuntime {
        worker: resolve_frontend_recon_worker(app)?,
        proxies: approved_strix_proxies(&settings),
        no_proxy: settings
            .get("noProxy")
            .and_then(JsonValue::as_str)
            .unwrap_or("127.0.0.1,localhost")
            .to_string(),
        packet_budget: frontend_packet_budget(&settings, deployment),
        adaptive,
        web_policy,
        skill_names,
        skill_instructions,
    })
}

/// One target's place in the attempt's backend matrix (Phase 2 §3.2).
#[derive(Clone, Debug, PartialEq)]
struct ScanTargetBackend {
    url: String,
    backend: AgentBackendKind,
    selection_reason: String,
}

/// The immutable per-attempt backend matrix. Dependency resolution happens only
/// after every target has been classified, so a mixed scan can never run half way
/// and then discover the Strix binary or the Docker daemon is missing.
#[derive(Clone, Debug, PartialEq)]
struct ScanBackendPlan {
    scan_id: String,
    attempt_number: i64,
    targets: Vec<ScanTargetBackend>,
    requires_strix: bool,
    requires_docker: bool,
    requires_node: bool,
    requires_browser: bool,
}

impl ScanBackendPlan {
    fn as_json(&self) -> JsonValue {
        serde_json::json!({
            "schemaVersion": 1,
            "scanId": self.scan_id,
            "attemptNumber": self.attempt_number,
            "targets": self.targets.iter().map(|target| serde_json::json!({
                "url": target.url,
                "backend": target.backend.as_str(),
                "selectionReason": target.selection_reason,
            })).collect::<Vec<_>>(),
            "requiresStrix": self.requires_strix,
            "requiresDocker": self.requires_docker,
            "requiresNode": self.requires_node,
            "requiresBrowser": self.requires_browser,
        })
    }

    fn from_json(value: &JsonValue) -> Option<Self> {
        if value.get("schemaVersion").and_then(JsonValue::as_i64) != Some(1) {
            return None;
        }
        let targets = value
            .get("targets")
            .and_then(JsonValue::as_array)?
            .iter()
            .filter_map(|row| {
                Some(ScanTargetBackend {
                    url: row.get("url")?.as_str()?.to_string(),
                    backend: AgentBackendKind::parse(row.get("backend")?.as_str()?)?,
                    selection_reason: row
                        .get("selectionReason")
                        .and_then(JsonValue::as_str)
                        .unwrap_or_default()
                        .to_string(),
                })
            })
            .collect::<Vec<_>>();
        let flag = |key: &str| {
            value
                .get(key)
                .and_then(JsonValue::as_bool)
                .unwrap_or(false)
        };
        Some(Self {
            scan_id: value
                .get("scanId")
                .and_then(JsonValue::as_str)
                .unwrap_or_default()
                .to_string(),
            attempt_number: value
                .get("attemptNumber")
                .and_then(JsonValue::as_i64)
                .unwrap_or(1),
            targets,
            requires_strix: flag("requiresStrix"),
            requires_docker: flag("requiresDocker"),
            requires_node: flag("requiresNode"),
            requires_browser: flag("requiresBrowser"),
        })
    }

    fn backend_of(&self, url: &str) -> Option<ScanTargetBackend> {
        self.targets.iter().find(|target| target.url == url).cloned()
    }

    fn refresh_requirements(&mut self, node_needed: bool) {
        self.requires_strix = self
            .targets
            .iter()
            .any(|target| target.backend == AgentBackendKind::Strix);
        // Strix runs in the sandbox, so anything on Strix needs Docker too.
        self.requires_docker = self.requires_strix;
        self.requires_node = node_needed && !self.targets.is_empty();
        self.requires_browser = self.requires_node;
    }
}

fn read_scan_backend_plan(
    db_path: &Path,
    scan_id: &str,
    attempt_number: i64,
) -> Option<ScanBackendPlan> {
    // The attempt row is the authority once it exists. Before that, the matrix is
    // still frozen — it just lives on the scan-level projection row, which is how a
    // decision taken before the attempt row was created survives until then.
    if let Some(plan) = db::open(db_path).ok().and_then(|connection| {
        let stored: String = connection
            .query_row(
                "SELECT backend_plan_json FROM sentinel_scan_attempts WHERE scan_id=?1 AND attempt_number=?2",
                params![scan_id, attempt_number],
                |row| row.get(0),
            )
            .unwrap_or_default();
        if stored.trim().is_empty() {
            return None;
        }
        ScanBackendPlan::from_json(&json(stored))
    }) {
        return Some(plan);
    }
    let staged = read_agent_checkpoint(db_path, scan_id, "", "scan_backend_plan");
    ScanBackendPlan::from_json(&staged)
        .filter(|plan| plan.scan_id == scan_id && plan.attempt_number == attempt_number)
}

fn write_scan_backend_plan(db_path: &Path, plan: &ScanBackendPlan) -> Result<(), String> {
    let payload = plan.as_json();
    // Never insert an attempt row from here: the pipeline owns that row, and a
    // phantom row would show up as a fake attempt in the UI.
    // A matrix that cannot be stored still governs this start-up decision, because
    // the caller holds the value; the failure is returned so the entry can report it.
    let projection = write_agent_checkpoint(db_path, &plan.scan_id, "", "scan_backend_plan", &payload);
    let attempt_row = db::open(db_path)
        .map_err(|error| error.to_string())
        .and_then(|connection| {
            connection
                .execute(
                    "UPDATE sentinel_scan_attempts SET backend_plan_json=?1,updated_at=datetime('now','localtime') WHERE scan_id=?2 AND attempt_number=?3",
                    params![payload.to_string(), plan.scan_id, plan.attempt_number],
                )
                .map(|_| ())
                .map_err(|error| error.to_string())
        });
    projection.and(attempt_row)
}

/// Decide the backend of every target in this attempt, once. A stored matrix is
/// returned untouched, so editing settings mid-run cannot move a target between
/// backends (Phase 2 §3.2).
fn plan_scan_backends(
    db_path: &Path,
    scan_id: &str,
    attempt_number: i64,
    urls: &[String],
    settings: &JsonValue,
    native_eligible: bool,
) -> Result<ScanBackendPlan, String> {
    let mut plan = read_scan_backend_plan(db_path, scan_id, attempt_number).unwrap_or_else(|| {
        ScanBackendPlan {
            scan_id: scan_id.to_string(),
            attempt_number,
            targets: Vec::new(),
            requires_strix: false,
            requires_docker: false,
            requires_node: false,
            requires_browser: false,
        }
    });
    let mut changed = false;
    for url in urls {
        if url.trim().is_empty() || plan.backend_of(url).is_some() {
            continue;
        }
        let (backend, reason) =
            agent_backend_choice(db_path, scan_id, attempt_number, url, settings, native_eligible);
        plan.targets.push(ScanTargetBackend {
            url: url.clone(),
            backend,
            selection_reason: reason,
        });
        changed = true;
    }
    if changed {
        plan.refresh_requirements(native_eligible);
        // §5.2: the frozen matrix is a durable record, not a suggestion.
        write_scan_backend_plan(db_path, &plan)?;
    }
    Ok(plan)
}

/// The ONE backend-selection function. Both task-creating entries call it, and the
/// answer depends on the **current attempt's** execution mode (Phase 2 §2.2):
///
/// * `initial` / `fresh` — today's settings decide, and the previous attempt's
///   pinned plan is deliberately ignored, so switching policy and re-running works;
/// * `resume` — only the parent attempt's frozen plan may decide;
/// * an attempt that already recorded its own plan — that plan decides, so a
///   settings edit can never switch a running attempt.
fn agent_select_backend(
    db_path: &Path,
    scan_id: &str,
    attempt_number: i64,
    url: &str,
    settings: &JsonValue,
    native_eligible: bool,
) -> AgentBackendKind {
    agent_backend_choice(db_path, scan_id, attempt_number, url, settings, native_eligible).0
}

/// Same decision, plus the reason the matrix records for the UI and the logs.
fn agent_backend_choice(
    db_path: &Path,
    scan_id: &str,
    attempt_number: i64,
    url: &str,
    settings: &JsonValue,
    native_eligible: bool,
) -> (AgentBackendKind, String) {
    // A target that already appears in this attempt's frozen matrix never changes.
    if let Some(plan) = read_scan_backend_plan(db_path, scan_id, attempt_number) {
        if let Some(target) = plan.backend_of(url) {
            return (target.backend, target.selection_reason);
        }
    }
    let from_settings = || match AgentBackendPolicy::from_settings(settings) {
        // §7.1 after the Phase 2 gates: a URL-only web or grey-box task is owned by
        // the native agent, so `auto` selects it. Source, CI and anything native
        // cannot express stay on the compatibility backend. Neither side switches
        // after a failure; the choice is frozen per attempt above.
        AgentBackendPolicy::Native | AgentBackendPolicy::Auto if native_eligible => {
            AgentBackendKind::Native
        }
        AgentBackendPolicy::Strix => AgentBackendKind::Strix,
        // Explicit native on a task type it does not own, or `auto` on a source or
        // CI scan: the sandboxed engine is the only backend that can run it.
        _ => AgentBackendKind::Strix,
    };
    let lineage = agent_attempt_lineage(db_path, scan_id, attempt_number);
    let inherited_from = if lineage.resume_kind == AgentResumeKind::ContinueIncomplete {
        Some(lineage.parent_attempt_number.unwrap_or(attempt_number))
    } else if persisted_attempt_backend(db_path, scan_id, attempt_number, url).is_some() {
        // This very attempt already started on a backend; keep it.
        Some(attempt_number)
    } else {
        None
    };
    let Some(attempt) = inherited_from else {
        let policy = AgentBackendPolicy::from_settings(settings);
        return (
            from_settings(),
            format!("{policy:?} 策略在当前配置下选定，attempt {attempt_number} 全新开始"),
        );
    };
    match persisted_attempt_backend(db_path, scan_id, attempt, url) {
        // The attempt (or its parent, for a continuation) froze this backend.
        Some(backend) => (
            backend,
            format!("继承 attempt {attempt} 的冻结后端（{}）", backend.as_str()),
        ),
        // A continuation with no parent plan must not guess a backend here: the
        // frozen-plan step reports `resume_incompatible` before any request.
        None => (
            from_settings(),
            "父 attempt 没有冻结计划，按当前配置选择".to_string(),
        ),
    }
}

/// `execution_mode` of the current attempt: only a resumed attempt may pick up
/// the unfinished queue of its own previous run.
fn agent_attempt_is_resume(db_path: &Path, scan_id: &str, attempt_number: i64) -> bool {
    let Ok(connection) = db::open(db_path) else {
        return false;
    };
    connection
        .query_row(
            "SELECT execution_mode FROM sentinel_scan_attempts WHERE scan_id=?1 AND attempt_number=?2",
            params![scan_id, attempt_number],
            |row| row.get::<_, String>(0),
        )
        .unwrap_or_default()
        == "resume"
}

fn agent_attempt_number(db_path: &Path, scan_id: &str) -> i64 {
    let Ok(connection) = db::open(db_path) else {
        return 1;
    };
    connection
        .query_row(
            "SELECT attempt_count FROM sentinel_scans WHERE id=?1",
            [scan_id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(1)
        .max(1)
}

fn agent_build_run_context(
    db_path: &Path,
    scan_id: &str,
    prepared: &PreparedFrontendTarget,
    environment: &StrixRuntimeEnv,
    adaptive: &AdaptiveStrixSettings,
    backend: AgentBackendKind,
    log_path: &Path,
) -> AgentRunContext {
    let evidence_path = prepared.target_dir.join("frontend-evidence.json");
    let evidence = json(
        fs::read_to_string(&evidence_path)
            .unwrap_or_else(|_| "{\"error\":\"frontend_evidence_unavailable\"}".into()),
    );
    let capabilities = json(
        fs::read_to_string(prepared.target_dir.join(SRC_ASSURANCE_ADAPTER_NAME))
            .unwrap_or_else(|_| "{\"schemaVersion\":0}".into()),
    );
    let identities = match db::open(db_path) {
        Ok(connection) => scan_identity_keys(&connection, scan_id)
            .unwrap_or_default()
            .into_iter()
            .map(|key| agent_identity_from_key(&key))
            .collect(),
        Err(_) => vec![AgentIdentity::anonymous()],
    };
    let identities = if identities.is_empty() {
        vec![AgentIdentity::anonymous()]
    } else {
        identities
    };
    let attempt_number = agent_attempt_number(db_path, scan_id);
    // A continuation must run on the parent's frozen plan; only a fresh attempt
    // reads today's settings into a new one (§3.5).
    let plan_and_rejection = agent_frozen_plan(
        db_path,
        scan_id,
        &prepared.route.url,
        &prepared.route,
        environment,
        adaptive,
        backend,
        attempt_number,
    )
    .map(|plan| (plan, None)).unwrap_or_else(|rejection| {
        // Nothing is executed in this case: the loop reports the rejection as the
        // attempt's terminal state, and the parent's own plan row stays intact.
        append_runner_log(
            log_path,
            &format!("{}；{}", AGENT_STOP_RESUME_INCOMPATIBLE, rejection.message()),
        );
        (
            build_agent_execution_plan(adaptive, &prepared.route, environment, backend, db_path, scan_id)
                .with_attempt(attempt_number),
            Some(rejection),
        )
    });
    let (plan, plan_rejection) = plan_and_rejection;
    AgentRunContext {
        scan_id: scan_id.to_string(),
        attempt_number,
        target_url: prepared.route.url.clone(),
        target_dir: prepared.target_dir.clone(),
        db_path: db_path.to_path_buf(),
        route: prepared.route.clone(),
        execution_plan: plan,
        evidence,
        capabilities,
        identities,
        proxy: prepared.proxy.clone(),
        log_path: log_path.to_path_buf(),
        environment: environment.clone(),
        resume: agent_attempt_is_resume(db_path, scan_id, agent_attempt_number(db_path, scan_id)),
        browser: prepared.browser.clone(),
        plan_rejection,
        // Filled in by the orchestrator once the run row is registered.
        run: None,
    }
}

/// `scan_identity_keys` yields ordered `session:{id}:{name}` / `anonymous` keys.
fn agent_identity_from_key(key: &str) -> AgentIdentity {
    let mut parts = key.split(':');
    match (parts.next(), parts.next()) {
        (Some("session"), Some(session_id)) if !session_id.is_empty() => {
            AgentIdentity::scoped(session_id)
        }
        _ => AgentIdentity::anonymous(),
    }
}

/// Shared per-target entry used by every web pipeline.
fn run_agent_target(
    prepared: &PreparedFrontendTarget,
    db_path: &Path,
    scan_id: &str,
    settings: &JsonValue,
    strix_backend: &StrixAgentBackend<'_>,
) -> AgentTargetOutcome {
    let backend = agent_select_backend(
        db_path,
        scan_id,
        agent_attempt_number(db_path, scan_id),
        &prepared.route.url,
        settings,
        true,
    );
    let mut context = agent_build_run_context(
        db_path,
        scan_id,
        prepared,
        strix_backend.environment,
        strix_backend.adaptive,
        backend,
        strix_backend.log_path,
    );
    // §11: the run row exists with its plan and budgets from the moment the
    // backend starts, so a pause, crash or cancel leaves something to recover.
    let run = runtime_open_run(db_path, scan_id, &prepared.route);
    context.run = run;
    if backend != AgentBackendKind::Native {
        return strix_backend.execute(&context);
    }
    if context.evidence.get("error").is_some() {
        return AgentTargetOutcome::failed(
            "原生 Agent 缺少本地证据包，已停止且不会回退 Strix 重跑",
        );
    }
    append_runner_log(
        &context.log_path,
        &format!(
            "{} backend selected for {} · plan {}",
            NativeAgentBackend.kind().as_str(),
            context.target_url,
            context.execution_plan.hash()
        ),
    );
    // §19: a native attempt never restarts the same target on Strix, so a retry
    // can never double-spend. What the native loop reports is what the reducer
    // receives.
    NativeAgentBackend.execute(&context)
}
/// Backend selection, the per-target orchestrator and the pipeline tally.
///
/// Every backend reports an `AgentTargetOutcome`; only this file turns it into
/// persisted target status and the scan summary, so native and Strix can never
/// grow separate "部分完成" wording.
#[derive(Default)]
struct AgentPipelineTally {
    completed: usize,
    /// Targets that closed inside their bounds **with named coverage gaps** (§10).
    /// Counting them as `completed` is what let a scan claim a clean finish while
    /// whole families were never verified.
    completed_with_gaps: usize,
    partial: usize,
    skipped: usize,
    manual_review: usize,
    limited: usize,
    failed: usize,
    failure_details: Vec<String>,
}

impl AgentPipelineTally {
    fn counted(&self) -> usize {
        self.completed
            + self.completed_with_gaps
            + self.partial
            + self.skipped
            + self.manual_review
            + self.limited
            + self.failed
    }

    fn deferred(&self, total: usize) -> usize {
        total.saturating_sub(self.counted())
    }

    fn push_detail(&mut self, detail: String) {
        if self.failure_details.len() < 5 {
            self.failure_details.push(detail);
        }
    }

    /// The single scan-level terminal state writer.
    fn finalize(&self, db_path: &Path, scan_id: &str, total: usize) {
        let deferred = self.deferred(total);
        let gaps = self.completed_with_gaps;
        let failure_suffix = if self.failure_details.is_empty() {
            String::new()
        } else {
            format!("；报错细节：{}", self.failure_details.join("；"))
        };
        let uninterrupted = self.failed == 0 && self.limited == 0 && self.partial == 0 && deferred == 0;
        if uninterrupted && gaps == 0 {
            sentinel_scan_update(
                db_path,
                scan_id,
                "completed",
                &format!(
                    "本轮执行完成：自动验证 {}，确定性侦察收口 {}，复杂前端自动收口 {}，无异常中断",
                    self.completed, self.skipped, self.manual_review
                ),
            );
        } else if uninterrupted {
            // §10: the run answered every target, but some of them closed with
            // declared holes. That is finished work with visible gaps, and the
            // wording has to say so instead of "无异常中断".
            sentinel_scan_update(
                db_path,
                scan_id,
                "completed",
                &format!(
                    "本轮执行完成但存在覆盖缺口：自动验证 {}，带覆盖缺口完成 {}，确定性侦察收口 {}，复杂前端自动收口 {}；缺口未计入无异常完成{failure_suffix}",
                    self.completed, gaps, self.skipped, self.manual_review
                ),
            );
        } else if self.completed + gaps + self.partial + self.skipped + self.manual_review > 0 {
            let gap_note = if gaps > 0 {
                format!("，带覆盖缺口完成 {gaps}")
            } else {
                String::new()
            };
            let summary = if self.failed == 0 && self.limited == 0 && deferred == 0 {
                format!(
                    "本轮未完整结束：自动验证 {}{gap_note}，待补充验证 {}，确定性侦察收口 {}，复杂前端自动收口 {}；无执行失败，待补充与缺口项未计入自动验证完成{failure_suffix}",
                    self.completed, self.partial, self.skipped, self.manual_review
                )
            } else {
                format!(
                    "本轮执行异常：自动验证 {}{gap_note}，待补充验证 {}，确定性侦察收口 {}，复杂前端自动收口 {}，熔断 {}，执行失败 {}，未处理 {deferred}{failure_suffix}",
                    self.completed,
                    self.partial,
                    self.skipped,
                    self.manual_review,
                    self.limited,
                    self.failed
                )
            };
            sentinel_scan_update(db_path, scan_id, "partial", &summary);
        } else {
            sentinel_scan_update(
                db_path,
                scan_id,
                "failed",
                &format!(
                    "流水线没有有效完成目标：可重试无进展 {}，失败 {}，未处理 {deferred}{failure_suffix}",
                    self.limited, self.failed
                ),
            );
        }
    }
}

/// Route modes that never reach a backend; recorded here so both pipelines
/// share one implementation.
fn record_agent_routing_skip(
    db_path: &Path,
    scan_id: &str,
    route: &FrontendRoute,
    position: usize,
    total: usize,
    tally: &mut AgentPipelineTally,
) -> bool {
    if route.mode == "manual_review" {
        tally.manual_review += 1;
        update_target_route(db_path, scan_id, route, "manual_review");
        sentinel_scan_update(
            db_path,
            scan_id,
            "scanning",
            &format!(
                "目标 {position}/{total} · 复杂前端已完成本地证据提取，转人工分析 · {}",
                route.reason_text()
            ),
        );
        return false;
    }
    if route.mode == "skip" {
        tally.skipped += 1;
        update_target_route(db_path, scan_id, route, "recon_only");
        sentinel_scan_update(
            db_path,
            scan_id,
            "scanning",
            &format!(
                "目标 {position}/{total} · 前端价值 {} 分，跳过模型调查 · {}",
                route.score,
                route.reason_text()
            ),
        );
        return false;
    }
    true
}

/// The terminal state the reducer committed for this target's run, if any.
fn reduced_terminal_status(db_path: &Path, scan_id: &str, route: &FrontendRoute) -> Option<&'static str> {
    use crate::agent_runtime::contract::TerminalState;
    let connection = db::open(db_path).ok()?;
    let attempt_number: i64 = connection
        .query_row(
            "SELECT attempt_count FROM sentinel_scans WHERE id=?1",
            [scan_id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(1)
        .max(1);
    let stored: String = connection
        .query_row(
            "SELECT terminal_state FROM agent_runs WHERE scan_id=?1 AND attempt_number=?2 AND target_url=?3 ORDER BY created_at DESC LIMIT 1",
            params![scan_id, attempt_number, route.url],
            |row| row.get(0),
        )
        .unwrap_or_default();
    TerminalState::parse(stored.trim()).map(TerminalState::to_sentinel_status)
}

/// Facts this attempt owes the runtime store, independent of the outcome: the
/// frozen plan, its hashes and its four budget ceilings. §11 requires the plan
/// budgets to land on the run row, otherwise `agent_runs` reports a 0 ceiling and
/// the usage columns cannot be read as a ratio.
fn runtime_report(
    db_path: &Path,
    scan_id: &str,
    route: &FrontendRoute,
) -> crate::agent_runtime::strix_adapter::BackendReport {
    use crate::agent_runtime::store::stable_hash;
    use crate::agent_runtime::strix_adapter::BackendReport;
    let attempt_number: i64 = db::open(db_path)
        .ok()
        .and_then(|connection| {
            connection
                .query_row(
                    "SELECT attempt_count FROM sentinel_scans WHERE id=?1",
                    [scan_id],
                    |row| row.get::<_, i64>(0),
                )
                .ok()
        })
        .unwrap_or(1)
        .max(1);
    let plan_json = frozen_plan_of(db_path, scan_id, attempt_number, &route.url);
    let backend = plan_json
        .get("backend")
        .and_then(JsonValue::as_str)
        .and_then(AgentBackendKind::parse)
        .unwrap_or(AgentBackendKind::Strix);
    let mut report = BackendReport::new(scan_id, attempt_number, route.url.clone(), backend);
    report.plan_hash = stable_hash(&plan_json.to_string());
    report.plan_json = plan_json.clone();
    let budgets = plan_json
        .get("budgets")
        .cloned()
        .unwrap_or(JsonValue::Null);
    let budget = |key: &str| -> i64 {
        budgets
            .get(key)
            .and_then(JsonValue::as_i64)
            .unwrap_or(0)
    };
    report.soft_token_budget = budget("softUncachedTokens");
    report.hard_token_budget = budget("hardTotalTokens");
    report.soft_request_budget = budget("softModelRequests");
    report.hard_request_budget = budget("hardModelRequests");
    let native_state = NativeAgentState::read(db_path, scan_id, &route.url);
    report.evidence_hash = native_state
        .as_ref()
        .map(|value| value.evidence_hash.clone())
        .unwrap_or_default();
    report.required_families = AGENT_COVERAGE_FAMILIES
        .iter()
        .map(|value| value.to_string())
        .collect();
    report
}

/// Register the run before the backend starts, so a target that crashes, is
/// paused or is cancelled still has a `running` row with its plan and budgets.
/// Re-opening an unfinished run reuses the same row instead of forking history,
/// and hands back the ledger the loop mirrors its live facts into.
fn runtime_open_run(
    db_path: &Path,
    scan_id: &str,
    route: &FrontendRoute,
) -> Option<AgentRunLedger> {
    use crate::agent_runtime::strix_adapter;
    let Ok(connection) = db::open(db_path) else {
        return None;
    };
    let report = runtime_report(db_path, scan_id, route);
    strix_adapter::open_run(&connection, &report)
        .ok()
        .map(|run_id| AgentRunLedger {
            db_path: db_path.to_path_buf(),
            run_id,
        })
}

/// Phase 0 wiring: the runtime reduces the terminal state into
/// `agent_runs`/`agent_events`. Legacy status columns are written elsewhere and
/// unchanged, so current scan behaviour stays as it was.
fn record_runtime_terminal_facts(
    db_path: &Path,
    scan_id: &str,
    route: &FrontendRoute,
    outcome: &AgentTargetOutcome,
) {
    use crate::agent_runtime::strix_adapter;
    let Ok(connection) = db::open(db_path) else {
        return;
    };
    let mut report = runtime_report(db_path, scan_id, route);
    let native_state = NativeAgentState::read(db_path, scan_id, &route.url);
    report.pending_contracts = native_state
        .as_ref()
        .map(|value| value.pending_queue.len() as i64)
        .unwrap_or(0);
    report.completed_contracts = native_state
        .as_ref()
        .map(|value| value.completed_contract_keys.clone())
        .unwrap_or_default();
    report.target_requests = native_state
        .as_ref()
        .map(|value| value.target_requests)
        .unwrap_or(0);
    // The completion only exists on the two *Completed states; a limited,
    // cancelled or interrupted run still owes its real spend, which the native
    // checkpoint already carries.
    report.usage = match outcome.completion() {
        Some(completion) => crate::agent_runtime::store::UsageDelta {
            total_tokens: completion.total_tokens,
            model_requests: completion.model_requests,
            ..Default::default()
        },
        None => native_state
            .as_ref()
            .map(|value| crate::agent_runtime::store::UsageDelta {
                input_tokens: value.token_usage.input_tokens,
                cached_input_tokens: value.token_usage.cached_input_tokens,
                output_tokens: value.token_usage.output_tokens,
                total_tokens: value.token_usage.total_tokens,
                model_requests: value.token_usage.model_requests,
            })
            .unwrap_or_default(),
    };
    if let Some(completion) = outcome.completion() {
        report.covered_families = completion.covered_families.clone();
        report.evidence_records = completion.verified_tool_results;
        report.confirmed_findings = completion.confirmed_findings;
        report.ledger_closed = completion.ledger_reported;
        // The ledger's own accounting is the requirement: families it proved plus
        // families it named as gaps. Anything the close-out declared not applicable
        // is therefore neither a gap nor a coverage claim (§9.5).
        report.required_families = completion
            .covered_families
            .iter()
            .chain(completion.uncovered_families.iter())
            .cloned()
            .collect();
    }
    let stop = outcome.stop();
    if let Some(stop) = stop {
        let reason = stop.reason.clone();
        match stop.code {
            // §5.2: the reduced state has to say "the local record failed", or the
            // projection below would blame the model for a disk or database error.
            AGENT_STOP_PERSISTENCE => report.persistence_failure = Some(reason),
            AGENT_STOP_CONFIGURATION | AGENT_STOP_EVIDENCE_INTEGRITY => {
                report.configuration_error = Some(reason)
            }
            AGENT_STOP_WAF | AGENT_STOP_RATE_LIMIT | AGENT_STOP_SCOPE => {
                report.protection_signal = Some(reason)
            }
            AGENT_STOP_HARD_TOKENS | AGENT_STOP_HARD_REQUESTS => {
                report.hard_limit_reason = Some(reason)
            }
            AGENT_STOP_SOFT_TOKENS | AGENT_STOP_SOFT_REQUESTS | AGENT_STOP_NO_PROGRESS => {
                report.soft_budget_stall_reason = Some(reason)
            }
            AGENT_STOP_UNSUPPORTED => report.unsupported_capability = Some(reason),
            AGENT_STOP_RESUME_INCOMPATIBLE => report.resume_incompatible = Some(reason),
            _ => report.detail = reason,
        }
    }
    report.cancelled = matches!(outcome, AgentTargetOutcome::Cancelled);
    // The checkpoint is the authority on whether this attempt can continue, so
    // the run row agrees with it instead of claiming a terminal state early.
    report.resumable = native_state
        .as_ref()
        .map(|value| value.terminal_reason.is_empty())
        .unwrap_or(false)
        && matches!(
            outcome,
            AgentTargetOutcome::Incomplete(_) | AgentTargetOutcome::Cancelled
        );
    if let Ok(run_id) = strix_adapter::open_run(&connection, &report) {
        let _ = strix_adapter::close_run(&connection, &run_id, &report);
    }
}

/// Apply one backend outcome to target state and the tally.
/// Returns `false` when the pipeline must stop (user cancelled).
fn record_agent_target_outcome(
    db_path: &Path,
    scan_id: &str,
    route: &FrontendRoute,
    outcome: AgentTargetOutcome,
    tally: &mut AgentPipelineTally,
) -> bool {
    let reason = outcome.detail();
    record_runtime_terminal_facts(db_path, scan_id, route, &outcome);
    // §4.2: the reducer decides the terminal state and `agent_runs` holds it; the
    // legacy columns are a projection of that decision. Only when the run stayed
    // open (a pause, a resumable stop, or no run row at all) does the outcome
    // itself supply the state, so the two can never disagree in the other direction.
    let outcome_status = reduced_terminal_status(db_path, scan_id, route)
        .unwrap_or_else(|| outcome.terminal_status());
    // One traceable terminal record per target, whatever produced it.
    // Bookkeeping after the attempt is over: nothing further can be spent, so a
    // failure here is recorded in the log rather than changing the outcome.
    let _ = write_agent_checkpoint(
        db_path,
        scan_id,
        &route.url,
        "agent_terminal",
        &serde_json::json!({
            "code": outcome.terminal_code(),
            "status": outcome.terminal_status(),
            "detail": reason,
            "stop": outcome.stop().map(|stop| serde_json::json!({"code": stop.code, "reason": stop.reason})),
            "completion": outcome.completion().map(|completion| serde_json::json!({
                "coveredFamilies": completion.covered_families,
                "uncoveredFamilies": completion.uncovered_families,
                "confirmedFindings": completion.confirmed_findings,
                "ledgerReported": completion.ledger_reported,
            })),
        }),
    );
    match outcome {
        AgentTargetOutcome::Completed(completion) => {
            tally.completed += 1;
            let mut completed_route = route.clone();
            if completion.ledger_reported {
                completed_route
                    .reasons
                    .push(format!("覆盖账本：{}", completion.summary));
            }
            update_target_route(db_path, scan_id, &completed_route, outcome_status);
        }
        AgentTargetOutcome::BoundedCompleted(_) => {
            tally.completed_with_gaps += 1;
            let mut completed_route = route.clone();
            completed_route.reasons.push(format!("有界调查已完成：{reason}"));
            update_target_route(db_path, scan_id, &completed_route, outcome_status);
        }
        AgentTargetOutcome::Incomplete(stop) => {
            tally.partial += 1;
            let mut incomplete_route = route.clone();
            tally.push_detail(format!("{}：{}", route.url, stop.reason));
            incomplete_route.reasons.push(format!(
                "自动验证尚未取得目标请求/响应；前端证据已保留，可重试未完成阶段：{}",
                stop.reason
            ));
            update_target_route(db_path, scan_id, &incomplete_route, outcome_status);
        }
        AgentTargetOutcome::Limited(stop) => {
            let mut stopped_route = route.clone();
            if stop.requires_fuse() {
                tally.limited += 1;
                stopped_route
                    .reasons
                    .push(format!("确认拦截并熔断：{}", stop.reason));
                update_target_route(db_path, scan_id, &stopped_route, outcome_status);
                add_target_to_fuse_zone(db_path, scan_id, &route.url, &stop.reason);
            } else {
                tally.partial += 1;
                tally.push_detail(format!("{}：{}", route.url, stop.reason));
                stopped_route.reasons.push(format!(
                    "本地模型资源策略需要调整；前端证据已保留，可重试未完成阶段：{}",
                    stop.reason
                ));
                // A limit that is not a protection stop (local model capacity) is
                // resumable work, so it takes the reducer's paused spelling.
                update_target_route(
                    db_path,
                    scan_id,
                    &stopped_route,
                    crate::agent_runtime::contract::TerminalState::Incomplete.to_sentinel_status(),
                );
            }
        }
        AgentTargetOutcome::Failed(stop)
            if stop.code != AGENT_STOP_PERSISTENCE
                && (strix_configuration_failure(&stop.reason)
                    || strix_retryable_provider_failure(&stop.reason)) =>
        {
            tally.failed += 1;
            let mut failed_route = route.clone();
            tally.push_detail(format!("{}：{}", route.url, stop.reason));
            failed_route.reasons.push(format!(
                "模型服务不可用或配置错误，自动流程无法继续；已保留完整前端侦察结果：{}",
                stop.reason
            ));
            update_target_route(db_path, scan_id, &failed_route, outcome_status);
        }
        AgentTargetOutcome::Failed(stop) => {
            tally.failed += 1;
            let mut failed_route = route.clone();
            let detail = format!("{}：{}", route.url, stop.reason);
            tally.push_detail(detail.clone());
            failed_route.reasons.push(detail);
            update_target_route(db_path, scan_id, &failed_route, outcome_status);
        }
        // §11: a continuation that cannot inherit its parent's state is its own
        // terminal state. It is never reported as a model or tool failure, and the
        // target is excluded from automatic resume so only 重新执行 can pick it up.
        AgentTargetOutcome::ResumeIncompatible(stop) => {
            tally.manual_review += 1;
            let mut stopped_route = route.clone();
            tally.push_detail(format!("{}：{}", route.url, stop.reason));
            stopped_route
                .reasons
                .push(format!("续跑状态不兼容，需要重新执行：{}", stop.reason));
            update_target_route(db_path, scan_id, &stopped_route, outcome_status);
        }
        AgentTargetOutcome::Cancelled => return false,
    }
    true
}
