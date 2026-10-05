#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GapProposalCost {
    model_tokens: u64,
    model_requests: u64,
    target_requests: u64,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GapProposalWire {
    summary: String,
    next_step: String,
    gap_code: String,
    supporting_fact_refs: Vec<String>,
    missing_evidence: Vec<String>,
    prerequisites: Vec<String>,
    proposed_contracts: Vec<String>,
    expected_information_gain: f64,
    impact_ceiling: String,
    estimated_cost: GapProposalCost,
    side_effect_class: String,
    overlap_keys: Vec<String>,
    falsification_condition: String,
    stop_condition: String,
}

/// Validates the *suggestion*, not an execution contract. Only the Broker may
/// validate and claim a target request; no field here becomes an authorization.
fn parse_gap_proposal(
    value: &JsonValue,
    trusted_fact_refs: &[String],
    reviewer_missing_evidence: &JsonValue,
) -> Result<JsonValue, String> {
    parse_gap_proposal_versioned(value, trusted_fact_refs, reviewer_missing_evidence, 3)
}

// V2 is read only for acknowledged historical mailbox rounds. A new round
// cannot describe a not-yet-approved control group as an approved contract.
fn parse_gap_proposal_versioned(
    value: &JsonValue,
    trusted_fact_refs: &[String],
    reviewer_missing_evidence: &JsonValue,
    schema_version: i64,
) -> Result<JsonValue, String> {
    if !matches!(schema_version, 2 | 3) {
        return Err("gap_proposal_unknown_version".into());
    }
    let proposal: GapProposalWire = serde_json::from_value(value.clone())
        .map_err(|_| "gap_proposal_invalid_schema".to_string())?;
    let bounded = |text: &str, max: usize| !text.trim().is_empty() && text.chars().count() <= max;
    let strings = |values: &[String], max_items: usize| {
        values.len() <= max_items && values.iter().all(|text| bounded(text, 240))
    };
    if !bounded(&proposal.summary, 500)
        || !bounded(&proposal.falsification_condition, 240)
        || !bounded(&proposal.stop_condition, 240)
        || !bounded(&proposal.gap_code, 64)
        || !proposal.gap_code.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
        || !strings(&proposal.supporting_fact_refs, 16)
        || !strings(&proposal.missing_evidence, 8) || proposal.missing_evidence.is_empty()
        || !strings(&proposal.prerequisites, 8)
        || !strings(&proposal.proposed_contracts, 2)
        || !strings(&proposal.overlap_keys, 8)
        || !proposal.expected_information_gain.is_finite()
        || !(0.0..=1.0).contains(&proposal.expected_information_gain)
        || !matches!(proposal.impact_ceiling.as_str(), "low" | "medium" | "high" | "critical")
        || proposal.side_effect_class != "read_only"
        || proposal.estimated_cost.model_tokens > 4_000
        || proposal.estimated_cost.model_requests > 1
        || proposal.estimated_cost.target_requests > 3
        || !matches!(proposal.next_step.as_str(),
            "observe_existing_evidence" | "request_new_contract" | "manual_review")
    {
        return Err("gap_proposal_invalid_bounds".into());
    }
    if proposal.supporting_fact_refs.iter().any(|reference| !trusted_fact_refs.contains(reference)) {
        return Err("gap_proposal_unattributed_fact".into());
    }
    if proposal.proposed_contracts.iter().any(|contract| !matches!(contract.as_str(),
        "existing_evidence_review" | "operator_approved_control_group" | "new_attempt_control_group_request")) {
        return Err("gap_proposal_unknown_contract".into());
    }
    // The specialist may describe a proposed way to fill the Reviewer's gap,
    // but must not replace that gap with a different, easier-to-satisfy one.
    // This is checked again on mailbox replay, not just before the first send.
    if serde_json::json!(proposal.missing_evidence)
        != crate::agent_runtime::secrets::redact_json(reviewer_missing_evidence)
    {
        return Err("gap_proposal_reviewer_gap_mismatch".into());
    }
    let contract_consistent = match proposal.next_step.as_str() {
        "manual_review" => proposal.proposed_contracts.is_empty()
            && proposal.estimated_cost.target_requests == 0,
        "observe_existing_evidence" => proposal.proposed_contracts == ["existing_evidence_review"]
            && proposal.estimated_cost.target_requests == 0,
        "request_new_contract" => proposal.proposed_contracts == [if schema_version == 3 {
                "new_attempt_control_group_request"
            } else {
                "operator_approved_control_group"
            }]
            && proposal.estimated_cost.target_requests == 3,
        _ => false,
    };
    if !contract_consistent {
        return Err("gap_proposal_contract_cost_mismatch".into());
    }
    serde_json::to_value(proposal).map_err(|_| "gap_proposal_encoding_failed".into())
}

fn gap_assessment_reason(proposal: &JsonValue, schema_version: i64) -> &'static str {
    match proposal["nextStep"].as_str() {
        Some("manual_review") => "human_review_required",
        Some("observe_existing_evidence") => "no_new_verified_fact_in_revision",
        _ if schema_version == 3 => "operator_approval_new_attempt_required",
        _ => "proposal_is_not_a_verified_execution_contract",
    }
}
