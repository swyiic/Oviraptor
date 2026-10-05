//! One IMMEDIATE creator transaction, explicit 15 writes and old-row proof.
use super::FreshSourceRoot;
use crate::agent_runtime::multi_agent::{attempts::audit_rows::Rows, budget::root::RootOwner};
use rusqlite::{
    hooks::{AuthAction, AuthContext, Authorization},
    params,
    types::Value,
    Transaction,
};
use serde_json::{json, Value as Json};

const TABLES: [&str; 9] = [
    "agent_runs",
    "agent_coordinator_leases",
    "agent_root_budget_attempts",
    "agent_budget_limits",
    "agent_budget_clock_origins",
    "agent_budget_entries",
    "agent_root_budget_definitions",
    "agent_root_model_journal",
    "agent_specialist_calls",
];

pub(crate) struct SourceCreationWriter<'tx, 'db> {
    fresh: FreshSourceRoot<'tx, 'db>,
    old: Vec<Rows>,
    events: Rows,
    floor: i64,
    changes: u64,
    root_row: Option<Rows>,
    coordinator_row: Option<Rows>,
    owner_row: Option<Rows>,
    active: bool,
}
impl<'tx, 'db> SourceCreationWriter<'tx, 'db> {
    pub(crate) fn begin(
        tx: &'tx Transaction<'db>,
        root: &str,
        scan: &str,
        attempt: i64,
        target: &str,
    ) -> Result<Self, String> {
        let existing:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 OR
            (scan_id=?2 AND attempt_number=?3 AND role='coordinator' AND (target_url=?4 OR target_url LIKE 'source:%')))
            OR EXISTS(SELECT 1 FROM agent_coordinator_leases WHERE root_run_id=?1 OR
            (scan_id=?2 AND attempt_number=?3 AND (target_key=?4 OR target_key LIKE 'source:%')))",
            params![root,scan,attempt,target],|r|r.get(0)).map_err(|e|e.to_string())?;
        if existing {
            return Err("source_finance_requires_new_root".into());
        }
        let floor = tx
            .query_row(
                "SELECT COALESCE(MAX(sequence),0) FROM agent_collaboration_events",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let old = TABLES
            .iter()
            .map(|table| {
                Rows::read(
                    tx,
                    &format!("SELECT rowid,* FROM {table} ORDER BY rowid"),
                    [],
                )
            })
            .collect::<Result<_, _>>()?;
        let guard = Self {
            fresh: FreshSourceRoot {
                tx,
                root: root.into(),
                scan: scan.into(),
                attempt,
                target: target.into(),
                birth: crate::agent_runtime::store::SourceRootInsertion::capture_absent(
                    tx, root, scan, attempt, target,
                )?,
            },
            old,
            events: Rows::read(
                tx,
                "SELECT rowid,* FROM agent_collaboration_events WHERE sequence<=?1 ORDER BY rowid",
                [floor],
            )?,
            floor,
            changes: tx.total_changes(),
            root_row: None,
            coordinator_row: None,
            owner_row: None,
            active: true,
        };
        tx.authorizer(Some(authorize)).map_err(|e| e.to_string())?;
        crate::collaboration_events::source_creation_schema::verify(tx)?;
        Ok(guard)
    }
    pub(crate) fn publish_after_insert(&mut self) -> Result<(), String> {
        if self.root_row.is_some() {
            return Err("source_finance_fresh_proof_reused".into());
        }
        let f = &self.fresh;
        let pristine:bool=f.tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND scan_id=?2
            AND attempt_number=?3 AND target_url=?4 AND backend='native' AND role='coordinator' AND status='prepared'
            AND root_run_id=id AND parent_run_id IS NULL AND assignment_id='' AND lane='' AND orchestration_policy='multi'
            AND started_at='' AND finished_at='' AND heartbeat_at='' AND lease_expires_at='' AND cancel_requested_at=''
            AND used_tokens=0 AND used_cached_tokens=0 AND used_requests=0 AND reserved_tokens=0 AND reserved_requests=0
            AND capability_lease_json='[]' AND terminal_state='' AND terminal_code='' AND terminal_reason='')",
            params![f.root,f.scan,f.attempt,f.target],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !pristine {
            return Err("source_finance_requires_new_root".into());
        }
        self.root_row = Some(Rows::read(
            f.tx,
            "SELECT rowid,* FROM agent_runs WHERE id=?1",
            [&f.root],
        )?);
        let (actor, id) = f.publish()?;
        let c = Rows::read(
            f.tx,
            "SELECT rowid,* FROM agent_coordinator_leases WHERE root_run_id=?1",
            [&f.root],
        )?;
        let owner = Rows::read(
            f.tx,
            "SELECT rowid,* FROM agent_root_budget_attempts WHERE root_run_id=?1",
            [&f.root],
        )?;
        if c.values.len() != 1 || owner.values.len() != 1 || uuid::Uuid::parse_str(&id).is_err() {
            return Err("source_finance_initial_owner_unconfirmed".into());
        }
        RootOwner::load_original(f.tx, &f.root)?.require_original_coordinator(f.tx, &actor)?;
        self.coordinator_row = Some(c);
        self.owner_row = Some(owner);
        Ok(())
    }
    pub(crate) fn finish(mut self) -> Result<(), String> {
        let f = &self.fresh;
        if self.root_row.as_ref()
            != Some(&Rows::read(
                f.tx,
                "SELECT rowid,* FROM agent_runs WHERE id=?1",
                [&f.root],
            )?)
            || self.coordinator_row.as_ref()
                != Some(&Rows::read(
                    f.tx,
                    "SELECT rowid,* FROM agent_coordinator_leases WHERE root_run_id=?1",
                    [&f.root],
                )?)
            || self.owner_row.as_ref()
                != Some(&Rows::read(
                    f.tx,
                    "SELECT rowid,* FROM agent_root_budget_attempts WHERE root_run_id=?1",
                    [&f.root],
                )?)
        {
            return Err("source_finance_original_creation_changed".into());
        }
        for (table, old) in TABLES.iter().zip(&self.old) {
            let column = match *table {
                "agent_runs" => "id",
                _ => "root_run_id",
            };
            if Rows::read(
                f.tx,
                &format!("SELECT rowid,* FROM {table} WHERE {column}<>?1 ORDER BY rowid"),
                [&f.root],
            )? != *old
            {
                return Err("source_finance_old_rows_changed".into());
            }
        }
        let events = Rows::read(
            f.tx,
            "SELECT rowid,* FROM agent_collaboration_events WHERE sequence<=?1 ORDER BY rowid",
            [self.floor],
        )?;
        let new = Rows::read(
            f.tx,
            "SELECT scan_id,attempt_number,entity_type,entity_id,event_type,payload_json
            FROM agent_collaboration_events WHERE sequence>?1 ORDER BY sequence",
            [self.floor],
        )?;
        let event_ok = new.values.len() == 1
            && new.values[0][..5]
                == [
                    Value::Text(f.scan.clone()),
                    Value::Integer(f.attempt),
                    Value::Text("agent_run".into()),
                    Value::Text(f.root.clone()),
                    Value::Text("agent_run".into()),
                ]
            && matches!(&new.values[0][5],Value::Text(text) if serde_json::from_str::<Json>(text).ok()==Some(json!({"role":"coordinator","status":"prepared","terminalState":""})));
        if events != self.events
            || !event_ok
            || f.tx.total_changes().checked_sub(self.changes) != Some(15)
        {
            return Err("source_finance_creation_collateral_write".into());
        }
        let actor = super::original_for_execution(f.tx, &f.root)?;
        if actor.lease_epoch != 1 {
            return Err("source_finance_initial_epoch_conflict".into());
        }
        crate::collaboration_events::source_creation_schema::verify(f.tx)?;
        f.tx.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
            .map_err(|e| e.to_string())?;
        self.active = false;
        Ok(())
    }
}
impl Drop for SourceCreationWriter<'_, '_> {
    fn drop(&mut self) {
        if self.active {
            let _ = self
                .fresh
                .tx
                .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
        }
    }
}
fn authorize(c: AuthContext<'_>) -> Authorization {
    let allowed = match c.action {
        AuthAction::Insert { table_name }
            if c.database_name == Some("main") && c.accessor.is_none() =>
        {
            matches!(
                table_name,
                "agent_runs"
                    | "agent_coordinator_leases"
                    | "agent_root_budget_attempts"
                    | "agent_budget_limits"
                    | "agent_budget_clock_origins"
            )
        }
        AuthAction::Insert {
            table_name: "agent_collaboration_events",
        } if c.database_name == Some("main")
            && c.accessor == Some("agent_collaboration_run_insert") =>
        {
            true
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
