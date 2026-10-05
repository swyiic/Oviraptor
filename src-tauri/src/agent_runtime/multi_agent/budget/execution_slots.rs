//! New explicit Roots use the existing one-owner-per-lane execution lease.
//! Capacity is independent of actual race-batch budget; no fake consume/release.
use crate::agent_runtime::multi_agent::{
    attempts,
    lease::{self, CoordinatorLease},
};
use rusqlite::{params, Transaction};

pub(super) fn require(
    tx: &Transaction<'_>,
    actor: &CoordinatorLease,
    assignment: &str,
    reserve: bool,
    terminal_receipt: bool,
) -> Result<(), String> {
    let declaration = super::root_definition::read(tx, &actor.root_run_id)?
        .ok_or("execution_slot_declaration_missing")?;
    super::limits::verify_root_contract(tx, &actor.root_run_id)?;
    if terminal_receipt {
        let original: bool=tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_coordinator_leases c JOIN agent_runs r ON r.id=c.root_run_id
             WHERE r.id=?1 AND r.status='terminal' AND r.backend='native' AND r.role='coordinator'
             AND r.scan_id=?2 AND r.attempt_number=?3 AND r.target_url=?4
             AND c.scan_id=r.scan_id AND c.attempt_number=r.attempt_number AND c.target_key=r.target_url
             AND c.lease_epoch=?5 AND c.fencing_token=?6)",
            params![actor.root_run_id,actor.scan_id,actor.attempt_number,actor.target_key,
                actor.lease_epoch,actor.fencing_token],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !original {
            return Err("execution_slot_terminal_owner_conflict".into());
        }
    } else {
        lease::validate_coordinator_lease(tx, actor)?;
    }
    let worker = attempts::current(tx, actor, assignment)?;
    if reserve {
        attempts::require_reservation(tx, actor, assignment)?;
    }
    let bound: bool=tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_lane_leases l JOIN agent_assignments a ON a.id=l.assignment_id
         JOIN agent_runs r ON r.id=a.coordinator_run_id
         WHERE l.assignment_id=?1 AND r.id=?2 AND a.child_run_id=?3 AND a.target_key=?4
         AND l.scan_id=?5 AND l.attempt_number=?6 AND l.target_key=a.target_key AND l.lane=a.lane
         AND r.scan_id=l.scan_id AND r.attempt_number=l.attempt_number AND r.target_url=l.target_key
         AND l.lane IN ('target_touching','read_only_analysis','review'))",
        params![assignment,actor.root_run_id,worker.child_run_id,actor.target_key,
            actor.scan_id,actor.attempt_number],|r|r.get(0)).map_err(|e|e.to_string())?;
    let occupied: i64=tx.query_row(
        "SELECT count(*) FROM agent_lane_leases WHERE scan_id=?1 AND attempt_number=?2 AND target_key=?3",
        params![actor.scan_id,actor.attempt_number,actor.target_key],|r|r.get(0)).map_err(|e|e.to_string())?;
    // A present v2 declaration can never adopt old synthetic slot accounting.
    let old_slot: bool=tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_budget_entries WHERE root_run_id=?1
         AND dimension='concurrency_batches' AND
         (source_id LIKE 'assignment:%' OR source_id LIKE 'terminal:%'
           OR idempotency_key LIKE 'slot:%' OR idempotency_key LIKE 'slot-finish:%'
           OR idempotency_key LIKE 'worker:%:slot:%' OR idempotency_key LIKE 'worker:%:slot-finish:%'))",
        [&actor.root_run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !bound
        || occupied < 1
        || occupied > i64::from(declaration.execution_slot_capacity)
        || old_slot
    {
        return Err("execution_slot_binding_conflict".into());
    }
    Ok(())
}
