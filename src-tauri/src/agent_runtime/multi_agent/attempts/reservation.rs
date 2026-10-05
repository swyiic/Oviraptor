use super::*;

// Initial reservation shares the transaction that issues the first grant and
// creates its child. Later reservations require that exact live child worker.
pub(crate) fn require_reservation(
    db: &Connection,
    lease: &CoordinatorLease,
    assignment: &str,
) -> Result<(), String> {
    super::super::lease::require_executable_coordinator(db, lease)?;
    let original = current(db, lease, assignment)?;
    let initial:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a
        JOIN agent_assignment_attempts x ON x.assignment_id=a.id AND x.child_run_id=a.child_run_id
        WHERE a.id=?1 AND x.id=?2 AND a.started_at='' AND a.finished_at=''
          AND ((a.state='prepared' AND a.lease_epoch=0 AND a.fencing_token='')
            OR (a.state='leased' AND a.lease_epoch=x.coordinator_epoch AND a.fencing_token=x.coordinator_fencing_token))
          AND x.state='leased' AND datetime(x.expires_at)>datetime('now','localtime')
          AND NOT EXISTS(SELECT 1 FROM agent_runs WHERE id=x.child_run_id))",
        params![assignment,original.id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !initial {
        require_live_for_run(db, &original.child_run_id)?;
    }
    Ok(())
}
