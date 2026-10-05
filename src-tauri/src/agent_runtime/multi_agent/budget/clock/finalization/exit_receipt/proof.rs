//! Exact original physical sources, ten dimensions, and immutable DDL.
use crate::agent_runtime::multi_agent::budget;
use rusqlite::{types::Value, Connection};
use serde::{Deserialize, Serialize};

#[derive(PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Dimension {
    name: String,
    hard: i64,
    reserved: i64,
    consumed: i64,
    indeterminate: i64,
}

pub(super) fn dimensions(db: &Connection, root: &str) -> Result<Vec<Dimension>, String> {
    budget::DIMENSIONS.into_iter().map(|name| {
        let b = budget::balance(db, root, None, name)?;
        let hard = db.query_row("SELECT hard_limit FROM agent_budget_limits WHERE root_run_id=?1 AND dimension=?2",
            rusqlite::params![root,name], |r|r.get(0)).map_err(|e|e.to_string())?;
        Ok(Dimension { name: name.into(), hard, reserved: b.reserved, consumed: b.consumed,
            indeterminate: b.indeterminate })
    }).collect()
}

pub(super) fn physical_root(db: &Connection, root: &str) -> Result<String, String> {
    rows(
        db,
        "SELECT rowid,id,scan_id,attempt_number,target_url,backend,role,parent_run_id,
        root_run_id,assignment_id,orchestration_policy,plan_hash,plan_json,created_at,
        soft_token_budget,hard_token_budget,soft_request_budget,hard_request_budget
        FROM agent_runs WHERE id=?1",
        root,
    )
}

pub(super) fn terminal_event(db: &Connection, root: &str) -> Result<String, String> {
    rows(db, "SELECT rowid,* FROM agent_collaboration_events WHERE entity_type='agent_run'
        AND entity_id=?1 AND event_type='agent_run' AND json_extract(payload_json,'$.role')='coordinator'
        AND json_extract(payload_json,'$.status')='terminal' ORDER BY rowid", root)
}

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

fn schema(db: &Connection) -> Result<Vec<(String, String)>, String> {
    let mut q = db.prepare("SELECT name,sql FROM sqlite_master WHERE name IN
        ('agent_multi_exit_receipts','multi_exit_no_update','multi_exit_no_delete','multi_exit_no_replace') ORDER BY name")
        .map_err(|e|e.to_string())?;
    let values = q
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<_>>()
        .map_err(|e| e.to_string())?;
    Ok(values)
}

pub(super) fn verify_schema(db: &Connection) -> Result<(), String> {
    // Trusted DDL only, no source rows or executable Root in this private DB.
    let expected = Connection::open_in_memory().map_err(|e| e.to_string())?;
    expected
        .execute_batch(include_str!("schema.sql"))
        .map_err(|e| e.to_string())?;
    if schema(db)? != schema(&expected)? {
        return Err("budget_multi_exit_immutable_schema_conflict".into());
    }
    Ok(())
}
