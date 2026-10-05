// Restore only delivery of an already received Investigator result. Target
// capabilities, model transport and scheduler activation are never used here.
#[allow(clippy::too_many_arguments)]
fn gap_receipt_already_completed(
    tx: &rusqlite::Transaction<'_>,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    candidate_id: &str,
    revision: i64,
    missing_evidence: &JsonValue,
    target_dir: &Path,
) -> Result<bool, String> {
    let closed: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id \
         WHERE a.id=?1 AND a.child_run_id=?2 AND a.coordinator_run_id=?3 AND a.role='deep_investigator' \
         AND a.lane='read_only_analysis' AND a.trigger_code='reviewer_insufficient_evidence' \
         AND a.evidence_revision=?4 AND a.lease_epoch=?5 AND a.fencing_token=?6 \
         AND a.state='completed' AND a.budget_settled_at<>'' AND a.reserved_tokens=0 AND a.reserved_requests=0 \
         AND r.terminal_code='gap_receipt_reconciled' \
         AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE child_run_id=r.id AND revoked_at='') \
         AND NOT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=a.id))",
        params![child.assignment_id,child.run_id,lease.root_run_id,revision,lease.lease_epoch,lease.fencing_token],
        |row| row.get(0),
    ).map_err(|error|format!("gap_recovery_replay_state:{error}"))?;
    if !closed {
        return Ok(false);
    }
    completed_gap_round_valid(
        tx,
        &lease.root_run_id,
        &child.assignment_id,
        &child.run_id,
        candidate_id,
        revision,
        missing_evidence,
        target_dir,
    )
}

