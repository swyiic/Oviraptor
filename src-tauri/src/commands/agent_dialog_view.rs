// UI read receipts are not mailbox acknowledgements, task receipts, or grants.
// They belong to the local app operator and are isolated by scan and attempt.
const DIALOG_VIEW_MAX_INTEGER: i64 = 9_007_199_254_740_991;
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentDialogView {
    scan_id: String,
    attempt_number: i64,
    revision: i64,
    selected_thread: String,
    all_read_sequence: i64,
    thread_read_sequences: std::collections::BTreeMap<String, i64>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentDialogViewInput {
    scan_id: String,
    attempt_number: i64,
    expected_revision: i64,
    selected_thread: String,
    mark_read_through: Option<i64>,
    selected_thread_sequence: Option<i64>,
}

fn check_dialog_view_scope(db: &rusqlite::Connection, scan: &str, attempt: i64) -> Result<(), String> {
    if !(1..=DIALOG_VIEW_MAX_INTEGER).contains(&attempt) || scan.is_empty() || scan.len() > 256 {
        return Err("dialog_view_invalid_scope".into());
    }
    let current: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sentinel_scans s WHERE s.id=?1 AND s.attempt_count=?2
         AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans d WHERE d.scan_id=s.id))",
        params![scan, attempt], |r| r.get(0),
    ).map_err(|e| e.to_string())?;
    if !current { return Err("dialog_view_attempt_changed_or_deleted".into()); }
    Ok(())
}

fn read_dialog_view_in(db: &rusqlite::Connection, scan: &str, attempt: i64) -> Result<AgentDialogView, String> {
    check_dialog_view_scope(db, scan, attempt)?;
    let stored: Option<(i64, String, i64, String)> = db.query_row(
        "SELECT revision,selected_thread,all_read_sequence,thread_read_sequences_json
         FROM agent_dialog_views WHERE scan_id=?1 AND attempt_number=?2",
        params![scan,attempt], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
    ).optional().map_err(|e| e.to_string())?;
    let mut view = AgentDialogView { scan_id: scan.into(), attempt_number: attempt,
        revision: 0, selected_thread: String::new(), all_read_sequence: 0,
        thread_read_sequences: Default::default() };
    if let Some((revision, thread, all_read, cursors)) = stored {
        if !(1..=DIALOG_VIEW_MAX_INTEGER).contains(&revision) || thread.len() > 2048
            || !(0..=DIALOG_VIEW_MAX_INTEGER).contains(&all_read) || cursors.len() > 1_100_000 {
            return Err("dialog_view_invalid_record".into());
        }
        view.thread_read_sequences = serde_json::from_str(&cursors).map_err(|_| "dialog_view_invalid_record")?;
        if view.thread_read_sequences.len() > 512 || view.thread_read_sequences.iter()
            .any(|(key,value)| key.is_empty() || key.len() > 2048 || !(0..=DIALOG_VIEW_MAX_INTEGER).contains(value)) {
            return Err("dialog_view_invalid_record".into());
        }
        view.revision = revision;
        view.selected_thread = thread;
        view.all_read_sequence = all_read;
        let maximum: i64 = db.query_row(
            "SELECT COALESCE(MAX(sequence),0) FROM agent_collaboration_events WHERE scan_id=?1 AND attempt_number=?2",
            params![scan,attempt], |r| r.get(0),
        ).map_err(|e| e.to_string())?;
        if all_read > maximum || view.thread_read_sequences.values().any(|value| *value > maximum) {
            return Err("dialog_view_future_cursor".into());
        }
    }
    Ok(view)
}

fn load_dialog_view(db: &rusqlite::Connection, scan: &str, attempt: i64) -> Result<AgentDialogView, String> {
    let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
    read_dialog_view_in(&tx, scan, attempt)
}

fn dialog_timeline_thread(item: &JsonValue) -> &str {
    ["threadKey", "correlationId", "assignmentId", "targetKey"].into_iter()
        .find_map(|key| item.get(key).and_then(JsonValue::as_str).filter(|value| !value.is_empty()))
        .unwrap_or("team")
}

