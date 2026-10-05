//! Original incurred elapsed only; no business finalization or execution grant.
use crate::agent_runtime::multi_agent::{budget::root::RootOwner, lease::CoordinatorLease};
use rusqlite::{
    hooks::{AuthAction, AuthContext, Authorization},
    Connection, OpenFlags, Transaction,
};

pub(crate) fn record_original_exit(db: &Connection, c: &CoordinatorLease) -> Result<(), String> {
    if !db.is_autocommit() {
        return Err("budget_exit_observation_transaction_required".into());
    }
    let path = db
        .path()
        .filter(|v| !v.is_empty())
        .ok_or("budget_exit_observation_database_missing")?;
    // Own a private RW/no CREATE connection; preserve the caller's hooks/txn.
    let private = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|e| e.to_string())?;
    private
        .busy_timeout(std::time::Duration::from_millis(250))
        .map_err(|e| e.to_string())?;
    private
        .execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;")
        .map_err(|e| e.to_string())?;
    private
        .authorizer(Some(authorize))
        .map_err(|e| e.to_string())?;
    let tx = Transaction::new_unchecked(&private, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    RootOwner::load_original(&tx, &c.root_run_id)?.require_original_coordinator(&tx, c)?;
    // One financial transaction and one cutoff. Delayed persistence cannot
    // convert an under-limit original exit into a different deadline fact.
    let cutoff: String = tx
        .query_row(
            "SELECT strftime('%Y-%m-%d %H:%M:%f','now','localtime')",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    #[cfg(test)]
    after_cutoff_for_test();
    super::elapsed_fact::record_at(&tx, c, true, Some(&cutoff))?;
    let mut observation = super::FinalClock::capture_observed(&tx, c, Some(&cutoff))?;
    observation.sample(&tx)?;
    observation.verify_observation(&tx)?;
    tx.commit().map_err(|e| e.to_string())
}
fn authorize(c: AuthContext<'_>) -> Authorization {
    match c.action {
        AuthAction::Insert {
            table_name: "agent_budget_entries" | "agent_root_elapsed_facts",
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

// Deterministic local commit-delay seam, never present in an installed build.
#[cfg(test)]
thread_local! {
    static EXIT_CUTOFF_CHECKPOINT: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
pub(crate) fn checkpoint_once_for_test(work: impl FnOnce() + 'static) {
    EXIT_CUTOFF_CHECKPOINT.with(|cell| {
        assert!(cell.borrow().is_none());
        *cell.borrow_mut() = Some(Box::new(work));
    });
}
#[cfg(test)]
fn after_cutoff_for_test() {
    let hook = EXIT_CUTOFF_CHECKPOINT.with(|cell| cell.borrow_mut().take());
    if let Some(work) = hook {
        work();
    }
}
