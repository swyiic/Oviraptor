/// Append one runtime event for the run that owns this attempt and target. Used
/// for facts that must survive even when no ledger handle exists yet, such as a
/// checkpoint migration (§4.2). Silently does nothing when the run row is absent.
fn append_native_event(
    db_path: &Path,
    scan_id: &str,
    attempt_number: i64,
    target_url: &str,
    kind: crate::agent_runtime::contract::AgentEventKind,
    payload: JsonValue,
) {
    use crate::agent_runtime::{contract::AgentRole, store};
    let Ok(connection) = db::open(db_path) else {
        return;
    };
    let Some(run) = store::find_run(&connection, scan_id, attempt_number, target_url, AgentRole::Coordinator)
        .ok()
        .flatten()
    else {
        return;
    };
    let _ = store::append_event(&connection, &run.id, kind, &payload, &[]);
}

/// Task-level identity handles for this scan. Handles only — the credentials
/// behind them never enter the plan, the prompt or an artifact index.
fn scan_identity_handles(db_path: &Path, scan_id: &str) -> Vec<String> {
    let handles = db::open(db_path)
        .ok()
        .and_then(|connection| scan_identity_keys(&connection, scan_id).ok())
        .unwrap_or_default();
    if handles.is_empty() {
        vec!["anonymous".to_string()]
    } else {
        handles
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AgentExecutionPlan {
    backend: AgentBackendKind,
    mode: String,
    surface: String,
    /// Which attempt this snapshot is being executed by. It is deliberately kept
    /// out of `hash()`: a continuation refreshes only this field (§3.4).
    attempt_number: i64,
    /// Frozen with the plan so a continuation cannot be pointed at another host.
    target_url: String,
    /// Task-level identity handles, never the credentials behind them.
    identities: Vec<String>,
    /// `local` or `cloud`; the provider class is part of the frozen contract.
    model_provider: String,
    timeout_seconds: u64,
    soft_uncached_tokens: i64,
    hard_total_tokens: i64,
    soft_model_requests: i64,
    hard_model_requests: i64,
    max_turns: i64,
    contract_limit: i64,
    discovery_passes: i64,
    verifier_limit: i64,
    no_progress_window: i64,
    /// Registrable domains this attempt may be probed. Frozen with the plan so a
    /// later evidence refresh can never widen the authorized scope (§13).
    allowed_origins: Vec<String>,
}

impl AgentExecutionPlan {
    fn as_json(&self) -> JsonValue {
        let mut plan = self.frozen_json();
        if let Some(object) = plan.as_object_mut() {
            object.insert("attemptNumber".to_string(), serde_json::json!(self.attempt_number));
            object.insert("targetUrl".to_string(), serde_json::json!(self.target_url));
        }
        plan
    }

    /// Everything a continuation must inherit verbatim. Attempt identity and run
    /// timestamps stay out so the hash is stable across the logical chain (§3.4).
    fn frozen_json(&self) -> JsonValue {
        serde_json::json!({
            "schemaVersion": 1,
            "owner": "oviraptor",
            "backend": self.backend.as_str(),
            "mode": self.mode,
            "surface": self.surface,
            "identities": self.identities,
            "modelProvider": self.model_provider,
            "timeoutSeconds": self.timeout_seconds,
            "budgets": {
                "softUncachedTokens": self.soft_uncached_tokens,
                "hardTotalTokens": self.hard_total_tokens,
                "softModelRequests": self.soft_model_requests,
                "hardModelRequests": self.hard_model_requests,
                "maxTurns": self.max_turns
            },
            "coverage": {
                "contractLimit": self.contract_limit,
                "discoveryPasses": self.discovery_passes,
                "verifierLimit": self.verifier_limit,
                "requiredFamilies": AGENT_COVERAGE_FAMILIES,
            },
            "allowedOrigins": self.allowed_origins,
            "stopping": {
                "softBudgetsRequireNoProgress": true,
                "noProgressWindow": self.no_progress_window,
                "wafChallengeStopsImmediately": true,
                "ordinary401Or403DoesNotStopTarget": true
            }
        })
    }

    fn hash(&self) -> String {
        agent_stable_hash(&self.frozen_json())
    }

    /// The child snapshot of a frozen plan: identical content, new attempt.
    fn with_attempt(&self, attempt_number: i64) -> Self {
        Self {
            attempt_number,
            ..self.clone()
        }
    }

    fn from_json(value: &JsonValue) -> Option<Self> {
        if value.get("owner").and_then(JsonValue::as_str) != Some("oviraptor") {
            return None;
        }
        let text = |key: &str| -> Option<String> {
            value.get(key).and_then(JsonValue::as_str).map(str::to_string)
        };
        let number = |node: &JsonValue, key: &str| -> i64 {
            node.get(key).and_then(JsonValue::as_i64).unwrap_or(0)
        };
        let budgets = value.get("budgets")?;
        let coverage = value.get("coverage")?;
        let stopping = value.get("stopping")?;
        Some(Self {
            backend: AgentBackendKind::parse(&text("backend")?)?,
            mode: text("mode")?,
            surface: text("surface")?,
            attempt_number: number(value, "attemptNumber"),
            target_url: text("targetUrl").unwrap_or_default(),
            identities: value
                .get("identities")
                .and_then(JsonValue::as_array)
                .map(|rows| rows.iter().filter_map(JsonValue::as_str).map(str::to_string).collect())
                .unwrap_or_default(),
            model_provider: text("modelProvider")?,
            timeout_seconds: number(value, "timeoutSeconds").max(0) as u64,
            soft_uncached_tokens: number(budgets, "softUncachedTokens"),
            hard_total_tokens: number(budgets, "hardTotalTokens"),
            soft_model_requests: number(budgets, "softModelRequests"),
            hard_model_requests: number(budgets, "hardModelRequests"),
            max_turns: number(budgets, "maxTurns"),
            contract_limit: number(coverage, "contractLimit"),
            discovery_passes: number(coverage, "discoveryPasses"),
            verifier_limit: number(coverage, "verifierLimit"),
            no_progress_window: number(stopping, "noProgressWindow"),
            allowed_origins: value
                .get("allowedOrigins")
                .and_then(JsonValue::as_array)
                .map(|rows| rows.iter().filter_map(JsonValue::as_str).map(str::to_string).collect())
                .unwrap_or_default(),
        })
    }
}

/// Native loop budgets. The native backend issues fewer, denser model requests
/// than Strix because Oviraptor itself drives the tool calls, so the hard
/// request ceiling is tighter while the coverage limits stay identical.
fn native_agent_limits(
    local: bool,
    mode: &str,
) -> (u64, i64, i64, i64, i64, i64) {
    match (local, mode) {
        (true, "deep") => (2_400, 400_000, 900_000, 16, 22, 26),
        (true, "standard") => (1_500, 200_000, 500_000, 10, 14, 18),
        (true, _) => (900, 100_000, 250_000, 6, 8, 10),
        (false, "deep") => (1_800, 600_000, 1_600_000, 24, 40, 48),
        (false, "standard") => (1_200, 300_000, 900_000, 14, 22, 28),
        (false, _) => (600, 150_000, 400_000, 8, 12, 14),
    }
}

/// The registrable domains this attempt may probe, frozen at plan time.
/// Only two authorities count: the target's own domain and the domains the
/// project's ownership profile explicitly approved. Evidence content is never
/// consulted here, so a third-party API captured by CDP cannot widen scope.
fn agent_allowed_origins(db_path: &Path, scan_id: &str, target_url: &str) -> Vec<String> {
    let mut origins = vec![normalize_ownership_domain(target_url)];
    let Ok(connection) = db::open(db_path) else {
        return origins;
    };
    let project_id: i64 = connection
        .query_row(
            "SELECT project_id FROM sentinel_scans WHERE id=?1",
            [scan_id],
            |row| row.get::<_, Option<i64>>(0),
        )
        .ok()
        .flatten()
        .unwrap_or(0);
    if project_id > 0 {
        let rows: Vec<(String, String)> = connection
            .prepare(
                "SELECT approved_domains_json, excluded_domains_json FROM asset_ownership_profiles WHERE project_id=?1",
            )
            .ok()
            .map(|mut statement| {
                statement
                    .query_map([project_id], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })
                    .ok()
                    .and_then(|rows| rows.flatten().next())
                    .map(|row| vec![row])
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        if let Some((approved, excluded)) = rows.first() {
            let list = |text: &str| -> Vec<String> {
                json(text.to_string())
                    .as_array()
                    .map(|rows| {
                        rows.iter()
                            .filter_map(JsonValue::as_str)
                            .map(normalize_ownership_domain)
                            .filter(|value| !value.is_empty())
                            .collect()
                    })
                    .unwrap_or_default()
            };
            let excluded = list(excluded);
            for domain in list(approved) {
                if excluded.iter().any(|value| domain_matches(&domain, value)) {
                    continue;
                }
                if !origins.contains(&domain) {
                    origins.push(domain);
                }
            }
        }
    }
    origins
}

fn build_agent_execution_plan(
    adaptive: &AdaptiveStrixSettings,
    route: &FrontendRoute,
    environment: &StrixRuntimeEnv,
    backend: AgentBackendKind,
    db_path: &Path,
    scan_id: &str,
) -> AgentExecutionPlan {
    let allowed_origins = agent_allowed_origins(db_path, scan_id, &route.url);
    // §3.5: the plan freezes which attempt it belongs to, the exact target, the
    // task-level identity handles and the provider class, so a continuation can
    // inherit all of it instead of recomputing from today's settings.
    let frozen = AgentExecutionPlan {
        backend,
        mode: route.mode.clone(),
        surface: route.surface.clone(),
        attempt_number: agent_attempt_number(db_path, scan_id),
        target_url: route.url.clone(),
        identities: scan_identity_handles(db_path, scan_id),
        model_provider: if environment.deployment == "local" {
            "local".to_string()
        } else {
            "cloud".to_string()
        },
        timeout_seconds: 0,
        soft_uncached_tokens: 0,
        hard_total_tokens: 0,
        soft_model_requests: 0,
        hard_model_requests: 0,
        max_turns: 0,
        contract_limit: 0,
        discovery_passes: 0,
        verifier_limit: 0,
        no_progress_window: 0,
        allowed_origins: allowed_origins.clone(),
    };
    if route.mode == "manual_review" {
        return frozen;
    }

    let (configured_timeout, configured_tokens, configured_requests) = adaptive.limits(&route.mode);
    if route.surface == "static_frontend" {
        return AgentExecutionPlan {
            backend,
            mode: route.mode.clone(),
            surface: route.surface.clone(),
            timeout_seconds: configured_timeout.min(90),
            soft_uncached_tokens: configured_tokens.clamp(25_000, 50_000),
            hard_total_tokens: 100_000,
            soft_model_requests: 2,
            hard_model_requests: 4,
            max_turns: 6,
            contract_limit: 4,
            discovery_passes: 1,
            verifier_limit: 1,
            no_progress_window: 2,
            allowed_origins: allowed_origins.clone(),
            ..frozen.clone()
        };
    }

    let local = environment.deployment == "local";
    let (timeout_cap, token_default, hard_tokens, request_default, hard_requests, max_turns) =
        if backend == AgentBackendKind::Native {
            native_agent_limits(local, &route.mode)
        } else {
            match (local, route.mode.as_str()) {
                (true, "deep") => (3_600, 700_000, 1_400_000, 16, 24, 28),
                (true, "standard") => (2_400, 400_000, 800_000, 12, 18, 22),
                (true, _) => (1_200, 200_000, 400_000, 6, 10, 14),
                (false, "deep") => (2_400, 800_000, 6_000_000, 24, 72, 76),
                (false, "standard") => (1_200, 400_000, 2_000_000, 14, 36, 40),
                (false, _) => (600, 200_000, 600_000, 6, 12, 16),
            }
        };
    let full_power_multiplier = if environment.full_power { 2 } else { 1 };
    let soft_requests = configured_requests.max(request_default).max(1);
    let hard_requests = (hard_requests * full_power_multiplier)
        .min(if local { 32 } else { 96 })
        .max(soft_requests);
    let timeout = configured_timeout
        .max(if local { timeout_cap / 2 } else { timeout_cap })
        .min(timeout_cap * full_power_multiplier as u64);
    let soft_tokens = if configured_tokens <= 0 {
        token_default
    } else {
        configured_tokens.max(token_default)
    };

    AgentExecutionPlan {
        backend,
        mode: route.mode.clone(),
        surface: route.surface.clone(),
        timeout_seconds: timeout,
        soft_uncached_tokens: soft_tokens,
        hard_total_tokens: hard_tokens * full_power_multiplier,
        soft_model_requests: soft_requests,
        hard_model_requests: hard_requests.max(soft_requests),
        max_turns: (max_turns * full_power_multiplier).min(if local { 40 } else { 100 }),
        contract_limit: web_mode_contract_limit(&route.mode),
        discovery_passes: web_mode_discovery_passes(&route.mode),
        verifier_limit: web_mode_verifier_limit(&route.mode),
        allowed_origins,
        no_progress_window: adaptive.no_tool_turn_limit.clamp(
            match route.mode.as_str() {
                "deep" => 4,
                "standard" => 3,
                _ => 2,
            },
            12,
        ),
        ..frozen
    }
}

/// The plan one attempt runs on. A continuation inherits the parent's frozen
/// snapshot and refreshes only the attempt identity; rebuilding from today's
/// settings is what "重新执行" is for (§3.4, §3.5).
#[allow(clippy::too_many_arguments)]
fn agent_frozen_plan(
    db_path: &Path,
    scan_id: &str,
    url: &str,
    route: &FrontendRoute,
    environment: &StrixRuntimeEnv,
    adaptive: &AdaptiveStrixSettings,
    backend: AgentBackendKind,
    attempt_number: i64,
) -> Result<AgentExecutionPlan, NativeStateRejection> {
    let lineage = agent_attempt_lineage(db_path, scan_id, attempt_number);
    if lineage.resume_kind == AgentResumeKind::ContinueIncomplete {
        // Only a continuation reads another attempt's record, and only its parent's.
        let parent = lineage.parent_attempt_number.unwrap_or(attempt_number);
        let stored = frozen_plan_of(db_path, scan_id, parent, url);
        if let Some(inherited) = AgentExecutionPlan::from_json(&stored) {
            let plan = inherited.with_attempt(attempt_number);
            persist_agent_execution_plan(db_path, scan_id, attempt_number, url, &plan).map_err(
                |error| NativeStateRejection {
                    kind: NativeStateRejectionKind::PersistenceFailed,
                    parent_attempt_number: lineage.parent_attempt_number,
                    current_attempt_number: attempt_number,
                    detail: error,
                },
            )?;
            return Ok(plan);
        }
        return Err(NativeStateRejection {
            kind: NativeStateRejectionKind::NoCheckpoint,
            parent_attempt_number: lineage.parent_attempt_number,
            current_attempt_number: attempt_number,
            detail: "该 URL 没有可继承的冻结执行计划".to_string(),
        });
    }
    // initial / fresh: today's settings decide, and the plan is frozen for this
    // attempt only. Nothing from the previous attempt is read here (Phase 2 §2.2).
    let plan =
        build_agent_execution_plan(adaptive, route, environment, backend, db_path, scan_id)
            .with_attempt(attempt_number);
    persist_agent_execution_plan(db_path, scan_id, attempt_number, url, &plan).map_err(
        |error| NativeStateRejection {
            kind: NativeStateRejectionKind::PersistenceFailed,
            parent_attempt_number: lineage.parent_attempt_number,
            current_attempt_number: attempt_number,
            detail: error,
        },
    )?;
    Ok(plan)
}

/// Persist the frozen plan twice: on the attempt's own `agent_runs` row (the
/// authority, Phase 2 §2.2) and on the legacy per-URL checkpoint row, which is only a
/// compatibility projection for the detail panel. Neither write deletes history.
fn persist_agent_execution_plan(
    db_path: &Path,
    scan_id: &str,
    attempt_number: i64,
    url: &str,
    plan: &AgentExecutionPlan,
) -> Result<(), String> {
    if let Ok(connection) = db::open(db_path) {
        let _ = crate::agent_runtime::store::record_attempt_plan(
            &connection,
            scan_id,
            attempt_number,
            url,
            plan.backend,
            &plan.hash(),
            &plan.as_json(),
        );
    }
    write_agent_checkpoint(db_path, scan_id, url, "agent_execution_plan", &plan.as_json())
}

/// The plan one attempt froze, preferring the attempt-scoped record. The per-URL
/// projection is only a fallback for rows written before Phase 2.
fn frozen_plan_of(db_path: &Path, scan_id: &str, attempt_number: i64, url: &str) -> JsonValue {
    attempt_plan_of(db_path, scan_id, attempt_number, url)
        .unwrap_or_else(|| read_agent_checkpoint(db_path, scan_id, url, "agent_execution_plan"))
}

/// The attempt-scoped record only, with no fallback. Selection has to use this: a
/// stale per-URL projection is exactly what pinned a fresh attempt to the previous
/// attempt's backend (Phase 2 §2.1).
fn attempt_plan_of(
    db_path: &Path,
    scan_id: &str,
    attempt_number: i64,
    url: &str,
) -> Option<JsonValue> {
    let connection = db::open(db_path).ok()?;
    crate::agent_runtime::store::attempt_plan(&connection, scan_id, attempt_number, url)
}

/// The backend an attempt actually ran on. A settings edit must never switch an
/// attempt that already started, and a fresh attempt must never be pinned by the
/// previous one (Phase 2 §2.1).
fn persisted_attempt_backend(
    db_path: &Path,
    scan_id: &str,
    attempt_number: i64,
    url: &str,
) -> Option<AgentBackendKind> {
    AgentExecutionPlan::from_json(&attempt_plan_of(db_path, scan_id, attempt_number, url)?)
        .map(|plan| plan.backend)
}

/// Facts about the **live** run. The native loop keeps owning its working state,
/// but every model round, tool call and checkpoint is mirrored here so
/// `agent_events`, `tool_invocations` and `agent_snapshots` describe the attempt
/// as it happens instead of only after it closed (§11).
#[derive(Clone, Debug)]
struct AgentRunLedger {
    db_path: PathBuf,
    run_id: String,
}

impl AgentRunLedger {
    fn record(
        &self,
        kind: crate::agent_runtime::contract::AgentEventKind,
        payload: JsonValue,
    ) -> Result<(), String> {
        let connection = db::open(&self.db_path).map_err(|error| error.to_string())?;
        crate::agent_runtime::store::append_event(&connection, &self.run_id, kind, &payload, &[])
            .map(|_| ())
    }

    fn model_round(
        &self,
        turns: i64,
        usage: &AgentTokenUsage,
        tools: &[String],
    ) -> Result<(), String> {
        use crate::agent_runtime::contract::AgentEventKind;
        self.record(
            AgentEventKind::ModelRoundCompleted,
            serde_json::json!({
                "turns": turns,
                "modelRequests": usage.model_requests,
                "totalTokens": usage.total_tokens,
                "uncachedInputTokens": usage.uncached_input(),
                "toolCalls": tools,
            }),
        )
    }

    /// §4.2 step 2: the invocation row exists in `running` state before the tool can
    /// touch the target, so a crash leaves an auditable gap that recovery can close
    /// instead of a call that simply never happened.
    fn begin_tool(
        &self,
        name: &str,
        arguments: &JsonValue,
        invocation_id: &str,
    ) -> Result<i64, String> {
        use crate::agent_runtime::contract::AgentEventKind;
        let connection = db::open(&self.db_path).map_err(|error| error.to_string())?;
        let row = crate::agent_runtime::store::begin_tool_invocation(
            &connection,
            &self.run_id,
            invocation_id,
            name,
            1,
            &value_first(arguments, &["contractKey"]),
            &value_first(arguments, &["identity", "leftIdentity"]),
            arguments,
            "allow",
        )?;
        self.record(
            AgentEventKind::ToolInvocationStarted,
            serde_json::json!({"tool": name, "invocationId": invocation_id}),
        )?;
        Ok(row)
    }

    /// §4.2 step 5-7: the deterministic end state, the response artifact and the
    /// event are committed together. A refused call is recorded too — a log that
    /// only keeps successes cannot answer "why did this contract not close" (§6).
    fn finish_tool(
        &self,
        row: Option<i64>,
        name: &str,
        arguments: &JsonValue,
        result: &JsonValue,
        invocation_id: &str,
    ) -> Result<(), String> {
        use crate::agent_runtime::contract::AgentEventKind;
        let code = value_first(result, &["code"]);
        let status = if code.is_empty() { "completed" } else { "refused" };
        let connection = db::open(&self.db_path).map_err(|error| error.to_string())?;
        let _ = &arguments;
        match row {
            Some(row) => {
                let _ = crate::agent_runtime::store::finish_tool_invocation(
                    &connection,
                    row,
                    status,
                    &result
                        .get("endpoint")
                        .or_else(|| result.get("action"))
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                    &value_first(result, &["rawArtifactId", "responseDifferenceArtifactId"]),
                    &code,
                );
            }
            // The run row was unavailable when the call started; keep the audit
            // anyway so the attempt is still explainable after the fact.
            None => {
                let _ = connection.execute(
                    "INSERT INTO tool_invocations(run_id,invocation_id,tool_name,contract_key,identity_handle,policy_decision,status,progress_signature,response_artifact_id,error_class,finished_at) VALUES(?1,?2,?3,?4,?5,'allow',?6,?7,?8,?9,datetime('now','localtime'))",
                    params![
                        self.run_id,
                        invocation_id,
                        name,
                        value_first(arguments, &["contractKey"]),
                        value_first(arguments, &["identity", "leftIdentity"]),
                        status,
                        result
                            .get("endpoint")
                            .or_else(|| result.get("action"))
                            .map(|value| value.to_string())
                            .unwrap_or_default(),
                        value_first(result, &["rawArtifactId", "responseDifferenceArtifactId"]),
                        code,
                    ],
                );
            }
        }
        self.record(
            AgentEventKind::ToolInvocationCompleted,
            serde_json::json!({
                "tool": name,
                "invocationId": invocation_id,
                "status": status,
                "code": code,
                "artifactId": value_first(result, &["rawArtifactId"]),
            }),
        )
    }

    /// §4.2: recover this run through the ledger — snapshot, replay the newer
    /// events, mark unfinished calls interrupted, and requeue only the contracts that
    /// never produced a determined result. Budget already spent is never refunded.
    fn recover(&self) -> Option<crate::agent_runtime::checkpoint::RecoveredRun> {
        let connection = db::open(&self.db_path).ok()?;
        crate::agent_runtime::checkpoint::recover(&connection, &self.run_id).ok()
    }

    fn checkpoint(&self, state: &NativeAgentState) -> Result<(), String> {
        let connection = db::open(&self.db_path).map_err(|error| error.to_string())?;
        let mut snapshot = crate::agent_runtime::checkpoint::RunState::new(
            self.run_id.clone(),
            state.pending_queue.clone(),
        );
        snapshot.turns = state.turns;
        snapshot.model_requests = state.token_usage.model_requests;
        snapshot.target_requests = state.target_requests;
        snapshot.input_tokens = state.token_usage.input_tokens;
        snapshot.cached_input_tokens = state.token_usage.cached_input_tokens;
        snapshot.output_tokens = state.token_usage.output_tokens;
        snapshot.used_tokens = state.token_usage.total_tokens;
        snapshot.covered_families = state.covered_families.clone();
        snapshot.completed_contracts = state.completed_contract_keys.clone();
        snapshot.evidence_records = state.observed_requests.len() as i64;
        snapshot.confirmed_findings = state.confirmed_findings;
        snapshot.progress_signature = state.progress_signature.clone();
        snapshot.no_progress_streak = state.no_progress_streak;
        crate::agent_runtime::checkpoint::write_checkpoint(&connection, &snapshot)
    }
}
