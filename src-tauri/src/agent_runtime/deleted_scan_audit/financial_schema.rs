//! Full trusted guards and permanent financial identity before task deletion.
use rusqlite::Connection;
fn schema(db: &Connection) -> Result<Vec<(String, String)>, String> {
    let mut q=db.prepare("SELECT name,sql FROM sqlite_master WHERE name IN('agent_root_tick_receipts','root_tick_no_update','root_tick_no_delete','root_tick_no_replace',
        'agent_root_tick_timeline_receipts','root_tick_timeline_no_update','root_tick_timeline_no_delete','root_tick_timeline_no_replace') ORDER BY name").map_err(|e|e.to_string())?;
    let rows = q
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string());
    rows
}
pub(super) fn require_original(db: &Connection) -> Result<(), String> {
    let expected = Connection::open_in_memory().map_err(|e| e.to_string())?;
    expected
        .execute_batch(include_str!(
            "../multi_agent/budget/root/model/tick/schema.sql"
        ))
        .map_err(|e| e.to_string())?;
    if schema(db)? != schema(&expected)? {
        return Err("deleted_audit_financial_schema_migration_required".into());
    }
    Ok(())
}
