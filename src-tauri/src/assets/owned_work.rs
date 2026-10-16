//! Durable native Assets admission evidence. Never an authority to adopt old rows.
//! Receipts are written AFTER existing domain transactions: a receipt I/O failure
//! is explicitly unknown, never claimed to be an atomic domain/receipt commit.
use crate::{asset_service::service_instance_on, db};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex, MutexGuard},
};
static ADMISSION: Mutex<()> = Mutex::new(());
static RECEIPT_UNKNOWN: LazyLock<Mutex<std::collections::HashSet<PathBuf>>> =
    LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));
const ACTIVE_LIMIT: usize = 10_000;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Family {
    Ownership,
    Exposure,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    OwnershipAnalysis,
    OwnershipOperation,
    ExposureRun,
}
impl Kind {
    fn label(self) -> &'static str {
        match self {
            Self::OwnershipAnalysis => "ownership_analysis",
            Self::OwnershipOperation => "ownership_operation",
            Self::ExposureRun => "exposure_run",
        }
    }
}
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Receipt {
    schema_version: u32,
    instance_id: String,
    kind: Kind,
    project_id: i64,
    work_id: String,
    birth_sha256: String,
}
#[derive(Debug)]
struct Active {
    kind: Kind,
    id: String,
    known_status: bool,
}
fn failure() -> String {
    "asset_service_owned_work_unavailable".into()
}
pub(crate) fn admission() -> Result<MutexGuard<'static, ()>, String> {
    ADMISSION.lock().map_err(|_| failure())
}
fn directory(db_path: &Path, instance: &str) -> Result<PathBuf, String> {
    Ok(db_path
        .parent()
        .ok_or_else(failure)?
        .join(format!("asset-domain-work-{instance}")))
}
fn safe_directory(path: &Path, create: bool) -> Result<bool, String> {
    if !path.exists() && !create {
        return Ok(false);
    }
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
            Err(_) => return Err(failure()),
        }
    }
    let meta = fs::symlink_metadata(path).map_err(|_| failure())?;
    if meta.file_type().is_symlink()
        || !meta.is_dir()
        || fs::canonicalize(path).map_err(|_| failure())? != path
    {
        return Err(failure());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0 {
            return Err(failure());
        }
    }
    Ok(true)
}
fn receipt_path(directory: &Path, kind: Kind, id: &str) -> PathBuf {
    directory.join(format!(
        "{:x}.json",
        Sha256::digest(format!("{}\0{}", kind.label(), id).as_bytes())
    ))
}
fn read_receipt(path: &Path) -> Result<Option<Receipt>, String> {
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(failure()),
    };
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 8192 {
        return Err(failure());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.nlink() != 1 || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0
        {
            return Err(failure());
        }
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path).map_err(|_| failure())?;
    let same_file = |actual: &fs::Metadata| -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            (
                actual.dev(),
                actual.ino(),
                actual.len(),
                actual.mtime(),
                actual.mtime_nsec(),
                actual.mode(),
                actual.uid(),
                actual.nlink(),
            ) == (
                meta.dev(),
                meta.ino(),
                meta.len(),
                meta.mtime(),
                meta.mtime_nsec(),
                meta.mode(),
                meta.uid(),
                meta.nlink(),
            )
        }
        #[cfg(not(unix))]
        {
            let _ = actual;
            false
        }
    };
    if !same_file(&file.metadata().map_err(|_| failure())?) {
        return Err(failure());
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(8193)
        .read_to_end(&mut bytes)
        .map_err(|_| failure())?;
    if bytes.len() > 8192
        || !same_file(&file.metadata().map_err(|_| failure())?)
        || !same_file(&fs::symlink_metadata(path).map_err(|_| failure())?)
    {
        return Err(failure());
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| failure())
}
fn identity_on(
    connection: &rusqlite::Connection,
    kind: Kind,
    id: &str,
) -> Result<(i64, String), String> {
    let (project,identity): (i64,Value)=match kind {
        Kind::OwnershipAnalysis|Kind::ExposureRun=>{
            let sql=if kind==Kind::OwnershipAnalysis {"SELECT project_id,created_at FROM ownership_analysis_runs WHERE id=?1"}else{"SELECT project_id,created_at FROM exposure_runs WHERE id=?1"};
            connection.query_row(sql,[id],|row|{let project:i64=row.get(0)?;let born:String=row.get(1)?;Ok((project,json!([kind.label(),id,project,born]))) }).optional().map_err(|_|failure())?.ok_or_else(failure)?
        },
        Kind::OwnershipOperation=>connection.query_row("SELECT project_id,created_at,payload_hash,expected_action_hash,kind,action FROM ownership_operations WHERE operation_id=?1",[id],|row|{
            let project:i64=row.get(0)?;let born:String=row.get(1)?;let payload:String=row.get(2)?;let expected:String=row.get(3)?;let kind:String=row.get(4)?;let action:String=row.get(5)?;
            Ok((project,json!([id,project,born,payload,expected,kind,action])))
        }).optional().map_err(|_|failure())?.ok_or_else(failure)?,
    };
    Ok((
        project,
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&identity).map_err(|_| failure())?)
        ),
    ))
}
fn expected_on(
    connection: &rusqlite::Connection,
    instance: &str,
    kind: Kind,
    id: &str,
) -> Result<Receipt, String> {
    if id.is_empty() || id.len() > 512 {
        return Err(failure());
    }
    let (project_id, birth_sha256) = identity_on(connection, kind, id)?;
    Ok(Receipt {
        schema_version: 1,
        instance_id: instance.into(),
        kind,
        project_id,
        work_id: id.into(),
        birth_sha256,
    })
}
fn matches_on(
    db_path: &Path,
    connection: &rusqlite::Connection,
    instance: &str,
    kind: Kind,
    id: &str,
) -> Result<bool, String> {
    let dir = directory(db_path, instance)?;
    if !safe_directory(&dir, false)? {
        return Ok(false);
    }
    let expected = expected_on(connection, instance, kind, id)?;
    Ok(read_receipt(&receipt_path(&dir, kind, id))?.as_ref() == Some(&expected))
}
pub(crate) fn require_existing_owned(db_path: &Path, kind: Kind, id: &str) -> Result<(), String> {
    let connection = db::open(db_path)?;
    let instance = service_instance_on(&connection)?;
    if !matches_on(db_path, &connection, &instance, kind, id)? {
        return Err("asset_service_foreign_work_not_adopted".into());
    }
    Ok(())
}
pub(crate) fn require_owned_if_exists(db_path: &Path, kind: Kind, id: &str) -> Result<(), String> {
    let connection = db::open(db_path)?;
    let sql = match kind {
        Kind::OwnershipAnalysis => {
            "SELECT EXISTS(SELECT 1 FROM ownership_analysis_runs WHERE id=?1)"
        }
        Kind::OwnershipOperation => {
            "SELECT EXISTS(SELECT 1 FROM ownership_operations WHERE operation_id=?1)"
        }
        Kind::ExposureRun => "SELECT EXISTS(SELECT 1 FROM exposure_runs WHERE id=?1)",
    };
    let exists: bool = connection
        .query_row(sql, [id], |r| r.get(0))
        .map_err(|_| failure())?;
    if exists {
        require_existing_owned(db_path, kind, id)?;
    }
    Ok(())
}
pub(crate) fn record(db_path: &Path, kind: Kind, project: i64, id: &str) -> Result<(), String> {
    let result = (|| {
        let connection = db::open(db_path)?;
        let instance = service_instance_on(&connection)?;
        let expected = expected_on(&connection, &instance, kind, id)?;
        if expected.project_id != project {
            return Err(failure());
        }
        let dir = directory(db_path, &instance)?;
        safe_directory(&dir, true)?;
        let path = receipt_path(&dir, kind, id);
        if let Some(old) = read_receipt(&path)? {
            return if old == expected {
                Ok(())
            } else {
                Err(failure())
            };
        }
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options.open(&path).map_err(|_| failure())?;
        file.write_all(&serde_json::to_vec(&expected).map_err(|_| failure())?)
            .map_err(|_| failure())?;
        file.sync_all().map_err(|_| failure())?;
        fs::File::open(&dir)
            .and_then(|dir| dir.sync_all())
            .map_err(|_| failure())?;
        if read_receipt(&path)?.as_ref() != Some(&expected) {
            return Err(failure());
        }
        Ok(())
    })();
    result.map_err(|_| {
        if let Ok(mut unknown) = RECEIPT_UNKNOWN.lock() {
            unknown.insert(db_path.to_path_buf());
        }
        "asset_service_work_receipt_outcome_unknown".into()
    })
}
fn active_on(connection: &rusqlite::Connection, family: Family) -> Result<Vec<Active>, String> {
    let mut items = Vec::new();
    let specs:&[(Kind,&str)]=match family {
        Family::Ownership=>&[(Kind::OwnershipAnalysis,"SELECT CAST(id AS TEXT),status FROM ownership_analysis_runs WHERE status NOT IN ('completed','cancelled','failed') LIMIT 10001"),(Kind::OwnershipOperation,"SELECT operation_id,status FROM ownership_operations WHERE status NOT IN ('completed','failed') LIMIT 10001")],
        Family::Exposure=>&[(Kind::ExposureRun,"SELECT CAST(id AS TEXT),status FROM exposure_runs WHERE status NOT IN ('completed','partial','inconclusive','cancelled','interrupted','failed') LIMIT 10001")],
    };
    for (kind, sql) in specs {
        let mut statement = connection.prepare(sql).map_err(|_| failure())?;
        let rows = statement
            .query_map([], |row| {
                let id: String = row.get(0)?;
                let status: String = row.get(1)?;
                let known_status = match kind {
                    Kind::OwnershipAnalysis => matches!(status.as_str(), "queued" | "running"),
                    Kind::OwnershipOperation => matches!(status.as_str(), "applying" | "blocked"),
                    Kind::ExposureRun => matches!(status.as_str(), "queued" | "running"),
                };
                Ok(Active {
                    kind: *kind,
                    id,
                    known_status,
                })
            })
            .map_err(|_| failure())?;
        for row in rows {
            items.push(row.map_err(|_| failure())?);
            if items.len() > ACTIVE_LIMIT {
                return Ok(items);
            }
        }
    }
    Ok(items)
}
pub(crate) fn guard_recovery(db_path: &Path, family: Family) -> Result<(), String> {
    let connection = db::open(db_path)?;
    let instance = service_instance_on(&connection)?;
    let active = active_on(&connection, family)?;
    if active.len() > ACTIVE_LIMIT {
        return Err(failure());
    }
    for work in active {
        if !work.known_status || !matches_on(db_path, &connection, &instance, work.kind, &work.id)?
        {
            return Err("asset_service_foreign_work_not_adopted".into());
        }
    }
    Ok(())
}
pub(crate) fn status_on(
    db_path: &Path,
    family: Family,
    live_workers: usize,
) -> Result<Value, String> {
    let connection = db::open(db_path)?;
    let instance = service_instance_on(&connection)?;
    let active = active_on(&connection, family)?;
    let mut unowned = 0;
    for work in &active {
        if !work.known_status || !matches_on(db_path, &connection, &instance, work.kind, &work.id)?
        {
            unowned += 1;
        }
    }
    let uncertain = RECEIPT_UNKNOWN
        .lock()
        .map_err(|_| failure())?
        .contains(db_path)
        || unowned > 0
        || active.len() > ACTIVE_LIMIT
        || (!active.is_empty() && live_workers == 0);
    Ok(
        json!({"busy":live_workers>0||!active.is_empty()||uncertain,"uncertain":uncertain,"liveWorkers":live_workers,"activeRows":active.len(),"unownedRows":unowned,"admissionReceiptAtomicWithDomainTransaction":false}),
    )
}

#[cfg(test)]
#[path = "domain_service_test_support.rs"]
pub(super) mod test_support;

#[cfg(test)]
#[path = "owned_work_tests.rs"]
mod tests;
