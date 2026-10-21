//! Assets-owned compatibility store and public native-application adapter.
//! RPC receives no database, configuration, resource or export path.
use super::{AssetService, AssetServiceEnvironment};
use crate::{
    db,
    jobs::{AssetJobContext, AssetJobEventSink},
    native_app_sdk::{Bootstrap, CallSurface, EventSink, NativeApplication, NativeCallContext},
};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

const STORE: &str = "engine-assets-v1";
const APP: &str = "org.oviraptor.assets";
const SCHEMA:&str="
CREATE TABLE asset_service_store(singleton INTEGER PRIMARY KEY CHECK(singleton=1), instance_id TEXT NOT NULL UNIQUE,schema_version INTEGER NOT NULL CHECK(schema_version=1));
CREATE TABLE asset_service_projects(host_project_id INTEGER PRIMARY KEY,native_project_id INTEGER NOT NULL UNIQUE REFERENCES projects(id),host_name TEXT NOT NULL,host_status TEXT NOT NULL,grant_revision INTEGER NOT NULL,app_revision INTEGER NOT NULL,permissions_json TEXT NOT NULL);
CREATE TABLE asset_service_operations(scope_id INTEGER NOT NULL,operation_id TEXT NOT NULL,arguments_sha256 TEXT NOT NULL,instance_id TEXT NOT NULL REFERENCES asset_service_store(instance_id),surface TEXT NOT NULL CHECK(surface IN ('project','application')),method TEXT NOT NULL,phase TEXT NOT NULL CHECK(phase IN ('pending','completed','uncertain')),outcome_json TEXT,PRIMARY KEY(scope_id,operation_id));
CREATE TRIGGER asset_service_store_no_update BEFORE UPDATE ON asset_service_store BEGIN SELECT RAISE(ABORT,'asset store immutable');END;
CREATE TRIGGER asset_service_store_no_delete BEFORE DELETE ON asset_service_store BEGIN SELECT RAISE(ABORT,'asset store immutable');END;
CREATE TRIGGER asset_service_project_identity_no_update BEFORE UPDATE ON asset_service_projects WHEN NEW.host_project_id<>OLD.host_project_id OR NEW.native_project_id<>OLD.native_project_id BEGIN SELECT RAISE(ABORT,'asset mapping immutable');END;
CREATE TRIGGER asset_service_project_no_delete BEFORE DELETE ON asset_service_projects BEGIN SELECT RAISE(ABORT,'asset mapping immutable');END;
CREATE TRIGGER asset_service_operation_identity_no_update BEFORE UPDATE ON asset_service_operations WHEN OLD.phase<>'pending' OR NEW.scope_id<>OLD.scope_id OR NEW.operation_id<>OLD.operation_id OR NEW.arguments_sha256<>OLD.arguments_sha256 OR NEW.instance_id<>OLD.instance_id OR NEW.surface<>OLD.surface OR NEW.method<>OLD.method OR NEW.phase NOT IN ('completed','uncertain') BEGIN SELECT RAISE(ABORT,'asset operation immutable');END;
CREATE TRIGGER asset_service_operation_no_delete BEFORE DELETE ON asset_service_operations BEGIN SELECT RAISE(ABORT,'asset operation immutable');END;
";
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Marker {
    schema_version: u32,
    app_id: String,
    instance_id: String,
    root: PathBuf,
    device: u64,
    inode: u64,
}
fn file_identity(path: &Path) -> Result<(u64, u64), String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "asset_store_file_unavailable")?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("asset_store_file_invalid".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
            || metadata.nlink() != 1
        {
            return Err("asset_store_file_permissions".into());
        }
        Ok((metadata.dev(), metadata.ino()))
    }
    #[cfg(not(unix))]
    {
        Err("asset_bootstrap_platform_file_identity_unavailable".into())
    }
}

