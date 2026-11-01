// B3.3c (design Option a) opt-in: thread the single controller's planning
// reasoning into the execution run's conversation as continuity ("one brain"),
// instead of the execution run opening a fresh conversation seeded only by state.
// Gated by a frozen-plan rule, so EVERY current plan is unaffected: when the rule
// is absent the opening system turn is byte-identical to before. The execution
// run keeps its own identity/budget/settlement (the `used_*=0` root invariant
// forbids moving execution onto the coordinator run), so only the reasoning
// conversation unifies. A target page can drive the agent, so this is a fixed
// frozen-plan rule, never a runtime/model choice.
const CONTROLLER_EXECUTION_CONVERSATION_RULE: &str = "unified_controller_conversation";

fn controller_execution_conversation_enabled(bootstrap: &JsonValue) -> bool {
    bootstrap["controllerExecutionRule"] == CONTROLLER_EXECUTION_CONVERSATION_RULE
}

// The execution run's opening system turn. Without controller planning this is
// EXACTLY the former seed (`{role:system, content:AGENT_SYSTEM_RULES}`); with it,
// the planning continuity is appended to the SAME system turn. Keeping it one
// turn preserves the `[system, state]` history shape and the adapter's
// `history[1] = agent_state_block(..)` per-round refresh — no extra turn, no
// index shift.
fn native_executor_system_turn(system_rules: &str, controller_planning: Option<&str>) -> JsonValue {
    let content = match controller_planning {
        Some(planning) if !planning.is_empty() => format!("{system_rules}\n\n{planning}"),
        _ => system_rules.to_string(),
    };
    serde_json::json!({"role": "system", "content": content, "oviraptorKeep": true})
}

// Fail-safe: any lookup error, the flag absent, or no published handoff keeps
// today's seed (None). The continuity text is projected from the coordinator's
// already-published handoff decision summary (the controller's own reasoning
// output) — never new evidence and never a new grant.
fn native_controller_execution_planning(context: &AgentRunContext) -> Option<String> {
    let enabled = (|| -> Result<bool, String> {
        let connection = db::open(&context.db_path)?;
        let run = context.run.as_ref().ok_or("executor_run_missing")?;
        let root: String = connection
            .query_row(
                "SELECT root_run_id FROM agent_runs WHERE id=?1",
                [&run.run_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let bootstrap = crate::agent_runtime::web_mode::root::read(&connection, &root)?
            .and_then(|mode| mode.bootstrap_dispatch())
            .unwrap_or(JsonValue::Null);
        Ok(controller_execution_conversation_enabled(&bootstrap))
    })()
    .unwrap_or(false);
    if !enabled {
        return None;
    }
    let decision = context
        .evidence
        .get("multiAgentCoordinatorHandoff")
        .and_then(|handoff| handoff.get("decisionSummary"))?;
    Some(format!(
        "[控制器规划连续性] 你与本目标的规划阶段是同一个控制器。以下是你在规划阶段得出的交接决策，据此直接执行验证合同，不要重新发现已知结论：\n{decision}"
    ))
}

#[cfg(test)]
mod controller_execution_conversation_tests {
    use super::{
        controller_execution_conversation_enabled, native_executor_system_turn,
        CONTROLLER_EXECUTION_CONVERSATION_RULE,
    };
    use serde_json::json;

    #[test]
    fn flag_is_off_unless_the_exact_rule_is_present() {
        assert!(!controller_execution_conversation_enabled(&json!({})));
        assert!(!controller_execution_conversation_enabled(&serde_json::Value::Null));
        assert!(!controller_execution_conversation_enabled(
            &json!({"controllerExecutionRule": "other"})
        ));
        assert!(!controller_execution_conversation_enabled(
            &json!({"controllerExecutionRule": true})
        ));
        assert!(controller_execution_conversation_enabled(
            &json!({"controllerExecutionRule": CONTROLLER_EXECUTION_CONVERSATION_RULE})
        ));
    }

    #[test]
    fn system_turn_is_byte_identical_to_the_former_seed_when_off() {
        let rules = "RULES-XYZ";
        let former = json!({"role": "system", "content": rules, "oviraptorKeep": true});
        assert_eq!(native_executor_system_turn(rules, None), former);
        // An empty planning string is treated as off (no continuity appended).
        assert_eq!(native_executor_system_turn(rules, Some("")), former);
    }

    #[test]
    fn system_turn_appends_planning_when_on() {
        let turn = native_executor_system_turn("RULES", Some("PLAN-CONTINUITY"));
        assert_eq!(turn["role"], "system");
        assert_eq!(turn["oviraptorKeep"], true);
        let content = turn["content"].as_str().unwrap();
        assert!(content.starts_with("RULES"));
        assert!(content.contains("PLAN-CONTINUITY"));
    }
}

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
    let _original_execution = match native_web_original_execution_owner(context) {
        Ok(owner) => owner,
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
    // Model rounds are numbered per original Root (journal/lifetime rounds are
    // 1..N for that Root). A resumed attempt inherits the parent's turn count for
    // its turn budget, but owns a fresh Root whose first provider call is round 1.
    let attempt_round_base = state.turns;
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

    // Opt-in (B3.3c Option a): when the frozen plan enables it, the controller's
    // planning continuity rides inside the system turn; otherwise this is the
    // former `[system, state]` seed unchanged. `history[1]` stays the state block
    // the adapter refreshes each round.
    let controller_planning = native_controller_execution_planning(context);
    let history = vec![
        native_executor_system_turn(AGENT_SYSTEM_RULES, controller_planning.as_deref()),
        agent_state_block(context, &runtime, &state, &queue),
    ];

    let registered=specs.iter().map(|spec|spec.name.to_string()).collect();
    native_run_web_controlled_adapter(NativeWebExecutorAdapter {
        context,client,specs,registered,state,runtime,queue,history,
        attempt_round_base,finished:false,stop_outcome:None,
    })
}
