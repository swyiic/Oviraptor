//! Operator-set Hunter spending envelope for the built-in agent, plus the hard
//! gate that admits an agent-initiated Hunter expansion.
//!
//! The money wall for the agent has three independent layers:
//!   1. The envelope (this module): the operator, and only the operator, turns
//!      `allow` on, sets a points `budget`, and records the Hunter `account_hash`
//!      the budget was authorized for. The agent has no tool that can write it.
//!   2. The gate (`hunter_gate`, pure + unit-tested): every start must satisfy
//!      allow ∧ key-present ∧ account-match ∧ within-remaining-budget ∧ idle.
//!   3. The run cap: the background pagination is handed a `stopped` closure that
//!      halts it once this expansion's spend reaches the per-run cap, so spend
//!      can never run past the operator's remaining budget.
//! The agent never confirms scope and never widens it; Hunter targets are still
//! gated to confirmed seeds by `hunter_expansion::evidence_anchor`.
use super::*;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS agent_hunter_envelope(
 project_id INTEGER PRIMARY KEY REFERENCES projects(id),
 allow INTEGER NOT NULL DEFAULT 0,
 budget INTEGER NOT NULL DEFAULT 0,
 account_hash TEXT NOT NULL DEFAULT '',
 updated_at INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS agent_hunter_runs(
 project_id INTEGER NOT NULL,
 query_id TEXT NOT NULL,
 requested_max INTEGER NOT NULL DEFAULT 0,
 started_at INTEGER NOT NULL DEFAULT 0,
 PRIMARY KEY(project_id, query_id));";

pub(crate) fn ensure(c: &Connection) -> Result<(), String> {
    c.execute_batch(SCHEMA).map_err(err)
}

/// The operator-authorized envelope: (allow, budget_points, authorized_account_hash).
pub(crate) fn read_envelope(c: &Connection, p: i64) -> Result<(bool, i64, String), String> {
    ensure(c)?;
    Ok(c
        .query_row(
            "SELECT allow, budget, account_hash FROM agent_hunter_envelope WHERE project_id=?1",
            [p],
            |r| Ok((r.get::<_, i64>(0)? != 0, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(err)?
        .unwrap_or((false, 0, String::new())))
}

/// Points the agent has already caused to be spent on Hunter for this project.
/// Whole-expansion attribution: every expansion the agent started counts in full,
/// so the running total is never an under-count of the agent's spend.
pub(crate) fn agent_spent(c: &Connection, p: i64) -> Result<i64, String> {
    let has: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='hunter_expansions')",
            [],
            |r| r.get(0),
        )
        .map_err(err)?;
    if !has {
        return Ok(0);
    }
    c.query_row(
        "SELECT COALESCE(SUM(h.spent),0) FROM agent_hunter_runs r JOIN hunter_expansions h ON h.id=r.query_id AND h.project_id=r.project_id WHERE r.project_id=?1",
        [p],
        |r| r.get(0),
    )
    .map_err(err)
}

/// True when the project has no queued/running asset job and no live Hunter
/// pagination — the same one-writer-per-project rule the job and Hunter paths use.
pub(crate) fn project_idle(c: &Connection, p: i64) -> Result<bool, String> {
    let job_busy: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM runs WHERE project_id=?1 AND status IN ('queued','running','cancel_requested'))",
            [p],
            |r| r.get(0),
        )
        .optional()
        .map_err(err)?
        .unwrap_or(false);
    if job_busy {
        return Ok(false);
    }
    let hunter_busy: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='hunter_expansions')",
            [],
            |r| r.get(0),
        )
        .map_err(err)?
        && c.query_row(
            "SELECT EXISTS(SELECT 1 FROM hunter_expansions WHERE project_id=?1 AND status IN ('running','stopping'))",
            [p],
            |r| r.get(0),
        )
        .map_err(err)?;
    Ok(!hunter_busy)
}

