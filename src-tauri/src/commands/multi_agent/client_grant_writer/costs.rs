//! Only original new-worker reserves and the one shared Root-clock delta.
use super::rows::Table;
use crate::agent_runtime::{
    multi_agent::{budget, lease::CoordinatorLease},
    store,
};
use rusqlite::{types::Value, Connection};
use std::collections::BTreeSet;
pub(super) struct Costs {
    origin: String,
    elapsed: i64,
    consumed: i64,
    v2: bool,
}
impl Costs {
    pub(super) fn capture(db: &Connection, lease: &CoordinatorLease) -> Result<Self, String> {
        let (origin, elapsed) = clock(db, &lease.root_run_id)?;
        let before = budget::balance(db, &lease.root_run_id, None, "wall_time_ms")?;
        if before.reserved != 0 || before.indeterminate != 0 || elapsed < before.consumed {
            return Err("client_side_grant_clock_conflict".into());
        }
        let v2 = budget::root_definition::read(db, &lease.root_run_id)?.is_some();
        Ok(Self {
            origin,
            elapsed,
            consumed: before.consumed,
            v2,
        })
    }
    pub(super) fn verify(
        &self,
        db: &Connection,
        t: &Table,
        rows: &[&[Value]],
        lease: &CoordinatorLease,
        assignment: &str,
        worker: &str,
    ) -> Result<(), String> {
        let (origin, end) = clock(db, &lease.root_run_id)?;
        if self.origin != origin || end < self.elapsed {
            return Err("client_side_grant_clock_conflict".into());
        }
        let source = format!("assignment:{assignment}");
        let mut reserves = BTreeSet::new();
        let mut clock_rows = Vec::new();
        let mut slots = 0;
        for row in rows {
            t.only(row, "root_run_id", &lease.root_run_id)?;
            t.only(row, "assignment_id", assignment)?;
            t.only(row, "lease_attempt_id", worker)?;
            if uuid::Uuid::parse_str(t.text(row, "entry_id")?).is_err() {
                return Err("client_side_grant_new_cost_conflict".into());
            }
            let dimension = t.text(row, "dimension")?;
            match dimension {
                "model_input_tokens"
                | "model_cached_tokens"
                | "model_output_tokens"
                | "model_requests" => {
                    t.only(row, "kind", "reserve")?;
                    t.only(row, "source_id", &source)?;
                    t.only(
                        row,
                        "idempotency_key",
                        &format!("reserve:{assignment}:{dimension}"),
                    )?;
                    let expected = if dimension == "model_requests" {
                        1
                    } else {
                        8000
                    };
                    if t.number(row, "amount")? != expected || !reserves.insert(dimension) {
                        return Err("client_side_grant_new_cost_conflict".into());
                    }
                }
                "concurrency_batches" => {
                    slots += 1;
                    t.only(row, "kind", "reserve")?;
                    t.only(row, "source_id", &source)?;
                    t.only(row, "idempotency_key", &format!("slot:{assignment}"))?;
                    if self.v2 || t.number(row, "amount")? != 1 {
                        return Err("client_side_grant_new_cost_conflict".into());
                    }
                }
                "wall_time_ms" => clock_rows.push(*row),
                _ => return Err("client_side_grant_new_cost_conflict".into()),
            }
        }
        if reserves.len() != 4 || slots != usize::from(!self.v2) {
            return Err("client_side_grant_new_cost_conflict".into());
        }
        if clock_rows.is_empty() {
            return if self.consumed >= self.elapsed && self.consumed <= end {
                Ok(())
            } else {
                Err("client_side_grant_clock_conflict".into())
            };
        }
        if clock_rows.len() != 2 {
            return Err("client_side_grant_clock_conflict".into());
        }
        let prefix = format!("clock:{}:", store::stable_hash(&self.origin));
        let sample = t.text(clock_rows[0], "source_id")?;
        let elapsed = sample
            .strip_prefix(&prefix)
            .and_then(|s| s.parse::<i64>().ok())
            .ok_or("client_side_grant_clock_conflict")?;
        let amount = elapsed
            .checked_sub(self.consumed)
            .filter(|v| *v > 0)
            .ok_or("client_side_grant_clock_conflict")?;
        let mut kinds = BTreeSet::new();
        for row in clock_rows {
            let kind = t.text(row, "kind")?;
            if !matches!(kind, "reserve" | "consume")
                || !kinds.insert(kind)
                || t.text(row, "source_id")? != format!("{prefix}{elapsed}")
                || t.text(row, "idempotency_key")? != format!("{prefix}{elapsed}:{kind}")
                || t.number(row, "amount")? != amount
                || elapsed < self.elapsed
                || elapsed > end
            {
                return Err("client_side_grant_clock_conflict".into());
            }
        }
        Ok(())
    }
}
fn clock(db: &Connection, root: &str) -> Result<(String, i64), String> {
    let (origin,current,elapsed):(String,String,Option<i64>)=db.query_row(
        "SELECT o.started_at,r.created_at,CAST((julianday('now','localtime')-julianday(o.started_at))*86400000 AS INTEGER)
         FROM agent_budget_clock_origins o JOIN agent_runs r ON r.id=o.root_run_id WHERE r.id=?1",
        [root],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|"client_side_grant_clock_conflict")?;
    if origin != current {
        return Err("client_side_grant_clock_conflict".into());
    }
    Ok((
        origin,
        elapsed
            .filter(|v| *v >= 0)
            .ok_or("client_side_grant_clock_conflict")?,
    ))
}
