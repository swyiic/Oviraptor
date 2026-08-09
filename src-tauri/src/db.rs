use rusqlite::{params, Connection, OpenFlags};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

const DEFAULT_BUSINESS_FRONTEND_SKILL: &str =
    include_str!("../resources/skills/business_frontend_deep_analysis.md");

pub fn open(path: &Path) -> Result<Connection, String> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .map_err(|error| error.to_string())?;
    connection
        .busy_timeout(Duration::from_secs(10))
        .map_err(|error| error.to_string())?;
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(|error| error.to_string())?;
    // journal_mode is persistent and is enabled once during initialize().
    // Re-applying it for every UI command asks SQLite for a write lock. While a
    // collector is writing this made otherwise read-only page changes wait for
    // the full busy timeout and looked like the application had frozen.
    connection
        .pragma_update(None, "synchronous", "NORMAL")
        .map_err(|error| error.to_string())?;
    Ok(connection)
}

fn column_exists(connection: &Connection, table: &str, column: &str) -> Result<bool, String> {
    if !table
        .chars()
        .all(|value| value.is_ascii_alphanumeric() || value == '_')
        || !column
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || value == '_')
    {
        return Err("数据库迁移包含非法表名或字段名".into());
    }
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|error| error.to_string())?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(columns.iter().any(|value| value == column))
}

fn ensure_column(
    connection: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<(), String> {
    if column_exists(connection, table, column)? {
        return Ok(());
    }
    connection
        .execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
            [],
        )
        .map_err(|error| format!("数据库迁移失败：{table}.{column}：{error}"))?;
    if column_exists(connection, table, column)? {
        Ok(())
    } else {
        Err(format!("数据库迁移未生效：{table}.{column}"))
    }
}

fn migration_version(connection: &Connection, key: &str) -> i64 {
    connection
        .query_row(
            "SELECT COALESCE(CAST(value AS INTEGER),0) FROM app_settings WHERE key=?1",
            [key],
            |row| row.get(0),
        )
        .unwrap_or(0)
}

fn finish_migration(connection: &Connection, key: &str, version: i64) -> Result<(), String> {
    connection
        .execute(
            "INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, version.to_string()],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn deduplicate_project_assets(connection: &mut Connection) -> Result<i64, String> {
    let groups = {
        let mut statement = connection
            .prepare(
                "SELECT pa.project_id,a.canonical_key FROM project_assets pa JOIN assets a ON a.id=pa.asset_id WHERE a.canonical_key<>'' AND pa.is_deleted=0 GROUP BY pa.project_id,a.canonical_key HAVING COUNT(*)>1",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?
    };
    if groups.is_empty() {
        return Ok(0);
    }

    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    let mut removed = 0i64;
    for (project_id, canonical_key) in groups {
        let rows = {
            let mut statement = transaction
                .prepare(
                    "SELECT pa.asset_id,pa.decision,pa.note,pa.first_seen,pa.last_seen,pa.last_run_id FROM project_assets pa JOIN assets a ON a.id=pa.asset_id WHERE pa.project_id=?1 AND a.canonical_key=?2 AND pa.is_deleted=0 ORDER BY CASE pa.decision WHEN 'confirmed' THEN 4 WHEN 'rejected' THEN 3 WHEN 'uncertain' THEN 2 ELSE 1 END DESC,pa.asset_id",
                )
                .map_err(|error| error.to_string())?;
            let mapped = statement
                .query_map(params![project_id, canonical_key], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<i64>>(5)?,
                    ))
                })
                .map_err(|error| error.to_string())?;
            mapped
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?
        };
        let Some(keeper) = rows.first() else { continue };
        let first_seen = rows
            .iter()
            .map(|row| row.3.as_str())
            .min()
            .unwrap_or(&keeper.3);
        let last_seen = rows
            .iter()
            .map(|row| row.4.as_str())
            .max()
            .unwrap_or(&keeper.4);
        let last_run_id = rows.iter().filter_map(|row| row.5).max();
        transaction
            .execute(
                "UPDATE project_assets SET decision=?1,note=?2,first_seen=?3,last_seen=?4,last_run_id=?5 WHERE project_id=?6 AND asset_id=?7",
                params![keeper.1, keeper.2, first_seen, last_seen, last_run_id, project_id, keeper.0],
            )
            .map_err(|error| error.to_string())?;
        for duplicate in rows.iter().skip(1) {
            let note = if duplicate.2.trim().is_empty() {
                format!("系统自动隔离重复端点；保留资产 #{}", keeper.0)
            } else {
                format!(
                    "{} · 系统自动隔离重复端点；保留资产 #{}",
                    duplicate.2, keeper.0
                )
            };
            removed += transaction
                .execute(
                    "UPDATE project_assets SET decision=CASE WHEN decision IN ('pending','uncertain','') THEN 'rejected' ELSE decision END,note=?1,is_deleted=1,last_run_id=COALESCE(last_run_id,?2) WHERE project_id=?3 AND asset_id=?4 AND is_deleted=0",
                    params![note, last_run_id, project_id, duplicate.0],
                )
                .map_err(|error| error.to_string())? as i64;
        }
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(removed)
}

