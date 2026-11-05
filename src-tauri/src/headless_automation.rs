//! Headless URL scan automation without Tauri `AppHandle` (CLI + background wait jobs).

use crate::{
    db,
    scanning::service_environment::{install_event_sink, ScanEnvironment, ScanEvent, ScanEventSink},
    tool_supply::ToolAdministratorConfiguration,
    worker_resources,
};
use rusqlite::{params, Connection};
use serde_json::Value as JsonValue;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

struct NullScanEventSink;

impl ScanEventSink for NullScanEventSink {
    fn emit(&self, _event: ScanEvent<'_>) -> Result<(), String> {
        Ok(())
    }
}

fn ensure_cli_event_sink() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        let _ = install_event_sink(Arc::new(NullScanEventSink));
    });
}

/// Desktop-free scan environment: bundled workers + service-root tool config.
pub struct CliScanEnvironment {
    app_data_dir: PathBuf,
    resource_root: PathBuf,
}

impl CliScanEnvironment {
    pub fn from_paths(app_data_dir: PathBuf, resource_root: Option<PathBuf>) -> Self {
        let resource_root = resource_root.unwrap_or_else(resolve_cli_resource_root);
        Self {
            app_data_dir,
            resource_root,
        }
    }

    pub fn from_default_home() -> Result<Self, String> {
        let home = default_oviraptor_data_dir()?;
        Ok(Self::from_paths(home.clone(), None))
    }
}

impl ScanEnvironment for CliScanEnvironment {
    fn frontend_worker(&self) -> Result<PathBuf, String> {
        worker_resources::resolve_frontend_recon_worker_in(&self.resource_root).or_else(|_| {
            worker_resources::resolve_frontend_recon_worker_in(Path::new(env!(
                "CARGO_MANIFEST_DIR"
            )))
        })
    }

    fn tool_configuration(&self) -> Result<ToolAdministratorConfiguration, String> {
        ToolAdministratorConfiguration::from_service_root(&self.app_data_dir)
    }
}

pub fn default_oviraptor_data_dir() -> Result<PathBuf, String> {
    if let Ok(from_env) = std::env::var("OVIRAPTOR_DATA_DIR") {
        let path = PathBuf::from(from_env);
        if path.is_absolute() {
            return Ok(path);
        }
    }
    let home = dirs_home().ok_or("home_dir_unavailable")?;
    Ok(home.join("oviraptor"))
}

