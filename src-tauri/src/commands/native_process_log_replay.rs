// Committed Native diagnostics only. Reads do not claim or restart any worker.
#[derive(serde::Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativeProcessLogSnapshot {
    schema_version: u8,
    scan_id: String,
    attempt: i64,
    requested_after_sequence: i64,
    reset_cursor: bool,
    #[serde(flatten)]
    page: crate::native_pipeline::process::log::Page,
}

pub(crate) fn read_native_process_log_snapshot(
    path: &Path,
    scan_id: &str,
    attempt: i64,
    cursor_attempt: Option<i64>,
    execution_id: Option<&str>,
    after: i64,
    limit: i64,
) -> Result<NativeProcessLogSnapshot, String> {
    // No CREATE, schema migration, write pragma, app-data path or CAS access.
    let mut connection = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .map_err(|_| "native_process_log_database_unavailable")?;
    connection
        .busy_timeout(Duration::from_millis(25))
        .map_err(|_| "native_process_log_database_unavailable")?;
    connection
        .pragma_update(None, "query_only", true)
        .map_err(|_| "native_process_log_database_unavailable")?;
    read_native_process_log_snapshot_in(
        &mut connection,
        scan_id,
        attempt,
        cursor_attempt,
        execution_id,
        after,
        limit,
    )
}

fn read_native_process_log_snapshot_in(
    connection: &mut rusqlite::Connection,
    scan_id: &str,
    attempt: i64,
    cursor_attempt: Option<i64>,
    execution_id: Option<&str>,
    after: i64,
    limit: i64,
) -> Result<NativeProcessLogSnapshot, String> {
    if runner_log_scan_error(scan_id).is_some()
        || attempt < 0
        || after < 0
        || !(1..=300).contains(&limit)
        || cursor_attempt.is_some_and(|n| n <= 0)
        || (after > 0 && cursor_attempt.is_none())
    {
        return Err("native_process_log_cursor_invalid".into());
    }
    if execution_id.is_some_and(|id| uuid::Uuid::parse_str(id).is_err()) {
        return Err("native_process_log_execution_invalid".into());
    }
    // Scope, high-water cursor, state and rows share one WAL snapshot. A writer
    // may commit concurrently; those later rows belong to the next replay page.
    let tx = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)
        .map_err(|_| "native_process_log_read_failed")?;
    let latest: i64 = tx
        .query_row(
            "SELECT s.attempt_count FROM sentinel_scans s WHERE s.id=?1
        AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=s.id)",
            [scan_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| "native_process_log_read_failed")?
        .ok_or("native_process_log_scan_unavailable")?;
    let actual = if attempt == 0 { latest } else { attempt };
    let exists: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sentinel_scan_attempts
        WHERE scan_id=?1 AND attempt_number=?2)",
            params![scan_id, actual],
            |r| r.get(0),
        )
        .map_err(|_| "native_process_log_read_failed")?;
    if actual <= 0 || !exists {
        return Err("native_process_log_attempt_unavailable".into());
    }
    let reset_cursor = cursor_attempt.is_some_and(|n| n != actual);
    if reset_cursor && (attempt > 0 || execution_id.is_some()) {
        return Err("native_process_log_cursor_scope_mismatch".into());
    }
    if let Some(id) = execution_id {
        let owned: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM native_process_log_executions
            WHERE execution_id=?1 AND scan_id=?2 AND attempt_number=?3)",
                params![id, scan_id, actual],
                |r| r.get(0),
            )
            .map_err(|_| "native_process_log_read_failed")?;
        if !owned {
            return Err("native_process_log_execution_unavailable".into());
        }
    }
    let page = crate::native_pipeline::process::log::page(
        &tx,
        scan_id,
        actual,
        execution_id,
        if reset_cursor { 0 } else { after },
        limit,
    )?;
    tx.commit().map_err(|_| "native_process_log_read_failed")?;
    Ok(NativeProcessLogSnapshot {
        schema_version: 1,
        scan_id: scan_id.into(),
        attempt: actual,
        requested_after_sequence: after,
        reset_cursor,
        page,
    })
}

#[cfg(test)]
mod native_process_log_replay_tests {
    use super::*;
    include!("native_process_log_replay_tests.rs");
}
