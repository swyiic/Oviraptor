//! New scans keep their heavy detail in `agent-jobs/<scan_id>/scan.sqlite3`.
//! The main database only records `sentinel_scans.detail_store`. Execution
//! still writes the main file until the publish path is switched; this module
//! only creates the file and the marker.

use rusqlite::{Connection, OpenFlags, OptionalExtension};
use std::{fs, path::{Path, PathBuf}};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS scan_detail_store (
    schema_version INTEGER PRIMARY KEY CHECK (schema_version = 1)
);
";

pub(crate) fn provision_new_scan(
    main_db: &Path,
    app_data_dir: &Path,
    scan_id: &str,
) -> Result<PathBuf, String> {
    validate_scan_id(scan_id)?;
    let main = crate::db::open(main_db)?;
    let exists: bool = main
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1)",
            [scan_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("无法核对扫描索引：{error}"))?;
    if !exists {
        return Err("扫描不存在，未建立独立库".into());
    }
    let path = create_file(app_data_dir, scan_id)?;
    let changed = main
        .execute(
            "UPDATE sentinel_scans SET detail_store='scan_file' WHERE id=?1 AND detail_store='main'",
            [scan_id],
        )
        .map_err(|error| format!("无法标记扫描独立库：{error}"))?;
    if changed == 1 {
        return Ok(path);
    }
    let current: Option<String> = main
        .query_row(
            "SELECT detail_store FROM sentinel_scans WHERE id=?1",
            [scan_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("无法核对扫描存储标记：{error}"))?;
    if current.as_deref() == Some("scan_file") {
        return Ok(path);
    }
    Err("扫描存储标记未能写成独立库".into())
}

pub(crate) fn detail_path(app_data_dir: &Path, scan_id: &str) -> Result<PathBuf, String> {
    validate_scan_id(scan_id)?;
    Ok(app_data_dir
        .join(super::SCAN_WORK_DIRECTORY)
        .join(scan_id)
        .join("scan.sqlite3"))
}

fn validate_scan_id(scan_id: &str) -> Result<(), String> {
    if scan_id.trim().is_empty()
        || scan_id.contains('/')
        || scan_id.contains('\\')
        || scan_id.contains("..")
        || scan_id.chars().any(char::is_control)
    {
        return Err("任务 ID 非法".into());
    }
    Ok(())
}

fn create_file(app_data_dir: &Path, scan_id: &str) -> Result<PathBuf, String> {
    let path = detail_path(app_data_dir, scan_id)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("无法创建扫描目录：{error}"))?;
    }
    if path.exists() {
        verify_file(&path)?;
        return Ok(path);
    }
    let connection = Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .map_err(|error| format!("无法创建扫描库：{error}"))?;
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .map_err(|error| format!("无法创建扫描库：{error}"))?;
    connection
        .execute_batch(SCHEMA)
        .map_err(|error| format!("无法创建扫描库：{error}"))?;
    let inserted = connection
        .execute(
            "INSERT INTO scan_detail_store(schema_version) VALUES (1)",
            [],
        )
        .map_err(|error| format!("无法创建扫描库：{error}"))?;
    if inserted != 1 {
        return Err("无法创建扫描库".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("无法收紧扫描库权限：{error}"))?;
    }
    verify_file(&path)?;
    Ok(path)
}

