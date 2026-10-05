fn store_workbench_ci_policy(
    connection: &rusqlite::Connection, scan_id: &str, attempt: i64, policy: &JsonValue,
) -> Result<(), String> {
    let parsed = GatePolicy::from_workbench(policy)?;
    let limits = parsed.release_limits.as_ref().ok_or("workbench_ci_policy_invalid")?;
    let changed = connection.execute(
        "INSERT INTO source_ci_policies(scan_id,attempt_number,max_critical,max_high,block_release) VALUES(?1,?2,?3,?4,?5)",
        params![scan_id,attempt,limits.max_critical as i64,limits.max_high as i64,limits.block_release],
    ).map_err(|error| format!("workbench_ci_policy_not_persisted:{error}"))?;
    if changed != 1 || load_workbench_ci_policy(connection,scan_id,attempt)? != parsed {
        return Err("workbench_ci_policy_postcondition".into());
    }
    Ok(())
}

fn load_workbench_ci_policy(
    connection: &rusqlite::Connection, scan_id: &str, attempt: i64,
) -> Result<GatePolicy, String> {
    let (critical, high, block): (i64,i64,i64) = connection.query_row(
        "SELECT p.max_critical,p.max_high,p.block_release FROM source_ci_policies p
         JOIN sentinel_scans s ON s.id=p.scan_id AND s.attempt_count=p.attempt_number
         JOIN sentinel_scan_attempts a ON a.scan_id=p.scan_id AND a.attempt_number=p.attempt_number
         WHERE p.scan_id=?1 AND p.attempt_number=?2 AND s.scan_type='cicd' AND s.status='scanning'
         AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=p.scan_id)",
        params![scan_id,attempt], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).map_err(|_| "workbench_ci_policy_unavailable_start_new_attempt")?;
    if !matches!(block,0|1) { return Err("workbench_ci_policy_invalid".into()); }
    GatePolicy::from_workbench(&json!({"maxCritical":critical,"maxHigh":high,"blockRelease":block==1}))
}

// Read-only report provenance, not permission to execute an old attempt. Keep
// the active-execution loader above strict: completed/current-attempt checks
// must not be relaxed just to enable historical exports.
fn load_historical_source_ci_policy(
    connection: &rusqlite::Connection, scan_id: &str, attempt: i64,
) -> Result<GatePolicy, String> {
    if connection.is_autocommit() { return Err("source_ci_transaction_required".into()); }
    let (critical, high, block): (i64,i64,i64) = connection.query_row(
        "SELECT p.max_critical,p.max_high,p.block_release FROM source_ci_policies p
         JOIN sentinel_scans s ON s.id=p.scan_id
         JOIN sentinel_scan_attempts a ON a.scan_id=p.scan_id AND a.attempt_number=p.attempt_number
         WHERE p.scan_id=?1 AND p.attempt_number=?2 AND s.scan_type='cicd'
         AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=p.scan_id)",
        params![scan_id,attempt], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).map_err(|_| "source_ci_historical_policy_unavailable")?;
    if !matches!(block,0|1) { return Err("workbench_ci_policy_invalid".into()); }
    GatePolicy::from_workbench(&json!({"maxCritical":critical,"maxHigh":high,"blockRelease":block==1}))
}
