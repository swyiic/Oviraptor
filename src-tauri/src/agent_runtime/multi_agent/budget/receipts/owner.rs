//! Financial owner proof. Original receipts never create execution authority.
use super::super::{entries, historical::CostOwner, Balance, Kind};
use crate::agent_runtime::multi_agent::lease::CoordinatorLease;
use rusqlite::{params, Connection, Transaction};

pub(super) struct ReceiptOwner {
    pub(super) lease: CoordinatorLease,
    pub(super) assignment: String,
    pub attempt: String,
    original: Option<CostOwner>,
}

impl ReceiptOwner {
    pub fn current(
        db: &Connection,
        lease: &CoordinatorLease,
        assignment: &str,
    ) -> Result<Self, String> {
        Ok(Self {
            lease: lease.clone(),
            assignment: assignment.into(),
            attempt: super::super::super::attempts::current(db, lease, assignment)?.id,
            original: None,
        })
    }

    pub fn original(
        db: &Connection,
        run: &str,
        lease: &CoordinatorLease,
        assignment: &str,
    ) -> Result<Self, String> {
        let proof =
            CostOwner::for_run(db, run)?.ok_or("budget_original_worker_binding_conflict")?;
        if proof.lease.root_run_id != lease.root_run_id
            || proof.lease.scan_id != lease.scan_id
            || proof.lease.attempt_number != lease.attempt_number
            || proof.lease.target_key != lease.target_key
            || proof.lease.lease_epoch != lease.lease_epoch
            || proof.lease.fencing_token != lease.fencing_token
            || proof.assignment != assignment
        {
            return Err("budget_original_worker_binding_conflict".into());
        }
        Ok(Self {
            lease: lease.clone(),
            assignment: assignment.into(),
            attempt: proof.attempt.id.clone(),
            original: Some(proof),
        })
    }

    pub fn verify(&self, db: &Connection) -> Result<(), String> {
        if let Some(proof) = &self.original {
            proof.verify(db)
        } else if super::super::super::attempts::current(db, &self.lease, &self.assignment)?.id
            == self.attempt
        {
            Ok(())
        } else {
            Err("budget_attempt_scope_conflict".into())
        }
    }

    pub fn balance(&self, db: &Connection, dimension: &str) -> Result<Balance, String> {
        entries::balance_for_attempt(
            db,
            &self.lease.root_run_id,
            &self.assignment,
            &self.attempt,
            dimension,
        )
    }

    pub fn initial_tokens(&self, db: &Connection) -> Result<i64, String> {
        if self.original.is_none() {
            return db
                .query_row(
                    "SELECT reserved_tokens FROM agent_assignments WHERE id=?1",
                    [&self.assignment],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string());
        }
        let key = super::super::scope::stored_key(
            db,
            &self.lease.root_run_id,
            &self.assignment,
            &self.attempt,
            &format!("reserve:{}:model_input_tokens", self.assignment),
        )?;
        db.query_row("SELECT amount FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2
            AND lease_attempt_id=?3 AND dimension='model_input_tokens' AND kind='reserve' AND idempotency_key=?4 AND source_id=?5",
            params![self.lease.root_run_id,self.assignment,self.attempt,key,format!("assignment:{}",self.assignment)],
            |r|r.get(0)).map_err(|_|"budget_original_model_reservation_missing".into())
    }

    pub fn append(
        &self,
        tx: &Transaction<'_>,
        dimension: &str,
        kind: Kind,
        amount: i64,
        key: &str,
        source: &str,
    ) -> Result<(), String> {
        self.verify(tx)?;
        if let Some(proof) = &self.original {
            super::super::validate_received_transition(tx, &self.lease, dimension, kind)?;
            if amount <= 0
                || !super::super::DIMENSIONS[..4].contains(&dimension)
                || key.trim().is_empty()
                || source.trim().is_empty()
            {
                return Err("budget_entry_invalid".into());
            }
            let write = if kind == Kind::Reserve {
                entries::persist_known_cost
            } else {
                entries::persist
            };
            write(
                tx,
                &proof.lease.root_run_id,
                &self.assignment,
                self.attempt.clone(),
                dimension,
                kind,
                amount,
                key,
                source,
            )?;
        } else {
            super::super::append_received_cost(
                tx,
                &self.lease,
                &self.assignment,
                dimension,
                kind,
                amount,
                key,
                source,
            )?;
        }
        self.verify(tx)
    }
}
