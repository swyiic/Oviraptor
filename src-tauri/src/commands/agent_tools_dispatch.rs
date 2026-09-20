/// What one tool execution produced (§6.1). `model_view` is the only part that
/// may enter the conversation; `artifact_id` points at the raw bytes, and
/// `progress` carries the facts the ledger counts without any payload at all.
struct ToolExecutionResult {
    model_view: JsonValue,
    artifact_id: String,
    invocation_id: String,
}

fn agent_execute_tool_at(
    context: &AgentRunContext,
    runtime: &mut AgentToolRuntime,
    name: &str,
    arguments: &JsonValue,
    invocation_id: &str,
) -> ToolExecutionResult {
    runtime.set_invocation(invocation_id);
    agent_execute_tool(context, runtime, name, arguments)
}

/// A call that opens its own invocation, for direct (single-tool) drivers.
fn agent_execute_tool(
    context: &AgentRunContext,
    runtime: &mut AgentToolRuntime,
    name: &str,
    arguments: &JsonValue,
) -> ToolExecutionResult {
    let invocation_id = runtime.begin_invocation(context.attempt_number);
    runtime.set_invocation(&invocation_id);
    let executed = agent_execute_tool_inner(context, runtime, name, arguments);
    ToolExecutionResult {
        // The HTTP path already projected its payload because the audit record
        // embeds the same view; re-projecting is a no-op on already-marked text.
        model_view: agent_model_view(executed, runtime),
        artifact_id: runtime.last_artifact_id.clone(),
        invocation_id: runtime.current_invocation.clone(),
    }
}

fn agent_execute_tool_inner(
    context: &AgentRunContext,
    runtime: &mut AgentToolRuntime,
    name: &str,
    arguments: &JsonValue,
) -> JsonValue {
    runtime.last_artifact_id.clear();
    if let Err(rejection) = agent_validate_arguments(name, arguments) {
        runtime.note_argument_error(&rejection.fingerprint);
        return serde_json::json!({
            "error": rejection.message,
            "code": "invalid_arguments",
            "schemaErrors": rejection.errors,
            "errorFingerprint": rejection.fingerprint,
        });
    }
    runtime.clear_argument_error();
    if runtime.cancelled() {
        return agent_tool_error("任务已暂停或取消，不再向目标发请求", "cancelled");
    }
    if let Some(stop) = &runtime.protected_stop {
        // Once the target is protected, further requests are not authorized.
        return serde_json::json!({"code": "protected", "stop": stop});
    }
    // Anything the model asked for that is not one of our tools is refused before
    // a payload could be echoed back.

    if runtime.target_requests >= agent_target_request_ceiling(context) {
        return agent_tool_error(
            "目标请求总数已达到本轮硬上限，请调用 finish_target 收口",
            "request_budget_exhausted",
        );
    }
    match name {
        "inspect_evidence" => agent_tool_inspect_evidence(context, arguments),
        "replay_http" => agent_tool_replay_http(context, runtime, arguments),
        "compare_identities" => agent_tool_compare_identities(context, runtime, arguments),
        "targeted_discovery" => agent_tool_targeted_discovery(context, runtime, arguments),
        "browser_action" => agent_tool_browser_action(context, runtime, arguments),
        "record_hypothesis_result" => {
            agent_tool_record_hypothesis_result(context, runtime, arguments)
        }
        "finish_target" => agent_tool_finish_target(context, runtime, arguments),
        other => agent_tool_error(&format!("未知工具 {other}"), "unknown_tool"),
    }
}

/// Requests against the target are bounded by the plan, not by model turns.
fn agent_target_request_ceiling(context: &AgentRunContext) -> usize {
    let plan = &context.execution_plan;
    (plan.hard_model_requests.max(1) * 4).min(400) as usize
}
