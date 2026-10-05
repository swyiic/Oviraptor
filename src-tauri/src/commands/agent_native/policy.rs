fn agent_run_tokens(context: &AgentRunContext, state: &NativeAgentState) -> i64 {
    context
        .run_budget
        .as_ref()
        .map(|budget| {
            state
                .token_usage
                .total_tokens
                .saturating_sub(budget.starting_tokens)
        })
        .unwrap_or(state.token_usage.total_tokens)
        .max(0)
}

fn agent_run_requests(context: &AgentRunContext, state: &NativeAgentState) -> i64 {
    context
        .run_budget
        .as_ref()
        .map(|budget| {
            state
                .token_usage
                .model_requests
                .saturating_sub(budget.starting_requests)
        })
        .unwrap_or(state.token_usage.model_requests)
        .max(0)
}

fn agent_effective_hard_tokens(context: &AgentRunContext, plan: &AgentExecutionPlan) -> i64 {
    match context.run_budget.as_ref().map(|budget| budget.hard_tokens) {
        Some(window) if window > 0 && plan.hard_total_tokens > 0 => {
            window.min(plan.hard_total_tokens)
        }
        Some(window) if window > 0 => window,
        _ => plan.hard_total_tokens,
    }
}

fn agent_effective_hard_requests(
    context: &AgentRunContext,
    plan: &AgentExecutionPlan,
    state: &NativeAgentState,
) -> i64 {
    let base = plan.hard_model_requests.max(0);
    let plan_limit = if state.budget_extensions > 0 {
        base.saturating_mul(2)
    } else {
        base
    };
    match context.run_budget.as_ref().map(|budget| budget.hard_requests) {
        Some(window) if window > 0 && plan_limit > 0 => window.min(plan_limit),
        Some(window) if window > 0 => window,
        _ => plan_limit,
    }
}

enum StallAction {
    Continue,
    Check,
    Stop,
}

/// The first no-progress window is a check, not a stop. The second one ends the target.
fn agent_stall_action(state: &mut NativeAgentState, plan: &AgentExecutionPlan) -> StallAction {
    if state.no_progress_streak < plan.no_progress_window.max(1) {
        return StallAction::Continue;
    }
    if state.stall_checks < 1 {
        state.stall_checks += 1;
        state.no_progress_streak = 0;
        state.last_expansion_reason = "连续无进展，先检查队列再继续".to_string();
        return StallAction::Check;
    }
    StallAction::Stop
}

/// A text-only model round has already consumed a turn and its usage. Classify
/// that cost before the no-progress check changes stall state; otherwise a hard
/// ceiling is incorrectly persisted as a no-progress stop.
fn agent_text_only_stop(
    context: &AgentRunContext,
    plan: &AgentExecutionPlan,
    state: &mut NativeAgentState,
    runtime: &AgentToolRuntime,
) -> Option<(&'static str, String)> {
    if state.turns >= plan.max_turns.max(1) {
        return Some((AGENT_STOP_DERIVED, "达到计划内最大轮数".to_string()));
    }
    if let Some(stop) = agent_budget_stop(context, plan, state, runtime) {
        return Some(stop);
    }
    if matches!(agent_stall_action(state, plan), StallAction::Stop) {
        return Some((
            AGENT_STOP_NO_PROGRESS,
            "连续两次检查都没有新增证据".to_string(),
        ));
    }
    None
}

