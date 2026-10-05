fn value_first(value: &JsonValue, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| {
            value.get(*key).and_then(|item| match item {
                JsonValue::String(text) if !text.trim().is_empty() => Some(text.to_string()),
                _ => None,
            })
        })
        .unwrap_or_default()
}
#[allow(clippy::too_many_arguments)]
fn insert_finding(
    connection: &rusqlite::Connection,
    scan_id: &str,
    target_url: &str,
    stage: &str,
    kind: &str,
    record_key: &str,
    title: &str,
    severity: &str,
    value: &JsonValue,
) -> Result<(), String> {
    // This helper also runs inside larger publication transactions. A nested
    // SQLite savepoint lets us reject an AFTER trigger that silently rewrites
    // the row without committing either the forged finding or its side effects.
    connection
        .execute_batch("SAVEPOINT agent_finding_write")
        .map_err(|error| format!("sentinel_finding_savepoint_failed:{error}"))?;
    let record_json = value.to_string();
    let result = (|| {
        let changed = connection.execute("INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title,severity,record_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(scan_id,target_url,stage,kind,record_key) DO UPDATE SET title=excluded.title,severity=excluded.severity,record_json=excluded.record_json,updated_at=datetime('now','localtime')", params![scan_id,target_url,stage,kind,record_key,title,severity,&record_json]).map_err(|e| e.to_string())?;
        if changed != 1 {
            return Err("sentinel_finding_write_not_applied".into());
        }
        let persisted: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sentinel_findings WHERE scan_id=?1 AND target_url=?2 AND stage=?3 AND kind=?4 AND record_key=?5 AND title=?6 AND severity=?7 AND record_json=?8)",
            params![scan_id,target_url,stage,kind,record_key,title,severity,&record_json],
            |row| row.get(0),
        ).map_err(|error| format!("sentinel_finding_verify_failed:{error}"))?;
        if !persisted {
            return Err("sentinel_finding_write_mismatch".into());
        }
        Ok(())
    })();
    if let Err(error) = result {
        connection
            .execute_batch("ROLLBACK TO SAVEPOINT agent_finding_write; RELEASE SAVEPOINT agent_finding_write")
            .map_err(|rollback| format!("{error};sentinel_finding_rollback_unconfirmed:{rollback}"))?;
        return Err(error);
    }
    connection
        .execute_batch("RELEASE SAVEPOINT agent_finding_write")
        .map_err(|error| format!("sentinel_finding_commit_unconfirmed:{error}"))
}
