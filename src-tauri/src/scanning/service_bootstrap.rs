//! Preparation for the standalone native process, not the public SDK v2 runner.
//! New service-owned URL drafts only. No start/resume/retry/model/target method.
use super::service_environment::ScanServiceEnvironment;
use crate::db;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, HashMap},
    fs::{self, File, OpenOptions},
    io::{self, BufRead, Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

const MAX_FRAME: usize = 128 * 1024;
const MAX_RESULT: usize = 64 * 1024;
const MAX_INTEGER: i64 = 9_007_199_254_740_991;
const STORE: &str = "engine-bootstrap-v1";
const SCHEMA: &str = "
CREATE TABLE scan_service_store(instance_id TEXT PRIMARY KEY, schema_version INTEGER NOT NULL CHECK(schema_version=1));
CREATE TABLE scan_service_operations(project_id INTEGER NOT NULL REFERENCES projects(id),operation_id TEXT NOT NULL,arguments_sha256 TEXT NOT NULL,instance_id TEXT NOT NULL REFERENCES scan_service_store(instance_id),scan_id TEXT NOT NULL UNIQUE REFERENCES sentinel_scans(id),receipt_json TEXT NOT NULL,PRIMARY KEY(project_id,operation_id));
CREATE TRIGGER scan_service_store_no_update BEFORE UPDATE ON scan_service_store BEGIN SELECT RAISE(ABORT,'service store immutable');END;
CREATE TRIGGER scan_service_store_no_delete BEFORE DELETE ON scan_service_store BEGIN SELECT RAISE(ABORT,'service store immutable');END;
CREATE TRIGGER scan_service_operation_no_update BEFORE UPDATE ON scan_service_operations BEGIN SELECT RAISE(ABORT,'service operation immutable');END;
CREATE TRIGGER scan_service_operation_no_delete BEFORE DELETE ON scan_service_operations BEGIN SELECT RAISE(ABORT,'service operation immutable');END;
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
    let metadata = fs::symlink_metadata(path).map_err(|_| "scan_store_file_unavailable")?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("scan_store_file_invalid".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
            || metadata.nlink() != 1
        {
            return Err("scan_store_file_permissions".into());
        }
        Ok((metadata.dev(), metadata.ino()))
    }
    #[cfg(not(unix))]
    {
        Err("scan_bootstrap_platform_file_identity_unavailable".into())
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
    .map_err(|_| "scan_store_connection_unavailable")?;
    db.busy_timeout(Duration::from_secs(10))
        .map_err(|_| "scan_store_connection_unavailable")?;
    db.pragma_update(None, "foreign_keys", "ON")
        .map_err(|_| "scan_store_connection_unavailable")?;
    db.pragma_update(None, "synchronous", "FULL")
        .map_err(|_| "scan_store_sync_unavailable")?;
    crate::collaboration_events::install_commit_notifications(&db);
    Ok(db)
}

fn hold_database(path: &Path, expected: (u64, u64)) -> Result<File, String> {
    if file_identity(path)? != expected {
        return Err("scan_store_custody_changed".into());
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
        .map_err(|_| "scan_store_custody_changed")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let held = file.metadata().map_err(|_| "scan_store_custody_changed")?;
        if (held.dev(), held.ino()) != expected || held.nlink() != 1 {
            return Err("scan_store_custody_changed".into());
        }
    }
    if file_identity(path)? != expected {
        return Err("scan_store_custody_changed".into());
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
        return Err("scan_store_custody_changed".into());
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
            .map_err(|_| "scan_store_custody_changed")?;
        if (held.dev(), held.ino()) != expected
            || held.nlink() != 1
            || held.uid() != unsafe { libc::geteuid() }
            || held.mode() & 0o077 != 0
        {
            return Err("scan_store_custody_changed".into());
        }
    }
    if file_identity(path).map_err(|_| "scan_store_custody_changed")? != expected
        || db.path().map(Path::new) != Some(path)
        || fs::canonicalize(path).map_err(|_| "scan_store_custody_changed")? != path
    {
        return Err("scan_store_custody_changed".into());
    }
    sqlite_not_moved(db)?;
    if let Some(instance) = instance {
        let owners: Vec<(String, i64)> = db
            .prepare("SELECT instance_id,schema_version FROM main.scan_service_store")
            .and_then(|mut statement| {
                statement
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect()
            })
            .map_err(|_| "scan_store_original_owner_missing")?;
        if owners != vec![(instance.into(), 1)] {
            return Err("scan_store_original_owner_mismatch".into());
        }
    }
    // Detect a path switch while the original owner row was being read too.
    if file_identity(path).map_err(|_| "scan_store_custody_changed")? != expected {
        return Err("scan_store_custody_changed".into());
    }
    sqlite_not_moved(db)
}

struct Service {
    _environment: ScanServiceEnvironment,
    // Keep SQLite and the original file for the whole controller lifetime. Drop
    // them before the controller lock; closing SQLite can finish a checkpoint.
    database: Connection,
    original_database: File,
    db_path: PathBuf,
    identity: (u64, u64),
    instance_id: String,
    scopes: HashMap<i64, HostProject>,
    stopped: bool,
    _controller: crate::agent_runtime::execution_owner::NativeInvocationOwner,
}

impl Service {
    fn open(environment: ScanServiceEnvironment) -> Result<Self, String> {
        if !cfg!(unix) {
            return Err("scan_bootstrap_platform_file_identity_unavailable".into());
        }
        let root = environment.data_root().join(STORE);
        let fresh = match fs::create_dir(&root) {
            Ok(()) => true,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => false,
            Err(_) => return Err("scan_store_directory_unavailable".into()),
        };
        if fresh {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
                    .map_err(|_| "scan_store_directory_permissions")?;
            }
        }
        let metadata =
            fs::symlink_metadata(&root).map_err(|_| "scan_store_directory_unavailable")?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || fs::canonicalize(&root).map_err(|_| "scan_store_directory_unavailable")? != root
        {
            return Err("scan_store_directory_invalid".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
                return Err("scan_store_directory_permissions".into());
            }
        }
        let db_path = root.join("oviraptor.sqlite3");
        let marker_path = root.join("service-store.json");
        let marker = if fresh {
            // Only the newly and exclusively created directory reaches the broad
            // compatibility initializer. Existing stores are never auto-migrated.
            let actual = db::initialize(&root)?;
            if actual != db_path {
                return Err("scan_store_database_identity_changed".into());
            }
            let (device, inode) = file_identity(&db_path)?;
            let instance_id = uuid::Uuid::new_v4().to_string();
            let original = hold_database(&db_path, (device, inode))?;
            let db = connection(&db_path)?;
            require_database_custody(&db, &db_path, &original, (device, inode), None)?;
            let tx = rusqlite::Transaction::new_unchecked(&db, TransactionBehavior::Immediate)
                .map_err(|_| "scan_store_initialization_failed")?;
            require_database_custody(&tx, &db_path, &original, (device, inode), None)?;
            tx.execute_batch(SCHEMA)
                .map_err(|_| "scan_store_initialization_failed")?;
            tx.execute(
                "INSERT INTO scan_service_store VALUES(?1,1)",
                [&instance_id],
            )
            .map_err(|_| "scan_store_initialization_failed")?;
            require_database_custody(
                &tx,
                &db_path,
                &original,
                (device, inode),
                Some(&instance_id),
            )?;
            tx.commit()
                .map_err(|_| "scan_store_initialization_commit_unconfirmed")?;
            require_database_custody(
                &db,
                &db_path,
                &original,
                (device, inode),
                Some(&instance_id),
            )?;
            let marker = Marker {
                schema_version: 1,
                app_id: "org.oviraptor.scanning".into(),
                instance_id,
                root: root.clone(),
                device,
                inode,
            };
            let bytes = serde_json::to_vec(&marker).map_err(|_| "scan_store_marker_invalid")?;
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
            }
            let mut file = options
                .open(&marker_path)
                .map_err(|_| "scan_store_marker_unconfirmed")?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|_| "scan_store_marker_unconfirmed")?;
            #[cfg(unix)]
            fs::File::open(&root)
                .and_then(|file| file.sync_all())
                .map_err(|_| "scan_store_marker_unconfirmed")?;
            marker
        } else {
            // A foreign desktop DB, incomplete first boot or missing marker is a
            // refusal. Never run initialize, adopt rows, or repair it to fit.
            file_identity(&marker_path)?;
            let mut options = OpenOptions::new();
            options.read(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.custom_flags(libc::O_NOFOLLOW);
            }
            let mut file = options
                .open(&marker_path)
                .map_err(|_| "scan_store_marker_unavailable")?;
            let mut bytes = Vec::new();
            (&mut file)
                .take(4097)
                .read_to_end(&mut bytes)
                .map_err(|_| "scan_store_marker_unavailable")?;
            if bytes.len() > 4096 {
                return Err("scan_store_marker_invalid".into());
            }
            serde_json::from_slice::<Marker>(&bytes).map_err(|_| "scan_store_marker_invalid")?
        };
        if marker.schema_version != 1
            || marker.app_id != "org.oviraptor.scanning"
            || marker.root != root
            || uuid::Uuid::parse_str(&marker.instance_id).is_err()
            || file_identity(&db_path)? != (marker.device, marker.inode)
        {
            return Err("scan_store_original_identity_mismatch".into());
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
        // Controller ownership is a separate OS lock, not a task lease or a
        // permission. One process owns host-scope state for this private store.
        // The original lock mechanism preserves its inode and releases on exit.
        let controller = crate::agent_runtime::execution_owner::claim_native_invocation(
            &db_path,
            "",
            0,
            "scanning_service",
            "bootstrap",
        )
        .map_err(|_| "scan_bootstrap_controller_unavailable")?;
        let service = Self {
            _environment: environment,
            database,
            original_database,
            db_path,
            identity,
            instance_id: marker.instance_id,
            scopes: HashMap::new(),
            stopped: false,
            _controller: controller,
        };
        service.require_custody(&service.database)?;
        Ok(service)
    }

    fn require_custody(&self, db: &Connection) -> Result<(), String> {
        require_database_custody(
            db,
            &self.db_path,
            &self.original_database,
            self.identity,
            Some(&self.instance_id),
        )
    }

    fn register_process_custody(&self) -> Result<(), String> {
        self.require_custody(&self.database)?;
        let instance = self.instance_id.clone();
        crate::db::service_custody::register_process_store(
            &self.db_path,
            &self.database,
            format!("org.oviraptor.scanning:{instance}"),
            move |db| {
                let owners: Vec<(String, i64)> = db
                    .prepare("SELECT instance_id,schema_version FROM main.scan_service_store")
                    .and_then(|mut statement| {
                        statement
                            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                            .collect()
                    })
                    .map_err(|_| "scan_store_original_owner_missing")?;
                if owners == vec![(instance.clone(), 1)] {
                    Ok(())
                } else {
                    Err("scan_store_original_owner_mismatch".into())
                }
            },
        )
    }

    fn set_project(&mut self, project: HostProject) -> Result<Value, String> {
        project.validate()?;
        self.require_custody(&self.database)?;
        if let Some(old) = self.scopes.get(&project.project_id) {
            if project.grant_revision < old.grant_revision
                || (project.grant_revision == old.grant_revision && old != &project)
            {
                return Err("scan_host_scope_revision_changed".into());
            }
        }
        if self.scopes.get(&project.project_id) == Some(&project) {
            return Ok(
                json!({"projectId":project.project_id,"grantRevision":project.grant_revision,"executionGranted":false}),
            );
        }
        let tx =
            rusqlite::Transaction::new_unchecked(&self.database, TransactionBehavior::Immediate)
                .map_err(|_| "scan_host_scope_write_failed")?;
        self.require_custody(&tx)?;
        // Host names need not be unique; the original engine requires unique
        // project names. Keep a deterministic, unambiguous host-ID prefix.
        let engine_name = format!("[host:{}] {}", project.project_id, project.name);
        tx.execute("INSERT INTO projects(id,name,status) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET name=excluded.name,status=excluded.status",
            params![project.project_id,engine_name,project.status]).map_err(|_| "scan_host_scope_write_failed")?;
        let actual: (String, String) = tx
            .query_row(
                "SELECT name,status FROM projects WHERE id=?1",
                [project.project_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|_| "scan_host_scope_write_unconfirmed")?;
        if actual != (engine_name, project.status.clone()) {
            return Err("scan_host_scope_write_unconfirmed".into());
        }
        self.require_custody(&tx)?;
        tx.commit()
            .map_err(|_| "scan_host_scope_commit_unconfirmed")?;
        self.require_custody(&self.database)?;
        let result = json!({"projectId":project.project_id,"grantRevision":project.grant_revision,"executionGranted":false});
        self.scopes.insert(project.project_id, project);
        Ok(result)
    }

    fn authorize(&self, context: &Context, permission: &str) -> Result<(), String> {
        context.validate()?;
        let project = self
            .scopes
            .get(&context.host_bound_project_id)
            .ok_or("scan_host_scope_unavailable")?;
        if project.status != "active"
            || project.grant_revision != context.grant_revision
            || !project.permissions.contains(permission)
        {
            return Err("scan_host_scope_permission_denied".into());
        }
        Ok(())
    }

    fn create(&self, context: &Context, input: NewDraft) -> Result<Value, String> {
        self.authorize(context, "draft.write")?;
        let expected_targets = input.validate()?;
        self.require_custody(&self.database)?;
        let arguments = serde_json::to_vec(&input).map_err(|_| "scan_arguments_invalid")?;
        let hash = format!("{:x}", Sha256::digest(arguments));
        let db = &self.database;
        let old:Option<(String,String)>=db.query_row("SELECT arguments_sha256,receipt_json FROM scan_service_operations WHERE project_id=?1 AND operation_id=?2",
            params![context.host_bound_project_id,context.operation_id],|row| Ok((row.get(0)?,row.get(1)?))).optional().map_err(|_| "scan_operation_lookup_failed")?;
        if let Some((previous, receipt)) = old {
            if previous != hash {
                return Err("scan_operation_conflict".into());
            }
            self.require_custody(db)?;
            return serde_json::from_str(&receipt)
                .map_err(|_| "scan_operation_receipt_invalid".into());
        }
        let mut receipt = None;
        super::create_sentinel_url_scan_publishing_in(
            &db,
            context.host_bound_project_id,
            input.title,
            input.targets,
            input.scan_mode,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            false,
            |tx| self.require_custody(tx),
            |tx, scan| {
                self.require_custody(tx)?;
                let actual: Vec<String> = tx
                    .prepare("SELECT url FROM main.sentinel_targets WHERE scan_id=?1 ORDER BY url")
                    .and_then(|mut statement| {
                        statement.query_map([&scan.id], |row| row.get(0))?.collect()
                    })
                    .map_err(|_| "scan_target_publication_unconfirmed")?;
                if actual != expected_targets {
                    return Err("scan_target_publication_changed".into());
                }
                // A competing process can only cause this entire original draft
                // transaction to roll back; it cannot commit an orphan draft.
                let value = json!({"kind":"original_draft_creation_receipt","liveState":false,"taskId":scan.id,
                    "hostBoundProjectId":context.host_bound_project_id,"status":"draft","executionStarted":false,"executionAvailable":false});
                let encoded = value.to_string();
                tx.execute("INSERT INTO scan_service_operations(project_id,operation_id,arguments_sha256,instance_id,scan_id,receipt_json) VALUES(?1,?2,?3,?4,?5,?6)",
                    params![context.host_bound_project_id,context.operation_id,hash,self.instance_id,scan.id,encoded])
                    .map_err(|_| "scan_operation_publication_conflict")?;
                let confirmed:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM scan_service_operations WHERE project_id=?1 AND operation_id=?2 AND arguments_sha256=?3 AND instance_id=?4 AND scan_id=?5 AND receipt_json=?6)",
                    params![context.host_bound_project_id,context.operation_id,hash,self.instance_id,scan.id,encoded],|row|row.get(0)).map_err(|_| "scan_operation_publication_unconfirmed")?;
                if !confirmed {
                    return Err("scan_operation_publication_unconfirmed".into());
                }
                self.require_custody(tx)?;
                receipt = Some(value);
                Ok(())
            },
        )?;
        self.require_custody(db)?;
        receipt.ok_or_else(|| "scan_operation_publication_unconfirmed".into())
    }

    fn read(&self, context: &Context, task: &str) -> Result<Value, String> {
        self.authorize(context, "draft.read")?;
        valid_text(task, 128, false)?;
        self.require_custody(&self.database)?;
        let db = &self.database;
        let record:Option<(String,String,i64)>=db.query_row("SELECT s.status,s.task_name,s.attempt_count FROM scan_service_operations o JOIN sentinel_scans s ON s.id=o.scan_id WHERE o.instance_id=?1 AND o.project_id=?2 AND s.project_id=?2 AND s.id=?3 AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans d WHERE d.scan_id=s.id)",
            params![self.instance_id,context.host_bound_project_id,task],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().map_err(|_| "scan_task_read_failed")?;
        let (status, title, attempt) = record.ok_or("scan_task_unavailable")?;
        let title = crate::agent_runtime::secrets::redact_text_with(&title, None);
        self.require_custody(db)?;
        Ok(
            json!({"taskId":task,"hostBoundProjectId":context.host_bound_project_id,"status":status,"title":title,
            "attemptNumber":attempt,"executionAvailable":false,"asOf":chrono::Utc::now().to_rfc3339()}),
        )
    }

    fn status(&self) -> Result<Value, String> {
        self.require_custody(&self.database)?;
        let db = &self.database;
        // Conservative bootstrap-only stop policy: no engine execution method
        // exists yet. Any mature runtime state requires a future owner-aware
        // controller; never call its mere terminal label proof of safe exit.
        let busy:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE status<>'draft' OR attempt_count<>0 OR task_path<>'') OR EXISTS(SELECT 1 FROM agent_runs) OR EXISTS(SELECT 1 FROM native_branch_dispatches) OR EXISTS(SELECT 1 FROM sentinel_processes) OR EXISTS(SELECT 1 FROM analyzer_container_receipts)",[],|row|row.get(0)).map_err(|_|"scan_bootstrap_busy_state_unknown")?;
        let drafts: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM scan_service_operations WHERE instance_id=?1",
                [&self.instance_id],
                |row| row.get(0),
            )
            .map_err(|_| "scan_bootstrap_status_unavailable")?;
        self.require_custody(db)?;
        Ok(
            json!({"adapter":"scanning-bootstrap-v1","instanceId":self.instance_id,"newDrafts":drafts,"busy":busy,
            "executionAvailable":false,"nativeSdkIntegrationAvailable":false,"schemaOwnership":"private_legacy_compatibility_schema","oldTaskAdoptionAllowed":false}),
        )
    }
}

