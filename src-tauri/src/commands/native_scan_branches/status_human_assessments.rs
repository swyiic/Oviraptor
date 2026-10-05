// Snapshot of original unpublished Human assessment obligations; no chat cursor.
fn native_human_assessment_obligations(
    db: &rusqlite::Connection,
    scan: &str,
    attempt: i64,
) -> Result<JsonValue, String> {
    use crate::agent_runtime::multi_agent::budget::root::model::tick::Tick;
    if db.is_autocommit() {
        return Err("root_tick_projection_requires_snapshot".into());
    }
    let mut q=db.prepare("SELECT q.root_run_id,q.call_id FROM agent_root_tick_receipts q JOIN agent_root_budget_attempts x ON x.id=q.lease_attempt_id JOIN agent_runs r ON r.id=q.root_run_id WHERE q.phase='request' AND ((r.scan_id=?1 AND r.attempt_number=?2) OR (json_extract(x.contract_json,'$.root.scan')=?1 AND json_extract(x.contract_json,'$.root.attempt')=?2)) AND NOT EXISTS(SELECT 1 FROM agent_root_tick_receipts p WHERE p.call_id=q.call_id AND p.phase='publication') ORDER BY q.created_at,q.root_run_id,q.round").map_err(|e|e.to_string())?;
    let rows = q
        .query_map(params![scan, attempt], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?;
    let mut items = Vec::new();
    let mut truncated = false;
    for row in rows {
        let (root, call) = row.map_err(|e| e.to_string())?;
        if let Some(item) = Tick::project_unpublished_human(db, &root, &call, scan, attempt)? {
            // Full audit precedes the display bound; truncation cannot hide a bad receipt.
            if items.len() < 50 {
                items.push(item);
            } else {
                truncated = true;
            }
        }
    }
    Ok(
        json!({"schemaVersion":1,"executionAllowed":false,"automaticResumeAllowed":false,"items":items,"truncated":truncated}),
    )
}
