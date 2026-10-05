//! Delivery releases only the already paid original worker's remaining estimates.
use super::{
    proof::Proof,
    rows::{get, Table, Tables},
};
use crate::agent_runtime::multi_agent::{budget, lease::CoordinatorLease};
use rusqlite::Connection;
use std::collections::BTreeSet;
pub(super) struct Fees {
    reserves: [i64; 4],
    slot: bool,
}
const DIMS: [&str; 4] = [
    "model_input_tokens",
    "model_cached_tokens",
    "model_output_tokens",
    "model_requests",
];
impl Fees {
    pub(super) fn capture(db: &Connection, p: &Proof) -> Result<Self, String> {
        let mut reserves = [0; 4];
        for (i, (dim, n)) in DIMS.into_iter().zip(values(p)).enumerate() {
            let own = budget::balance_for_attempt(db, &p.root, &p.assignment, &p.worker, dim)?;
            if own.consumed != n
                || own.indeterminate != 0
                || own.reserved < 0
                || (p.replay && own.reserved != 0)
            {
                return Err("client_side_delivery_original_paid_cost_conflict".into());
            }
            reserves[i] = own.reserved;
        }
        let v2 = budget::root_definition::read(db, &p.root)?.is_some();
        let slot = budget::balance_for_attempt(
            db,
            &p.root,
            &p.assignment,
            &p.worker,
            "concurrency_batches",
        )?;
        if v2 && (slot.reserved != 0 || slot.consumed != 0 || slot.indeterminate != 0) {
            return Err("client_side_delivery_batch_slot_conflict".into());
        }
        if !v2 && slot.reserved != i64::from(!p.replay) {
            return Err("client_side_delivery_old_slot_conflict".into());
        }
        Ok(Self {
            reserves,
            slot: !v2 && !p.replay,
        })
    }
    pub(super) fn verify(
        &self,
        db: &Connection,
        p: &Proof,
        actual: &Tables,
        lease: &CoordinatorLease,
        end: &str,
    ) -> Result<(), String> {
        let t = get(actual, "agent_budget_entries")?;
        if t.columns.len() != 11
            || !t.columns.iter().all(|c| {
                matches!(
                    c.as_str(),
                    "rowid"
                        | "entry_id"
                        | "root_run_id"
                        | "assignment_id"
                        | "lease_attempt_id"
                        | "dimension"
                        | "kind"
                        | "amount"
                        | "idempotency_key"
                        | "source_id"
                        | "created_at"
                )
            })
        {
            return Err("client_side_delivery_cost_schema_conflict".into());
        }
        let rows = t.added(get(&p.tables, "agent_budget_entries")?)?;
        let mut seen = BTreeSet::new();
        let source = format!(
            "child:{}:usage:{}",
            p.assignment,
            crate::agent_runtime::store::stable_hash(&p.usage.as_json().to_string())
        );
        for row in &rows {
            exact(t, row, "root_run_id", &p.root)?;
            exact(t, row, "assignment_id", &p.assignment)?;
            exact(t, row, "lease_attempt_id", &p.worker)?;
            exact(t, row, "kind", "release")?;
            if uuid::Uuid::parse_str(t.text(row, "entry_id")?).is_err() {
                return Err("client_side_delivery_new_cost_conflict".into());
            }
            let dim = t.text(row, "dimension")?;
            let (amount, key, source) = if dim == "concurrency_batches" && self.slot {
                (
                    1,
                    format!("slot-finish:{}", p.assignment),
                    format!("terminal:{}", p.assignment),
                )
            } else {
                let i = DIMS
                    .iter()
                    .position(|d| *d == dim)
                    .ok_or("client_side_delivery_new_cost_conflict")?;
                (
                    self.reserves[i],
                    format!("settle:{}:{dim}:release", p.assignment),
                    source.clone(),
                )
            };
            let key = if p.ordinal == 1 {
                key
            } else {
                format!("worker:{}:{key}", p.worker)
            };
            exact(t, row, "source_id", &source)?;
            exact(t, row, "idempotency_key", &key)?;
            if amount <= 0 || t.number(row, "amount")? != amount || !seen.insert(dim) {
                return Err("client_side_delivery_new_cost_conflict".into());
            }
            let created = t.text(row, "created_at")?;
            if created < p.now.as_str() || created > end {
                return Err("client_side_delivery_cost_timestamp_conflict".into());
            }
        }
        let expected = self.reserves.iter().filter(|n| **n > 0).count() + usize::from(self.slot);
        if rows.len() != expected {
            return Err("client_side_delivery_cost_count_conflict".into());
        }
        for (dim, n) in DIMS.into_iter().zip(values(p)) {
            let own =
                budget::balance_for_attempt(db, &lease.root_run_id, &p.assignment, &p.worker, dim)?;
            if own.consumed != n || own.reserved != 0 || own.indeterminate != 0 {
                return Err("client_side_delivery_paid_cost_changed".into());
            }
        }
        Ok(())
    }
}
fn values(p: &Proof) -> [i64; 4] {
    [
        p.usage.input_tokens,
        p.usage.cached_input_tokens,
        p.usage.output_tokens,
        p.usage.model_requests,
    ]
}
fn exact(t: &Table, row: &[rusqlite::types::Value], col: &str, val: &str) -> Result<(), String> {
    if t.text(row, col)? == val {
        Ok(())
    } else {
        Err("client_side_delivery_new_cost_scope_conflict".into())
    }
}
