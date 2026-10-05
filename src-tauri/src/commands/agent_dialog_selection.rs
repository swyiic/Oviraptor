// Navigation preferences never resume a scan or revive a deleted task.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDialogSelection {
    project_id: Option<i64>,
    revision: i64,
    selected_scan_id: Option<String>,
    selected_scan: Option<SentinelScan>,
    selection_unavailable: bool,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentDialogSelectionInput {
    project_id: Option<i64>,
    expected_revision: i64,
    scan_id: String,
}

fn dialog_selection_scope(db: &rusqlite::Connection, project: Option<i64>) -> Result<String, String> {
    match project {
        None => Ok("all".into()),
        Some(id) if (1..=DIALOG_VIEW_MAX_INTEGER).contains(&id) => {
            let exists: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1)", [id], |r| r.get(0))
                .map_err(|e| e.to_string())?;
            if !exists { return Err("dialog_selection_project_unavailable".into()); }
            Ok(format!("project:{id}"))
        }
        _ => Err("dialog_selection_invalid_project".into()),
    }
}

fn dialog_task_in(db: &rusqlite::Connection, project: Option<i64>, scan: &str) -> Result<Option<SentinelScan>, String> {
    dialog_selection_scope(db, project)?;
    if scan.is_empty() || scan.len() > 256 { return Err("dialog_selection_invalid_scan".into()); }
    db.query_row(
        &format!("SELECT {SENTINEL_SCAN_COLUMNS} FROM sentinel_scans WHERE id=?1
          AND (?2 IS NULL OR project_id=?2)
          AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans d WHERE d.scan_id=sentinel_scans.id)"),
        params![scan,project], sentinel_scan_row,
    ).optional().map_err(|e| e.to_string())
}

fn dialog_selection_in(db: &rusqlite::Connection, project: Option<i64>) -> Result<AgentDialogSelection, String> {
    let scope = dialog_selection_scope(db, project)?;
    let stored: Option<(Option<i64>, i64, Option<String>)> = db.query_row(
        "SELECT project_id,revision,scan_id FROM agent_dialog_selections WHERE scope_key=?1",
        [scope], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).optional().map_err(|e| e.to_string())?;
    let Some((stored_project, revision, scan)) = stored else {
        return Ok(AgentDialogSelection { project_id: project, revision: 0,
            selected_scan_id: None, selected_scan: None, selection_unavailable: false });
    };
    if stored_project != project || !(1..=DIALOG_VIEW_MAX_INTEGER).contains(&revision) {
        return Err("dialog_selection_invalid_record".into());
    }
    let selected_scan = match scan.as_deref() {
        Some(id) => dialog_task_in(db, project, id)?,
        None => None,
    };
    Ok(AgentDialogSelection { project_id: project, revision, selected_scan_id: scan,
        selection_unavailable: selected_scan.is_none(), selected_scan })
}

fn load_dialog_selection(db: &rusqlite::Connection, project: Option<i64>) -> Result<AgentDialogSelection, String> {
    let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
    dialog_selection_in(&tx, project)
}

fn persist_dialog_selection(db: &rusqlite::Connection, input: &AgentDialogSelectionInput) -> Result<AgentDialogSelection, String> {
    if !(0..DIALOG_VIEW_MAX_INTEGER).contains(&input.expected_revision) {
        return Err("dialog_selection_invalid_revision".into());
    }
    let tx = rusqlite::Transaction::new_unchecked(db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let scope = dialog_selection_scope(&tx, input.project_id)?;
    if dialog_task_in(&tx, input.project_id, &input.scan_id)?.is_none() {
        return Err("dialog_selection_scan_unavailable".into());
    }
    let current = dialog_selection_in(&tx, input.project_id)?;
    if current.revision != input.expected_revision { return Err("dialog_selection_revision_conflict".into()); }
    let next = current.revision + 1;
    let changed = tx.execute(
        "INSERT INTO agent_dialog_selections(scope_key,project_id,revision,scan_id) VALUES(?1,?2,?3,?4)
         ON CONFLICT(scope_key) DO UPDATE SET revision=excluded.revision,scan_id=excluded.scan_id
         WHERE agent_dialog_selections.revision=?5",
        params![scope,input.project_id,next,input.scan_id,input.expected_revision],
    ).map_err(|e| format!("dialog_selection_save_failed:{e}"))?;
    let saved = dialog_selection_in(&tx, input.project_id)?;
    if changed != 1 || saved.revision != next || saved.selected_scan_id.as_deref() != Some(&input.scan_id)
        || saved.selection_unavailable {
        return Err("dialog_selection_write_not_persisted".into());
    }
    tx.commit().map_err(|e| format!("dialog_selection_commit_unconfirmed:{e}"))?;
    Ok(saved)
}

#[tauri::command]
pub async fn get_agent_dialog_selection(state: State<'_, AppState>, project_id: Option<i64>) -> Result<AgentDialogSelection, String> {
    let path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || load_dialog_selection(&db::open(&path)?, project_id))
        .await.map_err(|e| format!("dialog_selection_read_worker:{e}"))?
}

#[tauri::command]
pub async fn save_agent_dialog_selection(state: State<'_, AppState>, input: AgentDialogSelectionInput) -> Result<AgentDialogSelection, String> {
    let path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || persist_dialog_selection(&db::open(&path)?, &input))
        .await.map_err(|e| format!("dialog_selection_save_worker:{e}"))?
}

#[tauri::command]
pub async fn get_agent_dialog_task(state: State<'_, AppState>, project_id: Option<i64>, scan_id: String) -> Result<Option<SentinelScan>, String> {
    let path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let db = db::open(&path)?;
        let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
        dialog_task_in(&tx, project_id, &scan_id)
    }).await.map_err(|e| format!("dialog_task_read_worker:{e}"))?
}
