// Publication follows the durable provider receipt. A failed publication stops
// the loop; the receipt is a cost fact and never a grant to rerun the provider.
fn native_publish_model_round(
    context: &AgentRunContext,
    client: &AgentModelClient,
    turn: &crate::agent_runtime::model::gateway::ModelResponse,
    state: &mut NativeAgentState,
    directives: &HumanDirectiveContext,
) -> Result<Vec<crate::agent_runtime::model::gateway::ToolCall>, String> {
    client.record_usage_line(&context.target_dir, turn);
    state.token_usage = agent_add_usage(&state.token_usage, &turn.usage);
    persist_agent_usage(&context.db_path, &context.scan_id, &turn.usage)?;
    state.turns += 1;
    let calls = turn.deduplicated_tool_calls();
    if let Some(run) = &context.run {
        run.model_round_with_directives(
            state.turns,
            &state.token_usage,
            &calls
                .iter()
                .map(|call| call.name.clone())
                .collect::<Vec<_>>(),
            directives,
        )?;
    }
    Ok(calls)
}
