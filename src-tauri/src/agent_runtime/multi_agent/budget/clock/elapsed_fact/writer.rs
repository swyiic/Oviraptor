//! Whole private financial transaction: one new immutable fact table only.
use rusqlite::{
    hooks::{AuthAction, AuthContext, Authorization},
    Connection, Transaction,
};

pub(super) fn run<T>(
    db: &Connection,
    work: impl FnOnce(&Transaction<'_>) -> Result<T, String>,
) -> Result<T, String> {
    if !db.is_autocommit() {
        return Err("budget_elapsed_fact_private_transaction_required".into());
    }
    let path = db.path().filter(|path| !path.is_empty())
        .ok_or("budget_elapsed_fact_database_missing")?;
    let private = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|e| e.to_string())?;
    private.busy_timeout(std::time::Duration::from_secs(10)).map_err(|e|e.to_string())?;
    private.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;").map_err(|e|e.to_string())?;
    let db = &private;
    db.authorizer(Some(authorize)).map_err(|e| e.to_string())?;
    (|| {
        let tx = Transaction::new_unchecked(db, rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let value = work(&tx)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(value)
    })()
}

fn authorize(c: AuthContext<'_>) -> Authorization {
    match c.action {
        AuthAction::Insert {
            table_name: "agent_root_elapsed_facts",
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
