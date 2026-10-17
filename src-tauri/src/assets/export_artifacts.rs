//! Assets-owned export receipts and bounded artifact reads. Host transports
//! bytes; it receives no authority to select a filesystem path. Desktop export
//! DTOs remain unchanged; only this app adapter returns opaque references.
use crate::{asset_service::service_instance_on, db};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
const PREFIX: &str = "oviraptor-asset-export:";
const MAX_CHUNK: usize = 192 * 1024;
const TTL: u64 = 7 * 24 * 60 * 60;
const MAX_SAFE: u64 = 9_007_199_254_740_991;
fn error(code: &str) -> String {
    format!("asset_artifact_{code}")
}
fn now() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_secs())
        .map_err(|_| error("clock_unavailable"))
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Identity {
    device: u64,
    inode: u64,
    bytes: u64,
    modified_seconds: u64,
    modified_nanos: u32,
}
fn identity(meta: &fs::Metadata, private: bool) -> Result<Identity, String> {
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > MAX_SAFE {
        return Err(error("file_invalid"));
    }
    #[cfg(unix)]
    let (device, inode) = {
        use std::os::unix::fs::MetadataExt;
        if meta.nlink() != 1
            || meta.uid() != unsafe { libc::geteuid() }
            || meta.mode() & if private { 0o077 } else { 0o022 } != 0
        {
            return Err(error("file_permissions"));
        }
        (meta.dev(), meta.ino())
    };
    #[cfg(not(unix))]
    let (device, inode) = {
        let _ = private;
        return Err(error("platform_unavailable"));
    };
    let modified = meta
        .modified()
        .map_err(|_| error("file_invalid"))?
        .duration_since(UNIX_EPOCH)
        .map_err(|_| error("file_invalid"))?;
    Ok(Identity {
        device,
        inode,
        bytes: meta.len(),
        modified_seconds: modified.as_secs(),
        modified_nanos: modified.subsec_nanos(),
    })
}
fn directory(path: &Path, create: bool) -> Result<(), String> {
    if create {
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(error("directory_unavailable")),
        }
    }
    let meta = fs::symlink_metadata(path).map_err(|_| error("directory_unavailable"))?;
    if !path.is_absolute()
        || !meta.is_dir()
        || meta.file_type().is_symlink()
        || fs::canonicalize(path).map_err(|_| error("directory_unavailable"))? != path
    {
        return Err(error("directory_invalid"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0 {
            return Err(error("directory_permissions"));
        }
    }
    Ok(())
}
pub(crate) fn project_directory(export_dir: &Path, project: i64) -> Result<PathBuf, String> {
    if !(1..=MAX_SAFE as i64).contains(&project) {
        return Err(error("project_invalid"));
    }
    directory(export_dir, false)?;
    let project_dir = export_dir.join(format!("project-{project}"));
    directory(&project_dir, true)?;
    Ok(project_dir)
}
fn project_directory_existing(export_dir: &Path, project: i64) -> Result<PathBuf, String> {
    if !(1..=MAX_SAFE as i64).contains(&project) {
        return Err(error("project_invalid"));
    }
    directory(export_dir, false)?;
    let path = export_dir.join(format!("project-{project}"));
    directory(&path, false)?;
    Ok(path)
}
fn id(value: &str) -> Result<String, String> {
    let parsed = uuid::Uuid::parse_str(value).map_err(|_| error("id_invalid"))?;
    if parsed.get_version_num() != 4 || parsed.to_string() != value {
        return Err(error("id_invalid"));
    }
    Ok(value.into())
}
fn reference_id(reference: &str) -> Result<String, String> {
    id(reference
        .strip_prefix(PREFIX)
        .ok_or_else(|| error("reference_required"))?)
}
fn open_file(path: &Path, private: bool) -> Result<(File, Identity), String> {
    let expected = identity(
        &fs::symlink_metadata(path).map_err(|_| error("file_unavailable"))?,
        private,
    )?;
    if fs::canonicalize(path).map_err(|_| error("file_unavailable"))? != path {
        return Err(error("file_invalid"));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path).map_err(|_| error("file_unavailable"))?;
    if identity(
        &file.metadata().map_err(|_| error("file_unavailable"))?,
        private,
    )? != expected
    {
        return Err(error("file_changed"));
    }
    Ok((file, expected))
}
fn digest(file: &mut File) -> Result<String, String> {
    file.seek(SeekFrom::Start(0))
        .map_err(|_| error("read_failed"))?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|_| error("read_failed"))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Receipt {
    schema_version: u32,
    artifact_id: String,
    instance_id: String,
    project_id: i64,
    file_name: String,
    kind: String,
    rows: i64,
    identity: Identity,
    sha256: String,
    created_at: u64,
    expires_at: u64,
}
pub(crate) struct Registered {
    pub(crate) reference: String,
    pub(crate) artifact_id: String,
    pub(crate) sha256: String,
    pub(crate) total_bytes: u64,
    pub(crate) expires_at: u64,
}
pub(crate) struct Resolved {
    pub(crate) directory: PathBuf,
    pub(crate) path: PathBuf,
    pub(crate) sha256: String,
    pub(crate) total_bytes: u64,
    pub(crate) artifact_id: String,
    file: File,
    identity: Identity,
}
fn receipt_path(project_dir: &Path, artifact_id: &str, create: bool) -> Result<PathBuf, String> {
    let dir = project_dir.join(".receipts");
    directory(&dir, create)?;
    Ok(dir.join(format!("{artifact_id}.json")))
}
fn filename(value: &str) -> bool {
    let path = Path::new(value);
    let mut parts = path.components();
    matches!(parts.next(), Some(Component::Normal(_)))
        && parts.next().is_none()
        && !value.is_empty()
        && value.len() <= 1024
        && !value.contains(['\0', '/', '\\'])
}
pub(crate) fn register(
    db_path: &Path,
    export_dir: &Path,
    project: i64,
    path: &Path,
    rows: i64,
    kind: &str,
) -> Result<Registered, String> {
    if rows < 0 || !matches!(kind, "assets" | "exposure") {
        return Err(error("receipt_invalid"));
    }
    let instance = service_instance_on(&db::open(db_path)?)?;
    let project_dir = project_directory(export_dir, project)?;
    if path.parent() != Some(project_dir.as_path()) {
        return Err(error("project_mismatch"));
    }
    let name = path
        .file_name()
        .and_then(|v| v.to_str())
        .filter(|v| filename(v))
        .ok_or_else(|| error("file_invalid"))?;
    let (mut file, expected) = open_file(path, false)?;
    let sha256 = digest(&mut file)?;
    if identity(&file.metadata().map_err(|_| error("file_changed"))?, false)? != expected
        || identity(
            &fs::symlink_metadata(path).map_err(|_| error("file_changed"))?,
            false,
        )? != expected
    {
        return Err(error("file_changed"));
    }
    let artifact_id = uuid::Uuid::new_v4().to_string();
    let created = now()?;
    let receipt = Receipt {
        schema_version: 1,
        artifact_id: artifact_id.clone(),
        instance_id: instance,
        project_id: project,
        file_name: name.into(),
        kind: kind.into(),
        rows,
        identity: expected.clone(),
        sha256: sha256.clone(),
        created_at: created,
        expires_at: created
            .checked_add(TTL)
            .ok_or_else(|| error("clock_unavailable"))?,
    };
    let target = receipt_path(&project_dir, &artifact_id, true)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut output = options
        .open(&target)
        .map_err(|_| error("receipt_write_unknown"))?;
    output
        .write_all(&serde_json::to_vec(&receipt).map_err(|_| error("receipt_invalid"))?)
        .map_err(|_| error("receipt_write_unknown"))?;
    output
        .sync_all()
        .map_err(|_| error("receipt_write_unknown"))?;
    File::open(target.parent().ok_or_else(|| error("receipt_invalid"))?)
        .and_then(|dir| dir.sync_all())
        .map_err(|_| error("receipt_write_unknown"))?;
    Ok(Registered {
        reference: format!("{PREFIX}{artifact_id}"),
        artifact_id,
        sha256,
        total_bytes: expected.bytes,
        expires_at: receipt.expires_at,
    })
}
fn resolve_id(
    db_path: &Path,
    export_dir: &Path,
    project: i64,
    artifact_id: &str,
) -> Result<Resolved, String> {
    let artifact_id = id(artifact_id)?;
    let instance = service_instance_on(&db::open(db_path)?)?;
    let project_dir = project_directory_existing(export_dir, project)?;
    let meta_path = receipt_path(&project_dir, &artifact_id, false)?;
    let (mut meta, meta_identity) = open_file(&meta_path, true)?;
    if meta_identity.bytes > 16384 {
        return Err(error("receipt_invalid"));
    }
    let mut encoded = Vec::new();
    (&mut meta)
        .take(16385)
        .read_to_end(&mut encoded)
        .map_err(|_| error("receipt_invalid"))?;
    if identity(
        &meta.metadata().map_err(|_| error("receipt_invalid"))?,
        true,
    )? != meta_identity
        || identity(
            &fs::symlink_metadata(&meta_path).map_err(|_| error("receipt_invalid"))?,
            true,
        )? != meta_identity
    {
        return Err(error("receipt_invalid"));
    }
    let receipt: Receipt =
        serde_json::from_slice(&encoded).map_err(|_| error("receipt_invalid"))?;
    if receipt.schema_version != 1
        || receipt.artifact_id != artifact_id
        || receipt.instance_id != instance
        || receipt.project_id != project
        || !filename(&receipt.file_name)
        || !matches!(receipt.kind.as_str(), "assets" | "exposure")
        || receipt.sha256.len() != 64
        || !receipt.sha256.bytes().all(|b| b.is_ascii_hexdigit())
        || receipt.expires_at
            != receipt
                .created_at
                .checked_add(TTL)
                .ok_or_else(|| error("receipt_invalid"))?
    {
        return Err(error("receipt_scope_mismatch"));
    }
    let time = now()?;
    if time < receipt.created_at || time >= receipt.expires_at {
        return Err(error("expired"));
    }
    let path = project_dir.join(&receipt.file_name);
    let (mut file, current) = open_file(&path, false)?;
    if current != receipt.identity
        || digest(&mut file)? != receipt.sha256
        || identity(&file.metadata().map_err(|_| error("file_changed"))?, false)? != current
        || identity(
            &fs::symlink_metadata(&path).map_err(|_| error("file_changed"))?,
            false,
        )? != current
    {
        return Err(error("file_changed"));
    }
    Ok(Resolved {
        directory: project_dir,
        path,
        sha256: receipt.sha256,
        total_bytes: current.bytes,
        artifact_id,
        file,
        identity: current,
    })
}
pub(crate) fn resolve(
    db_path: &Path,
    export_dir: &Path,
    project: i64,
    reference: &str,
) -> Result<Resolved, String> {
    resolve_id(db_path, export_dir, project, &reference_id(reference)?)
}
pub(crate) fn wrap_result_on(
    db_path: &Path,
    project: i64,
    export_dir: &Path,
    mut value: Value,
) -> Result<Value, String> {
    let path = value
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| error("result_invalid"))?
        .to_string();
    let rows = value
        .get("rows")
        .and_then(Value::as_i64)
        .ok_or_else(|| error("result_invalid"))?;
    let receipt = register(
        db_path,
        export_dir,
        project,
        Path::new(&path),
        rows,
        "assets",
    )?;
    value["path"] = Value::String(receipt.reference);
    value["artifactId"] = Value::String(receipt.artifact_id);
    value["sha256"] = Value::String(receipt.sha256);
    value["totalBytes"] = json!(receipt.total_bytes);
    value["expiresAt"] = json!(receipt.expires_at);
    Ok(value)
}
pub(crate) fn read(
    db_path: &Path,
    export_dir: &Path,
    project: i64,
    artifact_id: &str,
    offset: u64,
    max_bytes: usize,
) -> Result<Value, String> {
    if max_bytes == 0 || max_bytes > MAX_CHUNK || offset > MAX_SAFE {
        return Err(error("range_invalid"));
    }
    // The parent dispatcher must recheck live project/grant/instance admission
    // on EVERY chunk. This owner independently checks instance and receipt scope.
    let mut resolved = resolve_id(db_path, export_dir, project, artifact_id)?;
    if offset > resolved.total_bytes {
        return Err(error("range_invalid"));
    }
    let take = (resolved.total_bytes - offset).min(max_bytes as u64) as usize;
    let mut bytes = vec![0; take];
    resolved
        .file
        .seek(SeekFrom::Start(offset))
        .map_err(|_| error("read_failed"))?;
    resolved
        .file
        .read_exact(&mut bytes)
        .map_err(|_| error("read_failed"))?;
    if identity(
        &resolved
            .file
            .metadata()
            .map_err(|_| error("file_changed"))?,
        false,
    )? != resolved.identity
        || identity(
            &fs::symlink_metadata(&resolved.path).map_err(|_| error("file_changed"))?,
            false,
        )? != resolved.identity
    {
        return Err(error("file_changed"));
    }
    let end = offset + take as u64;
    let done = end == resolved.total_bytes;
    Ok(
        json!({"artifactId":resolved.artifact_id,"offset":offset,"bytesBase64":STANDARD.encode(bytes),"nextOffset":end,"totalBytes":resolved.total_bytes,"sha256":resolved.sha256,"done":done}),
    )
}

#[cfg(test)]
#[path = "export_artifacts_tests.rs"]
mod tests;
