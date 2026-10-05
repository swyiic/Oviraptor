#[allow(clippy::too_many_arguments)]
fn completed_gap_round_valid(
    connection: &rusqlite::Connection,
    root_run_id: &str,
    assignment_id: &str,
    child_run_id: &str,
    candidate_id: &str,
    revision: i64,
    missing_evidence: &JsonValue,
    target_dir: &Path,
) -> Result<bool, String> {
    let terminal: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND root_run_id=?2 \
         AND role='deep_investigator' AND lane='read_only_analysis' \
         AND status='terminal' AND terminal_state='completed')",
            params![child_run_id, root_run_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("gap_replay_child_unavailable:{error}"))?;
    if !terminal {
        return Ok(false);
    }
    let mut statement = connection
        .prepare(
            "SELECT kind,from_run_id,to_run_id,from_agent,to_agent,correlation_id,payload_json,\
         evidence_revision,delivered_at,acknowledged_at FROM agent_messages \
         WHERE assignment_id=?1 AND root_run_id=?2",
        )
        .map_err(|error| format!("gap_replay_messages_unavailable:{error}"))?;
    let mut rows = statement
        .query(params![assignment_id, root_run_id])
        .map_err(|error| format!("gap_replay_messages_unavailable:{error}"))?;
    let correlation = format!("review-gap:{candidate_id}:{revision}");
    let mut proposal = false;
    let mut assessment = false;
    let mut proposal_reason: Option<&str> = None;
    let mut proposal_version = 0;
    let mut assessment_version = 0;
    let mut proposal_requests_new_attempt = false;
    let trusted_fact_refs = review_fact_refs(connection, root_run_id, target_dir)?;
    let mut count = 0;
    while let Some(row) = rows
        .next()
        .map_err(|error| format!("gap_replay_messages_unavailable:{error}"))?
    {
        count += 1;
        if count > 2 {
            return Ok(false);
        }
        let kind: String = row
            .get(0)
            .map_err(|e| format!("gap_replay_message_invalid:{e}"))?;
        let from: String = row
            .get(1)
            .map_err(|e| format!("gap_replay_message_invalid:{e}"))?;
        let to: String = row
            .get(2)
            .map_err(|e| format!("gap_replay_message_invalid:{e}"))?;
        let from_role: String = row
            .get(3)
            .map_err(|e| format!("gap_replay_message_invalid:{e}"))?;
        let to_role: String = row
            .get(4)
            .map_err(|e| format!("gap_replay_message_invalid:{e}"))?;
        let key: String = row
            .get(5)
            .map_err(|e| format!("gap_replay_message_invalid:{e}"))?;
        let raw: String = row
            .get(6)
            .map_err(|e| format!("gap_replay_message_invalid:{e}"))?;
        let message_revision: i64 = row
            .get(7)
            .map_err(|e| format!("gap_replay_message_invalid:{e}"))?;
        let delivered: String = row
            .get(8)
            .map_err(|e| format!("gap_replay_message_invalid:{e}"))?;
        let ack: String = row
            .get(9)
            .map_err(|e| format!("gap_replay_message_invalid:{e}"))?;
        let Ok(payload): Result<JsonValue, _> = serde_json::from_str(&raw) else {
            return Ok(false);
        };
        if message_revision != revision
            || key != correlation
            || delivered.is_empty()
            || ack.is_empty()
            || payload["candidateId"] != candidate_id
            || payload["evidenceRevision"] != revision
        {
            return Ok(false);
        }
        match kind.as_str() {
            "gap_proposed"
                if !proposal
                    && from == child_run_id
                    && to == root_run_id
                    && from_role == "deep_investigator"
                    && to_role == "coordinator"
                    && payload["missingEvidence"]
                        == crate::agent_runtime::secrets::redact_json(missing_evidence)
                    && payload["summary"]
                        .as_str()
                        .is_some_and(|s| !s.trim().is_empty())
                    && matches!(
                        payload["nextStep"].as_str(),
                        Some(
                            "observe_existing_evidence" | "request_new_contract" | "manual_review"
                        )
                    ) =>
            {
                if payload.get("schemaVersion").is_some() || payload.get("proposal").is_some() {
                    let Some(version) = payload["schemaVersion"].as_i64() else {
                        return Ok(false);
                    };
                    let Ok(validated) = parse_gap_proposal_versioned(
                        &payload["proposal"],
                        &trusted_fact_refs,
                        missing_evidence,
                        version,
                    ) else {
                        return Ok(false);
                    };
                    if validated != payload["proposal"]
                        || validated["summary"] != payload["summary"]
                        || validated["nextStep"] != payload["nextStep"]
                    {
                        return Ok(false);
                    }
                    proposal_reason = Some(gap_assessment_reason(&validated, version));
                    proposal_requests_new_attempt = validated["nextStep"] == "request_new_contract";
                    proposal_version = version;
                } else {
                    proposal_reason = Some("proposal_is_not_a_verified_execution_contract");
                }
                proposal = true;
            }
            "proposal_assessed"
                if !assessment
                    && from == root_run_id
                    && to == child_run_id
                    && from_role == "coordinator"
                    && to_role == "deep_investigator"
                    && payload["decision"] == "deferred_requires_new_evidence_revision"
                    && payload["reasonCode"].as_str().is_some() =>
            {
                assessment_version = payload["schemaVersion"].as_i64().unwrap_or(0);
                if assessment_version == 3
                    && (payload["targetRequestsGranted"] != 0
                        || payload["newAttemptRequired"].as_bool().is_none())
                {
                    return Ok(false);
                }
                assessment = true;
            }
            _ => return Ok(false),
        }
    }
    // The SQL result has no ordering guarantee. Re-read the recorded assessment
    // after validating the proposal, and ensure its deterministic decision agrees.
    let assessment_payload: Option<String> = connection.query_row(
        "SELECT payload_json FROM agent_messages WHERE assignment_id=?1 AND root_run_id=?2 AND kind='proposal_assessed'",
        params![assignment_id, root_run_id], |row| row.get(0),
    ).optional().map_err(|error| format!("gap_replay_assessment_unavailable:{error}"))?;
    let assessed = assessment_payload
        .as_deref()
        .and_then(|text| serde_json::from_str::<JsonValue>(text).ok());
    let assessed_reason = assessed
        .as_ref()
        .and_then(|value| value["reasonCode"].as_str());
    let new_attempt_consistent = proposal_version != 3
        || assessed
            .as_ref()
            .and_then(|value| value["newAttemptRequired"].as_bool())
            == Some(proposal_requests_new_attempt);
    Ok(count == 2
        && proposal
        && assessment
        && proposal_version == assessment_version
        && new_attempt_consistent
        && assessed_reason == proposal_reason)
}

