//! Trusted job paths, process-owned activity and display event transport.
//! The existing asset pipeline and job lifecycle remain the execution owner.
use super::tray_activity;
use crate::{db, AppState};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
};
use tauri::{AppHandle, Emitter};

pub(crate) trait AssetJobEventSink: Send + Sync {
    fn publish(&self, event: &str, payload: &Value) -> Result<(), String>;
}
/// The native call graph contains only these erased capabilities. The concrete
/// desktop handles and vtables are constructed solely by the desktop adapter.
pub(super) trait JobActivityFactory: Send + Sync {
    fn begin(
        &self,
        active_jobs: Arc<AtomicUsize>,
        cancellations: Arc<Mutex<HashMap<i64, Arc<AtomicBool>>>>,
        run_id: i64,
    ) -> tray_activity::JobActivityGuard;
}
struct HeadlessActivity;
impl JobActivityFactory for HeadlessActivity {
    fn begin(
        &self,
        active_jobs: Arc<AtomicUsize>,
        cancellations: Arc<Mutex<HashMap<i64, Arc<AtomicBool>>>>,
        run_id: i64,
    ) -> tray_activity::JobActivityGuard {
        begin_headless_activity(active_jobs, cancellations, run_id)
    }
}
struct DesktopActivity(AppHandle);
impl JobActivityFactory for DesktopActivity {
    fn begin(
        &self,
        active_jobs: Arc<AtomicUsize>,
        cancellations: Arc<Mutex<HashMap<i64, Arc<AtomicBool>>>>,
        run_id: i64,
    ) -> tray_activity::JobActivityGuard {
        super::begin_tray_activity(self.0.clone(), active_jobs, cancellations, run_id)
    }
}
struct DesktopEvents(AppHandle);
impl AssetJobEventSink for DesktopEvents {
    fn publish(&self, event: &str, payload: &Value) -> Result<(), String> {
        self.0
            .emit(event, payload)
            .map_err(|_| "asset_job_notification_unavailable".into())
    }
}
#[derive(Clone)]
pub(super) struct JobEnvironment {
    sink: Option<Arc<dyn AssetJobEventSink>>,
    activity: Arc<dyn JobActivityFactory>,
}
impl JobEnvironment {
    fn desktop(app: AppHandle) -> Self {
        Self {
            sink: Some(Arc::new(DesktopEvents(app.clone()))),
            activity: Arc::new(DesktopActivity(app)),
        }
    }
    fn headless(sink: Option<Arc<dyn AssetJobEventSink>>) -> Self {
        Self {
            sink,
            activity: Arc::new(HeadlessActivity),
        }
    }
    pub(super) fn emit(&self, event: &str, payload: impl Serialize) -> Result<(), String> {
        let value =
            serde_json::to_value(payload).map_err(|_| "asset_job_event_serialization_failed")?;
        match &self.sink {
            Some(sink) => sink
                .publish(event, &value)
                .map_err(|_| "asset_job_notification_unavailable".into()),
            None => Ok(()),
        }
    }
    pub(super) fn begin_activity(
        &self,
        active_jobs: Arc<AtomicUsize>,
        cancellations: Arc<Mutex<HashMap<i64, Arc<AtomicBool>>>>,
        run_id: i64,
    ) -> tray_activity::JobActivityGuard {
        self.activity.begin(active_jobs, cancellations, run_id)
    }
}
pub(crate) const OWNERS_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS asset_service_job_owners(
 run_id INTEGER PRIMARY KEY REFERENCES runs(id),
 instance_id TEXT NOT NULL REFERENCES asset_service_store(instance_id),
 created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE TABLE IF NOT EXISTS asset_service_configuration_files(
 id TEXT PRIMARY KEY,instance_id TEXT NOT NULL REFERENCES asset_service_store(instance_id),content_sha256 TEXT NOT NULL,content_bytes INTEGER NOT NULL
);
CREATE TRIGGER IF NOT EXISTS asset_service_configuration_no_update BEFORE UPDATE ON asset_service_configuration_files
BEGIN SELECT RAISE(ABORT,'asset configuration immutable'); END;
CREATE TRIGGER IF NOT EXISTS asset_service_configuration_no_delete BEFORE DELETE ON asset_service_configuration_files
BEGIN SELECT RAISE(ABORT,'asset configuration immutable'); END;
CREATE TRIGGER IF NOT EXISTS asset_service_job_owner_no_update BEFORE UPDATE ON asset_service_job_owners
BEGIN SELECT RAISE(ABORT,'asset job owner immutable'); END;
CREATE TRIGGER IF NOT EXISTS asset_service_job_owner_no_delete BEFORE DELETE ON asset_service_job_owners
BEGIN SELECT RAISE(ABORT,'asset job owner immutable'); END;
";

#[derive(Clone)]
pub(crate) struct AssetJobContext {
    pub(super) db_path: PathBuf,
    pub(super) app_data_dir: PathBuf,
    pub(super) cancellations: Arc<Mutex<HashMap<i64, Arc<AtomicBool>>>>,
    pub(super) active_jobs: Arc<AtomicUsize>,
    pub(super) environment: JobEnvironment,
    instance_id: Option<String>,
}
impl AssetJobContext {
    pub(super) fn desktop(app: AppHandle, state: &AppState) -> Self {
        Self {
            db_path: state.db_path.clone(),
            app_data_dir: state.app_data_dir.clone(),
            cancellations: state.cancellations.clone(),
            active_jobs: state.active_jobs.clone(),
            environment: JobEnvironment::desktop(app),
            instance_id: None,
        }
    }
    pub(super) fn desktop_without_events(state: &AppState) -> Self {
        Self {
            db_path: state.db_path.clone(),
            app_data_dir: state.app_data_dir.clone(),
            cancellations: state.cancellations.clone(),
            active_jobs: state.active_jobs.clone(),
            environment: JobEnvironment::headless(None),
            instance_id: None,
        }
    }
    pub(crate) fn service(
        db_path: &Path,
        app_data_dir: &Path,
        instance_id: String,
        sink: Arc<dyn AssetJobEventSink>,
    ) -> Result<Self, String> {
        if !db_path.is_absolute()
            || !app_data_dir.is_absolute()
            || instance_id.is_empty()
            || instance_id.len() > 128
            || instance_id.chars().any(char::is_control)
        {
            return Err("asset_job_service_context_invalid".into());
        }
        let connection = db::open(db_path)?;
        if crate::asset_service::service_instance_on(&connection)
            .map_err(|_| "asset_job_service_instance_unavailable")?
            != instance_id
        {
            return Err("asset_job_service_instance_unavailable".into());
        }
        connection
            .execute_batch(OWNERS_SCHEMA)
            .map_err(|_| "asset_job_service_owners_unavailable")?;
        Ok(Self {
            db_path: db_path.into(),
            app_data_dir: app_data_dir.into(),
            cancellations: Arc::new(Mutex::new(HashMap::new())),
            active_jobs: Arc::new(AtomicUsize::new(0)),
            environment: JobEnvironment::headless(Some(sink)),
            instance_id: Some(instance_id),
        })
    }
    /// Called inside the original start transaction before its commit/launch.
    /// No imported/historical run is ever adopted as this process's birth.
    pub(super) fn record_started_on(
        &self,
        connection: &Connection,
        run_id: i64,
    ) -> Result<(), String> {
        if let Some(instance) = &self.instance_id {
            if crate::asset_service::service_instance_on(connection)
                .map_err(|_| "asset_job_service_instance_unavailable")?
                != *instance
            {
                return Err("asset_job_service_instance_changed".into());
            }
            connection
                .execute(
                    "INSERT INTO asset_service_job_owners(run_id,instance_id) VALUES(?1,?2)",
                    params![run_id, instance],
                )
                .map_err(|_| "asset_job_service_owner_not_persisted")?;
        }
        Ok(())
    }
    pub(super) fn require_owned_on(
        &self,
        connection: &Connection,
        run_id: i64,
    ) -> Result<(), String> {
        if let Some(instance) = &self.instance_id {
            let owned:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM asset_service_job_owners WHERE run_id=?1 AND instance_id=?2)",params![run_id,instance],|r|r.get(0))
                .map_err(|_|"asset_job_service_owner_unavailable")?;
            if !owned {
                return Err("asset_job_service_historical_run_not_owned".into());
            }
        }
        Ok(())
    }
    pub(crate) fn recover_pending_terminal_results(&self) -> Result<usize,String> {
        let owners=self.cancellations.lock().map_err(|_|"asset_job_service_lock_unavailable")?;
        let mut connection=db::open(&self.db_path)?;
        super::terminal_recovery::reconcile_bootstrap_on(&mut connection,&self.app_data_dir,&owners.keys().copied().collect(),self.instance_id())
    }
    fn configuration_directory(&self) -> Result<PathBuf, String> {
        let root = self.app_data_dir.join("configuration");
        let metadata =
            fs::symlink_metadata(&root).map_err(|_| "asset_configuration_directory_unavailable")?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || fs::canonicalize(&root).map_err(|_| "asset_configuration_directory_unavailable")?
                != root
        {
            return Err("asset_configuration_directory_invalid".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
                return Err("asset_configuration_directory_permissions".into());
            }
        }
        Ok(root)
    }
    pub(crate) fn import_collection_configuration(&self, content: &str) -> Result<Value, String> {
        let instance = self
            .instance_id
            .as_ref()
            .ok_or("asset_configuration_import_requires_service")?;
        if content.is_empty() || content.len() > 65_536 || content.contains('\0') {
            return Err("asset_configuration_content_invalid".into());
        }
        let root = self.configuration_directory()?;
        let id = uuid::Uuid::new_v4().to_string();
        let path = root.join(format!("{id}.ini"));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options
            .open(&path)
            .map_err(|_| "asset_configuration_import_unavailable")?;
        if file
            .write_all(content.as_bytes())
            .and_then(|_| file.sync_all())
            .is_err()
        {
            let _ = fs::remove_file(&path);
            return Err("asset_configuration_import_unconfirmed".into());
        }
        // Reuse the original parser locally; no provider/account/network call.
        if super::native_config::parse_fofa_key(content).is_none() {
            let _ = fs::remove_file(&path);
            return Err("asset_configuration_fofa_key_missing".into());
        }
        let digest = format!("{:x}", Sha256::digest(content.as_bytes()));
        let connection = db::open(&self.db_path)?;
        let saved=connection.execute("INSERT INTO asset_service_configuration_files(id,instance_id,content_sha256,content_bytes) VALUES(?1,?2,?3,?4)",params![id,instance,digest,content.len() as i64]);
        if !matches!(saved, Ok(1)) {
            let _ = fs::remove_file(&path);
            return Err("asset_configuration_import_unconfirmed".into());
        }
        fs::File::open(root)
            .and_then(|file| file.sync_all())
            .map_err(|_| "asset_configuration_import_unconfirmed")?;
        Ok(json!({"configurationId":id,"fofaKeyConfigured":true}))
    }
    fn configuration_file(&self, id: &str) -> Result<(PathBuf, Vec<u8>), String> {
        let instance = self
            .instance_id
            .as_ref()
            .ok_or("asset_configuration_requires_service")?;
        if !uuid::Uuid::parse_str(id).is_ok_and(|parsed| parsed.to_string() == id) {
            return Err("asset_configuration_id_invalid".into());
        }
        let path = self.configuration_directory()?.join(format!("{id}.ini"));
        let metadata =
            fs::symlink_metadata(&path).map_err(|_| "asset_configuration_file_unavailable")?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || fs::canonicalize(&path).map_err(|_| "asset_configuration_file_unavailable")? != path
            || metadata.len() > 65_536
        {
            return Err("asset_configuration_file_invalid".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.uid() != unsafe { libc::geteuid() }
                || metadata.mode() & 0o077 != 0
                || metadata.nlink() != 1
            {
                return Err("asset_configuration_file_permissions".into());
            }
        }
        let connection = db::open(&self.db_path)?;
        let expected:Option<(String,i64)>=connection.query_row("SELECT content_sha256,content_bytes FROM asset_service_configuration_files WHERE id=?1 AND instance_id=?2",params![id,instance],|row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(|_|"asset_configuration_receipt_unavailable")?;
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let file = options
            .open(&path)
            .map_err(|_| "asset_configuration_file_unavailable")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let opened = file
                .metadata()
                .map_err(|_| "asset_configuration_file_unavailable")?;
            if (opened.dev(), opened.ino(), opened.nlink()) != (metadata.dev(), metadata.ino(), 1) {
                return Err("asset_configuration_file_changed".into());
            }
        }
        let mut bytes = Vec::new();
        file.take(65_537)
            .read_to_end(&mut bytes)
            .map_err(|_| "asset_configuration_file_unavailable")?;
        let observed = (format!("{:x}", Sha256::digest(&bytes)), bytes.len() as i64);
        if expected != Some(observed) {
            return Err("asset_configuration_content_changed".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let after =
                fs::symlink_metadata(&path).map_err(|_| "asset_configuration_file_changed")?;
            if after.file_type().is_symlink()
                || (after.dev(), after.ino(), after.nlink()) != (metadata.dev(), metadata.ino(), 1)
            {
                return Err("asset_configuration_file_changed".into());
            }
        }
        Ok((path, bytes))
    }
    pub(crate) fn configuration_path(&self, id: &str) -> Result<PathBuf, String> {
        Ok(self.configuration_file(id)?.0)
    }
    pub(crate) fn configuration_id(&self, path: &str) -> Result<String, String> {
        let path = Path::new(path);
        let id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or("asset_configuration_file_invalid")?;
        if self.configuration_path(id)? != path {
            return Err("asset_configuration_file_outside_scope".into());
        }
        Ok(id.into())
    }
    pub(super) fn validate_settings(&self, settings: &Value) -> Result<(), String> {
        if self.instance_id.is_some() {
            if let Some(path) = settings
                .get("configPath")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
            {
                self.configuration_id(path)?;
            }
        }
        Ok(())
    }
    pub(super) fn fofa_key(&self, settings: &Value) -> Result<String, String> {
        if self.instance_id.is_none() {
            return super::native_config::fofa_key(settings);
        }
        self.validate_settings(settings)?;
        let explicit = settings
            .get("fofaKey")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if !explicit.is_empty() {
            return super::native_config::fofa_key(&json!({"fofaKey":explicit}));
        }
        let path = settings
            .get("configPath")
            .and_then(Value::as_str)
            .unwrap_or("");
        let id = self.configuration_id(path)?;
        let (_, bytes) = self.configuration_file(&id)?;
        let text =
            std::str::from_utf8(&bytes).map_err(|_| "asset_configuration_content_invalid")?;
        super::native_config::parse_fofa_key(text)
            .ok_or_else(|| "asset_configuration_fofa_key_missing".into())
    }
    pub(super) fn prepare_settings(
        &self,
        settings: &mut Value,
        pipeline: &str,
    ) -> Result<(), String> {
        if self.instance_id.is_some() {
            self.validate_settings(settings)?;
            if pipeline != "reprobe" {
                let key = self.fofa_key(settings)?;
                settings["fofaKey"] = json!(key);
            }
            // The private snapshot never asks a later worker to reopen a path.
            if let Some(object) = settings.as_object_mut() {
                object.insert("configPath".into(), json!(""));
            }
        }
        Ok(())
    }
    pub(crate) fn status(&self) -> Result<Value, String> {
        let mut ids = self
            .cancellations
            .lock()
            .map_err(|_| "asset_job_service_lock_unavailable")?
            .keys()
            .copied()
            .collect::<Vec<_>>();
        ids.sort_unstable();
        let active = self.active_jobs.load(Ordering::SeqCst);
        let mut unresolved = Vec::new();
        if let Some(instance) = &self.instance_id {
            let connection = db::open(&self.db_path)?;
            let mut statement = connection.prepare(
                "SELECT r.id,r.status,o.instance_id FROM runs r LEFT JOIN asset_service_job_owners o ON o.run_id=r.id WHERE r.status NOT IN ('completed','failed','cancelled','interrupted','restarted','historical_import') ORDER BY r.id",
            ).map_err(|_| "asset_job_service_status_unavailable")?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                })
                .map_err(|_| "asset_job_service_status_unavailable")?;
            for row in rows {
                let (run, status, owner) =
                    row.map_err(|_| "asset_job_service_status_unavailable")?;
                if owner.as_deref() != Some(instance.as_str())
                    || !ids.contains(&run)
                    || !matches!(status.as_str(), "queued" | "running" | "cancel_requested")
                {
                    unresolved.push(run);
                }
            }
        }
        let uncertain = !unresolved.is_empty();
        Ok(
            json!({"activeJobs":active,"runIds":ids,"busy":active>0||!ids.is_empty()||uncertain,"uncertain":uncertain,"unresolvedJobs":unresolved.len(),"unresolvedRunIds":unresolved.into_iter().take(128).collect::<Vec<_>>()}),
        )
    }
    /// Only for bootstrap after acquiring the store's exclusive OS controller.
    /// This does not prove durable run completion and is not a stop receipt.
    pub(crate) fn has_local_activity(&self) -> Result<bool, String> {
        Ok(self.active_jobs.load(Ordering::SeqCst) > 0
            || !self
                .cancellations
                .lock()
                .map_err(|_| "asset_job_service_lock_unavailable")?
                .is_empty())
    }
    pub(crate) fn busy(&self) -> Result<bool, String> {
        Ok(self.status()?["busy"].as_bool().unwrap_or(true))
    }
    pub(crate) fn database_path(&self) -> &Path {
        &self.db_path
    }
    pub(crate) fn instance_id(&self) -> Option<&str> {
        self.instance_id.as_deref()
    }
}

/// Headless jobs use exactly the same guard cleanup and active counter, without
/// starting a GUI thread/tray animator. The desktop branch stays unchanged.
pub(super) fn begin_headless_activity(
    active_jobs: Arc<AtomicUsize>,
    cancellations: Arc<Mutex<HashMap<i64, Arc<AtomicBool>>>>,
    run_id: i64,
) -> tray_activity::JobActivityGuard {
    tray_activity::JobActivityGuard::begin(active_jobs, move |_| {
        cancellations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&run_id);
    })
    .0
}

#[cfg(test)]
#[path = "service_environment_tests.rs"]
mod tests;
