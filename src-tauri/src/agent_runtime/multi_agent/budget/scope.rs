//! Worker accounting namespace only; callers prove execution or receipt rights.
//! First-worker Native keys stay unchanged. Later workers cannot replay them.
use super::{entries, Balance};
use crate::agent_runtime::multi_agent::{attempts, lease::CoordinatorLease};
use rusqlite::{params, Connection};

pub(super) fn stored_key(
    db: &Connection,
    root: &str,
    assignment: &str,
    attempt: &str,
    key: &str,
) -> Result<String, String> {
    if assignment.is_empty() {
        return super::root::stored_key(db, root, attempt, key);
    }
    let ordinal: i64 = db
        .query_row(
            "SELECT lease_epoch FROM agent_assignment_attempts
             WHERE id=?1 AND root_run_id=?2 AND assignment_id=?3",
            params![attempt, root, assignment],
            |r| r.get(0),
        )
        .map_err(|_| "budget_attempt_scope_conflict")?;
    if ordinal <= 0 || uuid::Uuid::parse_str(attempt).is_err() || key.trim().is_empty() {
        return Err("budget_attempt_scope_conflict".into());
    }
    let prefix = format!("worker:{attempt}:");
    if key.starts_with("worker:") {
        if ordinal > 1 && key.starts_with(&prefix) && key.len() > prefix.len() {
            return Ok(key.into());
        }
        return Err("budget_attempt_key_scope_conflict".into());
    }
    Ok(if ordinal == 1 {
        key.into()
    } else {
        format!("{prefix}{key}")
    })
}

pub(super) fn current_balance(
    db: &Connection,
    lease: &CoordinatorLease,
    assignment: &str,
    dimension: &str,
) -> Result<Balance, String> {
    let worker = attempts::current(db, lease, assignment)?;
    entries::balance_for_attempt(db, &lease.root_run_id, assignment, &worker.id, dimension)
}
