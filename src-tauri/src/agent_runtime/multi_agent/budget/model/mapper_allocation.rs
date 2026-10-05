//! Frozen first-worker Mapper invoices pin allocation across live-balance changes.
use crate::agent_runtime::multi_agent::{attempts, lease::CoordinatorLease};
use rusqlite::{params, Connection};

pub(crate) fn original_mapper_grant(
    db: &Connection,
    lease: &CoordinatorLease,
    assignment: &str,
) -> Result<i64, String> {
    require_original_invoice_guards(db)?;
    let worker = attempts::current(db, lease, assignment)?;
    if worker.lease_epoch != 1 {
        return Err("mapper_allocation_original_worker_conflict".into());
    }
    let (held_tokens, held_requests, state, settled): (i64, i64, String, String) = db.query_row("SELECT reserved_tokens,reserved_requests,state,budget_settled_at FROM agent_assignments WHERE id=?1 AND coordinator_run_id=?2 AND role='spa_api_mapper'", params![assignment,lease.root_run_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|_| "mapper_allocation_original_grant_conflict")?;
    let mut q = db.prepare("SELECT dimension,amount,idempotency_key,source_id FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND kind='reserve' AND dimension IN ('model_input_tokens','model_cached_tokens','model_output_tokens','model_requests') ORDER BY dimension").map_err(|e|e.to_string())?;
    let rows = q
        .query_map(params![lease.root_run_id, assignment, worker.id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let tokens = rows
        .iter()
        .find(|row| row.0 == "model_input_tokens")
        .map(|row| row.1)
        .ok_or("mapper_allocation_original_grant_conflict")?;
    let intact_projection =
        state == "running" && held_tokens == tokens && held_requests == 1 && settled.is_empty()
            || matches!(state.as_str(), "completed" | "paused")
                && held_tokens == 0
                && held_requests == 0
                && !settled.is_empty();
    if !intact_projection
        || !(1..=8_000).contains(&tokens)
        || rows.len() != 4
        || rows.iter().any(|(dimension, amount, key, source)| {
            *amount
                != if dimension == "model_requests" {
                    1
                } else {
                    tokens
                }
                || key != &format!("reserve:{assignment}:{dimension}")
                || source != &format!("assignment:{assignment}")
        })
    {
        return Err("mapper_allocation_original_grant_conflict".into());
    }
    Ok(tokens)
}

pub(super) fn require_original_invoice_guards(db: &Connection) -> Result<(), String> {
    // Do not accept mutable/injected invoices as original allocation proof.
    for (name, suffix) in [
        ("budget_entry_no_update", "BEFORE UPDATE ON agent_budget_entries BEGIN SELECT RAISE(ABORT,'budget_entry_immutable'); END"),
        ("budget_entry_no_delete", "BEFORE DELETE ON agent_budget_entries BEGIN SELECT RAISE(ABORT,'budget_entry_immutable'); END"),
        ("budget_entry_no_replace", "BEFORE INSERT ON agent_budget_entries WHEN EXISTS(SELECT 1 FROM agent_budget_entries WHERE entry_id=NEW.entry_id OR (root_run_id=NEW.root_run_id AND idempotency_key=NEW.idempotency_key)) BEGIN SELECT RAISE(ABORT,'budget_entry_immutable'); END"),
    ] {
        let actual: String = db.query_row("SELECT sql FROM main.sqlite_master WHERE type='trigger' AND name=?1", [name], |r| r.get(0)).map_err(|_| "mapper_allocation_invoice_guard_conflict")?;
        let normalize = |s: &str| s.trim().trim_end_matches(';').replace(" IF NOT EXISTS", "").split_whitespace().collect::<Vec<_>>().join(" ");
        if normalize(&actual) != normalize(&format!("CREATE TRIGGER {name} {suffix}")) {
            return Err("mapper_allocation_invoice_guard_conflict".into());
        }
    }
    Ok(())
}
