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

fn attempt_target_urls(work_dir: &str) -> Vec<String> {
    let Ok(bytes) = fs::read(Path::new(work_dir).join("targets.json")) else {
        return Vec::new();
    };
    serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| {
            value.as_str().map(str::to_string).or_else(|| {
                value
                    .get("url")
                    .and_then(|url| url.as_str())
                    .map(str::to_string)
            })
        })
        .filter(|url| !url.trim().is_empty())
        .collect()
}

fn backfill_sentinel_target_attempts(connection: &Connection) -> Result<(), String> {
    if migration_version(connection, "sentinel_target_attempt_version") >= 1 {
        return Ok(());
    }
    let attempts = {
        let mut statement = connection
            .prepare(
                "SELECT scan_id,attempt_number,work_dir FROM sentinel_scan_attempts WHERE trim(work_dir)<>'' ORDER BY scan_id,attempt_number",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        rows
    };
    for (scan_id, attempt_number, work_dir) in attempts {
        for url in attempt_target_urls(&work_dir) {
            connection
                .execute(
                    "UPDATE sentinel_targets SET last_attempt_number=MAX(last_attempt_number,?1) WHERE scan_id=?2 AND url=?3",
                    params![attempt_number, scan_id, url],
                )
                .map_err(|error| error.to_string())?;
        }
    }
    finish_migration(connection, "sentinel_target_attempt_version", 1)
}

fn concise_attempt_detail(value: &str) -> String {
    let value = value.trim();
    let detail = value
        .rsplit_once("可重试未完成阶段：")
        .map(|(_, detail)| detail)
        .unwrap_or(value)
        .trim();
    detail.chars().take(420).collect()
}

fn repair_latest_attempt_summaries(connection: &Connection) -> Result<(), String> {
    if migration_version(connection, "sentinel_attempt_scope_summary_version") >= 1 {
        return Ok(());
    }
    let attempts = {
        let mut statement = connection
            .prepare(
                "SELECT s.id,s.attempt_count,a.status FROM sentinel_scans s JOIN sentinel_scan_attempts a ON a.scan_id=s.id AND a.attempt_number=s.attempt_count WHERE s.scan_type='web' AND s.attempt_count>0 AND a.status IN ('completed','partial','failed','limited','cancelled')",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        rows
    };
    for (scan_id, attempt_number, attempt_status) in attempts {
        let rows = {
            let mut statement = connection
                .prepare(
                    "SELECT status,routing_reason FROM sentinel_targets WHERE scan_id=?1 AND last_attempt_number=?2 ORDER BY id",
                )
                .map_err(|error| error.to_string())?;
            let rows = statement
                .query_map(params![scan_id, attempt_number], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?;
            rows
        };
        if rows.is_empty() {
            continue;
        }
        let count = |status: &str| rows.iter().filter(|row| row.0 == status).count();
        let completed = count("completed");
        let partial = count("partial");
        let recon_only = count("recon_only");
        let manual_review = count("manual_review");
        let limited = count("limited");
        let failed = count("failed");
        let deferred = rows
            .len()
            .saturating_sub(completed + partial + recon_only + manual_review + limited + failed);
        let mut summary = if partial + limited + failed + deferred == 0 {
            format!(
                "本轮执行完成：自动验证 {completed}，确定性侦察收口 {recon_only}，复杂前端自动收口 {manual_review}，没有异常中断"
            )
        } else {
            format!(
                "本轮未完整结束：自动验证 {completed}，待补充验证 {partial}，确定性侦察收口 {recon_only}，复杂前端自动收口 {manual_review}，熔断 {limited}，执行失败 {failed}，未处理 {deferred}"
            )
        };
        let details = rows
            .iter()
            .filter(|row| matches!(row.0.as_str(), "partial" | "limited" | "failed"))
            .filter_map(|row| {
                let detail = concise_attempt_detail(&row.1);
                (!detail.is_empty()).then_some(detail)
            })
            .take(3)
            .collect::<Vec<_>>();
        if !details.is_empty() {
            summary.push_str("；本轮原因：");
            summary.push_str(&details.join("；"));
        }
        connection
            .execute(
                "UPDATE sentinel_scan_attempts SET status=?1,stage=CASE WHEN ?1 IN ('completed','partial') THEN 'complete' ELSE 'stopped' END,checkpoint=?2,stop_reason=?2,updated_at=datetime('now','localtime') WHERE scan_id=?3 AND attempt_number=?4",
                params![attempt_status, summary, scan_id, attempt_number],
            )
            .map_err(|error| error.to_string())?;
        connection
            .execute(
                "UPDATE sentinel_scans SET current_checkpoint=?1 WHERE id=?2",
                params![
                    format!("最新第 {attempt_number} 次执行：{summary}"),
                    scan_id
                ],
            )
            .map_err(|error| error.to_string())?;
    }
    finish_migration(connection, "sentinel_attempt_scope_summary_version", 1)
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
        "ALTER TABLE sentinel_targets ADD COLUMN last_attempt_number INTEGER NOT NULL DEFAULT 0",
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
    ensure_column(
        &connection,
        "browser_auth_sessions",
        "capture_previous_status",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_column(
        &connection,
        "browser_auth_sessions",
        "owner_scan_id",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_column(
        &connection,
        "browser_auth_sessions",
        "draft_scope_id",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    connection
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_browser_auth_sessions_task_scope ON browser_auth_sessions(owner_scan_id,draft_scope_id,project_id,updated_at DESC)",
            [],
        )
        .map_err(|error| format!("创建任务会话作用域索引失败：{error}"))?;
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
                execution_mode TEXT NOT NULL DEFAULT 'initial',
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
                owner_scan_id TEXT NOT NULL DEFAULT '',
                draft_scope_id TEXT NOT NULL DEFAULT '',
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
                capture_previous_status TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
            );
            CREATE INDEX IF NOT EXISTS idx_browser_auth_sessions_project ON browser_auth_sessions(project_id,status,updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_browser_auth_sessions_task_scope ON browser_auth_sessions(owner_scan_id,draft_scope_id,project_id,updated_at DESC);
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
    ensure_column(
        &connection,
        "sentinel_scan_attempts",
        "execution_mode",
        "TEXT NOT NULL DEFAULT 'initial'",
    )?;
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
    // Only confirmed protection-system blocks become persistent fuse entries.
    // Budget, context and no-progress stops remain retryable checkpoints.
    connection.execute(
        "INSERT OR IGNORE INTO sentinel_fuse_zone(project_id,asset_id,company,url,normalized_url,source_scan_id,reason) SELECT project_id,asset_id,company,url,lower(rtrim(trim(url),'/')),COALESCE(scan_id,''),routing_reason FROM sentinel_targets WHERE status='limited' AND trim(url)<>'' AND (lower(routing_reason) LIKE '%waf%' OR lower(routing_reason) LIKE '%captcha%' OR lower(routing_reason) LIKE '%cloudflare%' OR lower(routing_reason) LIKE '%rate limit%' OR routing_reason LIKE '%验证码%' OR routing_reason LIKE '%人机验证%' OR routing_reason LIKE '%持续限流%')",
        [],
    ).map_err(|error| error.to_string())?;
    if migration_version(&connection, "soft_fuse_cleanup_version") < 1 {
        connection.execute(
            "UPDATE sentinel_fuse_zone SET archived=1,note='旧版将预算、上下文或无进展软暂停误记为熔断；现已恢复为可继续任务',updated_at=datetime('now','localtime') WHERE archived=0 AND verdict='pending' AND trim(evidence)='' AND NOT (lower(reason) LIKE '%waf%' OR lower(reason) LIKE '%captcha%' OR lower(reason) LIKE '%cloudflare%' OR lower(reason) LIKE '%rate limit%' OR reason LIKE '%验证码%' OR reason LIKE '%人机验证%' OR reason LIKE '%持续限流%')",
            [],
        ).map_err(|error| error.to_string())?;
        finish_migration(&connection, "soft_fuse_cleanup_version", 1)?;
    }
    if migration_version(&connection, "builtin_src_assurance_version") < 1 {
        connection.execute(
            "UPDATE config_profiles SET settings_json=json_remove(settings_json,'$.strixOastEndpoint','$.strixRawHttpEnabled','$.strixRaceEnabled','$.strixMaxRaceConcurrency','$.strixControlledWriteEnabled','$.strixAttackChainEnabled'),updated_at=datetime('now','localtime') WHERE json_valid(settings_json)",
            [],
        ).map_err(|error| error.to_string())?;
        finish_migration(&connection, "builtin_src_assurance_version", 1)?;
    }
    if migration_version(&connection, "strix_false_completion_repair_version") < 1 {
        connection.execute_batch(
            r#"
            UPDATE sentinel_targets
            SET status='partial',
                routing_reason=CASE
                  WHEN routing_reason LIKE '%历史修复：未取得目标工具证据%' THEN routing_reason
                  ELSE routing_reason || '；历史修复：未取得目标工具证据，不计入自动验证完成'
                END,
                updated_at=datetime('now','localtime')
            WHERE status='completed'
              AND routing_reason LIKE '%自动验证已按边界收口（本轮未形成新的工具证据）%'
              AND (
                routing_reason LIKE '%没有取得目标请求/响应%'
                OR routing_reason LIKE '%没有形成可用工具结果%'
                OR routing_reason LIKE '%没有形成任何工具证据%'
                OR routing_reason LIKE '%只读取了本地证据%'
              );

            UPDATE sentinel_scans
            SET status='partial',
                current_checkpoint='调查已收口：自动验证 '
                  || (SELECT COUNT(*) FROM sentinel_targets t WHERE t.scan_id=sentinel_scans.id AND t.status='completed')
                  || '，保留待验证 '
                  || (SELECT COUNT(*) FROM sentinel_targets t WHERE t.scan_id=sentinel_scans.id AND t.status='partial')
                  || '，仅侦察收口 '
                  || (SELECT COUNT(*) FROM sentinel_targets t WHERE t.scan_id=sentinel_scans.id AND t.status='recon_only')
                  || '；旧版曾将未取得目标请求/响应的回合误记为完成，现已校正',
                updated_at=datetime('now','localtime')
            WHERE scan_type='web'
              AND status='completed'
              AND EXISTS(
                SELECT 1 FROM sentinel_targets t
                WHERE t.scan_id=sentinel_scans.id
                  AND t.status='partial'
                  AND t.routing_reason LIKE '%历史修复：未取得目标工具证据%'
              );
            "#,
        ).map_err(|error| format!("修复 Strix 假完成历史状态失败：{error}"))?;
        finish_migration(&connection, "strix_false_completion_repair_version", 1)?;
    }
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
    let _ = connection.execute(
        "ALTER TABLE sentinel_targets ADD COLUMN last_attempt_number INTEGER NOT NULL DEFAULT 0",
        [],
    );
    backfill_sentinel_target_attempts(&connection)?;
    repair_latest_attempt_summaries(&connection)?;
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
CREATE TABLE IF NOT EXISTS investigation_validations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL,
    opportunity_id INTEGER REFERENCES sentinel_opportunities(id) ON DELETE SET NULL,
    hypothesis_id INTEGER REFERENCES investigation_hypotheses(id) ON DELETE SET NULL,
    api_key TEXT NOT NULL DEFAULT '',
    identity_id TEXT NOT NULL DEFAULT '',
    method TEXT NOT NULL DEFAULT 'GET',
    request_url TEXT NOT NULL DEFAULT '',
    request_headers_json TEXT NOT NULL DEFAULT '{}',
    request_body TEXT NOT NULL DEFAULT '',
    response_status INTEGER NOT NULL DEFAULT 0,
    response_status_text TEXT NOT NULL DEFAULT '',
    response_headers_json TEXT NOT NULL DEFAULT '{}',
    response_body TEXT NOT NULL DEFAULT '',
    decoded_body TEXT NOT NULL DEFAULT '',
    verdict TEXT NOT NULL DEFAULT 'needs_more_evidence',
    severity TEXT NOT NULL DEFAULT 'info',
    confidence TEXT NOT NULL DEFAULT 'low',
    ai_assessment TEXT NOT NULL DEFAULT '',
    note TEXT NOT NULL DEFAULT '',
    next_action TEXT NOT NULL DEFAULT '',
    evidence_refs_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE INDEX IF NOT EXISTS idx_investigation_validations_scan ON investigation_validations(scan_id,updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_investigation_validations_opportunity ON investigation_validations(opportunity_id,updated_at DESC);
        "#).map_err(|error| error.to_string())?;
    }
    // Older opportunity scoring treated inferred paths, frontend routes and
    // fingerprint knowledge as directly verifiable. Reclassify them once as
    // evidence-enrichment work. Only a concrete request contract or fresh
    // runtime/probe response may remain in the Strix verification queue.
    if migration_version(&connection, "opportunity_readiness_gate_version") < 2 {
        connection.execute_batch(r#"
            UPDATE sentinel_opportunities
            SET status='queued',
                record_json=CASE WHEN json_valid(record_json) THEN json_set(
                    record_json,
                    '$.candidateOnly', CASE WHEN source='evidence-reconstruction' THEN json('true') ELSE COALESCE(json_extract(record_json,'$.candidateOnly'),json('false')) END,
                    '$.readiness.stage', CASE
                        WHEN category='frontend_feature' THEN 'needs_runtime'
                        WHEN category='product_match' THEN 'template_match'
                        ELSE 'needs_contract' END,
                    '$.readiness.reason', CASE
                        WHEN category='frontend_feature' THEN 'frontend_route_must_be_rendered_before_security_validation'
                        WHEN category='product_match' THEN 'fingerprint_selects_a_poc_but_is_not_vulnerability_evidence'
                        ELSE 'inferred_candidate_missing_verified_method_or_response' END
                ) ELSE record_json END,
                last_seen=datetime('now','localtime')
            WHERE status IN ('ready','in_progress') AND (
                category IN ('frontend_feature','product_match','fallback_discovery')
                OR source IN ('evidence-reconstruction','string-heuristic','regex-fallback','route-structure-fallback','fingerprint')
                OR upper(COALESCE(json_extract(record_json,'$.method'),'')) IN ('','UNKNOWN')
            );

            UPDATE investigation_hypotheses AS hypothesis
            SET status='candidate',
                decision_json=json_set(
                    CASE WHEN json_valid(decision_json) THEN decision_json ELSE '{}' END,
                    '$.eligibleForModel',json('false'),
                    '$.reason','historical_inferred_candidate_reclassified'
                ),
                updated_at=datetime('now','localtime')
            WHERE status IN ('ready','in_progress') AND EXISTS (
                SELECT 1 FROM sentinel_opportunities AS opportunity
                WHERE opportunity.scan_id=hypothesis.scan_id
                  AND opportunity.target_url=hypothesis.target_url
                  AND opportunity.opportunity_key=hypothesis.source_opportunity_key
                  AND opportunity.status='queued'
            );
        "#).map_err(|error| format!("升级机会验证门禁失败：{error}"))?;
        finish_migration(&connection, "opportunity_readiness_gate_version", 2)?;
    }
    // UNKNOWN static paths are retained in the raw frontend evidence, but they
    // are not user-facing opportunities and must never compete for model
    // budget. Earlier builds left them as queued cards, which made string
    // search output look like captured HTTP traffic.
    if migration_version(&connection, "opportunity_readiness_gate_version") < 3 {
        connection
            .execute_batch(
                r#"
            UPDATE sentinel_opportunities
            SET status='dismissed',
                record_json=CASE WHEN json_valid(record_json) THEN json_set(
                    record_json,
                    '$.disposition','static_clue_only',
                    '$.readiness.stage','static_clue',
                    '$.readiness.reason','missing_observed_or_verified_http_method'
                ) ELSE record_json END,
                last_seen=datetime('now','localtime')
            WHERE status IN ('queued','ready')
              AND upper(COALESCE(json_extract(record_json,'$.method'),'')) IN ('','UNKNOWN')
              AND source<>'runtime-request'
              AND COALESCE(json_extract(record_json,'$.verification.verified'),0)<>1
              AND COALESCE(json_extract(record_json,'$.requestContext.status'),0)<=0;

            UPDATE investigation_hypotheses AS hypothesis
            SET status='rejected',
                decision_json=json_set(
                    CASE WHEN json_valid(decision_json) THEN decision_json ELSE '{}' END,
                    '$.eligibleForModel',json('false'),
                    '$.reason','historical_unknown_static_clue_removed_from_queue'
                ),
                updated_at=datetime('now','localtime')
            WHERE status IN ('candidate','ready','needs_more_evidence') AND EXISTS (
                SELECT 1 FROM sentinel_opportunities AS opportunity
                WHERE opportunity.scan_id=hypothesis.scan_id
                  AND opportunity.target_url=hypothesis.target_url
                  AND opportunity.opportunity_key=hypothesis.source_opportunity_key
                  AND opportunity.status='dismissed'
                  AND json_extract(opportunity.record_json,'$.disposition')='static_clue_only'
            );
        "#,
            )
            .map_err(|error| format!("升级静态接口线索门禁失败：{error}"))?;
        finish_migration(&connection, "opportunity_readiness_gate_version", 3)?;
    }
    // A captured or AST-derived HTTP request proves endpoint existence, not a
    // vulnerability hypothesis. Retire historical ready items that lack the
    // deterministic risk signal introduced by gate v4. The raw API evidence
    // remains intact and validated/manual conclusions are never touched.
    if migration_version(&connection, "opportunity_readiness_gate_version") < 4 {
        connection
            .execute_batch(
                r#"
            UPDATE sentinel_opportunities
            SET status='dismissed',
                record_json=CASE WHEN json_valid(record_json) THEN json_set(
                    record_json,
                    '$.disposition','api_inventory_only',
                    '$.readiness.stage','inventory_only',
                    '$.readiness.reason','formal_api_without_security_risk_signal'
                ) ELSE record_json END,
                last_seen=datetime('now','localtime')
            WHERE status='ready'
              AND COALESCE(json_extract(record_json,'$.riskEvidence.present'),0)<>1;

            UPDATE investigation_hypotheses AS hypothesis
            SET status='rejected',
                decision_json=json_set(
                    CASE WHEN json_valid(decision_json) THEN decision_json ELSE '{}' END,
                    '$.eligibleForModel',json('false'),
                    '$.reason','historical_formal_api_without_security_risk_signal'
                ),
                updated_at=datetime('now','localtime')
            WHERE status IN ('candidate','ready','needs_more_evidence') AND EXISTS (
                SELECT 1 FROM sentinel_opportunities AS opportunity
                WHERE opportunity.scan_id=hypothesis.scan_id
                  AND opportunity.target_url=hypothesis.target_url
                  AND opportunity.opportunity_key=hypothesis.source_opportunity_key
                  AND opportunity.status='dismissed'
                  AND json_extract(opportunity.record_json,'$.disposition')='api_inventory_only'
            );
        "#,
            )
            .map_err(|error| format!("升级正式接口风险门禁失败：{error}"))?;
        finish_migration(&connection, "opportunity_readiness_gate_version", 4)?;
    }
    // Generic transport identifiers such as device_id/client_id identify the
    // browser instance, not an application-owned object. Older v4 scoring
    // treated the `_id` suffix itself as an IDOR signal and promoted ordinary
    // session recovery/callback traffic. Retire only untouched active rows;
    // manual and terminal decisions remain authoritative.
    if migration_version(&connection, "opportunity_readiness_gate_version") < 5 {
        connection
            .execute_batch(
                r#"
            UPDATE sentinel_opportunities
            SET status='dismissed',
                record_json=CASE WHEN json_valid(record_json) THEN json_set(
                    record_json,
                    '$.disposition','transport_identifier_only',
                    '$.readiness.stage','inventory_only',
                    '$.readiness.reason','transport_identifier_is_not_an_object_authorization_boundary'
                ) ELSE record_json END,
                last_seen=datetime('now','localtime')
            WHERE status IN ('queued','ready')
              AND json_valid(record_json)
              AND COALESCE(json_extract(record_json,'$.riskEvidence.signalCount'),0)=1
              AND json_extract(record_json,'$.riskEvidence.signals[0].type')='object_boundary_parameter'
              AND NOT EXISTS (
                  SELECT 1
                  FROM json_each(json_extract(record_json,'$.riskEvidence.signals[0].fields')) AS field
                  WHERE lower(CAST(field.value AS TEXT)) NOT IN (
                      'device_id','deviceid','client_id','clientid','request_id','requestid',
                      'trace_id','traceid','session_id','sessionid','nonce','hkey','_time',
                      'timestamp','version','web_version','x_client_version'
                  )
              );

            UPDATE investigation_hypotheses AS hypothesis
            SET status='rejected',
                decision_json=json_set(
                    CASE WHEN json_valid(decision_json) THEN decision_json ELSE '{}' END,
                    '$.eligibleForModel',json('false'),
                    '$.reason','transport_identifier_removed_from_security_queue'
                ),
                updated_at=datetime('now','localtime')
            WHERE status IN ('candidate','ready','needs_more_evidence') AND EXISTS (
                SELECT 1 FROM sentinel_opportunities AS opportunity
                WHERE opportunity.scan_id=hypothesis.scan_id
                  AND opportunity.target_url=hypothesis.target_url
                  AND opportunity.opportunity_key=hypothesis.source_opportunity_key
                  AND opportunity.status='dismissed'
                  AND json_extract(opportunity.record_json,'$.disposition')='transport_identifier_only'
            );
        "#,
            )
            .map_err(|error| format!("升级传输标识误报门禁失败：{error}"))?;
        finish_migration(&connection, "opportunity_readiness_gate_version", 5)?;
    }
    // Investigation hypotheses use a stable category|method|path source key,
    // while the inbox historically stored a hashed per-observation key. Keep
    // the graph and Action Center consistent after the transport-ID cleanup.
    if migration_version(&connection, "opportunity_readiness_gate_version") < 6 {
        connection
            .execute_batch(
                r#"
            UPDATE investigation_hypotheses AS hypothesis
            SET status='rejected',
                decision_json=json_set(
                    CASE WHEN json_valid(decision_json) THEN decision_json ELSE '{}' END,
                    '$.eligibleForModel',json('false'),
                    '$.reason','transport_identifier_removed_from_security_queue'
                ),
                updated_at=datetime('now','localtime')
            WHERE status IN ('candidate','ready','needs_more_evidence') AND EXISTS (
                SELECT 1 FROM sentinel_opportunities AS opportunity
                WHERE opportunity.scan_id=hypothesis.scan_id
                  AND opportunity.target_url=hypothesis.target_url
                  AND opportunity.status='dismissed'
                  AND json_extract(opportunity.record_json,'$.disposition')='transport_identifier_only'
                  AND lower(opportunity.category)||'|'||upper(COALESCE(json_extract(opportunity.record_json,'$.method'),''))||'|'||lower(COALESCE(json_extract(opportunity.record_json,'$.normalizedPath'),''))
                      = lower(hypothesis.source_opportunity_key)
            );

            UPDATE investigation_metrics AS metric
            SET hypothesis_count=(
                    SELECT COUNT(*) FROM investigation_hypotheses AS hypothesis
                    WHERE hypothesis.scan_id=metric.scan_id
                      AND hypothesis.target_url=metric.target_url
                      AND hypothesis.status IN ('ready','in_progress')
                ),
                decision_json=json_set(
                    CASE WHEN json_valid(decision_json) THEN decision_json ELSE '{}' END,
                    '$.readyHypotheses',(
                        SELECT COUNT(*) FROM investigation_hypotheses AS hypothesis
                        WHERE hypothesis.scan_id=metric.scan_id
                          AND hypothesis.target_url=metric.target_url
                          AND hypothesis.status IN ('ready','in_progress')
                    ),
                    '$.eligibleForModel',CASE WHEN EXISTS (
                        SELECT 1 FROM investigation_hypotheses AS hypothesis
                        WHERE hypothesis.scan_id=metric.scan_id
                          AND hypothesis.target_url=metric.target_url
                          AND hypothesis.status IN ('ready','in_progress')
                    ) THEN json('true') ELSE json('false') END
                ),
                updated_at=datetime('now','localtime');
        "#,
            )
            .map_err(|error| format!("升级机会与调查图谱对账失败：{error}"))?;
        finish_migration(&connection, "opportunity_readiness_gate_version", 6)?;
    }
    // v6 compared a mixed-case method segment against a lower-cased source
    // key. Repeat the reconciliation with one canonical lower-case expression.
    if migration_version(&connection, "opportunity_readiness_gate_version") < 7 {
        connection
            .execute_batch(
                r#"
            UPDATE investigation_hypotheses AS hypothesis
            SET status='rejected',
                decision_json=json_set(
                    CASE WHEN json_valid(decision_json) THEN decision_json ELSE '{}' END,
                    '$.eligibleForModel',json('false'),
                    '$.reason','transport_identifier_removed_from_security_queue'
                ),
                updated_at=datetime('now','localtime')
            WHERE status IN ('candidate','ready','needs_more_evidence') AND EXISTS (
                SELECT 1 FROM sentinel_opportunities AS opportunity
                WHERE opportunity.scan_id=hypothesis.scan_id
                  AND opportunity.target_url=hypothesis.target_url
                  AND opportunity.status='dismissed'
                  AND json_extract(opportunity.record_json,'$.disposition')='transport_identifier_only'
                  AND lower(opportunity.category||'|'||COALESCE(json_extract(opportunity.record_json,'$.method'),'')||'|'||COALESCE(json_extract(opportunity.record_json,'$.normalizedPath'),''))
                      = lower(hypothesis.source_opportunity_key)
            );

            UPDATE investigation_metrics AS metric
            SET hypothesis_count=(
                    SELECT COUNT(*) FROM investigation_hypotheses AS hypothesis
                    WHERE hypothesis.scan_id=metric.scan_id
                      AND hypothesis.target_url=metric.target_url
                      AND hypothesis.status IN ('ready','in_progress')
                ),
                decision_json=json_set(
                    CASE WHEN json_valid(decision_json) THEN decision_json ELSE '{}' END,
                    '$.readyHypotheses',(
                        SELECT COUNT(*) FROM investigation_hypotheses AS hypothesis
                        WHERE hypothesis.scan_id=metric.scan_id
                          AND hypothesis.target_url=metric.target_url
                          AND hypothesis.status IN ('ready','in_progress')
                    ),
                    '$.eligibleForModel',CASE WHEN EXISTS (
                        SELECT 1 FROM investigation_hypotheses AS hypothesis
                        WHERE hypothesis.scan_id=metric.scan_id
                          AND hypothesis.target_url=metric.target_url
                          AND hypothesis.status IN ('ready','in_progress')
                    ) THEN json('true') ELSE json('false') END
                ),
                updated_at=datetime('now','localtime');
        "#,
            )
            .map_err(|error| format!("修复机会与调查图谱大小写对账失败：{error}"))?;
        finish_migration(&connection, "opportunity_readiness_gate_version", 7)?;
    }
    if migration_version(&connection, "automatic_contract_authorization_version") < 1 {
        connection
            .execute_batch(
                r#"
                UPDATE investigation_hypotheses
                SET status=CASE
                      WHEN status IN ('awaiting_authorization','blocked_by_authorization') THEN 'ready'
                      ELSE status
                    END,
                    contract_json=CASE
                      WHEN json_valid(contract_json) THEN json_set(
                        contract_json,
                        '$.mutationPolicy',CASE COALESCE(json_extract(contract_json,'$.mutationPolicy'),'')
                          WHEN 'read_only_unless_explicitly_approved' THEN 'automatic_bounded_same_contract'
                          WHEN 'benign_marker_only_and_cleanup' THEN 'automatic_benign_marker_and_cleanup'
                          WHEN 'discovery_only_no_account_creation' THEN 'automatic_discovery_no_account_creation'
                          WHEN 'read_only_or_non_destructive' THEN 'automatic_bounded_non_destructive'
                          ELSE COALESCE(json_extract(contract_json,'$.mutationPolicy'),'automatic_bounded_non_destructive')
                        END
                      )
                      ELSE json_object('mutationPolicy','automatic_bounded_non_destructive')
                    END,
                    decision_json=CASE
                      WHEN json_valid(decision_json) THEN json_set(
                        decision_json,
                        '$.requiresHuman',json('false'),
                        '$.authorizationMode','automatic_bounded',
                        '$.verificationMode',CASE
                          WHEN status IN ('awaiting_authorization','blocked_by_authorization') THEN 'ai_auto'
                          ELSE COALESCE(json_extract(decision_json,'$.verificationMode'),'ai_auto')
                        END
                      )
                      ELSE json_object(
                        'requiresHuman',json('false'),
                        'authorizationMode','automatic_bounded',
                        'verificationMode','ai_auto'
                      )
                    END,
                    updated_at=datetime('now','localtime')
                WHERE status IN ('ready','in_progress','awaiting_authorization','blocked_by_authorization');
                "#,
            )
            .map_err(|error| format!("迁移 AI 自动验证授权策略失败：{error}"))?;
        finish_migration(&connection, "automatic_contract_authorization_version", 1)?;
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

    // A scan is complete when its bounded queue is exhausted, even when every
    // target legitimately ends at deterministic reconnaissance. Target rows
    // still preserve `recon_only`; the task-level status represents lifecycle
    // completion and must not look like an interruption requiring a retry.
    let _ = connection.execute_batch(
        r#"
        UPDATE sentinel_scans
        SET status='completed',
            current_checkpoint=CASE
              WHEN trim(current_checkpoint)='' THEN '扫描完成：目标均由确定性侦察正常收口，未发生异常中断'
              ELSE current_checkpoint
            END,
            updated_at=datetime('now','localtime')
        WHERE scan_type='web'
          AND status='recon_only'
          AND EXISTS(
            SELECT 1 FROM sentinel_targets t WHERE t.scan_id=sentinel_scans.id
          )
          AND NOT EXISTS(
            SELECT 1 FROM sentinel_targets t
            WHERE t.scan_id=sentinel_scans.id AND t.status NOT IN ('recon_only','manual_review')
          );
        "#,
    );

    // Older investigation-gate runs used `partial` for a deterministic
    // no-high-value stop. That state is not a pause or a failed Strix run:
    // the local evidence is complete and the target is recon-only. Repair the
    // persisted classification once so the queue and resume UI are truthful.
    let _ = connection.execute_batch(
        r#"
        UPDATE sentinel_targets
        SET status='recon_only',
            scan_mode=CASE WHEN scan_mode IN ('', 'quick', 'standard', 'deep') THEN 'skip' ELSE scan_mode END,
            updated_at=datetime('now','localtime')
        WHERE status='partial'
          AND routing_reason LIKE '%no_high_value_hypothesis%';

        UPDATE sentinel_scans
        SET status='recon_only',
            current_checkpoint=CASE
              WHEN trim(current_checkpoint)='' OR current_checkpoint LIKE '%流水线%' THEN '本地前端调查完成：没有达到高价值假设门禁；已保留全部证据'
              ELSE current_checkpoint
            END,
            updated_at=datetime('now','localtime')
        WHERE status='partial'
          AND scan_type='web'
          AND EXISTS(SELECT 1 FROM sentinel_targets t WHERE t.scan_id=sentinel_scans.id)
          AND NOT EXISTS(
            SELECT 1 FROM sentinel_targets t
            WHERE t.scan_id=sentinel_scans.id
              AND t.status NOT IN ('recon_only','manual_review')
          )
          AND EXISTS(
            SELECT 1 FROM sentinel_targets t
            WHERE t.scan_id=sentinel_scans.id
              AND t.routing_reason LIKE '%no_high_value_hypothesis%'
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
    // Version 5 gives cloud models enough time and request budget to consume
    // the richer browser/AST evidence. Local deployments keep the former
    // conservative values at runtime until their policy is tuned separately.
    let _ = connection.execute_batch(
        r#"
        UPDATE config_profiles SET settings_json=json_set(
            settings_json,
            '$.strixFrontendPacketBudgetKb',CASE WHEN json_extract(settings_json,'$.strixFrontendPacketBudgetKb')=12 THEN 24 ELSE json_extract(settings_json,'$.strixFrontendPacketBudgetKb') END,
            '$.strixQuickTimeout',CASE WHEN json_extract(settings_json,'$.strixQuickTimeout')=120 THEN 240 ELSE json_extract(settings_json,'$.strixQuickTimeout') END,
            '$.strixStandardTimeout',CASE WHEN json_extract(settings_json,'$.strixStandardTimeout')=300 THEN 600 ELSE json_extract(settings_json,'$.strixStandardTimeout') END,
            '$.strixDeepTimeout',CASE WHEN json_extract(settings_json,'$.strixDeepTimeout')=600 THEN 1200 ELSE json_extract(settings_json,'$.strixDeepTimeout') END,
            '$.strixQuickTokenLimit',CASE WHEN json_extract(settings_json,'$.strixQuickTokenLimit')=50000 THEN 100000 ELSE json_extract(settings_json,'$.strixQuickTokenLimit') END,
            '$.strixStandardTokenLimit',CASE WHEN json_extract(settings_json,'$.strixStandardTokenLimit')=120000 THEN 300000 ELSE json_extract(settings_json,'$.strixStandardTokenLimit') END,
            '$.strixDeepTokenLimit',CASE WHEN json_extract(settings_json,'$.strixDeepTokenLimit')=250000 THEN 700000 ELSE json_extract(settings_json,'$.strixDeepTokenLimit') END,
            '$.strixQuickRequestLimit',CASE WHEN json_extract(settings_json,'$.strixQuickRequestLimit')=4 THEN 6 ELSE json_extract(settings_json,'$.strixQuickRequestLimit') END,
            '$.strixStandardRequestLimit',CASE WHEN json_extract(settings_json,'$.strixStandardRequestLimit')=8 THEN 14 ELSE json_extract(settings_json,'$.strixStandardRequestLimit') END,
            '$.strixDeepRequestLimit',CASE WHEN json_extract(settings_json,'$.strixDeepRequestLimit')=12 THEN 24 ELSE json_extract(settings_json,'$.strixDeepRequestLimit') END,
            '$.strixNoToolTurnLimit',CASE WHEN json_extract(settings_json,'$.strixNoToolTurnLimit')=4 THEN 6 ELSE json_extract(settings_json,'$.strixNoToolTurnLimit') END,
            '$.strixBudgetPolicyVersion',5
        )
        WHERE json_valid(settings_json)
          AND COALESCE(json_extract(settings_json,'$.strixBudgetPolicyVersion'),0)<5;
        "#,
    );
    // Version 6 removes the accidental local/frontend 50k effective ceiling.
    // Only migrate the shipped defaults; explicit user budgets (including 0)
    // remain untouched.
    let _ = connection.execute_batch(
        r#"
        UPDATE config_profiles SET settings_json=json_set(
            settings_json,
            '$.strixQuickTimeout',CASE WHEN json_extract(settings_json,'$.strixQuickTimeout')=240 THEN 300 ELSE json_extract(settings_json,'$.strixQuickTimeout') END,
            '$.strixStandardTimeout',CASE WHEN json_extract(settings_json,'$.strixStandardTimeout')=600 THEN 480 ELSE json_extract(settings_json,'$.strixStandardTimeout') END,
            '$.strixDeepTimeout',CASE WHEN json_extract(settings_json,'$.strixDeepTimeout')=1200 THEN 900 ELSE json_extract(settings_json,'$.strixDeepTimeout') END,
            '$.strixQuickTokenLimit',CASE WHEN json_extract(settings_json,'$.strixQuickTokenLimit') IN (50000,100000) THEN 200000 ELSE json_extract(settings_json,'$.strixQuickTokenLimit') END,
            '$.strixStandardTokenLimit',CASE WHEN json_extract(settings_json,'$.strixStandardTokenLimit')=300000 THEN 400000 ELSE json_extract(settings_json,'$.strixStandardTokenLimit') END,
            '$.strixDeepTokenLimit',CASE WHEN json_extract(settings_json,'$.strixDeepTokenLimit')=700000 THEN 800000 ELSE json_extract(settings_json,'$.strixDeepTokenLimit') END,
            '$.strixQuickRequestLimit',CASE WHEN json_extract(settings_json,'$.strixQuickRequestLimit')=6 THEN 8 ELSE json_extract(settings_json,'$.strixQuickRequestLimit') END,
            '$.strixStandardRequestLimit',CASE WHEN json_extract(settings_json,'$.strixStandardRequestLimit')=14 THEN 12 ELSE json_extract(settings_json,'$.strixStandardRequestLimit') END,
            '$.strixDeepRequestLimit',CASE WHEN json_extract(settings_json,'$.strixDeepRequestLimit')=24 THEN 16 ELSE json_extract(settings_json,'$.strixDeepRequestLimit') END,
            '$.strixBudgetPolicyVersion',6
        )
        WHERE json_valid(settings_json)
          AND COALESCE(json_extract(settings_json,'$.strixBudgetPolicyVersion'),0)<6;
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
            "strixFrontendPacketBudgetKb": 24,
            "strixBatchSize": 15,
            "strixQuickTimeout": 300,
            "strixStandardTimeout": 480,
            "strixDeepTimeout": 900,
            "strixQuickTokenLimit": 200000,
            "strixStandardTokenLimit": 400000,
            "strixDeepTokenLimit": 800000,
            "strixQuickRequestLimit": 8,
            "strixStandardRequestLimit": 12,
            "strixDeepRequestLimit": 16,
            "strixNoToolTurnLimit": 6,
            "strixBudgetPolicyVersion": 6,
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
    last_attempt_number INTEGER NOT NULL DEFAULT 0,
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
    execution_mode TEXT NOT NULL DEFAULT 'initial',
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
    owner_scan_id TEXT NOT NULL DEFAULT '',
    draft_scope_id TEXT NOT NULL DEFAULT '',
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
    capture_previous_status TEXT NOT NULL DEFAULT '',
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
CREATE TABLE IF NOT EXISTS investigation_validations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL,
    opportunity_id INTEGER REFERENCES sentinel_opportunities(id) ON DELETE SET NULL,
    hypothesis_id INTEGER REFERENCES investigation_hypotheses(id) ON DELETE SET NULL,
    api_key TEXT NOT NULL DEFAULT '',
    identity_id TEXT NOT NULL DEFAULT '',
    method TEXT NOT NULL DEFAULT 'GET',
    request_url TEXT NOT NULL DEFAULT '',
    request_headers_json TEXT NOT NULL DEFAULT '{}',
    request_body TEXT NOT NULL DEFAULT '',
    response_status INTEGER NOT NULL DEFAULT 0,
    response_status_text TEXT NOT NULL DEFAULT '',
    response_headers_json TEXT NOT NULL DEFAULT '{}',
    response_body TEXT NOT NULL DEFAULT '',
    decoded_body TEXT NOT NULL DEFAULT '',
    verdict TEXT NOT NULL DEFAULT 'needs_more_evidence',
    severity TEXT NOT NULL DEFAULT 'info',
    confidence TEXT NOT NULL DEFAULT 'low',
    ai_assessment TEXT NOT NULL DEFAULT '',
    note TEXT NOT NULL DEFAULT '',
    next_action TEXT NOT NULL DEFAULT '',
    evidence_refs_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE INDEX IF NOT EXISTS idx_investigation_validations_scan ON investigation_validations(scan_id,updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_investigation_validations_opportunity ON investigation_validations(opportunity_id,updated_at DESC);

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
    fn migrates_legacy_browser_sessions_before_creating_task_scope_index() {
        let root =
            std::env::temp_dir().join(format!("oviraptor-legacy-auth-session-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("oviraptor.sqlite3");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE browser_auth_sessions (
                    id TEXT PRIMARY KEY,
                    project_id INTEGER NOT NULL,
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
                    created_at TEXT NOT NULL DEFAULT '',
                    updated_at TEXT NOT NULL DEFAULT ''
                );
                "#,
            )
            .unwrap();
        drop(connection);

        let migrated = initialize(&root).unwrap();
        let connection = open(&migrated).unwrap();
        for column in ["capture_previous_status", "owner_scan_id", "draft_scope_id"] {
            assert!(column_exists(&connection, "browser_auth_sessions", column).unwrap());
        }
        let index: String = connection
            .query_row(
                "SELECT name FROM sqlite_master WHERE type='index' AND name='idx_browser_auth_sessions_task_scope'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(index, "idx_browser_auth_sessions_task_scope");
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

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
    fn migration_downgrades_inferred_opportunities_from_agent_queue() {
        let root = std::env::temp_dir().join(format!(
            "oviraptor-opportunity-readiness-{}",
            Uuid::new_v4()
        ));
        let path = initialize(&root).unwrap();
        let connection = open(&path).unwrap();
        connection
            .execute("INSERT INTO projects(name) VALUES('Readiness')", [])
            .unwrap();
        let project_id = connection.last_insert_rowid();
        connection.execute("INSERT INTO sentinel_scans(id,project_id,project_name) VALUES('readiness-scan',?1,'Readiness')", [project_id]).unwrap();
        connection.execute("INSERT INTO sentinel_opportunities(project_id,scan_id,target_url,opportunity_key,category,title,score,status,confidence,source,record_json) VALUES(?1,'readiness-scan','https://example.test','inferred-login','identity_surface','inferred',88,'ready','high','evidence-reconstruction','{\"score\":88,\"method\":\"UNKNOWN\"}')", [project_id]).unwrap();
        connection.execute("INSERT INTO investigation_hypotheses(project_id,scan_id,target_url,hypothesis_key,category,title,status,score,confidence,decision_json,source_opportunity_key) VALUES(?1,'readiness-scan','https://example.test','hypothesis','identity_surface','inferred','ready',88,'high','{\"eligibleForModel\":true}','inferred-login')", [project_id]).unwrap();
        connection.execute("INSERT INTO sentinel_opportunities(project_id,scan_id,target_url,opportunity_key,category,title,score,status,confidence,source,record_json) VALUES(?1,'readiness-scan','https://example.test','ordinary-session','identity_surface','session restore',86,'ready','high','runtime-request','{\"score\":86,\"method\":\"GET\",\"endpoint\":\"/account/restore_login\",\"readiness\":{\"stage\":\"agent_ready\"}}')", [project_id]).unwrap();
        connection.execute("INSERT INTO sentinel_opportunities(project_id,scan_id,target_url,opportunity_key,category,title,score,status,confidence,source,record_json) VALUES(?1,'readiness-scan','https://example.test','transport-device','identity_surface','session restore with device id',100,'ready','high','runtime-request','{\"score\":100,\"method\":\"GET\",\"endpoint\":\"/account/restore_login\",\"riskEvidence\":{\"present\":true,\"signalCount\":1,\"signals\":[{\"type\":\"object_boundary_parameter\",\"fields\":[\"device_id\"]}]}}')", [project_id]).unwrap();
        connection
            .execute(
                "DELETE FROM app_settings WHERE key='opportunity_readiness_gate_version'",
                [],
            )
            .unwrap();
        drop(connection);

        initialize(&root).unwrap();
        let connection = open(&path).unwrap();
        let opportunity_status: String = connection
            .query_row(
                "SELECT status FROM sentinel_opportunities WHERE opportunity_key='inferred-login'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let (hypothesis_status, eligible): (String, i64) = connection
            .query_row(
                "SELECT status,COALESCE(json_extract(decision_json,'$.eligibleForModel'),1) FROM investigation_hypotheses WHERE hypothesis_key='hypothesis'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let (ordinary_status, ordinary_disposition): (String, String) = connection
            .query_row(
                "SELECT status,json_extract(record_json,'$.disposition') FROM sentinel_opportunities WHERE opportunity_key='ordinary-session'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let (transport_status, transport_disposition): (String, String) = connection
            .query_row(
                "SELECT status,json_extract(record_json,'$.disposition') FROM sentinel_opportunities WHERE opportunity_key='transport-device'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(opportunity_status, "dismissed");
        assert_eq!(hypothesis_status, "rejected");
        assert_eq!(eligible, 0);
        assert_eq!(ordinary_status, "dismissed");
        assert_eq!(ordinary_disposition, "api_inventory_only");
        assert_eq!(transport_status, "dismissed");
        assert_eq!(transport_disposition, "transport_identifier_only");
        assert_eq!(
            migration_version(&connection, "opportunity_readiness_gate_version"),
            7
        );
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
    fn relabels_recon_only_task_as_completed_when_queue_is_exhausted() {
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
                "INSERT INTO sentinel_scans(id,project_id,project_name,status,scan_type) VALUES('recon-only-scan',?1,'Recon only','recon_only','web')",
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
        assert_eq!(status, "completed");
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn restores_latest_attempt_scope_without_counting_historical_targets() {
        let root =
            std::env::temp_dir().join(format!("oviraptor-attempt-scope-test-{}", Uuid::new_v4()));
        let path = initialize(&root).unwrap();
        let connection = open(&path).unwrap();
        connection
            .execute("INSERT INTO projects(name) VALUES('Attempt scope')", [])
            .unwrap();
        let project_id = connection.last_insert_rowid();
        connection.execute(
            "INSERT INTO sentinel_scans(id,project_id,project_name,status,scan_type,attempt_count,current_checkpoint) VALUES('attempt-scope',?1,'Attempt scope','partial','web',2,'任务累计状态：待补充验证 1，确定性侦察收口 1')",
            [project_id],
        ).unwrap();
        connection.execute(
            "INSERT INTO sentinel_targets(project_id,scan_id,url,status,scan_mode,routing_reason) VALUES(?1,'attempt-scope','https://historical.invalid','recon_only','skip','历史确定性收口'),(?1,'attempt-scope','https://current.invalid','partial','standard','本轮没有形成任何工具证据')",
            [project_id],
        ).unwrap();
        let first = root.join("strix-jobs/attempt-scope/attempt-0001");
        let second = root.join("strix-jobs/attempt-scope/attempt-0002");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        fs::write(
            first.join("targets.json"),
            br#"[{"url":"https://historical.invalid"}]"#,
        )
        .unwrap();
        fs::write(
            second.join("targets.json"),
            br#"[{"url":"https://current.invalid"}]"#,
        )
        .unwrap();
        connection.execute(
            "INSERT INTO sentinel_scan_attempts(scan_id,attempt_number,status,stage,checkpoint,stop_reason,work_dir) VALUES('attempt-scope',1,'completed','complete','旧轮次','旧轮次',?1),('attempt-scope',2,'partial','complete','待补充验证 1，仅侦察收口 1','待补充验证 1，仅侦察收口 1',?2)",
            params![first.to_string_lossy(), second.to_string_lossy()],
        ).unwrap();
        connection.execute(
            "DELETE FROM app_settings WHERE key IN ('sentinel_target_attempt_version','sentinel_attempt_scope_summary_version')",
            [],
        ).unwrap();
        drop(connection);

        initialize(&root).unwrap();
        let connection = open(&path).unwrap();
        let attempts: Vec<(String, i64)> = connection.prepare(
            "SELECT url,last_attempt_number FROM sentinel_targets WHERE scan_id='attempt-scope' ORDER BY url",
        ).unwrap().query_map([], |row| Ok((row.get(0)?, row.get(1)?))).unwrap()
            .collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(
            attempts,
            vec![
                ("https://current.invalid".into(), 2),
                ("https://historical.invalid".into(), 1),
            ]
        );
        let summary: String = connection.query_row(
            "SELECT stop_reason FROM sentinel_scan_attempts WHERE scan_id='attempt-scope' AND attempt_number=2",
            [],
            |row| row.get(0),
        ).unwrap();
        assert!(summary.contains("待补充验证 1"));
        assert!(summary.contains("确定性侦察收口 0"));
        assert!(!summary.contains("确定性侦察收口 1"));
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}
