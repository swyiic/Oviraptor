// Only uncompleted initial source roles are scheduled. The caller supplies a
// read-only audited prefix; completed children are never re-entered just to
// discover their already acknowledged model and mailbox receipts again.
fn run_source_initial_phases_from(
    connection: &rusqlite::Connection,
    context: &SpecialistTransportContext<'_>,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    profile: &AgentModelProfile,
    start_index: usize,
) -> Result<Vec<JsonValue>, String> {
    use crate::agent_runtime::{contract::AgentRole, multi_agent::{lease as leases, scheduler, source}};
    if start_index > 2 {
        return Err("source_initial_phase_invalid_start_index".into());
    }
    context.require_supervision(lease)?;
    let mut assessments = Vec::new();
    for role in [AgentRole::RepoMapper, AgentRole::SourceAnalyst]
        .into_iter()
        .skip(start_index)
    {
        if context.deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline) {
            return Err("source_model_deadline_exceeded".into());
        }
        context.require_supervision(lease)?;
        leases::validate_coordinator_lease(connection, lease)?;
        crate::agent_runtime::multi_agent::directive::source_guidance::freeze(connection, lease, role, false)?;
        let input = source::assessment_input(connection, lease, role)?;
        let slice = source::task_slice(connection, lease, role)?;
        let (tokens, _) = source_assessment_budget(
            &source_assessment_messages(SOURCE_ASSESSMENT_SYSTEM, &input), profile,
        )?;
        let child = scheduler::prepare_readonly_child(connection, lease, role, "source_results_ready", &slice, tokens)?;
        let (text, usage) = specialist_round_transport(context, lease, &child, SOURCE_ASSESSMENT_SYSTEM, input)
            .map_err(|error| failed_specialist_error(connection, lease, &child, &error))?;
        context.require_supervision(lease)?;
        let result = deliver_readonly_assessment(
            connection, lease, &child, &usage, &json!({"sourceTask":slice,"summary":text}), None,
        )?;
        assessments.push(json!({"role":role.as_str(),"assignmentId":child.assignment_id,
            "runId":child.run_id,"assessment":result}));
    }
    Ok(assessments)
}
