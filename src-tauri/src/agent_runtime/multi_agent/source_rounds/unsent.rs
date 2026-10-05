//! A typed pre-transport outcome saves one immutable fact, never a refund or retry.
use super::*;

pub(crate) fn record_not_sent(
    db: &Connection,
    call: &PendingRound,
    code: &str,
) -> Result<(), String> {
    if code != "user_cancelled" {
        return Err("source_round_no_send_reason_invalid".into());
    }
    let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let existing: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_model_cost_facts
         WHERE family='source-round' AND child_run_id=?1 AND round_number=?2)",
            params![call.child.run_id, call.number],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let writes = tx.total_changes();
    super::super::budget::model_facts::source_round(
        &tx,
        &call.lease,
        &call.child,
        call.number,
        &store::stable_hash(&call.request.to_string()),
        call.reserved_tokens,
        super::super::budget::model_facts::Fact::Unsent,
    )?;
    // There is exactly one allowed new row, or a completely read-only replay.
    // SQLite counts trigger/cascade writes too: collateral business, resource
    // or financial changes must roll back even if the original owner survives.
    if tx.total_changes().checked_sub(writes) != Some(u64::from(!existing)) {
        return Err("source_round_no_send_unexpected_write".into());
    }
    tx.commit().map_err(|e| e.to_string())
}

pub(super) fn require_no_terminal_fact(db: &Connection, call: &PendingRound) -> Result<(), String> {
    let phase: Option<String> = db
        .query_row(
            "SELECT phase FROM agent_model_cost_facts WHERE family='source-round'
         AND child_run_id=?1 AND round_number=?2",
            params![call.child.run_id, call.number],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    match phase.as_deref() {
        Some("unsent") => Err("source_round_model_not_sent_new_attempt_required".into()),
        Some(_) => Err("source_round_original_cost_fact_precludes_execution".into()),
        None => Ok(()),
    }
}

/// Read-only zero-cost proof for current Coordinator cleanup, including its
/// last-write check. A later round or any published result cannot claim zero.
pub(crate) fn audit_unsent_first(
    db: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<bool, String> {
    if db.is_autocommit() {
        return Err("source_no_send_audit_transaction_required".into());
    }
    if !source::is_source_role(child.role) {
        return Ok(false);
    }
    let bound: bool = db.query_row(&format!("SELECT EXISTS(SELECT 1 FROM agent_source_model_rounds c
        JOIN agent_assignment_attempts x ON x.child_run_id=c.child_run_id AND x.assignment_id=c.assignment_id AND x.root_run_id=c.root_run_id
        JOIN agent_runs r ON r.id=c.child_run_id JOIN agent_assignments a ON a.id=c.assignment_id AND a.child_run_id=r.id
        WHERE c.child_run_id=?1 AND c.assignment_id=?2 AND c.root_run_id=?3 AND c.role=?4 AND c.round_number=1
          AND c.lease_epoch=?5 AND c.fencing_token=?6 AND r.role=c.role AND a.role=c.role
          AND a.coordinator_run_id=c.root_run_id AND r.root_run_id=c.root_run_id AND r.parent_run_id=c.root_run_id
          AND r.scan_id=?7 AND r.attempt_number=?8 AND r.target_url=?9 AND a.target_key=?9
          AND r.backend='native' AND r.orchestration_policy='multi' AND r.assignment_id=a.id AND r.lane=a.lane
          AND r.used_tokens=0 AND r.used_cached_tokens=0 AND r.used_requests=0
          AND json_extract(a.task_slice_json,'$.phase')='source_tools'
          AND EXISTS(SELECT 1 FROM agent_model_cost_facts f WHERE {unsent})
          AND (SELECT count(*) FROM agent_source_model_rounds WHERE child_run_id=r.id)=1
          AND NOT EXISTS(SELECT 1 FROM agent_specialist_calls WHERE child_run_id=r.id)
          AND NOT EXISTS(SELECT 1 FROM agent_events WHERE run_id=r.id)
          AND NOT EXISTS(SELECT 1 FROM agent_snapshots WHERE run_id=r.id)
          AND NOT EXISTS(SELECT 1 FROM tool_invocations WHERE run_id=r.id)
          AND NOT EXISTS(SELECT 1 FROM agent_messages WHERE from_run_id=r.id OR run_id=r.id)
          AND NOT EXISTS(SELECT 1 FROM agent_evidence_nodes WHERE created_by_run_id=r.id)
          AND NOT EXISTS(SELECT 1 FROM agent_evidence_edges WHERE created_by_run_id=r.id)
          AND NOT EXISTS(SELECT 1 FROM agent_budget_entries WHERE lease_attempt_id=x.id
            AND dimension NOT IN ('wall_time_ms','concurrency_batches') AND kind IN ('consume','forfeit','reconcile')))",
        unsent=super::super::budget::model_facts::SOURCE_UNSENT_BINDING),
        params![child.run_id,child.assignment_id,lease.root_run_id,child.role.as_str(),lease.lease_epoch,lease.fencing_token,lease.scan_id,lease.attempt_number,lease.target_key],
        |r|r.get(0)).map_err(|e|format!("source_no_send_audit:{e}"))?;
    if bound {
        let seed = PendingRound {
            lease: lease.clone(),
            child: child.clone(),
            number: 1,
            request: Value::Null,
            reserved_tokens: 0,
        };
        let call = historical(db, &seed, 1)?;
        load(db, &call)?.ok_or("source_round_claim_missing")?;
    }
    Ok(bound)
}