pub fn initialize(app_data_dir: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(app_data_dir).map_err(|error| error.to_string())?;
    let path = app_data_dir.join("oviraptor.sqlite3");
    let mut connection = open(&path)?;
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    connection
        .execute_batch(SCHEMA)
        .map_err(|error| error.to_string())?;
    let _ = connection.execute("ALTER TABLE worker_nodes ADD COLUMN last_sync_at TEXT", []);
    // 轻量迁移：旧版数据库缺少该列时添加，已存在时忽略 duplicate column。
    let _ = connection.execute(
        "ALTER TABLE assets ADD COLUMN probe_hash TEXT NOT NULL DEFAULT ''",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE assets ADD COLUMN canonical_key TEXT NOT NULL DEFAULT ''",
        [],
    );
    let _ = connection.execute("ALTER TABLE sentinel_targets ADD COLUMN scan_id TEXT", []);
    let _ = connection.execute(
        "ALTER TABLE sentinel_targets ADD COLUMN value_score INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_targets ADD COLUMN scan_mode TEXT NOT NULL DEFAULT ''",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_targets ADD COLUMN routing_reason TEXT NOT NULL DEFAULT ''",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_scans ADD COLUMN previous_scan_id TEXT NOT NULL DEFAULT ''",
        [],
    );
    // Retries created by older releases copied their own targets/checkpoints
    // but still pointed at the first scan for future execution. If that parent
    // was deleted, the dangling comparison link made an otherwise complete
    // child impossible to retry. New retries run in place; detach legacy links
    // whose parent no longer exists.
    let _ = connection.execute(
        "UPDATE sentinel_scans SET previous_scan_id='' WHERE trim(previous_scan_id)<>'' AND NOT EXISTS (SELECT 1 FROM sentinel_scans parent WHERE parent.id=sentinel_scans.previous_scan_id)",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_scans ADD COLUMN llm_requests INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_scans ADD COLUMN input_tokens INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_scans ADD COLUMN output_tokens INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_scans ADD COLUMN cached_tokens INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_scans ADD COLUMN total_tokens INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_scans ADD COLUMN scan_type TEXT NOT NULL DEFAULT 'web'",
        [],
    );
    // Strix 1.5 reports Oviraptor's staged evidence folder as a local target.
    // It is valid input provenance, but never a Web asset/company. Repair
    // historical rows at startup so the false group disappears before the
    // first result-sync poll; source/code targets are intentionally untouched.
    let _ = connection.execute(
        "UPDATE sentinel_findings AS finding SET target_url=COALESCE((SELECT CASE WHEN COUNT(*)=1 THEN MIN(target.url) ELSE '*' END FROM sentinel_targets AS target WHERE target.scan_id=finding.scan_id AND (lower(trim(target.url)) LIKE 'http://%' OR lower(trim(target.url)) LIKE 'https://%')),'*'),updated_at=datetime('now','localtime') WHERE finding.target_url<>'*' AND lower(trim(finding.target_url)) NOT LIKE 'http://%' AND lower(trim(finding.target_url)) NOT LIKE 'https://%' AND (finding.target_url LIKE '%/strix-jobs/%' OR finding.target_url LIKE '%strix-evidence-input%') AND EXISTS (SELECT 1 FROM sentinel_scans AS scan WHERE scan.id=finding.scan_id AND scan.scan_type='web')",
        [],
    );
    let _ = connection.execute(
        "DELETE FROM sentinel_targets WHERE lower(trim(url)) NOT LIKE 'http://%' AND lower(trim(url)) NOT LIKE 'https://%' AND EXISTS (SELECT 1 FROM sentinel_scans AS scan WHERE scan.id=sentinel_targets.scan_id AND scan.scan_type='web')",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_scans ADD COLUMN task_name TEXT NOT NULL DEFAULT ''",
        [],
    );
    // These two fields were introduced on macOS first. Verify the migration so a
    // locked or interrupted Windows upgrade cannot silently keep an older schema.
    ensure_column(
        &connection,
        "sentinel_scans",
        "source_path",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_column(
        &connection,
        "sentinel_scans",
        "skill_names",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_column(
        &connection,
        "sentinel_scans",
        "attempt_count",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    let _ = connection.execute(
        "UPDATE sentinel_scans SET attempt_count=1 WHERE attempt_count=0 AND status<>'draft'",
        [],
    );
    for sql in [
        "ALTER TABLE security_rule_packs ADD COLUMN progress INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE security_rule_packs ADD COLUMN progress_stage TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE security_rule_packs ADD COLUMN progress_message TEXT NOT NULL DEFAULT ''",
    ] {
        let _ = connection.execute(sql, []);
    }
    connection
        .execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS sentinel_processes (
                scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
                process_id INTEGER NOT NULL DEFAULT 0,
                engine TEXT NOT NULL DEFAULT '',
                work_dir TEXT NOT NULL DEFAULT '',
                started_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                PRIMARY KEY(scan_id,process_id)
            );
            CREATE TABLE IF NOT EXISTS sentinel_scan_attempts (
                scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
                attempt_number INTEGER NOT NULL,
                status TEXT NOT NULL DEFAULT 'scanning',
                stage TEXT NOT NULL DEFAULT 'initializing',
                checkpoint TEXT NOT NULL DEFAULT '',
                stop_reason TEXT NOT NULL DEFAULT '',
                work_dir TEXT NOT NULL DEFAULT '',
                llm_requests_start INTEGER NOT NULL DEFAULT 0,
                input_tokens_start INTEGER NOT NULL DEFAULT 0,
                output_tokens_start INTEGER NOT NULL DEFAULT 0,
                cached_tokens_start INTEGER NOT NULL DEFAULT 0,
                total_tokens_start INTEGER NOT NULL DEFAULT 0,
                llm_requests_delta INTEGER NOT NULL DEFAULT 0,
                input_tokens_delta INTEGER NOT NULL DEFAULT 0,
                output_tokens_delta INTEGER NOT NULL DEFAULT 0,
                cached_tokens_delta INTEGER NOT NULL DEFAULT 0,
                total_tokens_delta INTEGER NOT NULL DEFAULT 0,
                started_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                finished_at TEXT NOT NULL DEFAULT '',
                updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                PRIMARY KEY(scan_id,attempt_number)
            );
            CREATE INDEX IF NOT EXISTS idx_sentinel_attempt_scan ON sentinel_scan_attempts(scan_id,attempt_number DESC);
            CREATE TABLE IF NOT EXISTS strix_skills (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                description TEXT NOT NULL DEFAULT '',
                instructions TEXT NOT NULL,
                builtin INTEGER NOT NULL DEFAULT 0,
                enabled INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
            );
            CREATE TABLE IF NOT EXISTS security_rule_packs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                key TEXT NOT NULL UNIQUE,
                name TEXT NOT NULL,
                engine TEXT NOT NULL,
                repository TEXT NOT NULL,
                reference TEXT NOT NULL DEFAULT 'main',
                local_path TEXT NOT NULL DEFAULT '',
                previous_version TEXT NOT NULL DEFAULT '',
                version TEXT NOT NULL DEFAULT '',
                enabled INTEGER NOT NULL DEFAULT 1,
                builtin INTEGER NOT NULL DEFAULT 0,
                status TEXT NOT NULL DEFAULT 'not_installed',
                last_sync_at TEXT NOT NULL DEFAULT '',
                error TEXT NOT NULL DEFAULT '',
                added_count INTEGER NOT NULL DEFAULT 0,
                modified_count INTEGER NOT NULL DEFAULT 0,
                deleted_count INTEGER NOT NULL DEFAULT 0,
                change_summary TEXT NOT NULL DEFAULT '[]',
                progress INTEGER NOT NULL DEFAULT 0,
                progress_stage TEXT NOT NULL DEFAULT '',
                progress_message TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
            );
            CREATE INDEX IF NOT EXISTS idx_security_rule_packs_enabled ON security_rule_packs(enabled,engine);
            CREATE TABLE IF NOT EXISTS strix_knowledge_entries (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                scan_id TEXT NOT NULL UNIQUE,
                project_id INTEGER REFERENCES projects(id) ON DELETE SET NULL,
                title TEXT NOT NULL,
                summary TEXT NOT NULL,
                patterns_json TEXT NOT NULL DEFAULT '{}',
                skill_instructions TEXT NOT NULL DEFAULT '',
                source_hash TEXT NOT NULL DEFAULT '',
                skill_id INTEGER REFERENCES strix_skills(id) ON DELETE SET NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
            );
            CREATE INDEX IF NOT EXISTS idx_strix_knowledge_project ON strix_knowledge_entries(project_id,updated_at);
            CREATE TABLE IF NOT EXISTS strix_learning_candidates (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
                project_id INTEGER REFERENCES projects(id) ON DELETE SET NULL,
                scan_type TEXT NOT NULL DEFAULT 'web',
                title TEXT NOT NULL,
                summary TEXT NOT NULL DEFAULT '',
                candidate_json TEXT NOT NULL DEFAULT '{}',
                status TEXT NOT NULL DEFAULT 'pending',
                target_skill_id INTEGER REFERENCES strix_skills(id) ON DELETE SET NULL,
                source_hash TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                reviewed_at TEXT NOT NULL DEFAULT '',
                updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                UNIQUE(scan_id,source_hash)
            );
            CREATE INDEX IF NOT EXISTS idx_strix_learning_candidates_status ON strix_learning_candidates(status,updated_at);
            CREATE INDEX IF NOT EXISTS idx_strix_learning_candidates_project ON strix_learning_candidates(project_id,updated_at);
            CREATE TABLE IF NOT EXISTS sentinel_scan_contexts (
                scan_id TEXT PRIMARY KEY REFERENCES sentinel_scans(id) ON DELETE CASCADE,
                environment TEXT NOT NULL DEFAULT '',
                auth_profile_name TEXT NOT NULL DEFAULT '',
                auth_type TEXT NOT NULL DEFAULT 'none',
                authenticated INTEGER NOT NULL DEFAULT 0,
                ci_provider TEXT NOT NULL DEFAULT '',
                repository_url TEXT NOT NULL DEFAULT '',
                branch TEXT NOT NULL DEFAULT '',
                commit_sha TEXT NOT NULL DEFAULT '',
                build_id TEXT NOT NULL DEFAULT '',
                policy_json TEXT NOT NULL DEFAULT '{}',
                gate_status TEXT NOT NULL DEFAULT 'not_evaluated',
                gate_reason TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
            );
            CREATE TABLE IF NOT EXISTS browser_auth_sessions (
                id TEXT PRIMARY KEY,
                project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                name TEXT NOT NULL DEFAULT '',
                entry_url TEXT NOT NULL,
                final_url TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL DEFAULT 'capturing',
                scope_hosts_json TEXT NOT NULL DEFAULT '[]',
                cookie_count INTEGER NOT NULL DEFAULT 0,
                header_count INTEGER NOT NULL DEFAULT 0,
                storage_count INTEGER NOT NULL DEFAULT 0,
                captured_request_count INTEGER NOT NULL DEFAULT 0,
                session_json TEXT NOT NULL DEFAULT '{}',
                last_validated_at TEXT NOT NULL DEFAULT '',
                expires_at TEXT NOT NULL DEFAULT '',
                last_error TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
            );
            CREATE INDEX IF NOT EXISTS idx_browser_auth_sessions_project ON browser_auth_sessions(project_id,status,updated_at DESC);
            CREATE TABLE IF NOT EXISTS appsec_vulnerabilities (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                fingerprint TEXT NOT NULL,
                title TEXT NOT NULL,
                vulnerability_type TEXT NOT NULL DEFAULT '',
                severity TEXT NOT NULL DEFAULT 'info',
                status TEXT NOT NULL DEFAULT 'open',
                confidence TEXT NOT NULL DEFAULT '',
                asset TEXT NOT NULL DEFAULT '',
                environment TEXT NOT NULL DEFAULT '',
                url TEXT NOT NULL DEFAULT '',
                http_method TEXT NOT NULL DEFAULT '',
                parameter TEXT NOT NULL DEFAULT '',
                file TEXT NOT NULL DEFAULT '',
                symbol TEXT NOT NULL DEFAULT '',
                start_line INTEGER NOT NULL DEFAULT 0,
                correlation_score INTEGER NOT NULL DEFAULT 0,
                correlation_json TEXT NOT NULL DEFAULT '{}',
                first_seen TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                last_seen TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                owner TEXT NOT NULL DEFAULT '',
                UNIQUE(project_id,fingerprint)
            );
            CREATE TABLE IF NOT EXISTS appsec_vulnerability_sources (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                vulnerability_id INTEGER NOT NULL REFERENCES appsec_vulnerabilities(id) ON DELETE CASCADE,
                scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
                finding_id INTEGER REFERENCES sentinel_findings(id) ON DELETE CASCADE,
                source_type TEXT NOT NULL,
                source_key TEXT NOT NULL,
                engine TEXT NOT NULL DEFAULT '',
                evidence_json TEXT NOT NULL DEFAULT '{}',
                created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                UNIQUE(vulnerability_id,scan_id,source_type,source_key)
            );
            CREATE INDEX IF NOT EXISTS idx_appsec_vuln_project ON appsec_vulnerabilities(project_id,last_seen);
            CREATE INDEX IF NOT EXISTS idx_appsec_source_scan ON appsec_vulnerability_sources(scan_id,vulnerability_id);
            CREATE TABLE IF NOT EXISTS sentinel_fuse_zone (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                asset_id INTEGER REFERENCES assets(id) ON DELETE SET NULL,
                company TEXT NOT NULL DEFAULT '',
                url TEXT NOT NULL,
                normalized_url TEXT NOT NULL,
                source_scan_id TEXT NOT NULL DEFAULT '',
                reason TEXT NOT NULL DEFAULT '',
                verdict TEXT NOT NULL DEFAULT 'pending',
                note TEXT NOT NULL DEFAULT '',
                evidence TEXT NOT NULL DEFAULT '',
                archived INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                UNIQUE(project_id,normalized_url)
            );
            CREATE INDEX IF NOT EXISTS idx_sentinel_fuse_project ON sentinel_fuse_zone(project_id,archived,updated_at);
            "#,
        )
        .map_err(|error| error.to_string())?;
    let _ = connection.execute(
        "INSERT OR IGNORE INTO sentinel_scan_attempts(scan_id,attempt_number,status,stage,checkpoint,stop_reason,llm_requests_delta,input_tokens_delta,output_tokens_delta,cached_tokens_delta,total_tokens_delta,started_at,finished_at,updated_at) SELECT id,MAX(attempt_count,1),status,CASE WHEN status IN ('completed','partial') THEN 'complete' WHEN status IN ('failed','cancelled') THEN 'stopped' WHEN status IN ('paused','pausing') THEN 'paused' ELSE 'unknown' END,current_checkpoint,CASE WHEN status IN ('completed','partial','failed','cancelled','paused') THEN current_checkpoint ELSE '' END,llm_requests,input_tokens,output_tokens,cached_tokens,total_tokens,created_at,CASE WHEN status IN ('completed','partial','failed','cancelled','paused') THEN updated_at ELSE '' END,updated_at FROM sentinel_scans WHERE attempt_count>0",
        [],
    );
    let old_process_schema: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='sentinel_processes'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_default();
    if old_process_schema.contains("scan_id TEXT PRIMARY KEY") {
        connection
            .execute_batch(
                r#"
                ALTER TABLE sentinel_processes RENAME TO sentinel_processes_legacy;
                CREATE TABLE sentinel_processes (
                    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
                    process_id INTEGER NOT NULL DEFAULT 0,
                    engine TEXT NOT NULL DEFAULT '',
                    work_dir TEXT NOT NULL DEFAULT '',
                    started_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                    PRIMARY KEY(scan_id,process_id)
                );
                INSERT OR IGNORE INTO sentinel_processes(scan_id,process_id,engine,work_dir,started_at)
                  SELECT scan_id,process_id,engine,work_dir,started_at FROM sentinel_processes_legacy;
                DROP TABLE sentinel_processes_legacy;
                "#,
            )
            .map_err(|error| format!("升级扫描进程表失败：{error}"))?;
    }
    for statement in [
        "ALTER TABLE security_rule_packs ADD COLUMN added_count INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE security_rule_packs ADD COLUMN modified_count INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE security_rule_packs ADD COLUMN deleted_count INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE security_rule_packs ADD COLUMN change_summary TEXT NOT NULL DEFAULT '[]'",
        "ALTER TABLE security_rule_packs ADD COLUMN previous_version TEXT NOT NULL DEFAULT ''",
    ] {
        let _ = connection.execute(statement, []);
    }
    connection.execute_batch(
        r#"
        INSERT OR IGNORE INTO security_rule_packs(key,name,engine,repository,reference,enabled,builtin)
        VALUES
          ('semgrep-rules','Semgrep Rules','semgrep','https://github.com/semgrep/semgrep-rules.git','develop',1,1),
          ('codeql-queries','CodeQL queries','codeql','https://github.com/github/codeql.git','main',1,1),
          ('owasp-benchmark','OWASP Benchmark','benchmark','https://github.com/OWASP/Benchmark.git','master',1,1);
        "#,
    ).map_err(|error| error.to_string())?;
    connection.execute(
        "INSERT OR IGNORE INTO strix_skills(name,description,instructions,builtin,enabled) VALUES('业务前端深度分析','按看功能、触发请求、还原参数、分析业务 JS、匹配本地知识和一次性保底发现的顺序执行；只把证据充分的高价值候选交给 Strix。',?1,1,1)",
        [DEFAULT_BUSINESS_FRONTEND_SKILL],
    ).map_err(|error| error.to_string())?;
    // Existing limited targets become first-class fuse entries on upgrade.
    connection.execute(
        "INSERT OR IGNORE INTO sentinel_fuse_zone(project_id,asset_id,company,url,normalized_url,source_scan_id,reason) SELECT project_id,asset_id,company,url,lower(rtrim(trim(url),'/')),COALESCE(scan_id,''),routing_reason FROM sentinel_targets WHERE status='limited' AND trim(url)<>''",
        [],
    ).map_err(|error| error.to_string())?;
    // Refresh the shipped built-in prompt in older databases without touching
    // user-authored skills.
    let _ = connection.execute(
        "UPDATE strix_skills SET description='按看功能、触发请求、还原参数、分析业务 JS、匹配本地知识和一次性保底发现的顺序执行；只把证据充分的高价值候选交给 Strix。',instructions=?1,updated_at=datetime('now','localtime') WHERE name='业务前端深度分析' AND builtin=1 AND instructions<>?1",
        [DEFAULT_BUSINESS_FRONTEND_SKILL],
    );
    let old_target_schema: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='sentinel_targets'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_default();
    if old_target_schema.contains("UNIQUE(project_id,url)") {
        let _ = connection.execute_batch(r#"
            ALTER TABLE sentinel_targets RENAME TO sentinel_targets_legacy;
            CREATE TABLE sentinel_targets (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                scan_id TEXT REFERENCES sentinel_scans(id) ON DELETE CASCADE,
                asset_id INTEGER REFERENCES assets(id) ON DELETE SET NULL,
                company TEXT NOT NULL DEFAULT '',
                url TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'queued',
                created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                UNIQUE(project_id,scan_id,url)
            );
            INSERT INTO sentinel_targets(id,project_id,scan_id,asset_id,company,url,status,created_at,updated_at)
              SELECT id,project_id,scan_id,NULL,company,url,status,created_at,updated_at FROM sentinel_targets_legacy;
            DROP TABLE sentinel_targets_legacy;
        "#);
    }
    // 旧版表重建后再次补齐自适应路由字段；新版数据库上的重复列错误可忽略。
    let _ = connection.execute(
        "ALTER TABLE sentinel_targets ADD COLUMN value_score INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_targets ADD COLUMN scan_mode TEXT NOT NULL DEFAULT ''",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_targets ADD COLUMN routing_reason TEXT NOT NULL DEFAULT ''",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_targets ADD COLUMN asset_id INTEGER REFERENCES assets(id) ON DELETE SET NULL",
        [],
    );
    if migration_version(&connection, "canonical_key_backfill_version") < 1 {
        connection
            .execute(
                "UPDATE assets SET canonical_key=lower(rtrim(CASE WHEN trim(link)<>'' THEN trim(link) WHEN trim(host)<>'' THEN trim(host) WHEN trim(ip)<>'' OR trim(port)<>'' THEN trim(protocol)||'|'||trim(ip)||'|'||trim(port) ELSE asset_key END,'/')) WHERE canonical_key=''",
                [],
            )
            .map_err(|error| error.to_string())?;
        finish_migration(&connection, "canonical_key_backfill_version", 1)?;
    }
    connection
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_assets_canonical ON assets(canonical_key)",
            [],
        )
        .map_err(|error| error.to_string())?;
    connection.execute(
        "CREATE INDEX IF NOT EXISTS idx_sentinel_targets_asset ON sentinel_targets(project_id,asset_id)",
        [],
    ).map_err(|error| error.to_string())?;
    // Historical target-to-asset repair is a migration, not launch-time
    // maintenance. The old correlated query scanned the full assets table for
    // every unmatched URL and repeated forever for legitimate URL-only scans.
    // canonical_key makes the one-time lookup indexed; unresolved targets stay
    // URL-only and are not retried on every application start.
    if migration_version(&connection, "sentinel_asset_backfill_version") < 1 {
        connection.execute(
            "UPDATE sentinel_targets AS st SET asset_id=(SELECT a.id FROM assets a JOIN project_assets pa ON pa.asset_id=a.id WHERE pa.project_id=st.project_id AND a.canonical_key=lower(rtrim(trim(st.url),'/')) ORDER BY a.id LIMIT 1) WHERE st.asset_id IS NULL",
            [],
        ).map_err(|error| error.to_string())?;
        finish_migration(&connection, "sentinel_asset_backfill_version", 1)?;
    }
    let old_validation_schema: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='sentinel_validations'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_default();
    if old_validation_schema.contains("UNIQUE(scan_id,url)") {
        connection.execute_batch(r#"
            ALTER TABLE sentinel_validations RENAME TO sentinel_validations_legacy;
            CREATE TABLE sentinel_validations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
                url TEXT NOT NULL,
                finding_key TEXT NOT NULL DEFAULT 'url-summary',
                finding_kind TEXT NOT NULL DEFAULT '',
                verdict TEXT NOT NULL DEFAULT 'pending',
                severity TEXT NOT NULL DEFAULT '',
                note TEXT NOT NULL DEFAULT '',
                evidence TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                UNIQUE(scan_id,url,finding_key)
            );
            INSERT INTO sentinel_validations(id,scan_id,url,finding_key,finding_kind,verdict,severity,note,evidence,created_at,updated_at)
              SELECT id,scan_id,url,'url-summary','',verdict,severity,note,evidence,created_at,updated_at FROM sentinel_validations_legacy;
            DROP TABLE sentinel_validations_legacy;
            CREATE INDEX IF NOT EXISTS idx_sentinel_validation_scan ON sentinel_validations(scan_id,updated_at);
        "#).map_err(|error| error.to_string())?;
    }
    // 配置 JSON 使用向前兼容的字段级迁移，不覆盖用户已有值。
    let _ = connection.execute(
        "UPDATE config_profiles SET settings_json=json_set(settings_json,'$.replaceDefaultContentRules',json('false')) WHERE json_valid(settings_json) AND json_type(settings_json,'$.replaceDefaultContentRules') IS NULL",
        [],
    );
    connection
        .execute(
            "INSERT OR IGNORE INTO app_settings(key,value) VALUES('reminder_days','7')",
            [],
        )
        .map_err(|error| error.to_string())?;
    connection
        .execute(
            "INSERT OR IGNORE INTO app_settings(key,value) VALUES('custom_icon','false')",
            [],
        )
        .map_err(|error| error.to_string())?;
    // This is a one-time repair over the complete asset table. Running the
    // GROUP BY on every launch becomes noticeable once the DB reaches hundreds
    // of MB; normal imports already enforce canonical-key deduplication.
    let dedupe_migration: i64 = connection
        .query_row(
            "SELECT COALESCE((SELECT CAST(value AS INTEGER) FROM app_settings WHERE key='canonical_dedupe_migration'),0)",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if dedupe_migration < 1 {
        let deduplicated = deduplicate_project_assets(&mut connection)?;
        connection
            .execute(
                "INSERT INTO app_settings(key,value) VALUES('canonical_dedupe_migration','1') ON CONFLICT(key) DO UPDATE SET value='1'",
                [],
            )
            .map_err(|error| error.to_string())?;
        connection
            .execute(
                "INSERT INTO app_settings(key,value) VALUES('last_deduplicated',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [deduplicated.to_string()],
            )
            .map_err(|error| error.to_string())?;
    } else {
        connection
            .execute(
                "INSERT OR IGNORE INTO app_settings(key,value) VALUES('last_deduplicated','0')",
                [],
            )
            .map_err(|error| error.to_string())?;
    }
    // 0.5.4 将旧版 alive_clean 中混入的 TCP 与异常 HTTP 结果拆开。
    // 只使用已保存的探测证据迁移分类；之后的“复测现有资产”会刷新实时状态。
    let probe_classification_version: i64 = connection.query_row(
        "SELECT COALESCE((SELECT CAST(value AS INTEGER) FROM app_settings WHERE key='probe_classification_version'),0)",
        [], |row| row.get(0),
    ).unwrap_or(0);
    if probe_classification_version < 3 {
        connection.execute_batch(r#"
        UPDATE assets SET probe_outcome=CASE
          WHEN probe_entry_state='tcp_alive_non_http' THEN 'tcp_alive_non_http'
          WHEN CAST(status_code AS INTEGER) IN (401,403,407,429) THEN 'web_restricted'
          WHEN probe_entry_state IN ('reachable_but_path_missing','reachable_client_error','reachable_server_error','reachable_other','empty_response') THEN 'web_abnormal'
          WHEN CAST(status_code AS INTEGER) BETWEEN 200 AND 399 THEN 'web_alive'
          ELSE probe_outcome END
        WHERE probe_outcome='alive_clean';
        UPDATE project_assets SET decision='not_applicable',note='系统自动Web分类：当前不进入浏览器人工队列'
        WHERE decision IN ('pending','uncertain','') AND asset_id IN (
          SELECT id FROM assets WHERE probe_outcome IN ('tcp_alive_non_http','web_abnormal','unreachable','skipped')
        );
        UPDATE project_assets SET decision='rejected',note='系统自动Web分类：违规内容隔离'
        WHERE decision IN ('pending','uncertain','') AND asset_id IN (
          SELECT id FROM assets WHERE probe_outcome='blocked_content'
        );
        INSERT INTO app_settings(key,value) VALUES('probe_classification_version','3')
          ON CONFLICT(key) DO UPDATE SET value='3';
        "#).map_err(|error| error.to_string())?;
    }
    let _ = connection.execute(
        "UPDATE config_profiles SET settings_json=json_set(settings_json,'$.fofaEmail','') WHERE json_valid(settings_json) AND json_type(settings_json,'$.fofaEmail') IS NULL",
        [],
    );
    let _ = connection.execute(
        "UPDATE config_profiles SET settings_json=json_set(settings_json,'$.fofaKey','') WHERE json_valid(settings_json) AND json_type(settings_json,'$.fofaKey') IS NULL",
        [],
    );
    for (key, value) in [
        ("hackerOneUsername", ""),
        ("hackerOneToken", ""),
        ("proxyUrl", ""),
        ("noProxy", "127.0.0.1,localhost"),
        ("strixExecutable", ""),
        ("strixRunsDirectory", "~/strix_runs"),
    ] {
        let sql = format!(
            "UPDATE config_profiles SET settings_json=json_set(settings_json,'$.{key}',?1) WHERE json_valid(settings_json) AND json_type(settings_json,'$.{key}') IS NULL"
        );
        let _ = connection.execute(&sql, [value]);
    }
    // Promote the legacy single Strix model fields into a switchable list.
    // Legacy fields remain synchronized for older application versions.
    let _ = connection.execute_batch(
        r#"
        UPDATE config_profiles
        SET settings_json=json_set(
            settings_json,
            '$.strixLlmProfiles',
            json(CASE
                WHEN trim(COALESCE(json_extract(settings_json,'$.strixLlm'),''))<>''
                  OR trim(COALESCE(json_extract(settings_json,'$.strixApiBase'),''))<>''
                  OR trim(COALESCE(json_extract(settings_json,'$.strixApiKey'),''))<>''
                THEN json_array(json_object(
                    'id','legacy-default',
                    'name','默认模型',
                    'llm',COALESCE(json_extract(settings_json,'$.strixLlm'),''),
                    'apiBase',COALESCE(json_extract(settings_json,'$.strixApiBase'),''),
                    'apiKey',COALESCE(json_extract(settings_json,'$.strixApiKey'),'')
                ))
                ELSE '[]'
            END),
            '$.strixActiveLlmProfileId',
            CASE
                WHEN trim(COALESCE(json_extract(settings_json,'$.strixLlm'),''))<>''
                  OR trim(COALESCE(json_extract(settings_json,'$.strixApiBase'),''))<>''
                  OR trim(COALESCE(json_extract(settings_json,'$.strixApiKey'),''))<>''
                THEN 'legacy-default'
                ELSE ''
            END
        )
        WHERE json_valid(settings_json)
          AND json_type(settings_json,'$.strixLlmProfiles') IS NULL;

        UPDATE config_profiles
        SET settings_json=json_set(
            settings_json,
            '$.strixActiveLlmProfileId',
            COALESCE(json_extract(settings_json,'$.strixLlmProfiles[0].id'),'')
        )
        WHERE json_valid(settings_json)
          AND json_type(settings_json,'$.strixLlmProfiles')='array'
          AND json_type(settings_json,'$.strixActiveLlmProfileId') IS NULL;
        "#,
    );

    // Older per-URL pipelines marked a task as completed even when every
    // target was routing-only and Strix was never launched. Preserve the
    // target evidence while giving the task an honest top-level state.
    let _ = connection.execute_batch(
        r#"
        UPDATE sentinel_scans
        SET status='recon_only',
            current_checkpoint=CASE
              WHEN trim(current_checkpoint)='' THEN '仅前端解析完成：任务未调用 Strix'
              ELSE current_checkpoint
            END,
            updated_at=datetime('now','localtime')
        WHERE scan_type='web'
          AND status='completed'
          AND EXISTS(
            SELECT 1 FROM sentinel_targets t WHERE t.scan_id=sentinel_scans.id
          )
          AND NOT EXISTS(
            SELECT 1 FROM sentinel_targets t
            WHERE t.scan_id=sentinel_scans.id AND t.status NOT IN ('recon_only','manual_review')
          );
        "#,
    );

    // MLX owns local-model context and generation limits. Remove the retired
    // per-profile overrides so old profiles cannot silently reintroduce them.
    let _ = connection.execute_batch(
        r#"
        UPDATE config_profiles
        SET settings_json=json_set(
            settings_json,
            '$.strixLlmProfiles',
            json(COALESCE((
                SELECT json_group_array(json_remove(value,'$.contextWindow','$.maxOutputTokens'))
                FROM json_each(settings_json,'$.strixLlmProfiles')
            ),'[]'))
        )
        WHERE json_valid(settings_json)
          AND json_type(settings_json,'$.strixLlmProfiles')='array';
        "#,
    );

    // Version 5 adds an explicit frontend packet budget so low-context local
    // models do not receive the old multi-file 40KB+ evidence packet.
    let _ = connection.execute_batch(
        r#"
        UPDATE config_profiles SET settings_json=json_set(
            settings_json,
            '$.strixFrontendPacketMode',COALESCE(json_extract(settings_json,'$.strixFrontendPacketMode'),'balanced'),
            '$.strixFrontendPacketBudgetKb',CASE
              WHEN json_type(settings_json,'$.strixFrontendPacketBudgetKb') IN ('integer','real')
                THEN MIN(MAX(CAST(json_extract(settings_json,'$.strixFrontendPacketBudgetKb') AS INTEGER),4),64)
              ELSE 12
            END
        ) WHERE json_valid(settings_json);
        "#,
    );
    // Version 3 repairs the exact policy signature written by the old
    // "local full power" UI watcher. That watcher permanently replaced the
    // user's governed-mode values with max/zero settings even after the
    // switch was turned off.
    let _ = connection.execute_batch(
        r#"
        UPDATE config_profiles SET settings_json=json_set(
            settings_json,
            '$.strixBatchSize',15,
            '$.strixQuickScore',30,
            '$.strixStandardScore',55,
            '$.strixDeepScore',80,
            '$.strixQuickTimeout',120,
            '$.strixStandardTimeout',300,
            '$.strixDeepTimeout',600,
            '$.strixQuickTokenLimit',50000,
            '$.strixStandardTokenLimit',120000,
            '$.strixDeepTokenLimit',250000,
            '$.strixQuickRequestLimit',4,
            '$.strixStandardRequestLimit',8,
            '$.strixDeepRequestLimit',12,
            '$.strixNoToolTurnLimit',2,
            '$.strixBudgetPolicyVersion',3
        )
        WHERE json_valid(settings_json)
          AND COALESCE(json_extract(settings_json,'$.strixBudgetPolicyVersion'),0)<3
          AND json_extract(settings_json,'$.strixQuickScore')=1
          AND json_extract(settings_json,'$.strixStandardScore')=2
          AND json_extract(settings_json,'$.strixDeepScore')=3
          AND json_extract(settings_json,'$.strixQuickTimeout')=3600
          AND json_extract(settings_json,'$.strixStandardTimeout')=7200
          AND json_extract(settings_json,'$.strixDeepTimeout')=14400
          AND json_extract(settings_json,'$.strixQuickTokenLimit')=0
          AND json_extract(settings_json,'$.strixStandardTokenLimit')=0
          AND json_extract(settings_json,'$.strixDeepTokenLimit')=0
          AND json_extract(settings_json,'$.strixQuickRequestLimit')=100
          AND json_extract(settings_json,'$.strixStandardRequestLimit')=200
          AND json_extract(settings_json,'$.strixDeepRequestLimit')=300
          AND json_extract(settings_json,'$.strixNoToolTurnLimit')=100;
        UPDATE config_profiles
          SET settings_json=json_set(settings_json,'$.strixBudgetPolicyVersion',3)
          WHERE json_valid(settings_json)
            AND COALESCE(json_extract(settings_json,'$.strixBudgetPolicyVersion'),0)<3;
        "#,
    );
    // Adaptive Strix limits are migrated field-by-field so an explicit 0
    // remains a user-selected disabled uncached-token budget.
    for (key, value) in [
        ("strixQuickTimeout", "120"),
        ("strixStandardTimeout", "300"),
        ("strixDeepTimeout", "600"),
        ("strixQuickTokenLimit", "50000"),
        ("strixStandardTokenLimit", "120000"),
        ("strixDeepTokenLimit", "250000"),
        ("strixQuickRequestLimit", "4"),
        ("strixStandardRequestLimit", "8"),
        ("strixDeepRequestLimit", "12"),
        ("strixNoToolTurnLimit", "4"),
    ] {
        let sql = format!(
            "UPDATE config_profiles SET settings_json=json_set(settings_json,'$.{key}',json(?1)) WHERE json_valid(settings_json) AND json_type(settings_json,'$.{key}') IS NULL"
        );
        let _ = connection.execute(&sql, [value]);
    }
    // Version 2 replaces the legacy high-token defaults. Explicit numeric
    // uncached-token limits, including 0 (disabled layer), remain user-controlled.
    let _ = connection.execute_batch(
        r#"
        UPDATE config_profiles SET settings_json=json_set(settings_json,'$.strixQuickTokenLimit',50000)
          WHERE json_valid(settings_json) AND COALESCE(json_extract(settings_json,'$.strixBudgetPolicyVersion'),0)<2
            AND (json_type(settings_json,'$.strixQuickTokenLimit') NOT IN ('integer','real') OR json_extract(settings_json,'$.strixQuickTokenLimit')=100000);
        UPDATE config_profiles SET settings_json=json_set(settings_json,'$.strixStandardTokenLimit',120000)
          WHERE json_valid(settings_json) AND COALESCE(json_extract(settings_json,'$.strixBudgetPolicyVersion'),0)<2
            AND (json_type(settings_json,'$.strixStandardTokenLimit') NOT IN ('integer','real') OR json_extract(settings_json,'$.strixStandardTokenLimit')=250000);
        UPDATE config_profiles SET settings_json=json_set(settings_json,'$.strixDeepTokenLimit',250000)
          WHERE json_valid(settings_json) AND COALESCE(json_extract(settings_json,'$.strixBudgetPolicyVersion'),0)<2
            AND (json_type(settings_json,'$.strixDeepTokenLimit') NOT IN ('integer','real') OR json_extract(settings_json,'$.strixDeepTokenLimit')=500000);
        UPDATE config_profiles SET settings_json=json_set(
            settings_json,
            '$.strixQuickTimeout',MIN(COALESCE(CAST(json_extract(settings_json,'$.strixQuickTimeout') AS INTEGER),120),120),
            '$.strixStandardTimeout',MIN(COALESCE(CAST(json_extract(settings_json,'$.strixStandardTimeout') AS INTEGER),300),300),
            '$.strixDeepTimeout',MIN(COALESCE(CAST(json_extract(settings_json,'$.strixDeepTimeout') AS INTEGER),600),600),
            '$.strixQuickRequestLimit',MIN(COALESCE(CAST(json_extract(settings_json,'$.strixQuickRequestLimit') AS INTEGER),4),4),
            '$.strixStandardRequestLimit',MIN(COALESCE(CAST(json_extract(settings_json,'$.strixStandardRequestLimit') AS INTEGER),8),8),
            '$.strixDeepRequestLimit',MIN(COALESCE(CAST(json_extract(settings_json,'$.strixDeepRequestLimit') AS INTEGER),12),12),
            '$.strixNoToolTurnLimit',MIN(COALESCE(CAST(json_extract(settings_json,'$.strixNoToolTurnLimit') AS INTEGER),2),2),
            '$.strixBudgetPolicyVersion',2
        ) WHERE json_valid(settings_json) AND COALESCE(json_extract(settings_json,'$.strixBudgetPolicyVersion'),0)<2;
        "#,
    );
    // Version 4 raises the cloud no-progress default from two to four model
    // turns. Local full-power scans bypass this fuse at runtime.
    let _ = connection.execute_batch(
        r#"
        UPDATE config_profiles SET settings_json=json_set(
            settings_json,
            '$.strixNoToolTurnLimit',CASE
              WHEN json_extract(settings_json,'$.strixNoToolTurnLimit')=2 THEN 4
              ELSE json_extract(settings_json,'$.strixNoToolTurnLimit')
            END,
            '$.strixBudgetPolicyVersion',4
        )
        WHERE json_valid(settings_json)
          AND COALESCE(json_extract(settings_json,'$.strixBudgetPolicyVersion'),0)<4;
        "#,
    );

    let count: i64 = connection
        .query_row("SELECT COUNT(*) FROM config_profiles", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    if count == 0 {
        let defaults = json!({
            "pythonExecutable": "python3",
            "strixExecutable": "",
            "strixRunsDirectory": "~/strix_runs",
            "strixLlm": "",
            "strixApiBase": "",
            "strixApiKey": "",
            "strixLlmProfiles": [],
            "strixActiveLlmProfileId": "",
            "strixFrontendPacketMode": "balanced",
            "strixFrontendPacketBudgetKb": 12,
            "strixBatchSize": 15,
            "strixQuickTimeout": 120,
            "strixStandardTimeout": 300,
            "strixDeepTimeout": 600,
            "strixQuickTokenLimit": 50000,
            "strixStandardTokenLimit": 120000,
            "strixDeepTokenLimit": 250000,
            "strixQuickRequestLimit": 4,
            "strixStandardRequestLimit": 8,
            "strixDeepRequestLimit": 12,
            "strixNoToolTurnLimit": 4,
            "strixBudgetPolicyVersion": 4,
            "strixProxyEnabled": false,
            "authorizedProxyPool": [],
            "fofaEmail": "",
            "fofaKey": "",
            "hackerOneUsername": "",
            "hackerOneToken": "",
            "proxyUrl": "",
            "noProxy": "127.0.0.1,localhost",
            "scriptsDirectory": "",
            "configPath": "",
            "collectionMode": "all",
            "fofaProfile": "professional",
            "pageSize": 500,
            "maxPages": 0,
            "interval": 6.0,
            "collectionTimeout": 45,
            "fullHistory": false,
            "enableCidr24": false,
            "includeWeakFingerprints": false,
            "runRefine": true,
            "runProbe": true,
            "includeOther": true,
            "includeWeak": false,
            "priorityRate": 20.0,
            "otherRate": 10.0,
            "workers": 64,
            "probeTimeout": 6,
            "probeRetries": 0,
            "contentThreshold": 12,
            "gamblingKeywords": ["在线赌博", "博彩平台", "真人视讯", "体育投注"],
            "pornKeywords": ["色情网站", "成人网站", "成人视频", "情色直播"],
            "negativeKeywords": ["打击赌博", "扫黄打非", "反诈", "公安", "法院"],
            "replaceDefaultContentRules": false
        });
        connection.execute(
            "INSERT INTO config_profiles(name, description, is_default, settings_json) VALUES(?1, ?2, 1, ?3)",
            ("默认配置", "安全、完整度与速度平衡的默认配置", defaults.to_string()),
        ).map_err(|error| error.to_string())?;
    }
    Ok(path)
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS projects (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'active',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

CREATE TABLE IF NOT EXISTS config_profiles (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL DEFAULT '',
    is_default INTEGER NOT NULL DEFAULT 0,
    settings_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

CREATE TABLE IF NOT EXISTS app_settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS worker_nodes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    endpoint TEXT NOT NULL UNIQUE,
    access_token TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    last_seen_at TEXT,
    last_sync_at TEXT,
    last_error TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

CREATE TABLE IF NOT EXISTS content_rules (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    keyword TEXT NOT NULL,
    normalized_keyword TEXT NOT NULL UNIQUE,
    category TEXT NOT NULL DEFAULT 'custom_rule',
    source_asset_id INTEGER REFERENCES assets(id) ON DELETE SET NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE INDEX IF NOT EXISTS idx_content_rules_enabled ON content_rules(enabled,normalized_keyword);

CREATE TABLE IF NOT EXISTS hackerone_programs (
    id TEXT PRIMARY KEY,
    handle TEXT UNIQUE NOT NULL,
    name TEXT NOT NULL,
    icon_url TEXT NOT NULL DEFAULT '',
    policy TEXT NOT NULL DEFAULT '',
    policy_hash TEXT NOT NULL DEFAULT '',
    submission_state TEXT NOT NULL DEFAULT '',
    program_state TEXT NOT NULL DEFAULT '',
    offers_bounties INTEGER NOT NULL DEFAULT 0,
    open_scope INTEGER NOT NULL DEFAULT 0,
    fast_payments INTEGER NOT NULL DEFAULT 0,
    safe_harbor INTEGER NOT NULL DEFAULT 0,
    collaboration INTEGER NOT NULL DEFAULT 0,
    started_accepting_at TEXT,
    last_synced_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    custom_industry TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS hackerone_scopes (
    id TEXT PRIMARY KEY,
    program_handle TEXT NOT NULL,
    asset_type TEXT NOT NULL DEFAULT '',
    asset_identifier TEXT NOT NULL DEFAULT '',
    eligible_for_submission INTEGER NOT NULL DEFAULT 0,
    eligible_for_bounty INTEGER NOT NULL DEFAULT 0,
    max_severity TEXT NOT NULL DEFAULT '',
    instruction TEXT NOT NULL DEFAULT '',
    reference TEXT NOT NULL DEFAULT '',
    created_at TEXT,
    updated_at TEXT,
    active INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS hackerone_exclusions (
    id TEXT PRIMARY KEY,
    program_handle TEXT NOT NULL,
    category TEXT NOT NULL DEFAULT '',
    details TEXT NOT NULL DEFAULT '',
    updated_at TEXT,
    active INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS hackerone_notes (
    program_handle TEXT PRIMARY KEY,
    bookmarked INTEGER NOT NULL DEFAULT 0,
    tags TEXT NOT NULL DEFAULT '',
    notes TEXT NOT NULL DEFAULT '',
    last_tested_at TEXT
);

CREATE TABLE IF NOT EXISTS hackerone_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    program_handle TEXT NOT NULL,
    event_type TEXT NOT NULL,
    summary TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

CREATE INDEX IF NOT EXISTS idx_h1_program_handle ON hackerone_programs(handle);
CREATE INDEX IF NOT EXISTS idx_h1_scope_program ON hackerone_scopes(program_handle,active);
CREATE INDEX IF NOT EXISTS idx_h1_event_program ON hackerone_events(program_handle,created_at);

CREATE TABLE IF NOT EXISTS sentinel_targets (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    scan_id TEXT REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    asset_id INTEGER REFERENCES assets(id) ON DELETE SET NULL,
    company TEXT NOT NULL DEFAULT '',
    url TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'queued',
    value_score INTEGER NOT NULL DEFAULT 0,
    scan_mode TEXT NOT NULL DEFAULT '',
    routing_reason TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(project_id,scan_id,url)
);
CREATE TABLE IF NOT EXISTS sentinel_fuse_zone (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    asset_id INTEGER REFERENCES assets(id) ON DELETE SET NULL,
    company TEXT NOT NULL DEFAULT '',
    url TEXT NOT NULL,
    normalized_url TEXT NOT NULL,
    source_scan_id TEXT NOT NULL DEFAULT '',
    reason TEXT NOT NULL DEFAULT '',
    verdict TEXT NOT NULL DEFAULT 'pending',
    note TEXT NOT NULL DEFAULT '',
    evidence TEXT NOT NULL DEFAULT '',
    archived INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(project_id,normalized_url)
);
CREATE INDEX IF NOT EXISTS idx_sentinel_fuse_project ON sentinel_fuse_zone(project_id,archived,updated_at);
CREATE TABLE IF NOT EXISTS sentinel_scans (
    id TEXT PRIMARY KEY,
    project_id INTEGER REFERENCES projects(id) ON DELETE SET NULL,
    project_name TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'queued',
    current_checkpoint TEXT NOT NULL DEFAULT '',
    task_path TEXT NOT NULL DEFAULT '',
    previous_scan_id TEXT NOT NULL DEFAULT '',
    llm_requests INTEGER NOT NULL DEFAULT 0,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cached_tokens INTEGER NOT NULL DEFAULT 0,
    total_tokens INTEGER NOT NULL DEFAULT 0,
    scan_type TEXT NOT NULL DEFAULT 'web',
    task_name TEXT NOT NULL DEFAULT '',
    source_path TEXT NOT NULL DEFAULT '',
    skill_names TEXT NOT NULL DEFAULT '',
    attempt_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE TABLE IF NOT EXISTS sentinel_scan_attempts (
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    attempt_number INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'scanning',
    stage TEXT NOT NULL DEFAULT 'initializing',
    checkpoint TEXT NOT NULL DEFAULT '',
    stop_reason TEXT NOT NULL DEFAULT '',
    work_dir TEXT NOT NULL DEFAULT '',
    llm_requests_start INTEGER NOT NULL DEFAULT 0,
    input_tokens_start INTEGER NOT NULL DEFAULT 0,
    output_tokens_start INTEGER NOT NULL DEFAULT 0,
    cached_tokens_start INTEGER NOT NULL DEFAULT 0,
    total_tokens_start INTEGER NOT NULL DEFAULT 0,
    llm_requests_delta INTEGER NOT NULL DEFAULT 0,
    input_tokens_delta INTEGER NOT NULL DEFAULT 0,
    output_tokens_delta INTEGER NOT NULL DEFAULT 0,
    cached_tokens_delta INTEGER NOT NULL DEFAULT 0,
    total_tokens_delta INTEGER NOT NULL DEFAULT 0,
    started_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    finished_at TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    PRIMARY KEY(scan_id,attempt_number)
);
CREATE TABLE IF NOT EXISTS strix_skills (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL DEFAULT '',
    instructions TEXT NOT NULL,
    builtin INTEGER NOT NULL DEFAULT 0,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE TABLE IF NOT EXISTS strix_learning_candidates (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    project_id INTEGER REFERENCES projects(id) ON DELETE SET NULL,
    scan_type TEXT NOT NULL DEFAULT 'web',
    title TEXT NOT NULL,
    summary TEXT NOT NULL DEFAULT '',
    candidate_json TEXT NOT NULL DEFAULT '{}',
    status TEXT NOT NULL DEFAULT 'pending',
    target_skill_id INTEGER REFERENCES strix_skills(id) ON DELETE SET NULL,
    source_hash TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    reviewed_at TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(scan_id,source_hash)
);
CREATE TABLE IF NOT EXISTS sentinel_deleted_scans (
    scan_id TEXT PRIMARY KEY,
    deleted_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE TABLE IF NOT EXISTS sentinel_processes (
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    process_id INTEGER NOT NULL DEFAULT 0,
    engine TEXT NOT NULL DEFAULT '',
    work_dir TEXT NOT NULL DEFAULT '',
    started_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    PRIMARY KEY(scan_id,process_id)
);
CREATE TABLE IF NOT EXISTS sentinel_checkpoints (
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    url TEXT NOT NULL,
    stage TEXT NOT NULL,
    raw_json TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    PRIMARY KEY(scan_id,url,stage)
);
CREATE TABLE IF NOT EXISTS sentinel_findings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL DEFAULT '',
    stage TEXT NOT NULL,
    kind TEXT NOT NULL,
    record_key TEXT NOT NULL DEFAULT '',
    title TEXT NOT NULL DEFAULT '',
    severity TEXT NOT NULL DEFAULT '',
    record_json TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(scan_id,target_url,stage,kind,record_key)
);
CREATE INDEX IF NOT EXISTS idx_sentinel_findings_scan ON sentinel_findings(scan_id,stage,kind);
CREATE TABLE IF NOT EXISTS sentinel_opportunities (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL DEFAULT '',
    opportunity_key TEXT NOT NULL,
    category TEXT NOT NULL DEFAULT '',
    title TEXT NOT NULL DEFAULT '',
    score INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'queued',
    confidence TEXT NOT NULL DEFAULT '',
    why_json TEXT NOT NULL DEFAULT '[]',
    evidence_json TEXT NOT NULL DEFAULT '[]',
    recommended_action_json TEXT NOT NULL DEFAULT '{}',
    source TEXT NOT NULL DEFAULT '',
    record_json TEXT NOT NULL DEFAULT '{}',
    first_seen TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    last_seen TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(scan_id,target_url,opportunity_key)
);
CREATE INDEX IF NOT EXISTS idx_sentinel_opportunities_inbox ON sentinel_opportunities(project_id,status,score DESC,last_seen DESC);
CREATE INDEX IF NOT EXISTS idx_sentinel_opportunities_scan ON sentinel_opportunities(scan_id,target_url,score DESC);
CREATE TABLE IF NOT EXISTS sentinel_scan_contexts (
    scan_id TEXT PRIMARY KEY REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    environment TEXT NOT NULL DEFAULT '',
    auth_profile_name TEXT NOT NULL DEFAULT '',
    auth_type TEXT NOT NULL DEFAULT 'none',
    authenticated INTEGER NOT NULL DEFAULT 0,
    ci_provider TEXT NOT NULL DEFAULT '',
    repository_url TEXT NOT NULL DEFAULT '',
    branch TEXT NOT NULL DEFAULT '',
    commit_sha TEXT NOT NULL DEFAULT '',
    build_id TEXT NOT NULL DEFAULT '',
    policy_json TEXT NOT NULL DEFAULT '{}',
    gate_status TEXT NOT NULL DEFAULT 'not_evaluated',
    gate_reason TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE TABLE IF NOT EXISTS browser_auth_sessions (
    id TEXT PRIMARY KEY,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name TEXT NOT NULL DEFAULT '',
    entry_url TEXT NOT NULL,
    final_url TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'capturing',
    scope_hosts_json TEXT NOT NULL DEFAULT '[]',
    cookie_count INTEGER NOT NULL DEFAULT 0,
    header_count INTEGER NOT NULL DEFAULT 0,
    storage_count INTEGER NOT NULL DEFAULT 0,
    captured_request_count INTEGER NOT NULL DEFAULT 0,
    session_json TEXT NOT NULL DEFAULT '{}',
    last_validated_at TEXT NOT NULL DEFAULT '',
    expires_at TEXT NOT NULL DEFAULT '',
    last_error TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE INDEX IF NOT EXISTS idx_browser_auth_sessions_project ON browser_auth_sessions(project_id,status,updated_at DESC);
CREATE TABLE IF NOT EXISTS appsec_vulnerabilities (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    fingerprint TEXT NOT NULL,
    title TEXT NOT NULL,
    vulnerability_type TEXT NOT NULL DEFAULT '',
    severity TEXT NOT NULL DEFAULT 'info',
    status TEXT NOT NULL DEFAULT 'open',
    confidence TEXT NOT NULL DEFAULT '',
    asset TEXT NOT NULL DEFAULT '',
    environment TEXT NOT NULL DEFAULT '',
    url TEXT NOT NULL DEFAULT '',
    http_method TEXT NOT NULL DEFAULT '',
    parameter TEXT NOT NULL DEFAULT '',
    file TEXT NOT NULL DEFAULT '',
    symbol TEXT NOT NULL DEFAULT '',
    start_line INTEGER NOT NULL DEFAULT 0,
    correlation_score INTEGER NOT NULL DEFAULT 0,
    correlation_json TEXT NOT NULL DEFAULT '{}',
    first_seen TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    last_seen TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    owner TEXT NOT NULL DEFAULT '',
    UNIQUE(project_id,fingerprint)
);
CREATE TABLE IF NOT EXISTS appsec_vulnerability_sources (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    vulnerability_id INTEGER NOT NULL REFERENCES appsec_vulnerabilities(id) ON DELETE CASCADE,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    finding_id INTEGER REFERENCES sentinel_findings(id) ON DELETE CASCADE,
    source_type TEXT NOT NULL,
    source_key TEXT NOT NULL,
    engine TEXT NOT NULL DEFAULT '',
    evidence_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(vulnerability_id,scan_id,source_type,source_key)
);
CREATE INDEX IF NOT EXISTS idx_appsec_vuln_project ON appsec_vulnerabilities(project_id,last_seen);
CREATE INDEX IF NOT EXISTS idx_appsec_source_scan ON appsec_vulnerability_sources(scan_id,vulnerability_id);
CREATE TABLE IF NOT EXISTS sentinel_validations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    url TEXT NOT NULL,
    finding_key TEXT NOT NULL DEFAULT 'url-summary',
    finding_kind TEXT NOT NULL DEFAULT '',
    verdict TEXT NOT NULL DEFAULT 'pending',
    severity TEXT NOT NULL DEFAULT '',
    note TEXT NOT NULL DEFAULT '',
    evidence TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(scan_id,url,finding_key)
);
CREATE INDEX IF NOT EXISTS idx_sentinel_scan_updated ON sentinel_scans(updated_at);
CREATE INDEX IF NOT EXISTS idx_sentinel_scan_project_updated ON sentinel_scans(project_id,updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_sentinel_attempt_scan ON sentinel_scan_attempts(scan_id,attempt_number DESC);
CREATE INDEX IF NOT EXISTS idx_sentinel_targets_project_updated ON sentinel_targets(project_id,updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_sentinel_targets_scan ON sentinel_targets(scan_id);
CREATE INDEX IF NOT EXISTS idx_sentinel_validation_scan ON sentinel_validations(scan_id,updated_at);

-- Investigation graph: deterministic browser/AST evidence is kept as first-class
-- data instead of being flattened into generic findings.  The graph is rebuilt
-- idempotently for each scan target, while baselines and learned layers retain
-- cross-scan history.
CREATE TABLE IF NOT EXISTS investigation_nodes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL,
    node_key TEXT NOT NULL,
    node_type TEXT NOT NULL,
    label TEXT NOT NULL DEFAULT '',
    confidence TEXT NOT NULL DEFAULT '',
    value_score INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'observed',
    payload_json TEXT NOT NULL DEFAULT '{}',
    first_seen TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    last_seen TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(scan_id,target_url,node_key)
);
CREATE INDEX IF NOT EXISTS idx_investigation_nodes_scan ON investigation_nodes(scan_id,target_url,node_type);
CREATE INDEX IF NOT EXISTS idx_investigation_nodes_project ON investigation_nodes(project_id,node_type,last_seen DESC);

CREATE TABLE IF NOT EXISTS investigation_edges (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL,
    source_key TEXT NOT NULL,
    relation TEXT NOT NULL,
    target_key TEXT NOT NULL,
    confidence TEXT NOT NULL DEFAULT '',
    evidence_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(scan_id,target_url,source_key,relation,target_key)
);
CREATE INDEX IF NOT EXISTS idx_investigation_edges_scan ON investigation_edges(scan_id,target_url,source_key);

CREATE TABLE IF NOT EXISTS investigation_actions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL,
    action_key TEXT NOT NULL,
    state_key TEXT NOT NULL DEFAULT '',
    action_type TEXT NOT NULL DEFAULT 'interaction',
    label TEXT NOT NULL DEFAULT '',
    outcome TEXT NOT NULL DEFAULT '',
    value_score INTEGER NOT NULL DEFAULT 0,
    protocol_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(scan_id,target_url,action_key)
);
CREATE INDEX IF NOT EXISTS idx_investigation_actions_scan ON investigation_actions(scan_id,target_url,value_score DESC);

CREATE TABLE IF NOT EXISTS investigation_api_models (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL,
    api_key TEXT NOT NULL,
    method TEXT NOT NULL DEFAULT 'UNKNOWN',
    url TEXT NOT NULL DEFAULT '',
    normalized_path TEXT NOT NULL DEFAULT '',
    source TEXT NOT NULL DEFAULT '',
    confidence TEXT NOT NULL DEFAULT '',
    auth_scope TEXT NOT NULL DEFAULT 'unknown',
    parameters_json TEXT NOT NULL DEFAULT '[]',
    request_schema_json TEXT NOT NULL DEFAULT '{}',
    response_schema_json TEXT NOT NULL DEFAULT '{}',
    state_keys_json TEXT NOT NULL DEFAULT '[]',
    action_keys_json TEXT NOT NULL DEFAULT '[]',
    identity_keys_json TEXT NOT NULL DEFAULT '[]',
    observed_count INTEGER NOT NULL DEFAULT 1,
    baseline_status TEXT NOT NULL DEFAULT 'new',
    payload_json TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(scan_id,target_url,api_key)
);
CREATE INDEX IF NOT EXISTS idx_investigation_api_scan ON investigation_api_models(scan_id,target_url,baseline_status);
CREATE INDEX IF NOT EXISTS idx_investigation_api_project ON investigation_api_models(project_id,normalized_path,method);

CREATE TABLE IF NOT EXISTS investigation_hypotheses (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL,
    hypothesis_key TEXT NOT NULL,
    category TEXT NOT NULL DEFAULT '',
    title TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'candidate',
    score INTEGER NOT NULL DEFAULT 0,
    confidence TEXT NOT NULL DEFAULT '',
    contract_json TEXT NOT NULL DEFAULT '{}',
    evidence_json TEXT NOT NULL DEFAULT '[]',
    decision_json TEXT NOT NULL DEFAULT '{}',
    source_opportunity_key TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(scan_id,target_url,hypothesis_key)
);
CREATE INDEX IF NOT EXISTS idx_investigation_hypotheses_queue ON investigation_hypotheses(project_id,status,score DESC,updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_investigation_hypotheses_scan ON investigation_hypotheses(scan_id,target_url,score DESC);

-- A mutation-capable verification is disabled by default. Approval is scoped
-- to one hypothesis, endpoint/method contract, a small attempt budget and an
-- expiry time so entering the validation queue never implies broad consent.
CREATE TABLE IF NOT EXISTS investigation_mutation_approvals (
    hypothesis_id INTEGER PRIMARY KEY REFERENCES investigation_hypotheses(id) ON DELETE CASCADE,
    approved INTEGER NOT NULL DEFAULT 0,
    scope_json TEXT NOT NULL DEFAULT '{}',
    max_attempts INTEGER NOT NULL DEFAULT 1,
    note TEXT NOT NULL DEFAULT '',
    expires_at TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE INDEX IF NOT EXISTS idx_investigation_mutation_expiry ON investigation_mutation_approvals(approved,expires_at);

CREATE TABLE IF NOT EXISTS investigation_identity_diffs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL,
    api_key TEXT NOT NULL,
    left_identity_key TEXT NOT NULL,
    right_identity_key TEXT NOT NULL,
    difference_type TEXT NOT NULL DEFAULT '',
    risk_score INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'observed',
    matrix_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(scan_id,target_url,api_key,left_identity_key,right_identity_key,difference_type)
);
CREATE INDEX IF NOT EXISTS idx_investigation_identity_scan ON investigation_identity_diffs(scan_id,target_url,risk_score DESC);

CREATE TABLE IF NOT EXISTS investigation_metrics (
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    node_count INTEGER NOT NULL DEFAULT 0,
    edge_count INTEGER NOT NULL DEFAULT 0,
    state_count INTEGER NOT NULL DEFAULT 0,
    action_count INTEGER NOT NULL DEFAULT 0,
    api_count INTEGER NOT NULL DEFAULT 0,
    parameter_count INTEGER NOT NULL DEFAULT 0,
    hypothesis_count INTEGER NOT NULL DEFAULT 0,
    added_count INTEGER NOT NULL DEFAULT 0,
    changed_count INTEGER NOT NULL DEFAULT 0,
    removed_count INTEGER NOT NULL DEFAULT 0,
    duplicate_count INTEGER NOT NULL DEFAULT 0,
    information_gain INTEGER NOT NULL DEFAULT 0,
    token_worthy INTEGER NOT NULL DEFAULT 0,
    stop_reason TEXT NOT NULL DEFAULT '',
    decision_json TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    PRIMARY KEY(scan_id,target_url)
);
CREATE INDEX IF NOT EXISTS idx_investigation_metrics_project ON investigation_metrics(project_id,information_gain DESC,updated_at DESC);

CREATE TABLE IF NOT EXISTS investigation_baselines (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL,
    identity_key TEXT NOT NULL DEFAULT 'anonymous',
    source_scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    signature TEXT NOT NULL,
    api_signatures_json TEXT NOT NULL DEFAULT '[]',
    parameter_signatures_json TEXT NOT NULL DEFAULT '[]',
    metrics_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(project_id,target_url,identity_key,source_scan_id)
);
CREATE INDEX IF NOT EXISTS idx_investigation_baseline_lookup ON investigation_baselines(project_id,target_url,identity_key,created_at DESC);

CREATE TABLE IF NOT EXISTS knowledge_facts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    fact_key TEXT NOT NULL,
    fact_type TEXT NOT NULL,
    subject TEXT NOT NULL DEFAULT '',
    predicate TEXT NOT NULL DEFAULT '',
    object_json TEXT NOT NULL DEFAULT '{}',
    confidence TEXT NOT NULL DEFAULT '',
    source_scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL DEFAULT '',
    evidence_hash TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    last_seen TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(project_id,fact_key,source_scan_id,target_url)
);
CREATE INDEX IF NOT EXISTS idx_knowledge_facts_subject ON knowledge_facts(project_id,fact_type,subject,last_seen DESC);

CREATE TABLE IF NOT EXISTS knowledge_strategies (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    strategy_key TEXT NOT NULL,
    category TEXT NOT NULL DEFAULT '',
    title TEXT NOT NULL DEFAULT '',
    conditions_json TEXT NOT NULL DEFAULT '{}',
    playbook_json TEXT NOT NULL DEFAULT '{}',
    support_count INTEGER NOT NULL DEFAULT 0,
    success_count INTEGER NOT NULL DEFAULT 0,
    failure_count INTEGER NOT NULL DEFAULT 0,
    promoted INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(project_id,strategy_key)
);
CREATE INDEX IF NOT EXISTS idx_knowledge_strategies_project ON knowledge_strategies(project_id,promoted,support_count DESC);

CREATE TABLE IF NOT EXISTS knowledge_outcomes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL,
    hypothesis_key TEXT NOT NULL DEFAULT '',
    strategy_key TEXT NOT NULL DEFAULT '',
    outcome TEXT NOT NULL DEFAULT '',
    stop_reason TEXT NOT NULL DEFAULT '',
    evidence_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(scan_id,target_url,hypothesis_key,strategy_key)
);
CREATE INDEX IF NOT EXISTS idx_knowledge_outcomes_strategy ON knowledge_outcomes(project_id,strategy_key,outcome);

CREATE TABLE IF NOT EXISTS targets (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    target_type TEXT NOT NULL,
    value TEXT NOT NULL,
    normalized_value TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(project_id, target_type, normalized_value)
);

CREATE TABLE IF NOT EXISTS runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    profile_id INTEGER REFERENCES config_profiles(id) ON DELETE SET NULL,
    name TEXT NOT NULL,
    pipeline TEXT NOT NULL DEFAULT 'collect',
    status TEXT NOT NULL DEFAULT 'queued',
    stage TEXT NOT NULL DEFAULT 'queued',
    progress REAL NOT NULL DEFAULT 0,
    processed INTEGER NOT NULL DEFAULT 0,
    total INTEGER NOT NULL DEFAULT 0,
    config_snapshot TEXT NOT NULL DEFAULT '{}',
    output_dir TEXT NOT NULL DEFAULT '',
    error TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    started_at TEXT,
    finished_at TEXT
);

CREATE TABLE IF NOT EXISTS assets (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    asset_key TEXT NOT NULL UNIQUE,
    company TEXT NOT NULL DEFAULT '',
    host TEXT NOT NULL DEFAULT '',
    link TEXT NOT NULL DEFAULT '',
    ip TEXT NOT NULL DEFAULT '',
    port TEXT NOT NULL DEFAULT '',
    protocol TEXT NOT NULL DEFAULT '',
    domain TEXT NOT NULL DEFAULT '',
    title TEXT NOT NULL DEFAULT '',
    status_code TEXT NOT NULL DEFAULT '',
    probe_outcome TEXT NOT NULL DEFAULT '',
    probe_entry_state TEXT NOT NULL DEFAULT '',
    review_tier TEXT NOT NULL DEFAULT '',
    content_category TEXT NOT NULL DEFAULT '',
    score TEXT NOT NULL DEFAULT '',
    state_hash TEXT NOT NULL DEFAULT '',
    probe_hash TEXT NOT NULL DEFAULT '',
    canonical_key TEXT NOT NULL DEFAULT '',
    extra_json TEXT NOT NULL DEFAULT '{}',
    first_seen TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    last_seen TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    last_alive TEXT
);

CREATE TABLE IF NOT EXISTS project_assets (
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    asset_id INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
    decision TEXT NOT NULL DEFAULT 'pending',
    note TEXT NOT NULL DEFAULT '',
    is_deleted INTEGER NOT NULL DEFAULT 0,
    deleted_at TEXT,
    first_seen TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    last_seen TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    last_run_id INTEGER REFERENCES runs(id) ON DELETE SET NULL,
    PRIMARY KEY(project_id, asset_id)
);

CREATE TABLE IF NOT EXISTS asset_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    asset_id INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
    run_id INTEGER REFERENCES runs(id) ON DELETE SET NULL,
    event_type TEXT NOT NULL,
    summary TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

CREATE TABLE IF NOT EXISTS logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER REFERENCES runs(id) ON DELETE CASCADE,
    level TEXT NOT NULL DEFAULT 'info',
    stage TEXT NOT NULL DEFAULT '',
    message TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

CREATE TABLE IF NOT EXISTS saved_views (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    columns_json TEXT NOT NULL DEFAULT '[]',
    filters_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

CREATE INDEX IF NOT EXISTS idx_targets_project ON targets(project_id);
CREATE INDEX IF NOT EXISTS idx_runs_project_created ON runs(project_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_assets_ip ON assets(ip);
CREATE INDEX IF NOT EXISTS idx_assets_domain ON assets(domain);
CREATE INDEX IF NOT EXISTS idx_assets_host ON assets(host);
CREATE INDEX IF NOT EXISTS idx_assets_probe ON assets(probe_outcome);
CREATE INDEX IF NOT EXISTS idx_assets_tier ON assets(review_tier);
CREATE INDEX IF NOT EXISTS idx_assets_review_order ON assets(review_tier, score, last_seen DESC);
CREATE INDEX IF NOT EXISTS idx_project_assets_project_deleted ON project_assets(project_id, is_deleted);
CREATE INDEX IF NOT EXISTS idx_project_assets_project_decision ON project_assets(project_id, is_deleted, decision, last_seen DESC);
CREATE INDEX IF NOT EXISTS idx_project_assets_asset ON project_assets(asset_id);
CREATE INDEX IF NOT EXISTS idx_events_project_created ON asset_events(project_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_logs_run_created ON logs(run_id, created_at DESC);
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn initializes_security_opportunity_inbox() {
        let root =
            std::env::temp_dir().join(format!("oviraptor-opportunity-inbox-{}", Uuid::new_v4()));
        let path = initialize(&root).unwrap();
        let connection = open(&path).unwrap();
        let columns: Vec<String> = connection
            .prepare("PRAGMA table_info(sentinel_opportunities)")
            .unwrap()
            .query_map([], |row| row.get(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        for expected in [
            "opportunity_key",
            "score",
            "status",
            "why_json",
            "evidence_json",
            "recommended_action_json",
            "record_json",
        ] {
            assert!(
                columns.iter().any(|column| column == expected),
                "missing {expected}"
            );
        }
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn initializes_learning_candidate_lifecycle_table() {
        let root =
            std::env::temp_dir().join(format!("oviraptor-learning-candidate-{}", Uuid::new_v4()));
        let path = initialize(&root).unwrap();
        let connection = open(&path).unwrap();
        let table: String = connection
            .query_row(
                "SELECT name FROM sqlite_master WHERE type='table' AND name='strix_learning_candidates'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(table, "strix_learning_candidates");
        let columns: Vec<String> = connection
            .prepare("PRAGMA table_info(strix_learning_candidates)")
            .unwrap()
            .query_map([], |row| row.get(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        for expected in [
            "scan_id",
            "candidate_json",
            "status",
            "target_skill_id",
            "source_hash",
        ] {
            assert!(
                columns.iter().any(|column| column == expected),
                "missing {expected}"
            );
        }
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn target_asset_backfill_is_indexed_and_runs_only_once() {
        let root = std::env::temp_dir().join(format!("oviraptor-db-migration-{}", Uuid::new_v4()));
        let path = initialize(&root).unwrap();
        let connection = open(&path).unwrap();
        connection
            .execute("INSERT INTO projects(name) VALUES('Migration')", [])
            .unwrap();
        let project_id = connection.last_insert_rowid();
        connection
            .execute(
                "INSERT INTO assets(asset_key,link,canonical_key) VALUES('asset','https://matched.invalid/','https://matched.invalid')",
                [],
            )
            .unwrap();
        let asset_id = connection.last_insert_rowid();
        connection
            .execute(
                "INSERT INTO project_assets(project_id,asset_id) VALUES(?1,?2)",
                params![project_id, asset_id],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO sentinel_scans(id,project_id,project_name) VALUES('migration-scan',?1,'Migration')",
                [project_id],
            )
            .unwrap();
        for url in ["https://matched.invalid/", "https://url-only.invalid/"] {
            connection
                .execute(
                    "INSERT INTO sentinel_targets(project_id,scan_id,url) VALUES(?1,'migration-scan',?2)",
                    params![project_id, url],
                )
                .unwrap();
        }
        connection
            .execute(
                "DELETE FROM app_settings WHERE key='sentinel_asset_backfill_version'",
                [],
            )
            .unwrap();
        drop(connection);

        initialize(&root).unwrap();
        let connection = open(&path).unwrap();
        let matched: Option<i64> = connection
            .query_row(
                "SELECT asset_id FROM sentinel_targets WHERE url='https://matched.invalid/'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let unmatched: Option<i64> = connection
            .query_row(
                "SELECT asset_id FROM sentinel_targets WHERE url='https://url-only.invalid/'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(matched, Some(asset_id));
        assert_eq!(unmatched, None);
        assert_eq!(
            migration_version(&connection, "sentinel_asset_backfill_version"),
            1
        );

        connection
            .execute(
                "UPDATE sentinel_targets SET asset_id=NULL WHERE url='https://matched.invalid/'",
                [],
            )
            .unwrap();
        drop(connection);
        initialize(&root).unwrap();
        let connection = open(&path).unwrap();
        let not_repeated: Option<i64> = connection
            .query_row(
                "SELECT asset_id FROM sentinel_targets WHERE url='https://matched.invalid/'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(not_repeated, None);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn relabels_completed_scan_when_every_target_is_recon_only() {
        let root = std::env::temp_dir().join(format!(
            "oviraptor-recon-only-status-test-{}",
            Uuid::new_v4()
        ));
        let path = initialize(&root).unwrap();
        let connection = open(&path).unwrap();
        connection
            .execute("INSERT INTO projects(name) VALUES('Recon only')", [])
            .unwrap();
        let project_id = connection.last_insert_rowid();
        connection
            .execute(
                "INSERT INTO sentinel_scans(id,project_id,project_name,status,scan_type) VALUES('recon-only-scan',?1,'Recon only','completed','web')",
                [project_id],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO sentinel_targets(project_id,scan_id,url,status,scan_mode) VALUES(?1,'recon-only-scan','https://static.invalid','recon_only','skip')",
                [project_id],
            )
            .unwrap();
        drop(connection);

        initialize(&root).unwrap();
        let connection = open(&path).unwrap();
        let status: String = connection
            .query_row(
                "SELECT status FROM sentinel_scans WHERE id='recon-only-scan'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "recon_only");
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}