/// Record that the agent started this Hunter expansion, so its spend counts
/// against the operator budget on the next gate evaluation.
pub(crate) fn record_run(c: &Connection, p: i64, query_id: &str, requested_max: i64) -> Result<(), String> {
    ensure(c)?;
    c.execute(
        "INSERT INTO agent_hunter_runs(project_id,query_id,requested_max,started_at) VALUES(?1,?2,?3,strftime('%s','now'))
         ON CONFLICT(project_id,query_id) DO UPDATE SET requested_max=excluded.requested_max,started_at=excluded.started_at",
        params![p, query_id, requested_max],
    )
    .map_err(err)?;
    Ok(())
}

/// The operator-only envelope setter. Records allow + budget + the account hash
/// of the currently saved Hunter key, so a later key/account change fails the
/// gate instead of spending a different account's points.
pub(crate) fn set_envelope_on(c: &Connection, p: i64, allow: bool, budget: i64, key: &str) -> Result<Value, String> {
    project(c, p)?;
    ensure(c)?;
    if budget < 0 || budget > 100_000_000 {
        return Err("Hunter 预算积分无效（0 到 1 亿之间）".into());
    }
    let account_hash = if key.is_empty() {
        String::new()
    } else {
        format!("{:x}", Sha256::digest(key.as_bytes()))
    };
    c.execute(
        "INSERT INTO agent_hunter_envelope(project_id,allow,budget,account_hash,updated_at) VALUES(?1,?2,?3,?4,strftime('%s','now'))
         ON CONFLICT(project_id) DO UPDATE SET allow=excluded.allow,budget=excluded.budget,account_hash=excluded.account_hash,updated_at=excluded.updated_at",
        params![p, allow as i64, budget, account_hash],
    )
    .map_err(err)?;
    envelope_status(c, p)
}

/// Read-only envelope view for the operator UI and the agent's status reads.
/// Never returns the raw key or account id, only whether an account is bound and
/// how much budget remains.
pub(crate) fn envelope_status(c: &Connection, p: i64) -> Result<Value, String> {
    let (allow, budget, account_hash) = read_envelope(c, p)?;
    let spent = agent_spent(c, p)?;
    Ok(json!({
        "allow": allow,
        "budget": budget,
        "spent": spent,
        "remaining": (budget - spent).max(0),
        "accountBound": !account_hash.is_empty(),
    }))
}

/// Pure admission gate. Returns the effective per-run spend cap in points when
/// every condition holds, or a stable deny code otherwise. No network, no DB.
///
/// `requested_max` over the remaining budget is denied (never silently capped),
/// so the agent must ask within budget; the returned cap is still clamped to the
/// remaining budget as a backstop for the run loop.
pub(crate) fn hunter_gate(
    allow: bool,
    key_present: bool,
    account_match: bool,
    budget: i64,
    already_spent: i64,
    requested_max: i64,
    idle: bool,
) -> Result<i64, &'static str> {
    if !allow {
        return Err("hunter_not_authorized");
    }
    if !key_present {
        return Err("hunter_key_missing");
    }
    if !account_match {
        return Err("hunter_account_changed");
    }
    if budget <= 0 {
        return Err("hunter_budget_unset");
    }
    let remaining = budget - already_spent.max(0);
    if remaining <= 0 {
        return Err("hunter_budget_exhausted");
    }
    if requested_max < 1 {
        return Err("hunter_points_invalid");
    }
    if requested_max > remaining {
        return Err("hunter_points_over_budget");
    }
    if !idle {
        return Err("hunter_project_busy");
    }
    Ok(requested_max.min(remaining))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnvelopeInput {
    pub project_id: i64,
    pub allow: bool,
    pub budget: i64,
}

/// Operator-only. Deliberately a desktop command, never an agent tool: the agent
/// has no way to turn `allow` on, set a budget, or rebind the account.
#[tauri::command]
pub async fn set_agent_hunter_envelope(
    state: tauri::State<'_, crate::AppState>,
    input: EnvelopeInput,
) -> Result<Value, String> {
    let path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let c = db::open(&path)?;
        let (_, key, _, _) = setting(&c)?;
        set_envelope_on(&c, input.project_id, input.allow, input.budget, &key)
    })
    .await
    .map_err(err)?
}

#[cfg(test)]
mod tests {
    use super::{agent_spent, ensure, hunter_gate, read_envelope, set_envelope_on};
    use rusqlite::Connection;
    use sha2::{Digest, Sha256};

