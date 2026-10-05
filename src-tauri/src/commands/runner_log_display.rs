/// Both event delivery and tail reads must apply the same presentation filter.
/// The on-disk evidence is not rewritten by a display operation.
fn runner_log_display_line(line: &str) -> String {
    crate::log_display::line(line)
}

fn runner_log_notification(scan_id: &str, attempt: i64, line: &str) -> JsonValue {
    serde_json::json!({
        "scanId": scan_id,
        "attempt": attempt,
        "line": runner_log_display_line(line),
    })
}

#[cfg(test)]
mod runner_log_write_tests {
    include!("runner_log_write_tests.rs");
}

// Native process journal notifications contain identity/cursors only. Consumers
// must use journal replay, never treat a hint as delivered output.
pub(crate) fn notify_native_process_log(
    hint: &crate::native_pipeline::process::log::Hint,
) -> Result<(), String> {
    let Some(app) = NEST_LOG_APP.get() else {
        return Ok(());
    };
    app.emit("nest-native-process-log", hint)
        .map_err(|_| "native_process_log_notification_unavailable".into())
}

#[tauri::command]
pub async fn read_native_process_log(
    state: State<'_, AppState>,
    scan_id: String,
    attempt: i64,
    cursor_attempt: Option<i64>,
    execution_id: Option<String>,
    after_sequence: i64,
    limit: Option<i64>,
) -> Result<NativeProcessLogSnapshot, String> {
    let db_path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        read_native_process_log_snapshot(
            &db_path,
            &scan_id,
            attempt,
            cursor_attempt,
            execution_id.as_deref(),
            after_sequence,
            limit.unwrap_or(300),
        )
    })
    .await
    .map_err(|_| "native_process_log_read_failed".to_string())?
}