fn persist_dialog_view(db: &rusqlite::Connection, input: &AgentDialogViewInput) -> Result<AgentDialogView, String> {
    check_dialog_view_scope(db, &input.scan_id, input.attempt_number)?;
    if !(0..DIALOG_VIEW_MAX_INTEGER).contains(&input.expected_revision) || input.selected_thread.len() > 2048
        || input.mark_read_through.is_some_and(|value| !(0..=DIALOG_VIEW_MAX_INTEGER).contains(&value))
        || input.selected_thread_sequence.is_some_and(|value| !(1..DIALOG_VIEW_MAX_INTEGER).contains(&value)) {
        return Err("dialog_view_invalid_input".into());
    }
    // Only the sanitized timeline or server-projected recipients supply keys.
    // No supplied JSON may invent a channel, content, or future read cursor.
    let status = native_scan_status(db, &input.scan_id)?;
    if status["attemptNumber"].as_i64() != Some(input.attempt_number) {
        return Err("dialog_view_attempt_changed_or_deleted".into());
    }
    let maximum = status["latestSequence"].as_i64().ok_or("dialog_view_status_invalid")?;
    if input.mark_read_through.is_some_and(|value| value > maximum) {
        return Err("dialog_view_future_cursor".into());
    }
    let current_thread = status["timeline"].as_array()
        .is_some_and(|items| items.iter().any(|item| dialog_timeline_thread(item) == input.selected_thread))
        || status["directiveRecipients"].as_array().is_some_and(|items| items.iter()
            .any(|item| item["threadKey"].as_str() == Some(input.selected_thread.as_str())));
    let historical_thread = if !input.selected_thread.is_empty() && !current_thread {
        if let Some(sequence) = input.selected_thread_sequence.filter(|sequence| *sequence <= maximum) {
            let page = native_scan_timeline_page(db, &input.scan_id, input.attempt_number, sequence + 1)?;
            page["timeline"].as_array().is_some_and(|items| items.iter().any(|item|
                item["sequence"].as_i64() == Some(sequence)
                    && dialog_timeline_thread(item) == input.selected_thread))
        } else { false }
    } else { false };
    if !input.selected_thread.is_empty() && !current_thread && !historical_thread {
        return Err("dialog_view_thread_unavailable".into());
    }
    let tx = rusqlite::Transaction::new_unchecked(db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    // Recheck inside the write lock; a new attempt may have started during the read.
    let mut next = read_dialog_view_in(&tx, &input.scan_id, input.attempt_number)?;
    if next.revision != input.expected_revision { return Err("dialog_view_revision_conflict".into()); }
    next.revision = next.revision.checked_add(1).ok_or("dialog_view_revision_exhausted")?;
    next.selected_thread.clone_from(&input.selected_thread);
    if let Some(sequence) = input.mark_read_through {
        if input.selected_thread.is_empty() {
            next.all_read_sequence = next.all_read_sequence.max(sequence);
            next.thread_read_sequences.retain(|_, value| *value > next.all_read_sequence);
        } else if sequence > next.all_read_sequence {
            let cursor = next.thread_read_sequences.entry(input.selected_thread.clone()).or_default();
            *cursor = (*cursor).max(sequence);
        }
    }
    if next.thread_read_sequences.len() > 512 { return Err("dialog_view_thread_limit".into()); }
    let cursors = serde_json::to_string(&next.thread_read_sequences).map_err(|e| e.to_string())?;
    let changed = tx.execute(
        "INSERT INTO agent_dialog_views(scan_id,attempt_number,revision,selected_thread,all_read_sequence,thread_read_sequences_json)
         VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(scan_id,attempt_number) DO UPDATE SET
         revision=excluded.revision,selected_thread=excluded.selected_thread,
         all_read_sequence=excluded.all_read_sequence,thread_read_sequences_json=excluded.thread_read_sequences_json
         WHERE agent_dialog_views.revision=?7",
        params![next.scan_id,next.attempt_number,next.revision,next.selected_thread,next.all_read_sequence,cursors,input.expected_revision],
    ).map_err(|e| format!("dialog_view_save_failed:{e}"))?;
    if changed != 1 || read_dialog_view_in(&tx, &input.scan_id, input.attempt_number)? != next {
        return Err("dialog_view_write_not_persisted".into());
    }
    tx.commit().map_err(|e| format!("dialog_view_commit_unconfirmed:{e}"))?;
    Ok(next)
}

#[tauri::command]
pub fn get_agent_dialog_view(state: State<'_, AppState>, scan_id: String, attempt_number: i64) -> Result<AgentDialogView, String> {
    load_dialog_view(&db::open(&state.db_path)?, &scan_id, attempt_number)
}

#[tauri::command]
pub fn save_agent_dialog_view(state: State<'_, AppState>, input: AgentDialogViewInput) -> Result<AgentDialogView, String> {
    persist_dialog_view(&db::open(&state.db_path)?, &input)
}
