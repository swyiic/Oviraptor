//! Pure original financial identities. Expired/replaced C never becomes live.
use crate::agent_runtime::multi_agent::{budget::root::RootOwner, lease::CoordinatorLease};
use rusqlite::Connection;

pub(super) fn original(
    db: &Connection,
    c: &CoordinatorLease,
) -> Result<(String, String, String), String> {
    let root_owner: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_root_budget_attempts WHERE root_run_id=?1)",
            [&c.root_run_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if root_owner {
        let owner = RootOwner::load_original(db, &c.root_run_id)?;
        owner.require_original_coordinator(db, c)?;
        return Ok(("root_control".into(), owner.id, String::new()));
    }
    Err("budget_original_worker_financial_proof_incomplete".into())
}

pub(super) fn binding(
    db: &Connection,
    c: &CoordinatorLease,
    owner: &(String, String, String),
) -> Result<String, String> {
    let header:String=db.query_row("SELECT scan_id,attempt_number,target_url,plan_hash,plan_json,created_at,
        hard_token_budget,hard_request_budget,orchestration_policy,root_run_id,backend,role,assignment_id,parent_run_id
        FROM agent_runs WHERE id=?1",[&c.root_run_id],|r| {
        let values=(0..14).map(|i|r.get::<_,rusqlite::types::Value>(i)).collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(format!("{values:?}"))
    }).map_err(|e|e.to_string())?;
    Ok(crate::agent_runtime::store::stable_hash(&serde_json::json!({
        "header":header,"owner":owner,"root":c.root_run_id,"scan":c.scan_id,"attempt":c.attempt_number,
        "target":c.target_key,"epoch":c.lease_epoch,"fence":c.fencing_token,
    }).to_string()))
}
