fn frontend_recon_signature_key(scan_id: &str, root: &Path, path: &Path) -> String {
    let source_identity = path.strip_prefix(root).unwrap_or(path).to_string_lossy();
    let source_hash = format!("{:x}", Sha256::digest(source_identity.as_bytes()));
    format!("frontend-recon-signature:{scan_id}:{}", &source_hash[..24])
}

fn sync_pending_frontend_recon(
    connection: &rusqlite::Connection,
    state: &AppState,
) -> Result<i64, String> {
    let root = state.app_data_dir.join("strix-jobs");
    if !root.is_dir() {
        return Ok(0);
    }
    let mut synced = 0;
    let mut recon_files = Vec::new();
    fn collect_recon(path: &Path, depth: usize, found: &mut Vec<PathBuf>) -> Result<(), String> {
        if !path.is_dir() || depth == 0 {
            return Ok(());
        }
        for name in ["oviraptor_recon.json", "asset_atlas_recon.json"] {
            let candidate = path.join(name);
            if candidate.is_file() {
                found.push(candidate);
                break;
            }
        }
        for entry in fs::read_dir(path).map_err(|error| error.to_string())? {
            let child = entry.map_err(|error| error.to_string())?.path();
            if child.is_dir() {
                collect_recon(&child, depth - 1, found)?;
            }
        }
        Ok(())
    }
    collect_recon(&root, 5, &mut recon_files)?;
    // A scan can contain several immutable attempt directories. Filesystem
    // iteration order is undefined; without sorting, an older attempt may be
    // ingested after the newest one and overwrite its complete A/B CDP matrix.
    // Attempt directories are zero padded, so path order is also chronological
    // within each scan and the newest evidence deterministically wins.
    recon_files.sort();
    for path in recon_files {
        let Some(dir) = path.parent() else { continue };
        let Some(scan_id) = oviraptor_scan_id_for_run(dir) else {
            continue;
        };
        let deleted = connection
            .query_row(
                "SELECT COUNT(*) FROM sentinel_deleted_scans WHERE scan_id=?1",
                [&scan_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0);
        if deleted > 0 {
            continue;
        }
        let Ok(bytes) = fs::read(&path) else { continue };
        let signature = format!("{:x}", Sha256::digest(&bytes));
        // Track each immutable recon source independently. The previous
        // scan-level key alternated between attempt signatures on every app
        // startup, causing all historical attempts to be re-imported in an
        // arbitrary order.
        let signature_key = frontend_recon_signature_key(&scan_id, &root, &path);
        let previous = connection
            .query_row(
                "SELECT value FROM app_settings WHERE key=?1",
                [&signature_key],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if previous.as_deref() == Some(signature.as_str()) {
            continue;
        }
        let Ok(recon) = serde_json::from_slice::<JsonValue>(&bytes) else {
            continue;
        };
        insert_frontend_recon(connection, &scan_id, &recon)?;
        connection.execute(
            "INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![signature_key, signature],
        ).map_err(|error| error.to_string())?;
        synced += 1;
    }
    Ok(synced)
}

fn sentinel_result_signature(dir: &Path) -> Result<String, String> {
    fn visit(
        path: &Path,
        root: &Path,
        depth: usize,
        hasher: &mut DefaultHasher,
    ) -> Result<(), String> {
        if depth == 0 || !path.is_dir() {
            return Ok(());
        }
        let mut entries = fs::read_dir(path)
            .map_err(|error| error.to_string())?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect::<Vec<_>>();
        entries.sort();
        for child in entries {
            if child.is_dir() {
                visit(&child, root, depth - 1, hasher)?;
                continue;
            }
            if !matches!(
                child.extension().and_then(|value| value.to_str()),
                Some("json" | "jsonl")
            ) || child.file_name().and_then(|value| value.to_str()) == Some("meta.json")
            {
                continue;
            }
            let metadata = fs::metadata(&child).map_err(|error| error.to_string())?;
            child.strip_prefix(root).unwrap_or(&child).hash(hasher);
            metadata.len().hash(hasher);
            metadata
                .modified()
                .ok()
                .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|value| value.as_nanos())
                .unwrap_or_default()
                .hash(hasher);
        }
        Ok(())
    }
    let mut hasher = DefaultHasher::new();
    visit(dir, dir, 8, &mut hasher)?;
    Ok(format!("{:016x}", hasher.finish()))
}

#[tauri::command]
pub async fn sync_sentinel_results(state: State<'_, AppState>) -> Result<i64, String> {
    let root = state
        .app_data_dir
        .parent()
        .unwrap_or(&state.app_data_dir)
        .join(".trae-cn/scan-results");
    let connection = db::open(&state.db_path)?;
    repair_web_target_pollution(&connection)?;
    let mut count = 0;
    if root.exists() {
        for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
            let dir = entry.map_err(|e| e.to_string())?.path();
            if !dir.is_dir() {
                continue;
            }
            let meta_path = dir.join("meta.json");
            if !meta_path.exists() {
                continue;
            }
            let meta: JsonValue =
                serde_json::from_slice(&fs::read(&meta_path).map_err(|e| e.to_string())?)
                    .unwrap_or_default();
            let scan_id = meta
                .get("scanId")
                .and_then(JsonValue::as_str)
                .unwrap_or_else(|| dir.file_name().and_then(|v| v.to_str()).unwrap_or(""));
            if scan_id.is_empty() {
                continue;
            }
            let deleted: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM sentinel_deleted_scans WHERE scan_id=?1",
                    [scan_id],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            if deleted > 0 {
                // 墓碑优先：即使旧版本进程曾把目录重新同步回来，也立即清掉残留行。
                let _ = connection.execute("DELETE FROM sentinel_scans WHERE id=?1", [scan_id]);
                continue;
            }
            let summary: JsonValue = fs::read(dir.join("summary.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_default();
            let existing: Option<(Option<i64>, String, String)> = connection
                .query_row(
                    "SELECT project_id,project_name,status FROM sentinel_scans WHERE id=?1",
                    [scan_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()
                .map_err(|error| error.to_string())?;
            // Agent 可能一直不回写 meta.status。summary.completedAt 表示结果已封盘，
            // 其完成态优先于 meta.json 中滞留的 scanning。
            let completed = meta
                .get("completedAt")
                .and_then(JsonValue::as_str)
                .is_some_and(|v| !v.is_empty())
                || summary
                    .get("completedAt")
                    .and_then(JsonValue::as_str)
                    .is_some_and(|v| !v.is_empty())
                || summary
                    .get("status")
                    .and_then(JsonValue::as_str)
                    .is_some_and(|v| matches!(v, "completed" | "complete" | "finished"));
            let status = if completed {
                "completed"
            } else {
                meta.get("status")
                    .and_then(JsonValue::as_str)
                    .unwrap_or("scanning")
            };
            let status_changed = existing
                .as_ref()
                .is_none_or(|(_, _, previous_status)| previous_status != status);
            // Completed result directories are immutable. Re-parsing every
            // checkpoint on each page mount deleted and reinserted thousands of
            // rows even when nothing had changed.
            if completed
                && existing
                    .as_ref()
                    .is_some_and(|(_, _, old_status)| old_status == "completed")
            {
                continue;
            }
            // projectName 是 Oviraptor 项目归属；yakitProject 只是扫描器工作区名。
            let mut project_name = meta
                .get("projectName")
                .and_then(JsonValue::as_str)
                .filter(|v| !v.trim().is_empty())
                .or_else(|| {
                    existing
                        .as_ref()
                        .map(|item| item.1.as_str())
                        .filter(|v| !v.trim().is_empty())
                })
                .or_else(|| meta.get("yakitProject").and_then(JsonValue::as_str))
                .unwrap_or("")
                .to_string();
            let mut project_id = existing.as_ref().and_then(|item| item.0);
            if project_id.is_none() && !project_name.is_empty() {
                project_id = connection
                    .query_row(
                        "SELECT id FROM projects WHERE lower(name)=lower(?1) LIMIT 1",
                        [&project_name],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(|error| error.to_string())?;
            }
            if project_id.is_none() {
                if let Some(task_id) = meta.get("taskId").and_then(JsonValue::as_str) {
                    project_id = connection
                        .query_row(
                            "SELECT project_id FROM sentinel_scans WHERE id=?1",
                            [task_id],
                            |row| row.get(0),
                        )
                        .optional()
                        .map_err(|error| error.to_string())?;
                }
            }
            // Agent 既未写公司也未写 Atlas 项目时，用结果 URL 反查本地资产归属。
            if project_id.is_none() {
                let target_arrays = [
                    meta.get("targets"),
                    summary.get("perTarget"),
                    summary.get("targets"),
                ];
                'outer: for array in target_arrays.into_iter().flatten() {
                    if let Some(items) = array.as_array() {
                        for item in items {
                            let url = item
                                .as_str()
                                .map(str::to_string)
                                .unwrap_or_else(|| value_first(item, &["url", "target", "host"]));
                            let key = url.trim().trim_end_matches('/').to_ascii_lowercase();
                            if key.is_empty() {
                                continue;
                            }
                            let matched: Option<(i64,String)>=connection.query_row("SELECT pa.project_id,p.name FROM project_assets pa JOIN assets a ON a.id=pa.asset_id JOIN projects p ON p.id=pa.project_id WHERE pa.is_deleted=0 AND (lower(rtrim(a.link,'/'))=?1 OR lower(rtrim(a.host,'/'))=?1) ORDER BY pa.last_seen DESC LIMIT 1",[key],|row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(|error|error.to_string())?;
                            if let Some((id, name)) = matched {
                                project_id = Some(id);
                                project_name = name;
                                break 'outer;
                            }
                        }
                    }
                }
            }
            let checkpoint = meta
                .get("currentCheckpoint")
                .and_then(JsonValue::as_str)
                .unwrap_or("");
            connection.execute("INSERT INTO sentinel_scans(id,project_id,project_name,status,current_checkpoint,task_path) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(id) DO UPDATE SET project_id=COALESCE(excluded.project_id,sentinel_scans.project_id),project_name=CASE WHEN excluded.project_name='' THEN sentinel_scans.project_name ELSE excluded.project_name END,status=excluded.status,current_checkpoint=excluded.current_checkpoint,task_path=excluded.task_path,updated_at=datetime('now','localtime')", params![scan_id,project_id,project_name,status,checkpoint,dir.to_string_lossy()]).map_err(|e| e.to_string())?;
            let artifact_signature = sentinel_result_signature(&dir)?;
            let signature_key = format!("sentinel-result-signature:{scan_id}");
            let previous_signature = connection
                .query_row(
                    "SELECT value FROM app_settings WHERE key=?1",
                    [&signature_key],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|error| error.to_string())?;
            if previous_signature.as_deref() == Some(artifact_signature.as_str()) {
                // meta/status is still refreshed above, but immutable findings
                // are not deleted and reconstructed on every polling tick.
                if status_changed {
                    count += 1;
                }
                continue;
            }
            connection
                .execute(
                    "DELETE FROM sentinel_findings WHERE scan_id=?1 AND stage NOT IN ('local-inventory','local-sast')",
                    [scan_id],
                )
                .map_err(|e| e.to_string())?;
            connection
                .execute(
                    "DELETE FROM sentinel_checkpoints WHERE scan_id=?1",
                    [scan_id],
                )
                .map_err(|e| e.to_string())?;
            let mut checkpoint_files = Vec::new();
            collect_checkpoint_files(&dir, &mut checkpoint_files)?;
            // 根目录 S1-S5 是聚合摘要，URL 子目录文件通常更完整。先解析聚合，
            // 再用同 URL、同阶段的详细文件整体替换，避免重复漏洞和重复端点。
            checkpoint_files.sort_by_key(|(_, value)| {
                value
                    .get("url")
                    .and_then(JsonValue::as_str)
                    .filter(|url| !url.trim().is_empty())
                    .map(|_| 1)
                    .unwrap_or(0)
            });
            for (stage, value) in checkpoint_files {
                let url = value_first(&value, &["url", "target", "host"]);
                let url = if url.is_empty() { "*".to_string() } else { url };
                if url != "*" && matches!(stage.as_str(), "s1" | "s2" | "s3" | "s4" | "s5") {
                    connection
                    .execute(
                        "DELETE FROM sentinel_findings WHERE scan_id=?1 AND target_url=?2 AND stage=?3",
                        params![scan_id, url, stage],
                    )
                    .map_err(|error| error.to_string())?;
                }
                connection.execute("INSERT INTO sentinel_checkpoints(scan_id,url,stage,raw_json) VALUES(?1,?2,?3,?4) ON CONFLICT(scan_id,url,stage) DO UPDATE SET raw_json=excluded.raw_json,updated_at=datetime('now','localtime')", params![scan_id,url,stage,value.to_string()]).map_err(|e| e.to_string())?;
                parse_checkpoint_findings(&connection, scan_id, &stage, &value)?;
            }
            if let Some(project_id) = project_id {
                let mut company_by_url: HashMap<String, String> = HashMap::new();
                let mut asset_statement = connection.prepare("SELECT a.company,a.link,a.host,a.domain,a.ip,a.port FROM project_assets pa JOIN assets a ON a.id=pa.asset_id WHERE pa.project_id=?1 AND pa.is_deleted=0").map_err(|error| error.to_string())?;
                let asset_rows = asset_statement
                    .query_map([project_id], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, String>(5)?,
                        ))
                    })
                    .map_err(|error| error.to_string())?;
                for row in asset_rows {
                    let (company, link, host, domain, ip, port) =
                        row.map_err(|error| error.to_string())?;
                    for raw in [
                        link,
                        host,
                        domain,
                        if ip.is_empty() {
                            String::new()
                        } else if port.is_empty() {
                            ip
                        } else {
                            format!("{}:{}", ip, port)
                        },
                    ] {
                        let key = raw.trim().trim_end_matches('/').to_ascii_lowercase();
                        if !key.is_empty() {
                            company_by_url
                                .entry(key.clone())
                                .or_insert_with(|| company.clone());
                            company_by_url
                                .entry(
                                    key.trim_start_matches("https://")
                                        .trim_start_matches("http://")
                                        .to_string(),
                                )
                                .or_insert_with(|| company.clone());
                        }
                    }
                }
                let mut target_map: HashMap<String, (String, String)> = HashMap::new();
                let mut add_targets = |value: &JsonValue| {
                    if let Some(items) = value.as_array() {
                        for item in items {
                            let (url, company, target_status) = if let Some(url) = item.as_str() {
                                (url.to_string(), String::new(), String::new())
                            } else {
                                (
                                    value_first(item, &["url", "target", "host"]),
                                    value_first(item, &["company", "organization", "org"]),
                                    value_first(item, &["status", "state"]),
                                )
                            };
                            if !is_web_target_url(&url) {
                                continue;
                            }
                            let key = url.trim().trim_end_matches('/').to_ascii_lowercase();
                            let current = target_map
                                .entry(key)
                                .or_insert((String::new(), String::new()));
                            if !company.is_empty() {
                                current.0 = company
                            }
                            if !target_status.is_empty() {
                                current.1 = target_status
                            }
                        }
                    }
                };
                add_targets(meta.get("targets").unwrap_or(&JsonValue::Null));
                add_targets(summary.get("perTarget").unwrap_or(&JsonValue::Null));
                add_targets(summary.get("targets").unwrap_or(&JsonValue::Null));
                let mut finding_statement=connection.prepare("SELECT DISTINCT target_url FROM sentinel_findings WHERE scan_id=?1 AND target_url<>'' AND target_url<>'*'").map_err(|error| error.to_string())?;
                let finding_urls = finding_statement
                    .query_map([scan_id], |row| row.get::<_, String>(0))
                    .map_err(|error| error.to_string())?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| error.to_string())?;
                for url in finding_urls {
                    if !is_web_target_url(&url) {
                        continue;
                    }
                    target_map
                        .entry(url.trim().trim_end_matches('/').to_ascii_lowercase())
                        .or_insert((String::new(), String::new()));
                }
                for (url, (mut company, target_status)) in target_map {
                    if company.trim().is_empty() {
                        let without_scheme = url
                            .trim_start_matches("https://")
                            .trim_start_matches("http://");
                        company = company_by_url
                            .get(&url)
                            .or_else(|| company_by_url.get(without_scheme))
                            .cloned()
                            .unwrap_or_default();
                    }
                    connection.execute("INSERT INTO sentinel_targets(project_id,scan_id,company,url,status) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(project_id,scan_id,url) DO UPDATE SET company=CASE WHEN excluded.company='' THEN sentinel_targets.company ELSE excluded.company END,status=CASE WHEN excluded.status='' THEN sentinel_targets.status ELSE excluded.status END,updated_at=datetime('now','localtime')",params![project_id,scan_id,company,url,target_status]).map_err(|error| error.to_string())?;
                }
            }
            connection.execute(
                "INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![signature_key, artifact_signature],
            ).map_err(|error| error.to_string())?;
            count += 1;
        }
    }
    count += sync_pending_frontend_recon(&connection, &state)?;
    count += sync_strix_results(&connection, &state)?;
    Ok(count)
}
