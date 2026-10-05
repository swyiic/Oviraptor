//! Preserve the original Root clock and verify only its real admission delta.
use super::rows::Table;
use crate::agent_runtime::{
    multi_agent::{budget, lease::CoordinatorLease},
    store,
};
use rusqlite::{types::Value, Connection};
use std::collections::BTreeSet;
pub(super) struct Clock {
    origin: String,
    elapsed: i64,
    consumed: i64,
}
#[derive(Clone, Copy)]
pub(super) struct Scope<'a> {
    pub(super) lease: &'a CoordinatorLease,
    pub(super) assignment: &'a str,
    pub(super) worker: &'a str,
    pub(super) ordinal: i64,
    pub(super) now: &'a str,
    pub(super) end: &'a str,
}
impl Clock {
    pub(super) fn capture(db: &Connection, lease: &CoordinatorLease) -> Result<Self, String> {
        let (origin, elapsed) = read(db, &lease.root_run_id)?;
        let before = budget::balance(db, &lease.root_run_id, None, "wall_time_ms")?;
        if before.reserved != 0 || before.indeterminate != 0 || elapsed < before.consumed {
            return Err("client_side_sdk_dispatch_clock_conflict".into());
        }
        Ok(Self {
            origin,
            elapsed,
            consumed: before.consumed,
        })
    }
    pub(super) fn verify(
        &self,
        db: &Connection,
        t: &Table,
        rows: &[&[Value]],
        s: &Scope<'_>,
    ) -> Result<(), String> {
        let Scope {
            lease,
            assignment,
            worker,
            ordinal,
            now,
            end,
        } = *s;
        let (origin, elapsed) = read(db, &lease.root_run_id)?;
        if origin != self.origin || elapsed < self.elapsed {
            return Err("client_side_sdk_dispatch_clock_conflict".into());
        }
        if rows.is_empty() {
            return if self.consumed >= self.elapsed && self.consumed <= elapsed {
                Ok(())
            } else {
                Err("client_side_sdk_dispatch_clock_conflict".into())
            };
        }
        if rows.len() != 2 {
            return Err("client_side_sdk_dispatch_new_cost_conflict".into());
        }
        let prefix = format!("clock:{}:", store::stable_hash(&self.origin));
        let source = t.text(rows[0], "source_id")?;
        let sample = source
            .strip_prefix(&prefix)
            .and_then(|s| s.parse::<i64>().ok())
            .ok_or("client_side_sdk_dispatch_clock_conflict")?;
        let amount = sample
            .checked_sub(self.consumed)
            .filter(|v| *v > 0)
            .ok_or("client_side_sdk_dispatch_clock_conflict")?;
        let mut kinds = BTreeSet::new();
        for row in rows {
            let kind = t.text(row, "kind")?;
            let key = format!("{prefix}{sample}:{kind}");
            let key = if ordinal == 1 {
                key
            } else {
                format!("worker:{worker}:{key}")
            };
            if t.text(row, "root_run_id")? != lease.root_run_id
                || t.text(row, "assignment_id")? != assignment
                || t.text(row, "lease_attempt_id")? != worker
                || t.text(row, "dimension")? != "wall_time_ms"
                || !matches!(kind, "reserve" | "consume")
                || !kinds.insert(kind)
                || t.text(row, "source_id")? != format!("{prefix}{sample}")
                || t.text(row, "idempotency_key")? != key
                || t.number(row, "amount")? != amount
                || sample < self.elapsed
                || sample > elapsed
                || uuid::Uuid::parse_str(t.text(row, "entry_id")?).is_err()
                || t.text(row, "created_at")? < now
                || t.text(row, "created_at")? > end
            {
                return Err("client_side_sdk_dispatch_new_cost_conflict".into());
            }
        }
        Ok(())
    }
}
fn read(db: &Connection, root: &str) -> Result<(String, i64), String> {
    let (origin, current, elapsed): (String, String, Option<i64>) = db
        .query_row(
            "SELECT o.started_at,r.created_at,
        CAST((julianday('now','localtime')-julianday(o.started_at))*86400000 AS INTEGER)
        FROM agent_budget_clock_origins o JOIN agent_runs r ON r.id=o.root_run_id WHERE r.id=?1",
            [root],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|_| "client_side_sdk_dispatch_clock_missing")?;
    if origin != current {
        return Err("client_side_sdk_dispatch_clock_conflict".into());
    }
    Ok((
        origin,
        elapsed
            .filter(|v| *v >= 0)
            .ok_or("client_side_sdk_dispatch_clock_conflict")?,
    ))
}
