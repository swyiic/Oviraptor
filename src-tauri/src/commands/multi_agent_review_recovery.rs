// Local completion of a saved Reviewer response. This is deliberately not an
// executor resume API: no dispatch, lease renewal or capability grant occurs.
#[allow(clippy::too_many_arguments)]
fn prepare_review_receipt_completion(
    tx: &rusqlite::Transaction<'_>,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    request_id: &str,
    candidate_id: &str,
    revision: i64,
    decision: &ValidatedReviewDecision,
    target_dir: &Path,
) -> Result<(), String> {
    use crate::agent_runtime::{contract::AgentRole, multi_agent::specialist};
    if child.role != AgentRole::EvidenceReviewer {
        return Err("review_receipt_role_invalid".into());
    }
    let received = specialist::received_for_reconciliation(tx, lease, child)?;
    if !received.rejection.is_empty() || validated_review_decision(&received.text)? != *decision {
        return Err("review_receipt_decision_mismatch".into());
    }
    let candidate: String = tx.query_row(
        "SELECT q.candidate_json FROM agent_review_requests q JOIN agent_assignments a ON a.id=q.assignment_id \
         WHERE q.id=?1 AND q.root_run_id=?2 AND q.reviewer_run_id=?3 AND q.assignment_id=?4 \
         AND q.candidate_id=?5 AND q.candidate_revision=?6 AND q.status='failed' AND q.decision_id IS NULL \
         AND q.lease_epoch=?7 AND q.fencing_token=?8 AND a.evidence_revision=?6 AND a.trigger_code='candidate_ready' \
         AND json_extract(a.task_slice_json,'$.candidateId')=?5 \
         AND json_extract(a.task_slice_json,'$.candidateRevision')=?6 \
         AND NOT EXISTS(SELECT 1 FROM agent_review_requests newer WHERE newer.root_run_id=q.root_run_id \
           AND newer.candidate_id=q.candidate_id AND newer.candidate_revision>q.candidate_revision) \
         AND NOT EXISTS(SELECT 1 FROM agent_review_decisions WHERE reviewer_run_id=?3) \
         AND NOT EXISTS(SELECT 1 FROM agent_messages WHERE assignment_id=?4)",
        params![request_id,lease.root_run_id,child.run_id,child.assignment_id,candidate_id,revision,lease.lease_epoch,lease.fencing_token],
        |row| row.get(0),
    ).map_err(|_| "review_receipt_request_not_recoverable".to_string())?;
    verify_review_snapshot(tx, &lease.root_run_id, candidate_id, revision, target_dir)?;
    let candidate: JsonValue =
        serde_json::from_str(&candidate).map_err(|_| "review_receipt_candidate_invalid")?;
    let request: String = tx
        .query_row(
            "SELECT request_json FROM agent_specialist_calls WHERE assignment_id=?1 AND child_run_id=?2",
            params![child.assignment_id,child.run_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let request: JsonValue =
        serde_json::from_str(&request).map_err(|_| "review_receipt_model_request_invalid")?;
    let model_candidate = request
        .pointer("/messages/1/content")
        .and_then(JsonValue::as_str)
        .and_then(|text| serde_json::from_str::<JsonValue>(text).ok());
    if request["schemaVersion"] != 1
        || request["tools"] != serde_json::json!([])
        || request["messages"].as_array().map(Vec::len) != Some(2)
        || request["messages"][0]["role"] != "system"
        || request["messages"][1]["role"] != "user"
        || model_candidate != Some(crate::agent_runtime::secrets::redact_json(&candidate))
    {
        return Err("review_receipt_candidate_input_mismatch".into());
    }
    let expired =
        crate::agent_runtime::multi_agent::attempts::ExpiredSavedProof::capture(tx, lease, child)?;
    let (state, recoverable): (String,bool) = tx.query_row(
        "SELECT a.state, NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE child_run_id=r.id AND revoked_at='') \
         AND ((a.state='failed' AND a.failure_class='child_execution_failed' AND a.budget_settled_at<>'' \
           AND a.reserved_tokens=0 AND a.reserved_requests=0 AND r.status='terminal' AND r.terminal_state='failed' \
           AND r.terminal_code='child_failed' AND NOT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=a.id)) \
         OR (a.state='paused' AND (a.failure_class='child_usage_reconciliation_required' OR (?3 AND a.failure_class='worker_lease_expired')) AND a.budget_settled_at='' \
           AND r.status='paused' AND EXISTS(SELECT 1 FROM agent_lane_leases l WHERE l.assignment_id=a.id \
             AND l.scan_id=r.scan_id AND l.attempt_number=r.attempt_number AND l.target_key=r.target_url AND l.lane=a.lane))) \
         FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE a.id=?1 AND r.id=?2",
        params![child.assignment_id,child.run_id,expired.is_some()], |row|Ok((row.get(0)?,row.get(1)?)),
    ).map_err(|error|format!("review_receipt_state:{error}"))?;
    if !recoverable {
        return Err("review_receipt_state_not_recoverable".into());
    }
    // A settled failed worker is only checked, never charged twice. A paused
    // worker is charged under its existing reservation without reactivation.
    settle_child_usage_for_status(tx, lease, child, &received.usage, "paused")?;
    let changed = tx.execute(
        "UPDATE agent_assignments SET state='completed',failure_class='',finished_at=datetime('now','localtime'),updated_at=datetime('now','localtime') \
         WHERE id=?1 AND child_run_id=?2 AND state=?3 AND budget_settled_at<>'' AND reserved_tokens=0 AND reserved_requests=0",
        params![child.assignment_id,child.run_id,state],
    ).map_err(|error|error.to_string())?;
    let finished = tx.execute(
        "UPDATE agent_runs SET status='terminal',terminal_state='completed',terminal_code='review_receipt_reconciled', \
         terminal_reason=terminal_reason || ' [Saved review delivered locally; no model retry or capability restoration]', \
         finished_at=datetime('now','localtime'),updated_at=datetime('now','localtime') \
         WHERE id=?1 AND root_run_id=?2 AND (status='paused' OR (status='terminal' AND terminal_state='failed'))",
        params![child.run_id,lease.root_run_id],
    ).map_err(|error|error.to_string())?;
    if changed != 1 || finished != 1 {
        return Err("review_receipt_completion_write_conflict".into());
    }
    // Prove the original worker's occupied slot before deleting its lane.
    // The surrounding transaction verifies deletion and rolls back on damage.
    if state == "paused" {
        crate::agent_runtime::multi_agent::budget::limits::release_slot(
            tx,
            lease,
            &child.assignment_id,
        )?;
    }
    tx.execute(
        "DELETE FROM agent_lane_leases WHERE assignment_id=?1",
        [&child.assignment_id],
    )
    .map_err(|error| error.to_string())?;
    crate::agent_runtime::multi_agent::attempts::finish_saved(tx, lease, child)?;
    Ok(())
}

fn recover_received_review(
    connection: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    candidate_id: &str,
    revision: i64,
    candidate_text: &str,
    target_dir: &Path,
) -> Result<ValidatedReviewDecision, String> {
    use crate::agent_runtime::{
        contract::AgentRole,
        multi_agent::{
            lease::require_executable_coordinator, scheduler::ScheduledChild, specialist,
        },
    };
    let tx =
        rusqlite::Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| format!("review_receipt_lock:{error}"))?;
    require_executable_coordinator(&tx, lease)?;
    let (request_id,assignment_id,run_id): (String,String,String) = tx.query_row(
        "SELECT id,assignment_id,reviewer_run_id FROM agent_review_requests WHERE root_run_id=?1 \
         AND candidate_id=?2 AND candidate_revision=?3 AND candidate_json=?4 AND status='failed' \
         AND lease_epoch=?5 AND fencing_token=?6",
        params![lease.root_run_id,candidate_id,revision,candidate_text,lease.lease_epoch,lease.fencing_token],
        |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).map_err(|_|"review_receipt_request_not_recoverable".to_string())?;
    let child = ScheduledChild {
        assignment_id,
        run_id,
        role: AgentRole::EvidenceReviewer,
    };
    let received = specialist::received_for_reconciliation(&tx, lease, &child)?;
    let decision = validated_review_decision(&received.text)?;
    complete_review_delivery_in_transaction(
        &tx,
        lease,
        &child,
        &request_id,
        candidate_id,
        revision,
        &decision,
        target_dir,
        true,
    )?;
    tx.commit()
        .map_err(|error| format!("review_receipt_commit:{error}"))?;
    Ok(decision)
}
