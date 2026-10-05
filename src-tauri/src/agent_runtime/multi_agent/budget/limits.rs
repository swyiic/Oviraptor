//! Freeze the current Native execution contract, never current UI defaults.
//! Missing plans are not authority. Existing roots without journal sources
//! require reconciliation rather than an automatic historical backfill.
use super::{balance, DIMENSIONS};
mod contract;

pub(crate) fn verify_root_contract(db: &rusqlite::Connection, root: &str) -> Result<(), String> {
    contract::verify(db, root)
}
use crate::agent_runtime::multi_agent::lease::CoordinatorLease;
use rusqlite::{params, Transaction};
use serde_json::Value;

pub(super) fn frozen_wall_time(plan: &Value) -> Result<i64, String> {
    let timeout = if plan["surface"] == "source" {
        &plan["runtime"]["budget"]["timeoutSeconds"]
    } else {
        &plan["timeoutSeconds"]
    };
    timeout
        .as_i64()
        .filter(|v| *v > 0)
        .and_then(|v| v.checked_mul(1000))
        .ok_or_else(|| "budget_frozen_timeout_invalid".into())
}

pub(crate) fn initialize(tx: &Transaction<'_>, lease: &CoordinatorLease) -> Result<(), String> {
    initialize_root(tx, &lease.root_run_id)
}

pub(super) fn initialize_root(tx: &Transaction<'_>, root: &str) -> Result<(), String> {
    let values = contract::frozen(tx, root)?;
    let count: i64 = tx
        .query_row(
            "SELECT count(*) FROM agent_budget_limits WHERE root_run_id=?1",
            [root],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if count == 0 {
        let historical: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_assignments WHERE coordinator_run_id=?1)",
                [root],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if historical {
            return Err("budget_history_requires_reconciliation".into());
        }
        super::clock::freeze(tx, root)?;
        for (dimension, limit) in DIMENSIONS.iter().zip(values) {
            let changed=tx.execute("INSERT INTO agent_budget_limits(root_run_id,dimension,hard_limit) VALUES(?1,?2,?3)",
                params![root,dimension,limit]).map_err(|e|e.to_string())?;
            if changed != 1 {
                return Err("budget_dimension_contract_incomplete".into());
            }
        }
    } else if count != 10 {
        return Err("budget_dimension_contract_incomplete".into());
    }
    contract::verify(tx, root)?;
    for dimension in DIMENSIONS {
        if balance(tx, root, None, dimension)?.indeterminate > 0 {
            return Err("budget_indeterminate_requires_reconciliation".into());
        }
    }
    let uncertain:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_specialist_calls WHERE root_run_id=?1 AND state='uncertain') OR EXISTS(SELECT 1 FROM agent_source_model_rounds WHERE root_run_id=?1 AND state='uncertain')",
        [root],|r|r.get(0)).map_err(|e|e.to_string())?;
    if uncertain {
        return Err("budget_indeterminate_requires_reconciliation".into());
    }
    Ok(())
}

pub(crate) fn reserve_slot(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    assignment: &str,
) -> Result<(), String> {
    if super::root_definition::read(tx, &lease.root_run_id)?.is_some() {
        return super::execution_slots::require(tx, lease, assignment, true, false);
    }
    super::append(
        tx,
        lease,
        assignment,
        "concurrency_batches",
        super::Kind::Reserve,
        1,
        &format!("slot:{assignment}"),
        &format!("assignment:{assignment}"),
    )
}

pub(crate) fn release_slot(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    assignment: &str,
) -> Result<(), String> {
    release_slot_with(tx, lease, assignment, false)
}

pub(crate) fn release_terminal_slot(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    assignment: &str,
) -> Result<(), String> {
    release_slot_with(tx, lease, assignment, true)
}

fn release_slot_with(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    assignment: &str,
    terminal_receipt: bool,
) -> Result<(), String> {
    if super::root_definition::read(tx, &lease.root_run_id)?.is_some() {
        return super::execution_slots::require(tx, lease, assignment, false, terminal_receipt);
    }
    let reserved =
        super::scope::current_balance(tx, lease, assignment, "concurrency_batches")?.reserved;
    if reserved != 1 {
        return Err("budget_concurrency_reservation_conflict".into());
    }
    let write = if terminal_receipt {
        super::append_terminal_receipt
    } else {
        super::append
    };
    write(
        tx,
        lease,
        assignment,
        "concurrency_batches",
        super::Kind::Release,
        1,
        &format!("slot-finish:{assignment}"),
        &format!("terminal:{assignment}"),
    )
}
