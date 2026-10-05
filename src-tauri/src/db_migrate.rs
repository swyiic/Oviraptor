// Connection helpers and the repeatable one-time migrations, kept apart from the
// bootstrap function. Included from db.rs, so they stay private to that module (§12).

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

/// Write a file that must only be readable by the current user: request and
/// response artifacts, vault material and runtime snapshots all use this.
pub fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(path, bytes).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}
