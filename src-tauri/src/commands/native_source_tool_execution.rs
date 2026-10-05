fn execute_source_tool_assignment(
    connection: &rusqlite::Connection,
    context: &SpecialistTransportContext<'_>,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    slice: &JsonValue,
) -> Result<JsonValue, String> {
    use crate::agent_runtime::{
        multi_agent::{source, source_rounds},
        secrets::redact_json,
        store::stable_hash,
    };
    context.require_supervision(lease)?;
    let mut profile = agent_model_profile(context.environment, context.proxy)?;
    let capabilities = source::tool_capabilities(child.role)?;
    let tools = agent_tool_specs()
        .into_iter()
        .filter(|s| capabilities.iter().any(|n| n == s.name))
        .collect::<Vec<_>>();
    let input = crate::agent_runtime::multi_agent::directive::source_guidance::attach(
        connection,
        lease,
        child.role,
        true,
        json!({"sourceTask":slice,"maxModelRounds":3}),
    )?;
    let messages = source_assessment_messages(SOURCE_TOOLS_SYSTEM, &input);
    let (_, output) = source_assessment_budget(&messages, &profile)?;
    profile.max_output_tokens = Some(output);
    let mut frozen = redact_json(
        &json!({"schemaVersion":1,"messages":messages,"tools":tools.iter().map(|s|s.as_function_spec()).collect::<Vec<_>>(),
        "temperature":0.0,"model":profile.model,"endpointHash":stable_hash(&profile.endpoint),"proxyHash":stable_hash(profile.proxy.as_deref().unwrap_or_default()),
        "local":profile.local,"maxOutputTokens":profile.max_output_tokens,"maxContextTokens":profile.max_context_tokens}),
    );
    let client = AgentModelClient::new(profile.clone(), &[]);
    for number in 1..=3 {
        let check =
            |tx: &rusqlite::Connection| authorize_source_tool_phase(tx, context, lease, child);
        if number > 1 {
            frozen = source_rounds::prepare_next_authorized(
                connection, lease, child, number, &frozen, check,
            )?;
        }
        // Includes tool schemas and all accumulated tool results in admission.
        let (required, _) = source_assessment_budget(
            &[json!({"messages":frozen["messages"],"tools":frozen["tools"]})],
            &profile,
        )?;
        let (start, _model_lifetime) = source_rounds::start_for_transport(
            connection, lease, child, number, &frozen, required, check,
        )?;
        let (pending, receipt, model_log) = match start {
            source_rounds::Start::Received(pending, receipt) => (pending, receipt, None),
            source_rounds::Start::Dispatch(pending) => {
                let model_log = crate::agent_runtime::model::diagnostics::ModelLog::child(
                    context.db_path,
                    lease,
                    child,
                    number,
                    true,
                );
                let messages = pending.request()["messages"]
                    .as_array()
                    .ok_or("source_tool_phase_messages_invalid")?
                    .clone();
                let mut request = agent_model_request(messages, &tools);
                let remaining = crate::agent_runtime::multi_agent::budget::clock::remaining(
                    connection,
                    &lease.root_run_id,
                )?;
                let shared_deadline = std::time::Instant::now() + remaining;
                let deadline = context
                    .deadline
                    .map_or(shared_deadline, |d| d.min(shared_deadline));
                request.request_timeout =
                    Some(deadline.saturating_duration_since(std::time::Instant::now()));
                let scoped_cancel = source_tool_model_cancel_token(context, lease, child);
                let cancel = CancelToken::from_checker(move || {
                    scoped_cancel.is_cancelled() || std::time::Instant::now() >= deadline
                });
                let result = match client.complete_once_observed_with_lifecycle(
                    &request,
                    &cancel,
                    model_log.observer(),
                ) {
                    Ok(result) => result,
                    Err(crate::agent_runtime::model::OneShotFailure::BeforeTransport(error)) => {
                        source_rounds::record_not_sent(connection, &pending, error.code())?;
                        return Err(format!("source_tool_model_failed:{}", error.code()));
                    }
                    Err(crate::agent_runtime::model::OneShotFailure::TransportOutcomeUnknown(
                        error,
                    )) => {
                        source_rounds::record_uncertain(connection, &pending, error.code())?;
                        return Err(format!("source_tool_model_failed:{}", error.code()));
                    }
                };
                client.record_usage_line(context.usage_dir, &result);
                let receipt = source_rounds::record_received(connection, &pending, &result);
                model_log.cost_saved();
                (pending, receipt?, Some(model_log))
            }
        };
        context.require_supervision(lease)?;
        if receipt.response["rejection"] != "" {
            return Err(format!(
                "source_tool_response_rejected:{}",
                receipt.response["rejection"]
            ));
        }
        let calls = receipt.response["toolCalls"]
            .as_array()
            .ok_or("source_tool_phase_response_invalid")?;
        // This SDK response is validated. Tool execution still has its own guards.
        if let Some(log) = model_log {
            log.finish(true);
        }
        for (index, tool) in calls.iter().enumerate() {
            source_rounds::execute_tool(
                connection,
                &pending,
                index as i64,
                check,
                |tx, name, args| source_tool_broker_call(tx, lease, child, name, args),
            )?;
            if tool["name"] == "assignment.finish" {
                return complete_source_tool_assignment(
                    connection, context, lease, child, &pending,
                );
            }
        }
        frozen = source_rounds::continuation(connection, &pending)?;
    }
    close_exhausted_source_tool_assignment(connection, context, lease, child)?;
    Err("source_tool_phase_round_budget_exhausted_without_finish".into())
}

