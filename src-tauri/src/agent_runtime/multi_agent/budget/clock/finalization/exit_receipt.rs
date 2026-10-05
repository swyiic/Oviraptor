//! Durable original financial closure, never deletion or execution authority.
use super::{
    super::{elapsed_fact, final_receipt},
    FinalClock,
};
use crate::agent_runtime::multi_agent::{budget::root::RootOwner, lease::CoordinatorLease};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
mod proof;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Fact {
    version: u8,
    receipt_id: String,
    root: String,
    control: String,
    scan: String,
    attempt: i64,
    target: String,
    epoch: i64,
    fence: String,
    cutoff: String,
    state: String,
    code: String,
    reason: String,
    binding: String,
    sources: String,
    physical_root: String,
    terminal_event: String,
    wall: String,
    costs: String,
    dimensions: Vec<proof::Dimension>,
}

pub(super) fn persist_or_verify(
    db: &Connection,
    clock: &FinalClock,
    state: &str,
    code: &str,
    reason: &str,
) -> Result<(), String> {
    let root = &clock.lease.root_run_id;
    let original: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_root_budget_attempts WHERE root_run_id=?1)",
            [root],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !original {
        // Existing lower-level worker finance has no original Root control.
        // This path cannot create a new paid Root receipt or deletion proof.
        return Ok(());
    }
    if clock.was_terminal {
        // Never reconstruct a missing original receipt from mutable old labels.
        return FinalClock::verify_original_exit(db, root);
    }
    proof::verify_schema(db)?;
    let owner = RootOwner::load_original(db, root)?;
    owner.require_original_coordinator(db, &clock.lease)?;
    let c = &clock.lease;
    let fact = Fact {
        version: 1,
        receipt_id: uuid::Uuid::new_v4().to_string(),
        root: root.clone(),
        control: owner.id,
        scan: c.scan_id.clone(),
        attempt: c.attempt_number,
        target: c.target_key.clone(),
        epoch: c.lease_epoch,
        fence: c.fencing_token.clone(),
        cutoff: clock.cutoff.clone(),
        state: state.into(),
        code: code.into(),
        reason: reason.into(),
        binding: clock.binding.clone(),
        sources: clock.sources.clone(),
        physical_root: proof::physical_root(db, root)?,
        terminal_event: proof::terminal_event(db, root)?,
        wall: final_receipt::wall(db, root)?,
        costs: final_receipt::all_costs(db, root)?,
        dimensions: proof::dimensions(db, root)?,
    };
    let text = serde_json::to_string(&fact).map_err(|e| e.to_string())?;
    let n = db.execute("INSERT INTO agent_multi_exit_receipts(receipt_id,root_run_id,control_id,fact_json) VALUES(?1,?2,?3,?4)",
        params![fact.receipt_id,fact.root,fact.control,text]).map_err(|e|e.to_string())?;
    let exact: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM agent_multi_exit_receipts WHERE receipt_id=?1 AND root_run_id=?2 AND control_id=?3 AND fact_json=?4)",
        params![fact.receipt_id,fact.root,fact.control,text], |r|r.get(0)).map_err(|e|e.to_string())?;
    if n != 1 || !exact {
        return Err("budget_multi_exit_write_unconfirmed".into());
    }
    FinalClock::verify_original_exit(db, root)
}

pub(super) fn verify_original(db: &Connection, root: &str) -> Result<(), String> {
    if db.is_autocommit() {
        return Err("budget_multi_exit_requires_snapshot".into());
    }
    proof::verify_schema(db)?;
    let saved: Option<(String, String, String)> = db.query_row(
        "SELECT receipt_id,control_id,fact_json FROM agent_multi_exit_receipts WHERE root_run_id=?1",
        [root], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|e|e.to_string())?;
    let (id, control, text) = saved.ok_or("budget_multi_exit_original_receipt_missing")?;
    let fact: Fact =
        serde_json::from_str(&text).map_err(|_| "budget_multi_exit_receipt_invalid")?;
    let owner = RootOwner::load_original(db, root)?;
    if fact.version != 1
        || uuid::Uuid::parse_str(&fact.receipt_id).is_err()
        || fact.root != root
        || fact.receipt_id != id
        || fact.control != control
        || control != owner.id
        || serde_json::to_string(&fact).map_err(|e| e.to_string())? != text
        || proof::physical_root(db, root)? != fact.physical_root
        || proof::terminal_event(db, root)? != fact.terminal_event
        || proof::dimensions(db, root)? != fact.dimensions
    {
        return Err("budget_multi_exit_original_receipt_conflict".into());
    }
    let lease = CoordinatorLease {
        scan_id: fact.scan,
        attempt_number: fact.attempt,
        target_key: fact.target,
        root_run_id: fact.root,
        lease_epoch: fact.epoch,
        fencing_token: fact.fence,
        lease_expires_at: String::new(),
    };
    owner.require_original_coordinator(db, &lease)?;
    let clock = FinalClock {
        exceptional: elapsed_fact::verify_existing(db, &lease)?,
        lease,
        cutoff: fact.cutoff,
        was_terminal: true,
        binding: fact.binding,
        sources: fact.sources,
        wall_receipt: Some(fact.wall),
        receipt: Some(fact.costs),
        event_floor: db
            .query_row(
                "SELECT COALESCE(MAX(sequence),0) FROM agent_collaboration_events",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?,
    };
    // Uses the same original financial proof, without requiring or adopting live C.
    // Unknown dimensions remain unknown; no writer, owner or capability escapes.
    clock.verify_financial_closed(db, &fact.state, &fact.code, &fact.reason)
}
