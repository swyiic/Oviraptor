//! Exact write set for one new ClientSide grant; old rows never become grants.
use super::{costs::Costs, rows::Table, sequence::Sequence};
use crate::agent_runtime::{multi_agent::lease::CoordinatorLease, store};
use rusqlite::{params, types::Value, Connection};
use serde_json::{json, Value as JsonValue};
use std::collections::BTreeSet;
const TABLES: [&str; 15] = [
    "agent_runs",
    "agent_assignments",
    "agent_assignment_attempts",
    "agent_capability_leases",
    "agent_lane_leases",
    "agent_contract_owners",
    "agent_budget_ledger",
    "agent_budget_entries",
    "agent_collaboration_events",
    "agent_root_budget_attempts",
    "agent_root_model_journal",
    "agent_budget_limits",
    "agent_budget_clock_origins",
    "agent_root_budget_definitions",
    "agent_root_mode_definitions",
];
pub(super) struct Proof {
    tables: Vec<Table>,
    assignment: String,
    run: String,
    replay: bool,
    schema: String,
    now: String,
    costs: Costs,
    sequence: Sequence,
}
impl Proof {
    pub(super) fn capture(
        db: &Connection,
        lease: &CoordinatorLease,
        task: &JsonValue,
        trigger: &str,
    ) -> Result<Self, String> {
        let assignment = format!(
            "asg-{}",
            &store::stable_hash(&format!(
                "{}:client_side:{trigger}:{}:1",
                lease.root_run_id, lease.target_key
            ))[..24]
        );
        let replay: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_assignments WHERE id=?1)",
                [&assignment],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        crate::agent_runtime::multi_agent::client_side::Task::from_value(task, lease, 1)?;
        let tables = TABLES
            .into_iter()
            .map(|t| Table::read(db, t))
            .collect::<Result<Vec<_>, _>>()?;
        let now = db
            .query_row("SELECT datetime('now','localtime')", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        Ok(Self {
            run: format!("run-{assignment}"),
            assignment,
            replay,
            tables,
            schema: schema(db)?,
            now,
            costs: Costs::capture(db, lease)?,
            sequence: Sequence::capture(db)?,
        })
    }
    pub(super) fn verify(&self, db: &Connection, lease: &CoordinatorLease) -> Result<(), String> {
        if self.schema != schema(db)? {
            return Err("client_side_grant_schema_conflict".into());
        }
        let worker: String = if self.replay {
            String::new()
        } else {
            db.query_row("SELECT id FROM agent_assignment_attempts WHERE assignment_id=?1 AND child_run_id=?2
                AND root_run_id=?3 AND coordinator_epoch=?4 AND coordinator_fencing_token=?5 AND lease_epoch=1 AND state='running'",
                params![self.assignment,self.run,lease.root_run_id,lease.lease_epoch,lease.fencing_token],|r|r.get(0))
                .map_err(|_|"client_side_grant_worker_scope_conflict")?
        };
        if !self.replay && uuid::Uuid::parse_str(&worker).is_err() {
            return Err("client_side_grant_worker_scope_conflict".into());
        }
        for old in &self.tables {
            let current = Table::read(db, old.name)?;
            if old.name == "agent_budget_ledger" && !self.replay {
                self.ledger(db, &current, old, lease)?;
            } else {
                current.require_old(old)?;
            }
            let added = current.new_rows(old)?;
            if self.replay {
                if !added.is_empty() {
                    return Err("client_side_grant_replay_wrote_rows".into());
                }
                continue;
            }
            self.added(db, &current, &added, lease, &worker)?;
        }
        self.sequence.verify(db, if self.replay { 0 } else { 5 })?;
        Ok(())
    }
    fn ledger(
        &self,
        db: &Connection,
        current: &Table,
        old: &Table,
        lease: &CoordinatorLease,
    ) -> Result<(), String> {
        if current.columns != old.columns || current.rows.len() != old.rows.len() {
            return Err("client_side_grant_ledger_conflict".into());
        }
        let end: String = db
            .query_row("SELECT datetime('now','localtime')", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let mut count = 0;
        for row in &old.rows {
            let mut expected = row.clone();
            if old.text(row, "root_run_id")? == lease.root_run_id {
                count += 1;
                for (column, delta) in [("reserved_tokens", 8000), ("reserved_requests", 1)] {
                    let n = old
                        .columns
                        .iter()
                        .position(|c| c == column)
                        .ok_or("client_side_grant_schema_conflict")?;
                    expected[n] = Value::Integer(
                        old.number(row, column)?
                            .checked_add(delta)
                            .ok_or("budget_amount_overflow")?,
                    );
                }
                let actual = current.physical(row)?;
                let updated = current.text(actual, "updated_at")?;
                if updated < self.now.as_str() || updated > end.as_str() {
                    return Err("client_side_grant_ledger_conflict".into());
                }
                let n = old
                    .columns
                    .iter()
                    .position(|c| c == "updated_at")
                    .ok_or("client_side_grant_schema_conflict")?;
                expected[n] = Value::Text(updated.into());
            }
            if !current.rows.contains(&expected) {
                return Err("client_side_grant_original_row_changed".into());
            }
        }
        if count != 1 {
            return Err("client_side_grant_ledger_missing".into());
        }
        Ok(())
    }
    fn added(
        &self,
        db: &Connection,
        t: &Table,
        rows: &[&[Value]],
        lease: &CoordinatorLease,
        worker: &str,
    ) -> Result<(), String> {
        let count = match t.name {
            "agent_assignments"
            | "agent_runs"
            | "agent_assignment_attempts"
            | "agent_lane_leases"
            | "agent_contract_owners" => Some(1),
            "agent_capability_leases" => Some(2),
            "agent_budget_entries" | "agent_collaboration_events" => None,
            _ => Some(0),
        };
        if count.is_some_and(|n| rows.len() != n) {
            return Err("client_side_grant_write_count_conflict".into());
        }
        let mut event_states = BTreeSet::new();
        for row in rows {
            match t.name {
                "agent_runs" => {
                    t.only(row, "id", &self.run)?;
                    t.only(row, "root_run_id", &lease.root_run_id)?;
                    t.only(row, "parent_run_id", &lease.root_run_id)?;
                    t.only(row, "assignment_id", &self.assignment)?;
                    t.only(row, "scan_id", &lease.scan_id)?;
                    t.only(row, "target_url", &lease.target_key)?;
                    t.only(row, "role", "client_side")?;
                    t.only(row, "lane", "read_only_analysis")?;
                    t.only(row, "status", "running")?;
                    if t.number(row, "attempt_number")? != lease.attempt_number {
                        return Err("client_side_grant_row_scope_conflict".into());
                    }
                }
                "agent_assignments" => {
                    t.only(row, "id", &self.assignment)?;
                    t.only(row, "child_run_id", &self.run)?;
                    t.only(row, "coordinator_run_id", &lease.root_run_id)?;
                    t.only(row, "target_key", &lease.target_key)?;
                    t.only(row, "role", "client_side")?;
                    t.only(row, "lane", "read_only_analysis")?;
                    t.only(row, "fencing_token", &lease.fencing_token)?;
                    t.only(row, "state", "running")?;
                    if t.number(row, "lease_epoch")? != lease.lease_epoch
                        || t.number(row, "reserved_tokens")? != 8000
                        || t.number(row, "reserved_requests")? != 1
                    {
                        return Err("client_side_grant_row_scope_conflict".into());
                    }
                }
                "agent_assignment_attempts" => {
                    t.only(row, "id", worker)?;
                    t.only(row, "root_run_id", &lease.root_run_id)?;
                    t.only(row, "assignment_id", &self.assignment)?;
                    t.only(row, "child_run_id", &self.run)?;
                    t.only(row, "coordinator_fencing_token", &lease.fencing_token)?;
                    t.only(row, "state", "running")?;
                    if t.number(row, "coordinator_epoch")? != lease.lease_epoch
                        || t.number(row, "lease_epoch")? != 1
                    {
                        return Err("client_side_grant_row_scope_conflict".into());
                    }
                }
                "agent_capability_leases" => {
                    t.only(row, "assignment_id", &self.assignment)?;
                    t.only(row, "child_run_id", &self.run)?;
                    t.only(row, "root_run_id", &lease.root_run_id)?;
                    t.only(row, "fencing_token", &lease.fencing_token)?;
                    t.only(row, "revoked_at", "")?;
                    if t.number(row, "lease_epoch")? != lease.lease_epoch
                        || !matches!(
                            t.text(row, "capability")?,
                            "evidence.read" | "mailbox.write"
                        )
                    {
                        return Err("client_side_grant_row_scope_conflict".into());
                    }
                }
                "agent_lane_leases" => {
                    t.only(row, "assignment_id", &self.assignment)?;
                    t.only(row, "scan_id", &lease.scan_id)?;
                    t.only(row, "target_key", &lease.target_key)?;
                    t.only(row, "lane", "read_only_analysis")?;
                    if t.number(row, "attempt_number")? != lease.attempt_number {
                        return Err("client_side_grant_row_scope_conflict".into());
                    }
                }
                "agent_contract_owners" => {
                    t.only(row, "root_run_id", &lease.root_run_id)?;
                    t.only(row, "assignment_id", &self.assignment)?;
                    t.only(row, "fencing_token", &lease.fencing_token)?;
                    t.only(row, "state", "held")?;
                    t.only(
                        row,
                        "contract_key",
                        &crate::agent_runtime::multi_agent::contract_owner::schedule_contract_key(
                            lease.attempt_number,
                            &lease.target_key,
                            "client_side",
                            super::TRIGGER,
                            1,
                        ),
                    )?;
                    t.only(row, "released_at", "")?;
                    t.only(row, "result_node_id", "")?;
                    if t.number(row, "lease_epoch")? != lease.lease_epoch {
                        return Err("client_side_grant_row_scope_conflict".into());
                    }
                }
                "agent_budget_entries" => {}
                "agent_collaboration_events" => {
                    t.only(row, "scan_id", &lease.scan_id)?;
                    if t.number(row, "attempt_number")? != lease.attempt_number {
                        return Err("client_side_grant_event_scope_conflict".into());
                    }
                    let payload: JsonValue = serde_json::from_str(t.text(row, "payload_json")?)
                        .map_err(|_| "client_side_grant_event_scope_conflict")?;
                    let allowed=match (t.text(row,"entity_type")?,t.text(row,"entity_id")?) {
                        ("agent_run",id) if id==self.run=>t.text(row,"event_type")?=="agent_run" && ["prepared","running"].into_iter().any(|status|payload==json!({"role":"client_side","status":status,"terminalState":""})),
                        ("assignment",id) if id==self.assignment=>t.text(row,"event_type")?=="assignment" && ["prepared","leased","running"].into_iter().any(|state|payload==json!({"role":"client_side","state":state})),
                        _=>false,
                    };
                    let state = if t.text(row, "entity_type")? == "agent_run" {
                        payload["status"].as_str()
                    } else {
                        payload["state"].as_str()
                    }
                    .ok_or("client_side_grant_event_scope_conflict")?;
                    if !allowed
                        || !event_states
                            .insert((t.text(row, "entity_type")?.to_string(), state.to_string()))
                    {
                        return Err("client_side_grant_event_scope_conflict".into());
                    }
                }
                _ => return Err("client_side_grant_write_set_conflict".into()),
            }
        }
        if t.name == "agent_budget_entries" {
            self.costs
                .verify(db, t, rows, lease, &self.assignment, worker)?;
        }
        if t.name == "agent_collaboration_events" && rows.len() != 5 {
            return Err("client_side_grant_event_count_conflict".into());
        }
        Ok(())
    }
}
fn schema(db: &Connection) -> Result<String, String> {
    let mut text = String::new();
    for schema in ["sqlite_master", "sqlite_temp_master"] {
        let mut q = db
            .prepare(&format!(
                "SELECT type,name,tbl_name,sql FROM {schema} ORDER BY type,name"
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
        text.push_str(&format!("{schema}:{rows:?}"));
    }
    Ok(store::stable_hash(&text))
}
