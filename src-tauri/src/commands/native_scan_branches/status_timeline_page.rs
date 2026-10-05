// A read-only, attempt-fenced history projection. The receipt audit runs before
// the page filter, so an older or missing attestation cannot be hidden by a
// newer page. No history read changes mailbox ack or execution state.
fn native_scan_timeline_page(
    connection: &rusqlite::Connection,
    scan_id: &str,
    attempt_number: i64,
    before_sequence: i64,
) -> Result<JsonValue, String> {
    if attempt_number <= 0 || before_sequence <= 0 {
        return Err("无效的历史时间线游标".into());
    }
    let transaction = connection.unchecked_transaction().map_err(|error| error.to_string())?;
    let current_attempt: i64 = transaction.query_row(
        "SELECT attempt_count FROM sentinel_scans WHERE id=?1
         AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)",
        [scan_id], |row| row.get(0),
    ).map_err(|error| match error {
        rusqlite::Error::QueryReturnedNoRows => "任务不存在".to_string(),
        other => format!("无法读取任务轮次：{other}"),
    })?;
    if current_attempt != attempt_number {
        return Err("历史时间线轮次已变化，请刷新当前任务".into());
    }
    let latest: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(sequence),0) FROM agent_collaboration_events
         WHERE scan_id=?1 AND attempt_number=?2",
        params![scan_id, attempt_number], |row| row.get(0),
    ).map_err(|error| error.to_string())?;
    if before_sequence > latest.saturating_add(1) {
        return Err("历史时间线游标超出当前轮次".into());
    }
    let window = TimelineWindow::older(&transaction, scan_id, attempt_number, before_sequence)?;
    let closure_handoff = closure_handoff_relations(&transaction, scan_id)?;
    let administrative_closure = verified_administrative_closure(&transaction, scan_id)?;
    let (timeline, _) = native_status_timeline(
        &transaction, scan_id, attempt_number, &window, &closure_handoff,
        administrative_closure.as_ref(),
    )?;
    Ok(crate::agent_runtime::secrets::redact_json(&json!({
        "scanId":scan_id,"attemptNumber":attempt_number,"beforeSequence":before_sequence,
        "timelineBeforeSequence":window.first_sequence,"hasEarlierTimeline":window.has_earlier,
        "timeline":timeline,
    })))
}
