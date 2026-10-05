//! Financially read-only inputs stay physical byte exact across local publication.
use super::{
    events,
    fees::Fees,
    message, mutations,
    rows::{self, get, Tables},
};
use crate::agent_runtime::{
    contract::AgentRole,
    multi_agent::{attempts, lease::CoordinatorLease, scheduler::ScheduledChild},
    store::UsageDelta,
};
use rusqlite::Connection;
pub(super) struct Proof {
    pub(super) tables: Tables,
    pub(super) root: String,
    pub(super) run: String,
    pub(super) assignment: String,
    pub(super) worker: String,
    pub(super) ordinal: i64,
    pub(super) usage: UsageDelta,
    pub(super) summary: String,
    pub(super) payload: String,
    pub(super) scan: String,
    pub(super) attempt: i64,
    pub(super) now: String,
    pub(super) paused: bool,
    pub(super) replay: bool,
    schema: String,
    fees: Option<Fees>,
}
impl Proof {
    pub(super) fn capture(
        db: &Connection,
        lease: &CoordinatorLease,
        child: &ScheduledChild,
        usage: &UsageDelta,
        payload: &serde_json::Value,
    ) -> Result<Self, String> {
        if db.is_autocommit() || child.role != AgentRole::ClientSide {
            return Err("client_side_delivery_context_invalid".into());
        }
        let worker = attempts::current(db, lease, &child.assignment_id)?;
        if worker.child_run_id != child.run_id {
            return Err("client_side_delivery_original_worker_conflict".into());
        }
        let tables = rows::capture(db)?;
        let t = get(&tables, "agent_assignments")?;
        let row = t.find("id", &child.assignment_id)?;
        let state = t.text(row, "state")?;
        if !matches!(state, "running" | "paused" | "completed") {
            return Err("client_side_delivery_state_conflict".into());
        }
        if t.text(row, "coordinator_run_id")? != lease.root_run_id
            || t.text(row, "child_run_id")? != child.run_id
            || t.text(row, "role")? != "client_side"
            || t.text(row, "lane")? != "read_only_analysis"
            || t.text(row, "fencing_token")? != lease.fencing_token
            || t.number(row, "lease_epoch")? != lease.lease_epoch
            || t.text(row, "target_key")? != lease.target_key
        {
            return Err("client_side_delivery_scope_conflict".into());
        }
        let redacted = crate::agent_runtime::secrets::redact_json(payload);
        let mut p = Self {
            root: lease.root_run_id.clone(),
            run: child.run_id.clone(),
            assignment: child.assignment_id.clone(),
            worker: worker.id,
            ordinal: worker.lease_epoch,
            usage: *usage,
            summary: redacted["summary"]
                .as_str()
                .ok_or("client_side_delivery_summary_missing")?
                .into(),
            payload: redacted.to_string(),
            scan: lease.scan_id.clone(),
            attempt: lease.attempt_number,
            now: db
                .query_row("SELECT datetime('now','localtime')", [], |r| r.get(0))
                .map_err(|e| e.to_string())?,
            paused: state == "paused",
            replay: state == "completed",
            schema: schema(db)?,
            tables,
            fees: None,
        };
        p.fees = Some(Fees::capture(db, &p)?);
        Ok(p)
    }
    pub(super) fn verify(
        &self,
        db: &Connection,
        lease: &CoordinatorLease,
        child: &ScheduledChild,
    ) -> Result<(), String> {
        if schema(db)? != self.schema
            || child.run_id != self.run
            || child.assignment_id != self.assignment
            || lease.root_run_id != self.root
        {
            return Err("client_side_delivery_scope_changed".into());
        }
        let actual = rows::capture(db)?;
        let end: String = db
            .query_row("SELECT datetime('now','localtime')", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let mut expected = mutations::expected(self, &actual, &end)?;
        let (id, insert, ack) = message::expected(self, &actual, &mut expected, &end)?;
        self.fees
            .as_ref()
            .ok_or("client_side_delivery_cost_proof_missing")?
            .verify(db, self, &actual, lease, &end)?;
        events::verify(self, &actual, &mut expected, &id, insert, ack, &end)?;
        if actual.len() != expected.len() {
            return Err("client_side_delivery_table_set_changed".into());
        }
        for (name, old) in &expected {
            let now = get(&actual, name)?;
            if old.columns != now.columns
                || old
                    .rows
                    .iter()
                    .any(|(id, row)| now.rows.get(id) != Some(row))
            {
                return Err(format!("client_side_delivery_original_row_changed:{name}"));
            }
            let added = now.added(old)?;
            let allowed = match name.as_str() {
                "agent_budget_entries" | "agent_collaboration_events" => true,
                "agent_messages" => insert,
                "sqlite_sequence" => {
                    !added.is_empty()
                        && !old.rows.values().any(|r| {
                            old.text(r, "name")
                                .is_ok_and(|n| n == "agent_collaboration_events")
                        })
                }
                _ => false,
            };
            if !allowed && !added.is_empty() {
                return Err(format!("client_side_delivery_extra_row:{name}"));
            }
        }
        Ok(())
    }
}
fn schema(db: &Connection) -> Result<String, String> {
    let mut text = String::new();
    for master in ["sqlite_master", "sqlite_temp_master"] {
        let mut q = db
            .prepare(&format!(
                "SELECT type,name,tbl_name,sql FROM {master} ORDER BY type,name"
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
        text.push_str(&format!("{master}:{rows:?}"));
    }
    Ok(crate::agent_runtime::store::stable_hash(&text))
}
