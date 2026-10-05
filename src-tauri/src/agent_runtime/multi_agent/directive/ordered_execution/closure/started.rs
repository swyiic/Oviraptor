//! Revoke only original readonly execution authority; all costs remain untouched.
use super::*;
use crate::agent_runtime::multi_agent::attempts::audit_rows::Rows;
use rusqlite::types::Value as SqlValue;
const REASON: &str = "ordered_task_ended_reconciliation_required";

pub(super) fn pause(tx: &rusqlite::Transaction<'_>, job: &ActionJob) -> Result<(), String> {
    // Original immutable dispatch proof if one exists; no fresh grant/live check.
    financial::metadata(tx, job)?;
    if job.state == "received" { financial::verify_projection(tx, job)?; }
    let worker = attempts::current(tx, &job.scope, &job.child.assignment_id)?;
    let bound: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
        WHERE a.id=?1 AND r.id=?2 AND a.coordinator_run_id=?3 AND r.root_run_id=?3 AND r.parent_run_id=?3
        AND a.state IN ('running','paused') AND r.status IN ('running','paused') AND a.budget_settled_at=''
        AND a.finished_at='' AND r.finished_at='' AND r.terminal_state='' AND a.lane='read_only_analysis' AND r.lane=a.lane
        AND a.lease_epoch=?4 AND a.fencing_token=?5)", params![job.child.assignment_id,job.child.run_id,job.scope.root_run_id,
        job.scope.lease_epoch,job.scope.fencing_token], |r|r.get(0)).map_err(|e|e.to_string())?;
    if !bound || !matches!(worker.state.as_str(), "running" | "paused") || !worker.finished_at.is_empty() {
        return Err("ordered_started_closure_original_worker_conflict".into());
    }
    let immutable = frozen(tx, job)?;
    let now: String = tx.query_row("SELECT datetime('now','localtime')", [], |r|r.get(0)).map_err(|e|e.to_string())?;
    let mut expected = runtime(tx, job)?;
    expected[0].set(&job.child.assignment_id, "state", "paused")?;
    expected[0].set(&job.child.assignment_id, "failure_class", REASON)?;
    expected[0].set(&job.child.assignment_id, "updated_at", &now)?;
    expected[1].set(&job.child.run_id, "status", "paused")?;
    expected[1].set(&job.child.run_id, "updated_at", &now)?;
    expected[2].set(&worker.id, "state", "paused")?;
    expected[2].set(&worker.id, "failure_class", REASON)?;
    let cap_ids = expected[3].values.iter().map(|row| match &row[expected[3].column("id")?] {
        SqlValue::Text(id) => Ok(id.clone()), _ => Err("ordered_started_closure_capability_conflict".to_string()),
    }).collect::<Result<Vec<_>,String>>()?;
    for id in cap_ids {
        if expected[3].text(&id, "root_run_id")? != job.scope.root_run_id
            || expected[3].text(&id, "assignment_id")? != job.child.assignment_id
            || expected[3].text(&id, "child_run_id")? != job.child.run_id
            || expected[3].text(&id, "fencing_token")? != job.scope.fencing_token
            || expected[3].row(&id)?[expected[3].column("lease_epoch")?] != SqlValue::Integer(job.scope.lease_epoch) {
            return Err("ordered_started_closure_capability_conflict".into());
        }
        if expected[3].text(&id, "revoked_at")?.is_empty() { expected[3].set(&id, "revoked_at", &now)?; }
    }
    let lane_count: i64 = tx.query_row("SELECT COUNT(*) FROM agent_lane_leases WHERE assignment_id=?1", [&job.child.assignment_id], |r|r.get(0)).map_err(|e|e.to_string())?;
    let a = tx.execute("UPDATE agent_assignments SET state='paused',failure_class=?4,updated_at=?5 WHERE id=?1 AND child_run_id=?2
        AND coordinator_run_id=?3 AND state IN ('running','paused') AND budget_settled_at=''", params![job.child.assignment_id,job.child.run_id,job.scope.root_run_id,REASON,now]).map_err(|e|e.to_string())?;
    let r = tx.execute("UPDATE agent_runs SET status='paused',updated_at=?3 WHERE id=?1 AND root_run_id=?2 AND status IN ('running','paused')",
        params![job.child.run_id,job.scope.root_run_id,now]).map_err(|e|e.to_string())?;
    attempts::pause(tx, &job.scope, &job.child, REASON)?;
    tx.execute("UPDATE agent_capability_leases SET revoked_at=?4 WHERE assignment_id=?1 AND child_run_id=?2 AND root_run_id=?3 AND revoked_at=''",
        params![job.child.assignment_id,job.child.run_id,job.scope.root_run_id,now]).map_err(|e|e.to_string())?;
    let deleted = tx.execute("DELETE FROM agent_lane_leases WHERE assignment_id=?1 AND scan_id=?2 AND attempt_number=?3 AND target_key=?4 AND lane='read_only_analysis'",
        params![job.child.assignment_id,job.scope.scan_id,job.scope.attempt_number,job.scope.target_key]).map_err(|e|e.to_string())?;
    let active: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_capability_leases WHERE (assignment_id=?1 OR child_run_id=?2) AND revoked_at='')
        OR EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=?1)", params![job.child.assignment_id,job.child.run_id], |r|r.get(0)).map_err(|e|e.to_string())?;
    if a != 1 || r != 1 || i64::try_from(deleted).map_err(|e|e.to_string())? != lane_count || active
        || runtime(tx, job)? != expected || frozen(tx, job)? != immutable {
        return Err("ordered_started_closure_postcondition_conflict".into());
    }
    financial::metadata(tx, job)?;
    Ok(())
}

fn runtime(db: &Connection, job: &ActionJob) -> Result<Vec<Rows>, String> {
    Ok(vec![
        Rows::read(db,"SELECT rowid,* FROM agent_assignments WHERE id=?1",[&job.child.assignment_id])?,
        Rows::read(db,"SELECT rowid,* FROM agent_runs WHERE id=?1",[&job.child.run_id])?,
        Rows::read(db,"SELECT rowid,* FROM agent_assignment_attempts WHERE assignment_id=?1 ORDER BY rowid",[&job.child.assignment_id])?,
        Rows::read(db,"SELECT rowid,* FROM agent_capability_leases WHERE assignment_id=?1 OR child_run_id=?2 ORDER BY rowid",params![job.child.assignment_id,job.child.run_id])?,
    ])
}
fn frozen(db: &Connection, job: &ActionJob) -> Result<Vec<Rows>, String> {
    let root = &job.scope.root_run_id;
    Ok(vec![
        Rows::read(db,"SELECT rowid,* FROM agent_budget_ledger WHERE root_run_id=?1",[root])?,
        Rows::read(db,"SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 ORDER BY rowid",[root])?,
        Rows::read(db,"SELECT rowid,* FROM agent_specialist_calls WHERE root_run_id=?1 ORDER BY rowid",[root])?,
        Rows::read(db,"SELECT rowid,* FROM agent_model_cost_facts WHERE root_run_id=?1 ORDER BY rowid",[root])?,
        Rows::read(db,"SELECT rowid,* FROM agent_messages WHERE root_run_id=?1 ORDER BY rowid",[root])?,
        Rows::read(db,"SELECT rowid,* FROM agent_directive_ordered_actions WHERE directive_id=?1 ORDER BY rowid",[&job.directive_id])?,
        Rows::read(db,"SELECT rowid,* FROM agent_directive_ordered_receipts WHERE directive_id=?1 ORDER BY rowid",[&job.directive_id])?,
    ])
}
