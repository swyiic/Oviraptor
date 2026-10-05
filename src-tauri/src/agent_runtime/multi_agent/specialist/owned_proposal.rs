use super::*;

/// Original human-proposal accounting audit only. A frozen proof in the actual
/// provider dispatch distinguishes this path from legacy direct SDK records.
/// The caller verifies the proposal and full Native Root contract in this same
/// transaction. This never grants execution, retries, renewals or mailbox ACK.
pub(crate) fn received_for_owned_proposal(
    db: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
    proof: &Value,
) -> Result<StoredResponse, String> {
    if db.is_autocommit()
        || !matches!(
            child.role,
            AgentRole::SpaApiMapper | AgentRole::DeepInvestigator
        )
    {
        return Err("proposal_original_receipt_context_invalid".into());
    }
    let (request, request_hash): (String, String) = db.query_row(
        "SELECT request_json,request_hash FROM agent_specialist_calls WHERE assignment_id=?1 AND child_run_id=?2",
        params![child.assignment_id,child.run_id], |r| Ok((r.get(0)?,r.get(1)?)),
    ).map_err(|_| "proposal_original_dispatch_required")?;
    let value: Value =
        serde_json::from_str(&request).map_err(|_| "proposal_original_dispatch_invalid")?;
    if value["humanDirectiveDispatch"] != *proof {
        return Err("proposal_original_dispatch_binding_conflict".into());
    }
    let original = super::super::budget::receipts::original_owner(
        db,
        &child.run_id,
        lease,
        &child.assignment_id,
    )?;
    original.verify(db)?;
    if proof["leaseAttemptId"] != original.attempt_id() {
        return Err("proposal_original_worker_binding_conflict".into());
    }
    let call = PendingCall {
        lease: lease.clone(),
        child: child.clone(),
        request_hash,
    };
    let row = load(db, &call)?.ok_or("proposal_original_dispatch_required")?;
    verify_received(db, &call, &row)
}