#[allow(clippy::too_many_arguments)]
fn prepare_gap_receipt_completion(
    tx: &rusqlite::Transaction<'_>,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    candidate_id: &str,
    revision: i64,
    missing_evidence: &JsonValue,
    input: &JsonValue,
) -> Result<String, String> {
    use crate::agent_runtime::{
        contract::AgentRole, multi_agent::specialist, secrets::redact_json,
    };
    if child.role != AgentRole::DeepInvestigator {
        return Err("gap_receipt_role_invalid".into());
    }
    let received = specialist::received_for_reconciliation(tx, lease, child)?;
    if !received.rejection.is_empty() {
        return Err("gap_receipt_response_rejected".into());
    }
    let task: String = tx.query_row(
        "SELECT a.task_slice_json FROM agent_assignments a WHERE a.id=?1 AND a.child_run_id=?2 \
         AND a.coordinator_run_id=?3 AND a.role='deep_investigator' AND a.lane='read_only_analysis' \
         AND a.trigger_code='reviewer_insufficient_evidence' AND a.evidence_revision=?4 \
         AND a.lease_epoch=?5 AND a.fencing_token=?6 \
         AND NOT EXISTS(SELECT 1 FROM agent_messages WHERE assignment_id=a.id)",
        params![child.assignment_id,child.run_id,lease.root_run_id,revision,lease.lease_epoch,lease.fencing_token], |row|row.get(0),
    ).map_err(|_|"gap_receipt_assignment_not_recoverable".to_string())?;
    let task: JsonValue = serde_json::from_str(&task).map_err(|_| "gap_receipt_task_invalid")?;
    if task
        != serde_json::json!({"candidateId":candidate_id,"revision":revision,"missingEvidence":redact_json(missing_evidence)})
    {
        return Err("gap_receipt_task_mismatch".into());
    }
    let request: String = tx
        .query_row(
            "SELECT request_json FROM agent_specialist_calls WHERE assignment_id=?1 AND child_run_id=?2",
            params![child.assignment_id,child.run_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let request: JsonValue =
        serde_json::from_str(&request).map_err(|_| "gap_receipt_request_invalid")?;
    let saved_input = request
        .pointer("/messages/1/content")
        .and_then(JsonValue::as_str)
        .and_then(|text| serde_json::from_str::<JsonValue>(text).ok());
    if request["schemaVersion"] != 1
        || request["tools"] != serde_json::json!([])
        || request["messages"].as_array().map(Vec::len) != Some(2)
        || request["messages"][0]["role"] != "system"
        || request["messages"][1]["role"] != "user"
        || saved_input != Some(redact_json(input))
    {
        return Err("gap_receipt_frozen_input_mismatch".into());
    }
    let expired =
        crate::agent_runtime::multi_agent::attempts::ExpiredSavedProof::capture(tx, lease, child)?;
    let (state,recoverable): (String,bool) = tx.query_row(
        "SELECT a.state, NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE child_run_id=r.id AND revoked_at='') \
         AND ((a.state='failed' AND a.failure_class='child_execution_failed' AND a.budget_settled_at<>'' \
           AND a.reserved_tokens=0 AND a.reserved_requests=0 AND r.status='terminal' AND r.terminal_state='failed' \
           AND r.terminal_code='child_failed' AND NOT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=a.id)) \
         OR (a.state='paused' AND (a.failure_class='child_usage_reconciliation_required' OR (?3 AND a.failure_class='worker_lease_expired')) AND a.budget_settled_at='' \
           AND r.status='paused' AND EXISTS(SELECT 1 FROM agent_lane_leases l WHERE l.assignment_id=a.id \
             AND l.scan_id=r.scan_id AND l.attempt_number=r.attempt_number AND l.target_key=r.target_url AND l.lane=a.lane))) \
         FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE a.id=?1 AND r.id=?2",
        params![child.assignment_id,child.run_id,expired.is_some()], |row|Ok((row.get(0)?,row.get(1)?)),
    ).map_err(|error|format!("gap_receipt_state:{error}"))?;
    if !recoverable {
        return Err("gap_receipt_state_not_recoverable".into());
    }
    settle_child_usage_for_status(tx, lease, child, &received.usage, "paused")?;
    let assignment = tx.execute(
        "UPDATE agent_assignments SET state='completed',failure_class='',finished_at=datetime('now','localtime'),updated_at=datetime('now','localtime') \
         WHERE id=?1 AND child_run_id=?2 AND state=?3 AND budget_settled_at<>'' AND reserved_tokens=0 AND reserved_requests=0",
        params![child.assignment_id,child.run_id,state],
    ).map_err(|error|error.to_string())?;
    let run = tx.execute(
        "UPDATE agent_runs SET status='terminal',terminal_state='completed',terminal_code='gap_receipt_reconciled', \
         terminal_reason=terminal_reason || ' [Saved gap assessment delivered locally; no model retry or capability restoration]', \
         finished_at=datetime('now','localtime'),updated_at=datetime('now','localtime') \
         WHERE id=?1 AND root_run_id=?2 AND (status='paused' OR (status='terminal' AND terminal_state='failed'))",
        params![child.run_id,lease.root_run_id],
    ).map_err(|error|error.to_string())?;
    if assignment != 1 || run != 1 {
        return Err("gap_receipt_completion_write_conflict".into());
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
    Ok(received.text)
}

fn verify_gap_delivery_events(
    tx: &rusqlite::Transaction<'_>,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    proposal_id: &str,
    assessment_id: &str,
    event_floor: i64,
) -> Result<(), String> {
    let visible: i64 = tx.query_row(
        "SELECT COUNT(DISTINCT entity_type || ':' || entity_id) FROM agent_collaboration_events e \
         WHERE sequence>?1 AND scan_id=?2 AND attempt_number=?3 AND entity_type=event_type AND ( \
           (event_type='assignment' AND entity_id=?4 AND json_extract(payload_json,'$.state')='completed') \
           OR (event_type='agent_run' AND entity_id=?5 AND json_extract(payload_json,'$.status')='terminal' \
             AND json_extract(payload_json,'$.terminalState')='completed') \
           OR (event_type='mailbox_message' AND entity_id IN (?6,?7) \
             AND EXISTS(SELECT 1 FROM agent_messages m WHERE m.id=e.entity_id \
               AND m.delivered_at<>'' AND m.acknowledged_at<>'' \
               AND json_extract(e.payload_json,'$.kind')=m.kind \
               AND json_extract(e.payload_json,'$.deliveredAt')=m.delivered_at \
               AND json_extract(e.payload_json,'$.acknowledgedAt')=m.acknowledged_at)))",
        params![event_floor,lease.scan_id,lease.attempt_number,child.assignment_id,child.run_id,proposal_id,assessment_id], |row|row.get(0),
    ).map_err(|error|format!("gap_delivery_event_postcondition:{error}"))?;
    if visible != 4 {
        return Err("gap_delivery_event_postcondition_failed".into());
    }
    Ok(())
}
