//! Protect the local projection; its one UPDATE may not mutate business rows,
//! another proposal, earlier events, immutable input or the original cost.
use crate::agent_runtime::multi_agent::attempts::audit_rows::Rows;
use rusqlite::{
    hooks::{AuthAction, AuthContext, Authorization},
    Connection,
};
pub(super) struct Writer<'a> {
    db: &'a Connection,
    id: String,
    proposals: Rows,
    events: Rows,
    floor: i64,
    changes: u64,
    active: bool,
}
impl<'a> Writer<'a> {
    pub(super) fn install(db: &'a Connection, id: &str) -> Result<Self, String> {
        let floor = db
            .query_row(
                "SELECT COALESCE(MAX(sequence),0) FROM agent_collaboration_events",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let guard=Self{db,id:id.into(),proposals:Rows::read(db,"SELECT rowid,* FROM agent_directive_proposals WHERE directive_id<>?1 ORDER BY rowid",[id])?,
            events:Rows::read(db,"SELECT rowid,* FROM agent_collaboration_events WHERE sequence<=?1 ORDER BY sequence",[floor])?,floor,changes:db.total_changes(),active:true};
        db.authorizer(Some(authorize)).map_err(|e| e.to_string())?;
        Ok(guard)
    }
    pub(super) fn verify(&self) -> Result<(), String> {
        if Rows::read(self.db,"SELECT rowid,* FROM agent_directive_proposals WHERE directive_id<>?1 ORDER BY rowid",[&self.id])?!=self.proposals
            ||Rows::read(self.db,"SELECT rowid,* FROM agent_collaboration_events WHERE sequence<=?1 ORDER BY sequence",[self.floor])?!=self.events {
            return Err("proposal_owned_unrelated_rows_changed".into());
        }
        let rows=Rows::read(self.db,"SELECT scan_id,attempt_number,entity_type,entity_id,event_type,payload_json FROM agent_collaboration_events WHERE sequence>?1 ORDER BY sequence",[self.floor])?;
        // A received replay writes nothing; a newly persisted projection writes
        // one own row and the real collaboration update notification only.
        let delta = self
            .db
            .total_changes()
            .checked_sub(self.changes)
            .ok_or("proposal_owned_write_count_invalid")?;
        if delta == 0 {
            if !rows.values.is_empty() {
                return Err("proposal_owned_event_changed".into());
            }
        } else {
            let expected=Rows::read(self.db,"SELECT d.scan_id,d.attempt_number,'user_directive',d.id,'user_directive',
                json_object('proposalState',p.state)
                FROM agent_user_directives d JOIN agent_directive_proposals p ON p.directive_id=d.id WHERE d.id=?1",[&self.id])?;
            if delta != 2 || rows.values != expected.values {
                return Err("proposal_owned_projection_event_invalid".into());
            }
        }
        Ok(())
    }
}
impl Drop for Writer<'_> {
    fn drop(&mut self) {
        if self.active {
            let _ = self
                .db
                .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
        }
    }
}
fn authorize(context: AuthContext<'_>) -> Authorization {
    let allowed = match context.action {
        AuthAction::Update {
            table_name: "agent_directive_proposals",
            column_name,
        } if context.database_name == Some("main") && context.accessor.is_none() => matches!(
            column_name,
            "state" | "response_json" | "usage_json" | "updated_at"
        ),
        AuthAction::Insert {
            table_name: "agent_collaboration_events",
        } if context.database_name == Some("main") => {
            context.accessor == Some("agent_collaboration_directive_proposal")
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

// Refuse caller transactions; a private no-CREATE writer preserves caller hooks.
pub(super) fn private_connection(db: &Connection) -> Result<Connection, String> {
    if !db.is_autocommit() {
        return Err("proposal_owned_private_transaction_required".into());
    }
    let path = db
        .path()
        .filter(|p| !p.is_empty())
        .ok_or("proposal_owned_database_missing")?;
    let private = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
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
    Ok(private)
}