pub(crate) fn materialize(main_db: &Path, app_data_dir: &Path, scan_id: &str) -> Result<PathBuf, String> {
    validate_scan_id(scan_id)?;
    let store = detail_path(app_data_dir, scan_id)?;
    let marker: Option<String> = crate::db::open(main_db)?
        .query_row(
            "SELECT detail_store FROM sentinel_scans WHERE id=?1",
            [scan_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("无法读取扫描存储标记：{error}"))?;
    if marker.as_deref() != Some("scan_file") {
        return Err("扫描仍使用主库".into());
    }
    if !store.exists() {
        create_file(app_data_dir, scan_id)?;
    }
    ensure_main_path_column(&store)?;
    let live = Connection::open_with_flags(&store, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_FULL_MUTEX)
        .and_then(|connection| connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='sentinel_scans')",
            [],
            |row| row.get::<_, i64>(0),
        ))
        .unwrap_or(0) == 1;
    if !live {
        clone_schema(main_db, &store)?;
        copy_scan(main_db, &store, scan_id)?;
    }
    let dest = crate::db::open(&store)?;
    dest.execute(
        "UPDATE scan_detail_store SET main_path=?1 WHERE schema_version=1",
        [main_db.to_string_lossy().as_ref()],
    ).map_err(|error| format!("无法记录主库路径：{error}"))?;
    Ok(store)
}

pub(crate) fn mark_main(main_db: &Path, scan_id: &str) {
    if let Ok(main) = crate::db::open(main_db) {
        let _ = main.execute(
            "UPDATE sentinel_scans SET detail_store='main' WHERE id=?1 AND detail_store='scan_file'",
            [scan_id],
        );
    }
}

pub(crate) fn status_database(main_db: &Path, app_data_dir: &Path, scan_id: &str) -> PathBuf {
    let marked = crate::db::open(main_db).ok().and_then(|main| {
        main.query_row(
            "SELECT detail_store='scan_file' FROM sentinel_scans WHERE id=?1",
            [scan_id],
            |row| row.get::<_, bool>(0),
        ).ok()
    }).unwrap_or(false);
    let store = detail_path(app_data_dir, scan_id).ok().filter(|path| path.exists());
    let ready = store.as_ref().and_then(|path| Connection::open(path).ok()).and_then(|connection| {
        connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='sentinel_scans')",
            [],
            |row| row.get::<_, i64>(0),
        ).ok()
    }).unwrap_or(0) == 1;
    if marked && ready { store.unwrap() } else { main_db.to_path_buf() }
}

pub(crate) fn mirror_status(main_db: &Path, scan_id: &str) {
    let Some(parent) = main_db.parent() else { return };
    let Ok(path) = detail_path(parent, scan_id) else { return };
    let _ = apply_status(&path, main_db, scan_id);
}

