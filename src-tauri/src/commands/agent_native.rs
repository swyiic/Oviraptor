/// Native agent loop (§6) and its result ingestion (§8).
///
/// Oviraptor drives the model, the tools, the budgets and the terminal state;
/// no Strix process is involved on this path.
const AGENT_SYSTEM_RULES: &str = "你是 Oviraptor 的授权资产安全调查 Agent。你只在当前任务目标与其证据记录的关联业务域名内工作，\
工具调用是唯一执行途径：不得假设未验证的响应，不得声称未做过的事。\n\
规则：\n\
1. 每轮只调用一个或一组互不重复的工具；先读证据再发请求，避免重复侦察。\n\
2. 优先执行已有的高价值契约，其次是运行时正式 API，最后才是定向发现。\n\
3. 普通 401/403 只说明权限边界，记录后继续其他分支；确认 WAF/验证码/机器人挑战必须立即收口。\n\
4. 写请求需要已授权契约、尝试次数、清理步骤与恢复条件，缺一不可。\n\
5. 只有 record_hypothesis_result 的 confirmed（含控制请求、测试请求、响应差异、影响与复现步骤）才是漏洞结论；\
   insufficient_evidence 既不是漏洞，也不算验证成功。\n\
6. 没有发现也是正常终态：必须调用 finish_target 输出覆盖账本、未覆盖原因（不得写成“安全”）、排除项与人工深入建议。\n\
7. 未覆盖的原因必须如实写入覆盖账本，不得为了结束而声称已覆盖。";

/// Which families a route still owes, used both for the initial queue and for a
/// derived terminal state.
fn agent_required_families() -> Vec<&'static str> {
    AGENT_COVERAGE_FAMILIES.to_vec()
}

fn agent_initial_queue(context: &AgentRunContext) -> Vec<String> {
    let mut queue = Vec::new();
    // §6 rule 3: existing high-value contracts first.
    for pointer in ["/investigation/hypotheses", "/opportunities"] {
        let Some(items) = context.evidence.pointer(pointer).and_then(JsonValue::as_array) else {
            continue;
        };
        for item in items {
            let status = value_first(item, &["status"]);
            if !status.is_empty() && !matches!(status.as_str(), "ready" | "candidate" | "in_progress") {
                continue;
            }
            let category = value_first(item, &["category", "kind"]);
            let endpoint = value_first(item, &["endpoint", "path", "url"]);
            if category.is_empty() || endpoint.is_empty() {
                continue;
            }
            queue.push(format!(
                "contract:{category}|{}",
                normalized_investigation_path(&endpoint)
            ));
        }
    }
    // Then the formal runtime APIs actually observed in the browser.
    if let Some(items) = context
        .evidence
        .pointer("/apiCandidates")
        .and_then(JsonValue::as_array)
    {
        for item in items.iter().take(40) {
            if investigation_background_noise(item) {
                continue;
            }
            if !standard_investigation_api(item) {
                continue;
            }
            let verb = value_first(item, &["method"]).to_ascii_uppercase();
            let path = normalized_investigation_path(&value_first(item, &["path", "url"]));
            if verb.is_empty() || path.is_empty() {
                continue;
            }
            queue.push(format!("api:{verb}|{path}"));
        }
    }
    // Then the coverage families that nothing has claimed yet.
    for family in agent_required_families() {
        queue.push(format!("family:{family}"));
    }
    queue
}

fn agent_state_block(
    context: &AgentRunContext,
    runtime: &AgentToolRuntime,
    state: &NativeAgentState,
    queue: &[String],
) -> JsonValue {
    serde_json::json!({
        "role": "user",
        "oviraptorKeep": true,
        "content": serde_json::json!({
            "targetUrl": context.target_url,
            "executionPlan": context.execution_plan.as_json(),
            "identities": context.identities.iter().map(|identity| serde_json::json!({
                "key": identity.key,
                "label": agent_identity_label(&context.identities, &identity.key),
                "anonymous": identity.anonymous,
            })).collect::<Vec<_>>(),
            "budgetUsage": {
                "modelRequests": state.token_usage.model_requests,
                "uncachedInputTokens": state.token_usage.uncached_input(),
                "totalTokens": state.token_usage.total_tokens,
                "targetRequests": runtime.target_requests,
                "discoveryRounds": runtime.discovery_rounds,
                "turns": state.turns,
                "noProgressStreak": state.no_progress_streak
            },
            "coveredFamilies": runtime.families.iter().collect::<Vec<_>>(),
            "closedContracts": state.completed_contract_keys,
            "contractAttempts": queue
                .iter()
                .filter_map(|key| key.strip_prefix("contract:"))
                .map(|contract| {
                    format!(
                        "{contract}={}/{}",
                        state.attempts_for(contract),
                        agent_evidence_contract_attempts(&context.evidence, contract)
                    )
                })
                .take(24)
                .collect::<Vec<_>>(),
            "confirmedFindings": runtime.confirmed_findings,
            "observedEndpoints": runtime.endpoints.iter().take(80).collect::<Vec<_>>(),
            "pendingQueue": queue.iter().take(60).collect::<Vec<_>>(),
            "evidenceDigest": agent_evidence_digest(&context.evidence),
            "capabilities": context.capabilities,
        })
        .to_string(),
    })
}

