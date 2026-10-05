//! Persist one append-only transition; callers prove their distinct authority.
use super::{add, apply, balance, Kind};
use rusqlite::{params, OptionalExtension, Transaction};

#[allow(clippy::too_many_arguments)]
pub(super) fn persist(
    tx: &Transaction<'_>,
    root: &str,
    assignment: &str,
    attempt: String,
    dimension: &str,
    kind: Kind,
    amount: i64,
    key: &str,
    source: &str,
) -> Result<(), String> {
    persist_inner(
        tx, root, assignment, attempt, dimension, kind, amount, key, source, false,
    )
}

/// An already verified original invoice can increase an unlimited token box.
/// This is financial settlement only; callers retain their original receipt
/// proof, and fresh work always uses `persist` and its unknown-cost gate.
#[allow(clippy::too_many_arguments)]
pub(super) fn persist_known_cost(
    tx: &Transaction<'_>,
    root: &str,
    assignment: &str,
    attempt: String,
    dimension: &str,
    kind: Kind,
    amount: i64,
    key: &str,
    source: &str,
) -> Result<(), String> {
    if kind != Kind::Reserve || !super::DIMENSIONS[..3].contains(&dimension) {
        return Err("budget_receipt_transition_invalid".into());
    }
    let limit: Option<i64> = tx
        .query_row(
            "SELECT hard_limit FROM agent_budget_limits WHERE root_run_id=?1 AND dimension=?2",
            params![root, dimension],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if limit.is_some() {
        return Err("budget_receipt_transition_invalid".into());
    }
    persist_inner(
        tx, root, assignment, attempt, dimension, kind, amount, key, source, true,
    )
}

/// The private Root receipt caller verifies the original captured invoice and
/// projected shared ceiling. This cannot authorize a new model dispatch.
#[allow(clippy::too_many_arguments)]
pub(super) fn persist_captured_cost(
    tx: &Transaction<'_>, root: &str, attempt: String, dimension: &str,
    amount: i64, key: &str, source: &str,
) -> Result<(), String> {
    if !super::DIMENSIONS[..3].contains(&dimension) || amount <= 0 {
        return Err("budget_receipt_transition_invalid".into());
    }
    persist_inner(tx, root, "", attempt, dimension, Kind::Reserve, amount, key, source, true)
}

#[allow(clippy::too_many_arguments)]
fn persist_inner(
    tx: &Transaction<'_>,
    root: &str,
    assignment: &str,
    attempt: String,
    dimension: &str,
    kind: Kind,
    amount: i64,
    key: &str,
    source: &str,
    known_cost: bool,
) -> Result<(), String> {
    let key = super::scope::stored_key(tx, root, assignment, &attempt, key)?;
    let replay:Option<(String,String,String,String,i64,String)>=tx.query_row(
        "SELECT assignment_id,lease_attempt_id,dimension,kind,amount,source_id FROM agent_budget_entries WHERE root_run_id=?1 AND idempotency_key=?2",
        params![root,key],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional().map_err(|e|e.to_string())?;
    let expected = (
        assignment.to_string(),
        attempt.clone(),
        dimension.to_string(),
        kind.as_str().to_string(),
        amount,
        source.to_string(),
    );
    if let Some(existing) = replay {
        return if existing == expected {
            Ok(())
        } else {
            Err("budget_entry_replay_conflict".into())
        };
    }
    let mut own = balance_for_attempt(tx, root, assignment, &attempt, dimension)?;
    apply(&mut own, kind.as_str(), amount)?;
    let mut total = balance(tx, root, None, dimension)?;
    if kind == Kind::Reserve && total.indeterminate > 0 && !known_cost {
        return Err("budget_indeterminate_requires_reconciliation".into());
    }
    apply(&mut total, kind.as_str(), amount)?;
    let limit: Option<i64> = tx
        .query_row(
            "SELECT hard_limit FROM agent_budget_limits WHERE root_run_id=?1 AND dimension=?2",
            params![root, dimension],
            |r| r.get(0),
        )
        .map_err(|_| "budget_dimension_not_configured")?;
    let charge = add(add(total.reserved, total.consumed)?, total.indeterminate)?;
    if limit.is_some_and(|limit| charge > limit) {
        return Err("budget_hard_limit_exceeded".into());
    }
    let entry_id = uuid::Uuid::new_v4().to_string();
    let inserted=tx.execute("INSERT INTO agent_budget_entries(entry_id,root_run_id,assignment_id,lease_attempt_id,dimension,kind,amount,idempotency_key,source_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![entry_id,root,assignment,attempt,dimension,kind.as_str(),amount,key,source]).map_err(|e|e.to_string())?;
    let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_budget_entries WHERE entry_id=?1 AND root_run_id=?2
        AND assignment_id=?3 AND lease_attempt_id=?4 AND dimension=?5 AND kind=?6 AND amount=?7 AND idempotency_key=?8 AND source_id=?9)",
        params![entry_id,root,assignment,attempt,dimension,kind.as_str(),amount,key,source],|r|r.get(0)).map_err(|e|e.to_string())?;
    if inserted != 1
        || !exact
        || balance_for_attempt(tx, root, assignment, &attempt, dimension)? != own
        || balance(tx, root, None, dimension)? != total
    {
        return Err("budget_entry_persistence_conflict".into());
    }
    Ok(())
}

pub(crate) fn balance_for_attempt(
    db: &rusqlite::Connection,
    root: &str,
    assignment: &str,
    attempt: &str,
    dimension: &str,
) -> Result<super::Balance, String> {
    let mut statement = db.prepare("SELECT kind,amount FROM agent_budget_entries
        WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND dimension=?4 ORDER BY rowid")
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map(params![root, assignment, attempt, dimension], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })
        .map_err(|e| e.to_string())?;
    let mut b = super::Balance::default();
    for row in rows {
        let (kind, amount) = row.map_err(|e| e.to_string())?;
        apply(&mut b, &kind, amount)?;
    }
    Ok(b)
}
