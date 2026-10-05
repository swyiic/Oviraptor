use super::*;

// A Coordinator can publish an exact saved receipt locally. That publication
// does not turn a failed worker into a successful or executable worker.
pub(crate) fn finish_saved(
    db: &Transaction<'_>,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<(), String> {
    if db.is_autocommit() {
        return Err("assignment_attempt_saved_transaction_required".into());
    }
    let original = current(db, lease, &child.assignment_id)?;
    if original.child_run_id != child.run_id {
        return Err("assignment_attempt_child_conflict".into());
    }
    super::super::specialist::received_for_reconciliation(db, lease, child)?;
    let closed:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
        WHERE a.id=?1 AND r.id=?2 AND a.state='completed' AND a.budget_settled_at<>''
          AND a.reserved_tokens=0 AND a.reserved_requests=0 AND r.status='terminal' AND r.terminal_state='completed')
        AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE child_run_id=?2 AND revoked_at='')
        AND NOT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=?1)",
        params![child.assignment_id,child.run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !closed {
        return Err("assignment_attempt_saved_delivery_not_closed".into());
    }
    match original.state.as_str() {
        "running" | "paused" => finish(db, lease, child, "completed"),
        "failed" if !original.finished_at.is_empty() => Ok(()),
        "expired" => ExpiredSavedProof::capture(db, lease, child)?
            .ok_or_else(|| "expired_saved_proof_required".to_string())
            .map(|_| ()),
        _ => Err("assignment_attempt_saved_state_conflict".into()),
    }
}
