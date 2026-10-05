//! Private normal/late SDK receipt writer with bounded SQL authority.
use rusqlite::{
    hooks::{AuthAction, AuthContext, Authorization},
    Connection, OpenFlags,
};

pub(super) fn run<T>(
    db: &Connection,
    work: impl FnOnce(&Connection) -> Result<T, String>,
) -> Result<T, String> {
    if !db.is_autocommit() {
        return Err("specialist_receipt_private_transaction_required".into());
    }
    let path = db
        .path()
        .filter(|p| !p.is_empty())
        .ok_or("specialist_receipt_database_missing")?;
    let private = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|e| e.to_string())?;
    private
        .busy_timeout(std::time::Duration::from_millis(250))
        .map_err(|e| e.to_string())?;
    private
        .execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;")
        .map_err(|e| e.to_string())?;
    // Own before the inner BEGIN and through its COMMIT/ROLLBACK; preserve caller hook.
    private
        .authorizer(Some(authorize))
        .map_err(|e| e.to_string())?;
    work(&private)
}
fn authorize(c: AuthContext<'_>) -> Authorization {
    let direct = c.database_name == Some("main") && c.accessor.is_none();
    let allowed = match c.action {
        AuthAction::Insert {
            table_name:
                "agent_events" | "agent_snapshots" | "agent_budget_entries" | "agent_model_cost_facts",
        } => direct,
        AuthAction::Update {
            table_name: "agent_specialist_calls",
            column_name:
                "state" | "response_json" | "usage_json" | "response_hash" | "event_sequence"
                | "finished_at",
        }
        | AuthAction::Update {
            table_name: "agent_snapshots",
            column_name: "last_sequence" | "schema_version" | "snapshot_json" | "updated_at",
        }
        | AuthAction::Update {
            table_name: "agent_user_directives",
            column_name: "payload_json" | "updated_at",
        } => direct,
        // This emitter is verified inside the same transaction before publication.
        AuthAction::Insert {
            table_name: "agent_collaboration_events",
        } => {
            c.database_name == Some("main")
                && c.accessor == Some("agent_collaboration_directive_delivery")
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
}