pub fn default_database_path() -> Result<PathBuf, String> {
    Ok(default_oviraptor_data_dir()?.join("oviraptor.sqlite3"))
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

pub fn resolve_cli_resource_root() -> PathBuf {
    std::env::var_os("OVIRAPTOR_RESOURCE_ROOT")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

const HEADLESS_JOBS_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS agent_headless_wait_jobs (
    job_id TEXT PRIMARY KEY,
    scan_id TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'running'
        CHECK(status IN ('running','completed','timed_out','failed')),
    timed_out INTEGER NOT NULL DEFAULT 0,
    error_message TEXT NOT NULL DEFAULT '',
    result_json TEXT NOT NULL DEFAULT '{}',
    max_wait_secs INTEGER NOT NULL DEFAULT 900,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    finished_at TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_agent_headless_wait_jobs_scan ON agent_headless_wait_jobs(scan_id, status);
"#;

fn ensure_headless_jobs(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(HEADLESS_JOBS_DDL)
        .map_err(|error| error.to_string())
}

pub fn spawn_headless_wait_job(
    db_path: PathBuf,
    scan_id: String,
    max_wait_secs: u32,
) -> Result<String, String> {
    let job_id = Uuid::new_v4().to_string();
    let connection = db::open(&db_path)?;
    ensure_headless_jobs(&connection)?;
    connection
        .execute(
            "INSERT INTO agent_headless_wait_jobs(job_id,scan_id,status,max_wait_secs) VALUES(?1,?2,'running',?3)",
            params![job_id, scan_id, i64::from(max_wait_secs)],
        )
        .map_err(|error| error.to_string())?;
    let job_for_thread = job_id.clone();
    thread::spawn(move || {
        let _ = run_headless_wait_loop(&db_path, &job_for_thread, &scan_id, max_wait_secs);
    });
    Ok(job_id)
}

fn run_headless_wait_loop(
    db_path: &Path,
    job_id: &str,
    scan_id: &str,
    max_wait_secs: u32,
) -> Result<(), String> {
    let cap = max_wait_secs.min(7200);
    let deadline = Instant::now() + Duration::from_secs(u64::from(cap));
    let mut timed_out = false;
    loop {
        thread::sleep(Duration::from_secs(2));
        let connection = db::open(db_path)?;
        let status = read_scan_status(&connection, scan_id)?;
        if !scan_status_active(&status) {
            break;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            break;
        }
    }
    let connection = db::open(db_path)?;
    let snapshot = headless_snapshot(&connection, scan_id)?;
    let payload = serde_json::json!({
        "jobId": job_id,
        "scanId": scan_id,
        "waited": true,
        "timedOut": timed_out,
        "snapshot": snapshot,
    });
    let status = if timed_out { "timed_out" } else { "completed" };
    connection
        .execute(
            "UPDATE agent_headless_wait_jobs SET status=?1, timed_out=?2, result_json=?3, \
             finished_at=datetime('now','localtime') WHERE job_id=?4",
            params![
                status,
                i64::from(timed_out),
                payload.to_string(),
                job_id,
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub fn read_headless_wait_job(db_path: &Path, job_id: &str) -> Result<JsonValue, String> {
    let connection = db::open(db_path)?;
    ensure_headless_jobs(&connection)?;
    connection
        .query_row(
            "SELECT job_id, scan_id, status, timed_out, error_message, result_json, max_wait_secs, created_at, finished_at \
             FROM agent_headless_wait_jobs WHERE job_id=?1",
            [job_id],
            |row| {
                Ok(serde_json::json!({
                    "jobId": row.get::<_, String>(0)?,
                    "scanId": row.get::<_, String>(1)?,
                    "status": row.get::<_, String>(2)?,
                    "timedOut": row.get::<_, i64>(3)? != 0,
                    "errorMessage": row.get::<_, String>(4)?,
                    "result": serde_json::from_str::<JsonValue>(&row.get::<_, String>(5)?).unwrap_or(JsonValue::Null),
                    "maxWaitSecs": row.get::<_, i64>(6)?,
                    "createdAt": row.get::<_, String>(7)?,
                    "finishedAt": row.get::<_, String>(8)?,
                }))
            },
        )
        .map_err(|_| "headless_job_not_found".into())
}

pub(crate) fn scan_status_active(status: &str) -> bool {
    matches!(status, "scanning" | "pausing")
}

pub(crate) fn read_scan_status(connection: &Connection, scan_id: &str) -> Result<String, String> {
    connection
        .query_row(
            "SELECT status FROM sentinel_scans WHERE id=?1",
            [scan_id],
            |row| row.get(0),
        )
        .map_err(|_| "scan_not_found".into())
}

pub(crate) fn finding_count(connection: &Connection, scan_id: &str) -> i64 {
    connection
        .query_row(
            "SELECT COUNT(*) FROM sentinel_findings WHERE scan_id=?1",
            [scan_id],
            |row| row.get(0),
        )
        .unwrap_or(0)
}

pub(crate) fn headless_snapshot(
    connection: &Connection,
    scan_id: &str,
) -> Result<JsonValue, String> {
    let status = read_scan_status(connection, scan_id)?;
    let stats = crate::scanning::scan_agent_stats_inner(connection, scan_id, None)?;
    Ok(serde_json::json!({
        "scanId": scan_id,
        "status": status,
        "findingCount": finding_count(connection, scan_id),
        "stats": stats,
    }))
}

/// Public entry for the standalone CLI binary.
pub fn run_url_scan_cli(
    db_path: PathBuf,
    app_data_dir: PathBuf,
    project_id: i64,
    task_name: String,
    urls: Vec<String>,
    scan_mode: Option<String>,
    wait: bool,
    max_wait_secs: Option<u32>,
) -> Result<JsonValue, String> {
    ensure_cli_event_sink();
    let environment = CliScanEnvironment::from_paths(app_data_dir.clone(), None);
    crate::scanning::headless_start_url_scan(
        &environment,
        &db_path,
        &app_data_dir,
        project_id,
        task_name,
        urls,
        scan_mode,
        Some(wait),
        max_wait_secs,
        true,
    )
}

#[cfg(test)]
mod cli_env_tests {
    use super::scan_status_active;

    #[test]
    fn active_statuses() {
        assert!(scan_status_active("scanning"));
        assert!(!scan_status_active("completed"));
    }
}
