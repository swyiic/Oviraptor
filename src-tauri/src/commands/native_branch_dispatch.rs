// A live OS lock establishes ownership, not replay safety. The receipt is
// committed before constructing a failure guard or allowing any external work.
// It is deliberately never reset when that lock is released or the app restarts.
fn register_native_dispatch_in(
    connection: &rusqlite::Connection, scan_id: &str, attempt: i64, branch: &str,
) -> Result<(), String> {
    if connection.is_autocommit() { return Err("native_dispatch_requires_transaction".into()); }
    let changed = connection.execute(
        "INSERT INTO native_branch_dispatches(scan_id,attempt_number,branch) VALUES(?1,?2,?3)",
        params![scan_id,attempt,branch],
    ).map_err(|error| error.to_string())?;
    if changed != 1 { return Err("native_dispatch_registration_not_persisted".into()); }
    Ok(())
}

fn native_dispatch_attempt_eligible(
    connection: &rusqlite::Connection, scan_id: &str, attempt: i64, branch: &str,
) -> Result<bool, String> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sentinel_scans s JOIN native_scan_branches b
         ON b.scan_id=s.id AND b.attempt_number=s.attempt_count
         WHERE s.id=?1 AND s.attempt_count=?2 AND s.status='scanning'
         AND b.branch=?3 AND b.status='pending'
         AND ((s.project_id IS NULL AND s.scan_type<>'web')
              OR EXISTS(SELECT 1 FROM projects p WHERE p.id=s.project_id AND p.status='active')))
         AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)
         AND NOT EXISTS(SELECT 1 FROM native_web_attempt_closures WHERE scan_id=?1 AND attempt_number=?2 AND branch=?3)
         AND NOT EXISTS(SELECT 1 FROM environment_preparation_lease)",
        params![scan_id,attempt,branch], |row| row.get(0),
    ).map_err(|error| error.to_string())
}

fn claim_native_dispatch_in(
    connection: &rusqlite::Connection, scan_id: &str, attempt: i64, branch: &str,
) -> Result<(), String> {
    if connection.is_autocommit() { return Err("native_dispatch_requires_transaction".into()); }
    if !native_dispatch_attempt_eligible(connection, scan_id, attempt, branch)? {
        return Err("native_dispatch_attempt_ineligible".into());
    }
    let claim = Uuid::new_v4().to_string();
    let claimed_at = chrono::Utc::now().to_rfc3339();
    let changed = connection.execute(
        "UPDATE native_branch_dispatches SET claim_id=?4,claimed_at=?5
         WHERE scan_id=?1 AND attempt_number=?2 AND branch=?3 AND claim_id='' AND claimed_at=''",
        params![scan_id,attempt,branch,claim,claimed_at],
    ).map_err(|error| error.to_string())?;
    if changed != 1 {
        // Missing (pre-journal) and previously claimed receipts both deny replay.
        return Err("native_dispatch_already_claimed_or_receipt_missing".into());
    }
    let persisted: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM native_branch_dispatches WHERE scan_id=?1 AND attempt_number=?2
         AND branch=?3 AND claim_id=?4 AND claimed_at=?5)",
        params![scan_id,attempt,branch,claim,claimed_at], |row| row.get(0),
    ).map_err(|error| error.to_string())?;
    if !persisted || !native_dispatch_attempt_eligible(connection, scan_id, attempt, branch)? {
        return Err("native_dispatch_claim_postcondition".into());
    }
    Ok(())
}