fn agent_reject_premature_finish(
    context: &AgentRunContext,
    plan: &AgentExecutionPlan,
    state: &NativeAgentState,
    runtime: &mut AgentToolRuntime,
    queue: &[String],
    model_view: &JsonValue,
) -> Option<JsonValue> {
    let accepted = model_view
        .get("accepted")
        .and_then(JsonValue::as_bool)
        == Some(true);
    if !accepted {
        return None;
    }
    let never_tried = model_view
        .get("notCoveredFamilies")
        .and_then(JsonValue::as_array)
        .map(|rows| rows.len())
        .unwrap_or(0);
    let open_observations = runtime.observation_finding_keys.is_empty()
        && runtime.confirmed_findings == 0
        && model_view
            .get("coverage")
            .and_then(JsonValue::as_array)
            .map(|rows| {
                rows.iter().any(|row| {
                    value_first(row, &["family"]) == "information_disclosure"
                        && matches!(
                            value_first(row, &["status"]).as_str(),
                            "partial" | "not_covered" | "insufficient_evidence"
                        )
                })
            })
            .unwrap_or(false);
    let open_surface = runtime.discovered_api_keys.iter().any(|key| {
        let (method, path) = key.split_once('|').unwrap_or(("GET", key.as_str()));
        !runtime.touched(method, path)
    }) || !runtime.pending_api_queue.is_empty();
    // observation_finding_keys non-empty means we already persisted passive findings;
    // reject only when headers were seen this run but nothing was confirmed yet is
    // handled inside HTTP emission. Here: never-tried families OR open disclosure gaps
    // OR harvested public paths still unvisited.
    let open_queue = !queue.is_empty();
    let unfinished_families = agent_required_families()
        .into_iter()
        .filter(|family| {
            !runtime.families.contains(*family) && !runtime.not_applicable.contains(*family)
        })
        .count();
    if never_tried == 0 && unfinished_families == 0 && !open_observations && !open_surface && !open_queue {
        return None;
    }
    let never_tried = never_tried
        .max(usize::from(open_observations))
        .max(usize::from(open_surface))
        .max(usize::from(open_queue));
    // Soft budget is not an excuse to stop. Only a hard ceiling ends the attempt,
    // and repeated finish_target calls do not create one.
    let spent_the_extension = state.budget_extensions > 0
        && agent_run_requests(context, state)
            >= agent_effective_hard_requests(context, plan, state);
    let hard_tokens = agent_effective_hard_tokens(context, plan);
    let token_or_turn_cap = (hard_tokens > 0
        && agent_run_tokens(context, state) >= hard_tokens)
        || state.turns + 1 >= plan.max_turns.max(1)
        || runtime.target_requests >= agent_target_request_ceiling(context);
    // Reaching the first model-call ceiling is not the end while work remains.
    // The next round records the one allowed extension instead of accepting finish.
    if spent_the_extension || token_or_turn_cap {
        return None;
    }
    runtime.finished = None;
    let families = model_view
        .get("notCoveredFamilies")
        .cloned()
        .unwrap_or_else(|| serde_json::json!([]));
    Some(serde_json::json!({
        "accepted": false,
        "code": "premature_finish_target",
        "error": format!(
            "仍有 {never_tried} 项覆盖族或公开面路径未执行请求，预算与轮次未耗尽；请先处理 pendingQueue/notCoveredFamilies，再 finish_target"
        ),
        "notCoveredFamilies": families,
        "rejectCount": state.turns,
        "hint": "优先对 notCoveredFamilies 中的族执行请求或 discovery，不要立刻再次 finish_target",
    }))
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
    let hard_tokens = agent_effective_hard_tokens(context, plan);
    let uncached = state.token_usage.uncached_input() + state.token_usage.output_tokens;
    let stalled = state.no_progress_streak >= plan.no_progress_window.max(1);
    if hard_tokens > 0 && agent_run_tokens(context, state) >= hard_tokens {
        return Some((
            AGENT_STOP_HARD_TOKENS,
            format!("累计 Token 达到 {hard_tokens} 硬上限"),
        ));
    }
    let hard_requests = agent_effective_hard_requests(context, plan, state);
    if hard_requests > 0 && agent_run_requests(context, state) >= hard_requests {
        return Some((
            AGENT_STOP_HARD_REQUESTS,
            format!("模型调用达到 {hard_requests} 次硬上限"),
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
        // `ledger_reported` means the close-out itself exists.  Coverage gaps are
        // carried separately in `uncovered_families`; equating the two made a
        // formally closed ledger with named gaps look like an open/resumable run.
        ledger_reported: runtime.finished.is_some(),
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
