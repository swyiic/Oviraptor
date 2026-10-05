// Preserve the original gateway error classifications for unsent or legacy
// calls. Accounted unknown transports stop through the cost-specific outcome.
fn native_model_error(
    context: &AgentRunContext,
    state: &mut NativeAgentState,
    runtime: &AgentToolRuntime,
    queue: &[String],
    client: &AgentModelClient,
    error: AgentModelError,
) -> AgentTargetOutcome {
    let detail = error.detail();
    native_sync(context, state, runtime, queue);
    if matches!(
        error,
        AgentModelError::Network(_) | AgentModelError::Server { .. }
    ) {
        // §6 rule 9: a transport stall keeps the evidence and stays
        // resumable, so it must not write a terminal reason.
        if let Err(error) = state.persist(&context.db_path, &context.scan_id, &context.target_url) {
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
        note_transport_cancel(context, client);
    }
    if let Err(error) = state.persist(&context.db_path, &context.scan_id, &context.target_url) {
        return persistence_stop(context, error);
    }
    match error {
        AgentModelError::Cancelled => AgentTargetOutcome::Cancelled,
        AgentModelError::UnsupportedCapability(_) => {
            AgentTargetOutcome::failed(format!("{detail}；模型服务不支持工具调用"))
        }
        AgentModelError::Authentication(_) => AgentTargetOutcome::failed(detail),
        AgentModelError::Protocol(_) => AgentTargetOutcome::failed(detail),
        AgentModelError::ContextOverflow(_)
        | AgentModelError::RateLimited(_)
        | AgentModelError::Timeout(_) => {
            if let Err(error) = persist_agent_coverage(context, runtime, state, &detail) {
                return persistence_stop(context, error);
            }
            AgentTargetOutcome::limited(detail)
        }
        AgentModelError::Network(_) | AgentModelError::Server { .. } => {
            AgentTargetOutcome::incomplete(detail)
        }
    }
}
