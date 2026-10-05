// Detection is refusal only: no old format decoding, data rewrite or deletion.
fn require_scan_without_retired_data(connection: &rusqlite::Connection,scan_id:&str)->Result<(),String> {
    let retired: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sentinel_findings WHERE scan_id=?1 AND stage IN ('strix','strix-coverage'))
         OR EXISTS(SELECT 1 FROM sentinel_checkpoints WHERE scan_id=?1 AND (stage IN ('strix_run','strix_events','strix_coverage') OR stage GLOB 'strix_run:*' OR stage GLOB 'strix_events:*' OR stage GLOB 'strix_coverage:*'))
         OR EXISTS(SELECT 1 FROM app_settings WHERE key=?2 OR key=?3 OR substr(key,1,length(?3)+1)=?3||':')
         OR EXISTS(SELECT 1 FROM agent_runs WHERE scan_id=?1 AND (backend<>'native' OR status='legacy_backend_removed'))",
        params![scan_id, format!("strix-current-attempt:{scan_id}"), format!("strix-result-signature:{scan_id}")],
        |row| row.get(0),
    ).map_err(|error| error.to_string())?;
    if retired {
        return Err("retired_result_data_requires_confirmed_cleanup".into());
    }
    Ok(())
}
