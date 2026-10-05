//! Current ledger observations at original call admission; saved calls reuse
//! their paid observation. No fresh observation turns a replay into a new call.
use super::Tick;
use crate::agent_runtime::{
    multi_agent::budget::{self, root::RootOwner},
    store,
};
use rusqlite::{params, OptionalExtension, Transaction};
use serde_json::{json, Value};
impl Tick {
    pub(crate) fn basis_hash(basis: &Value) -> String {
        let mut key = basis.clone();
        if let Some(object) = key
            .as_object_mut()
            .filter(|o| o.contains_key("liveBudgetObservation"))
        {
            object.remove("budgetSnapshot");
        }
        store::stable_hash(&key.to_string())
    }
    pub(crate) fn with_budget_snapshot(
        tx: &Transaction<'_>,
        root: &str,
        basis: &Value,
    ) -> Result<Value, String> {
        let Some(contract) = basis.get("liveBudgetObservation") else {
            return Ok(basis.clone());
        };
        let owner = RootOwner::load_original(tx, root)?;
        if owner.contract["root"]["liveBudgetObservation"] != *contract
            || basis.get("budgetSnapshot").is_some()
        {
            return Err("root_budget_observation_original_contract_conflict".into());
        }
        let key = Self::basis_hash(basis);
        let saved:Option<String>=tx.query_row("SELECT fact_json FROM agent_root_tick_receipts WHERE root_run_id=?1 AND phase='request' AND basis_hash=?2",params![root,key],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
        if let Some(text) = saved {
            let fact: Value = serde_json::from_str(&text)
                .map_err(|_| "root_budget_observation_receipt_invalid")?;
            let mut original = fact["request"]["basis"].clone();
            let observation = original
                .as_object_mut()
                .ok_or("root_budget_observation_receipt_invalid")?
                .remove("budgetSnapshot")
                .ok_or("root_budget_observation_receipt_invalid")?;
            let canonical = fact.to_string();
            if canonical != text || original != *basis || observation.is_null() {
                return Err("root_budget_observation_receipt_invalid".into());
            }
            let mut captured = original;
            captured["budgetSnapshot"] = observation;
            // Tick::begin verifies the complete original hash, paid fee and
            // physical request before it can return a saved decision.
            return Ok(captured);
        }
        owner.require_executable(tx)?;
        let (tokens, requests) = budget::root::remaining_child_capacity(tx, root)?;
        let mut dimensions = serde_json::Map::new();
        for dimension in budget::DIMENSIONS {
            let hard:Option<i64>=tx.query_row("SELECT hard_limit FROM agent_budget_limits WHERE root_run_id=?1 AND dimension=?2",params![root,dimension],|r|r.get(0)).map_err(|e|e.to_string())?;
            let b = budget::balance(tx, root, None, dimension)?;
            let occupied = b
                .consumed
                .checked_add(b.reserved)
                .and_then(|n| n.checked_add(b.indeterminate))
                .ok_or("budget_amount_overflow")?;
            let remaining = hard
                .map(|limit| {
                    limit
                        .checked_sub(occupied)
                        .filter(|n| *n >= 0)
                        .ok_or("budget_hard_limit_exceeded")
                })
                .transpose()?;
            dimensions.insert(dimension.into(),json!({"hardLimit":hard,"consumed":b.consumed,"reserved":b.reserved,"indeterminate":b.indeterminate,"remaining":remaining}));
        }
        let (count,last):(i64,i64)=tx.query_row("SELECT count(*),coalesce(max(rowid),0) FROM agent_budget_entries WHERE root_run_id=?1",[root],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
        let mut captured = basis.clone();
        captured["budgetSnapshot"] = json!({"schemaVersion":1,"timing":"before_this_model_call","executionGrant":false,
            "root":root,"financialOwner":owner.id,"ledgerEntryCount":count,"lastLedgerRow":last,
            "remainingModelTokens":tokens,"remainingModelRequests":requests,"dimensions":dimensions,
            "wallTimeRemainingMs":budget::clock::remaining(tx,root)?.as_millis(),
            "interpretation":"captured observation only; Rust rechecks current ledger for every admission"});
        Ok(captured)
    }
}
