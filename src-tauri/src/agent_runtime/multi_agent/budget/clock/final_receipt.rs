//! Physical original-source and expense receipts, including row identity.
use rusqlite::{types::Value, Connection};

pub(super) fn sources(db: &Connection, root: &str) -> Result<String, String> {
    let mut output = String::new();
    for table in [
        "agent_root_budget_attempts",
        "agent_budget_clock_origins",
        "agent_budget_limits",
        "agent_root_budget_definitions",
        "agent_coordinator_leases",
    ] {
        output.push_str(table);
        output.push_str(&rows(
            db,
            &format!("SELECT rowid,* FROM {table} WHERE root_run_id=?1 ORDER BY rowid"),
            root,
        )?);
    }
    Ok(crate::agent_runtime::store::stable_hash(&output))
}

pub(super) fn wall(db: &Connection, root: &str) -> Result<String, String> {
    rows(db,"SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND dimension='wall_time_ms' ORDER BY rowid",root)
}

pub(super) fn all_costs(db: &Connection, root: &str) -> Result<String, String> {
    rows(
        db,
        "SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 ORDER BY rowid",
        root,
    )
}

fn rows(db: &Connection, sql: &str, root: &str) -> Result<String, String> {
    let mut q = db.prepare(sql).map_err(|e| e.to_string())?;
    let count = q.column_count();
    let values = q
        .query_map([root], |r| {
            (0..count)
                .map(|i| r.get::<_, Value>(i))
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(crate::agent_runtime::store::stable_hash(&format!(
        "{values:?}"
    )))
}
