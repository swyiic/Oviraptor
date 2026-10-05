//! Original Client receipt delivery owns one private connection and one hook.
//! No cleanup, grant, provider retry, or borrowed authorizer replacement.
#[path = "client_delivery_writer/events.rs"]
mod events;
#[path = "client_delivery_writer/fees.rs"]
mod fees;
#[path = "client_delivery_writer/message.rs"]
mod message;
#[path = "client_delivery_writer/mutations.rs"]
mod mutations;
#[path = "client_delivery_writer/proof.rs"]
mod proof;
#[path = "client_delivery_writer/rows.rs"]
mod rows;
use crate::agent_runtime::multi_agent::{lease::CoordinatorLease, scheduler::ScheduledChild};
use rusqlite::{
    hooks::{AuthAction, AuthContext, Authorization},
    Connection, OpenFlags,
};
use std::{cell::RefCell, path::Path, time::Duration};
pub(super) fn complete(
    original: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
    usage: &super::AgentTokenUsage,
    payload: &serde_json::Value,
    target_dir: Option<&Path>,
) -> Result<serde_json::Value, String> {
    // Do not silently create an empty DB or bypass a caller's outer transaction.
    if !original.is_autocommit() {
        return Err("client_side_delivery_outer_transaction_forbidden".into());
    }
    let path = original
        .path()
        .filter(|p| !p.is_empty())
        .ok_or("client_side_delivery_existing_file_required")?;
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .map_err(|e| format!("client_side_delivery_open:{e}"))?;
    db.busy_timeout(Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    db.pragma_update(None, "foreign_keys", true)
        .map_err(|e| e.to_string())?;
    crate::collaboration_events::install_commit_notifications(&db);
    db.authorizer(Some(authorize)).map_err(|e| e.to_string())?;
    let frozen = RefCell::new(None::<proof::Proof>);
    let result = super::complete_assessment_at_checked(
        &db,
        lease,
        child,
        usage,
        payload,
        target_dir,
        |tx| {
            require_authority(tx)?;
            crate::collaboration_events::client_delivery_schema::verify(tx)?;
            let mut state = frozen.borrow_mut();
            match &*state {
                None => *state = Some(proof::Proof::capture(tx, lease, child, usage, payload)?),
                Some(p) => p.verify(tx, lease, child)?,
            }
            require_authority(tx)
        },
    );
    // Drop tears down only this owned connection. Never clear a borrowed hook.
    result.map_err(|code| {
        if code.contains("not authorized") || code.contains("authorization denied") {
            format!("client_side_delivery_write_rejected:{code}")
        } else {
            code
        }
    })
}
fn require_authority(db: &Connection) -> Result<(), String> {
    match db.prepare("UPDATE projects SET name=name WHERE 0") {
        Err(rusqlite::Error::SqliteFailure(e, _))
            if e.code == rusqlite::ErrorCode::AuthorizationForStatementDenied =>
        {
            Ok(())
        }
        _ => Err("client_side_delivery_authority_lost".into()),
    }
}
fn authorize(c: AuthContext<'_>) -> Authorization {
    let direct = c.database_name == Some("main") && c.accessor.is_none();
    let allow = match c.action {
        AuthAction::Insert {
            table_name: "agent_collaboration_events",
        } if c.database_name == Some("main") => c.accessor.is_some_and(|n| {
            crate::collaboration_events::client_delivery_schema::EMITTERS.contains(&n)
        }),
        AuthAction::Insert { table_name } if direct => {
            matches!(table_name, "agent_budget_entries" | "agent_messages")
        }
        AuthAction::Update {
            table_name,
            column_name,
        } if direct => match table_name {
            "agent_budget_ledger" => matches!(
                column_name,
                "reserved_tokens"
                    | "reserved_requests"
                    | "spent_tokens"
                    | "spent_requests"
                    | "updated_at"
            ),
            "agent_runs" => matches!(
                column_name,
                "used_tokens"
                    | "used_cached_tokens"
                    | "used_requests"
                    | "updated_at"
                    | "status"
                    | "terminal_state"
                    | "terminal_code"
                    | "terminal_reason"
                    | "finished_at"
            ),
            "agent_assignments" => matches!(
                column_name,
                "reserved_tokens"
                    | "reserved_requests"
                    | "budget_settled_at"
                    | "updated_at"
                    | "state"
                    | "failure_class"
                    | "finished_at"
            ),
            "agent_assignment_attempts" => {
                matches!(column_name, "state" | "finished_at" | "failure_class")
            }
            "agent_capability_leases" => column_name == "revoked_at",
            "agent_messages" => matches!(
                column_name,
                "delivered_at" | "acknowledged_at" | "delivery_attempts"
            ),
            _ => false,
        },
        AuthAction::Delete {
            table_name: "agent_lane_leases",
        } if direct => true,
        AuthAction::Read { .. }
        | AuthAction::Select
        | AuthAction::Function { .. }
        | AuthAction::Transaction { .. }
        | AuthAction::Savepoint { .. }
        | AuthAction::Recursive => true,
        _ => false,
    };
    if allow {
        Authorization::Allow
    } else {
        Authorization::Deny
    }
}
