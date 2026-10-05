//! Restrict each local ordered-action transaction, never provider I/O.
use super::*;
use crate::agent_runtime::multi_agent::attempts::audit_rows::Rows;
use rusqlite::{
    hooks::AuthContext,
    types::Value as SqlValue,
};
mod authorization;
mod events;
#[derive(Clone, Copy)]
pub(crate) enum Mode {
    Schedule,
    Defer,
    Consume,
    Start,
    Receive,
    Finish,
}
pub(crate) struct Writer<'a> {
    db: &'a Connection,
    scope: CoordinatorLease,
    directive: String,
    child: scheduler::ScheduledChild,
    mode: Mode,
    other: Vec<Rows>,
    floor: i64,
}
impl<'a> Writer<'a> {
    pub(crate) fn install(
        db: &'a Connection,
        scope: &CoordinatorLease,
        id: &str,
        child: &scheduler::ScheduledChild,
        mode: Mode,
    ) -> Result<Self, String> {
        let floor = db
            .query_row(
                "SELECT COALESCE(MAX(sequence),0) FROM agent_collaboration_events",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let guard = Self {
            db,
            scope: scope.clone(),
            directive: id.into(),
            child: child.clone(),
            mode,
            other: snapshots(db, scope, id, child, floor)?,
            floor,
        };
        db.authorizer(Some(move |ctx: AuthContext<'_>| {
            authorization::authorize(ctx, mode)
        }))
        .map_err(|e| e.to_string())?;
        Ok(guard)
    }
    pub(crate) fn verify(&self) -> Result<(), String> {
        if snapshots(
            self.db,
            &self.scope,
            &self.directive,
            &self.child,
            self.floor,
        )? != self.other
        {
            return Err("ordered_unrelated_rows_changed".into());
        }
        events::verify(
            self.db,
            &self.scope,
            &self.directive,
            &self.child,
            self.mode,
            self.floor,
        )?;
        Ok(())
    }
}
// Phase writers own a new connection and never replace a caller's hook or
// enlist in its transaction. Install authorization before BEGIN; keep it
// through verification, original-parent checks and COMMIT or ROLLBACK.
pub(crate) fn private_connection(db: &Connection, mode: Mode) -> Result<Connection, String> {
    if !db.is_autocommit() {
        return Err("ordered_private_transaction_required".into());
    }
    let path = db.path().filter(|path| !path.is_empty())
        .ok_or("ordered_database_missing")?;
    let private = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|e| e.to_string())?;
    private.busy_timeout(std::time::Duration::from_millis(250)).map_err(|e|e.to_string())?;
    private.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;").map_err(|e|e.to_string())?;
    private.authorizer(Some(move |ctx: AuthContext<'_>| authorization::authorize(ctx, mode)))
        .map_err(|e|e.to_string())?;
    Ok(private)
}
fn snapshots(
    db: &Connection,
    scope: &CoordinatorLease,
    id: &str,
    child: &scheduler::ScheduledChild,
    floor: i64,
) -> Result<Vec<Rows>, String> {
    let text = |s: &str| SqlValue::Text(s.into());
    let mut output = vec![];
    for(sql,args)in[
      ("SELECT rowid,* FROM agent_runs WHERE id<>?1 ORDER BY rowid",vec![text(&child.run_id)]),
      ("SELECT rowid,* FROM agent_assignments WHERE id<>?1 ORDER BY rowid",vec![text(&child.assignment_id)]),
      ("SELECT rowid,* FROM agent_assignment_attempts WHERE child_run_id<>?1 ORDER BY rowid",vec![text(&child.run_id)]),
      ("SELECT rowid,* FROM agent_capability_leases WHERE child_run_id<>?1 ORDER BY rowid",vec![text(&child.run_id)]),
      ("SELECT rowid,* FROM agent_contract_owners WHERE assignment_id<>?1 ORDER BY rowid",vec![text(&child.assignment_id)]),
      ("SELECT rowid,* FROM agent_lane_leases WHERE assignment_id<>?1 ORDER BY rowid",vec![text(&child.assignment_id)]),
      ("SELECT rowid,* FROM agent_budget_ledger WHERE root_run_id<>?1 ORDER BY rowid",vec![text(&scope.root_run_id)]),
      ("SELECT rowid,* FROM agent_budget_entries WHERE assignment_id<>?1 ORDER BY rowid",vec![text(&child.assignment_id)]),
      ("SELECT rowid,* FROM agent_budget_limits ORDER BY rowid",vec![]),
      ("SELECT rowid,* FROM agent_budget_clock_origins ORDER BY rowid",vec![]),
      ("SELECT rowid,* FROM agent_messages WHERE assignment_id<>?1 ORDER BY rowid",vec![text(&child.assignment_id)]),
      ("SELECT rowid,* FROM agent_user_directives WHERE id<>?1 ORDER BY rowid",vec![text(id)]),
      ("SELECT rowid,* FROM agent_directive_ordered_actions WHERE child_run_id<>?1 ORDER BY rowid",vec![text(&child.run_id)]),
      ("SELECT rowid,* FROM agent_directive_ordered_receipts WHERE child_run_id<>?1 ORDER BY rowid",vec![text(&child.run_id)]),
      ("SELECT rowid,* FROM agent_collaboration_events WHERE sequence<=?1 ORDER BY rowid",vec![SqlValue::Integer(floor)]),
    ]{output.push(Rows::read(db,sql,rusqlite::params_from_iter(args))?)}
    Ok(output)
}
