//! Protect only the actual Client durable dispatch before provider I/O.
//! Receipt and failure settlement use their own writers and contracts.
#[path = "client_sdk_dispatch_writer/clock.rs"]
mod clock;
#[path = "client_sdk_dispatch_writer/proof.rs"]
mod proof;
#[path = "client_sdk_dispatch_writer/rows.rs"]
mod rows;
use crate::agent_runtime::execution_owner::NativeInvocationOwner;
use crate::agent_runtime::multi_agent::{
    lease::CoordinatorLease, scheduler::ScheduledChild, specialist,
};
use rusqlite::{
    hooks::{AuthAction, AuthContext, Authorization},
    Connection, OpenFlags,
};
use std::{cell::RefCell, path::Path, time::Duration};
pub(super) fn start(
    path: &Path,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
    request: &serde_json::Value,
    authorize_runtime: impl Fn(&Connection) -> Result<(), String>,
) -> Result<(specialist::Start, Option<NativeInvocationOwner>), String> {
    let run = || {
        let db = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
        )
        .map_err(|e| format!("client_side_sdk_dispatch_open:{e}"))?;
        db.busy_timeout(Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        db.pragma_update(None, "foreign_keys", true)
            .map_err(|e| e.to_string())?;
        crate::collaboration_events::install_commit_notifications(&db);
        db.authorizer(Some(authority)).map_err(|e| e.to_string())?;
        let frozen = RefCell::new(None::<proof::Proof>);
        specialist::start_for_transport(&db, lease, child, request, |tx| {
            require_authority(tx)?;
            authorize_runtime(tx)?;
            let mut old = frozen.borrow_mut();
            match &*old {
                None => *old = Some(proof::Proof::capture(tx, lease, child, request)?),
                Some(p) => p.verify(tx, lease, child)?,
            }
            require_authority(tx)
        })
    };
    run().map_err(|error| {
        if error.starts_with("client_side_sdk_dispatch_") {
            error
        } else if error.contains("not authorized") || error.contains("authorization denied") {
            format!("client_side_sdk_dispatch_write_rejected:{error}")
        } else {
            format!("client_side_sdk_dispatch_rejected:{error}")
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
        _ => Err("client_side_sdk_dispatch_authority_lost".into()),
    }
}
fn authority(c: AuthContext<'_>) -> Authorization {
    let direct = c.database_name == Some("main") && c.accessor.is_none();
    let allow = match c.action {
        AuthAction::Insert { table_name } if direct => matches!(
            table_name,
            "agent_specialist_calls" | "agent_budget_entries"
        ),
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