/// A failed settlement is not proof of zero usage. Keep its reservation/lane
/// until reconciliation, while revoking execution. Already settled children
/// may use the normal failed-child cleanup (e.g. a later mailbox failure).
fn stop_failed_child_preserving_usage(
    connection: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    reason: &str,
) -> Result<(), String> {
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| format!("child_failure_cleanup_lock:{error}"))?;
    stop_failed_child_preserving_usage_in_transaction(&transaction, lease, child, reason)?;
    transaction
        .commit()
        .map_err(|error| format!("child_failure_cleanup_commit:{error}"))
}

fn failed_specialist_error(
    connection: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    reason: &str,
) -> String {
    // This caller did not own dispatch. A competing caller may still be
    // receiving/publishing its result; don't revoke that worker's permissions
    // or pause its assignment on an observation of the durable claim.
    if reason == "specialist_call_outcome_unknown_requires_reconciliation"
        || reason == "specialist_call_binding_or_request_changed"
        || reason == "specialist_model_not_sent_new_assignment_required"
        || reason == "source_round_outcome_unknown_requires_reconciliation"
        || reason == "source_round_model_not_sent_new_attempt_required"
        || reason == "source_round_original_cost_fact_precludes_execution"
    {
        return reason.to_string();
    }
    match stop_failed_child_preserving_usage(connection, lease, child, reason) {
        Ok(()) => reason.to_string(),
        Err(cleanup) => format!("{reason};specialist_cleanup:{cleanup}"),
    }
}

