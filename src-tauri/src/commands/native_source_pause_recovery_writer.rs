// Isolated six-column authority for historical result projection. No caller
// authorizer is displaced; all trigger, Root, budget, asset and DDL writes deny.
fn source_pause_recovery_write<T>(
    path: &Path,
    work: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T, String>,
) -> Result<T, String> {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    let db =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(|e| e.to_string())?;
    db.busy_timeout(Duration::from_secs(10))
        .map_err(|e| e.to_string())?;
    db.execute_batch("PRAGMA foreign_keys=ON;PRAGMA synchronous=FULL;")
        .map_err(|e| e.to_string())?;
    db.authorizer(Some(|c: AuthContext<'_>| {
        let allowed = match c.action {
            AuthAction::Update {
                table_name,
                column_name,
            } if c.database_name == Some("main") && c.accessor.is_none() => match table_name {
                "native_scan_branches" => matches!(
                    column_name,
                    "status" | "checkpoint" | "report_json" | "updated_at"
                ),
                "sentinel_targets" => matches!(column_name, "status" | "last_attempt_number"),
                _ => false,
            },
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
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let receipt = work(&tx)?;
    tx.commit()
        .map_err(|e| format!("source_pause_recovery_commit_unconfirmed:{e}"))?;
    Ok(receipt)
}
