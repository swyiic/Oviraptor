//! Incremental, offline import of historical probe CSVs. No source file changes.
#![recursion_limit = "256"]
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn first<'a>(row: &'a BTreeMap<String, String>, names: &[&str]) -> &'a str {
    names
        .iter()
        .filter_map(|name| row.get(*name))
        .map(|s| s.trim())
        .find(|s| !s.is_empty())
        .unwrap_or("")
}

fn digest(values: &[&str]) -> String {
    format!("{:x}", Sha256::digest(values.join("\u{1f}").as_bytes()))[..16].to_string()
}

fn canonical_identity(
    link: &str,
    host: &str,
    protocol: &str,
    ip: &str,
    port: &str,
    fallback: &str,
) -> String {
    if !link.trim().is_empty() {
        link.trim().trim_end_matches('/').to_lowercase()
    } else if !host.trim().is_empty() {
        host.trim().trim_end_matches('/').to_lowercase()
    } else if !ip.trim().is_empty() || !port.trim().is_empty() {
        format!("{}|{}|{}", protocol.trim(), ip.trim(), port.trim()).to_lowercase()
    } else {
        fallback.trim().trim_end_matches('/').to_lowercase()
    }
}

fn import_file(
    connection: &Connection,
    project: i64,
    run: i64,
    path: &Path,
) -> Result<(usize, usize)> {
    let mut reader = csv::ReaderBuilder::new().flexible(true).from_path(path)?;
    let headers = reader
        .headers()?
        .iter()
        .map(|v| v.trim_start_matches('\u{feff}').to_string())
        .collect::<Vec<_>>();
    let (mut imported, mut invalid) = (0, 0);
    for record in reader.records() {
        let record = record?;
        let row = headers
            .iter()
            .enumerate()
            .map(|(i, name)| (name.clone(), record.get(i).unwrap_or("").to_string()))
            .collect::<BTreeMap<_, _>>();
        let company = first(&row, &["company"]);
        let host = first(&row, &["host"]);
        let link = first(&row, &["link"]);
        let ip = first(&row, &["ip"]);
        let port = first(&row, &["port"]);
        let protocol = first(&row, &["protocol"]);
        let fallback = format!("{link}|{host}|{ip}|{port}");
        let source_key = first(&row, &["asset_key"]);
        let raw_key = if source_key.is_empty() {
            &fallback
        } else {
            source_key
        };
        if raw_key.trim_matches('|').is_empty() {
            invalid += 1;
            continue;
        }
        let canonical_key = canonical_identity(link, host, protocol, ip, port, raw_key);
        let existing: Option<String> = connection.query_row(
            "SELECT a.asset_key FROM project_assets pa JOIN assets a ON a.id=pa.asset_id WHERE pa.project_id=?1 AND a.canonical_key=?2 AND pa.is_deleted=0 ORDER BY pa.asset_id LIMIT 1",
            params![project, canonical_key], |row| row.get(0)).optional()?;
        let asset_key = existing.unwrap_or_else(|| format!("{company}\u{1f}{raw_key}"));
        let domain = first(&row, &["domain"]);
        let title = first(&row, &["probe_title", "title"]);
        let status = first(&row, &["probe_status_code", "status_code"]);
        let outcome = first(&row, &["probe_outcome"]);
        let entry_state = first(&row, &["probe_entry_state"]);
        let tier = first(&row, &["review_tier"]);
        let category = first(&row, &["content_category"]);
        let score = first(&row, &["score"]);
        let state_hash = digest(&[host, link, ip, port, protocol, domain, tier, score]);
        let probe_hash = digest(&[outcome, entry_state, status, title, category]);
        let extra = serde_json::to_string(
            &row.iter()
                .filter(|(_, v)| !v.is_empty())
                .collect::<BTreeMap<_, _>>(),
        )?;
        connection.execute(
            UPSERT_ASSET,
            params![
                asset_key,
                company,
                host,
                link,
                ip,
                port,
                protocol,
                domain,
                title,
                status,
                outcome,
                entry_state,
                tier,
                category,
                score,
                state_hash,
                probe_hash,
                canonical_key,
                extra,
                outcome
            ],
        )?;
        let asset: i64 = connection.query_row(
            "SELECT id FROM assets WHERE asset_key=?1",
            [&asset_key],
            |row| row.get(0),
        )?;
        if connection.execute("INSERT OR IGNORE INTO project_assets(project_id,asset_id,last_run_id) VALUES(?1,?2,?3)", params![project, asset, run])? > 0 {
            connection.execute("INSERT INTO asset_events(project_id,asset_id,run_id,event_type,summary) VALUES(?1,?2,?3,'new','从历史 P1/P2/P3 探测结果导入')", params![project, asset, run])?;
        } else {
            connection.execute("UPDATE project_assets SET last_seen=datetime('now','localtime'),last_run_id=?1 WHERE project_id=?2 AND asset_id=?3", params![run, project, asset])?;
        }
        imported += 1;
    }
    Ok((imported, invalid))
}

