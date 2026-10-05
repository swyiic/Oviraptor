// Deterministic Rust admission over already captured/verified closed facts.
// Model strings never set utility features, risk, cost, scope or capabilities.
#[derive(Clone, Copy)]
enum NativeCoordinatorTrigger {
    ChildOutputEnvelope,
    BudgetAllocation,
    HumanDirective,
    ClientReadonlyOutput,
    IdentityReadonlyOutput,
    ReviewerDecision,
    InvestigatorProposal,
}
impl NativeCoordinatorTrigger {
    fn classify(frame: &NativeCoordinatorFrame) -> Result<Self, String> {
        match frame.kind {
            "human-directive" => Ok(Self::HumanDirective),
            "budget-allocation" => Ok(Self::BudgetAllocation),
            "mapper-output" => Ok(Self::ChildOutputEnvelope),
            "client-side-output" => Ok(Self::ClientReadonlyOutput),
            "identity-session-output" => Ok(Self::IdentityReadonlyOutput),
            "reviewer-decision" => Ok(Self::ReviewerDecision),
            "investigator-proposal" => Ok(Self::InvestigatorProposal),
            _ => Err("root_trigger_not_enabled".into()),
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::ChildOutputEnvelope | Self::ClientReadonlyOutput | Self::IdentityReadonlyOutput | Self::InvestigatorProposal => "child_output_envelope",
            Self::BudgetAllocation => "capability_budget_state_changed",
            Self::HumanDirective => "human_directive",
            Self::ReviewerDecision => "reviewer_decision",
        }
    }
}
fn native_coordinator_cost_weight(amount: i64, limit: Option<i64>) -> i64 {
    match limit {
        Some(limit) if limit > 0 => ((i128::from(amount) * 20 + i128::from(limit) - 1)
            / i128::from(limit))
        .clamp(0, 20) as i64,
        Some(0) if amount == 0 => 0,
        _ => 20,
    }
}
fn native_coordinator_rust_dispatch_policy(
    context: &AgentRunContext,
    frame: &NativeCoordinatorFrame,
    decision: &NativeCoordinatorTickReceipt,
    role: crate::agent_runtime::contract::AgentRole,
    tokens: i64,
    requests: i64,
    limits: [Option<i64>; 3],
) -> Result<JsonValue, String> {
    native_coordinator_rust_dispatch_policy_with_rule(context, frame, decision, role, tokens, requests, limits,
        NativeCoordinatorModelCostRule::OriginalConservative)
}
#[allow(clippy::too_many_arguments)]
fn native_coordinator_rust_dispatch_policy_with_rule(
    context: &AgentRunContext,
    frame: &NativeCoordinatorFrame,
    decision: &NativeCoordinatorTickReceipt,
    role: crate::agent_runtime::contract::AgentRole,
    tokens: i64,
    requests: i64,
    limits: [Option<i64>; 3],
    model_cost_rule: NativeCoordinatorModelCostRule,
) -> Result<JsonValue, String> {
    use crate::agent_runtime::{contract::AgentRole, store};
    if tokens < 0 || requests < 0 {
        return Err("root_decision_cost_invalid".into());
    }
    let trigger = NativeCoordinatorTrigger::classify(frame)?;
    let target = match (trigger, role) {
        (NativeCoordinatorTrigger::ChildOutputEnvelope, AgentRole::WebExecutor) => true,
        (NativeCoordinatorTrigger::ReviewerDecision, AgentRole::DeepInvestigator) => false,
        _ => return Err("root_trigger_role_conflict".into()),
    };
    let summary = decision.summary.as_json();
    let proposals = summary["suggestions"]
        .as_array()
        .ok_or("root_decision_schema_invalid")?;
    let offered = proposals
        .iter()
        .filter(|v| v.as_str() == Some(frame.step()))
        .count();
    if offered == 0 {
        return Err("root_decision_did_not_select_step".into());
    }
    // Same fixed local step = one overlap key. Merge before assigning a lane;
    // duplicate advisory text neither increases benefit nor spends twice.
    let merged = 1;
    let missing = frame.semantic["missingEvidence"]
        .as_array()
        .map_or(0, Vec::len);
    let required = context.execution_plan.frozen_json()["coverage"]["requiredFamilies"]
        .as_array()
        .map_or(0, Vec::len)
        .min(8) as i64;
    // These weights describe permitted coverage work, not a vulnerability
    // severity claim. The independent closed origin earns only provenance;
    // Mapper/model priority strings are not target evidence or information gain.
    let impact = if target { 24 } else { 16 };
    let evidence = 8;
    let information = if !target && missing > 0 { 8 } else { 0 };
    let coverage = if target { required * 4 } else { 4 };
    let blocker = 8;
    let expected_target = if target {
        context.execution_plan.contract_limit.clamp(0, 32)
    } else {
        0
    };
    let target_cost = native_coordinator_cost_weight(expected_target, limits[2]);
    let model_cost = (model_cost_rule.weight(tokens, limits[0])?
        + model_cost_rule.weight(requests, limits[1])? + 1) / 2;
    let risk = if target { 20 } else { 0 };
    let overlap = 0; // All identical fixed-step proposals have already merged.
    let uncertainty = if target { 8 } else { 12 };
    let score = (impact + evidence + information + coverage + blocker
        - target_cost
        - model_cost
        - risk
        - overlap
        - uncertainty)
        .clamp(-100, 100);
    if score <= 0 {
        return Err("root_decision_utility_nonpositive".into());
    }
    Ok(
        json!({"schemaVersion":1,"trigger":trigger.name(),"factHash":store::stable_hash(&frame.fact().to_string()),
        "step":frame.step(),"offeredCount":offered,"mergedProposalCount":merged,
        "ignoredAdvisoryCount":proposals.len()-offered,"score":score,"selection":"accepted",
        "reasonCode":"verified_fact_bounded_coverage_work",
        "features":{"impactCeilingWeight":impact,"evidenceStrength":evidence,
            "informationGain":information,"coverageGain":coverage,"blockerReleaseValue":blocker,
            "targetRequestCost":target_cost,"modelCost":model_cost,"sideEffectRisk":risk,
            "overlapPenalty":overlap,"uncertaintyPenalty":uncertainty},
        "costBasis":{"reservedModelTokens":tokens,"reservedModelRequests":requests,
            "expectedTargetRequests":expected_target,"originalLimits":limits},
        "advisoryOnly":true,"targetEvidenceProven":false}),
    )
}
