//! Typed BeforeTransport accounting proof; optional logs never authorize refunds.
use super::*;
pub(crate) struct UnsentCall { call: PendingCall, proof: Value }
pub(crate) fn not_sent_for_owned_proposal(
    db: &Connection, lease: &CoordinatorLease, child: &ScheduledChild, proof: &Value,
) -> Result<Option<UnsentCall>, String> {
    if db.is_autocommit() || !matches!(child.role, AgentRole::SpaApiMapper | AgentRole::DeepInvestigator) {
        return Err("proposal_unsent_original_context_required".into());
    }
    let original = super::super::budget::receipts::original_owner(db, &child.run_id, lease, &child.assignment_id)?;
    original.verify(db)?;
    let worker = super::super::attempts::current(db, lease, &child.assignment_id)?;
    if proof["leaseAttemptId"] != original.attempt_id() || proof["workerId"] != worker.worker_id {
        return Err("proposal_unsent_original_worker_conflict".into());
    }
    let saved: Option<(String,String,String)> = db.query_row("SELECT request_json,request_hash,finished_at FROM agent_specialist_calls WHERE assignment_id=?1 AND child_run_id=?2",
        params![child.assignment_id,child.run_id], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|e|e.to_string())?;
    let Some((raw,hash,finished)) = saved else { return Ok(None); };
    let unique: bool = db.query_row("SELECT COUNT(*)=1 FROM agent_specialist_calls WHERE assignment_id=?1 OR child_run_id=?2",
        params![child.assignment_id,child.run_id], |r|r.get(0)).map_err(|e|e.to_string())?;
    if !unique { return Err("proposal_unsent_original_dispatch_conflict".into()); }
    let request: Value = serde_json::from_str(&raw).map_err(|_|"proposal_unsent_original_request_invalid")?;
    if request["humanDirectiveDispatch"] != *proof { return Err("proposal_unsent_original_request_conflict".into()); }
    let call = PendingCall { lease: lease.clone(), child: child.clone(), request_hash: hash };
    let row = load(db, &call)?.ok_or("proposal_unsent_original_dispatch_missing")?;
    if row.failure_code != "model_cancelled_before_transport" { return Ok(None); }
    if row.state != "executing" || finished.is_empty() || row.sequence != 0 || !row.response_hash.is_empty()
        || row.response != serde_json::json!({}) || row.usage != serde_json::json!({}) {
        return Err("proposal_unsent_receipt_conflict".into());
    }
    // Reject contradictory diagnostics; missing logs cannot replace a receipt.
    let conflict: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM native_sdk_log_owners o WHERE (o.assignment_id=?1 OR o.run_id=?2)
        AND (o.domain<>'specialist' OR o.assignment_id IS NOT ?1 OR o.run_id<>?2 OR o.root_run_id<>?3 OR o.scan_id<>?4
            OR o.attempt_number<>?5 OR o.lease_attempt_id<>?6 OR o.worker_id IS NOT ?7 OR o.round_number<>1 OR o.request_hash<>?8))
        OR (SELECT COUNT(*) FROM native_sdk_log_owners WHERE assignment_id=?1 OR run_id=?2)>1
        OR EXISTS(SELECT 1 FROM native_sdk_log_rows r JOIN native_sdk_log_owners o ON o.owner_id=r.owner_id
            WHERE (o.assignment_id=?1 OR o.run_id=?2) AND (r.stage IN ('sent','response_received','validated')
                OR r.cost_phase IN ('received','uncertain') OR r.terminal_state IN ('returned','withheld','uncertain')))
        OR EXISTS(SELECT 1 FROM native_sdk_log_gaps g JOIN native_sdk_log_owners o ON o.owner_id=g.owner_id
            WHERE (o.assignment_id=?1 OR o.run_id=?2) AND g.failed_stage IN ('sent','response_received','validated'))",
        params![child.assignment_id,child.run_id,lease.root_run_id,lease.scan_id,lease.attempt_number,original.attempt_id(),worker.worker_id,call.request_hash], |r|r.get(0)).map_err(|e|e.to_string())?;
    if conflict { return Err("proposal_unsent_transport_history_conflict".into()); }
    Ok(Some(UnsentCall { call, proof: proof.clone() }))
}
impl UnsentCall {
    pub(crate) fn require(&self, db: &Connection, lease: &CoordinatorLease, child: &ScheduledChild) -> Result<(), String> {
        let mut scope = lease.clone(); scope.lease_expires_at.clear();
        let mut original = self.call.lease.clone(); original.lease_expires_at.clear();
        if scope != original || *child != self.call.child {
            return Err("proposal_unsent_original_scope_conflict".into());
        }
        let saved = not_sent_for_owned_proposal(db, lease, child, &self.proof)?.ok_or("proposal_unsent_receipt_missing")?;
        if saved.call.request_hash != self.call.request_hash { return Err("proposal_unsent_original_request_conflict".into()); }
        Ok(())
    }
}
