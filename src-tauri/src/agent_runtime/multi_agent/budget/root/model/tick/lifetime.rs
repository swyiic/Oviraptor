//! Pure original dispatch audit and existing OS inode probe; no new work or cost.
use super::*;
use crate::agent_runtime::{execution_owner, multi_agent::lease::CoordinatorLease};
pub(crate) const INVOCATION_KIND: &str = "root-decision-sdk";

fn original_requests(db: &Connection, actor: &CoordinatorLease) -> Result<Vec<Tick>, String> {
    if db.is_autocommit() {
        return Err("root_decision_lifetime_transaction_required".into());
    }

    let mut q = db
        .prepare(
            "SELECT call_id,lease_attempt_id,round,request_hash,receipt_json
        FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='dispatch' ORDER BY round",
        )
        .map_err(|e| e.to_string())?;
    let rows = q
        .query_map([&actor.root_run_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let calls = rows
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    let requests: i64 = db.query_row("SELECT count(*) FROM agent_root_tick_receipts WHERE root_run_id=?1 AND phase='request'",
        [&actor.root_run_id], |r|r.get(0)).map_err(|e|e.to_string())?;
    if calls.is_empty() && requests == 0 {
        return Ok(vec![]);
    }
    if usize::try_from(requests).ok() != Some(calls.len()) {
        return Err("root_decision_lifetime_original_request_missing".into());
    }
    let owner = RootOwner::load_original(db, &actor.root_run_id)?;
    owner.require_original_coordinator(db, actor)?;
    let mut ticks = vec![];
    for (id, owner_id, round, hash, dispatch_raw) in calls {
        let raw: String = db
            .query_row(
                "SELECT fact_json FROM agent_root_tick_receipts WHERE call_id=?1 AND root_run_id=?2
            AND lease_attempt_id=?3 AND round=?4 AND request_hash=?5 AND phase='request'",
                params![id, actor.root_run_id, owner_id, round, hash],
                |r| r.get(0),
            )
            .map_err(|_| "root_decision_lifetime_original_request_missing")?;
        let value: Value = serde_json::from_str(&raw)
            .map_err(|_| "root_decision_lifetime_original_request_invalid")?;
        let request = &value["request"];
        let dispatch: Value = serde_json::from_str(&dispatch_raw)
            .map_err(|_| "root_decision_lifetime_original_dispatch_invalid")?;
        let tokens = dispatch["reservedTokens"]
            .as_i64()
            .filter(|n| *n > 0)
            .ok_or("root_decision_lifetime_original_dispatch_invalid")?;
        let canonical = value.to_string();
        if canonical != raw
            || owner_id != owner.id
            || round <= 0
            || value["version"] != 1
            || !matches!(request["version"].as_i64(), Some(1..=3))
            || request["owner"] != owner.contract
            || store::stable_hash(&request.to_string()) != hash
            || id != store::stable_hash(
                &json!({"owner":owner.id,"root":actor.root_run_id,"round":round,"request":hash})
                    .to_string(),
            )
        {
            return Err("root_decision_lifetime_original_binding_conflict".into());
        }
        let tick = Tick {
            call: RootModelCall {
                owner: owner.clone(),
                id,
                round,
                request: hash,
                tokens,
                remaining: Duration::ZERO,
            },
            basis: Tick::basis_hash(&request["basis"]),
            request_fact: request.clone(),
            dispatch_proof: value["dispatchProof"].clone(),
        };
        tick.verify_request(db)?;
        ticks.push(tick);
    }
    Ok(ticks)
}
pub(crate) fn verify_closed_original(
    db: &Connection,
    actor: &CoordinatorLease,
) -> Result<(), String> {
    let ticks = original_requests(db, actor)?;
    if ticks.is_empty() {
        return Err("root_decision_original_paid_request_required".into());
    }
    for tick in ticks {
        let saved = tick
            .load_decision(db)?
            .ok_or("root_decision_original_paid_decision_required")?;
        tick.verify_paid(db, &saved)?;
        if saved.unknown || tick.published(db, &saved)?.is_none() {
            return Err("root_decision_original_publication_required".into());
        }
    }
    Ok(())
}
pub(crate) fn require_idle_original(
    db: &Connection,
    actor: &CoordinatorLease,
) -> Result<Option<execution_owner::NativeInvocationOwner>, String> {
    if original_requests(db, actor)?.is_empty() {
        return Ok(None);
    }
    let path = db
        .path()
        .filter(|p| !p.is_empty())
        .ok_or("root_decision_lifetime_database_missing")?;
    execution_owner::probe_native_invocation(
        std::path::Path::new(path),
        &actor.scan_id,
        actor.attempt_number,
        INVOCATION_KIND,
        &actor.root_run_id,
    )
    .map_err(|e| format!("root_decision_transport_not_idle:{e}"))?
    .map(Some)
    .ok_or("root_decision_transport_original_exit_proof_missing".into())
}
