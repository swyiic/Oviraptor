//! Retain an incurred original bill after business publication rolled back.
//! No business writes, current authority adoption, response publication or retry.
use super::super::PendingCall;
use crate::agent_runtime::{
    multi_agent::budget::{model_facts, root::RootOwner},
    store::UsageDelta,
};
use rusqlite::{
    hooks::{AuthAction, AuthContext, Authorization},
    Connection, OpenFlags, Transaction, TransactionBehavior,
};

pub(super) fn record(
    db: &Connection,
    call: &PendingCall,
    hash: &str,
    usage: &UsageDelta,
    reported: bool,
) -> Result<(), String> {
    write(
        db,
        call,
        model_facts::Fact::Received {
            hash,
            usage,
            reported,
        },
    )
}

pub(super) fn uncertain(db: &Connection, call: &PendingCall) -> Result<(), String> {
    write(db, call, model_facts::Fact::Uncertain)
}

fn write(db: &Connection, call: &PendingCall, fact: model_facts::Fact<'_>) -> Result<(), String> {
    if !db.is_autocommit() {
        return Err("specialist_failed_cost_private_transaction_required".into());
    }
    let path = db
        .path()
        .filter(|p| !p.is_empty())
        .ok_or("specialist_failed_cost_database_missing")?;
    let private = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|e| e.to_string())?;
    private
        .busy_timeout(std::time::Duration::from_millis(250))
        .map_err(|e| e.to_string())?;
    private
        .execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;")
        .map_err(|e| e.to_string())?;
    // Own the hook before BEGIN through COMMIT. Never replace the caller's hook.
    private
        .authorizer(Some(authorize))
        .map_err(|e| e.to_string())?;
    let tx = Transaction::new_unchecked(&private, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    RootOwner::load_original(&tx, &call.lease.root_run_id)?
        .require_original_coordinator(&tx, &call.lease)?;
    model_facts::specialist(&tx, &call.lease, &call.child, &call.request_hash, fact)?;
    tx.commit().map_err(|e| e.to_string())
}
fn authorize(c: AuthContext<'_>) -> Authorization {
    match c.action {
        AuthAction::Insert {
            table_name: "agent_budget_entries" | "agent_model_cost_facts",
        } if c.database_name == Some("main") && c.accessor.is_none() => Authorization::Allow,
        AuthAction::Read { .. }
        | AuthAction::Select
        | AuthAction::Function { .. }
        | AuthAction::Transaction { .. }
        | AuthAction::Savepoint { .. }
        | AuthAction::Recursive => Authorization::Allow,
        _ => Authorization::Deny,
    }
}
