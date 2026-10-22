//! A completed execution must not become an unowned `running` row when SQLite
//! is temporarily unavailable. This receipt contains only the terminal result,
//! never credentials or authority to replay collection/probing.
use super::*;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

const RECEIPT: &str = "terminal-pending.json";
const MAX_RECEIPT_BYTES: u64 = 64 * 1024;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Receipt {
    version: u32,
    run_id: i64,
    attempt_id: String,
    error: Option<String>,
    cancelled: bool,
}

fn attempt(value: &Value) -> Option<&str> {
    value
        .get("_assetAttemptId")
        .and_then(Value::as_str)
        .filter(|id| Uuid::parse_str(id).is_ok())
}

pub(super) fn begin_attempt(settings: &mut Value) {
    settings["_assetAttemptId"] = json!(Uuid::new_v4().to_string());
}

fn read(path: &Path) -> Result<Receipt, String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_RECEIPT_BYTES
    {
        return Err("任务终态回执无效，未修改任务".into());
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(MAX_RECEIPT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_RECEIPT_BYTES {
        return Err("任务终态回执过大".into());
    }
    let receipt: Receipt =
        serde_json::from_slice(&bytes).map_err(|_| "任务终态回执损坏，未修改任务")?;
    if receipt.version != 1 || receipt.run_id <= 0 || Uuid::parse_str(&receipt.attempt_id).is_err()
    {
        return Err("任务终态回执版本无效，未修改任务".into());
    }
    Ok(receipt)
}

pub(super) fn record(
    run_dir: &Path,
    run_id: i64,
    settings: &Value,
    result: &Result<(), String>,
    cancelled: bool,
) -> Result<(), String> {
    let attempt_id = attempt(settings).ok_or("任务尝试标识缺失，未创建终态回执")?;
    fs::create_dir_all(run_dir).map_err(|e| e.to_string())?;
    let metadata = fs::symlink_metadata(run_dir).map_err(|e| e.to_string())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("任务目录无效".into());
    }
    let receipt = Receipt {
        version: 1,
        run_id,
        attempt_id: attempt_id.into(),
        error: result
            .as_ref()
            .err()
            .map(|e| crate::log_display::text(e, 8000)),
        cancelled,
    };
    let temporary = run_dir.join(format!(".terminal-{}.tmp", Uuid::new_v4()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let saved = (|| {
        let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
        file.write_all(&serde_json::to_vec(&receipt).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        fs::rename(&temporary, run_dir.join(RECEIPT)).map_err(|e| e.to_string())?;
        // Syncing the directory preserves the rename across abrupt shutdown.
        #[cfg(unix)]
        File::open(run_dir)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        Ok(())
    })();
    if saved.is_err() {
        let _ = fs::remove_file(temporary);
    }
    saved
}

pub(super) fn clear(run_dir: &Path, settings: &Value) {
    let path = run_dir.join(RECEIPT);
    if read(&path)
        .ok()
        .is_some_and(|r| Some(r.attempt_id.as_str()) == attempt(settings))
    {
        let _ = fs::remove_file(path);
    }
}

/// Only callers holding their process admission lock, or bootstrap before any
/// job can launch, may call this. Live owners are explicitly excluded.
#[cfg(test)]
pub(super) fn reconcile_on(
    connection: &mut rusqlite::Connection,
    app_data_dir: &Path,
    live: &HashSet<i64>,
) -> Result<usize, String> {
    reconcile_owned_on(connection, app_data_dir, live, None)
}

pub(super) fn reconcile_owned_on(
    connection: &mut rusqlite::Connection,
    app_data_dir: &Path,
    live: &HashSet<i64>,
    owner: Option<&str>,
) -> Result<usize, String> {
    reconcile_with_policy_on(connection, app_data_dir, live, owner, true)
}

pub(super) fn reconcile_bootstrap_on(
    connection: &mut rusqlite::Connection,
    app_data_dir: &Path,
    live: &HashSet<i64>,
    owner: Option<&str>,
) -> Result<usize, String> {
    reconcile_with_policy_on(connection, app_data_dir, live, owner, false)
}

fn reconcile_with_policy_on(
    connection: &mut rusqlite::Connection,
    app_data_dir: &Path,
    live: &HashSet<i64>,
    owner: Option<&str>,
    strict: bool,
) -> Result<usize, String> {
    let columns: HashSet<String> = connection
        .prepare("PRAGMA table_info(runs)")
        .map_err(|e| e.to_string())?
        .query_map([], |r| r.get(1))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    // Older read-only compatibility projections may not have execution fields.
    if !["output_dir", "config_snapshot"]
        .iter()
        .all(|s| columns.contains(*s))
    {
        return Ok(0);
    }
    let mut sql="SELECT id,status,output_dir,config_snapshot FROM runs WHERE pipeline IN ('full','collect','reprobe')".to_string();
    let values = if let Some(owner) = owner {
        sql.push_str(
            " AND id IN (SELECT run_id FROM asset_service_job_owners WHERE instance_id=?)",
        );
        vec![owner]
    } else {
        Vec::new()
    };
    let rows: Vec<(i64, String, String, String)> = connection
        .prepare(&sql)
        .map_err(|e| e.to_string())?
        .query_map(rusqlite::params_from_iter(values), |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    let mut recovered = 0;
    for (id, status, directory, snapshot) in rows {
        if live.contains(&id) {
            continue;
        }
        let settings: Value = serde_json::from_str(&snapshot).unwrap_or(Value::Null);
        let Some(expected_attempt) = attempt(&settings) else {
            continue;
        };
        let directory = PathBuf::from(directory);
        let path = directory.join(RECEIPT);
        let active = matches!(status.as_str(), "queued" | "running" | "cancel_requested");
        let checked = (|| -> Result<Option<Receipt>, String> {
            if !path.try_exists().map_err(|e| e.to_string())? {
                return Ok(None);
            }
            let jobs = app_data_dir
                .join("jobs")
                .canonicalize()
                .map_err(|_| "任务回执目录不存在")?;
            let canonical = directory.canonicalize().map_err(|_| "任务回执目录不存在")?;
            if canonical.parent() != Some(jobs.as_path())
                || fs::symlink_metadata(&directory)
                    .map_err(|e| e.to_string())?
                    .file_type()
                    .is_symlink()
            {
                return Err("任务终态回执不属于当前应用，未修改任务".into());
            }
            read(&path).map(Some)
        })();
        let receipt = match checked {
            Ok(Some(value)) => value,
            Ok(None) => continue,
            Err(error) if !active => {
                let _ = error;
                continue;
            }
            Err(error) if strict => return Err(error),
            Err(error) => {
                // Bootstrap can still open the App and preserve this unknown
                // execution as interrupted. Never trust or delete bad bytes,
                // and do not stop recovery of other healthy receipts.
                let message = format!(
                    "终态回执未通过核验，保留原记录并标为中断供核对，不会自动重放：{error}"
                );
                let _ = crate::asset_logs::append(
                    connection,
                    id,
                    "error",
                    "recovery",
                    &message,
                    |_| {},
                );
                continue;
            }
        };
        // A stale attempt can never supply the outcome of a resumed execution.
        if receipt.run_id != id || receipt.attempt_id != expected_attempt {
            continue;
        }
        if active {
            let result = receipt.error.map_or(Ok(()), Err);
            job_lifecycle::finish_with_fallback(connection, id, result, receipt.cancelled)?;
            recovered += 1;
        }
        // Every existing terminal state wins, including interrupted/restarted.
        clear(&directory, &settings);
    }
    Ok(recovered)
}

#[cfg(test)]
#[path = "terminal_recovery_tests.rs"]
mod tests;
