//! Private connection, one entire transaction, no target/publication authority.
use rusqlite::{
    hooks::{AuthAction, AuthContext, Authorization},
    Connection, Transaction, TransactionBehavior,
};
pub(super) fn write<T>(
    db: &Connection,
    work: impl FnOnce(&Transaction<'_>) -> Result<T, String>,
) -> Result<T, String> {
    if !db.is_autocommit() {
        return Err("budget_single_exit_private_transaction_required".into());
    }
    let path = db.path().filter(|path| !path.is_empty())
        .ok_or("budget_single_exit_database_missing")?;
    let private = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|e| e.to_string())?;
    private.busy_timeout(std::time::Duration::from_secs(10)).map_err(|e|e.to_string())?;
    private.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;").map_err(|e|e.to_string())?;
    let db = &private;
    db.authorizer(Some(|c: AuthContext<'_>| {
        let allowed = match c.action {
            AuthAction::Insert { table_name } => {
                c.database_name == Some("main")
                    && c.accessor.is_none()
                    && matches!(
                        table_name,
                        "agent_budget_entries" | "agent_single_exit_receipts"
                    )
            }
            AuthAction::Read { .. }
            | AuthAction::Select
            | AuthAction::Function { .. }
            | AuthAction::Transaction { .. }
            | AuthAction::Savepoint { .. }
            | AuthAction::Recursive => true,
            _ => false,
        };
        if allowed {
            Authorization::Allow
        } else {
            Authorization::Deny
        }
    }))
    .map_err(|e| e.to_string())?;
    (|| {
        let tx = Transaction::new_unchecked(db, TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let value = work(&tx)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(value)
    })()
}
