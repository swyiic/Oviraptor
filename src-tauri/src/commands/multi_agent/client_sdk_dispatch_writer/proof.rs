//! Dispatch may add one exact claim and a shared-clock delta; all old rows stay exact.
use super::{
    clock::{Clock, Scope},
    rows::{self, get, Table, Tables},
};
use crate::agent_runtime::{
    contract::AgentRole,
    multi_agent::{attempts, lease::CoordinatorLease, scheduler::ScheduledChild},
    store,
};
use rusqlite::Connection;
pub(super) struct Proof {
    tables: Tables,
    request: String,
    hash: String,
    worker: String,
    ordinal: i64,
    now: String,
    clock: Clock,
    schema: String,
}
impl Proof {
    pub(super) fn capture(
        db: &Connection,
        lease: &CoordinatorLease,
        child: &ScheduledChild,
        request: &serde_json::Value,
    ) -> Result<Self, String> {
        if db.is_autocommit() || child.role != AgentRole::ClientSide {
            return Err("client_side_sdk_dispatch_context_invalid".into());
        }
        let task = crate::agent_runtime::multi_agent::client_side::assignment(db, lease, child)?;
        if task.phase != "client_side_readonly" {
            return Err("client_side_sdk_dispatch_task_invalid".into());
        }
        let original = attempts::current(db, lease, &child.assignment_id)?;
        if original.child_run_id != child.run_id {
            return Err("client_side_sdk_dispatch_original_worker_conflict".into());
        }
        let request = crate::agent_runtime::secrets::redact_json(request).to_string();
        Ok(Self {
            tables: rows::capture(db)?,
            hash: store::stable_hash(&request),
            request,
            worker: original.id,
            ordinal: original.lease_epoch,
            now: db
                .query_row("SELECT datetime('now','localtime')", [], |r| r.get(0))
                .map_err(|e| e.to_string())?,
            clock: Clock::capture(db, lease)?,
            schema: schema(db)?,
        })
    }
    pub(super) fn verify(
        &self,
        db: &Connection,
        lease: &CoordinatorLease,
        child: &ScheduledChild,
    ) -> Result<(), String> {
        if schema(db)? != self.schema {
            return Err("client_side_sdk_dispatch_schema_changed".into());
        }
        let actual = rows::capture(db)?;
        if actual.len() != self.tables.len() {
            return Err("client_side_sdk_dispatch_table_set_changed".into());
        }
        let end: String = db
            .query_row("SELECT datetime('now','localtime')", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        for (name, old) in &self.tables {
            let current = get(&actual, name)?;
            if old.columns != current.columns
                || old
                    .rows
                    .iter()
                    .any(|(id, row)| current.rows.get(id) != Some(row))
            {
                return Err(format!(
                    "client_side_sdk_dispatch_original_row_changed:{name}"
                ));
            }
            let added = current.added(old)?;
            match name.as_str() {
                "agent_specialist_calls" => {
                    if added.len() != 1 {
                        return Err("client_side_sdk_dispatch_call_count_conflict".into());
                    }
                    self.call(current, added[0], lease, child, &end)?;
                }
                "agent_budget_entries" => self.clock.verify(
                    db,
                    current,
                    &added,
                    &Scope {
                        lease,
                        assignment: &child.assignment_id,
                        worker: &self.worker,
                        ordinal: self.ordinal,
                        now: &self.now,
                        end: &end,
                    },
                )?,
                _ if !added.is_empty() => {
                    return Err(format!("client_side_sdk_dispatch_extra_row:{name}"))
                }
                _ => {}
            }
        }
        Ok(())
    }
    fn call(
        &self,
        t: &Table,
        row: &[rusqlite::types::Value],
        lease: &CoordinatorLease,
        child: &ScheduledChild,
        end: &str,
    ) -> Result<(), String> {
        for col in &t.columns {
            let expected = match col.as_str() {
                "rowid" => continue,
                "assignment_id" => child.assignment_id.as_str(),
                "child_run_id" => child.run_id.as_str(),
                "root_run_id" => lease.root_run_id.as_str(),
                "role" => "client_side",
                "fencing_token" => lease.fencing_token.as_str(),
                "request_json" => self.request.as_str(),
                "request_hash" => self.hash.as_str(),
                "state" => "executing",
                "response_json" | "usage_json" => "{}",
                "response_hash" | "failure_code" | "finished_at" => "",
                "lease_epoch" if t.number(row, col)? == lease.lease_epoch => continue,
                "event_sequence" if t.number(row, col)? == 0 => continue,
                "created_at"
                    if t.text(row, col)? >= self.now.as_str() && t.text(row, col)? <= end =>
                {
                    continue
                }
                _ => return Err("client_side_sdk_dispatch_call_schema_conflict".into()),
            };
            if t.text(row, col)? != expected {
                return Err("client_side_sdk_dispatch_call_scope_conflict".into());
            }
        }
        Ok(())
    }
}
fn schema(db: &Connection) -> Result<String, String> {
    let mut text = String::new();
    for name in ["sqlite_master", "sqlite_temp_master"] {
        let mut q = db
            .prepare(&format!(
                "SELECT type,name,tbl_name,sql FROM {name} ORDER BY type,name"
            ))
            .map_err(|e| e.to_string())?;
        let rows = q
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        text.push_str(&format!("{name}:{rows:?}"));
    }
    Ok(store::stable_hash(&text))
}