fn input_files(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        let name = path.file_name().and_then(|v| v.to_str()).unwrap_or("");
        if path.is_file()
            && ["P1_", "P2_", "P3_"].iter().any(|p| name.starts_with(p))
            && name.ends_with(".csv")
            && ["alive_clean", "blocked_content", "unreachable"]
                .iter()
                .any(|p| name.contains(p))
        {
            files.push(path);
        }
    }
    files.sort();
    if files.is_empty() {
        return Err(format!("未在 {} 找到 P1/P2/P3 探测 CSV", directory.display()).into());
    }
    Ok(files)
}

fn run_import(
    db: &Path,
    directory: &Path,
    project_name: &str,
    mut output: impl Write,
) -> Result<serde_json::Value> {
    let files = input_files(directory)?;
    // Do not silently create an empty database when a path is misspelled.
    let mut connection = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    connection.busy_timeout(Duration::from_secs(60))?;
    connection.execute_batch("PRAGMA foreign_keys=ON")?;
    let transaction = connection.transaction()?;
    transaction.execute("INSERT OR IGNORE INTO projects(name,description) VALUES(?1,'由既有 P1/P2/P3 存活探测结果增量导入；源 CSV 保持不变')", [project_name])?;
    let project: i64 = transaction.query_row(
        "SELECT id FROM projects WHERE name=?1",
        [project_name],
        |r| r.get(0),
    )?;
    transaction.execute("INSERT INTO runs(project_id,name,pipeline,status,stage,progress,total,config_snapshot,output_dir,started_at) VALUES(?1,'导入既有 P1/P2/P3 探测结果','historical_import','running','import',0,?2,'{}',?3,datetime('now','localtime'))", params![project, files.len() as i64, directory.to_string_lossy()])?;
    let run = transaction.last_insert_rowid();
    transaction.commit()?;
    let result = (|| -> Result<serde_json::Value> {
        let (mut total, mut invalid) = (0usize, 0usize);
        for (index, path) in files.iter().enumerate() {
            let transaction = connection.transaction()?;
            let (count, bad) = import_file(&transaction, project, run, path)?;
            total += count;
            invalid += bad;
            transaction.execute(
                "UPDATE runs SET progress=?1,processed=?2 WHERE id=?3",
                params![
                    (index + 1) as f64 / files.len() as f64 * 100.0,
                    total as i64,
                    run
                ],
            )?;
            let message = format!(
                "{}：导入 {count}，无效 {bad}",
                path.file_name().unwrap_or_default().to_string_lossy()
            );
            transaction.execute(
                "INSERT INTO logs(run_id,level,stage,message) VALUES(?1,'info','import',?2)",
                params![run, message],
            )?;
            transaction.commit()?;
            writeln!(output, "[{}/{}] {message}", index + 1, files.len())?;
            output.flush()?;
        }
        let unique: i64 = connection.query_row(
            "SELECT COUNT(*) FROM project_assets WHERE project_id=?1 AND is_deleted=0",
            [project],
            |r| r.get(0),
        )?;
        let result = serde_json::json!({"project_id":project,"read":total,"invalid":invalid,"unique_assets":unique});
        let transaction = connection.transaction()?;
        transaction.execute("UPDATE runs SET status='completed',stage='completed',progress=100,processed=?1,finished_at=datetime('now','localtime') WHERE id=?2", params![total as i64, run])?;
        transaction.execute(
            "INSERT INTO logs(run_id,level,stage,message) VALUES(?1,'info','completed',?2)",
            params![
                run,
                format!("历史导入完成：读取 {total} 条，无效 {invalid} 条")
            ],
        )?;
        transaction.commit()?;
        Ok(result)
    })();
    if let Err(error) = &result {
        connection.execute("UPDATE runs SET status='failed',stage='failed',error=?1,finished_at=datetime('now','localtime') WHERE id=?2", params![error.to_string(), run])?;
    }
    result
}

