// Keep the local hook alive for the whole executor. Dropping its guard closes
// upstream sockets after cancellation or a persistence failure.
fn native_model_setup(
    context: &AgentRunContext,
    specs: &[AgentToolSpec],
) -> Result<(Option<crate::llm_hook::LlmHookHandle>, AgentModelClient), String> {
    let hook = if context.environment.deployment == "local" {
        let policy = local_model_runtime_policy(&context.environment);
        crate::llm_hook::start(
            &context.environment.api_base,
            &context.environment.api_key,
            &context.target_dir,
            &context.environment.prompt_audit_mode,
            context.proxy.as_deref(),
            policy.max_output_tokens,
            policy.max_context_tokens,
            policy.max_concurrent_requests,
        )?
    } else {
        None
    };
    let mut environment = context.environment.clone();
    if let Some(handle) = &hook {
        environment.api_base = handle.base_url().to_string();
    }
    let profile = agent_model_profile(&environment, context.proxy.as_deref())?;
    Ok((hook, AgentModelClient::new(profile, specs)))
}