#[derive(Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HostProject {
    project_id: i64,
    name: String,
    status: String,
    grant_revision: i64,
    permissions: BTreeSet<String>,
}
impl HostProject {
    fn validate(&self) -> Result<(), String> {
        if !(1..=MAX_INTEGER).contains(&self.project_id)
            || !(0..=MAX_INTEGER).contains(&self.grant_revision)
            || !matches!(self.status.as_str(), "active" | "archived")
            || self
                .permissions
                .iter()
                .any(|p| !matches!(p.as_str(), "draft.read" | "draft.write"))
        {
            return Err("scan_host_scope_invalid".into());
        }
        valid_text(&self.name, 120, false)
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Context {
    host_bound_project_id: i64,
    grant_revision: i64,
    operation_id: String,
}
impl Context {
    fn validate(&self) -> Result<(), String> {
        if !(1..=MAX_INTEGER).contains(&self.host_bound_project_id)
            || !(0..=MAX_INTEGER).contains(&self.grant_revision)
        {
            return Err("scan_context_invalid".into());
        }
        valid_text(&self.operation_id, 128, false)
    }
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NewDraft {
    title: String,
    targets: Vec<String>,
    scan_mode: Option<String>,
}
impl NewDraft {
    fn validate(&self) -> Result<Vec<String>, String> {
        valid_text(&self.title, 120, false)?;
        if self.targets.is_empty()
            || self.targets.len() > 32
            || self
                .scan_mode
                .as_deref()
                .is_some_and(|m| !matches!(m, "quick" | "standard" | "deep"))
        {
            return Err("scan_arguments_invalid".into());
        }
        let mut normalized = BTreeSet::new();
        for target in &self.targets {
            valid_text(target, 2048, false)?;
            // Bootstrap input is a literal URL, never the desktop paste-list
            // format. Preserve desktop normalization while rejecting every
            // separator/trim wrapper that could change this element's meaning.
            if target
                .chars()
                .any(|c| c.is_whitespace() || matches!(c, ',' | '，' | ';' | '；'))
                || target.trim_matches(|c| matches!(c, '"' | '\'' | '<' | '>' | '，' | '。'))
                    != target
            {
                return Err("scan_arguments_invalid".into());
            }
            let literal = target.trim_end_matches('/');
            let url = reqwest::Url::parse(literal).map_err(|_| "scan_arguments_invalid")?;
            if !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.fragment().is_some()
            {
                return Err("scan_arguments_invalid".into());
            }
            normalized.insert(literal.to_string());
        }
        if normalized.is_empty() || normalized.len() > 32 {
            return Err("scan_arguments_invalid".into());
        }
        Ok(normalized.into_iter().collect())
    }
}

fn valid_text(value: &str, max: usize, empty: bool) -> Result<(), String> {
    if value.len() > max
        || value.chars().any(char::is_control)
        || (!empty && value.trim().is_empty())
    {
        Err("scan_arguments_invalid".into())
    } else {
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    jsonrpc: String,
    id: Value,
    method: String,
    params: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    name: String,
    arguments: Value,
    context: Context,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReadTask {
    task_id: String,
}
fn bounded_id(id: Value) -> Option<Value> {
    match &id {
        Value::String(text)
            if !text.is_empty() && text.len() <= 128 && !text.chars().any(char::is_control) =>
        {
            Some(id)
        }
        Value::Number(number)
            if number
                .as_i64()
                .is_some_and(|n| (-MAX_INTEGER..=MAX_INTEGER).contains(&n)) =>
        {
            Some(id)
        }
        _ => None,
    }
}

fn response(service: &mut Service, frame: &[u8]) -> Value {
    if frame.len() > MAX_FRAME {
        return failure(Value::Null, "scan_protocol_frame_invalid");
    }
    let request = match serde_json::from_slice::<Request>(frame) {
        Ok(v) => v,
        Err(_) => return failure(Value::Null, "scan_protocol_invalid"),
    };
    // Invalid IDs are never echoed, even on an error path.
    let id = match bounded_id(request.id) {
        Some(id) => id,
        None => return failure(Value::Null, "scan_protocol_invalid"),
    };
    if request.jsonrpc != "2.0" {
        return failure(id, "scan_protocol_invalid");
    }
    if service.stopped && !matches!(request.method.as_str(), "describe" | "service/status") {
        return failure(id, "scan_service_stopped");
    }
    let result: Result<Value, String> = (|| match request.method.as_str() {
        "describe" if request.params == json!({"adapter":"scanning-bootstrap-v1"}) => Ok(
            json!({"adapter":"scanning-bootstrap-v1","protocolVersion":1,"executionAvailable":false,"nativeSdkIntegrationAvailable":false,"tools":["engine_draft_create","engine_draft_read"],"controls":["host/project","service/status","service/stop"]}),
        ),
        "host/project" => service.set_project(
            serde_json::from_value(request.params).map_err(|_| "scan_protocol_invalid")?,
        ),
        "service/status" if request.params == json!({}) => service.status(),
        "service/stop" if request.params == json!({}) => {
            let value = service.status()?;
            if value["busy"] == false {
                service.stopped = true;
            }
            Ok(value)
        }
        "tool/call" => {
            let call: Call =
                serde_json::from_value(request.params).map_err(|_| "scan_protocol_invalid")?;
            match call.name.as_str() {
                "engine_draft_create" => service.create(
                    &call.context,
                    serde_json::from_value(call.arguments).map_err(|_| "scan_arguments_invalid")?,
                ),
                "engine_draft_read" => {
                    let args: ReadTask = serde_json::from_value(call.arguments)
                        .map_err(|_| "scan_arguments_invalid")?;
                    service.read(&call.context, &args.task_id)
                }
                _ => Err("scan_execution_not_migrated".into()),
            }
        }
        _ => Err("scan_method_unavailable".into()),
    })();
    match result {
        Ok(value) => envelope(id, Ok(value)),
        Err(code) => failure(id, &code),
    }
}

fn safe_error(code: &str) -> &str {
    if code.len() <= 96
        && code.starts_with("scan_")
        && code.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
    {
        code
    } else {
        "scan_domain_operation_rejected"
    }
}

fn envelope(id: Value, result: Result<Value, &str>) -> Value {
    let id = bounded_id(id).unwrap_or(Value::Null);
    let value = match result {
        Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
        Err(code) => {
            let code = safe_error(code);
            json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":code,"data":{"code":code}}})
        }
    };
    // Budget the entire encoded response, including JSON escapes and NDJSON LF,
    // uniformly for successes and every error branch.
    if serde_json::to_vec(&value).is_ok_and(|bytes| bytes.len() + 1 <= MAX_RESULT) {
        return value;
    }
    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":"scan_result_too_large","data":{"code":"scan_result_too_large"}}})
}

fn failure(id: Value, code: &str) -> Value {
    envelope(id, Err(code))
}

fn encoded_frame(value: &Value) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec(value).map_err(|_| "scan_protocol_write_failed")?;
    if bytes.len() + 1 > MAX_RESULT {
        return Err("scan_protocol_response_budget_failed".into());
    }
    bytes.push(b'\n');
    Ok(bytes)
}

/// Direct trusted-bootstrap CLI only, pending SDK/native process versioning.
/// `host/project` is a parent-only control; it must never be registered as a
/// model/page tool. Exclusive stdio is not authentication for a public socket.
pub fn run_stdio() -> Result<(), String> {
    if let Some(code) = crate::tool_supply::container_parser::guardian_entry() {
        // Preserve the original guardian stdout/stderr and exact exit status.
        std::process::exit(code);
    }
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 6
        || args[0] != "--data-dir"
        || args[2] != "--configuration-dir"
        || args[4] != "--resource-dir"
    {
        return Err("scan_bootstrap_arguments_invalid".into());
    }
    let environment = ScanServiceEnvironment::from_roots(
        Path::new(&args[1]),
        Path::new(&args[3]),
        Path::new(&args[5]),
    )?;
    let mut service = Service::open(environment)?;
    // Only the actual independent process registers a permanent singleton.
    // Unit fixtures retain local stores; public processes cannot hot-switch or
    // drop this guard while a spawned mature-runtime thread can still reenter.
    service.register_process_custody()?;
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    loop {
        let mut frame = Vec::new();
        let n = (&mut input)
            .take((MAX_FRAME + 1) as u64)
            .read_until(b'\n', &mut frame)
            .map_err(|_| "scan_protocol_read_failed")?;
        if n == 0 {
            break;
        }
        if frame.len() > MAX_FRAME || !frame.ends_with(b"\n") {
            return Err("scan_protocol_frame_invalid".into());
        }
        let result = response(&mut service, &frame);
        let encoded = encoded_frame(&result)?;
        output
            .write_all(&encoded)
            .and_then(|_| output.flush())
            .map_err(|_| "scan_protocol_write_failed")?;
        if service.stopped {
            break;
        }
    }
    // No execution is admitted by this binary yet. If physical runtime state
    // nevertheless appears, EOF cannot be interpreted as a successful stop.
    while service.status()?.get("busy") == Some(&Value::Bool(true)) {
        std::thread::sleep(Duration::from_secs(1));
    }
    Ok(())
}

#[cfg(test)]
#[path = "service_bootstrap_tests.rs"]
mod tests;
