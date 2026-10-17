//! Assets-owned bounded staging. Original Exposure import validation and SQL stay authoritative.
//! No caller paths, network, model, scripts, generic file API, or Host database.
use crate::native_app_sdk::NativeCallContext;
use base64::Engine;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

pub(super) const CHUNK_BYTES: usize = 192 * 1024;
const FILE_BYTES: usize = 2 * 1024 * 1024;
const BATCH_BYTES: usize = 8 * 1024 * 1024;
const RESERVED_BYTES: i64 = 64 * 1024 * 1024;
const TTL_MS: i64 = 24 * 60 * 60 * 1000;
const METADATA_BYTES: usize = 1024 * 1024;
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS asset_exposure_uploads(
 upload_id TEXT PRIMARY KEY,instance_id TEXT NOT NULL,project_id INTEGER NOT NULL,host_project_id INTEGER NOT NULL,
 grant_revision INTEGER NOT NULL,app_revision INTEGER NOT NULL,scope_sha256 TEXT NOT NULL,cleanup_scope_sha256 TEXT NOT NULL,
 operation_id TEXT NOT NULL,manifest_sha256 TEXT NOT NULL,manifest_bytes INTEGER NOT NULL,
 total_bytes INTEGER NOT NULL,phase TEXT NOT NULL CHECK(phase IN ('receiving','discarded','expired','committing','completed','failed','uncertain')),
 created_ms INTEGER NOT NULL,expires_ms INTEGER NOT NULL,directory_dev INTEGER NOT NULL,directory_ino INTEGER NOT NULL,
 cleanup_pending INTEGER NOT NULL DEFAULT 1,recovered_after_owner_exit INTEGER NOT NULL DEFAULT 0,outcome_json TEXT);
CREATE TABLE IF NOT EXISTS asset_exposure_upload_files(
 upload_id TEXT NOT NULL REFERENCES asset_exposure_uploads(upload_id),file_index INTEGER NOT NULL,
 bytes INTEGER NOT NULL,sha256 TEXT NOT NULL,chunk_count INTEGER NOT NULL,PRIMARY KEY(upload_id,file_index));
CREATE TABLE IF NOT EXISTS asset_exposure_upload_chunks(
 upload_id TEXT NOT NULL REFERENCES asset_exposure_uploads(upload_id),file_index INTEGER NOT NULL,chunk_index INTEGER NOT NULL,
 bytes INTEGER NOT NULL,sha256 TEXT NOT NULL,device INTEGER NOT NULL,inode INTEGER NOT NULL,
 PRIMARY KEY(upload_id,file_index,chunk_index));
CREATE TABLE IF NOT EXISTS asset_exposure_upload_claims(
 instance_id TEXT NOT NULL,project_id INTEGER NOT NULL,operation_id TEXT NOT NULL,manifest_sha256 TEXT NOT NULL,
 upload_id TEXT NOT NULL UNIQUE REFERENCES asset_exposure_uploads(upload_id),PRIMARY KEY(instance_id,project_id,operation_id));
