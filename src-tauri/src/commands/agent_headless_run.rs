// Headless / automation entry: create a URL scan draft, confirm start, optionally
// wait in a background thread (desktop IPC) or block (CLI).

use crate::headless_automation::{
    headless_snapshot, spawn_headless_wait_job, read_headless_wait_job, scan_status_active,
    read_scan_status,
};

pub(crate) fn headless_start_url_scan(
    environment: &dyn ScanEnvironment,
    db_path: &Path,
    app_data_dir: &Path,
    project_id: i64,
    task_name: String,
    urls: Vec<String>,
    scan_mode: Option<String>,
    wait: Option<bool>,
    max_wait_secs: Option<u32>,
    blocking_wait: bool,
) -> Result<JsonValue, String> {
    if urls.is_empty() {
        return Err("headless_urls_empty".into());
    }
    let connection = db::open(db_path)?;
    let scan = create_sentinel_url_scan_with_mode_in(
        &connection,
        project_id,
        task_name,
        urls,
        scan_mode,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )?;
    crate::scanning::scan_detail_store::provision_new_scan(db_path, app_data_dir, &scan.id)?;
    let scan_id = scan.id.clone();
    let started = start_web_scan_with_environment(
        environment,
        db_path,
        app_data_dir,
        scan_id.clone(),
        WebStartMode::Confirm,
    )?;
    let should_wait = wait.unwrap_or(false);
    if !should_wait {
        let connection = db::open(db_path)?;
        return headless_snapshot(&connection, &scan_id).map(|snapshot| {
            serde_json::json!({
                "started": true,
                "waited": false,
                "scan": started,
                "snapshot": snapshot,
            })
        });
    }
    let cap = max_wait_secs.unwrap_or(900).min(7200);
    if !blocking_wait {
        let job_id = spawn_headless_wait_job(db_path.to_path_buf(), scan_id.clone(), cap)?;
        let connection = db::open(db_path)?;
        let snapshot = headless_snapshot(&connection, &scan_id)?;
        return Ok(serde_json::json!({
            "started": true,
            "waited": false,
            "waitMode": "background",
            "jobId": job_id,
            "maxWaitSecs": cap,
            "scan": started,
            "snapshot": snapshot,
        }));
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(u64::from(cap));
    loop {
        std::thread::sleep(Duration::from_secs(2));
        let connection = db::open(db_path)?;
        let status = read_scan_status(&connection, &scan_id)?;
        if !scan_status_active(&status) {
            let snapshot = headless_snapshot(&connection, &scan_id)?;
            return Ok(serde_json::json!({
                "started": true,
                "waited": true,
                "waitMode": "blocking",
                "timedOut": false,
                "scan": started,
                "snapshot": snapshot,
            }));
        }
        if std::time::Instant::now() >= deadline {
            let snapshot = headless_snapshot(&connection, &scan_id)?;
            return Ok(serde_json::json!({
                "started": true,
                "waited": true,
                "waitMode": "blocking",
                "timedOut": true,
                "scan": started,
                "snapshot": snapshot,
            }));
        }
    }
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn run_headless_url_scan(
    app: tauri::AppHandle,
    state: State<AppState>,
    project_id: i64,
    task_name: String,
    urls: Vec<String>,
    scan_mode: Option<String>,
    wait: Option<bool>,
    max_wait_secs: Option<u32>,
) -> Result<JsonValue, String> {
    let environment = DesktopScanEnvironment::new(&app);
    headless_start_url_scan(
        &environment,
        &state.db_path,
        &state.app_data_dir,
        project_id,
        task_name,
        urls,
        scan_mode,
        wait,
        max_wait_secs,
        false,
    )
}

#[tauri::command]
pub fn get_scan_headless_snapshot(
    state: State<AppState>,
    scan_id: String,
) -> Result<JsonValue, String> {
    let connection = db::open(&state.db_path)?;
    headless_snapshot(&connection, &scan_id)
}

#[tauri::command]
pub fn get_headless_wait_job(state: State<AppState>, job_id: String) -> Result<JsonValue, String> {
    read_headless_wait_job(&state.db_path, &job_id)
}

#[tauri::command]
pub fn export_project_agent_stats(
    state: State<AppState>,
    project_id: i64,
    limit: Option<i64>,
) -> Result<String, String> {
    let limit = limit.unwrap_or(50).clamp(1, 500);
    let connection = db::open(&state.db_path)?;
    let mut statement = connection
        .prepare(
            "SELECT id, task_name, status, total_tokens, llm_requests, input_tokens, output_tokens, cached_tokens, updated_at \
             FROM sentinel_scans WHERE project_id=?1 AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans d WHERE d.scan_id=sentinel_scans.id) \
             ORDER BY updated_at DESC LIMIT ?2",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![project_id, limit], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, String>(8)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let mut items = Vec::new();
    for (id, task_name, status, total, requests, input, output, cached, updated_at) in rows {
        let findings = crate::headless_automation::finding_count(&connection, &id);
        let tool_total: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM tool_invocations t JOIN agent_runs r ON r.id=t.run_id WHERE r.scan_id=?1",
                [&id],
                |row| row.get(0),
            )
            .unwrap_or(0);
        items.push(serde_json::json!({
            "scanId": id,
            "taskName": task_name,
            "status": status,
            "updatedAt": updated_at,
            "tokens": {
                "total": total,
                "input": input,
                "output": output,
                "cached": cached,
                "modelRequests": requests,
            },
            "findingCount": findings,
            "toolInvocations": tool_total,
        }));
    }
    serde_json::to_string_pretty(&serde_json::json!({
        "schemaVersion": 1,
        "projectId": project_id,
        "recordedAt": chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        "scans": items,
    }))
    .map_err(|error| error.to_string())
}

