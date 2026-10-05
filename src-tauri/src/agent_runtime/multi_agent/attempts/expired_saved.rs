//! Local publication proof for an expired original worker. It issues no lease.
use super::*;
use rusqlite::types::Value;

// Historical Source reads are not local publication authority. Check a stored
// expired close without requiring a live Root or renewing its Coordinator.
pub(crate) fn verify_expired_closed_audit(
    db: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<(), String> {
    let state: Option<String> = db
        .query_row(
            "SELECT state FROM agent_assignment_attempts WHERE child_run_id=?1",
            [&child.run_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("expired_closed_audit:{e}"))?;
    if state.as_deref() != Some("expired") {
        return Ok(());
    }
    let worker = current(db, lease, &child.assignment_id)?;
    let valid: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_assignment_attempts x
         JOIN agent_assignments a ON a.id=x.assignment_id AND a.child_run_id=x.child_run_id
         JOIN agent_runs r ON r.id=x.child_run_id AND r.assignment_id=a.id
         WHERE x.id=?1 AND x.child_run_id=?2 AND x.state='expired' AND x.failure_class='worker_lease_expired'
           AND datetime(x.expires_at)<=datetime(x.finished_at)
           AND datetime(x.finished_at)<=datetime('now','localtime')
           AND a.role=?3 AND r.role=a.role AND r.lane=a.lane
           AND a.state='completed' AND a.budget_settled_at<>'' AND a.reserved_tokens=0
           AND a.reserved_requests=0 AND a.finished_at<>''
           AND r.status='terminal' AND r.terminal_state='completed' AND r.finished_at<>'')
         AND NOT EXISTS(SELECT 1 FROM agent_capability_leases
           WHERE (assignment_id=?4 OR child_run_id=?2) AND revoked_at='')
         AND NOT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=?4)",
        params![worker.id,child.run_id,child.role.as_str(),child.assignment_id], |row| row.get(0),
    ).map_err(|e| format!("expired_closed_audit:{e}"))?;
    if !valid || db.is_autocommit() {
        return Err("expired_closed_audit_invalid".into());
    }
    Ok(())
}

pub(crate) struct ExpiredSavedProof {
    worker: AssignmentAttempt,
    rows: Vec<Vec<Value>>,
}

impl ExpiredSavedProof {
    pub(crate) fn capture(
        db: &Connection,
        lease: &CoordinatorLease,
        child: &ScheduledChild,
    ) -> Result<Option<Self>, String> {
        if db.is_autocommit() {
            return Err("expired_saved_transaction_required".into());
        }
        let worker = current(db, lease, &child.assignment_id)?;
        if worker.state != "expired" {
            return Ok(None);
        }
        Self::validate(db, lease, child, &worker)?;
        let rows = Self::read_rows(db, &worker)?;
        Ok(Some(Self { worker, rows }))
    }

    pub(crate) fn verify(
        &self,
        db: &Connection,
        lease: &CoordinatorLease,
        child: &ScheduledChild,
    ) -> Result<(), String> {
        let worker = current(db, lease, &child.assignment_id)?;
        if worker != self.worker || Self::read_rows(db, &worker)? != self.rows {
            return Err("expired_saved_original_proof_changed".into());
        }
        Self::validate(db, lease, child, &worker)
    }

