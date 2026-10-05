//! Charge shared elapsed Root time without manufacturing a child worker.
use super::RootOwner;
use crate::agent_runtime::multi_agent::budget::{balance, Kind};
use rusqlite::{params, Transaction};

pub(super) fn sample(tx: &Transaction<'_>, owner: &RootOwner) -> Result<(), String> {
    owner.require_live(tx)?;
    let (origin,elapsed):(String,Option<i64>)=tx.query_row("SELECT started_at,CAST((julianday('now','localtime')-julianday(started_at))*86400000 AS INTEGER)
        FROM agent_budget_clock_origins WHERE root_run_id=?1",[&owner.root],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
    let elapsed = elapsed
        .filter(|v| *v >= 0)
        .ok_or("budget_clock_origin_invalid")?;
    let previous = balance(tx, &owner.root, None, "wall_time_ms")?;
    if previous.reserved != 0 || previous.indeterminate != 0 {
        return Err("budget_clock_unsettled_sample".into());
    }
    let delta = elapsed
        .checked_sub(previous.consumed)
        .filter(|v| *v >= 0)
        .ok_or("budget_clock_moved_backwards")?;
    if delta > 0 {
        let source = format!(
            "root-clock:{elapsed}:{}",
            crate::agent_runtime::store::stable_hash(&origin)
        );
        owner.append(
            tx,
            "wall_time_ms",
            Kind::Reserve,
            delta,
            &format!("{source}:reserve"),
            &source,
        )?;
        owner.append(
            tx,
            "wall_time_ms",
            Kind::Consume,
            delta,
            &format!("{source}:consume"),
            &source,
        )?;
    }
    let unchanged:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_budget_clock_origins WHERE root_run_id=?1 AND started_at=?2)",params![owner.root,origin],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !unchanged {
        return Err("budget_clock_origin_conflict".into());
    }
    owner.require_live(tx)
}