/// Only the parts of the bundle the model cannot already have used.
fn agent_evidence_digest(evidence: &JsonValue) -> JsonValue {
    let top = |pointer: &str, count: usize| -> Vec<JsonValue> {
        evidence
            .pointer(pointer)
            .and_then(JsonValue::as_array)
            .map(|rows| {
                rows.iter()
                    .take(count)
                    .map(agent_compact_item)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    serde_json::json!({
        "surface": evidence.get("surface"),
        "valueScore": evidence.get("valueScore"),
        "verificationPlan": evidence.get("verificationPlan"),
        "stopRule": evidence.get("stopRule"),
        "apiCandidates": top("/apiCandidates", 12),
        "opportunities": top("/opportunities", 8),
        "sensitiveCandidates": top("/sensitiveCandidates", 8),
        "routeCandidates": top("/routeCandidates", 8),
        "identityDifferences": top("/investigation/identityDifferences", 8),
        "hypotheses": top("/investigation/hypotheses", 8),
    })
}

struct NativeAgentBackend;

impl AgentBackend for NativeAgentBackend {
    fn kind(&self) -> AgentBackendKind {
        AgentBackendKind::Native
    }

    fn execute(&self, context: &AgentRunContext) -> AgentTargetOutcome {
        run_native_agent(context)
    }
}

fn run_native_agent(context: &AgentRunContext) -> AgentTargetOutcome {
    let specs = agent_tool_specs();
    // §7: parameter schemas are compiled once up front, never lazily in the middle
    // of a model round.
    let _ = agent_schema_registry();
    // Local traffic still goes through the LLM hook because it clamps the output,
    // guards the context window and writes the usage ledger. It is no longer the
    // only way to stop a generation: the transport drops the request itself, and
    // dropping the guard when this function returns shuts the upstream socket too
    // (§10, §6.2).
    let _model_proxy = if context.environment.deployment == "local" {
        let policy = local_model_runtime_policy(&context.environment);
        match crate::llm_hook::start(
            &context.environment.api_base,
            &context.environment.api_key,
            &context.target_dir,
            &context.environment.prompt_audit_mode,
            context.proxy.as_deref(),
            policy.max_output_tokens,
            policy.max_context_tokens,
            policy.max_concurrent_requests,
        ) {
            Ok(handle) => handle,
            Err(error) => return AgentTargetOutcome::failed(error),
        }
    } else {
        None
    };
    let mut environment = context.environment.clone();
    if let Some(handle) = &_model_proxy {
        environment.api_base = handle.base_url().to_string();
    }
    let profile = match agent_model_profile(&environment, context.proxy.as_deref()) {
        Ok(value) => value,
        Err(error) => return AgentTargetOutcome::failed(error),
    };
    let client = AgentModelClient::new(profile, &specs);
    let plan = &context.execution_plan;
    let evidence_hash = agent_stable_hash(&context.evidence);
    let plan_hash = plan.hash();
    let full_queue = agent_initial_queue(context);

    // §3.6: an unsafe continuation stops before the first model or target
    // request, so resuming never burns Token on a state the runtime cannot trust.
    if let Some(rejection) = &context.plan_rejection {
        return rejection_outcome(context, rejection);
    }
    let mut state = match NativeAgentState::for_attempt(
        &context.db_path,
        &context.scan_id,
        context,
        &evidence_hash,
        &plan_hash,
    ) {
        Ok(state) => state,
        Err(rejection) => return rejection_outcome(context, &rejection),
    };
    if state.pending_queue.is_empty() {
        state.pending_queue = full_queue.clone();
    }

    // §7: a resumed attempt rebuilds the whole working set from the checkpoint —
    // spent HTTP budget, executed contracts and observed endpoints included.
    let mut runtime = AgentToolRuntime::restore(&state);
    // Tools share the loop's cancel token so an in-flight request stops touching
    // the target as soon as the user pauses or cancels.
    runtime.cancel = Some(agent_scan_cancel_token(
        &context.db_path,
        &context.scan_id,
    ));
    let mut queue: Vec<String> = if context.resume && !state.pending_queue.is_empty() {
        state.pending_queue.clone()
    } else {
        full_queue.clone()
    };
    queue.retain(|key| !runtime.queue_key_satisfied(context, key));
    // §4.2: a continuation first recovers through the run ledger. Unfinished calls
    // become `interrupted` in the audit and only their contracts return to the
    // queue; spent Token, spent HTTP requests and closed contracts stay spent.
    if context.resume {
        if let Some(run) = &context.run {
            if let Some(recovered) = run.recover() {
                for contract in &recovered.state.pending_contracts {
                    let key = format!("contract:{contract}");
                    if !contract.contains('|')
                        || runtime.contract_outcomes.contains_key(contract)
                        || queue.contains(&key)
                    {
                        continue;
                    }
                    queue.push(key.clone());
                    state.pending_queue.push(key);
                }
                if recovered.interrupted_invocations > 0 {
                    append_runner_log(
                        &context.log_path,
                        &format!(
                            "run {} 恢复：重放 {} 条事件，{} 条未完成调用标记为 interrupted，仅其契约重新排队",
                            run.run_id, recovered.events_replayed, recovered.interrupted_invocations
                        ),
                    );
                }
            }
        }
    }

    let mut history = vec![
        serde_json::json!({"role": "system", "content": AGENT_SYSTEM_RULES, "oviraptorKeep": true}),
        agent_state_block(context, &runtime, &state, &queue),
    ];
    loop {
        if sentinel_scan_pause_requested(&context.db_path, &context.scan_id) {
            // A pause must stay resumable: the ledger is written, but no terminal
            // reason is recorded, so the next attempt keeps its spent budget.
            if let Err(error) = native_commit(context, &mut state, &runtime, &queue) {
                return persistence_stop(context, error);
            }
            return AgentTargetOutcome::Cancelled;
        }
        if !sentinel_scan_is_active(&context.db_path, &context.scan_id) {
            state.terminal_reason = AGENT_STOP_USER_CANCELLED.to_string();
            if let Err(error) = native_commit(context, &mut state, &runtime, &queue) {
                return persistence_stop(context, error);
            }
            return AgentTargetOutcome::Cancelled;
        }
        if state.turns >= plan.max_turns.max(1) {
            return native_exit(
                context,
                &mut state,
                &runtime,
                &queue,
                "达到计划内最大轮数",
                AGENT_STOP_DERIVED,
                true,
            );
        }
        if let Some((code, reason)) = agent_budget_stop(context, plan, &state, &runtime) {
            return native_exit(context, &mut state, &runtime, &queue, &reason, code, true);
        }
        let (messages, _compacted) =
            agent_compact_messages(&history, client.profile().request_budget_bytes());
        let cancel = agent_scan_cancel_token(&context.db_path, &context.scan_id);
        let request = agent_model_request(messages, &specs);
        let turn = match client.complete(&request, &cancel) {
            Ok(turn) => turn,
            Err(error) => {
                let detail = error.detail();
                native_sync(context, &mut state, &runtime, &queue);
                if matches!(
                    error,
                    AgentModelError::Network(_) | AgentModelError::Server { .. }
                ) {
                    // §6 rule 9: a transport stall keeps the evidence and stays
                    // resumable, so it must not write a terminal reason.
                    if let Err(error) =
                        state.persist(&context.db_path, &context.scan_id, &context.target_url)
                    {
                        return persistence_stop(context, error);
                    }
                    return AgentTargetOutcome::incomplete(format!(
                        "{detail}；证据与已执行契约已保存，可继续同一尝试"
                    ));
                }
                if !matches!(error, AgentModelError::Cancelled) {
                    state.terminal_reason = detail.clone();
                }
                if matches!(error, AgentModelError::Cancelled) {
                    // §6.2: the request handle has already settled, so the stop is
                    // recorded with the timestamps that prove the transport closed
                    // before any terminal state is written.
                    note_transport_cancel(context, &client);
                }
                if let Err(error) =
                    state.persist(&context.db_path, &context.scan_id, &context.target_url)
                {
                    return persistence_stop(context, error);
                }
                return match error {
                    AgentModelError::Cancelled => AgentTargetOutcome::Cancelled,
                    AgentModelError::UnsupportedCapability(_) => AgentTargetOutcome::failed(format!(
                        "{detail}；模型服务不支持工具调用"
                    )),
                    AgentModelError::Authentication(_) => AgentTargetOutcome::failed(detail),
                    AgentModelError::Protocol(_) => AgentTargetOutcome::failed(detail),
                    AgentModelError::ContextOverflow(_)
                    | AgentModelError::RateLimited(_)
                    | AgentModelError::Timeout(_) => {
                        if let Err(error) =
                            persist_agent_coverage(context, &runtime, &state, &detail)
                        {
                            return persistence_stop(context, error);
                        }
                        AgentTargetOutcome::limited(detail)
                    }
                    AgentModelError::Network(_) | AgentModelError::Server { .. } => {
                        AgentTargetOutcome::incomplete(detail)
                    }
                };
            }
        };
        client.record_usage_line(&context.target_dir, &turn);
        state.token_usage = agent_add_usage(&state.token_usage, &turn.usage);
        if let Err(error) = persist_agent_usage(&context.db_path, &context.scan_id, &turn.usage) {
            return persistence_stop(context, error);
        }
        state.turns += 1;
        let calls = turn.deduplicated_tool_calls();
        if let Some(run) = &context.run {
            if let Err(error) = run.model_round(
                state.turns,
                &state.token_usage,
                &calls.iter().map(|call| call.name.clone()).collect::<Vec<_>>(),
            ) {
                return persistence_stop(context, error);
            }
        }
        history.push(serde_json::json!({
            "role": "assistant",
            "content": if turn.text.is_empty() { JsonValue::String(String::new()) } else { serde_json::json!(turn.text) },
            "tool_calls": calls.iter().map(|call| serde_json::json!({
                "id": call.id, "type": "function",
                "function": {"name": call.name, "arguments": call.arguments.to_string()},
            })).collect::<Vec<_>>(),
        }));
        if calls.is_empty() {
            // §6 rule 10: a normal exit without finish_target must still produce
            // a traceable terminal state derived from the local ledger.
            if turn.text.trim().is_empty() {
                return native_exit(
                    context,
                    &mut state,
                    &runtime,
                    &queue,
                    "模型只回文字、未继续调用工具",
                    AGENT_STOP_DERIVED,
                    false,
                );
            }
            history.push(serde_json::json!({
                "role": "user",
                "content": "不要只用文字结束。请调用工具继续，或调用 finish_target 输出覆盖账本。",
            }));
            state.no_progress_streak += 1;
            if state.no_progress_streak >= plan.no_progress_window.max(1) {
                return native_exit(
                    context,
                    &mut state,
                    &runtime,
                    &queue,
                    "连续多轮没有新增证据",
                    AGENT_STOP_NO_PROGRESS,
                    true,
                );
            }
            if let Err(error) = native_commit(context, &mut state, &runtime, &queue) {
                return persistence_stop(context, error);
            }
            continue;
        }
        let mut finished = false;
        for call in calls {
            // §4.2: the invocation row is opened before the tool can touch the
            // target, closed with its deterministic result, and only then does the
            // answer reach the model.
            let invocation_id = runtime.begin_invocation(context.attempt_number);
            let row = match &context.run {
                Some(run) => match run.begin_tool(&call.name, &call.arguments, &invocation_id) {
                    Ok(row) => Some(row),
                    Err(error) => {
                        // Without the audit row the call cannot be attributed, so it
                        // must not run at all (§5.2).
                        return persistence_stop(context, error);
                    }
                },
                None => None,
            };
            let result = agent_execute_tool_at(
                context,
                &mut runtime,
                &call.name,
                &call.arguments,
                &invocation_id,
            );
            if let Some(run) = &context.run {
                if let Err(error) = run.finish_tool(
                    row,
                    &call.name,
                    &call.arguments,
                    &result.model_view,
                    &result.invocation_id,
                ) {
                    return persistence_stop(context, error);
                }
            }
            if matches!(
                result.model_view.get("code").and_then(JsonValue::as_str),
                Some("evidence_write_failed") | Some("finding_persist_failed")
            ) {
                // §5.2: a tool that could not persist its own evidence ends the
                // attempt at once — no further model round, no further target call.
                return persistence_stop(
                    context,
                    value_first(&result.model_view, &["error"]),
                );
            }
            if call.name == "finish_target"
                && result
                    .model_view
                    .get("accepted")
                    .and_then(JsonValue::as_bool)
                    == Some(true)
            {
                finished = true;
            }
            // §6.1: only the redacted model view is appended to the conversation;
            // the raw bytes stay behind the artifact id.
            history.push(serde_json::json!({
                "role": "tool",
                "tool_call_id": call.id,
                "name": call.name,
                "artifactId": result.artifact_id,
                "content": agent_text_truncated(&result.model_view.to_string(), 24_000),
            }));
        }
        // Queue entries are consumed by what actually ran, never by what the
        // model merely passed as an argument.
        queue.retain(|key| !runtime.queue_key_satisfied(context, key));
        if let Some(stop) = runtime.protected_stop.clone() {
            native_sync(context, &mut state, &runtime, &queue);
            state.terminal_reason = stop.reason.clone();
            let commit = native_commit(context, &mut state, &runtime, &queue)
                .and_then(|_| persist_agent_coverage(context, &runtime, &state, &stop.reason));
            if let Err(error) = commit {
                return persistence_stop(context, error);
            }
            return AgentTargetOutcome::Limited(stop);
        }
        let progress = runtime.take_progress();
        // §7: repeating an already-rejected call is not progress, even if some
        // other counter moved.
        if progress.advanced() && !runtime.repeats_argument_error() {
            state.no_progress_streak = 0;
            state.progress_signature = progress.signature();
            if agent_soft_budget_exceeded(plan, &state) {
                state.last_expansion_reason =
                    format!("已超过软预算，但本轮仍有新增证据：{}", progress.signature());
            }
        } else {
            state.no_progress_streak += 1;
        }
        if finished {
            return native_exit(
                context,
                &mut state,
                &runtime,
                &queue,
                "模型已调用 finish_target 输出覆盖账本",
                AGENT_STOP_FINISH,
                false,
            );
        }
        if state.no_progress_streak >= plan.no_progress_window.max(1) {
            let stalled_reason =
                format!("连续 {} 轮没有新增端点、响应差异或结论", state.no_progress_streak);
            return native_exit(
                context,
                &mut state,
                &runtime,
                &queue,
                &stalled_reason,
                AGENT_STOP_NO_PROGRESS,
                true,
            );
        }
        if let Err(error) = native_commit(context, &mut state, &runtime, &queue) {
            return persistence_stop(context, error);
        }
        // The state block is replaced in place: appending one per round would
        // re-feed the model a growing, already-known snapshot.
        history[1] = agent_state_block(context, &runtime, &state, &queue);
    }
}

/// Copy the working ledger back into the checkpoint. These are the counters a
/// resumed attempt must not get a second time.
fn native_sync(
    context: &AgentRunContext,
    state: &mut NativeAgentState,
    runtime: &AgentToolRuntime,
    queue: &[String],
) {
    state.pending_queue = queue.to_vec();
    // §9.1: only families with a covered ledger entry are recorded as covered;
    // having merely sent a request for a family is "worked", not "covered".
    let mut ledger_covered: Vec<String> = runtime
        .coverage
        .iter()
        .filter(|entry| entry.result == "covered" && !entry.request_record_ids.is_empty())
        .map(|entry| entry.family.clone())
        .collect();
    ledger_covered.sort();
    ledger_covered.dedup();
    state.covered_families = ledger_covered;
    state.observed_requests = runtime.requests.clone();
    state.discovery_rounds = runtime.discovery_rounds as i64;
    state.target_requests = runtime.target_requests as i64;
    state.confirmed_findings = runtime.confirmed_findings;
    state.verdict_keys = sorted_copy(&runtime.verdict_keys);
    state.coverage_evidence = runtime.coverage.clone();
    state.contract_attempts = runtime
        .contract_attempts
        .iter()
        .map(|(key, attempts)| (key.clone(), *attempts))
        .collect();
    state.contract_attempts.sort();
    state.contract_outcomes = runtime
        .contract_outcomes
        .iter()
        .map(|(key, outcome)| (key.clone(), outcome.clone()))
        .collect();
    state.contract_outcomes.sort();
    let completed = state.completed_contracts();
    state.completed_contract_keys = completed;
    let mut exhausted: Vec<String> = runtime
        .contract_attempts
        .iter()
        .filter(|(key, attempts)| {
            **attempts >= agent_evidence_contract_attempts(&context.evidence, key)
                && !runtime.contract_outcomes.contains_key(*key)
        })
        .map(|(key, _)| key.clone())
        .collect();
    exhausted.sort();
    state.exhausted_contract_keys = exhausted;
    state.budget_usage = serde_json::json!({
        "modelRequests": state.token_usage.model_requests,
        "targetRequests": runtime.target_requests,
        "discoveryRounds": runtime.discovery_rounds,
        "contractAttempts": runtime.contract_attempts.len(),
    });
}

/// §6.2: a stop must be provable, not assumed. The transport reports when the
/// cancel was asked for and when the connection was really closed; both land in
/// the run ledger before the terminal state is reduced.
fn note_transport_cancel(context: &AgentRunContext, client: &AgentModelClient) {
    let Some(observation) = client.take_cancel_observation() else {
        return;
    };
    append_runner_log(
        &context.log_path,
        &format!(
            "模型请求已中止：请求于 {} 停止，传输于 {} 关闭（{} 毫秒内落定）",
            observation.cancel_requested_at, observation.transport_closed_at, observation.settle_millis
        ),
    );
    let Some(run) = &context.run else { return };
    if let Err(error) = run.record(
        crate::agent_runtime::contract::AgentEventKind::TransportCancelled,
        serde_json::json!({
            "cancelRequestedAt": observation.cancel_requested_at,
            "transportClosedAt": observation.transport_closed_at,
            "settleMillis": observation.settle_millis,
            "initiator": match observation.initiator {
                crate::agent_runtime::model::CancelInitiator::Peer => "peer",
                crate::agent_runtime::model::CancelInitiator::Local => "task",
            },
        }),
    ) {
        // Nothing further is spent after a stop, so a failed diagnostic write is
        // reported rather than turned into a different terminal state.
        append_runner_log(&context.log_path, &format!("中止记录写入失败：{error}"));
    }
}

fn persistence_stop(context: &AgentRunContext, error: String) -> AgentTargetOutcome {
    append_runner_log(
        &context.log_path,
        &format!("本地记录写入失败，已停止后续模型与目标请求：{error}"),
    );
    // §5.2: the wording the user sees is fixed here, so no call site can report a
    // local write failure as an unexplained stop.
    AgentTargetOutcome::persistence_failure(format!(
        "本地记录失败，已停止以避免重复消耗（{error}）"
    ))
}

/// §3.6 and §5.2 keep their own wording: an incompatible checkpoint asks the user to
/// re-run, a failed local write says the run stopped to avoid double spend.
fn rejection_outcome(
    context: &AgentRunContext,
    rejection: &NativeStateRejection,
) -> AgentTargetOutcome {
    append_runner_log(&context.log_path, &rejection.message());
    if rejection.kind == NativeStateRejectionKind::PersistenceFailed {
        return AgentTargetOutcome::persistence_failure(rejection.message());
    }
    AgentTargetOutcome::resume_incompatible(rejection.message())
}

fn native_commit(
    context: &AgentRunContext,
    state: &mut NativeAgentState,
    runtime: &AgentToolRuntime,
    queue: &[String],
) -> Result<(), String> {
    native_sync(context, state, runtime, queue);
    // §5.2: the round is only over once the checkpoint and its snapshot mirror are
    // both durable. Anything else stops the attempt before more budget is spent.
    state.persist(&context.db_path, &context.scan_id, &context.target_url)?;
    if let Some(run) = &context.run {
        run.checkpoint(state)?;
    }
    Ok(())
}

fn sorted_copy(values: &std::collections::HashSet<String>) -> Vec<String> {
    let mut rows = values.iter().cloned().collect::<Vec<_>>();
    rows.sort();
    rows
}

/// The single terminal exit. §15: the state is derived from the ledger, and only
/// `Incomplete` — the recoverable state — leaves the checkpoint resumable; every
/// other terminal state writes its reason so a resume can never re-spend it.
fn native_exit(
    context: &AgentRunContext,
    state: &mut NativeAgentState,
    runtime: &AgentToolRuntime,
    queue: &[String],
    reason: &str,
    terminal_code: &'static str,
    bounded: bool,
) -> AgentTargetOutcome {
    native_sync(context, state, runtime, queue);
    let ledger = agent_ledger_completion(runtime, state, terminal_code);
    let summary = format!("{reason}；{}", ledger.summary);
    let ledger_closed = queue.is_empty() && ledger.uncovered_families.is_empty();
    let has_evidence = runtime.confirmed_findings > 0
        || !runtime.families.is_empty()
        || !runtime.requests.is_empty();
    let outcome = if ledger_closed {
        AgentTargetOutcome::Completed(AgentCompletion {
            summary: summary.clone(),
            ..ledger
        })
    } else if bounded && has_evidence {
        // §15: a plan boundary with usable evidence and named gaps is bounded
        // completion; anything else that still owes work stays resumable.
        AgentTargetOutcome::BoundedCompleted(AgentCompletion {
            summary: summary.clone(),
            ..ledger
        })
    } else {
        AgentTargetOutcome::incomplete(format!(
            "{summary}；剩余 {} 项未完成，可继续同一尝试",
            queue.len()
        ))
    };
    state.terminal_reason = if matches!(outcome, AgentTargetOutcome::Incomplete(_)) {
        String::new()
    } else {
        reason.to_string()
    };
    // §5.2: the terminal boundary itself must be durable. If it is not, the attempt
    // is reported as a persistence failure rather than as a finish it cannot prove.
    let boundary = state
        .persist(&context.db_path, &context.scan_id, &context.target_url)
        .and_then(|_| match &context.run {
            Some(run) => run.checkpoint(state),
            None => Ok(()),
        })
        .and_then(|_| persist_agent_coverage(context, runtime, state, &summary));
    if let Err(error) = boundary {
        return persistence_stop(context, error);
    }
    outcome
}

fn agent_soft_budget_exceeded(plan: &AgentExecutionPlan, state: &NativeAgentState) -> bool {
    plan.soft_uncached_tokens > 0
        && state.token_usage.uncached_input() + state.token_usage.output_tokens
            >= plan.soft_uncached_tokens
}

/// §6 rules 7 and 8: soft budgets only stop when progress has stalled; the hard
/// ceilings always stop. The reason plus its terminal code feed `native_exit`,
/// so the ledger stays the single source of the final state.
fn agent_budget_stop(
    context: &AgentRunContext,
    plan: &AgentExecutionPlan,
    state: &NativeAgentState,
    runtime: &AgentToolRuntime,
) -> Option<(&'static str, String)> {
    let hard_tokens = plan.hard_total_tokens;
    let uncached = state.token_usage.uncached_input() + state.token_usage.output_tokens;
    let stalled = state.no_progress_streak >= plan.no_progress_window.max(1);
    if hard_tokens > 0 && state.token_usage.total_tokens >= hard_tokens {
        return Some((
            AGENT_STOP_HARD_TOKENS,
            format!("累计 Token 达到 {hard_tokens} 硬上限"),
        ));
    }
    if plan.hard_model_requests > 0
        && state.token_usage.model_requests >= plan.hard_model_requests
    {
        return Some((
            AGENT_STOP_HARD_REQUESTS,
            format!("模型调用达到 {} 次硬上限", plan.hard_model_requests),
        ));
    }
    if runtime.target_requests >= agent_target_request_ceiling(context) {
        return Some((
            AGENT_STOP_HARD_REQUESTS,
            "目标请求数达到本轮硬上限".to_string(),
        ));
    }
    if stalled && plan.soft_uncached_tokens > 0 && uncached >= plan.soft_uncached_tokens {
        return Some((
            AGENT_STOP_SOFT_TOKENS,
            format!(
                "新增输入与输出 Token 达到 {} 软预算，且连续 {} 轮无进展",
                plan.soft_uncached_tokens, plan.no_progress_window
            ),
        ));
    }
    if stalled
        && plan.soft_model_requests > 0
        && state.token_usage.model_requests >= plan.soft_model_requests
    {
        return Some((
            AGENT_STOP_SOFT_REQUESTS,
            format!(
                "模型调用达到 {} 次软预算，且连续 {} 轮无进展",
                plan.soft_model_requests, plan.no_progress_window
            ),
        ));
    }
    None
}

fn agent_ledger_completion(
    runtime: &AgentToolRuntime,
    state: &NativeAgentState,
    terminal_code: &'static str,
) -> AgentCompletion {
    // §9.1: `covered` is the ledger's own conclusion, not the list of families
    // something happened to touch. Worked-but-unproven families stay in `uncovered`.
    let covered: Vec<String> = {
        let mut rows = state.covered_families.clone();
        rows.sort();
        rows.dedup();
        rows
    };
    let uncovered: Vec<String> = agent_required_families()
        .into_iter()
        .filter(|family| {
            !covered.iter().any(|value| value == family) && !runtime.not_applicable.contains(*family)
        })
        .map(str::to_string)
        .collect();
    AgentCompletion {
        summary: format!(
            "覆盖 {} 个族，未覆盖 {} 个族，确认问题 {}，模型请求 {} 次，Token {}",
            covered.len(),
            uncovered.len(),
            runtime.confirmed_findings,
            state.token_usage.model_requests,
            state.token_usage.total_tokens
        ),
        terminal_code,
        ledger_reported: true,
        model_requests: state.token_usage.model_requests,
        total_tokens: state.token_usage.total_tokens,
        verified_tool_results: runtime.target_requests as i64,
        covered_families: covered,
        uncovered_families: uncovered,
        confirmed_findings: runtime.confirmed_findings,
    }
}

fn agent_add_usage(current: &AgentTokenUsage, delta: &AgentTokenUsage) -> AgentTokenUsage {
    AgentTokenUsage {
        input_tokens: current.input_tokens + delta.input_tokens,
        cached_input_tokens: current.cached_input_tokens + delta.cached_input_tokens,
        output_tokens: current.output_tokens + delta.output_tokens,
        total_tokens: current.total_tokens + delta.total_tokens,
        model_requests: current.model_requests + 1,
    }
}

/// Native usage accumulates on the scan row; `sync_sentinel_attempt` derives the
/// per-attempt delta from the value captured at attempt start.
/// §5.2: usage is part of the commit boundary. If it cannot be written, the loop
/// must not start another model round it would be unable to bill or explain.
fn persist_agent_usage(db_path: &Path, scan_id: &str, usage: &AgentTokenUsage) -> Result<(), String> {
    let connection = db::open(db_path).map_err(|error| error.to_string())?;
    connection.execute(
        "UPDATE sentinel_scans SET llm_requests=llm_requests+?1,input_tokens=input_tokens+?2,output_tokens=output_tokens+?3,cached_tokens=cached_tokens+?4,total_tokens=total_tokens+?5,updated_at=datetime('now','localtime') WHERE id=?6",
        params![
            usage.model_requests,
            usage.input_tokens,
            usage.output_tokens,
            usage.cached_input_tokens,
            usage.total_tokens,
            scan_id
        ],
    )
    .map_err(|error| error.to_string())?;
    connection
        .execute(
            "UPDATE sentinel_scans SET llm_requests=MAX(0,llm_requests) WHERE id=?1",
            [scan_id],
        )
        .map_err(|error| error.to_string())?;
    sync_sentinel_attempt(&connection, scan_id);
    Ok(())
}

fn agent_record_key(prefix: &str, value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    format!("{prefix}:{}", digest.chars().take(24).collect::<String>())
}

/// Request/response evidence lands in the unified findings table with its own
/// stage; the raw bytes stay under the target directory.
fn persist_agent_evidence(
    context: &AgentRunContext,
    summary: &JsonValue,
    tool: &str,
) -> Result<(), String> {
    let connection = db::open(&context.db_path).map_err(|error| error.to_string())?;
    let endpoint = format!(
        "{} {}",
        value_first(summary, &["method"]).to_ascii_uppercase(),
        value_first(summary, &["url", "endpoint"])
    );
    let status = summary.get("status").and_then(JsonValue::as_i64).unwrap_or(0);
    let key = agent_record_key(tool, &format!("{endpoint}|{status}"));
    insert_finding(
        &connection,
        &context.scan_id,
        &context.target_url,
        AGENT_EVIDENCE_STAGE,
        "evidence",
        &key,
        &agent_text_truncated(&endpoint, 200),
        "info",
        &serde_json::json!({
            "source": "native-agent",
            "tool": tool,
            "identity": value_first(summary, &["identity"]),
            "endpoint": endpoint,
            "status": status,
            "observedAt": chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            "detail": summary,
        }),
    )
}

/// §8: only `confirmed` reaches the vulnerability contract, and it carries the
/// full evidence shape the UI and the appsec ledger already expect.
fn persist_agent_vulnerability(
    context: &AgentRunContext,
    arguments: &JsonValue,
    hypothesis_key: &str,
    bound: &ConfirmedPair,
) -> Result<(), String> {
    let connection = db::open(&context.db_path).map_err(|error| error.to_string())?;
    let severity = {
        let value = value_first(arguments, &["severity"]).to_ascii_lowercase();
        if value.is_empty() { "medium".to_string() } else { value }
    };
    let title = {
        let value = value_first(arguments, &["title"]);
        if value.is_empty() {
            format!("原生 Agent 确认：{hypothesis_key}")
        } else {
            value
        }
    };
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let record = serde_json::json!({
        "source": "native-agent",
        "title": title,
        "severity": severity,
        "cwe": value_first(arguments, &["cwe"]),
        "cvss": value_first(arguments, &["cvss"]),
        "confidence": arguments.get("confidence").and_then(JsonValue::as_f64).unwrap_or(0.9),
        "confidenceRationale": value_first(arguments, &["confidenceRationale"]),
        // §10.1: the finding is its two executed requests plus the difference
        // record between them, not a paragraph the model typed.
        "controlRequestId": bound.control_request_id,
        "testRequestId": bound.test_request_id,
        "responseDifferenceArtifactId": bound.difference_artifact_id,
        "controlRequest": bound.control_summary,
        "pocRequest": bound.test_summary,
        "testRequest": bound.test_summary,
        "responseDifference": format!("响应差异记录 {}", bound.difference_artifact_id),
        "impact": value_first(arguments, &["impact"]),
        "reproductionSteps": value_first(arguments, &["reproductionSteps"]),
        "counterEvidence": value_first(arguments, &["counterEvidenceCheck"]),
        "severityChangeConditions": value_first(arguments, &["severityChangeConditions"]),
        "recommendation": value_first(arguments, &["remediation"]),
        "fixVerification": value_first(arguments, &["fixVerification"]),
        "verdict": "confirmed",
        "url": context.target_url,
        "updatedAt": now,
        "updateHistory": [{
            "at": now,
            "status": "confirmed",
            "note": "原生 Agent 工具循环确认",
            "backend": "native",
        }],
    });
    insert_finding(
        &connection,
        &context.scan_id,
        &context.target_url,
        AGENT_VULNERABILITY_STAGE,
        "vulnerability",
        &agent_record_key("native", hypothesis_key),
        &agent_text_truncated(&title, 200),
        &severity,
        &record,
    )
}

/// The coverage ledger is a finding row so the existing evidence UI can render
/// it without a new table.
/// The coverage ledger write is part of the terminal boundary (§5.2).
fn persist_agent_coverage(
    context: &AgentRunContext,
    runtime: &AgentToolRuntime,
    state: &NativeAgentState,
    summary: &str,
) -> Result<(), String> {
    let connection = db::open(&context.db_path).map_err(|error| error.to_string())?;
    let covered: Vec<&str> = state.covered_families.iter().map(String::as_str).collect();
    let uncovered = agent_required_families()
        .into_iter()
        .filter(|family| !covered.contains(family))
        .map(|family| {
            runtime
                .uncovered_families
                .iter()
                .find(|row| value_first(row, &["family"]) == family)
                .cloned()
                .unwrap_or_else(|| serde_json::json!({
                    "family": family,
                    "label": agent_coverage_family_label(family),
                    "status": "not_covered",
                    "reason": summary,
                }))
        })
        .collect::<Vec<_>>();
    insert_finding(
        &connection,
        &context.scan_id,
        &context.target_url,
        AGENT_COVERAGE_STAGE,
        "coverage",
        "coverage",
        &format!("原生 Agent 覆盖账本（{} 个族已覆盖）", covered.len()),
        "info",
        &serde_json::json!({
            "source": "native-agent",
            "coveredFamilies": covered,
            "uncoveredFamilies": uncovered,
            "coverageEvidence": runtime.coverage,
            "confirmedFindings": runtime.confirmed_findings,
            "exclusions": runtime.exclusions,
            "manualDeepDiveSuggestions": runtime.manual_suggestions,
            "modelRequests": state.token_usage.model_requests,
            "totalTokens": state.token_usage.total_tokens,
            "targetRequests": runtime.target_requests,
            "discoveryRounds": runtime.discovery_rounds,
            "turns": state.turns,
            "summary": summary,
        }),
    )
}
