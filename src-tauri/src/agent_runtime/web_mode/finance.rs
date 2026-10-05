//! Private same-INSERT financial proof; neither zero usage nor mode is a grant.
use super::{root::NewRootModeDeclaration, WebMode};
use crate::agent_runtime::{
    multi_agent::{
        attempts::audit_rows::Rows,
        budget::root::RootOwner,
        lease::{self, CoordinatorLease},
    },
    store,
};
use rusqlite::{params, Transaction};

const TABLES: [&str; 10] = [
    "agent_coordinator_leases",
    "agent_root_budget_attempts",
    "agent_budget_limits",
    "agent_budget_clock_origins",
    "agent_budget_entries",
    "agent_root_model_journal",
    "agent_specialist_calls",
    "agent_source_model_rounds",
    "agent_budget_ledger",
    "agent_http_request_claims",
];

pub(super) struct FinanceCreation {
    old: Vec<Rows>,
    new: Option<(String, Vec<Rows>)>,
    mode: WebMode,
}
impl FinanceCreation {
    pub(super) fn capture(
        tx: &Transaction<'_>,
        mode: &NewRootModeDeclaration,
    ) -> Result<Self, String> {
        let occupied: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_coordinator_leases
            WHERE scan_id=?1 AND attempt_number=?2 AND target_key=?3)",
                params![mode.fact.scan_id, mode.fact.attempt_number, mode.target_url],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if occupied {
            return Err("web_finance_requires_new_root_and_scope".into());
        }
        let old = TABLES
            .iter()
            .map(|table| {
                Rows::read(
                    tx,
                    &format!("SELECT rowid,* FROM {table} ORDER BY rowid"),
                    [],
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            old,
            new: None,
            mode: mode.mode(),
        })
    }
    pub(super) fn publish(
        &mut self,
        fresh: &store::NewlyInsertedNativeRoot<'_, '_>,
        mode: &NewRootModeDeclaration,
    ) -> Result<(), String> {
        if self.new.is_some() || self.mode != mode.mode() {
            return Err("web_finance_birth_proof_reused".into());
        }
        let tx = fresh.transaction();
        let root = fresh.id();
        let frozen = super::root::read(tx, root)?.ok_or("web_finance_private_mode_missing")?;
        if !frozen.same_binding(mode) {
            return Err("web_finance_private_mode_changed".into());
        }
        let history: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_root_budget_attempts WHERE root_run_id=?1)
            OR EXISTS(SELECT 1 FROM agent_budget_limits WHERE root_run_id=?1)
            OR EXISTS(SELECT 1 FROM agent_budget_clock_origins WHERE root_run_id=?1)
            OR EXISTS(SELECT 1 FROM agent_budget_entries WHERE root_run_id=?1)
            OR EXISTS(SELECT 1 FROM agent_assignments WHERE coordinator_run_id=?1)
            OR EXISTS(SELECT 1 FROM agent_coordinator_leases WHERE root_run_id=?1)
            OR EXISTS(SELECT 1 FROM agent_root_model_journal WHERE root_run_id=?1)
            OR EXISTS(SELECT 1 FROM agent_specialist_calls WHERE root_run_id=?1)
            OR EXISTS(SELECT 1 FROM agent_source_model_rounds WHERE root_run_id=?1)
            OR EXISTS(SELECT 1 FROM agent_http_request_claims WHERE run_id=?1)
            OR EXISTS(SELECT 1 FROM agent_budget_ledger WHERE root_run_id=?1)",
                [root],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if history {
            return Err("web_finance_requires_new_root".into());
        }
        lease::require_active_attempt(tx, &mode.fact.scan_id, mode.fact.attempt_number)?;
        let actor = if mode.mode() == WebMode::Multi {
            let (origin, text): (String, String) = tx
                .query_row(
                    "SELECT created_at,plan_json FROM agent_runs WHERE id=?1",
                    [root],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(|e| e.to_string())?;
            let plan: serde_json::Value =
                serde_json::from_str(&text).map_err(|_| "web_finance_native_plan_invalid")?;
            let seconds = plan["timeoutSeconds"]
                .as_i64()
                .filter(|v| *v > 0)
                .ok_or("web_finance_native_timeout_invalid")?;
            let expiry: String = tx
                .query_row(
                    "SELECT min(datetime('now','+600 seconds','localtime'),
                datetime(?1,printf('+%d seconds',?2)))",
                    params![origin, seconds],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            let c = CoordinatorLease {
                scan_id: mode.fact.scan_id.clone(),
                attempt_number: mode.fact.attempt_number,
                target_key: mode.target_url.clone(),
                root_run_id: root.into(),
                lease_epoch: 1,
                fencing_token: uuid::Uuid::new_v4().to_string(),
                lease_expires_at: expiry,
            };
            let count=tx.execute("INSERT INTO agent_coordinator_leases(scan_id,attempt_number,target_key,root_run_id,
                lease_epoch,fencing_token,lease_expires_at,heartbeat_at) VALUES(?1,?2,?3,?4,1,?5,?6,datetime('now','localtime'))",
                params![c.scan_id,c.attempt_number,c.target_key,c.root_run_id,c.fencing_token,c.lease_expires_at]).map_err(|e|e.to_string())?;
            if count != 1 {
                return Err("web_finance_first_coordinator_unconfirmed".into());
            }
            lease::validate_coordinator_lease(tx, &c)?;
            Some(c)
        } else {
            None
        };
        // Raw born initializer keeps the creator's single authorizer installed.
        let owner = RootOwner::initialize_new_native_root(fresh)?;
        if let Some(actor) = &actor {
            owner.require_original_coordinator(tx, actor)?;
        }
        owner.require_executable(tx)?;
        let new = TABLES[..4]
            .iter()
            .map(|table| {
                Rows::read(
                    tx,
                    &format!("SELECT rowid,* FROM {table} WHERE root_run_id=?1 ORDER BY rowid"),
                    [root],
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        if new[0].values.len() != usize::from(self.mode == WebMode::Multi)
            || new[1].values.len() != 1
            || new[2].values.len() != 10
            || new[3].values.len() != 1
        {
            return Err("web_finance_original_creation_unconfirmed".into());
        }
        self.new = Some((root.into(), new));
        Ok(())
    }
    pub(super) fn verify(&self, tx: &Transaction<'_>) -> Result<(), String> {
        let (root, new) = self
            .new
            .as_ref()
            .ok_or("web_finance_original_creation_missing")?;
        for (index, (table, old)) in TABLES.iter().zip(&self.old).enumerate() {
            let column = if *table == "agent_http_request_claims" {
                "run_id"
            } else {
                "root_run_id"
            };
            if Rows::read(
                tx,
                &format!("SELECT rowid,* FROM {table} WHERE {column}<>?1 ORDER BY rowid"),
                [root],
            )? != *old
            {
                return Err("web_finance_historical_rows_changed".into());
            }
            let rows = Rows::read(
                tx,
                &format!("SELECT rowid,* FROM {table} WHERE {column}=?1 ORDER BY rowid"),
                [root],
            )?;
            if index < 4 {
                if rows != new[index] {
                    return Err("web_finance_original_creation_changed".into());
                }
            } else if !rows.values.is_empty() {
                return Err("web_finance_creation_dispatched_work".into());
            }
        }
        RootOwner::load_original(tx, root)?.require_executable(tx)
    }
    pub(super) fn expected_writes(&self) -> u64 {
        12 + u64::from(self.mode == WebMode::Multi)
    }
}