CREATE TABLE IF NOT EXISTS asset_exposure_upload_births(
 upload_id TEXT PRIMARY KEY REFERENCES asset_exposure_uploads(upload_id),run_id INTEGER NOT NULL UNIQUE,batch_id INTEGER NOT NULL UNIQUE,
 project_id INTEGER NOT NULL,channel TEXT NOT NULL,payload_hash TEXT NOT NULL,created_at TEXT NOT NULL,
 plan_created_at TEXT NOT NULL,sources_json TEXT NOT NULL,planned_items INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS asset_exposure_upload_invalidations(upload_id TEXT PRIMARY KEY,reason TEXT NOT NULL);
CREATE TRIGGER IF NOT EXISTS asset_exposure_upload_identity_immutable BEFORE UPDATE ON asset_exposure_uploads
 WHEN NEW.upload_id<>OLD.upload_id OR NEW.instance_id<>OLD.instance_id OR NEW.project_id<>OLD.project_id OR NEW.host_project_id<>OLD.host_project_id
 OR NEW.grant_revision<>OLD.grant_revision OR NEW.app_revision<>OLD.app_revision OR NEW.scope_sha256<>OLD.scope_sha256
 OR NEW.cleanup_scope_sha256<>OLD.cleanup_scope_sha256 OR NEW.operation_id<>OLD.operation_id OR NEW.manifest_sha256<>OLD.manifest_sha256 OR NEW.manifest_bytes<>OLD.manifest_bytes
 OR NEW.total_bytes<>OLD.total_bytes OR NEW.created_ms<>OLD.created_ms OR NEW.expires_ms<>OLD.expires_ms
 OR NEW.directory_dev<>OLD.directory_dev OR NEW.directory_ino<>OLD.directory_ino
 BEGIN SELECT RAISE(ABORT,'upload identity immutable');END;
CREATE TRIGGER IF NOT EXISTS asset_exposure_upload_phase_immutable BEFORE UPDATE ON asset_exposure_uploads
 WHEN (NEW.phase<>OLD.phase AND NOT ((OLD.phase='receiving' AND NEW.phase IN ('committing','discarded','expired')) OR (OLD.phase='committing' AND NEW.phase IN ('completed','failed','uncertain'))))
 OR NEW.recovered_after_owner_exit<OLD.recovered_after_owner_exit OR NEW.cleanup_pending>OLD.cleanup_pending
 OR (OLD.outcome_json IS NOT NULL AND NEW.outcome_json IS NOT OLD.outcome_json)
 BEGIN SELECT RAISE(ABORT,'upload phase immutable');END;
CREATE TRIGGER IF NOT EXISTS asset_exposure_file_no_update BEFORE UPDATE ON asset_exposure_upload_files BEGIN SELECT RAISE(ABORT,'upload file immutable');END;
CREATE TRIGGER IF NOT EXISTS asset_exposure_file_no_delete BEFORE DELETE ON asset_exposure_upload_files BEGIN SELECT RAISE(ABORT,'upload file immutable');END;
CREATE TRIGGER IF NOT EXISTS asset_exposure_chunk_no_update BEFORE UPDATE ON asset_exposure_upload_chunks BEGIN SELECT RAISE(ABORT,'upload chunk immutable');END;
CREATE TRIGGER IF NOT EXISTS asset_exposure_chunk_no_delete BEFORE DELETE ON asset_exposure_upload_chunks BEGIN SELECT RAISE(ABORT,'upload chunk immutable');END;
CREATE TRIGGER IF NOT EXISTS asset_exposure_upload_no_delete BEFORE DELETE ON asset_exposure_uploads BEGIN SELECT RAISE(ABORT,'upload receipt immutable');END;
CREATE TRIGGER IF NOT EXISTS asset_exposure_claim_no_update BEFORE UPDATE ON asset_exposure_upload_claims BEGIN SELECT RAISE(ABORT,'upload claim immutable');END;
CREATE TRIGGER IF NOT EXISTS asset_exposure_claim_no_delete BEFORE DELETE ON asset_exposure_upload_claims BEGIN SELECT RAISE(ABORT,'upload claim immutable');END;
CREATE TRIGGER IF NOT EXISTS asset_exposure_birth_no_update BEFORE UPDATE ON asset_exposure_upload_births BEGIN SELECT RAISE(ABORT,'upload birth immutable');END;
CREATE TRIGGER IF NOT EXISTS asset_exposure_birth_no_delete BEFORE DELETE ON asset_exposure_upload_births BEGIN SELECT RAISE(ABORT,'upload birth immutable');END;
CREATE TRIGGER IF NOT EXISTS asset_exposure_invalidation_no_update BEFORE UPDATE ON asset_exposure_upload_invalidations BEGIN SELECT RAISE(ABORT,'upload invalidation immutable');END;
CREATE TRIGGER IF NOT EXISTS asset_exposure_invalidation_no_delete BEFORE DELETE ON asset_exposure_upload_invalidations BEGIN SELECT RAISE(ABORT,'upload invalidation immutable');END;
CREATE TRIGGER IF NOT EXISTS asset_exposure_upload_import_birth AFTER INSERT ON exposure_import_batches
 WHEN EXISTS(SELECT 1 FROM asset_exposure_upload_claims c JOIN asset_exposure_uploads u ON u.upload_id=c.upload_id
 WHERE c.project_id=NEW.project_id AND c.operation_id=NEW.operation_id AND u.phase='committing' AND u.instance_id=c.instance_id)
 BEGIN INSERT INTO asset_exposure_upload_births
 SELECT c.upload_id,NEW.run_id,NEW.id,NEW.project_id,NEW.channel,NEW.payload_hash,NEW.created_at,p.created_at,p.sources_json,p.planned_items
 FROM asset_exposure_upload_claims c JOIN asset_exposure_uploads u ON u.upload_id=c.upload_id JOIN exposure_run_plans p ON p.run_id=NEW.run_id
 WHERE c.project_id=NEW.project_id AND c.operation_id=NEW.operation_id AND u.phase='committing' AND u.instance_id=c.instance_id
 AND p.project_id=NEW.project_id AND p.attempt_number=1 AND p.scope_version=1 AND p.budget_requests=0 AND p.budget_bytes=0;
 END;
CREATE TRIGGER IF NOT EXISTS asset_exposure_upload_plan_reused AFTER UPDATE ON exposure_run_plans
 WHEN NEW.attempt_number IS NOT OLD.attempt_number OR NEW.project_id IS NOT OLD.project_id OR NEW.scope_version IS NOT OLD.scope_version
 OR NEW.scope_hash IS NOT OLD.scope_hash OR NEW.options_hash IS NOT OLD.options_hash OR NEW.sources_json IS NOT OLD.sources_json
 OR NEW.budget_requests IS NOT OLD.budget_requests OR NEW.budget_bytes IS NOT OLD.budget_bytes
 OR NEW.settings_version IS NOT OLD.settings_version OR NEW.settings_json IS NOT OLD.settings_json
 BEGIN INSERT OR IGNORE INTO asset_exposure_upload_invalidations SELECT upload_id,'plan_changed' FROM asset_exposure_upload_births WHERE run_id=NEW.run_id;END;
CREATE TRIGGER IF NOT EXISTS asset_exposure_upload_run_reused AFTER UPDATE ON exposure_runs
 WHEN NEW.project_id IS NOT OLD.project_id OR NEW.created_at IS NOT OLD.created_at
 OR (NEW.status IN ('queued','running') AND (NEW.stage NOT IN ('planned','executing') OR OLD.status NOT IN ('queued','running')))
 BEGIN INSERT OR IGNORE INTO asset_exposure_upload_invalidations SELECT upload_id,'run_reused' FROM asset_exposure_upload_births WHERE run_id=NEW.id;END;
CREATE TRIGGER IF NOT EXISTS asset_exposure_upload_item_added AFTER INSERT ON exposure_work_items
 BEGIN INSERT OR IGNORE INTO asset_exposure_upload_invalidations SELECT upload_id,'item_added' FROM asset_exposure_upload_births WHERE run_id=NEW.run_id;END;
CREATE TRIGGER IF NOT EXISTS asset_exposure_upload_item_reused AFTER UPDATE ON exposure_work_items
 WHEN NEW.run_id IS NOT OLD.run_id OR NEW.project_id IS NOT OLD.project_id OR NEW.attempt_number IS NOT OLD.attempt_number
 OR NEW.work_key IS NOT OLD.work_key OR NEW.kind IS NOT OLD.kind OR NEW.source_key IS NOT OLD.source_key
 OR NEW.asset_id IS NOT OLD.asset_id OR NEW.origin IS NOT OLD.origin OR NEW.entity IS NOT OLD.entity OR NEW.input_hash IS NOT OLD.input_hash
 OR NEW.attempts>1 OR NEW.claim_version>1 OR (NEW.owner<>'' AND NEW.owner NOT IN (SELECT 'exposure-import-'||u.operation_id FROM asset_exposure_upload_births b JOIN asset_exposure_uploads u ON u.upload_id=b.upload_id WHERE b.run_id=NEW.run_id))
 BEGIN INSERT OR IGNORE INTO asset_exposure_upload_invalidations SELECT upload_id,'item_reused' FROM asset_exposure_upload_births WHERE run_id IN (OLD.run_id,NEW.run_id);END;
"#;
pub(super) fn action(name: &str) -> Option<bool> {
    match name {
        "begin_exposure_import_upload"
        | "put_exposure_import_chunk"
        | "commit_exposure_import_upload"
        | "discard_exposure_import_upload" => Some(true),
        "exposure_import_upload_status" => Some(false),
        _ => None,
    }
}
fn error(code: &str) -> String {
    format!("asset_exposure_upload_{code}")
}
fn input<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|_| error("arguments_invalid"))
}
fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn valid_sha(v: &str) -> bool {
    v.len() == 64
        && v.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn uuid(v: &str) -> Result<(), String> {
    if uuid::Uuid::parse_str(v).is_ok_and(|id| id.get_version_num() == 4 && id.to_string() == v) {
        Ok(())
    } else {
        Err(error("id_invalid"))
    }
}
fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[derive(Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    operation_id: String,
    channel: String,
    operator: String,
    source_declaration: String,
    #[serde(default)]
    declared_url: String,
    #[serde(default)]
    batch_label: String,
    #[serde(default)]
    public_material: bool,
    #[serde(default)]
    public_material_note: String,
    #[serde(default)]
    extracted_from: String,
}
#[derive(Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FileSpec {
    name: String,
    format: String,
    bytes: usize,
    sha256: String,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    request: Request,
    files: Vec<FileSpec>,
}
impl Manifest {
    fn request(&self, bodies: Option<Vec<Vec<u8>>>) -> Value {
        let mut v = serde_json::to_value(&self.request).expect("serializable fixed DTO");
        v["files"]=json!(self.files.iter().enumerate().map(|(i,f)|json!({"name":f.name,"format":f.format,"contentBase64":bodies.as_ref().map(|b|base64::engine::general_purpose::STANDARD.encode(&b[i])).unwrap_or_default()})).collect::<Vec<_>>());
        v
    }
    fn validate(&self) -> Result<usize, String> {
        if self.files.is_empty() || self.files.len() > 20 {
            return Err(error("file_count"));
        }
        let mut sum = 0usize;
        for f in &self.files {
            if f.bytes > FILE_BYTES || !valid_sha(&f.sha256) {
                return Err(error("file_metadata_invalid"));
            }
            sum = sum
                .checked_add(f.bytes)
                .ok_or_else(|| error("batch_too_large"))?;
        }
        if sum > BATCH_BYTES {
            return Err(error("batch_too_large"));
        }
        Ok(sum)
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UploadArg {
    upload_id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FinalArg {
    upload_id: String,
    expected_manifest_digest: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ChunkArg {
    upload_id: String,
    file_index: usize,
    chunk_index: usize,
    content_base64: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Identity {
    device: u64,
    inode: u64,
}
fn identity(file: &File, directory: bool) -> Result<Identity, String> {
    let m = file.metadata().map_err(|_| error("custody"))?;
    if m.is_dir() != directory || (!directory && !m.is_file()) {
        return Err(error("custody"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if m.uid() != unsafe { libc::geteuid() }
            || m.mode() & 0o077 != 0
            || (!directory && m.nlink() != 1)
        {
            return Err(error("custody"));
        }
        Ok(Identity {
            device: m.dev(),
            inode: m.ino(),
        })
    }
    #[cfg(not(unix))]
    {
        Err(error("platform_unsupported"))
    }
}
struct Directory {
    file: File,
    path: PathBuf,
    id: Identity,
}
impl Directory {
    fn root(path: &Path) -> Result<Self, String> {
        if !path.is_absolute() || fs::canonicalize(path).map_err(|_| error("custody"))? != path {
            return Err(error("custody"));
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW);
        }
        let file = options.open(path).map_err(|_| error("custody"))?;
        let id = identity(&file, true)?;
        let result = Self {
            file,
            path: path.into(),
            id,
        };
        result.check()?;
        Ok(result)
    }
    fn check(&self) -> Result<(), String> {
        if fs::canonicalize(&self.path).map_err(|_| error("custody"))? != self.path
            || identity(&self.file, true)? != self.id
        {
            return Err(error("custody"));
        }
        let other = Self::open_path(&self.path)?;
        if identity(&other, true)? != self.id {
            return Err(error("custody"));
        }
        Ok(())
    }
    fn open_path(path: &Path) -> Result<File, String> {
        let mut o = OpenOptions::new();
        o.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW);
        }
        o.open(path).map_err(|_| error("custody"))
    }
    fn child(&self, name: &str, create: bool) -> Result<Self, String> {
        self.check()?;
        let name = std::ffi::CString::new(name).map_err(|_| error("id_invalid"))?;
        #[cfg(unix)]
        {
            use std::os::fd::{AsRawFd, FromRawFd};
            if create {
                let n = unsafe { libc::mkdirat(self.file.as_raw_fd(), name.as_ptr(), 0o700) };
                if n != 0
                    && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists
                {
                    return Err(error("storage"));
                }
            }
            let fd = unsafe {
                libc::openat(
                    self.file.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                return Err(error("custody"));
            }
            let file = unsafe { File::from_raw_fd(fd) };
            let id = identity(&file, true)?;
            let out = Self {
                file,
                path: self
                    .path
                    .join(name.to_str().map_err(|_| error("id_invalid"))?),
                id,
            };
            out.check()?;
            self.check()?;
            Ok(out)
        }
        #[cfg(not(unix))]
        {
            let _ = (name, create);
            Err(error("platform_unsupported"))
        }
    }
    fn open(&self, name: &str, create: bool) -> Result<File, String> {
        self.check()?;
        let name = std::ffi::CString::new(name).map_err(|_| error("id_invalid"))?;
        #[cfg(unix)]
        {
            use std::os::fd::{AsRawFd, FromRawFd};
            let flags = if create {
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL
            } else {
                libc::O_RDONLY
            };
            let fd = unsafe {
                libc::openat(
                    self.file.as_raw_fd(),
                    name.as_ptr(),
                    flags | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                    0o600,
                )
            };
            if fd < 0 {
                return Err(error("file_unavailable"));
            }
            let file = unsafe { File::from_raw_fd(fd) };
            identity(&file, false)?;
            self.check()?;
            Ok(file)
        }
        #[cfg(not(unix))]
        {
            let _ = (name, create);
            Err(error("platform_unsupported"))
        }
    }
    fn remove(&self, name: &str) -> Result<(), String> {
        self.check()?;
        let name = std::ffi::CString::new(name).map_err(|_| error("id_invalid"))?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let result = unsafe { libc::unlinkat(self.file.as_raw_fd(), name.as_ptr(), 0) };
            if result != 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::NotFound
            {
                return Err(error("cleanup_failed"));
            }
            self.file.sync_all().map_err(|_| error("cleanup_failed"))?;
            self.check()
        }
        #[cfg(not(unix))]
        {
            let _ = name;
            Err(error("platform_unsupported"))
        }
    }
    fn write(&self, name: &str, bytes: &[u8]) -> Result<File, String> {
        let mut f = self.open(name, true)?;
        f.write_all(bytes)
            .and_then(|_| f.sync_all())
            .and_then(|_| self.file.sync_all())
            .map_err(|_| error("storage"))?;
        self.check()?;
        Ok(f)
    }
    fn read(
        &self,
        name: &str,
        max: usize,
        expected: Option<Identity>,
    ) -> Result<(Vec<u8>, Identity), String> {
        let mut f = self.open(name, false)?;
        let before = identity(&f, false)?;
        if expected.is_some_and(|e| e != before) {
            return Err(error("custody"));
        }
        let old = f.metadata().map_err(|_| error("custody"))?;
        let mut bytes = Vec::new();
        (&mut f)
            .take((max + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| error("storage"))?;
        let after = f.metadata().map_err(|_| error("custody"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if old.len() != after.len()
                || old.mtime() != after.mtime()
                || old.mtime_nsec() != after.mtime_nsec()
                || old.ctime() != after.ctime()
                || old.ctime_nsec() != after.ctime_nsec()
            {
                return Err(error("custody"));
            }
        }
        if bytes.len() > max || identity(&f, false)? != before {
            return Err(error("file_invalid"));
        }
        let current = self.open(name, false)?;
        if identity(&current, false)? != before {
            return Err(error("custody"));
        }
        self.check()?;
        Ok((bytes, before))
    }
}

pub(super) struct UploadStore {
    data: Directory,
    configuration: Directory,
    root: Directory,
    instance: String,
    live: Arc<AtomicUsize>,
}
struct Live(Arc<AtomicUsize>);
impl Drop for Live {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
#[derive(Debug)]
struct Receipt {
    scope: String,
    cleanup_scope: String,
    manifest: String,
    manifest_bytes: usize,
    total: usize,
    phase: String,
    expires: i64,
    id: Identity,
    outcome: Option<String>,
}
impl UploadStore {
    pub(super) fn open(db: &Connection, data_root: &Path) -> Result<Self, String> {
        if !db.is_autocommit() {
            return Err(error("transaction_not_current"));
        }
        let instance = super::service_instance_on(db)?;
        let data = Directory::root(data_root)?;
        let configuration = data.child("configuration", false)?;
        let root = data.child("exposure-uploads-v1", true)?;
        crate::commands::exposure::state::ensure_exposure_schema(db)
            .map_err(|_| error("schema"))?;
        db.execute_batch(SCHEMA).map_err(|_| error("schema"))?;
        Ok(Self {
            data,
            configuration,
            root,
            instance,
            live: Arc::new(AtomicUsize::new(0)),
        })
    }
    pub(super) fn revalidate(&self, db: &Connection) -> Result<(), String> {
        self.data.check()?;
        self.configuration.check()?;
        self.root.check()?;
        if super::service_instance_on(db)? != self.instance {
            return Err(error("instance_changed"));
        }
        Ok(())
    }
    fn scope(
        &self,
        db: &Connection,
        project: i64,
        c: &NativeCallContext,
    ) -> Result<String, String> {
        self.revalidate(db)?;
        c.validate("inventory.exposure.manage")
            .map_err(|_| error("permission_denied"))?;
        if c.permission != "inventory.exposure.manage" || project <= 0 {
            return Err(error("scope"));
        }
        let active: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1 AND status='active')",
                [project],
                |r| r.get(0),
            )
            .map_err(|_| error("scope"))?;
        if !active {
            return Err(error("scope"));
        }
        let mut permissions = c.permissions.clone();
        permissions.sort();
        permissions.dedup();
        Ok(sha(&serde_json::to_vec(&(
            self.instance.as_str(),
            project,
            c.host_bound_project_id,
            c.grant_revision,
            c.app_revision,
            &c.project.name,
            &c.project.status,
            permissions,
            self.data.id.device,
            self.data.id.inode,
            self.configuration.id.device,
            self.configuration.id.inode,
            self.root.id.device,
            self.root.id.inode,
        ))
        .map_err(|_| error("scope"))?))
    }
    fn cleanup_scope(&self, project: i64, c: &NativeCallContext) -> Result<String, String> {
        Ok(sha(&serde_json::to_vec(&(
            self.instance.as_str(),
            project,
            c.host_bound_project_id,
            self.data.id.device,
            self.data.id.inode,
            self.configuration.id.device,
            self.configuration.id.inode,
            self.root.id.device,
            self.root.id.inode,
        ))
        .map_err(|_| error("scope"))?))
    }
    fn stored_receipt(&self, db: &Connection, id: &str) -> Result<Receipt, String> {
        uuid(id)?;
        let value=db.query_row("SELECT scope_sha256,manifest_sha256,manifest_bytes,total_bytes,phase,expires_ms,directory_dev,directory_ino,outcome_json,cleanup_scope_sha256 FROM asset_exposure_uploads WHERE upload_id=?1 AND instance_id=?2",params![id,self.instance],|r|Ok(Receipt{scope:r.get(0)?,manifest:r.get(1)?,manifest_bytes:r.get::<_,i64>(2)?as usize,total:r.get::<_,i64>(3)?as usize,phase:r.get(4)?,expires:r.get(5)?,id:Identity{device:r.get::<_,i64>(6)?as u64,inode:r.get::<_,i64>(7)?as u64},outcome:r.get(8)?,cleanup_scope:r.get(9)?})).optional().map_err(|_|error("receipt"))?.ok_or_else(||error("unavailable"))?;
        Ok(value)
    }
    fn receipt(&self, db: &Connection, id: &str, scope: &str) -> Result<Receipt, String> {
        let value = self.stored_receipt(db, id)?;
        if value.scope != scope {
            return Err(error("scope_changed"));
        }
        Ok(value)
    }
    fn directory(&self, id: &str, receipt: &Receipt) -> Result<Directory, String> {
        let d = self.root.child(id, false)?;
        if d.id != receipt.id {
            return Err(error("custody"));
        }
        Ok(d)
    }
    fn manifest(&self, d: &Directory, r: &Receipt) -> Result<Manifest, String> {
        let (bytes, _) = d.read("manifest.json", METADATA_BYTES, None)?;
        if bytes.len() != r.manifest_bytes || sha(&bytes) != r.manifest {
            return Err(error("manifest_invalid"));
        }
        let m: Manifest = serde_json::from_slice(&bytes).map_err(|_| error("manifest_invalid"))?;
        if m.validate()? != r.total {
            return Err(error("manifest_invalid"));
        }
        Ok(m)
    }
    pub(super) fn dispatch(
        &self,
        db: &Connection,
        project: i64,
        c: &NativeCallContext,
        name: &str,
        args: Value,
        validate: impl Fn(&Value) -> Result<(), String>,
        import: impl FnOnce(Value) -> Result<Value, String>,
    ) -> Result<Value, String> {
        self.dispatch_at(db, project, c, name, args, now_ms(), validate, import)
    }
    fn dispatch_at(
        &self,
        db: &Connection,
        project: i64,
        c: &NativeCallContext,
        name: &str,
        args: Value,
        now: i64,
        validate: impl Fn(&Value) -> Result<(), String>,
        import: impl FnOnce(Value) -> Result<Value, String>,
    ) -> Result<Value, String> {
        if !db.is_autocommit() {
            return Err(error("transaction_not_current"));
        }
        let scope = self.scope(db, project, c)?;
        match name {
            "begin_exposure_import_upload" => {
                self.begin(db, project, c, &scope, input(args)?, now, validate)
            }
            "put_exposure_import_chunk" => self.put(db, &scope, input(args)?, now),
            "exposure_import_upload_status" => self.status(db, &scope, input(args)?, now),
            "commit_exposure_import_upload" => {
                self.commit(db, project, &scope, input(args)?, now, import)
            }
            "discard_exposure_import_upload" => {
                self.discard(db, &self.cleanup_scope(project, c)?, input(args)?, now)
            }
            _ => Err(error("method_unavailable")),
        }
    }
    fn begin(
        &self,
        db: &Connection,
        project: i64,
        c: &NativeCallContext,
        scope: &str,
        m: Manifest,
        now: i64,
        validate: impl Fn(&Value) -> Result<(), String>,
    ) -> Result<Value, String> {
        let total = m.validate()?;
        let bytes = serde_json::to_vec(&m).map_err(|_| error("manifest_invalid"))?;
        if bytes.len() > METADATA_BYTES {
            return Err(error("metadata_too_large"));
        }
        validate(&m.request(None))?;
        self.expire(db, now)?;
        let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate)
            .map_err(|_| error("storage"))?;
        let (count,project_count,reserved):(i64,i64,i64)=tx.query_row("SELECT COUNT(*),COALESCE(SUM(project_id=?1),0),COALESCE(SUM(total_bytes),0) FROM asset_exposure_uploads WHERE cleanup_pending=1",[project],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|error("storage"))?;
        if count >= 8
            || project_count >= 2
            || reserved.saturating_add(total as i64) > RESERVED_BYTES
        {
            return Err(error("quota"));
        }
        let claimed:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM asset_exposure_upload_claims WHERE instance_id=?1 AND project_id=?2 AND operation_id=?3)",params![self.instance,project,m.request.operation_id],|r|r.get(0)).map_err(|_|error("storage"))?;
        if claimed {
            return Err(error("operation_already_claimed_do_not_replay"));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let directory = self.root.child(&id, true)?;
        let digest = sha(&bytes);
        let expires = now.checked_add(TTL_MS).ok_or_else(|| error("clock"))?;
        tx.execute("INSERT INTO asset_exposure_uploads(upload_id,instance_id,project_id,host_project_id,grant_revision,app_revision,scope_sha256,operation_id,manifest_sha256,manifest_bytes,total_bytes,phase,created_ms,expires_ms,directory_dev,directory_ino,cleanup_scope_sha256) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'receiving',?12,?13,?14,?15,?16)",params![id,self.instance,project,c.host_bound_project_id,c.grant_revision,c.app_revision,scope,m.request.operation_id,digest,bytes.len()as i64,total as i64,now,expires,directory.id.device as i64,directory.id.inode as i64,self.cleanup_scope(project,c)?]).map_err(|_|error("storage"))?;
        for (i, f) in m.files.iter().enumerate() {
            tx.execute(
                "INSERT INTO asset_exposure_upload_files VALUES(?1,?2,?3,?4,?5)",
                params![
                    id,
                    i as i64,
                    f.bytes as i64,
                    f.sha256,
                    f.bytes.div_ceil(CHUNK_BYTES) as i64
                ],
            )
            .map_err(|_| error("storage"))?;
        }
        tx.commit().map_err(|_| error("begin_unconfirmed"))?;
        if directory.write("manifest.json", &bytes).is_err() {
            let _=db.execute("UPDATE asset_exposure_uploads SET phase='discarded' WHERE upload_id=?1 AND phase='receiving'",[&id]);
            let _ = self.cleanup(db, &id);
            return Err(error("begin_storage_failed"));
        }
        self.revalidate(db)?;
        Ok(
            json!({"uploadId":id,"manifestDigest":digest,"chunkSize":CHUNK_BYTES,"expiresAt":expires,"files":m.files.iter().enumerate().map(|(i,f)|json!({"fileIndex":i,"bytes":f.bytes,"chunkCount":f.bytes.div_ceil(CHUNK_BYTES)})).collect::<Vec<_>>()}),
        )
    }
    fn receiving(r: &Receipt, now: i64) -> Result<(), String> {
        if r.phase != "receiving" {
            return Err(error("not_receiving"));
        }
        if now >= r.expires {
            return Err(error("expired"));
        }
        Ok(())
    }
    fn put(&self, db: &Connection, scope: &str, a: ChunkArg, now: i64) -> Result<Value, String> {
        let r = self.receipt(db, &a.upload_id, scope)?;
        Self::receiving(&r, now)?;
        let d = self.directory(&a.upload_id, &r)?;
        let m = self.manifest(&d, &r)?;
        let f = m
            .files
            .get(a.file_index)
            .ok_or_else(|| error("chunk_index"))?;
        let offset = a
            .chunk_index
            .checked_mul(CHUNK_BYTES)
            .ok_or_else(|| error("chunk_index"))?;
        if offset >= f.bytes {
            return Err(error("chunk_index"));
        }
        let expected = CHUNK_BYTES.min(f.bytes - offset);
        if a.content_base64.len() > CHUNK_BYTES.div_ceil(3) * 4 {
            return Err(error("chunk_too_large"));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&a.content_base64)
            .map_err(|_| error("base64_invalid"))?;
        if bytes.len() != expected
            || base64::engine::general_purpose::STANDARD.encode(&bytes) != a.content_base64
        {
            return Err(error("chunk_invalid"));
        }
        let digest = sha(&bytes);
        let name = format!("file-{}-chunk-{}", a.file_index, a.chunk_index);
        let existing:Option<(i64,String,i64,i64)>=db.query_row("SELECT bytes,sha256,device,inode FROM asset_exposure_upload_chunks WHERE upload_id=?1 AND file_index=?2 AND chunk_index=?3",params![a.upload_id,a.file_index as i64,a.chunk_index as i64],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|_|error("storage"))?;
        if let Some((length, old, device, inode)) = existing {
            if length != bytes.len() as i64 || old != digest {
                return Err(error("chunk_conflict"));
            }
            let (actual, _) = d.read(
                &name,
                CHUNK_BYTES,
                Some(Identity {
                    device: device as u64,
                    inode: inode as u64,
                }),
            )?;
            if actual != bytes {
                return Err(error("chunk_corrupt"));
            }
            return Ok(
                json!({"uploadId":a.upload_id,"fileIndex":a.file_index,"chunkIndex":a.chunk_index,"bytes":bytes.len(),"sha256":digest,"alreadyStored":true}),
            );
        }
        let written = match d.write(&name, &bytes) {
            Ok(f) => f,
            Err(_) => {
                let (actual, _) = d.read(&name, CHUNK_BYTES, None)?;
                if actual != bytes {
                    return Err(error("chunk_conflict"));
                }
                d.open(&name, false)?
            }
        };
        written
            .sync_all()
            .and_then(|_| d.file.sync_all())
            .map_err(|_| error("chunk_sync_unconfirmed"))?;
        let id = identity(&written, false)?;
        let (actual, _) = d.read(&name, CHUNK_BYTES, Some(id))?;
        if actual != bytes {
            return Err(error("chunk_corrupt"));
        }
        let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate)
            .map_err(|_| error("storage"))?;
        let current = self.receipt(&tx, &a.upload_id, scope)?;
        Self::receiving(&current, now)?;
        tx.execute(
            "INSERT INTO asset_exposure_upload_chunks VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![
                a.upload_id,
                a.file_index as i64,
                a.chunk_index as i64,
                bytes.len() as i64,
                digest,
                id.device as i64,
                id.inode as i64
            ],
        )
        .map_err(|_| error("chunk_receipt_unconfirmed"))?;
        tx.commit()
            .map_err(|_| error("chunk_receipt_unconfirmed"))?;
        self.revalidate(db)?;
        Ok(
            json!({"uploadId":a.upload_id,"fileIndex":a.file_index,"chunkIndex":a.chunk_index,"bytes":bytes.len(),"sha256":digest,"alreadyStored":false}),
        )
    }
    fn status(
        &self,
        db: &Connection,
        scope: &str,
        a: UploadArg,
        now: i64,
    ) -> Result<Value, String> {
        let r = self.receipt(db, &a.upload_id, scope)?;
        let mut statement=db.prepare("SELECT file_index,bytes,chunk_count FROM asset_exposure_upload_files WHERE upload_id=?1 ORDER BY file_index").map_err(|_|error("storage"))?;
        let files: Vec<(usize, usize, usize)> = statement
            .query_map([&a.upload_id], |r| {
                Ok((
                    r.get::<_, i64>(0)? as usize,
                    r.get::<_, i64>(1)? as usize,
                    r.get::<_, i64>(2)? as usize,
                ))
            })
            .map_err(|_| error("storage"))?
            .collect::<Result<_, _>>()
            .map_err(|_| error("storage"))?;
        let mut output = Vec::new();
        let mut complete = true;
        for (i, bytes, count) in files {
            let mut missing = Vec::new();
            let mut received = 0usize;
            for chunk in 0..count {
                let size:Option<i64>=db.query_row("SELECT bytes FROM asset_exposure_upload_chunks WHERE upload_id=?1 AND file_index=?2 AND chunk_index=?3",params![a.upload_id,i as i64,chunk as i64],|r|r.get(0)).optional().map_err(|_|error("storage"))?;
                match size {
                    Some(size) => received += size as usize,
                    None => missing.push(chunk),
                }
            }
            complete &= missing.is_empty();
            output.push(json!({"fileIndex":i,"bytes":bytes,"chunkCount":count,"receivedChunks":count-missing.len(),"receivedBytes":received,"missingChunks":missing}));
        }
        Ok(
            json!({"uploadId":a.upload_id,"manifestDigest":r.manifest,"phase":r.phase,"expiresAt":r.expires,"expired":now>=r.expires,"totalBytes":r.total,"files":output,"allChunksRecorded":complete,"contentVerified":r.phase=="completed","resultRequiresReview":r.phase=="committing"||r.phase=="uncertain"}),
        )
    }
    fn commit(
        &self,
        db: &Connection,
        project: i64,
        scope: &str,
        a: FinalArg,
        now: i64,
        import: impl FnOnce(Value) -> Result<Value, String>,
    ) -> Result<Value, String> {
        let r = self.receipt(db, &a.upload_id, scope)?;
        if !valid_sha(&a.expected_manifest_digest) || r.manifest != a.expected_manifest_digest {
            return Err(error("manifest_conflict"));
        }
        if matches!(r.phase.as_str(), "completed" | "failed") {
            self.cleanup(db, &a.upload_id)
                .map_err(|_| error("committed_cleanup_failed_do_not_reimport"))?;
            return outcome(&r.outcome.ok_or_else(|| error("outcome_missing"))?);
        }
        Self::receiving(&r, now)?;
        let d = self.directory(&a.upload_id, &r)?;
        let m = self.manifest(&d, &r)?;
        let mut bodies = Vec::new();
        for (i, f) in m.files.iter().enumerate() {
            let mut body = Vec::with_capacity(f.bytes);
            for chunk in 0..f.bytes.div_ceil(CHUNK_BYTES) {
                let(length,digest,device,inode):(i64,String,i64,i64)=db.query_row("SELECT bytes,sha256,device,inode FROM asset_exposure_upload_chunks WHERE upload_id=?1 AND file_index=?2 AND chunk_index=?3",params![a.upload_id,i as i64,chunk as i64],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|_|error("storage"))?.ok_or_else(||error("chunks_missing"))?;
                let (bytes, _) = d.read(
                    &format!("file-{i}-chunk-{chunk}"),
                    CHUNK_BYTES,
                    Some(Identity {
                        device: device as u64,
                        inode: inode as u64,
                    }),
                )?;
                if bytes.len() != length as usize
                    || sha(&bytes) != digest
                    || bytes.len() != CHUNK_BYTES.min(f.bytes - body.len())
                {
                    return Err(error("chunk_corrupt"));
                }
                body.extend(bytes);
            }
            if body.len() != f.bytes || sha(&body) != f.sha256 {
                return Err(error("file_digest_mismatch"));
            }
            bodies.push(body);
        }
        let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate)
            .map_err(|_| error("storage"))?;
        let current = self.receipt(&tx, &a.upload_id, scope)?;
        Self::receiving(&current, now)?;
        let old:Option<(String,String)>=tx.query_row("SELECT manifest_sha256,upload_id FROM asset_exposure_upload_claims WHERE instance_id=?1 AND project_id=?2 AND operation_id=?3",params![self.instance,project,m.request.operation_id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|error("storage"))?;
        if let Some((digest, upload)) = old {
            if digest != r.manifest {
                return Err(error("operation_conflict"));
            }
            if upload != a.upload_id {
                return Err(error("operation_already_claimed_do_not_replay"));
            }
            return Err(error("operation_uncertain_do_not_replay"));
        }
        tx.execute(
            "INSERT INTO asset_exposure_upload_claims VALUES(?1,?2,?3,?4,?5)",
            params![
                self.instance,
                project,
                m.request.operation_id,
                r.manifest,
                a.upload_id
            ],
        )
        .map_err(|_| error("claim_unconfirmed"))?;
        tx.execute("UPDATE asset_exposure_uploads SET phase='committing' WHERE upload_id=?1 AND phase='receiving'",[&a.upload_id]).map_err(|_|error("claim_unconfirmed"))?;
        tx.commit().map_err(|_| error("claim_unconfirmed"))?;
        self.live.fetch_add(1, Ordering::SeqCst);
        let _live = Live(self.live.clone());
        self.revalidate(db)?;
        let result = import(m.request(Some(bodies)));
        self.revalidate(db)?;
        let record = match &result {
            Ok(v) => json!({"ok":true,"value":v}),
            Err(e) => json!({"ok":false,"error":e}),
        };
        let phase = if result.is_ok() {
            "completed"
        } else {
            "failed"
        };
        let changed=db.execute("UPDATE asset_exposure_uploads SET phase=?1,outcome_json=?2 WHERE upload_id=?3 AND phase='committing'",params![phase,record.to_string(),a.upload_id]).map_err(|_|error("finalize_uncertain_do_not_replay"))?;
        if changed != 1 {
            return Err(error("finalize_uncertain_do_not_replay"));
        }
        self.revalidate(db)?;
        if self.cleanup(db, &a.upload_id).is_err() {
            return Err(error("committed_cleanup_failed_do_not_reimport"));
        }
        result
    }
    fn discard(
        &self,
        db: &Connection,
        cleanup_scope: &str,
        a: FinalArg,
        _now: i64,
    ) -> Result<Value, String> {
        let r = self.stored_receipt(db, &a.upload_id)?;
        if r.cleanup_scope != cleanup_scope {
            return Err(error("scope_changed"));
        }
        if !valid_sha(&a.expected_manifest_digest) || r.manifest != a.expected_manifest_digest {
            return Err(error("manifest_conflict"));
        }
        if r.phase != "receiving" || r.outcome.is_some() {
            return Err(error("discard_requires_receiving"));
        }
        let changed=db.execute("UPDATE asset_exposure_uploads SET phase='discarded' WHERE upload_id=?1 AND phase='receiving' AND outcome_json IS NULL AND NOT EXISTS(SELECT 1 FROM asset_exposure_upload_claims c WHERE c.upload_id=?1) AND NOT EXISTS(SELECT 1 FROM asset_exposure_upload_births b WHERE b.upload_id=?1) AND NOT EXISTS(SELECT 1 FROM asset_exposure_upload_invalidations i WHERE i.upload_id=?1)",[&a.upload_id]).map_err(|_|error("storage"))?;
        if changed != 1 {
            return Err(error("discard_requires_receiving_without_claim"));
        }
        self.cleanup(db, &a.upload_id)?;
        Ok(json!({"uploadId":a.upload_id,"discarded":true,"businessEffects":false}))
    }
    fn cleanup(&self, db: &Connection, id: &str) -> Result<(), String> {
        let pending: bool = db
            .query_row(
                "SELECT cleanup_pending FROM asset_exposure_uploads WHERE upload_id=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(|_| error("cleanup_failed"))?;
        if !pending {
            return Ok(());
        }
        let expected: (i64, i64) = db
            .query_row(
                "SELECT directory_dev,directory_ino FROM asset_exposure_uploads WHERE upload_id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| error("cleanup_failed"))?;
        let d = self.root.child(id, false)?;
        if d.id
            != (Identity {
                device: expected.0 as u64,
                inode: expected.1 as u64,
            })
        {
            return Err(error("custody"));
        }
        let mut statement = db
            .prepare(
                "SELECT file_index,chunk_count FROM asset_exposure_upload_files WHERE upload_id=?1",
            )
            .map_err(|_| error("cleanup_failed"))?;
        let rows: Vec<(i64, i64)> = statement
            .query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|_| error("cleanup_failed"))?
            .collect::<Result<_, _>>()
            .map_err(|_| error("cleanup_failed"))?;
        for (i, count) in rows {
            for chunk in 0..count {
                d.remove(&format!("file-{i}-chunk-{chunk}"))?;
            }
        }
        d.remove("manifest.json")?;
        // Retain one empty custody tombstone per upload. These and immutable SQL
        // receipts are lifetime history, not part of the active raw-byte quota.
        db.execute("UPDATE asset_exposure_uploads SET cleanup_pending=0 WHERE upload_id=?1 AND phase IN ('completed','failed','discarded','expired')",[id]).map_err(|_|error("cleanup_failed"))?;
        self.revalidate(db)
    }
    fn expire(&self, db: &Connection, now: i64) -> Result<(), String> {
        let mut statement=db.prepare("SELECT upload_id FROM asset_exposure_uploads WHERE phase='receiving' AND expires_ms<=?1 AND instance_id=?2").map_err(|_|error("storage"))?;
        let ids: Vec<String> = statement
            .query_map(params![now, self.instance], |r| r.get(0))
            .map_err(|_| error("storage"))?
            .collect::<Result<_, _>>()
            .map_err(|_| error("storage"))?;
        drop(statement);
        for id in ids {
            db.execute("UPDATE asset_exposure_uploads SET phase='expired' WHERE upload_id=?1 AND phase='receiving'",[&id]).map_err(|_|error("storage"))?;
            self.cleanup(db, &id)?;
        }
        // Retry only known-final byte cleanup, never a domain import or an
        // uncertain commit. Explicit discard itself remains receiving-only.
        let mut statement=db.prepare("SELECT upload_id FROM asset_exposure_uploads WHERE instance_id=?1 AND cleanup_pending=1 AND phase IN ('completed','failed','discarded','expired')").map_err(|_|error("storage"))?;
        let final_ids: Vec<String> = statement
            .query_map([&self.instance], |r| r.get(0))
            .map_err(|_| error("storage"))?
            .collect::<Result<_, _>>()
            .map_err(|_| error("storage"))?;
        drop(statement);
        for id in final_ids {
            self.cleanup(db, &id)?;
        }
        Ok(())
    }
    pub(super) fn recover_after_exclusive_owner(
        &self,
        db: &Connection,
        _controller: &crate::agent_runtime::execution_owner::NativeInvocationOwner,
        actual_workers: usize,
    ) -> Result<(), String> {
        self.revalidate(db)?;
        if actual_workers != 0 || self.live.load(Ordering::SeqCst) != 0 {
            return Err(error("active_owner_present"));
        }
        db.execute("UPDATE asset_exposure_uploads SET phase='uncertain',recovered_after_owner_exit=1 WHERE phase IN ('committing','uncertain') AND instance_id=?1",[&self.instance]).map_err(|_|error("recovery_unconfirmed"))?;
        Ok(())
    }
    pub(super) fn activity_status(&self, db: &Connection) -> Result<Value, String> {
        self.revalidate(db)?;
        let unknown:i64=db.query_row("SELECT COUNT(*) FROM asset_exposure_uploads WHERE instance_id=?1 AND phase IN ('committing','uncertain')",[&self.instance],|r|r.get(0)).map_err(|_|error("status"))?;
        let live = self.live.load(Ordering::SeqCst);
        Ok(
            json!({"liveWorkers":live,"busy":live>0,"uncertain":false,"operationResultsRequireReview":unknown>0,"unresolvedImports":unknown}),
        )
    }
}
/// A one-status-read capability, not deserializable and never accepted from RPC.
/// Construction requires the held exclusive native controller; consumption must
/// repeat the predicate and refuse any subsequent native asynchronous receipt.
pub(super) struct QuiescedImportProof {
    instance: String,
    run_id: i64,
}
impl QuiescedImportProof {
    pub(super) fn matches(
        &self,
        db: &Connection,
        instance: &str,
        run_id: i64,
    ) -> Result<bool, String> {
        Ok(self.instance == instance
            && self.run_id == run_id
            && recovered_sync_import_on(db, instance, run_id)?)
    }
}
impl UploadStore {
    pub(super) fn quiesced_import_proofs(
        &self,
        db: &Connection,
        _controller: &crate::agent_runtime::execution_owner::NativeInvocationOwner,
        all_actual_workers: usize,
    ) -> Result<Vec<QuiescedImportProof>, String> {
        self.revalidate(db)?;
        if all_actual_workers != 0 || self.live.load(Ordering::SeqCst) != 0 {
            return Ok(Vec::new());
        }
        let mut statement=db.prepare("SELECT b.run_id FROM asset_exposure_upload_births b JOIN asset_exposure_uploads u ON u.upload_id=b.upload_id WHERE u.instance_id=?1 AND u.phase='uncertain' AND u.recovered_after_owner_exit=1").map_err(|_|error("status"))?;
        let ids: Vec<i64> = statement
            .query_map([&self.instance], |r| r.get(0))
            .map_err(|_| error("status"))?
            .collect::<Result<_, _>>()
            .map_err(|_| error("status"))?;
        ids.into_iter()
            .filter_map(
                |run_id| match recovered_sync_import_on(db, &self.instance, run_id) {
                    Ok(true) => Some(Ok(QuiescedImportProof {
                        instance: self.instance.clone(),
                        run_id,
                    })),
                    Ok(false) => None,
                    Err(e) => Some(Err(e)),
                },
            )
            .collect()
    }
}
fn outcome(raw: &str) -> Result<Value, String> {
    let v: Value = serde_json::from_str(raw).map_err(|_| error("outcome_invalid"))?;
    if v["ok"] == true {
        Ok(v["value"].clone())
    } else {
        Err(v["error"]
            .as_str()
            .unwrap_or("asset_exposure_upload_original_error")
            .into())
    }
}
/// A later native asynchronous admission permanently removes the sync-only
/// classification, independently of the replaceable filesystem work receipt.
pub(super) fn invalidate_async_receipt_on(
    db: &Connection,
    instance: &str,
    run_id: &str,
) -> Result<(), String> {
    let exists:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='asset_exposure_upload_births')",[],|r|r.get(0)).map_err(|_|error("status"))?;
    if exists {
        db.execute("INSERT OR IGNORE INTO asset_exposure_upload_invalidations SELECT b.upload_id,'native_async_admission' FROM asset_exposure_upload_births b JOIN asset_exposure_uploads u ON u.upload_id=b.upload_id WHERE b.run_id=?1 AND u.instance_id=?2",params![run_id,instance]).map_err(|_|error("async_receipt_unconfirmed"))?;
    }
    Ok(())
}
/// Only the owner status adapter may use this fresh predicate. It is not an
/// authority to recover, replay, adopt, mutate a run, or ignore a live worker.
pub(super) fn recovered_sync_import_on(
    db: &Connection,
    instance: &str,
    run_id: i64,
) -> Result<bool, String> {
    let exists:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='asset_exposure_upload_births')",[],|r|r.get(0)).map_err(|_|error("status"))?;
    if !exists {
        return Ok(false);
    }
    db.query_row(r#"SELECT EXISTS(SELECT 1 FROM asset_exposure_upload_births b
 JOIN asset_exposure_uploads u ON u.upload_id=b.upload_id JOIN asset_exposure_upload_claims c ON c.upload_id=u.upload_id
 JOIN exposure_import_batches i ON i.id=b.batch_id AND i.run_id=b.run_id
 JOIN exposure_runs r ON r.id=b.run_id JOIN exposure_run_plans p ON p.run_id=b.run_id
 WHERE b.run_id=?1 AND u.instance_id=?2 AND c.instance_id=?2 AND u.phase='uncertain' AND u.recovered_after_owner_exit=1
 AND c.project_id=u.project_id AND c.operation_id=u.operation_id AND c.manifest_sha256=u.manifest_sha256
 AND i.project_id=b.project_id AND i.project_id=u.project_id AND i.operation_id=c.operation_id
 AND i.payload_hash=b.payload_hash AND i.channel=b.channel AND i.created_at=b.created_at AND i.import_version='oviraptor.exposure.import.v1'
 AND b.channel IN ('document_import','public_intel_import') AND r.project_id=b.project_id AND r.created_at=b.created_at
 AND r.status IN ('queued','running') AND r.stage IN ('planned','executing') AND r.current_source=''
 AND p.project_id=b.project_id AND p.attempt_number=1 AND p.scope_version=1 AND p.scope_hash='' AND p.options_hash=''
 AND p.created_at=b.plan_created_at AND p.sources_json=b.sources_json AND p.sources_json=json_array(b.channel)
 AND p.planned_items=b.planned_items AND p.budget_requests=0 AND p.budget_bytes=0 AND p.requests_used=0 AND p.bytes_used=0
 AND NOT EXISTS(SELECT 1 FROM asset_exposure_upload_invalidations x WHERE x.upload_id=u.upload_id)
 AND (SELECT COUNT(*) FROM exposure_work_items w WHERE w.run_id=b.run_id)=b.planned_items
 AND NOT EXISTS(SELECT 1 FROM exposure_work_items w WHERE w.run_id=b.run_id AND (w.project_id<>b.project_id OR w.attempt_number<>1 OR w.kind<>b.channel OR w.source_key<>b.channel OR w.asset_id IS NOT NULL OR w.origin<>'' OR w.attempts>1 OR w.claim_version>1 OR w.owner NOT IN ('','exposure-import-'||u.operation_id)))
 )"#,params![run_id,instance],|r|r.get(0)).map_err(|_|error("status"))
}

#[cfg(test)]
#[path = "exposure_uploads_tests.rs"]
mod tests;