fn main() {
    if let Err(error) = main_result() {
        eprintln!("历史导入失败：{error}");
        std::process::exit(1);
    }
}

fn main_result() -> Result<()> {
    let mut database = None;
    let mut input = None;
    let mut project = "中国移动历史资产".to_string();
    let mut args = std::env::args_os().skip(1);
    while let Some(flag) = args.next() {
        match flag.to_str() {
            Some("--help" | "-h") => {
                println!("import-existing-results --input-dir DIR [--db FILE] [--project-name NAME]\n仅增量导入；源 CSV 保持不变。默认数据库：用户目录/oviraptor/oviraptor.sqlite3");
                return Ok(());
            }
            Some("--db") => database = Some(PathBuf::from(args.next().ok_or("--db 缺少路径")?)),
            Some("--input-dir") => {
                input = Some(PathBuf::from(args.next().ok_or("--input-dir 缺少路径")?))
            }
            Some("--project-name") => {
                project = args
                    .next()
                    .ok_or("--project-name 缺少名称")?
                    .into_string()
                    .map_err(|_| "项目名称不是有效 UTF-8")?
            }
            _ => return Err(format!("未知参数：{}", flag.to_string_lossy()).into()),
        }
    }
    let directory = input.ok_or("请指定 --input-dir")?;
    let database = match database {
        Some(path) => path,
        None => PathBuf::from(
            std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .ok_or("无法定位用户目录，请指定 --db")?,
        )
        .join("oviraptor/oviraptor.sqlite3"),
    };
    let result = run_import(&database, &directory, &project, std::io::stdout().lock())?;
    println!("{result}");
    Ok(())
}

const UPSERT_ASSET: &str = r#"
INSERT INTO assets(asset_key,company,host,link,ip,port,protocol,domain,title,status_code,
  probe_outcome,probe_entry_state,review_tier,content_category,score,state_hash,probe_hash,
  canonical_key,extra_json,last_alive)
VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,CASE WHEN ?='alive_clean' THEN datetime('now','localtime') ELSE NULL END)
ON CONFLICT(asset_key) DO UPDATE SET
  company=excluded.company,host=excluded.host,link=excluded.link,ip=excluded.ip,port=excluded.port,
  protocol=excluded.protocol,domain=excluded.domain,title=excluded.title,status_code=excluded.status_code,
  probe_outcome=excluded.probe_outcome,probe_entry_state=excluded.probe_entry_state,
  review_tier=excluded.review_tier,content_category=excluded.content_category,score=excluded.score,
  state_hash=excluded.state_hash,probe_hash=excluded.probe_hash,
  canonical_key=excluded.canonical_key,
  extra_json=json_patch(assets.extra_json,excluded.extra_json),last_seen=datetime('now','localtime'),
  last_alive=CASE WHEN excluded.probe_outcome='alive_clean' THEN datetime('now','localtime') ELSE assets.last_alive END
"#;