fn connection(path: &Path) -> Result<Connection, String> {
    // CREATE and URI interpretation are deliberately absent. Only first boot's
    // exclusive new directory may reach the original initializer.
    let db = crate::db::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_FULL_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(|_| "asset_store_connection_unavailable")?;
    db.busy_timeout(Duration::from_secs(10))
        .map_err(|_| "asset_store_connection_unavailable")?;
    db.pragma_update(None, "foreign_keys", "ON")
        .map_err(|_| "asset_store_connection_unavailable")?;
    db.pragma_update(None, "synchronous", "FULL")
        .map_err(|_| "asset_store_sync_unavailable")?;
    crate::collaboration_events::install_commit_notifications(&db);
    Ok(db)
}

fn hold_database(path: &Path, expected: (u64, u64)) -> Result<File, String> {
    if file_identity(path)? != expected {
        return Err("asset_store_custody_changed".into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(path)
        .map_err(|_| "asset_store_custody_changed")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let held = file.metadata().map_err(|_| "asset_store_custody_changed")?;
        if (held.dev(), held.ino()) != expected || held.nlink() != 1 {
            return Err("asset_store_custody_changed".into());
        }
    }
    if file_identity(path)? != expected {
        return Err("asset_store_custody_changed".into());
    }
    Ok(file)
}

fn sqlite_not_moved(db: &Connection) -> Result<(), String> {
    let mut moved: std::os::raw::c_int = 0;
    // Same custody check used by the original preparation lease. This inspects
    // SQLite's actual opened file, not just its remembered pathname.
    let checked = unsafe {
        rusqlite::ffi::sqlite3_file_control(
            db.handle(),
            c"main".as_ptr(),
            rusqlite::ffi::SQLITE_FCNTL_HAS_MOVED,
            (&mut moved as *mut std::os::raw::c_int).cast(),
        )
    };
    if checked != rusqlite::ffi::SQLITE_OK || moved != 0 {
        return Err("asset_store_custody_changed".into());
    }
    Ok(())
}

fn require_database_custody(
    db: &Connection,
    path: &Path,
    original: &File,
    expected: (u64, u64),
    instance: Option<&str>,
) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let held = original
            .metadata()
            .map_err(|_| "asset_store_custody_changed")?;
        if (held.dev(), held.ino()) != expected
            || held.nlink() != 1
            || held.uid() != unsafe { libc::geteuid() }
            || held.mode() & 0o077 != 0
        {
            return Err("asset_store_custody_changed".into());
        }
    }
    if file_identity(path).map_err(|_| "asset_store_custody_changed")? != expected
        || db.path().map(Path::new) != Some(path)
        || fs::canonicalize(path).map_err(|_| "asset_store_custody_changed")? != path
    {
        return Err("asset_store_custody_changed".into());
    }
    sqlite_not_moved(db)?;
    if let Some(instance) = instance {
        if super::service_instance_on(db).map_err(|_| "asset_store_original_owner_missing")?
            != instance
        {
            return Err("asset_store_original_owner_mismatch".into());
        }
    }
    // Detect a path switch while the original owner row was being read too.
    if file_identity(path).map_err(|_| "asset_store_custody_changed")? != expected {
        return Err("asset_store_custody_changed".into());
    }
    sqlite_not_moved(db)
}

fn checked_root(path: &Path, private: bool) -> Result<(), String> {
    let m = fs::symlink_metadata(path).map_err(|_| "asset_store_directory_unavailable")?;
    if !path.is_absolute()
        || !m.is_dir()
        || m.file_type().is_symlink()
        || fs::canonicalize(path).map_err(|_| "asset_store_directory_unavailable")? != path
    {
        return Err("asset_store_directory_invalid".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let forbidden = if private { 0o077 } else { 0o022 };
        if m.uid() != unsafe { libc::geteuid() } || m.mode() & forbidden != 0 {
            return Err("asset_store_directory_permissions".into());
        }
    }
    Ok(())
}
fn create_private(path: &Path) -> Result<(), String> {
    match fs::create_dir(path) {
        Ok(()) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o700))
                    .map_err(|_| "asset_store_directory_permissions")?;
            }
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err("asset_store_directory_unavailable".into()),
    }
    checked_root(path, true)
}
struct Events(EventSink);
impl AssetJobEventSink for Events {
    fn publish(&self, name: &str, payload: &Value) -> Result<(), String> {
        if !matches!(name, "job-progress" | "asset-log-committed") {
            return Err("asset_job_event_unavailable".into());
        }
        // Notifications are refresh hints, not receipts or authority. The durable
        // business events/logs remain in this application's private database.
        self.0.notify(name, payload)
    }
}

