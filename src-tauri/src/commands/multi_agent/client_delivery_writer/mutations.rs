//! Exact original mutable rows; all other Roots, workers and grants stay byte exact.
use super::{
    proof::Proof,
    rows::{get, Table, Tables},
};
use rusqlite::types::Value;
pub(super) fn expected(p: &Proof, actual: &Tables, end: &str) -> Result<Tables, String> {
    let mut expected = p.tables.clone();
    if p.replay {
        return Ok(expected);
    }
    let old = get(&p.tables, "agent_assignments")?;
    let row = old.find("id", &p.assignment)?;
    let reserved = [
        old.number(row, "reserved_tokens")?,
        old.number(row, "reserved_requests")?,
    ];
    let old = get(&p.tables, "agent_budget_ledger")?;
    let mut row = old.find("root_run_id", &p.root)?.to_vec();
    for (col, delta) in [
        ("reserved_tokens", -reserved[0]),
        ("reserved_requests", -reserved[1]),
        ("spent_tokens", p.usage.total_tokens),
        ("spent_requests", p.usage.model_requests),
    ] {
        let n = old
            .number(&row, col)?
            .checked_add(delta)
            .filter(|v| *v >= 0)
            .ok_or("client_side_delivery_ledger_conflict")?;
        old.set(&mut row, col, Value::Integer(n))?;
    }
    stamp(
        old,
        &mut row,
        get(actual, "agent_budget_ledger")?,
        "root_run_id",
        &p.root,
        "updated_at",
        (p, end),
    )?;
    expected
        .get_mut("agent_budget_ledger")
        .unwrap()
        .replace(row)?;
    let old = get(&p.tables, "agent_assignments")?;
    let mut row = old.find("id", &p.assignment)?.to_vec();
    for (col, val) in [("state", "completed"), ("failure_class", "")] {
        old.set(&mut row, col, Value::Text(val.into()))?;
    }
    for col in ["reserved_tokens", "reserved_requests"] {
        old.set(&mut row, col, Value::Integer(0))?;
    }
    for col in ["budget_settled_at", "finished_at", "updated_at"] {
        stamp(
            old,
            &mut row,
            get(actual, "agent_assignments")?,
            "id",
            &p.assignment,
            col,
            (p, end),
        )?;
    }
    expected
        .get_mut("agent_assignments")
        .unwrap()
        .replace(row)?;
    let old = get(&p.tables, "agent_runs")?;
    let mut row = old.find("id", &p.run)?.to_vec();
    let (code, reason) = if p.paused {
        (
            "readonly_receipt_reconciled",
            "Saved readonly result delivered locally; no model retry or capability restoration"
                .to_string(),
        )
    } else {
        (
            "child_completed",
            crate::agent_runtime::secrets::redact_text_with(&p.summary, None),
        )
    };
    for (col, val) in [
        ("status", "terminal"),
        ("terminal_state", "completed"),
        ("terminal_code", code),
        ("terminal_reason", reason.as_str()),
    ] {
        old.set(&mut row, col, Value::Text(val.into()))?;
    }
    for (col, n) in [
        ("used_tokens", p.usage.total_tokens),
        ("used_cached_tokens", p.usage.cached_input_tokens),
        ("used_requests", p.usage.model_requests),
    ] {
        old.set(&mut row, col, Value::Integer(n))?;
    }
    for col in ["finished_at", "updated_at"] {
        stamp(
            old,
            &mut row,
            get(actual, "agent_runs")?,
            "id",
            &p.run,
            col,
            (p, end),
        )?;
    }
    expected.get_mut("agent_runs").unwrap().replace(row)?;
    let old = get(&p.tables, "agent_assignment_attempts")?;
    let mut row = old.find("id", &p.worker)?.to_vec();
    match old.text(&row, "state")? {
        "running" | "paused" => {
            old.set(&mut row, "state", Value::Text("completed".into()))?;
            old.set(&mut row, "failure_class", Value::Text(String::new()))?;
            stamp(
                old,
                &mut row,
                get(actual, "agent_assignment_attempts")?,
                "id",
                &p.worker,
                "finished_at",
                (p, end),
            )?;
            expected
                .get_mut("agent_assignment_attempts")
                .unwrap()
                .replace(row)?;
        }
        "expired" | "failed" if p.paused => {} // Saved proof preserves the original terminal worker.
        _ => return Err("client_side_delivery_original_worker_conflict".into()),
    }
    let old = get(&p.tables, "agent_capability_leases")?;
    let mut count = 0;
    for row in old.rows.values() {
        if old.text(row, "child_run_id")? != p.run {
            continue;
        }
        count += 1;
        if old.text(row, "revoked_at")?.is_empty() {
            if p.paused {
                return Err("client_side_delivery_paused_capability_live".into());
            }
            let mut expected_row = row.clone();
            stamp(
                old,
                &mut expected_row,
                get(actual, "agent_capability_leases")?,
                "id",
                old.text(row, "id")?,
                "revoked_at",
                (p, end),
            )?;
            expected
                .get_mut("agent_capability_leases")
                .unwrap()
                .replace(expected_row)?;
        }
    }
    if count != 2 {
        return Err("client_side_delivery_capability_count_conflict".into());
    }
    let old = get(&p.tables, "agent_lane_leases")?;
    let row = old.find("assignment_id", &p.assignment)?;
    let Some(Value::Integer(id)) = row.first() else {
        return Err("client_side_delivery_lane_rowid_conflict".into());
    };
    expected
        .get_mut("agent_lane_leases")
        .unwrap()
        .rows
        .remove(id);
    Ok(expected)
}
fn stamp(
    old: &Table,
    row: &mut [Value],
    actual: &Table,
    key: &str,
    id: &str,
    col: &str,
    window: (&Proof, &str),
) -> Result<(), String> {
    let (p, end) = window;
    let value = actual.text(actual.find(key, id)?, col)?;
    if value.is_empty() || value < p.now.as_str() || value > end {
        return Err("client_side_delivery_timestamp_conflict".into());
    }
    old.set(row, col, Value::Text(value.into()))
}
