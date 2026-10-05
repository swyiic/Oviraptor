use super::*;

pub(crate) fn start(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<(), String> {
    let original = current(tx, lease, &child.assignment_id)?;
    if original.child_run_id != child.run_id {
        return Err("assignment_attempt_child_conflict".into());
    }
    let n=tx.execute("UPDATE agent_assignment_attempts SET state='running',heartbeat_at=datetime('now','localtime')
        WHERE id=?1 AND worker_id=?2 AND fencing_token=?3 AND state='leased' AND datetime(expires_at)>datetime('now','localtime')",
        params![original.id,original.worker_id,original.fencing_token]).map_err(|e|e.to_string())?;
    let actual = current(tx, lease, &child.assignment_id)?;
    let mut expected = original;
    expected.state = "running".into();
    if n != 1 || actual != expected {
        return Err("assignment_attempt_start_unconfirmed".into());
    }
    Ok(())
}

pub(crate) fn finish(
    db: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
    state: &str,
) -> Result<(), String> {
    if db.is_autocommit() || !matches!(state, "completed" | "failed" | "cancelled") {
        return Err("assignment_attempt_finish_context_invalid".into());
    }
    let original = current(db, lease, &child.assignment_id)?;
    if original.child_run_id != child.run_id {
        return Err("assignment_attempt_child_conflict".into());
    }
    let (finished, failure): (String, String) = db
        .query_row(
            "SELECT datetime('now','localtime'),failure_class
        FROM agent_assignments WHERE id=?1 AND child_run_id=?2 AND state=?3",
            params![child.assignment_id, child.run_id, state],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    let n=db.execute("UPDATE agent_assignment_attempts SET state=?4,finished_at=?5,failure_class=?6
        WHERE id=?1 AND worker_id=?2 AND fencing_token=?3 AND state IN ('leased','running','paused')",
        params![original.id,original.worker_id,original.fencing_token,state,finished,failure]).map_err(|e|e.to_string())?;
    let actual = current(db, lease, &child.assignment_id)?;
    let mut expected = original;
    expected.state = state.into();
    expected.finished_at = finished;
    expected.failure_class = failure;
    if n != 1 || actual != expected {
        return Err("assignment_attempt_finish_unconfirmed".into());
    }
    Ok(())
}

pub(crate) fn pause(
    db: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
    failure: &str,
) -> Result<(), String> {
    if db.is_autocommit() || failure.is_empty() {
        return Err("assignment_attempt_pause_context_invalid".into());
    }
    let original = current(db, lease, &child.assignment_id)?;
    if original.child_run_id != child.run_id {
        return Err("assignment_attempt_child_conflict".into());
    }
    let n=db.execute("UPDATE agent_assignment_attempts SET state='paused',failure_class=?4
        WHERE id=?1 AND worker_id=?2 AND fencing_token=?3 AND state IN ('running','paused') AND finished_at=''
          AND EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
            WHERE a.id=?5 AND r.id=?6 AND a.state='paused' AND r.status='paused' AND a.budget_settled_at='')",
        params![original.id,original.worker_id,original.fencing_token,failure,child.assignment_id,child.run_id]).map_err(|e|e.to_string())?;
    let actual = current(db, lease, &child.assignment_id)?;
    let mut expected = original;
    expected.state = "paused".into();
    expected.failure_class = failure.into();
    if n != 1 || actual != expected {
        return Err("assignment_attempt_pause_unconfirmed".into());
    }
    Ok(())
}

pub(crate) fn renew(
    db: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
    expiry: &str,
) -> Result<(), String> {
    if db.is_autocommit() {
        return Err("assignment_attempt_renew_transaction_required".into());
    }
    let original = current(db, lease, &child.assignment_id)?;
    if original.child_run_id != child.run_id {
        return Err("assignment_attempt_child_conflict".into());
    }
    let n=db.execute("UPDATE agent_assignment_attempts SET expires_at=?4,heartbeat_at=datetime('now','localtime')
        WHERE id=?1 AND worker_id=?2 AND fencing_token=?3 AND state IN ('leased','running')
          AND datetime(expires_at)>datetime('now','localtime')
          AND EXISTS(SELECT 1 FROM agent_coordinator_leases WHERE root_run_id=?5 AND lease_epoch=?6
            AND fencing_token=?7 AND lease_expires_at=?4 AND lease_expires_at>datetime('now','localtime'))",
        params![original.id,original.worker_id,original.fencing_token,expiry,lease.root_run_id,lease.lease_epoch,lease.fencing_token])
        .map_err(|e|e.to_string())?;
    let actual = current(db, lease, &child.assignment_id)?;
    let mut expected = original;
    expected.expires_at = expiry.into();
    if n != 1 || actual != expected {
        return Err("assignment_attempt_renew_unconfirmed".into());
    }
    Ok(())
}