fn stop_failed_child_preserving_usage_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    reason: &str,
) -> Result<(), String> {
    use crate::agent_runtime::multi_agent::{lease::validate_coordinator_lease, scheduler};
    validate_coordinator_lease(transaction, lease)?;
    if crate::agent_runtime::multi_agent::attempts::try_expire_original_in_transaction(
        transaction,
        lease,
        child,
    )? {
        return Ok(());
    }
    let (settled, state, run_status, reserved_tokens, reserved_requests): (String, String, String, i64, i64) = transaction.query_row(
        "SELECT a.budget_settled_at,a.state,r.status,a.reserved_tokens,a.reserved_requests FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id \
         WHERE a.id=?1 AND a.child_run_id=?2 AND a.coordinator_run_id=?3 AND r.root_run_id=?3 \
         AND a.lease_epoch=?4 AND a.fencing_token=?5 AND r.scan_id=?6 AND r.attempt_number=?7 AND r.target_url=?8",
        params![child.assignment_id,child.run_id,lease.root_run_id,lease.lease_epoch,lease.fencing_token,
            lease.scan_id,lease.attempt_number,lease.target_key],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)),
    ).map_err(|error| format!("child_failure_cleanup_binding:{error}"))?;
    let no_send: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_specialist_calls c WHERE c.assignment_id=?1 \
         AND c.child_run_id=?2 AND c.root_run_id=?3 AND c.role=?4 \
         AND c.lease_epoch=?5 AND c.fencing_token=?6 AND c.state='executing' \
         AND c.failure_code='model_cancelled_before_transport' AND c.finished_at<>'' \
         AND c.response_json='{}' AND c.usage_json='{}' AND c.response_hash='' AND c.event_sequence=0)",
        params![child.assignment_id,child.run_id,lease.root_run_id,child.role.as_str(),
            lease.lease_epoch,lease.fencing_token],
        |row| row.get(0),
    ).map_err(|error| format!("child_no_send_lookup:{error}"))?;
    let source_no_send = crate::agent_runtime::multi_agent::source_rounds::audit_unsent_first(
        transaction,
        lease,
        child,
    )?;
    let no_send = no_send || source_no_send;
    if settled.is_empty() && no_send && state == "running" && run_status == "running" {
        // Zero usage is justified only by the durable typed no-send marker,
        // under the still-current coordinator fence and child ownership.
        settle_child_usage_for_status(
            transaction,
            lease,
            child,
            &AgentTokenUsage::default(),
            "running",
        )?;
    }
    if !settled.is_empty() || (no_send && state == "running" && run_status == "running") {
        // The typed pre-transport proof permits releasing this child only while
        // its current lease still owns the lane. An in-flight cancellation or
        // stale fence never passes this branch and keeps its reservation.
        scheduler::finish_child_in_transaction(transaction, lease, child, false, reason)?;
        let closed: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id \
             WHERE a.id=?1 AND a.child_run_id=?2 AND a.state='failed' AND a.failure_class='child_execution_failed' \
             AND a.budget_settled_at<>'' AND a.reserved_tokens=0 AND a.reserved_requests=0 \
             AND r.status='terminal' AND r.terminal_state='failed' AND r.terminal_code='child_failed') \
             AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE child_run_id=?2 AND revoked_at='') \
             AND NOT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=?1)",
            params![child.assignment_id,child.run_id], |row|row.get(0),
        ).map_err(|error|format!("child_failure_cleanup_postcondition:{error}"))?;
        if !closed
            || (source_no_send
                && !crate::agent_runtime::multi_agent::source_rounds::audit_unsent_first(
                    transaction,
                    lease,
                    child,
                )?)
        {
            return Err("child_failure_cleanup_postcondition".into());
        }
    } else {
        if !matches!(state.as_str(), "running" | "paused")
            || !matches!(run_status.as_str(), "running" | "paused")
        {
            return Err("child_failure_cleanup_state_conflict".into());
        }
        transaction.execute(
            "UPDATE agent_assignments SET state='paused',failure_class='child_usage_reconciliation_required',\
             updated_at=datetime('now','localtime') WHERE id=?1", [&child.assignment_id],
        ).map_err(|error| format!("child_failure_cleanup_assignment:{error}"))?;
        transaction.execute(
            "UPDATE agent_runs SET status='paused',updated_at=datetime('now','localtime') WHERE id=?1", [&child.run_id],
        ).map_err(|error| format!("child_failure_cleanup_run:{error}"))?;
        transaction
            .execute(
                "UPDATE agent_capability_leases SET revoked_at=datetime('now','localtime') \
             WHERE child_run_id=?1 AND assignment_id=?2 AND root_run_id=?3 AND revoked_at=''",
                params![child.run_id, child.assignment_id, lease.root_run_id],
            )
            .map_err(|error| format!("child_failure_cleanup_capability:{error}"))?;
        crate::agent_runtime::multi_agent::attempts::pause(
            transaction,
            lease,
            child,
            "child_usage_reconciliation_required",
        )
        .map_err(|error| format!("child_failure_cleanup_postcondition:{error}"))?;
        // Successful SQL is not proof that the requested state was stored:
        // triggers can silently ignore an update. Verify before committing the
        // surrounding review failure and never discard an unsettled reservation.
        let paused: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id \
             WHERE a.id=?1 AND a.child_run_id=?2 AND a.state='paused' AND r.status='paused' \
             AND a.failure_class='child_usage_reconciliation_required' AND a.budget_settled_at='' \
             AND a.reserved_tokens=?3 AND a.reserved_requests=?4) \
             AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE child_run_id=?2 AND revoked_at='')",
            params![child.assignment_id,child.run_id,reserved_tokens,reserved_requests],
            |row| row.get(0),
        ).map_err(|error| format!("child_failure_cleanup_postcondition:{error}"))?;
        if !paused {
            return Err("child_failure_cleanup_postcondition".into());
        }
    }
    Ok(())
}
