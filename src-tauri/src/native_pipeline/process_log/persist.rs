//! Bound INSERT/append postconditions under a private diagnostic authorizer.
use super::{guard, Event, Hint, Scope, MAX_BYTES, MAX_ROWS};
use rusqlite::{params, Connection, OpenFlags};
use std::{path::Path, time::Duration};
pub(super) fn begin(path: &Path, scope: &Scope) -> Result<Connection, String> {
    // Use an existing authoritative DB; never create one from a log request.
    let mut connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .map_err(|_| "native_process_log_database_unavailable")?;
    // A logger must not inherit the ordinary command's ten-second lock wait.
    connection
        .busy_timeout(Duration::from_millis(25))
        .map_err(|_| "native_process_log_database_unavailable")?;
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(|_| "native_process_log_database_unavailable")?;
    connection
        .pragma_update(None, "synchronous", "FULL")
        .map_err(|_| "native_process_log_database_unavailable")?;
    crate::collaboration_events::install_commit_notifications(&connection);
    let tx = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|_| "native_process_log_binding_failed")?;
    guard::protect(&tx, guard::Mode::Begin, || {
        let created: String = tx
            .query_row("SELECT datetime('now','localtime')", [], |r| r.get(0))
            .map_err(|_| "native_process_log_binding_failed")?;
        let inserted=tx.execute(
        "INSERT INTO native_process_log_executions(execution_id,scan_id,attempt_number,invocation_key,stage,branch,dispatch_claim_id,created_at)
        SELECT ?1,?2,?3,?4,?5,?6,?7,?8 WHERE EXISTS(SELECT 1 FROM sentinel_scan_attempts a
        JOIN sentinel_scans s ON s.id=a.scan_id
        JOIN native_scan_branches b ON b.scan_id=s.id AND b.attempt_number=a.attempt_number
        JOIN native_branch_dispatches d ON d.scan_id=b.scan_id AND d.attempt_number=b.attempt_number AND d.branch=b.branch
        WHERE a.scan_id=?2 AND a.attempt_number=?3 AND s.attempt_count=?3 AND s.status='scanning'
        AND b.branch=?6 AND b.status='pending' AND d.claim_id=?7 AND d.claimed_at<>'')
        AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?2)",
        params![scope.execution_id,scope.scan_id,scope.attempt,scope.invocation_key,scope.stage,scope.branch,scope.dispatch_claim_id,created])
        .map_err(|_|"native_process_log_binding_failed")?;
        let exact:bool=tx.query_row("SELECT COUNT(*)=1 FROM native_process_log_executions
        WHERE execution_id=?1 AND scan_id=?2 AND attempt_number=?3 AND invocation_key=?4 AND stage=?5
        AND branch=?6 AND dispatch_claim_id=?7 AND state='open' AND row_count=0 AND byte_count=0 AND created_at=?8
        AND NOT EXISTS(SELECT 1 FROM native_process_log_rows WHERE execution_id=?1)",
        params![scope.execution_id,scope.scan_id,scope.attempt,scope.invocation_key,scope.stage,scope.branch,scope.dispatch_claim_id,created],|r|r.get(0))
        .map_err(|_|"native_process_log_binding_failed")?;
        if inserted != 1 || !exact {
            return Err("native_process_log_binding_not_persisted".into());
        }
        Ok(())
    })?;
    tx.commit()
        .map_err(|_| "native_process_log_binding_failed")?;
    Ok(connection)
}
pub(super) fn append(
    connection: &mut Connection,
    scope: &Scope,
    event: &Event,
    message: &str,
    terminal: Option<&str>,
) -> Result<Hint, String> {
    let tx = connection
        .transaction()
        .map_err(|_| "native_process_log_commit_failed")?;
    let hint = guard::protect(&tx, guard::Mode::Append, || {
        // A begun process can finish its cancellation/EOF diagnostics under
        // ORIGINAL immutable scope. This never creates a new dispatch right.
        let (count,bytes,created):(i64,i64,String)=tx.query_row(
        "SELECT row_count,byte_count,created_at FROM native_process_log_executions
        WHERE execution_id=?1 AND scan_id=?2 AND attempt_number=?3 AND invocation_key=?4 AND stage=?5
        AND branch=?6 AND dispatch_claim_id=?7 AND state='open'",
        params![scope.execution_id,scope.scan_id,scope.attempt,scope.invocation_key,scope.stage,scope.branch,scope.dispatch_claim_id],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|"native_process_log_closed_or_budget_exhausted")?;
        let next_count = count
            .checked_add(1)
            .ok_or("native_process_log_counter_invalid")?;
        let next_bytes = bytes
            .checked_add(message.len() as i64)
            .ok_or("native_process_log_counter_invalid")?;
        if count < 0
            || bytes < 0
            || (terminal.is_none() && (count >= MAX_ROWS || next_bytes > MAX_BYTES))
        {
            return Err("native_process_log_closed_or_budget_exhausted".into());
        }
        let row_time: String = tx
            .query_row("SELECT datetime('now','localtime')", [], |r| r.get(0))
            .map_err(|_| "native_process_log_commit_failed")?;
        let changed = tx
            .execute(
                "UPDATE native_process_log_executions SET row_count=?2,byte_count=?3,state=?4
        WHERE execution_id=?1 AND row_count=?5 AND byte_count=?6 AND state='open'",
                params![
                    scope.execution_id,
                    next_count,
                    next_bytes,
                    terminal.unwrap_or("open"),
                    count,
                    bytes
                ],
            )
            .map_err(|_| "native_process_log_commit_failed")?;
        let inserted=tx.execute("INSERT INTO native_process_log_rows(execution_id,stream,stream_sequence,message,gap,created_at)
        VALUES(?1,?2,?3,?4,?5,?6)",
        params![scope.execution_id,event.stream,event.stream_sequence,message,event.gap,row_time])
        .map_err(|_|"native_process_log_commit_failed")?;
        let sequence = tx.last_insert_rowid();
        let exact: bool = tx
            .query_row(
                "SELECT COUNT(*)=1 FROM native_process_log_rows r
        JOIN native_process_log_executions e ON e.execution_id=r.execution_id
        WHERE r.sequence=?1 AND e.execution_id=?2 AND r.stream=?3 AND r.stream_sequence=?4
        AND r.message=?5 AND r.gap=?6 AND r.created_at=?7 AND e.scan_id=?8 AND e.attempt_number=?9
        AND e.invocation_key=?10 AND e.stage=?11 AND e.branch=?12 AND e.dispatch_claim_id=?13
        AND e.row_count=?14 AND e.byte_count=?15 AND e.state=?16 AND e.created_at=?17",
                params![
                    sequence,
                    scope.execution_id,
                    event.stream,
                    event.stream_sequence,
                    message,
                    event.gap,
                    row_time,
                    scope.scan_id,
                    scope.attempt,
                    scope.invocation_key,
                    scope.stage,
                    scope.branch,
                    scope.dispatch_claim_id,
                    next_count,
                    next_bytes,
                    terminal.unwrap_or("open"),
                    created
                ],
                |r| r.get(0),
            )
            .map_err(|_| "native_process_log_commit_failed")?;
        if changed != 1 || inserted != 1 || !exact {
            return Err("native_process_log_row_not_persisted".into());
        }
        Ok(Hint {
            scope: scope.clone(),
            sequence,
            stream: event.stream.into(),
            stream_sequence: event.stream_sequence,
        })
    })?;
    tx.commit()
        .map_err(|_| "native_process_log_commit_failed")?;
    Ok(hint)
}
