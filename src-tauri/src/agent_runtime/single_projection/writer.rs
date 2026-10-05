//! Only a privately opened connection may own this entire publication boundary.
use rusqlite::{
    hooks::{AuthAction, AuthContext, Authorization},
    Connection, Transaction, TransactionBehavior,
};
pub(super) fn write<T>(
    db: &Connection,
    work: impl FnOnce(&Transaction<'_>) -> Result<T, String>,
) -> Result<T, String> {
    if !db.is_autocommit() {
        return Err("single_projection_private_transaction_required".into());
    }
    db.authorizer(Some(authorize)).map_err(|e| e.to_string())?;
    let result: Result<T, String> = (|| {
        let tx = Transaction::new_unchecked(db, TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        crate::collaboration_events::closure_schema::verify(&tx)?;
        let value = work(&tx)?;
        crate::collaboration_events::closure_schema::verify(&tx)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(value)
    })();
    let clear = db
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .map_err(|e| e.to_string());
    let value = result?;
    clear?;
    Ok(value)
}
fn authorize(c: AuthContext<'_>) -> Authorization {
    let direct = c.database_name == Some("main") && c.accessor.is_none();
    let allowed = match c.action {
        AuthAction::Insert {
            table_name: "agent_collaboration_events",
        } => {
            c.database_name == Some("main") && c.accessor == Some("agent_collaboration_run_update")
        }
        AuthAction::Insert { table_name } if direct => matches!(
            table_name,
            "agent_events" | "agent_snapshots" | "agent_single_projection_receipts"
        ),
        AuthAction::Update {
            table_name: "agent_runs",
            column_name,
        } if direct => matches!(
            column_name,
            "status"
                | "terminal_state"
                | "terminal_code"
                | "terminal_reason"
                | "finished_at"
                | "updated_at"
        ),
        AuthAction::Update {
            table_name: "agent_snapshots",
            column_name,
        } if direct => matches!(
            column_name,
            "last_sequence" | "schema_version" | "snapshot_json" | "updated_at"
        ),
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
}
