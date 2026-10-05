//! Absence of durable dispatch is stricter than absence of a response.
use super::*;

pub(super) fn verify(
    db: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
    released: bool,
) -> Result<(), String> {
    let worker:String=db.query_row("SELECT x.id FROM agent_assignment_attempts x JOIN agent_runs r ON r.id=x.child_run_id
        WHERE x.child_run_id=?1 AND x.assignment_id=?2 AND x.root_run_id=?3 AND x.coordinator_epoch=?4
          AND x.coordinator_fencing_token=?5 AND x.state='expired' AND x.failure_class='worker_lease_expired'
          AND datetime(x.expires_at)<=datetime(x.finished_at) AND datetime(x.finished_at)<=datetime('now','localtime')
          AND r.assignment_id=?2 AND r.root_run_id=?3 AND r.parent_run_id=?3 AND r.scan_id=?6 AND r.attempt_number=?7
          AND r.target_url=?8 AND r.role=?9 AND r.lane='read_only_analysis' AND r.backend='native' AND r.orchestration_policy='multi'
          AND r.status='paused' AND r.finished_at='' AND r.terminal_state='' AND r.cancel_requested_at=''
          AND r.used_tokens=0 AND r.used_cached_tokens=0 AND r.used_requests=0",
        params![child.run_id,child.assignment_id,lease.root_run_id,lease.lease_epoch,lease.fencing_token,lease.scan_id,lease.attempt_number,lease.target_key,child.role.as_str()],|r|r.get(0))
        .map_err(|e|format!("assignment_replacement_original_audit:{e}"))?;
    let effect:bool=db.query_row("SELECT
        EXISTS(SELECT 1 FROM agent_specialist_calls WHERE child_run_id=?1 OR (assignment_id=?2 AND child_run_id=?1))
        OR EXISTS(SELECT 1 FROM agent_source_model_rounds WHERE child_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_source_tool_receipts WHERE child_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_web_model_journal WHERE child_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_model_cost_facts WHERE child_run_id=?1 OR lease_attempt_id=?3)
        OR EXISTS(SELECT 1 FROM tool_invocations WHERE run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_http_request_claims WHERE run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_authorization_probe_claims WHERE child_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_external_surface_captures WHERE child_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_events WHERE run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_snapshots WHERE run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_messages WHERE run_id=?1 OR from_run_id=?1 OR to_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_evidence_nodes WHERE created_by_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_evidence_edges WHERE created_by_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_capability_leases WHERE child_run_id=?1 AND revoked_at='')",
        params![child.run_id,child.assignment_id,worker],|r|r.get(0)).map_err(|e|e.to_string())?;
    if effect {
        return Err("assignment_replacement_dispatch_or_effect_requires_recovery".into());
    }
    let (tokens, requests): (i64, i64) = db
        .query_row(
            "SELECT reserved_tokens,reserved_requests FROM agent_runs WHERE id=?1",
            [&child.run_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    let slot =
        i64::from(super::super::budget::root_definition::read(db, &lease.root_run_id)?.is_none());
    for (dimension, amount) in super::super::budget::DIMENSIONS
        .iter()
        .zip([tokens, tokens, tokens, requests, 0, 0, 0, 0, slot, 0])
    {
        let balance = super::super::budget::balance_for_attempt(
            db,
            &lease.root_run_id,
            &child.assignment_id,
            &worker,
            dimension,
        )?;
        if *dimension == "wall_time_ms" {
            if balance.reserved != 0 || balance.indeterminate != 0 {
                return Err("assignment_replacement_original_budget_conflict".into());
            }
        } else if balance.reserved != if released { 0 } else { amount }
            || balance.consumed != 0
            || balance.indeterminate != 0
        {
            return Err("assignment_replacement_original_budget_conflict".into());
        }
    }
    Ok(())
}
