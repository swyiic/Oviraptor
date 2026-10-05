//! Separate immutable original full-source identity. Never a runtime grant.
use rusqlite::{params, Connection, Transaction};
fn controls(db: &Connection, scan: &str) -> Result<Vec<(String, String)>, String> {
    let mut q=db.prepare("SELECT root_run_id,id FROM agent_root_budget_attempts
        WHERE json_extract(contract_json,'$.root.scan')=?1 AND json_extract(contract_json,'$.root.policy')='multi' ORDER BY root_run_id").map_err(|e|e.to_string())?;
    let rows = q
        .query_map([scan], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string());
    rows
}
pub(super) fn persist(
    tx: &Transaction<'_>,
    scan: &str,
    hash: &str,
    stamp: &str,
) -> Result<(), String> {
    verify_schema(tx)?;
    for (root, control) in controls(tx, scan)? {
        if tx.execute("INSERT INTO native_deleted_scan_anchors(root_run_id,control_id,scan_id,audit_hash,deleted_at) VALUES(?1,?2,?3,?4,?5)",
            params![root,control,scan,hash,stamp]).map_err(|e|e.to_string())?!=1 {return Err("deleted_audit_anchor_write_unconfirmed".into());}
    }
    verify(tx, scan, hash, stamp)
}
pub(super) fn verify(db: &Connection, scan: &str, hash: &str, stamp: &str) -> Result<(), String> {
    let controls = controls(db, scan)?;
    if controls.is_empty() {
        return Ok(());
    } // Historical Single V1 remains supported.
    verify_schema(db)?;
    let mut q=db.prepare("SELECT root_run_id,control_id,audit_hash,deleted_at FROM native_deleted_scan_anchors WHERE scan_id=?1 ORDER BY root_run_id").map_err(|e|e.to_string())?;
    let rows = q
        .query_map([scan], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    let expected = controls
        .into_iter()
        .map(|(root, control)| (root, control, hash.to_string(), stamp.to_string()))
        .collect::<Vec<_>>();
    if rows != expected {
        return Err("deleted_audit_original_anchor_conflict".into());
    }
    Ok(())
}
fn schema(db: &Connection) -> Result<Vec<(String, String)>, String> {
    let mut q=db.prepare("SELECT name,sql FROM sqlite_master WHERE name IN('native_deleted_scan_anchors','deleted_anchor_no_update','deleted_anchor_no_delete','deleted_anchor_no_replace') ORDER BY name").map_err(|e|e.to_string())?;
    let rows = q
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string());
    rows
}
fn verify_schema(db: &Connection) -> Result<(), String> {
    let expected = Connection::open_in_memory().map_err(|e| e.to_string())?;
    expected
        .execute_batch(include_str!("schema.sql"))
        .map_err(|e| e.to_string())?;
    if schema(db)? != schema(&expected)? {
        return Err("deleted_audit_original_anchor_schema_conflict".into());
    }
    Ok(())
}
