// A publication checkpoint, never an executable worker grant. The same frozen
// rows survive from the original close until the last business write in its TX.
struct ReviewClosedAttemptProof {
    worker: crate::agent_runtime::multi_agent::attempts::AssignmentAttempt,
    rows: Vec<Vec<rusqlite::types::Value>>,
}

impl ReviewClosedAttemptProof {
    fn capture(
        tx: &rusqlite::Transaction<'_>,
        lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
        reviewer: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    ) -> Result<Self, String> {
        let worker = crate::agent_runtime::multi_agent::attempts::current(
            tx,
            lease,
            &reviewer.assignment_id,
        )?;
        if tx.is_autocommit()
            || reviewer.role != crate::agent_runtime::contract::AgentRole::EvidenceReviewer
            || worker.child_run_id != reviewer.run_id
            || !matches!(worker.state.as_str(), "completed" | "failed" | "expired")
            || worker.finished_at.is_empty()
            || !Self::is_closed(tx, lease, reviewer)?
        {
            return Err("review_delivery_attempt_not_closed".into());
        }
        let rows = Self::read_rows(tx, &worker)?;
        Ok(Self { worker, rows })
    }

    fn verify(
        &self,
        tx: &rusqlite::Transaction<'_>,
        lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
        reviewer: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    ) -> Result<(), String> {
        let actual = crate::agent_runtime::multi_agent::attempts::current(
            tx,
            lease,
            &reviewer.assignment_id,
        )?;
        // Do not replace the captured proof with a newly accepted close. This
        // checks all persisted worker, logical assignment and original run fields.
        if actual != self.worker
            || Self::read_rows(tx, &self.worker)? != self.rows
            || !Self::is_closed(tx, lease, reviewer)?
        {
            return Err("review_delivery_attempt_closure_changed".into());
        }
        Ok(())
    }

    fn read_rows(
        tx: &rusqlite::Transaction<'_>,
        worker: &crate::agent_runtime::multi_agent::attempts::AssignmentAttempt,
    ) -> Result<Vec<Vec<rusqlite::types::Value>>, String> {
        [
            ("agent_assignment_attempts", &worker.id),
            ("agent_assignments", &worker.assignment_id),
            ("agent_runs", &worker.child_run_id),
        ]
        .into_iter()
        .map(|(table, id)| {
            let mut statement = tx
                .prepare(&format!("SELECT * FROM {table} WHERE id=?1"))
                .map_err(|error| format!("review_closed_attempt_read:{error}"))?;
            let columns = statement.column_count();
            statement
                .query_row([id], |row| {
                    (0..columns)
                        .map(|column| row.get(column))
                        .collect::<rusqlite::Result<Vec<rusqlite::types::Value>>>()
                })
                .map_err(|error| format!("review_closed_attempt_read:{error}"))
        })
        .collect()
    }

    fn is_closed(
        tx: &rusqlite::Transaction<'_>,
        lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
        reviewer: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    ) -> Result<bool, String> {
        tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
             WHERE a.id=?1 AND r.id=?2 AND r.assignment_id=a.id AND a.coordinator_run_id=?3
               AND r.root_run_id=?3 AND r.parent_run_id=?3
               AND a.role='evidence_reviewer' AND r.role=a.role AND a.lane='review' AND r.lane=a.lane
               AND a.target_key=?4 AND r.target_url=?4 AND r.scan_id=?5 AND r.attempt_number=?6
               AND a.lease_epoch=?7 AND a.fencing_token=?8 AND a.state='completed'
               AND a.budget_settled_at<>'' AND a.reserved_tokens=0 AND a.reserved_requests=0
               AND a.finished_at<>'' AND r.status='terminal' AND r.terminal_state='completed' AND r.finished_at<>'')
             AND NOT EXISTS(SELECT 1 FROM agent_capability_leases
               WHERE (assignment_id=?1 OR child_run_id=?2) AND revoked_at='')
             AND NOT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=?1)",
            params![reviewer.assignment_id,reviewer.run_id,lease.root_run_id,lease.target_key,
                lease.scan_id,lease.attempt_number,lease.lease_epoch,lease.fencing_token],
            |row| row.get(0),
        ).map_err(|error| format!("review_closed_attempt_state:{error}"))
    }
}
