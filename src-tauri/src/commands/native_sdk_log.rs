// Separate SDK stage channel: identity/cursor hints only, never model content.
pub(crate) fn notify_native_sdk_log(
    hint: &crate::agent_runtime::model::diagnostics::Hint,
) -> Result<(), String> {
    let Some(app) = NEST_LOG_APP.get() else {
        return Ok(());
    };
    app.emit("nest-native-sdk-log", hint)
        .map_err(|_| "native_sdk_log_notification_unavailable".into())
}

#[tauri::command]
pub async fn read_native_sdk_log(
    state: State<'_, AppState>,
    scan_id: String,
    attempt: i64,
    cursor_attempt: Option<i64>,
    owner_id: Option<String>,
    after_sequence: i64,
    limit: Option<i64>,
) -> Result<crate::agent_runtime::model::diagnostics::replay::Snapshot, String> {
    let db_path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::agent_runtime::model::diagnostics::replay::read(
            &db_path,
            &scan_id,
            attempt,
            cursor_attempt,
            owner_id.as_deref(),
            after_sequence,
            limit.unwrap_or(300),
        )
    })
    .await
    .map_err(|_| "native_sdk_log_read_failed".to_string())?
}
