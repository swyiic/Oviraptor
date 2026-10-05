fn run_native_agent(context: &AgentRunContext) -> AgentTargetOutcome {
    if !native_web_attempt_current(&context.db_path, &context.scan_id, context.attempt_number) {
        return AgentTargetOutcome::Cancelled;
    }
    let _lease_heartbeat = match ExecutorLeaseHeartbeat::start(context) {
        Ok(heartbeat) => heartbeat,
        Err(error)
            if matches!(
                error.as_str(),
                "budget_indeterminate_requires_reconciliation"
                    | "budget_history_requires_reconciliation"
            ) =>
        {
            return agent_tool_terminal_outcome(&agent_http_claim_error(error.clone()))
                .unwrap_or_else(|| persistence_stop(context, error));
        }
        Err(error) => return persistence_stop(context, error),
    };
    let specs = agent_tool_specs_for(context);
    // §7: parameter schemas are compiled once up front, never lazily in the middle
    // of a model round.
    let _ = agent_schema_registry();
    let (_model_proxy, client) = match native_model_setup(context, &specs) {
        Ok(setup) => setup,
        Err(error) => return AgentTargetOutcome::failed(error),
    };
    let plan = &context.execution_plan;
    let evidence_hash = agent_stable_hash(&context.evidence);
    let plan_hash = plan.hash();
    let full_queue = agent_initial_queue(context);

    // §3.6: an unsafe continuation stops before the first model or target
    // request, so resuming never burns Token on a state the runtime cannot trust.
    if let Some(rejection) = &context.plan_rejection {
        return rejection_outcome(context, rejection);
    }
    // First Single control contract precedes every native state/tool write.
    // Existing Native history without original proof is never backfilled.
    if let Err(reason) = native_prepare_single_budget(context) {
        return agent_tool_terminal_outcome(&agent_http_claim_error(reason.clone()))
            .unwrap_or_else(|| persistence_stop(context, reason));
    }
    if let Err(reason) = native_coordinator_budget_before_executor(context) {
        append_runner_log(&context.log_path, &format!("原预算分配监督停止：{reason}"));
        return if reason == "root_budget_allocation_deferred" {
            AgentTargetOutcome::incomplete(reason)
        } else {
            multi_agent_bootstrap_outcome(&reason)
        };
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
    if context.run.is_some() {
        // Do not spend another model round just to rediscover a known unknown
        // request at send time. A header receipt alone also cannot rehydrate a
        // stale tool/checkpoint result. Fresh attempts use a distinct lineage.
        let usage = db::open(&context.db_path).and_then(|connection| {
            let tx = connection
                .unchecked_transaction()
                .map_err(|error| error.to_string())?;
            agent_request_accounting(
                &tx,
                &context.scan_id,
                context.attempt_number,
                &context.target_url,
            )
        });
        let stop = match usage {
            Ok(usage)
                if usage.executor_unresolved > 0
                    || usage.executor_recorded != state.target_requests =>
            {
                Some(serde_json::json!({"code":terminal_code::REQUEST_RECONCILIATION_REQUIRED}))
            }
            Ok(_) => None,
            Err(reason) => Some(agent_http_claim_error(reason)),
        };
        if let Some(outcome) = stop.as_ref().and_then(agent_tool_terminal_outcome) {
            append_runner_log(&context.log_path, &outcome.detail());
            return outcome;
        }
    }
    // Tools share the loop's cancel token so an in-flight request stops touching
    // the target as soon as the user pauses or cancels.
    runtime.cancel = Some(agent_scan_cancel_token(
        &context.db_path,
        &context.scan_id,
        context.attempt_number,
    ));
    let mut queue: Vec<String> = if context.resume && !state.pending_queue.is_empty() {
        state.pending_queue.clone()
    } else {
        full_queue.clone()
    };
    queue.retain(|key| !runtime.queue_key_satisfied(context, key));
    // §4.2: recover the ledger before any model or target work. An unfinished
    // target call has an unknown outcome and stops same-attempt replay; only
    // local read-only work may be requeued. Spent budget remains spent.
    if context.resume {
        if let Some(run) = &context.run {
            let recovered = match run.recover() {
                Ok(recovered) => recovered,
                Err(error)
                    if error
                        == "interrupted_tool_result_unknown_manual_reconciliation_required" =>
                {
                    append_runner_log(&context.log_path, "中断的工具调用可能已触达目标，拒绝同一 attempt 自动重放；须人工核对后新建任务");
                    return AgentTargetOutcome::Incomplete(AgentStop::new(
                        terminal_code::REQUEST_RECONCILIATION_REQUIRED,
                        "中断的工具调用结果不明，已停止自动恢复；预算保留，不自动重放。须先核对目标日志与本地证据，再决定后续任务，不能直接重跑来确认结果",
                    ));
                }
                Err(error) => return persistence_stop(context, error),
            };
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
                        "run {} 恢复：重放 {} 条事件，{} 条只读调用标记为 interrupted，其契约重新排队",
                        run.run_id, recovered.events_replayed, recovered.interrupted_invocations
                    ),
                );
            }
        }
    }

    let mut history = vec![
        serde_json::json!({"role": "system", "content": AGENT_SYSTEM_RULES, "oviraptorKeep": true}),
        agent_state_block(context, &runtime, &state, &queue),
    ];

    loop {
        // An old worker may not checkpoint over the new attempt's shared row.
        if !native_web_attempt_current(&context.db_path, &context.scan_id, context.attempt_number) {
            return AgentTargetOutcome::Cancelled;
        }
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
        if let Err(error) = native_web_require_determinate(context) {
            return agent_tool_terminal_outcome(&agent_http_claim_error(error.clone()))
                .unwrap_or_else(|| persistence_stop(context, error));
        }
        if plan.hard_model_requests > 0
            && agent_run_requests(context, &state) >= plan.hard_model_requests
            && state.budget_extensions == 0
            && !queue.is_empty()
        {
            state.budget_extensions = 1;
            state.last_expansion_reason = "队列未完成，延长一次同等模型预算".to_string();
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
        let mut directives = match take_human_directives(context) {
            Ok(directives) => directives,
            Err(error) => return persistence_stop(context, error),
        };
        if let Err(error) = apply_assessed_human_queue_actions(context, &mut directives, &mut queue) {
            append_runner_log(&context.log_path, &format!("原人工确认监督停止：{error}"));
            if matches!(error.as_str(), "child_budget_reservation_exceeded_or_stale" | "budget_model_requests_exhausted" | "budget_hard_limit_exceeded") {
                // No human action is authorized without a paid assessment.
                // Defer it under the original parent, then use the existing Web grant.
                if let Err(reason) = defer_human_directives_for_capacity(context, &mut directives) {
                    return persistence_stop(context, reason);
                }
            } else {
                return if error == "root_human_directive_deferred" {
                    AgentTargetOutcome::incomplete(error)
                } else {
                    multi_agent_bootstrap_outcome(&error)
                };
            }
        };
        let proposal_messages = match apply_human_proposal_actions(context, &mut directives) {
            Ok(messages) => messages,
            Err(error) if matches!(error.as_str(),"budget_indeterminate_requires_reconciliation" | "budget_history_requires_reconciliation") => {
                // Already published original invoices remain intact. Unknown
                // Root costs prohibit proposal work and the next model round.
                return agent_tool_terminal_outcome(&agent_http_claim_error(error.clone()))
                    .unwrap_or_else(|| persistence_stop(context,error));
            }
            Err(error) => return persistence_stop(context, error),
        };
        history[1] = agent_state_block(context, &runtime, &state, &queue);
        // Rehydrate accepted instructions, including on resume; never append
        // duplicate copies to history or let compaction rewrite their meaning.
        let mut round_history = history.clone();
        round_history.extend(directives.messages());
        round_history.extend(proposal_messages);
        let (messages, _compacted) =
            agent_compact_messages(&round_history, client.profile().request_budget_bytes());
        let turn = match native_model_transport(context, &client, messages, &specs, state.turns + 1)
        {
            Ok(turn) => turn,
            Err(NativeModelRoundFailure::Outcome(outcome)) => return *outcome,
            Err(NativeModelRoundFailure::Model(error)) => {
                return native_model_error(context, &mut state, &runtime, &queue, &client, error)
            }
        };
        let calls =
            match native_publish_model_round(context, &client, &turn, &mut state, &directives) {
                Ok(calls) => calls,
                Err(error) => return persistence_stop(context, error),
            };
        if let Err(error) = native_web_require_determinate(context) {
            return agent_tool_terminal_outcome(&agent_http_claim_error(error.clone()))
                .unwrap_or_else(|| persistence_stop(context, error));
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
            // An empty or text-only reply is not the end of the attempt while
            // budget remains. Count it as no progress and keep going; the
            // no-progress window or a hard ceiling is what actually stops.
            history.push(serde_json::json!({
                "role": "user",
                "content": "不要只用文字结束。请调用工具继续处理 pendingQueue 和未覆盖族；预算未耗尽时不要结束。",
            }));
            state.no_progress_streak += 1;
            if let Some((code, reason)) = agent_text_only_stop(context, plan, &mut state, &runtime)
            {
                return native_exit(context, &mut state, &runtime, &queue, &reason, code, true);
            }
            if state.last_expansion_reason == "连续无进展，先检查队列再继续"
                && state.no_progress_streak == 0
            {
                history.push(serde_json::json!({
                    "role": "user",
                    "content": "检查：上一轮没有新证据。请从 pendingQueue 和未覆盖族里继续下一项，不要重复已完成的请求。",
                }));
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
            let target_requests_before = runtime.target_requests;
            let mut result = agent_execute_tool_at(
                context,
                &mut runtime,
                &call.name,
                &call.arguments,
                &invocation_id,
            );
            if call.name == "finish_target" {
                if let Some(rejected) = agent_reject_premature_finish(
                    context,
                    plan,
                    &state,
                    &mut runtime,
                    &queue,
                    &result.model_view,
                ) {
                    result.model_view = rejected;
                }
            }
            if let Some(run) = &context.run {
                if let Err(error) = run.finish_tool(
                    row,
                    &call.name,
                    &call.arguments,
                    &result.model_view,
                    &result.invocation_id,
                    runtime
                        .target_requests
                        .saturating_sub(target_requests_before),
                ) {
                    return persistence_stop(context, error);
                }
            }
            // Fan-out tools may wrap or aggregate an exchange failure. Consult
            // the sticky send-boundary stop before looking at the outer result.
            if let Some(outcome) = runtime
                .target_request_stop
                .as_ref()
                .and_then(agent_tool_terminal_outcome)
                .or_else(|| agent_tool_terminal_outcome(&result.model_view))
            {
                append_runner_log(&context.log_path, &outcome.detail());
                return outcome;
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
        // Public-surface paths harvested from HTML/JS join the queue as api: entries
        // so classic onclick/ajax sites keep expanding beyond browser-observed XHR.
        for endpoint in runtime.drain_pending_apis() {
            let key = format!("api:{endpoint}");
            if !queue.contains(&key) {
                queue.push(key);
            }
        }
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
            let gaps_remain = !queue.is_empty() || !runtime.uncovered_families.is_empty();
            return native_exit(
                context,
                &mut state,
                &runtime,
                &queue,
                "模型已调用 finish_target 输出覆盖账本",
                AGENT_STOP_FINISH,
                gaps_remain,
            );
        }
        match agent_stall_action(&mut state, plan) {
            StallAction::Stop => {
                return native_exit(
                    context,
                    &mut state,
                    &runtime,
                    &queue,
                    "连续两次检查都没有新增端点、响应差异或结论",
                    AGENT_STOP_NO_PROGRESS,
                    true,
                );
            }
            StallAction::Check => {
                let next = queue
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "没有剩余队列".to_string());
                history.push(serde_json::json!({
                    "role": "user",
                    "content": format!("检查：上一轮没有新证据。下一项是 {next}。请继续这一项，不要重复已完成的请求，也不要因为一条 404 结束目标。"),
                }));
            }
            StallAction::Continue => {}
        }
        if let Err(error) = native_commit(context, &mut state, &runtime, &queue) {
            return persistence_stop(context, error);
        }
        // The state block is replaced in place: appending one per round would
        // re-feed the model a growing, already-known snapshot.
        history[1] = agent_state_block(context, &runtime, &state, &queue);
    }
}
