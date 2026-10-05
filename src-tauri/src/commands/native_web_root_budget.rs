// Creation-only budget choice from the already private, frozen Web start proof.
// No browser/write/upload or actual race dispatcher is implemented here.
fn fresh_web_root_budget_declaration(
    proof: &crate::agent_runtime::web_mode::VerifiedMode,
    target: &str,
    plan: &AgentExecutionPlan,
) -> Result<
    crate::agent_runtime::multi_agent::budget::root_definition::NewRootBudgetDeclaration,
    String,
> {
    use crate::agent_runtime::multi_agent::budget::root_definition::NewRootBudgetDeclaration;
    let fact = proof.fact();
    if fact.attempt_number != plan.attempt_number
        || plan.target_url != target
        || !fact.targets.iter().any(|v| v == target)
    {
        return Err("web_budget_private_creation_scope_changed".into());
    }
    let finite = |n| (n > 0).then_some(n);
    let declaration = NewRootBudgetDeclaration {
        schema_version: 2,
        declaration_id: uuid::Uuid::new_v4().to_string(),
        scan_id: fact.scan_id.clone(),
        attempt_number: fact.attempt_number,
        target_url: target.into(),
        plan_hash: plan.hash(),
        execution_slot_capacity: 3,
        limits: [
            finite(plan.hard_total_tokens),
            finite(plan.hard_total_tokens),
            finite(plan.hard_total_tokens),
            finite(plan.hard_model_requests),
            Some(plan.hard_model_requests.max(1).saturating_mul(4).min(400)),
            Some(0),
            Some(0),
            Some(0),
            Some(0), // Explicit actual race batches, independent of child slots.
            Some(
                i64::try_from(plan.timeout_seconds)
                    .ok()
                    .and_then(|n| n.checked_mul(1000))
                    .ok_or("web_budget_timeout_invalid")?,
            ),
        ],
    };
    declaration.validate(
        &fact.scan_id,
        fact.attempt_number,
        target,
        &plan.hash(),
        &plan.as_json(),
    )?;
    Ok(declaration)
}
