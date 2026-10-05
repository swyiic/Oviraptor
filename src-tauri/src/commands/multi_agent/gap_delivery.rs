#[allow(clippy::too_many_arguments)]
fn complete_gap_delivery(
    connection: &rusqlite::Connection,
    context: &AgentRunContext,
    session: &MultiAgentSession,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    candidate_id: &str,
    revision: i64,
    missing_evidence: &JsonValue,
    input: &JsonValue,
    text: &str,
    receipt_recovery: bool,
) -> Result<(), String> {
    use crate::agent_runtime::{
        contract::{AgentMessageKind, AgentRole},
        multi_agent::{mailbox, scheduler},
    };
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| format!("gap_delivery_lock:{error}"))?;
    crate::agent_runtime::multi_agent::lease::require_executable_coordinator(
        &transaction,
        &session.lease,
    )?;
    let expired = crate::agent_runtime::multi_agent::attempts::ExpiredSavedProof::capture(
        &transaction,
        &session.lease,
        child,
    )?;
    let event_floor: i64 = transaction
        .query_row(
            "SELECT COALESCE(MAX(sequence),0) FROM agent_collaboration_events",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("gap_delivery_event_cursor:{error}"))?;
    let (current_candidate, current_missing, trusted_fact_refs) =
        sealed_gap_review_candidate_in_transaction(
            &transaction,
            &session.lease.root_run_id,
            candidate_id,
            revision,
            missing_evidence,
            &context.target_dir,
        )?;
    let current_input = serde_json::json!({"candidateId":candidate_id,"evidenceRevision":revision,
        "reviewerMissingEvidence":current_missing,"trustedFactRefs":trusted_fact_refs,"frozenCandidate":current_candidate});
    if *input != current_input {
        return Err("gap_review_candidate_changed".into());
    }
    // Another caller may have completed the same saved receipt while this caller
    // waited for the write lock. Accept only the fully sealed delivery, without
    // replaying mailbox writes, settlement, or timeline events.
    if receipt_recovery
        && gap_receipt_already_completed(
            &transaction,
            &session.lease,
            child,
            candidate_id,
            revision,
            missing_evidence,
            &context.target_dir,
        )?
    {
        if let Some(proof) = &expired {
            proof.verify(&transaction, &session.lease, child)?;
        }
        return transaction
            .commit()
            .map_err(|error| format!("gap_replay_commit:{error}"));
    }
    let recovered_text = if receipt_recovery {
        Some(prepare_gap_receipt_completion(
            &transaction,
            &session.lease,
            child,
            candidate_id,
            revision,
            missing_evidence,
            input,
        )?)
    } else {
        None
    };
    let text = recovered_text.as_deref().unwrap_or(text);
    let raw: JsonValue =
        serde_json::from_str(text).map_err(|_| "gap_proposal_invalid_json".to_string())?;
    let current_fact_refs = review_fact_refs(
        &transaction,
        &session.lease.root_run_id,
        &context.target_dir,
    )?;
    let still_trusted: Vec<String> = trusted_fact_refs
        .into_iter()
        .filter(|reference| current_fact_refs.contains(reference))
        .collect();
    let parsed = parse_gap_proposal(&raw, &still_trusted, missing_evidence)?;
    let summary = parsed
        .get("summary")
        .and_then(JsonValue::as_str)
        .filter(|value| !value.trim().is_empty() && value.chars().count() <= 500)
        .ok_or_else(|| "gap_proposal_invalid_summary".to_string())?;
    let next_step = parsed
        .get("nextStep")
        .and_then(JsonValue::as_str)
        .filter(|value| {
            matches!(
                *value,
                "observe_existing_evidence" | "request_new_contract" | "manual_review"
            )
        })
        .ok_or_else(|| "gap_proposal_invalid_next_step".to_string())?;
    let correlation = format!("review-gap:{candidate_id}:{revision}");
    let proposal_payload = crate::agent_runtime::secrets::redact_json(&serde_json::json!({
        "candidateId":candidate_id,
        "evidenceRevision":revision,
        "summary":summary,
        "nextStep":next_step,
        "missingEvidence":missing_evidence,
        "schemaVersion":3,
        "proposal":parsed,
    }));
    let send = if receipt_recovery {
        mailbox::send_saved_specialist
    } else {
        mailbox::send
    };
    let proposal_id = send(
        &transaction,
        &session.lease,
        &child.run_id,
        &session.lease.root_run_id,
        AgentRole::DeepInvestigator.as_str(),
        AgentRole::Coordinator.as_str(),
        AgentMessageKind::GapProposed.as_str(),
        &correlation,
        &child.assignment_id,
        revision,
        &proposal_payload,
    )?;
    consume_proposal_mailbox_in_transaction(
        &transaction,
        &session.lease,
        &session.lease.root_run_id,
        &proposal_id,
        AgentMessageKind::GapProposed.as_str(),
        &proposal_payload,
    )?;
    let assessment_payload = crate::agent_runtime::secrets::redact_json(&serde_json::json!({
        "candidateId":candidate_id,
        "evidenceRevision":revision,
        "summary":"当前缺口需要新的已校验合同或证据 revision，暂不派发目标请求",
        "decision":"deferred_requires_new_evidence_revision",
        "reasonCode":gap_assessment_reason(&parsed, 3),
        "newAttemptRequired":next_step == "request_new_contract",
        "targetRequestsGranted":0,
        "schemaVersion":3,
    }));
    let assessment_id = send(
        &transaction,
        &session.lease,
        &session.lease.root_run_id,
        &child.run_id,
        AgentRole::Coordinator.as_str(),
        AgentRole::DeepInvestigator.as_str(),
        AgentMessageKind::ProposalAssessed.as_str(),
        &correlation,
        &child.assignment_id,
        revision,
        &assessment_payload,
    )?;
    consume_proposal_mailbox_in_transaction(
        &transaction,
        &session.lease,
        &child.run_id,
        &assessment_id,
        AgentMessageKind::ProposalAssessed.as_str(),
        &assessment_payload,
    )?;
    if !receipt_recovery {
        scheduler::finish_child_in_transaction(&transaction, &session.lease, child, true, summary)?;
    }
    if !completed_gap_round_valid(
        &transaction,
        &session.lease.root_run_id,
        &child.assignment_id,
        &child.run_id,
        candidate_id,
        revision,
        missing_evidence,
        &context.target_dir,
    )? {
        return Err("gap_delivery_postcondition_failed".into());
    }
    let closed: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_assignments WHERE id=?1 AND state='completed' \
         AND budget_settled_at<>'' AND reserved_tokens=0 AND reserved_requests=0) \
         AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE child_run_id=?2 AND revoked_at='') \
         AND NOT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=?1)",
        params![child.assignment_id,child.run_id], |row| row.get(0),
    ).map_err(|error| format!("gap_delivery_cleanup_postcondition:{error}"))?;
    if !closed {
        return Err("gap_delivery_cleanup_postcondition_failed".into());
    }
    verify_gap_delivery_events(
        &transaction,
        &session.lease,
        child,
        &proposal_id,
        &assessment_id,
        event_floor,
    )?;
    crate::agent_runtime::multi_agent::lease::require_executable_coordinator(
        &transaction,
        &session.lease,
    )?;
    if let Some(proof) = expired {
        proof.verify(&transaction, &session.lease, child)?;
    }
    crate::agent_runtime::multi_agent::lease::validate_coordinator_lease(
        &transaction,
        &session.lease,
    )?;
    transaction
        .commit()
        .map_err(|error| format!("gap_delivery_commit:{error}"))
}
