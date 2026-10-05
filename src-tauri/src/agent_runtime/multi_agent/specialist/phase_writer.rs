//! Private writer for original unknown/no-send call phase and fee bookkeeping.
//! The caller's connection and authorizer are never changed or committed.
use rusqlite::{
    hooks::{AuthAction, AuthContext, Authorization},
    Connection, OpenFlags,
};

pub(super) fn run<T>(
    db: &Connection,
    work: impl FnOnce(&Connection) -> Result<T, String>,
) -> Result<T, String> {
    if !db.is_autocommit() {
        return Err("specialist_phase_private_transaction_required".into());
    }
    let path = db
        .path()
        .filter(|p| !p.is_empty())
        .ok_or("specialist_phase_database_missing")?;
    let private = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|e| e.to_string())?;
    private
        .busy_timeout(std::time::Duration::from_millis(250))
        .map_err(|e| e.to_string())?;
    private
        .execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;")
        .map_err(|e| e.to_string())?;
    // Install before the inner BEGIN and keep through its COMMIT/ROLLBACK.
    private
        .authorizer(Some(authorize))
        .map_err(|e| e.to_string())?;
    work(&private)
}

fn authorize(c: AuthContext<'_>) -> Authorization {
    let allowed = match c.action {
        AuthAction::Update {
            table_name: "agent_specialist_calls",
            column_name: "state" | "failure_code" | "finished_at",
        }
        | AuthAction::Insert {
            table_name: "agent_budget_entries" | "agent_model_cost_facts",
        } => c.database_name == Some("main") && c.accessor.is_none(),
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
