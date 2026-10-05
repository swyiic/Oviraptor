//! Born-window financial authority; historical Root rows cannot construct it.
use crate::agent_runtime::multi_agent::{
    budget::root::RootOwner,
    lease::{self, CoordinatorLease},
};
use rusqlite::{params, Transaction};
use serde_json::Value;
mod writer;
pub(super) use writer::SourceCreationWriter;

// Constructor and fields are private to this module and its writer child.
struct FreshSourceRoot<'tx, 'db> {
    tx: &'tx Transaction<'db>,
    root: String,
    scan: String,
    attempt: i64,
    target: String,
    birth: crate::agent_runtime::store::SourceRootInsertion<'tx, 'db>,
}
impl FreshSourceRoot<'_, '_> {
    fn publish(&self) -> Result<(CoordinatorLease, String), String> {
        let tx = self.tx;
        let inserted = self.birth.inserted()?;
        lease::require_active_attempt(tx, &self.scan, self.attempt)?;
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_coordinator_leases
            WHERE (scan_id=?1 AND attempt_number=?2 AND target_key=?3) OR root_run_id=?4)
            OR EXISTS(SELECT 1 FROM agent_root_budget_attempts WHERE root_run_id=?4)
            OR EXISTS(SELECT 1 FROM agent_budget_limits WHERE root_run_id=?4)
            OR EXISTS(SELECT 1 FROM agent_budget_clock_origins WHERE root_run_id=?4)
            OR EXISTS(SELECT 1 FROM agent_budget_entries WHERE root_run_id=?4)
            OR EXISTS(SELECT 1 FROM agent_assignments WHERE coordinator_run_id=?4)
            OR EXISTS(SELECT 1 FROM agent_root_model_journal WHERE root_run_id=?4)
            OR EXISTS(SELECT 1 FROM agent_source_model_rounds WHERE root_run_id=?4)
            OR EXISTS(SELECT 1 FROM agent_specialist_calls WHERE root_run_id=?4)",
                params![self.scan, self.attempt, self.target, self.root],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if exists {
            return Err("source_finance_requires_new_root".into());
        }
        let (plan_text, origin): (String, String) = tx
            .query_row(
                "SELECT plan_json,created_at FROM agent_runs WHERE id=?1",
                [&self.root],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| e.to_string())?;
        let plan: Value =
            serde_json::from_str(&plan_text).map_err(|_| "source_finance_plan_invalid")?;
        crate::agent_runtime::multi_agent::source::SourcePhaseContract::from_plan(&plan)?;
        let canonical = plan.to_string();
        if plan["surface"] != "source" || plan_text != canonical {
            return Err("source_finance_plan_invalid".into());
        }
        let millis = plan["runtime"]["budget"]["timeoutSeconds"]
            .as_i64()
            .filter(|v| *v > 0)
            .and_then(|v| v.checked_mul(1000))
            .ok_or("source_finance_timeout_invalid")?;
        let expiry: String = tx
            .query_row(
                "SELECT min(datetime('now','+600 seconds','localtime'),
            datetime(?1,printf('+%f seconds',?2/1000.0)))",
                params![origin, millis],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let actor = CoordinatorLease {
            scan_id: self.scan.clone(),
            attempt_number: self.attempt,
            target_key: self.target.clone(),
            root_run_id: self.root.clone(),
            lease_epoch: 1,
            fencing_token: uuid::Uuid::new_v4().to_string(),
            lease_expires_at: expiry,
        };
        let inserted_rows = tx
            .execute(
                "INSERT INTO agent_coordinator_leases(scan_id,attempt_number,target_key,
            root_run_id,lease_epoch,fencing_token,lease_expires_at,heartbeat_at)
            VALUES(?1,?2,?3,?4,1,?5,?6,datetime('now','localtime'))",
                params![
                    actor.scan_id,
                    actor.attempt_number,
                    actor.target_key,
                    actor.root_run_id,
                    actor.fencing_token,
                    actor.lease_expires_at
                ],
            )
            .map_err(|e| e.to_string())?;
        if inserted_rows != 1 {
            return Err("source_finance_first_coordinator_unconfirmed".into());
        }
        lease::validate_coordinator_lease(tx, &actor)?;
        // Use the raw initializer under the one creator writer, never install
        // initialize_control's inner authorizer and erase the outer authority.
        let owner = RootOwner::initialize_new_source_root(&inserted)?;
        owner.require_original_coordinator(tx, &actor)?;
        let loaded = RootOwner::load_original(tx, &self.root)?;
        loaded.require_original_coordinator(tx, &actor)?;
        let id: String = tx
            .query_row(
                "SELECT id FROM agent_root_budget_attempts WHERE root_run_id=?1",
                [&self.root],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        Ok((actor, id))
    }
}

// Pure original financial identity read plus separate live publication check.
// Missing, expired or replaced original owners never issue another C/control.
pub(super) fn original_for_execution(
    db: &rusqlite::Connection,
    root: &str,
) -> Result<CoordinatorLease, String> {
    let actor = original_for_financial_exit(db, root)?;
    RootOwner::load_original(db, root)?.require_executable(db)?;
    lease::validate_coordinator_lease(db, &actor)?;
    lease::require_executable_coordinator(db, &actor)?;
    Ok(actor)
}

// Pure original identity: match the current stored C without requiring its
// TTL to be live. A replaced epoch/fence still fails; nothing is renewed.
pub(super) fn original_for_financial_exit(
    db: &rusqlite::Connection,
    root: &str,
) -> Result<CoordinatorLease, String> {
    let present: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_root_budget_attempts WHERE root_run_id=?1)",
            [root],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !present {
        return Err("source_root_original_finance_missing".into());
    }
    let owner = RootOwner::load_original(db, root)?;
    let actor=db.query_row("SELECT scan_id,attempt_number,target_key,root_run_id,lease_epoch,fencing_token,lease_expires_at
        FROM agent_coordinator_leases WHERE root_run_id=?1",[root],|r|Ok(CoordinatorLease {
            scan_id:r.get(0)?,attempt_number:r.get(1)?,target_key:r.get(2)?,root_run_id:r.get(3)?,
            lease_epoch:r.get(4)?,fencing_token:r.get(5)?,lease_expires_at:r.get(6)?,
        })).map_err(|_|"source_root_original_coordinator_missing")?;
    owner.require_original_coordinator(db, &actor)?;
    Ok(actor)
}

/// New work uses the original budget clock; this read never renews or mints.
pub(super) fn remaining_for_new_work(
    db: &rusqlite::Connection,
    actor: &CoordinatorLease,
) -> Result<std::time::Duration, String> {
    let owner = RootOwner::load_original(db, &actor.root_run_id)?;
    owner.require_original_coordinator(db, actor)?;
    owner.require_live(db)?;
    lease::validate_coordinator_lease(db, actor)?;
    lease::require_executable_coordinator(db, actor)?;
    crate::agent_runtime::multi_agent::budget::clock::remaining(db, &actor.root_run_id)
}
