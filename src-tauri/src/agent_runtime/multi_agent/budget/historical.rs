//! Original cost facts are distinct from current executable authority.
//! This proof never adopts the current Coordinator or assignment child pointer.
use crate::agent_runtime::{
    contract::{AgentLane, AgentRole},
    multi_agent::{attempts::AssignmentAttempt, lease::CoordinatorLease},
};
use rusqlite::{params, Connection, OptionalExtension, Transaction};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CostOwner {
    pub lease: CoordinatorLease,
    pub assignment: String,
    pub attempt: AssignmentAttempt,
    role: String,
    lane: String,
}

impl CostOwner {
    pub(super) fn for_run(db: &Connection, run: &str) -> Result<Option<Self>, String> {
        let (policy, standalone): (String, bool) = db
            .query_row(
                "SELECT orchestration_policy,
            assignment_id='' AND parent_run_id IS NULL AND (root_run_id='' OR root_run_id=id)
            FROM agent_runs WHERE id=?1 AND backend='native'",
                [run],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| "budget_run_binding_missing")?;
        if policy == "single" && standalone {
            return Ok(None);
        }
        if policy != "multi" {
            return Err("budget_original_worker_binding_conflict".into());
        }
        let row = db.query_row("SELECT r.scan_id,r.attempt_number,r.target_url,
            x.id,x.root_run_id,x.assignment_id,x.child_run_id,x.coordinator_epoch,
            x.coordinator_fencing_token,x.lease_epoch,x.fencing_token,x.worker_id,
            x.state,x.expires_at,x.finished_at,x.failure_class,r.role,r.lane
            FROM agent_runs r JOIN agent_assignment_attempts x ON x.child_run_id=r.id
            JOIN agent_assignments a ON a.id=x.assignment_id AND a.coordinator_run_id=x.root_run_id
            JOIN agent_runs root ON root.id=x.root_run_id
            WHERE r.id=?1 AND r.assignment_id=x.assignment_id AND r.root_run_id=x.root_run_id
              AND r.parent_run_id=x.root_run_id AND r.target_url=a.target_key
              AND r.role=a.role AND r.lane=a.lane AND r.backend='native' AND r.orchestration_policy='multi'
              AND root.scan_id=r.scan_id AND root.attempt_number=r.attempt_number AND root.target_url=r.target_url
              AND root.backend='native' AND root.role='coordinator' AND root.orchestration_policy='multi'
              AND root.root_run_id=root.id AND root.assignment_id='' AND root.parent_run_id IS NULL
              AND root.status<>'legacy_backend_removed'", [run], |r| {
            let attempt = AssignmentAttempt {
                id:r.get(3)?, root_run_id:r.get(4)?, assignment_id:r.get(5)?, child_run_id:r.get(6)?,
                coordinator_epoch:r.get(7)?, coordinator_fencing_token:r.get(8)?, lease_epoch:r.get(9)?,
                fencing_token:r.get(10)?, worker_id:r.get(11)?, state:r.get(12)?, expires_at:r.get(13)?,
                finished_at:r.get(14)?, failure_class:r.get(15)?,
            };
            Ok(Self {
                lease:CoordinatorLease {scan_id:r.get(0)?, attempt_number:r.get(1)?, target_key:r.get(2)?,
                    root_run_id:attempt.root_run_id.clone(), lease_epoch:attempt.coordinator_epoch,
                    fencing_token:attempt.coordinator_fencing_token.clone(), lease_expires_at:attempt.expires_at.clone()},
                assignment:attempt.assignment_id.clone(), attempt, role:r.get(16)?, lane:r.get(17)?,
            })
        }).optional().map_err(|e|format!("budget_original_worker_read:{e}"))?
            .ok_or("budget_original_worker_binding_conflict")?;
        if row.attempt.coordinator_epoch <= 0
            || row.attempt.lease_epoch <= 0
            || row.lease.attempt_number <= 0
            || row.attempt.coordinator_fencing_token.trim().is_empty()
            || [
                &row.attempt.id,
                &row.attempt.worker_id,
                &row.attempt.fencing_token,
            ]
            .iter()
            .any(|v| uuid::Uuid::parse_str(v).is_err())
            || !matches!(
                row.attempt.state.as_str(),
                "leased" | "running" | "paused" | "completed" | "failed" | "cancelled" | "expired"
            )
            || AgentRole::try_parse(&row.role).is_none()
            || AgentLane::try_parse(&row.lane).is_none()
        {
            return Err("budget_original_worker_binding_conflict".into());
        }
        Ok(Some(row))
    }

    pub(super) fn verify(&self, db: &Connection) -> Result<(), String> {
        if Self::for_run(db, &self.attempt.child_run_id)?.as_ref() != Some(self) {
            return Err("budget_original_worker_binding_conflict".into());
        }
        Ok(())
    }

    pub(super) fn receive_target(&self, tx: &Transaction<'_>, source: &str) -> Result<(), String> {
        self.verify(tx)?;
        if source.trim().is_empty() {
            return Err("budget_entry_invalid".into());
        }
        let bound: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_budget_entries
            WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3
              AND dimension='target_requests' AND kind='forfeit' AND amount=1
              AND idempotency_key=?4 AND source_id=?5)",
                params![
                    self.lease.root_run_id,
                    self.assignment,
                    self.attempt.id,
                    super::scope::stored_key(
                        tx,
                        &self.lease.root_run_id,
                        &self.assignment,
                        &self.attempt.id,
                        &format!("target:{source}:dispatch")
                    )?,
                    source
                ],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !bound {
            return Err("budget_target_claim_missing".into());
        }
        super::entries::persist(
            tx,
            &self.lease.root_run_id,
            &self.assignment,
            self.attempt.id.clone(),
            "target_requests",
            super::Kind::Reconcile,
            1,
            &format!("target:{source}:receipt"),
            source,
        )?;
        self.verify(tx)
    }
}