    fn validate(
        db: &Connection,
        lease: &CoordinatorLease,
        child: &ScheduledChild,
        worker: &AssignmentAttempt,
    ) -> Result<(), String> {
        super::super::lease::validate_coordinator_lease(db, lease)?;
        super::super::lease::require_executable_coordinator(db, lease)?;
        if worker.child_run_id != child.run_id
            || worker.state != "expired"
            || worker.failure_class != "worker_lease_expired"
        {
            return Err("expired_saved_worker_binding_invalid".into());
        }
        let eligible: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_assignment_attempts x
             JOIN agent_assignments a ON a.id=x.assignment_id AND a.child_run_id=x.child_run_id
             JOIN agent_runs r ON r.id=x.child_run_id AND r.assignment_id=a.id
             WHERE x.id=?1 AND a.coordinator_run_id=?2 AND r.root_run_id=?2 AND r.parent_run_id=?2
               AND a.target_key=?3 AND r.target_url=?3 AND r.scan_id=?4 AND r.attempt_number=?5
               AND a.role=?6 AND r.role=a.role AND r.lane=a.lane AND r.backend='native'
               AND a.lease_epoch=?7 AND a.fencing_token=?8 AND r.cancel_requested_at=''
               AND datetime(x.expires_at)<=datetime(x.finished_at)
               AND datetime(x.finished_at)<=datetime('now','localtime')
               AND ((a.state='paused' AND a.failure_class='worker_lease_expired'
                 AND a.budget_settled_at='' AND a.reserved_requests=1 AND a.finished_at=''
                 AND r.status='paused' AND r.terminal_state='' AND r.finished_at=''
                 AND EXISTS(SELECT 1 FROM agent_lane_leases l WHERE l.assignment_id=a.id
                   AND l.scan_id=r.scan_id AND l.attempt_number=r.attempt_number
                   AND l.target_key=r.target_url AND l.lane=a.lane))
               OR (a.state='completed' AND a.failure_class='' AND a.budget_settled_at<>''
                 AND a.reserved_tokens=0 AND a.reserved_requests=0 AND a.finished_at<>''
                 AND r.status='terminal' AND r.terminal_state='completed' AND r.finished_at<>''
                 AND NOT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=a.id))))
             AND NOT EXISTS(SELECT 1 FROM agent_capability_leases
               WHERE (assignment_id=?9 OR child_run_id=?10) AND revoked_at='')",
                params![
                    worker.id,
                    lease.root_run_id,
                    lease.target_key,
                    lease.scan_id,
                    lease.attempt_number,
                    child.role.as_str(),
                    lease.lease_epoch,
                    lease.fencing_token,
                    child.assignment_id,
                    child.run_id
                ],
                |row| row.get(0),
            )
            .map_err(|e| format!("expired_saved_state:{e}"))?;
        if !eligible {
            return Err("expired_saved_state_invalid".into());
        }
        let receipt = super::super::specialist::received_for_local_delivery(db, lease, child)?;
        if !receipt.rejection.is_empty() {
            return Err("expired_saved_received_result_required".into());
        }
        Ok(())
    }

    fn read_rows(db: &Connection, worker: &AssignmentAttempt) -> Result<Vec<Vec<Value>>, String> {
        // Every worker and call column is immutable. Logical rows may change
        // only the explicit local settlement/termination fields below.
        [
            ("agent_assignment_attempts", "id", &worker.id, &[][..]),
            (
                "agent_specialist_calls",
                "assignment_id",
                &worker.assignment_id,
                &[][..],
            ),
            (
                "agent_assignments",
                "id",
                &worker.assignment_id,
                &[
                    "state",
                    "failure_class",
                    "budget_settled_at",
                    "reserved_tokens",
                    "reserved_requests",
                    "finished_at",
                    "updated_at",
                ][..],
            ),
            (
                "agent_runs",
                "id",
                &worker.child_run_id,
                &[
                    "status",
                    "terminal_state",
                    "terminal_code",
                    "terminal_reason",
                    "used_tokens",
                    "used_cached_tokens",
                    "used_requests",
                    "finished_at",
                    "updated_at",
                ][..],
            ),
        ]
        .into_iter()
        .map(|(table, key, id, mutable)| {
            let mut statement = db
                .prepare(&format!("SELECT * FROM {table} WHERE {key}=?1"))
                .map_err(|e| format!("expired_saved_read:{e}"))?;
            let columns: Vec<usize> = statement
                .column_names()
                .iter()
                .enumerate()
                .filter_map(|(i, name)| (!mutable.contains(name)).then_some(i))
                .collect();
            statement
                .query_row([id], |row| columns.iter().map(|i| row.get(*i)).collect())
                .map_err(|e| format!("expired_saved_read:{e}"))
        })
        .collect()
    }
}