    fn project_db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.pragma_update(None, "foreign_keys", "ON").unwrap();
        c.execute_batch(
            "CREATE TABLE projects(id INTEGER PRIMARY KEY, name TEXT, status TEXT NOT NULL DEFAULT 'active');
             INSERT INTO projects(id,status) VALUES(1,'active');",
        )
        .unwrap();
        c
    }

    #[test]
    fn with_no_envelope_the_defaults_deny() {
        let c = project_db();
        ensure(&c).unwrap();
        assert_eq!(read_envelope(&c, 1).unwrap(), (false, 0, String::new()));
        assert_eq!(agent_spent(&c, 1).unwrap(), 0);
    }

    #[test]
    fn the_envelope_binds_the_current_account_and_a_different_key_never_matches() {
        let c = project_db();
        // Operator turns the envelope on while key "K1" is the saved Hunter key.
        set_envelope_on(&c, 1, true, 1000, "K1").unwrap();
        let (allow, budget, account) = read_envelope(&c, 1).unwrap();
        assert!(allow);
        assert_eq!(budget, 1000);
        let k1 = format!("{:x}", Sha256::digest("K1".as_bytes()));
        let k2 = format!("{:x}", Sha256::digest("K2".as_bytes()));
        assert_eq!(account, k1, "the envelope records the authorized account hash");
        assert_ne!(account, k2, "a different key's account hash never matches");
        // With no key saved, the account binding is cleared, never left stale.
        set_envelope_on(&c, 1, true, 1000, "").unwrap();
        assert_eq!(read_envelope(&c, 1).unwrap().2, String::new());
    }

    #[test]
    fn a_negative_or_oversized_budget_is_refused() {
        let c = project_db();
        assert!(set_envelope_on(&c, 1, true, -1, "K1").is_err());
        assert!(set_envelope_on(&c, 1, true, 100_000_001, "K1").is_err());
    }

    #[test]
    fn no_allow_is_denied_before_anything_else() {
        assert_eq!(
            hunter_gate(false, true, true, 1000, 0, 100, true),
            Err("hunter_not_authorized")
        );
    }

    #[test]
    fn a_missing_or_changed_account_is_denied() {
        assert_eq!(
            hunter_gate(true, false, false, 1000, 0, 100, true),
            Err("hunter_key_missing")
        );
        // Key present but its account no longer matches the one the operator
        // authorized the budget for: never spend a different account's points.
        assert_eq!(
            hunter_gate(true, true, false, 1000, 0, 100, true),
            Err("hunter_account_changed")
        );
    }

    #[test]
    fn an_unset_or_exhausted_budget_is_denied() {
        assert_eq!(
            hunter_gate(true, true, true, 0, 0, 100, true),
            Err("hunter_budget_unset")
        );
        assert_eq!(
            hunter_gate(true, true, true, 500, 500, 100, true),
            Err("hunter_budget_exhausted")
        );
        assert_eq!(
            hunter_gate(true, true, true, 500, 600, 100, true),
            Err("hunter_budget_exhausted")
        );
    }

    #[test]
    fn a_request_over_the_remaining_budget_is_denied_not_capped() {
        // 800 already spent of 1000 → 200 remaining; a 300-point ask is refused.
        assert_eq!(
            hunter_gate(true, true, true, 1000, 800, 300, true),
            Err("hunter_points_over_budget")
        );
    }

    #[test]
    fn an_invalid_point_request_is_denied() {
        assert_eq!(
            hunter_gate(true, true, true, 1000, 0, 0, true),
            Err("hunter_points_invalid")
        );
    }

    #[test]
    fn a_busy_project_is_denied_even_when_funded() {
        assert_eq!(
            hunter_gate(true, true, true, 1000, 0, 100, false),
            Err("hunter_project_busy")
        );
    }

    #[test]
    fn the_happy_path_returns_the_requested_cap_within_budget() {
        // 200 remaining, 150 requested → cap 150.
        assert_eq!(hunter_gate(true, true, true, 1000, 800, 150, true), Ok(150));
        // Exactly the remaining budget is allowed.
        assert_eq!(hunter_gate(true, true, true, 1000, 800, 200, true), Ok(200));
    }
}