fn apply_status(scan_db: &Path, main_db: &Path, scan_id: &str) -> Result<(), String> {
    if !scan_db.exists() { return Ok(()); }
    let scan = crate::db::open(scan_db)?;
    let main = crate::db::open(main_db)?;
    let row: Option<(String, String)> = main.query_row(
        "SELECT status,current_checkpoint FROM sentinel_scans WHERE id=?1",
        [scan_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional().map_err(|error| error.to_string())?;
    let Some((status, checkpoint)) = row else { return Ok(()) };
    scan.execute(
        "UPDATE sentinel_scans SET status=?1,current_checkpoint=?2,updated_at=datetime('now','localtime') WHERE id=?3",
        rusqlite::params![status, checkpoint, scan_id],
    ).map_err(|error| error.to_string())?;
    Ok(())
}

pub(crate) fn start_publisher(scan_db: &Path, scan_id: &str) {
    let scan_db = scan_db.to_path_buf();
    let scan_id = scan_id.to_string();
    let Ok(main_path) = Connection::open(&scan_db).and_then(|connection| connection.query_row(
        "SELECT main_path FROM scan_detail_store WHERE schema_version=1 AND main_path<>''",
        [],
        |row| row.get::<_, String>(0),
    )) else { return };
    std::thread::Builder::new().name("scan-detail-publish".into()).spawn(move || {
        let main_path = PathBuf::from(main_path);
        let mut previous = String::new();
        for _ in 0..86_400 {
            let (terminal, signature) = publish_once(&main_path, &scan_db, &scan_id).unwrap_or((false, String::new()));
            if signature != previous {
                previous = signature;
                let _ = notify(&main_path, &scan_id);
            }
            if terminal { break; }
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }).ok();
}

fn publish_once(main_db: &Path, scan_db: &Path, scan_id: &str) -> Result<(bool, String), String> {
    let main = crate::db::open(main_db)?;
    main.execute("ATTACH DATABASE ?1 AS detail", [scan_db.to_string_lossy().as_ref()])
        .map_err(|error| error.to_string())?;
    main.execute(
        "UPDATE sentinel_scans SET status=(SELECT status FROM detail.sentinel_scans WHERE id=?1),\
         current_checkpoint=(SELECT current_checkpoint FROM detail.sentinel_scans WHERE id=?1),\
         llm_requests=(SELECT llm_requests FROM detail.sentinel_scans WHERE id=?1),\
         input_tokens=(SELECT input_tokens FROM detail.sentinel_scans WHERE id=?1),\
         output_tokens=(SELECT output_tokens FROM detail.sentinel_scans WHERE id=?1),\
         cached_tokens=(SELECT cached_tokens FROM detail.sentinel_scans WHERE id=?1),\
         total_tokens=(SELECT total_tokens FROM detail.sentinel_scans WHERE id=?1),\
         attempt_count=(SELECT attempt_count FROM detail.sentinel_scans WHERE id=?1),\
         updated_at=(SELECT updated_at FROM detail.sentinel_scans WHERE id=?1)\
         WHERE id=?1 AND EXISTS(SELECT 1 FROM detail.sentinel_scans WHERE id=?1)",
        [scan_id],
    ).map_err(|error| error.to_string())?;
    let _ = main.execute(
        "INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title,severity,record_json,updated_at)\
         SELECT scan_id,target_url,stage,kind,record_key,title,severity,record_json,updated_at FROM detail.sentinel_findings WHERE scan_id=?1\
         ON CONFLICT(scan_id,target_url,stage,kind,record_key) DO UPDATE SET title=excluded.title,severity=excluded.severity,record_json=excluded.record_json,updated_at=excluded.updated_at",
        [scan_id],
    );
    let (status, signature): (String, String) = main.query_row(
        "SELECT status, status || current_checkpoint || total_tokens FROM sentinel_scans WHERE id=?1",
        [scan_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap_or_default();
    Ok((matches!(status.as_str(), "completed" | "completed_with_gaps" | "failed" | "cancelled" | "paused" | "protected_stop" | "interrupted" | "manual_review" | "limited" | "fuse_excluded" | "deferred" | "persistence_failure" | "resume_incompatible"), signature))
}

fn notify(main_db: &Path, scan_id: &str) -> Result<(), String> {
    let main = crate::db::open(main_db)?;
    let attempt: i64 = main.query_row(
        "SELECT attempt_count FROM sentinel_scans WHERE id=?1",
        [scan_id],
        |row| row.get(0),
    ).unwrap_or(0);
    if attempt <= 0 { return Ok(()); }
    main.execute(
        "INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json) VALUES(?1,?2,'scan_detail',?1,'scan_detail','{}')",
        rusqlite::params![scan_id, attempt],
    ).map_err(|error| error.to_string())?;
    Ok(())
}

fn ensure_main_path_column(store: &Path) -> Result<(), String> {
    let connection = crate::db::open(store)?;
    let _ = connection.execute_batch("ALTER TABLE scan_detail_store ADD COLUMN main_path TEXT NOT NULL DEFAULT ''");
    Ok(())
}

fn clone_schema(main_db: &Path, store: &Path) -> Result<(), String> {
    let main = crate::db::open(main_db)?;
    let dest = crate::db::open(store)?;
    dest.pragma_update(None, "foreign_keys", "OFF").map_err(|error| error.to_string())?;
    let mut statement = main.prepare(
        "SELECT sql FROM sqlite_master WHERE sql IS NOT NULL AND type IN ('table','index','trigger','view') AND name NOT LIKE 'sqlite%' ORDER BY CASE type WHEN 'table' THEN 0 WHEN 'view' THEN 1 WHEN 'index' THEN 2 ELSE 3 END, name",
    ).map_err(|error| error.to_string())?;
    let statements = statement.query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    for sql in statements {
        if sql.contains("scan_detail_store") { continue; }
        dest.execute_batch(&sql).map_err(|error| format!("无法复制扫描库结构：{error}"))?;
    }
    dest.pragma_update(None, "foreign_keys", "ON").map_err(|error| error.to_string())?;
    Ok(())
}

fn copy_scan(main_db: &Path, store: &Path, scan_id: &str) -> Result<(), String> {
    let main = crate::db::open(main_db)?;
    main.pragma_update(None, "foreign_keys", "OFF").map_err(|error| error.to_string())?;
    main.execute("ATTACH DATABASE ?1 AS detail", [store.to_string_lossy().as_ref()])
        .map_err(|error| format!("无法挂接扫描库：{error}"))?;
    let mut tables = main.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite%'")
        .map_err(|error| error.to_string())?;
    let names = tables.query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    drop(tables);
    for name in &names {
        if name == "scan_detail_store" || name == "projects" { continue; }
        let mut info = main.prepare(&format!("PRAGMA table_info(\"{name}\")")).map_err(|error| error.to_string())?;
        let columns = info.query_map([], |row| row.get::<_, String>(1))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        if columns.iter().any(|column| column == "scan_id") {
            main.execute(
                &format!("INSERT OR IGNORE INTO detail.\"{name}\" SELECT * FROM \"{name}\" WHERE scan_id=?1"),
                [scan_id],
            ).map_err(|error| format!("无法复制 {name}：{error}"))?;
        }
    }
    main.execute(
        "INSERT OR IGNORE INTO detail.projects SELECT * FROM projects WHERE id=(SELECT project_id FROM sentinel_scans WHERE id=?1)",
        [scan_id],
    ).map_err(|error| format!("无法复制项目：{error}"))?;
    for name in ["app_settings", "config_profiles", "agent_skills", "agent_knowledge_entries"] {
        if names.iter().any(|table| table == name) {
            let _ = main.execute(&format!("INSERT OR IGNORE INTO detail.\"{name}\" SELECT * FROM \"{name}\""), []);
        }
    }
    main.execute("DETACH DATABASE detail", []).ok();
    Ok(())
}

fn verify_file(path: &Path) -> Result<(), String> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .map_err(|error| format!("扫描库无法打开：{error}"))?;
    let version: i64 = connection
        .query_row(
            "SELECT schema_version FROM scan_detail_store",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("扫描库结构不符：{error}"))?;
    if version != 1 {
        return Err("扫描库结构版本不符".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn main_with_scan(dir: &Path, scan_id: &str) -> PathBuf {
        let path = dir.join("oviraptor.sqlite3");
        let connection = crate::db::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE sentinel_scans(id TEXT PRIMARY KEY, detail_store TEXT NOT NULL DEFAULT 'main');",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO sentinel_scans(id) VALUES (?1)",
                [scan_id],
            )
            .unwrap();
        path
    }

    #[test]
    fn new_scan_gets_its_own_file_and_marker() {
        let dir = std::env::temp_dir().join(format!("scan-store-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let main = main_with_scan(&dir, "scan-a");
        let created = provision_new_scan(&main, &dir, "scan-a").unwrap();
        assert_eq!(created, detail_path(&dir, "scan-a").unwrap());
        verify_file(&created).unwrap();
        let again = provision_new_scan(&main, &dir, "scan-a").unwrap();
        assert_eq!(again, created);
        let connection = crate::db::open(&main).unwrap();
        let marker: String = connection
            .query_row(
                "SELECT detail_store FROM sentinel_scans WHERE id='scan-a'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(marker, "scan_file");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn illegal_scan_id_creates_nothing() {
        let error = provision_new_scan(Path::new("unused"), Path::new("unused"), "../scan").unwrap_err();
        assert_eq!(error, "任务 ID 非法");
    }
}