#[cfg(test)]
fn run_source_tool_phases(
    connection: &rusqlite::Connection,
    context: &SpecialistTransportContext<'_>,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<Vec<JsonValue>, String> {
    run_source_tool_phases_from(connection, context, lease, 0)
}

fn run_source_tool_phases_from(
    connection: &rusqlite::Connection,
    context: &SpecialistTransportContext<'_>,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    start_index: usize,
) -> Result<Vec<JsonValue>, String> {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{scheduler, source},
    };
    if start_index > 1 {
        return Err("source_tool_phase_invalid_start_index".into());
    }
    let mut results = Vec::new();
    for (index, role) in [AgentRole::RepoMapper, AgentRole::SourceAnalyst]
        .into_iter()
        .enumerate()
        .skip(start_index)
    {
        context.require_supervision(lease)?;
        crate::agent_runtime::multi_agent::directive::source_guidance::freeze(
            connection, lease, role, true,
        )?;
        let revision:i64=connection.query_row("SELECT COALESCE(MAX(revision),1) FROM agent_evidence_revisions WHERE root_run_id=?1",[&lease.root_run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
        let slice = source::tool_task_slice(connection, lease, role, revision)?;
        let remaining:i64=connection.query_row("SELECT total_tokens-spent_tokens-reserved_tokens FROM agent_budget_ledger WHERE root_run_id=?1",[&lease.root_run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
        // Keep a share of the frozen token budget for the independent v2
        // Reviewer. This does not increase the root's authorized token limit.
        let review_share = i64::from(crate::agent_runtime::multi_agent::source_reviewer::enabled(
            connection, lease,
        )?) + i64::from(
            crate::agent_runtime::multi_agent::source_coverage_reviewer::enabled(
                connection, lease,
            )?,
        );
        let tokens = remaining / (2 + review_share - index as i64);
        if tokens < 1 {
            return Err("source_tool_phase_no_budget".into());
        }
        let child = scheduler::schedule_child(
            connection,
            lease,
            role,
            AgentLane::ReadOnlyAnalysis,
            "source_tools_ready",
            &slice,
            revision,
            &source::tool_capabilities(role)?,
            tokens,
            3,
        )?;
        scheduler::mark_child_running(connection, lease, &child)?;
        let result = execute_source_tool_assignment(connection, context, lease, &child, &slice)
            .map_err(|e| {
                // This exact return follows the atomic known-failure closure.
                // Other failures retain the existing unknown-usage cleanup.
                if e == "source_tool_phase_round_budget_exhausted_without_finish" {
                    e
                } else {
                    failed_specialist_error(connection, lease, &child, &e)
                }
            })?;
        results.push(json!({"role":role.as_str(),"assignmentId":child.assignment_id,"runId":child.run_id,"result":result}));
    }
    Ok(results)
}

include!("native_source_exhausted.rs");
