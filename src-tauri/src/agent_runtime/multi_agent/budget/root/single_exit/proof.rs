//! Full physical original sources, including rowid. Mutable UI state is not a grant.
use rusqlite::{params, types::Value, Connection};
fn rows(db: &Connection, sql: &str, root: &str) -> Result<String, String> {
    let mut q = db.prepare(sql).map_err(|e| e.to_string())?;
    let n = q.column_count();
    let values = q
        .query_map([root], |r| {
            (0..n)
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
pub(super) fn sources(db: &Connection, root: &str) -> Result<String, String> {
    let mut all = String::new();
    for table in [
        "agent_root_budget_attempts",
        "agent_budget_limits",
        "agent_budget_clock_origins",
        "agent_root_budget_definitions",
        "agent_root_mode_definitions",
    ] {
        all.push_str(table);
        all.push_str(&rows(
            db,
            &format!("SELECT rowid,* FROM {table} WHERE root_run_id=?1 ORDER BY rowid"),
            root,
        )?);
    }
    Ok(crate::agent_runtime::store::stable_hash(&all))
}
pub(super) fn binding(db: &Connection, root: &str) -> Result<String, String> {
    rows(
        db,
        "SELECT rowid,id,scan_id,attempt_number,target_url,backend,role,parent_run_id,
        root_run_id,assignment_id,orchestration_policy,plan_hash,plan_json,created_at,
        soft_token_budget,hard_token_budget,soft_request_budget,hard_request_budget
        FROM agent_runs WHERE id=?1",
        root,
    )
}
pub(super) fn wall(db: &Connection, root: &str) -> Result<String, String> {
    rows(db,"SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND dimension='wall_time_ms' ORDER BY rowid",root)
}
pub(super) fn clock(
    db: &Connection,
    root: &str,
    cutoff: &str,
) -> Result<(String, i64, i64), String> {
    let (origin,current,elapsed,future,hard):(String,String,Option<i64>,bool,i64)=db.query_row(
        "SELECT o.started_at,r.created_at,CAST((julianday(?2)-julianday(o.started_at))*86400000 AS INTEGER),
        COALESCE(julianday(?2)>julianday('now','localtime'),1),l.hard_limit
        FROM agent_budget_clock_origins o JOIN agent_runs r ON r.id=o.root_run_id
        JOIN agent_budget_limits l ON l.root_run_id=r.id AND l.dimension='wall_time_ms' WHERE r.id=?1",
        params![root,cutoff],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(|_|"budget_clock_origin_missing")?;
    let elapsed = elapsed
        .filter(|v| *v >= 0 && !future && hard > 0)
        .ok_or("budget_clock_origin_invalid")?;
    if origin != current {
        return Err("budget_clock_origin_conflict".into());
    }
    let bad:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_budget_entries WHERE root_run_id=?1 AND dimension='wall_time_ms'
        AND (assignment_id<>'' OR lease_attempt_id<>(SELECT id FROM agent_root_budget_attempts WHERE root_run_id=?1)))",[root],|r|r.get(0)).map_err(|e|e.to_string())?;
    if bad {
        return Err("budget_single_exit_owner_conflict".into());
    }
    Ok((origin, elapsed, hard))
}
