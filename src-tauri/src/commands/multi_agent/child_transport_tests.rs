#[cfg(test)]
thread_local! {
    static REAL_CHILD_TRANSPORT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn multi_agent_child_round(
    context: &AgentRunContext,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    system: &str,
    input: JsonValue,
) -> Result<(String, AgentTokenUsage), String> {
    if REAL_CHILD_TRANSPORT.with(std::cell::Cell::get) {
        return multi_agent_child_round_transport(context, lease, child, system, input);
    }
    use crate::agent_runtime::contract::AgentRole;
    let text = match child.role {
        AgentRole::SpaApiMapper => serde_json::json!({
            "summary": "test mapper inspected frozen evidence",
            "priorityContracts": [],
            "risks": []
        }),
        AgentRole::IdentitySession => serde_json::json!({
            "summary": "test identity specialist reviewed scoped identity metadata",
            "observedAuthentication": [],
            "evidenceGaps": [],
            "authorizationProven": false
        }),
        AgentRole::ExternalSurface => serde_json::json!({
            "summary": "test public entry assessment", "observations": [],
            "coverageGaps": ["single anonymous entry only"], "confirmedFindings": false
        }),
        AgentRole::DeepInvestigator => {
            assert!(input.get("frozenEvidence").is_none());
            assert!(input["frozenCandidate"]["evidence"]
                .get("volatileOnly")
                .is_none());
            serde_json::json!({
            "summary": "test investigator assessed the reviewer's frozen evidence gap",
            "nextStep": "request_new_contract",
            "gapCode":"missing_owner_control", "supportingFactRefs":[],
            "missingEvidence":input["reviewerMissingEvidence"], "prerequisites":["operator control group"],
            "proposedContracts":["new_attempt_control_group_request"],
            "expectedInformationGain":0.5, "impactCeiling":"medium",
            "estimatedCost":{"modelTokens":1000,"modelRequests":1,"targetRequests":3},
            "sideEffectClass":"read_only", "overlapKeys":["owner-control"],
            "falsificationCondition":"owner access absent", "stopCondition":"scope rejection"
            })
        }
        AgentRole::EvidenceReviewer => {
            if input.pointer("/evidence/forceReviewerInsufficientForTest")
                == Some(&JsonValue::Bool(true))
            {
                serde_json::json!({
                    "verdict":"insufficient_evidence",
                    "reasonCodes":["fixture_missing_control"],
                    "missingEvidence":["ownership control"],
                    "confidence":0.0,
                    "summary":"test reviewer requested a verified control"
                })
            } else if input.pointer("/evidence/forceReviewerRejectedForTest")
                == Some(&JsonValue::Bool(true))
            {
                serde_json::json!({
                    "verdict":"rejected",
                    "reasonCodes":["fixture_counterevidence"],
                    "missingEvidence":[],
                    "confidence":0.9,
                    "summary":"test reviewer rejected the frozen candidate bundle"
                })
            } else {
                serde_json::json!({
                    "verdict": "confirmed",
                    "reasonCodes": ["test_fixture_confirmed"],
                    "missingEvidence": [],
                    "confidence": 1.0,
                    "summary": "test reviewer confirmed the frozen candidate bundle"
                })
            }
        }
        _ => return Err("unsupported_test_child_role".into()),
    };
    let usage = AgentTokenUsage::default();
    record_child_model_round(context, child, &usage, &text.to_string())?;
    Ok((text.to_string(), usage))
}

#[cfg(test)]
fn record_child_model_round(
    context: &AgentRunContext,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    usage: &AgentTokenUsage,
    summary: &str,
) -> Result<(), String> {
    let ledger = AgentRunLedger {
        db_path: context.db_path.clone(),
        run_id: child.run_id.clone(),
    };
    ledger.model_round(1, usage, &[])?;
    let connection = db::open(&context.db_path)?;
    let mut snapshot = crate::agent_runtime::checkpoint::RunState::new(&child.run_id, Vec::new());
    snapshot.turns = 1;
    snapshot.model_requests = usage.model_requests.max(0);
    snapshot.target_requests =
        crate::agent_runtime::target_requests::child_received(&connection, &child.run_id)?;
    snapshot.input_tokens = usage.input_tokens.max(0);
    snapshot.cached_input_tokens = usage.cached_input_tokens.max(0);
    snapshot.output_tokens = usage.output_tokens.max(0);
    snapshot.used_tokens = usage.total_tokens.max(0);
    snapshot.progress_signature = crate::agent_runtime::store::stable_hash(summary);
    crate::agent_runtime::checkpoint::write_checkpoint(&connection, &snapshot)
}