pub(super) struct Application {
    service: AssetService,
    jobs: AssetJobContext,
    uploads: super::exposure_uploads::UploadStore,
    database: Connection,
    original_database: File,
    db_path: PathBuf,
    identity: (u64, u64),
    instance_id: String,
    stopped: bool,
    _controller: crate::agent_runtime::execution_owner::NativeInvocationOwner,
}
impl Application {
    pub(super) fn open(bootstrap: &Bootstrap) -> Result<Self, String> {
        Self::open_inner(bootstrap, true)
    }
    fn open_inner(bootstrap: &Bootstrap, register: bool) -> Result<Self, String> {
        if !cfg!(unix) {
            return Err("asset_bootstrap_platform_file_identity_unavailable".into());
        }
        if bootstrap.data_schema_version != 1 {
            return Err("asset_service_data_schema_unsupported".into());
        }
        if bootstrap.app_id != APP {
            return Err("asset_bootstrap_application_mismatch".into());
        }
        checked_root(&bootstrap.data_dir, true)?;
        checked_root(&bootstrap.package_dir, false)?;
        // Old Node inventory/drafts, unexpected files and a copied native
        // generation require an owner migration receipt. Never create an empty
        // engine beside them and report successful upgrade with missing assets.
        let entries =
            fs::read_dir(&bootstrap.data_dir).map_err(|_| "asset_store_directory_unavailable")?;
        for entry in entries {
            let entry = entry.map_err(|_| "asset_store_directory_unavailable")?;
            if entry.file_name() != STORE {
                return Err("asset_service_store_migration_required".into());
            }
        }
        let root = bootstrap.data_dir.join(STORE);
        let fresh = match fs::create_dir(&root) {
            Ok(()) => true,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => false,
            Err(_) => return Err("asset_store_directory_unavailable".into()),
        };
        if fresh {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
                    .map_err(|_| "asset_store_directory_permissions")?;
            }
        }
        checked_root(&root, true)?;
        let db_path = root.join("oviraptor.sqlite3");
        let marker_path = root.join("service-store.json");
        let marker = if fresh {
            // A directory created exclusively by this instance is the sole route to
            // the broad compatibility initializer. Existing desktop/copy stores are
            // never adopted or automatically migrated/resigned.
            if db::initialize(&root)? != db_path {
                return Err("asset_store_database_identity_changed".into());
            }
            let identity = file_identity(&db_path)?;
            let original = hold_database(&db_path, identity)?;
            let db = connection(&db_path)?;
            require_database_custody(&db, &db_path, &original, identity, None)?;
            let instance_id = uuid::Uuid::new_v4().to_string();
            let tx = rusqlite::Transaction::new_unchecked(&db, TransactionBehavior::Immediate)
                .map_err(|_| "asset_store_initialization_failed")?;
            tx.execute_batch(SCHEMA)
                .map_err(|_| "asset_store_initialization_failed")?;
            tx.execute(
                "INSERT INTO asset_service_store VALUES(1,?1,1)",
                [&instance_id],
            )
            .map_err(|_| "asset_store_initialization_failed")?;
            require_database_custody(&tx, &db_path, &original, identity, Some(&instance_id))?;
            tx.commit()
                .map_err(|_| "asset_store_initialization_commit_unconfirmed")?;
            require_database_custody(&db, &db_path, &original, identity, Some(&instance_id))?;
            let marker = Marker {
                schema_version: 1,
                app_id: APP.into(),
                instance_id,
                root: root.clone(),
                device: identity.0,
                inode: identity.1,
            };
            let bytes = serde_json::to_vec(&marker).map_err(|_| "asset_store_marker_invalid")?;
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
            }
            let mut file = options
                .open(&marker_path)
                .map_err(|_| "asset_store_marker_unconfirmed")?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|_| "asset_store_marker_unconfirmed")?;
            File::open(&root)
                .and_then(|f| f.sync_all())
                .map_err(|_| "asset_store_marker_unconfirmed")?;
            marker
        } else {
            file_identity(&marker_path)?;
            let mut options = OpenOptions::new();
            options.read(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.custom_flags(libc::O_NOFOLLOW);
            }
            let file = options
                .open(&marker_path)
                .map_err(|_| "asset_store_marker_unavailable")?;
            let mut bytes = Vec::new();
            file.take(4097)
                .read_to_end(&mut bytes)
                .map_err(|_| "asset_store_marker_unavailable")?;
            if bytes.len() > 4096 {
                return Err("asset_store_marker_invalid".into());
            }
            serde_json::from_slice::<Marker>(&bytes).map_err(|_| "asset_store_marker_invalid")?
        };
        if marker.schema_version != 1
            || marker.app_id != APP
            || marker.root != root
            || !uuid::Uuid::parse_str(&marker.instance_id)
                .is_ok_and(|id| id.get_version_num() == 4 && id.to_string() == marker.instance_id)
            || file_identity(&db_path)? != (marker.device, marker.inode)
        {
            return Err("asset_service_store_migration_required".into());
        }
        let identity = (marker.device, marker.inode);
        let original_database = hold_database(&db_path, identity)?;
        let database = connection(&db_path)?;
        require_database_custody(
            &database,
            &db_path,
            &original_database,
            identity,
            Some(&marker.instance_id),
        )?;
        let controller = crate::agent_runtime::execution_owner::claim_native_invocation(
            &db_path,
            "",
            0,
            "assets_service",
            "controller",
        )
        .map_err(|_| "asset_bootstrap_controller_unavailable")?;
        if register {
            // Existing-store helper/recovery opens already use NO_CREATE,
            // NOFOLLOW and the original database commit fence.
            register_process_custody(&db_path, &database, &marker.instance_id)?;
        }
        let export_dir = root.join("exports");
        create_private(&export_dir)?;
        // These roots belong to the app. They are not supplied in model arguments.
        create_private(&root.join("configuration"))?;
        let environment = AssetServiceEnvironment::from_paths(&root, &db_path, &export_dir)?;
        let jobs = AssetJobContext::service(
            &db_path,
            &root,
            marker.instance_id.clone(),
            Arc::new(Events(bootstrap.events.clone())),
        )?;
        let service = AssetService::new(environment).with_jobs(jobs.clone())?;
        let uploads = super::exposure_uploads::UploadStore::open(&database, &root)?;
        let app = Self {
            service,
            jobs,
            uploads,
            database,
            original_database,
            db_path,
            identity,
            instance_id: marker.instance_id,
            stopped: false,
            _controller: controller,
        };
        app.require_custody()?;
        super::preferences_service::initialize_on(&app.database)?;
        super::scan_selection::initialize_on(&app.database)?;
        app.require_custody()?;
        // No process is running in this store while this OS controller lock is ours.
        // Recover only this instance's originated runs, never desktop/import history.
        app.recover_owned_interrupted()?;
        let actual_workers = usize::from(app.jobs.has_local_activity()?)
            + crate::commands::ownership::active_background_workers(&app.db_path)?
            + crate::commands::exposure::active_background_workers(&app.db_path)?;
        app.uploads.recover_after_exclusive_owner(&app.database, &app._controller, actual_workers)?;
        Ok(app)
    }
    fn require_custody(&self) -> Result<(), String> {
        self.service.environment().revalidate()?;
        self.uploads.revalidate(&self.database)?;
        require_database_custody(
            &self.database,
            &self.db_path,
            &self.original_database,
            self.identity,
            Some(&self.instance_id),
        )
    }
    fn recover_owned_interrupted(&self) -> Result<(), String> {
        self.require_custody()?;
        if self.jobs.has_local_activity()? {
            return Err("asset_service_active_owner_present".into());
        }
        let tx =
            rusqlite::Transaction::new_unchecked(&self.database, TransactionBehavior::Immediate)
                .map_err(|_| "asset_service_recovery_failed")?;
        // Match legacy startup's interrupted state, scoped by immutable birth owner.
        tx.execute("UPDATE runs SET status='interrupted',stage='interrupted',error='上次运行时应用或电脑被关闭，任务未完成',finished_at=datetime('now','localtime') WHERE status IN ('queued','running','cancel_requested') AND id IN (SELECT run_id FROM asset_service_job_owners WHERE instance_id=?1)",[&self.instance_id]).map_err(|_|"asset_service_recovery_failed")?;
        tx.execute("UPDATE asset_service_operations SET phase='uncertain' WHERE phase='pending' AND instance_id=?1",[&self.instance_id]).map_err(|_|"asset_service_recovery_failed")?;
        require_database_custody(
            &tx,
            &self.db_path,
            &self.original_database,
            self.identity,
            Some(&self.instance_id),
        )?;
        tx.commit()
            .map_err(|_| "asset_service_recovery_unconfirmed")?;
        self.require_custody()
    }
    fn validate_context(&self, c: &NativeCallContext) -> Result<(), String> {
        if self.stopped {
            return Err("asset_service_stopped".into());
        }
        c.validate(&c.permission)
            .map_err(|_| "asset_service_context_invalid")?;
        self.require_custody()
    }
    fn mapped_project(&self, c: &NativeCallContext) -> Result<i64, String> {
        let mut permissions = c.permissions.clone();
        permissions.sort();
        permissions.dedup();
        let permissions =
            serde_json::to_string(&permissions).map_err(|_| "asset_service_context_invalid")?;
        let tx =
            rusqlite::Transaction::new_unchecked(&self.database, TransactionBehavior::Immediate)
                .map_err(|_| "asset_service_scope_write_failed")?;
        let old:Option<(i64,String,String,i64,i64,String)>=tx.query_row("SELECT native_project_id,host_name,host_status,grant_revision,app_revision,permissions_json FROM asset_service_projects WHERE host_project_id=?1",[c.host_bound_project_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional().map_err(|_|"asset_service_scope_unavailable")?;
        let native_id = if let Some((native_id, name, status, grant, app, old_permissions)) = old {
            if c.grant_revision < grant
                || c.app_revision < app
                || (c.grant_revision == grant
                    && c.app_revision == app
                    && (name != c.project.name
                        || status != c.project.status
                        || old_permissions != permissions))
            {
                return Err("asset_service_scope_revision_changed".into());
            }
            tx.execute("UPDATE asset_service_projects SET host_name=?1,host_status=?2,grant_revision=?3,app_revision=?4,permissions_json=?5 WHERE host_project_id=?6",params![c.project.name,c.project.status,c.grant_revision,c.app_revision,permissions,c.host_bound_project_id]).map_err(|_|"asset_service_scope_write_failed")?;
            native_id
        } else {
            // Native IDs are independently allocated; equal Host/Scan IDs convey no
            // relationship. Namespace names because the legacy table requires UNIQUE.
            tx.execute(
                "INSERT INTO projects(name,status) VALUES(?1,?2)",
                params![
                    format!("[host:{}] {}", c.host_bound_project_id, c.project.name),
                    c.project.status
                ],
            )
            .map_err(|_| "asset_service_scope_write_failed")?;
            let native_id = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO asset_service_projects VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![
                    c.host_bound_project_id,
                    native_id,
                    c.project.name,
                    c.project.status,
                    c.grant_revision,
                    c.app_revision,
                    permissions
                ],
            )
            .map_err(|_| "asset_service_scope_write_failed")?;
            native_id
        };
        tx.execute(
            "UPDATE projects SET name=?1,status=?2 WHERE id=?3",
            params![
                format!("[host:{}] {}", c.host_bound_project_id, c.project.name),
                c.project.status,
                native_id
            ],
        )
        .map_err(|_| "asset_service_scope_write_failed")?;
        require_database_custody(
            &tx,
            &self.db_path,
            &self.original_database,
            self.identity,
            Some(&self.instance_id),
        )?;
        tx.commit()
            .map_err(|_| "asset_service_scope_commit_unconfirmed")?;
        self.require_custody()?;
        Ok(native_id)
    }
    fn operation(
        &self,
        scope: i64,
        c: &NativeCallContext,
        surface: &str,
        name: &str,
        args: &Value,
        perform: impl FnOnce() -> Result<Value, String>,
    ) -> Result<Value, String> {
        let bytes = serde_json::to_vec(&(
            surface,
            name,
            args,
            c.host_bound_project_id,
            c.grant_revision,
            c.app_revision,
        ))
        .map_err(|_| "asset_service_arguments_invalid")?;
        let digest = format!("{:x}", Sha256::digest(bytes));
        let tx =
            rusqlite::Transaction::new_unchecked(&self.database, TransactionBehavior::Immediate)
                .map_err(|_| "asset_service_operation_unavailable")?;
        let old:Option<(String,String,Option<String>)>=tx.query_row("SELECT arguments_sha256,phase,outcome_json FROM asset_service_operations WHERE scope_id=?1 AND operation_id=?2",params![scope,c.operation_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_|"asset_service_operation_unavailable")?;
        if let Some((old_digest, phase, result)) = old {
            if old_digest != digest {
                return Err("asset_service_operation_conflict".into());
            }
            if phase != "completed" {
                return Err("asset_service_operation_uncertain_do_not_replay".into());
            }
            let result: Value =
                serde_json::from_str(&result.ok_or("asset_service_operation_receipt_missing")?)
                    .map_err(|_| "asset_service_operation_receipt_invalid")?;
            return if result["ok"] == true {
                Ok(result["value"].clone())
            } else {
                Err(result["error"]
                    .as_str()
                    .unwrap_or("asset_service_operation_failed")
                    .into())
            };
        }
        tx.execute("INSERT INTO asset_service_operations(scope_id,operation_id,arguments_sha256,instance_id,surface,method,phase) VALUES(?1,?2,?3,?4,?5,?6,'pending')",params![scope,c.operation_id,digest,self.instance_id,surface,name]).map_err(|_|"asset_service_operation_unavailable")?;
        require_database_custody(
            &tx,
            &self.db_path,
            &self.original_database,
            self.identity,
            Some(&self.instance_id),
        )?;
        tx.commit()
            .map_err(|_| "asset_service_operation_claim_unconfirmed")?;
        // The old owner opens its own original transaction. A crash after it commits
        // and before this receipt is finalized remains uncertain, never auto-retried.
        let result = perform();
        self.require_custody()?;
        let outcome = match &result {
            Ok(v) => json!({"ok":true,"value":v}),
            Err(e) => json!({"ok":false,"error":e}),
        };
        let changed=self.database.execute("UPDATE asset_service_operations SET phase='completed',outcome_json=?1 WHERE scope_id=?2 AND operation_id=?3 AND arguments_sha256=?4 AND phase='pending'",params![outcome.to_string(),scope,c.operation_id,digest]).map_err(|_|"asset_service_operation_finalize_uncertain")?;
        if changed != 1 {
            return Err("asset_service_operation_finalize_uncertain".into());
        }
        self.require_custody()?;
        result
    }
}

fn register_process_custody(
    path: &Path,
    database: &Connection,
    instance: &str,
) -> Result<(), String> {
    let instance = instance.to_string();
    db::service_custody::register_process_store(
        path,
        database,
        format!("{APP}:{instance}"),
        move |db| {
            if super::service_instance_on(db).is_ok_and(|actual| actual == instance) {
                Ok(())
            } else {
                Err("asset_store_original_owner_mismatch".into())
            }
        },
    )
}

fn method(surface: CallSurface, name: &str) -> Result<(&'static str, bool, &'static str), String> {
    match surface {
        CallSurface::Tool => match name {
            "saved_assets_stats" | "saved_assets_search" => Ok(("inventory.read", false, "tool")),
            _ => Err("asset_service_tool_unavailable".into()),
        },
        CallSurface::ProjectAction => match name {
            // Selection owns an atomic provenance ledger and revalidates current
            // members on replay; never return the generic action cache instead.
            "prepare_scan_selection" => Ok(("inventory.read",false,"project")),
            name if super::exposure_uploads::action(name).is_some() => Ok(("inventory.exposure.manage",super::exposure_uploads::action(name).unwrap(),"project")),
            "assets_context"
            | "list_asset_job_profiles"
            | "list_targets"
            | "list_assets"
            | "list_runs"
            | "list_logs"
            | "list_asset_events"
            | "list_page_rules"
            | "preview_page_rule"
            | "saved_assets_stats"
            | "saved_assets_search" => Ok(("inventory.read", false, "project")),
            "import_targets"
            | "remove_target"
            | "mark_asset_entry_opened"
            | "update_decision"
            | "update_asset_decisions"
            | "soft_delete_assets"
            | "soft_delete_asset_selections"
            | "acknowledge_interrupted_run" => Ok(("inventory.write", true, "project")),
            "apply_page_rule" | "disable_page_rule" => {
                Ok(("inventory.rules.manage", true, "project"))
            }
            "export_assets" => Ok(("inventory.export", true, "project")),
            "read_asset_artifact" => Ok(("inventory.export", false, "project")),
            "start_job" | "resume_job" | "cancel_job" => {
                Ok(("inventory.jobs.run", true, "project"))
            }
            name => super::secondary_action_permission(name)
                .map(|(p, m)| (p, m, "project"))
                .ok_or_else(|| "asset_service_project_method_unavailable".into()),
        },
        CallSurface::ApplicationAction => match name {
            "get_asset_rule_catalog" => Ok(("inventory.rules.global", false, "application")),
            "add_content_rule" => Ok(("inventory.rules.global", true, "application")),
            "get_asset_preferences" => Ok(("inventory.preferences.manage", false, "application")),
            "save_asset_preferences" => Ok(("inventory.preferences.manage", true, "application")),
            "list_config_profiles" => Ok(("inventory.configuration.manage", false, "application")),
            "import_asset_collection_configuration"
            | "save_config_profile"
            | "delete_config_profile" => {
                Ok(("inventory.configuration.manage", true, "application"))
            }
            name => super::secondary_application_permission(name)
                .map(|(p, m)| (p, m, "application"))
                .ok_or_else(|| "asset_service_application_method_unavailable".into()),
        },
    }
}
/// A past ambiguous synchronous write result is review metadata, not a live
/// worker. This finite exception never covers jobs, other domains or a method
/// added later without an explicit lifecycle review.
fn synchronous_inventory_operation(surface: &str, name: &str) -> bool {
    match surface {
        "project" => matches!(
            name,
            "import_targets"
                | "remove_target"
                | "mark_asset_entry_opened"
                | "update_decision"
                | "update_asset_decisions"
                | "soft_delete_assets"
                | "soft_delete_asset_selections"
                | "acknowledge_interrupted_run"
                | "apply_page_rule"
                | "disable_page_rule"
                | "export_assets"
                | "begin_exposure_import_upload"
                | "put_exposure_import_chunk"
                | "commit_exposure_import_upload"
                | "discard_exposure_import_upload"
        ),
        "application" => matches!(
            name,
            "add_content_rule"
                | "save_asset_preferences"
                | "save_config_profile"
                | "delete_config_profile"
                | "import_asset_collection_configuration"
        ),
        _ => false,
    }
}
impl NativeApplication for Application {
    fn data_schema_version(&self) -> u64 {
        1
    }
    fn call(
        &mut self,
        surface: CallSurface,
        name: &str,
        args: Value,
        c: &NativeCallContext,
    ) -> Result<Value, String> {
        self.validate_context(c)?;
        let (permission, mutating, surface_name) = method(surface, name)?;
        if c.permission != permission {
            return Err("asset_service_permission_denied".into());
        }
        let native_project = self.mapped_project(c)?;
        let perform = || {
            if name == "assets_context" {
                if !args.as_object().is_some_and(|v| v.is_empty()) {
                    return Err("asset_service_arguments_invalid".into());
                }
                return Ok(
                    json!({"hostProjectId":c.host_bound_project_id,"nativeProjectId":native_project,"name":c.project.name,"status":c.project.status}),
                );
            }
            if name=="prepare_scan_selection" {
                return super::scan_selection::prepare_on(&self.database,native_project,c,args.clone());
            }
            if super::exposure_uploads::action(name).is_some() {
                return self.uploads.dispatch(&self.database, native_project, c, name, args.clone(),
                    crate::commands::exposure::imports::validate_metadata,
                    |request| self.service.dispatch_project(native_project,"import_exposure_documents",json!({"projectId":native_project,"request":request})));
            }
            if surface_name == "application" {
                self.service
                    .dispatch_application_for_project(native_project, name, args.clone())
            } else {
                self.service
                    .dispatch_project(native_project, name, args.clone())
            }
        };
        let result = if mutating {
            self.operation(
                if surface_name == "application" {
                    0
                } else {
                    c.host_bound_project_id
                },
                c,
                surface_name,
                name,
                &args,
                perform,
            )
        } else {
            perform()
        };
        self.require_custody()?;
        result
    }
    fn status(&mut self) -> Result<Value, String> {
        self.require_custody()?;
        let mut value = self.jobs.status()?;
        let ownership = super::ownership_service::status_on(&self.db_path)?;
        let actual_exposure_workers = crate::commands::exposure::active_background_workers(&self.db_path)?;
        let all_actual_workers = usize::from(self.jobs.has_local_activity()?)
            + ownership["liveWorkers"].as_u64().ok_or("asset_service_owner_status_unconfirmed")? as usize
            + actual_exposure_workers;
        let proofs=self.uploads.quiesced_import_proofs(&self.database,&self._controller,all_actual_workers)?;
        let exposure=super::owned_work::exposure_status_with_quiesced_imports(&self.db_path,actual_exposure_workers,&proofs)?;
        let uploads=self.uploads.activity_status(&self.database)?;
        for owner in [&ownership, &exposure, &uploads] {
            let busy = owner
                .get("busy")
                .and_then(Value::as_bool)
                .ok_or("asset_service_owner_status_unconfirmed")?;
            let uncertain = owner
                .get("uncertain")
                .and_then(Value::as_bool)
                .ok_or("asset_service_owner_status_unconfirmed")?;
            if busy {
                value["busy"] = json!(true);
            }
            if uncertain {
                value["busy"] = json!(true);
                value["uncertain"] = json!(true);
            }
        }
        let import_review=exposure["operationResultsRequireReview"].as_bool().unwrap_or(false)
            || uploads["operationResultsRequireReview"].as_bool().unwrap_or(false);
        value["ownership"] = ownership;
        value["exposure"] = exposure;
        value["exposureUploads"] = uploads;

        let mut statement = self.database.prepare("SELECT surface,method FROM asset_service_operations WHERE instance_id=?1 AND phase IN ('pending','uncertain')")
            .map_err(|_| "asset_service_operation_status_unavailable")?;
        let rows = statement
            .query_map([&self.instance_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|_| "asset_service_operation_status_unavailable")?;
        let mut unresolved = 0u64;
        let mut unsafe_operations = 0u64;
        for row in rows {
            let (surface, name) = row.map_err(|_| "asset_service_operation_status_unavailable")?;
            unresolved += 1;
            if !synchronous_inventory_operation(&surface, &name) {
                unsafe_operations += 1;
            }
        }
        value["unresolvedOperations"] = json!(unresolved);
        value["operationResultsRequireReview"] = json!(unresolved > 0 || import_review);
        if unsafe_operations > 0 {
            value["busy"] = json!(true);
            value["uncertain"] = json!(true);
        }
        value["stopped"] = json!(self.stopped);
        value["instanceId"] = json!(self.instance_id);
        Ok(value)
    }
    fn stop(&mut self) -> Result<Value, String> {
        let mut value = self.status()?;
        if value["busy"] == false && value["uncertain"] != true {
            self.stopped = true;
        }
        value["stopped"] = json!(self.stopped);
        Ok(value)
    }
}

#[cfg(test)]
#[path = "service_bootstrap_tests.rs"]
mod tests;