// The shared database module is included for `initialize`/`open` only; the rest
// of its writers are used by the application, not by this import CLI.
#[cfg(test)]
#[allow(dead_code)]
#[path = "../src-tauri/src/db.rs"]
mod application_db;

// `application_db::initialize` installs the same durable collaboration-event
// schema as the desktop app. These aliases keep the shared module self-contained
// when it is compiled inside this test-only import harness.
#[cfg(test)]
#[allow(dead_code)]
#[path = "../src-tauri/src/collaboration_events.rs"]
#[allow(unused_imports)] // Desktop control exports are unused by this schema test harness.
mod collaboration_events;
#[cfg(test)]
mod db {
    pub use super::application_db::open;
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir()
                .join(format!("oviraptor-import-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }
        fn db(&self) -> PathBuf {
            application_db::initialize(&self.0).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn imports_bom_csv_idempotently_without_modifying_source_or_reviving_deleted_assets() {
        let fixture = Fixture::new();
        let database = fixture.db();
        let path = fixture.0.join("P1_alive_clean.csv");
        let csv = "\u{feff}company,host,link,ip,port,protocol,probe_title,probe_status_code,probe_outcome\n测试公司,EXAMPLE.TEST,https://example.test/,127.0.0.1,443,https,首页,200,alive_clean\n,,,,,,,,\n";
        std::fs::write(&path, csv).unwrap();
        for _ in 0..2 {
            let result = run_import(&database, &fixture.0, "测试项目", Vec::new()).unwrap();
            assert_eq!(result["read"], 1);
            assert_eq!(result["invalid"], 1);
            assert_eq!(result["unique_assets"], 1);
        }
        let connection = Connection::open(&database).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM asset_events", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            connection
                .query_row("SELECT title FROM assets", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "首页"
        );
        assert!(connection
            .query_row("SELECT last_alive FROM assets", [], |r| r
                .get::<_, Option<String>>(0))
            .unwrap()
            .is_some());
        connection
            .execute(
                "UPDATE project_assets SET is_deleted=1,decision='rejected'",
                [],
            )
            .unwrap();
        let result = run_import(&database, &fixture.0, "测试项目", Vec::new()).unwrap();
        assert_eq!(result["unique_assets"], 0);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), csv);
    }

    #[test]
    fn rolls_back_failed_file_and_records_terminal_failure() {
        let fixture = Fixture::new();
        let database = fixture.db();
        let connection = Connection::open(&database).unwrap();
        connection.execute_batch("CREATE TRIGGER reject_fixture BEFORE INSERT ON assets WHEN NEW.title='broken' BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
        std::fs::write(
            fixture.0.join("P1_unreachable.csv"),
            "company,link,title\nTest,https://one.test,ok\nTest,https://two.test,broken\n",
        )
        .unwrap();
        let error = run_import(&database, &fixture.0, "失败测试", Vec::new()).unwrap_err();
        assert!(error.to_string().contains("fixture failure"));
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM assets", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            connection
                .query_row("SELECT status FROM runs", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "failed"
        );
    }

    #[test]
    fn missing_database_is_not_created_and_file_selection_matches_legacy() {
        let fixture = Fixture::new();
        for name in [
            "P1_alive_clean.csv",
            "P2_blocked_content.csv",
            "P3_unreachable.csv",
            "P4_alive_clean.csv",
            "P1_all.csv",
            "notes.txt",
        ] {
            std::fs::write(fixture.0.join(name), "host\n").unwrap();
        }
        assert_eq!(input_files(&fixture.0).unwrap().len(), 3);
        let missing = fixture.0.join("missing.sqlite3");
        assert!(run_import(&missing, &fixture.0, "test", Vec::new()).is_err());
        assert!(!missing.exists());
        assert_eq!(
            canonical_identity("", "", "HTTPS", "127.0.0.1", "443", "x"),
            "https|127.0.0.1|443"
        );
        assert_eq!(digest(&[]), "e3b0c44298fc1c14");
    }
}
