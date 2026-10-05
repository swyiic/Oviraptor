//! Private ClientSide grant authority, held before BEGIN until COMMIT or rollback.
//! It owns its connection and never overwrites/clears a borrowed authorizer.
#[path = "client_grant_writer/costs.rs"]
mod costs;
#[path = "client_grant_writer/proof.rs"]
mod proof;
#[path = "client_grant_writer/rows.rs"]
mod rows;
#[path = "client_grant_writer/sequence.rs"]
mod sequence;
use crate::agent_runtime::{
    contract::AgentRole,
    multi_agent::{
        client_side::Task,
        lease::CoordinatorLease,
        scheduler::{self, ScheduledChild},
    },
};
use rusqlite::{
    hooks::{AuthAction, AuthContext, Authorization},
    Connection, OpenFlags,
};
use std::{cell::RefCell, path::Path, time::Duration};
const TRIGGER: &str = "client_side_frozen_observations_ready";
struct PrivateGrant {
    db: Connection,
}
impl PrivateGrant {
    fn open(path: &Path) -> Result<Self, String> {
        let db = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
        )
        .map_err(|e| format!("client_side_grant_open:{e}"))?;
        db.busy_timeout(Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        db.pragma_update(None, "foreign_keys", true)
            .map_err(|e| e.to_string())?;
        crate::collaboration_events::install_commit_notifications(&db);
        db.authorizer(Some(authorize)).map_err(|e| e.to_string())?;
        Ok(Self { db })
    }
}
pub(super) fn prepare(
    context: &super::AgentRunContext,
    lease: &CoordinatorLease,
    task: &serde_json::Value,
) -> Result<ScheduledChild, String> {
    let owner = PrivateGrant::open(&context.db_path)?;
    let frozen = Task::from_value(task, lease, 1)?;
    let proof = RefCell::new(None::<proof::Proof>);
    let result = scheduler::prepare_readonly_child_checked(
        &owner.db,
        lease,
        AgentRole::ClientSide,
        TRIGGER,
        task,
        8000,
        |tx| {
            require_authority(tx)?;
            crate::collaboration_events::client_grant_schema::verify(tx)?;
            super::client_side_verify_frozen_artifacts(&frozen, tx, lease, &context.target_dir)?;
            let mut state = proof.borrow_mut();
            match &*state {
                None => *state = Some(proof::Proof::capture(tx, lease, task, TRIGGER)?),
                Some(p) => p.verify(tx, lease)?,
            }
            require_authority(tx)
        },
    );
    // Dropping the private connection tears down its own hook. No None setter
    // can erase another writer's authority; no nested financial issuer is used.
    result.map_err(|code| {
        if code.contains("not authorized") || code.contains("authorization denied") {
            format!("client_side_grant_write_rejected:{code}")
        } else {
            code
        }
    })
}
fn require_authority(db: &Connection) -> Result<(), String> {
    // PREPARE only, never execute. Detect any nested hook clear before commit.
    // A lost hook cannot authorize a write: the whole caller transaction fails.
    match db.prepare("UPDATE projects SET name=name WHERE 0") {
        Err(rusqlite::Error::SqliteFailure(e, _))
            if e.code == rusqlite::ErrorCode::AuthorizationForStatementDenied =>
        {
            Ok(())
        }
        _ => Err("client_side_grant_authority_lost".into()),
    }
}
fn authorize(c: AuthContext<'_>) -> Authorization {
    let direct = c.database_name == Some("main") && c.accessor.is_none();
    let allowed = match c.action {
        AuthAction::Insert {
            table_name: "agent_collaboration_events",
        } if c.database_name == Some("main") => c.accessor.is_some_and(|n| {
            crate::collaboration_events::client_grant_schema::EMITTERS.contains(&n)
        }),
        AuthAction::Insert { table_name } if direct => matches!(
            table_name,
            "agent_assignments"
                | "agent_runs"
                | "agent_assignment_attempts"
                | "agent_capability_leases"
                | "agent_lane_leases"
                | "agent_contract_owners"
                | "agent_budget_ledger"
                | "agent_budget_entries"
        ),
        AuthAction::Update {
            table_name,
            column_name,
        } if direct => match table_name {
            "agent_assignments" => matches!(
                column_name,
                "lease_epoch"
                    | "fencing_token"
                    | "lease_expires_at"
                    | "state"
                    | "leased_at"
                    | "started_at"
                    | "updated_at"
            ),
            "agent_runs" => matches!(
                column_name,
                "status" | "plan_hash" | "evidence_hash" | "started_at" | "updated_at"
            ),
            "agent_assignment_attempts" => matches!(column_name, "state" | "heartbeat_at"),
            "agent_budget_ledger" => matches!(
                column_name,
                "reserved_tokens"
                    | "reserved_requests"
                    | "lease_epoch"
                    | "fencing_token"
                    | "updated_at"
            ),
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
}
