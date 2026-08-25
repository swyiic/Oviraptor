#[tauri::command]
pub fn create_sentinel_scan(
    state: State<AppState>,
    project_id: i64,
    asset_ids: Vec<i64>,
    scan_mode: Option<String>,
) -> Result<SentinelScan, String> {
    let connection = db::open(&state.db_path)?;
    let project_name: String = connection
        .query_row(
            "SELECT name FROM projects WHERE id=?1 AND status='active'",
            [project_id],
            |r| r.get(0),
        )
        .map_err(|_| "项目不存在或已归档；恢复工作空间后才能创建新任务".to_string())?;
    let ids_json = serde_json::to_string(&asset_ids).map_err(|e| e.to_string())?;
    let mut statement = connection.prepare("SELECT a.id,a.company,COALESCE(NULLIF(a.link,''),a.host) FROM assets a JOIN project_assets pa ON pa.asset_id=a.id WHERE pa.project_id=?1 AND pa.is_deleted=0 AND a.id IN (SELECT value FROM json_each(?2)) AND NOT EXISTS (SELECT 1 FROM sentinel_fuse_zone f WHERE f.project_id=pa.project_id AND f.normalized_url=lower(rtrim(trim(COALESCE(NULLIF(a.link,''),a.host)),'/')))").map_err(|e| e.to_string())?;
    let targets = statement
        .query_map(params![project_id, ids_json], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(statement);
    if targets.is_empty() {
        return Err("没有可发送的资产；所选 URL 可能都在 Strix 熔断区".into());
    }
    let excluded_count = asset_ids.len().saturating_sub(targets.len());
    let checkpoint = if excluded_count > 0 {
        format!("待确认；已自动排除熔断区中的 {excluded_count} 个 URL")
    } else {
        "待确认".to_string()
    };
    let scan_id = Uuid::new_v4().to_string();
    connection.execute("INSERT INTO sentinel_scans(id,project_id,project_name,status,current_checkpoint,task_path,scan_type,task_name) VALUES(?1,?2,?3,'draft',?4,'','web',?3)", params![scan_id,project_id,project_name,checkpoint]).map_err(|e| e.to_string())?;
    let scan_mode = normalized_web_scan_mode(scan_mode.as_deref());
    let policy = build_web_investigation_policy(
        Some(scan_mode),
        None,
        Vec::new(),
        &[],
        "",
        "asset-workspace",
    )?;
    connection.execute(
        "INSERT INTO sentinel_scan_contexts(scan_id,environment,policy_json) VALUES(?1,'internal',?2) ON CONFLICT(scan_id) DO UPDATE SET environment='internal',policy_json=excluded.policy_json,updated_at=datetime('now','localtime')",
        params![scan_id, policy.to_string()],
    ).map_err(|error| error.to_string())?;
    for (asset_id, company, url) in targets {
        connection
            .execute(
                "INSERT OR IGNORE INTO sentinel_targets(project_id,scan_id,asset_id,company,url) VALUES(?1,?2,?3,?4,?5)",
                params![project_id, scan_id, asset_id, company, url],
            )
            .map_err(|e| e.to_string())?;
    }
    sentinel_scan_by_id(&connection, &scan_id)
}

#[tauri::command]
pub fn create_sentinel_url_scan(
    state: State<AppState>,
    project_id: i64,
    task_name: String,
    urls: Vec<String>,
    scan_mode: Option<String>,
    max_budget_usd: Option<f64>,
    auth_session_id: Option<String>,
    auth_session_ids: Option<Vec<String>>,
    auth_session_scope_id: Option<String>,
    skill_ids: Option<Vec<i64>>,
    instruction: Option<String>,
) -> Result<SentinelScan, String> {
    let connection = db::open(&state.db_path)?;
    let project_name: String = connection
        .query_row(
            "SELECT name FROM projects WHERE id=?1 AND status='active'",
            [project_id],
            |r| r.get(0),
        )
        .map_err(|_| "项目不存在或已归档；恢复工作空间后才能创建新任务".to_string())?;
    let mut normalized = urls
        .into_iter()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    if normalized.is_empty() {
        return Err("至少需要一个 URL".into());
    }
    if normalized.len() > 200 {
        return Err("单个 Strix Web 任务最多 200 个 URL".into());
    }
    if normalized
        .iter()
        .any(|url| !(url.starts_with("http://") || url.starts_with("https://")))
    {
        return Err("URL 必须以 http:// 或 https:// 开头".into());
    }
    let scan_mode = normalized_web_scan_mode(scan_mode.as_deref()).to_string();
    if max_budget_usd.is_some_and(|value| value <= 0.0 || value > 10_000.0) {
        return Err("单任务费用上限必须大于 0 且不超过 10000 USD".into());
    }
    let auth_session_id = auth_session_id.unwrap_or_default().trim().to_string();
    let mut auth_session_ids = auth_session_ids
        .unwrap_or_default()
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if !auth_session_id.is_empty() {
        auth_session_ids.push(auth_session_id.clone());
    }
    auth_session_ids.sort();
    auth_session_ids.dedup();
    if auth_session_ids.len() > 5 {
        return Err("单个任务最多比较 5 个登录身份".into());
    }
    let auth_session_scope_id = auth_session_scope_id.unwrap_or_default();
    crate::auth_session::validate_draft_sessions_for_task(
        &connection,
        &auth_session_ids,
        project_id,
        &auth_session_scope_id,
    )?;
    crate::auth_session::distinct_session_documents_for_scan(
        &connection,
        &auth_session_ids,
        project_id,
    )?;
    let targets = normalized
        .into_iter()
        .filter(|url| {
            !connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sentinel_fuse_zone WHERE project_id=?1 AND normalized_url=lower(rtrim(trim(?2),'/')))",
                    params![project_id, url],
                    |row| row.get::<_, bool>(0),
                )
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    if targets.is_empty() {
        return Err("输入 URL 全部位于 Strix 熔断区".into());
    }
    let scan_id = Uuid::new_v4().to_string();
    let title = if task_name.trim().is_empty() {
        format!("{} · Web", project_name)
    } else {
        task_name.trim().chars().take(120).collect()
    };
    connection
        .execute(
            "INSERT INTO sentinel_scans(id,project_id,project_name,status,current_checkpoint,task_path,scan_type,task_name) VALUES(?1,?2,?3,'draft','待确认','','web',?4)",
            params![scan_id, project_id, project_name, title],
        )
        .map_err(|error| error.to_string())?;
    let policy = build_web_investigation_policy(
        Some(&scan_mode),
        max_budget_usd,
        auth_session_ids.clone(),
        &skill_ids.unwrap_or_default(),
        instruction.as_deref().unwrap_or(""),
        "strix-workbench",
    )?;
    connection.execute(
        "INSERT INTO sentinel_scan_contexts(scan_id,environment,policy_json) VALUES(?1,'internal',?2) ON CONFLICT(scan_id) DO UPDATE SET policy_json=excluded.policy_json,updated_at=datetime('now','localtime')",
        params![scan_id, policy.to_string()],
    ).map_err(|error| error.to_string())?;
    crate::auth_session::bind_draft_sessions_to_scan(
        &connection,
        &auth_session_ids,
        project_id,
        &auth_session_scope_id,
        &scan_id,
    )?;
    for url in targets {
        connection
            .execute(
                "INSERT INTO sentinel_targets(project_id,scan_id,company,url,status) VALUES(?1,?2,?3,?4,'queued')",
                params![project_id, scan_id, project_name, url],
            )
            .map_err(|error| error.to_string())?;
    }
    sentinel_scan_by_id(&connection, &scan_id)
}

const SENTINEL_SCAN_COLUMNS: &str = "id,project_id,project_name,status,current_checkpoint,task_path,previous_scan_id,llm_requests,input_tokens,output_tokens,cached_tokens,total_tokens,scan_type,task_name,source_path,skill_names,attempt_count,created_at,updated_at,CASE WHEN scan_type='web' THEN COALESCE((SELECT json_extract(policy_json,'$.webModeCeiling') FROM sentinel_scan_contexts WHERE scan_id=sentinel_scans.id),'standard') ELSE '' END,COALESCE((SELECT attempt_number FROM sentinel_scan_attempts WHERE scan_id=sentinel_scans.id ORDER BY attempt_number DESC LIMIT 1),0),COALESCE((SELECT status FROM sentinel_scan_attempts WHERE scan_id=sentinel_scans.id ORDER BY attempt_number DESC LIMIT 1),''),COALESCE((SELECT checkpoint FROM sentinel_scan_attempts WHERE scan_id=sentinel_scans.id ORDER BY attempt_number DESC LIMIT 1),''),COALESCE((SELECT stop_reason FROM sentinel_scan_attempts WHERE scan_id=sentinel_scans.id ORDER BY attempt_number DESC LIMIT 1),'')";

fn scan_llm_policy(task_path: &str) -> (String, String, bool) {
    let Some(value) = fs::read(task_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<JsonValue>(&bytes).ok())
    else {
        return (String::new(), "unknown".into(), false);
    };
    let policy = value.get("llmPolicy").unwrap_or(&JsonValue::Null);
    (
        policy
            .get("model")
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .to_string(),
        match policy.get("deployment").and_then(JsonValue::as_str) {
            Some("local") => "local".into(),
            Some("cloud") => "cloud".into(),
            _ => "unknown".into(),
        },
        policy
            .get("fullPower")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false),
    )
}

fn sentinel_scan_row(row: &Row<'_>) -> rusqlite::Result<SentinelScan> {
    let task_path: String = row.get(5)?;
    let (llm_model, llm_deployment, llm_full_power) = scan_llm_policy(&task_path);
    Ok(SentinelScan {
        id: row.get(0)?,
        project_id: row.get(1)?,
        project_name: row.get(2)?,
        status: row.get(3)?,
        current_checkpoint: row.get(4)?,
        task_path,
        previous_scan_id: row.get(6)?,
        llm_requests: row.get(7)?,
        input_tokens: row.get(8)?,
        output_tokens: row.get(9)?,
        cached_tokens: row.get(10)?,
        total_tokens: row.get(11)?,
        scan_type: row.get(12)?,
        task_name: row.get(13)?,
        source_path: row.get(14)?,
        skill_names: row.get(15)?,
        attempt_count: row.get(16)?,
        created_at: row.get(17)?,
        updated_at: row.get(18)?,
        requested_scan_mode: row.get(19)?,
        llm_model,
        llm_deployment,
        llm_full_power,
        latest_attempt_number: row.get(20)?,
        latest_attempt_status: row.get(21)?,
        latest_attempt_checkpoint: row.get(22)?,
        latest_attempt_stop_reason: row.get(23)?,
    })
}

fn sentinel_scan_by_id(
    connection: &rusqlite::Connection,
    scan_id: &str,
) -> Result<SentinelScan, String> {
    connection
        .query_row(
            &format!("SELECT {SENTINEL_SCAN_COLUMNS} FROM sentinel_scans WHERE id=?1"),
            [scan_id],
            sentinel_scan_row,
        )
        .map_err(|error| error.to_string())
}

fn sentinel_attempt_stage(status: &str, checkpoint: &str) -> String {
    let text = checkpoint.to_lowercase();
    if matches!(status, "completed" | "partial" | "recon_only") {
        return "complete".into();
    }
    if matches!(status, "failed" | "cancelled") {
        return "stopped".into();
    }
    if matches!(status, "paused" | "pausing") {
        return "paused".into();
    }
    if text.contains("漏洞")
        || text.contains("finding")
        || text.contains("结果")
        || text.contains("同步")
    {
        return "evidence".into();
    }
    if text.contains("strix")
        || text.contains("agent")
        || text.contains("模型")
        || text.contains("验证")
        || text.contains("poc")
    {
        return "validation".into();
    }
    if text.contains("前端")
        || text.contains("浏览器")
        || text.contains("javascript")
        || text.contains(" js")
        || text.contains("探测")
        || text.contains("接口")
    {
        return "frontend_recon".into();
    }
    if text.contains("队列") || text.contains("准备") || text.contains("docker") {
        return "preparing".into();
    }
    "running".into()
}

fn record_sentinel_attempt_start(
    connection: &rusqlite::Connection,
    scan_id: &str,
    attempt_number: i64,
    work_dir: &Path,
) -> Result<(), String> {
    let mode_key = format!("sentinel-next-attempt-mode:{scan_id}");
    let execution_mode = connection
        .query_row(
            "SELECT value FROM app_settings WHERE key=?1",
            [&mode_key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .filter(|value| matches!(value.as_str(), "fresh" | "resume"))
        .unwrap_or_else(|| "initial".into());
    let (status, checkpoint, requests, input, output, cached, total): (
        String,
        String,
        i64,
        i64,
        i64,
        i64,
        i64,
    ) = connection
        .query_row(
            "SELECT status,current_checkpoint,llm_requests,input_tokens,output_tokens,cached_tokens,total_tokens FROM sentinel_scans WHERE id=?1",
            [scan_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?)),
        )
        .map_err(|error| error.to_string())?;
    let stage = sentinel_attempt_stage(&status, &checkpoint);
    connection
        .execute(
            "INSERT INTO sentinel_scan_attempts(scan_id,attempt_number,execution_mode,status,stage,checkpoint,work_dir,llm_requests_start,input_tokens_start,output_tokens_start,cached_tokens_start,total_tokens_start) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12) ON CONFLICT(scan_id,attempt_number) DO UPDATE SET execution_mode=excluded.execution_mode,status=excluded.status,stage=excluded.stage,checkpoint=excluded.checkpoint,work_dir=excluded.work_dir,llm_requests_start=excluded.llm_requests_start,input_tokens_start=excluded.input_tokens_start,output_tokens_start=excluded.output_tokens_start,cached_tokens_start=excluded.cached_tokens_start,total_tokens_start=excluded.total_tokens_start,llm_requests_delta=0,input_tokens_delta=0,output_tokens_delta=0,cached_tokens_delta=0,total_tokens_delta=0,stop_reason='',finished_at='',started_at=datetime('now','localtime'),updated_at=datetime('now','localtime')",
            params![scan_id, attempt_number, execution_mode, status, stage, checkpoint, work_dir.to_string_lossy(), requests, input, output, cached, total],
        )
        .map_err(|error| error.to_string())?;
    connection
        .execute("DELETE FROM app_settings WHERE key=?1", [mode_key])
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn sync_sentinel_attempt(connection: &rusqlite::Connection, scan_id: &str) {
    let current = connection.query_row(
        "SELECT status,current_checkpoint,attempt_count FROM sentinel_scans WHERE id=?1",
        [scan_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        },
    );
    let Ok((status, checkpoint, attempt_number)) = current else {
        return;
    };
    if attempt_number <= 0 {
        return;
    }
    let stage = sentinel_attempt_stage(&status, &checkpoint);
    let terminal = matches!(
        status.as_str(),
        "completed" | "partial" | "recon_only" | "failed" | "cancelled" | "paused"
    );
    let _ = connection.execute(
        "UPDATE sentinel_scan_attempts SET status=?1,stage=?2,checkpoint=CASE WHEN ?4=1 AND trim(stop_reason)<>'' THEN checkpoint ELSE ?3 END,stop_reason=CASE WHEN ?4=1 AND trim(stop_reason)='' THEN ?3 ELSE stop_reason END,llm_requests_delta=MAX(0,(SELECT llm_requests FROM sentinel_scans WHERE id=?5)-llm_requests_start),input_tokens_delta=MAX(0,(SELECT input_tokens FROM sentinel_scans WHERE id=?5)-input_tokens_start),output_tokens_delta=MAX(0,(SELECT output_tokens FROM sentinel_scans WHERE id=?5)-output_tokens_start),cached_tokens_delta=MAX(0,(SELECT cached_tokens FROM sentinel_scans WHERE id=?5)-cached_tokens_start),total_tokens_delta=MAX(0,(SELECT total_tokens FROM sentinel_scans WHERE id=?5)-total_tokens_start),finished_at=CASE WHEN ?4=1 AND finished_at='' THEN datetime('now','localtime') WHEN ?4=0 THEN '' ELSE finished_at END,updated_at=datetime('now','localtime') WHERE scan_id=?5 AND attempt_number=?6",
        params![status, stage, checkpoint, terminal as i64, scan_id, attempt_number],
    );
}

const SENTINEL_RESCAN_COUNT_SQL: &str = "SELECT COUNT(*) FROM sentinel_targets t WHERE scan_id=?1 AND (?2=0 OR status NOT IN ('completed','recon_only','manual_review')) AND NOT EXISTS (SELECT 1 FROM sentinel_fuse_zone f WHERE f.archived=0 AND f.project_id=t.project_id AND f.normalized_url=lower(rtrim(trim(t.url),'/')))";
#[cfg(test)]
const SENTINEL_RESCAN_COPY_SQL: &str = "INSERT INTO sentinel_targets(project_id,scan_id,asset_id,company,url,status) SELECT t.project_id,?1,t.asset_id,t.company,t.url,'queued' FROM sentinel_targets t WHERE t.scan_id=?2 AND (?3=0 OR t.status NOT IN ('completed','recon_only','manual_review')) AND NOT EXISTS (SELECT 1 FROM sentinel_fuse_zone f WHERE f.archived=0 AND f.project_id=t.project_id AND f.normalized_url=lower(rtrim(trim(t.url),'/')))";
#[cfg(test)]
const SENTINEL_RESCAN_RECON_COPY_SQL: &str = "INSERT INTO sentinel_checkpoints(scan_id,url,stage,raw_json,updated_at) SELECT ?1,c.url,c.stage,c.raw_json,datetime('now','localtime') FROM sentinel_checkpoints c JOIN sentinel_targets t ON t.scan_id=?1 AND t.url=c.url WHERE c.scan_id=?2 AND c.stage='frontend_recon' ON CONFLICT(scan_id,url,stage) DO UPDATE SET raw_json=excluded.raw_json,updated_at=excluded.updated_at";
const SENTINEL_RESUME_COUNT_SQL: &str = "SELECT COUNT(*) FROM sentinel_targets WHERE scan_id=?1 AND status NOT IN ('completed','partial','recon_only','manual_review','limited','failed','fuse_excluded')";
const SENTINEL_RESUME_TARGETS_SQL: &str = "SELECT company,url FROM sentinel_targets WHERE scan_id=?1 AND status NOT IN ('completed','partial','recon_only','manual_review','limited','failed','fuse_excluded') ORDER BY id";

fn prepare_web_scan_retry(
    connection: &mut rusqlite::Connection,
    scan_id: &str,
    status: &str,
) -> Result<i64, String> {
    match status {
        "scanning" | "pausing" => return Err("任务仍在运行，请先暂停后再继续".into()),
        "paused" => return Err("暂停任务请使用“继续扫描”，无需重新执行".into()),
        "draft" => return Err("任务仍待确认，不能重复准备".into()),
        _ => {}
    }
    let retry_incomplete_only = retry_only_incomplete_targets(status);
    // Archived fuse rows are immutable history. Only active protection-system
    // entries may exclude a target from a new attempt.
    let target_count: i64 = connection
        .query_row(
            SENTINEL_RESCAN_COUNT_SQL,
            params![scan_id, retry_incomplete_only as i64],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if target_count == 0 {
        return Err("当前任务没有可继续执行的 URL；目标可能都在 Strix 熔断区".into());
    }
    let fuse_excluded: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sentinel_targets t WHERE t.scan_id=?1 AND (?2=0 OR status NOT IN ('completed','recon_only','manual_review')) AND EXISTS (SELECT 1 FROM sentinel_fuse_zone f WHERE f.archived=0 AND f.project_id=t.project_id AND f.normalized_url=lower(rtrim(trim(t.url),'/')))",
        params![scan_id, retry_incomplete_only as i64],
        |row| row.get(0),
    ).unwrap_or(0);
    let next_attempt = connection
        .query_row(
            "SELECT attempt_count+1 FROM sentinel_scans WHERE id=?1",
            [scan_id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(1)
        .max(1);
    let execution_mode = if retry_incomplete_only { "resume" } else { "fresh" };
    let checkpoint = if !retry_incomplete_only && fuse_excluded > 0 {
        format!("已建立第 {next_attempt} 次全新执行计划：清空当前结果面后重新处理 {target_count} 个 URL，排除熔断区 {fuse_excluded} 个；旧结果仅保留在执行历史中")
    } else if !retry_incomplete_only {
        format!("已建立第 {next_attempt} 次全新执行计划：清空当前结果面后重新处理 {target_count} 个 URL；旧结果仅保留在执行历史中")
    } else if fuse_excluded > 0 {
        format!("已建立第 {next_attempt} 次续跑计划：仅处理 {target_count} 个未完成 URL，排除熔断区 {fuse_excluded} 个；保留可复用证据，旧状态仅保留在执行历史中")
    } else {
        format!("已建立第 {next_attempt} 次续跑计划：仅处理 {target_count} 个未完成 URL；保留可复用证据，旧状态仅保留在执行历史中")
    };
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "UPDATE sentinel_targets SET status='queued',value_score=0,scan_mode='',routing_reason=?3,updated_at=datetime('now','localtime') WHERE scan_id=?1 AND (?2=0 OR status NOT IN ('completed','recon_only','manual_review')) AND NOT EXISTS (SELECT 1 FROM sentinel_fuse_zone f WHERE f.archived=0 AND f.project_id=sentinel_targets.project_id AND f.normalized_url=lower(rtrim(trim(sentinel_targets.url),'/')))",
            params![scan_id, retry_incomplete_only as i64, format!("第 {next_attempt} 次执行待重新分流；上一轮结束原因见执行历史")],
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute("DELETE FROM sentinel_processes WHERE scan_id=?1", [scan_id])
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![format!("sentinel-next-attempt-mode:{scan_id}"), execution_mode],
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "UPDATE sentinel_scans SET status='draft',current_checkpoint=?1,previous_scan_id='',updated_at=datetime('now','localtime') WHERE id=?2",
            params![checkpoint, scan_id],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(target_count)
}

fn retry_only_incomplete_targets(status: &str) -> bool {
    matches!(status, "partial" | "failed" | "limited" | "cancelled")
}

fn next_scan_attempt_work_dir(scan_root: &Path, minimum_attempt: u32) -> Result<PathBuf, String> {
    fs::create_dir_all(scan_root).map_err(|error| error.to_string())?;
    let mut max_attempt = 0u32;
    if let Ok(entries) = fs::read_dir(scan_root) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let Some(number) = name.strip_prefix("attempt-") else {
                continue;
            };
            if entry.path().is_dir() {
                max_attempt = max_attempt.max(number.parse::<u32>().unwrap_or(0));
            }
        }
    }
    let has_legacy_attempt = [
        "url-pipeline",
        "batches",
        "oviraptor-runner.log",
        "llm-hook.jsonl",
        "targets.json",
        "targets.txt",
    ]
    .iter()
    .any(|name| scan_root.join(name).exists());
    let next_discovered_attempt = if max_attempt > 0 {
        max_attempt.saturating_add(1)
    } else if has_legacy_attempt {
        2
    } else {
        1
    };
    let attempt = next_discovered_attempt.max(minimum_attempt.max(1));
    let work_dir = scan_root.join(format!("attempt-{attempt:04}"));
    fs::create_dir_all(&work_dir).map_err(|error| error.to_string())?;
    Ok(work_dir)
}

fn scan_attempt_number(work_dir: &Path) -> u32 {
    work_dir
        .file_name()
        .and_then(|value| value.to_str())
        .and_then(|value| value.strip_prefix("attempt-"))
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(1)
}

#[tauri::command]
pub fn rescan_sentinel_scan(
    app: AppHandle,
    state: State<AppState>,
    scan_id: String,
) -> Result<SentinelScan, String> {
    let db_path = state.db_path.clone();
    let mut connection = db::open(&state.db_path)?;
    let (scan_type, status, checkpoint, previous_scan_id): (String, String, String, String) = connection
        .query_row(
            "SELECT s.scan_type,s.status,s.current_checkpoint,s.previous_scan_id FROM sentinel_scans s JOIN projects p ON p.id=s.project_id AND p.status='active' WHERE s.id=?1",
            [&scan_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|_| "当前任务不存在，或工作空间已归档；请先恢复工作空间".to_string())?;
    if scan_type != "web" {
        return Err("非 Web 任务请使用工作台继续执行流程".into());
    }
    let targets = {
        let mut statement = connection
            .prepare("SELECT id,status,value_score,scan_mode,routing_reason FROM sentinel_targets WHERE scan_id=?1 ORDER BY id")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([&scan_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        rows
    };
    prepare_web_scan_retry(&mut connection, &scan_id, &status)?;
    drop(connection);
    match confirm_sentinel_scan(app, state, scan_id.clone()) {
        Ok(scan) => Ok(scan),
        Err(error) => {
            // Preparing and starting a retry is one logical operation. If a
            // preflight fails, restore the exact terminal task/target state so
            // the UI never gets stranded in a misleading draft with blank
            // routing metadata.
            if let Ok(mut connection) = db::open(&db_path) {
                if let Ok(transaction) = connection.transaction() {
                    let _ = transaction.execute(
                        "UPDATE sentinel_scans SET status=?1,current_checkpoint=?2,previous_scan_id=?3,updated_at=datetime('now','localtime') WHERE id=?4 AND status='draft'",
                        params![status, checkpoint, previous_scan_id, scan_id],
                    );
                    let _ = transaction.execute(
                        "DELETE FROM app_settings WHERE key=?1",
                        [format!("sentinel-next-attempt-mode:{scan_id}")],
                    );
                    for (id, target_status, value_score, scan_mode, routing_reason) in targets {
                        let _ = transaction.execute(
                            "UPDATE sentinel_targets SET status=?1,value_score=?2,scan_mode=?3,routing_reason=?4,updated_at=datetime('now','localtime') WHERE id=?5 AND scan_id=?6",
                            params![target_status, value_score, scan_mode, routing_reason, id, scan_id],
                        );
                    }
                    let _ = transaction.commit();
                }
            }
            Err(format!("未完成阶段重试未启动，任务已恢复到上一轮终态：{error}"))
        }
    }
}

fn sentinel_settings(connection: &rusqlite::Connection) -> JsonValue {
    connection
        .query_row(
            "SELECT settings_json FROM config_profiles ORDER BY is_default DESC,id LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_default()
}

#[tauri::command]
pub fn list_strix_skills(state: State<AppState>) -> Result<Vec<StrixSkill>, String> {
    let connection = db::open(&state.db_path)?;
    let mut statement = connection.prepare(
        "SELECT id,name,description,instructions,builtin,enabled,created_at,updated_at FROM strix_skills ORDER BY builtin DESC,updated_at DESC,id DESC"
    ).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok(StrixSkill {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                instructions: row.get(3)?,
                builtin: row.get::<_, i64>(4)? != 0,
                enabled: row.get::<_, i64>(5)? != 0,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

#[tauri::command]
pub fn save_strix_skill(state: State<AppState>, input: StrixSkillInput) -> Result<i64, String> {
    let name = input.name.trim();
    let instructions = input.instructions.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err("技能名称不能为空且不能超过 80 个字符".into());
    }
    if instructions.is_empty() || instructions.chars().count() > 30_000 {
        return Err("技能指令不能为空且不能超过 30000 个字符".into());
    }
    let connection = db::open(&state.db_path)?;
    if let Some(id) = input.id {
        let builtin: i64 = connection
            .query_row(
                "SELECT builtin FROM strix_skills WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .map_err(|_| "技能不存在".to_string())?;
        if builtin != 0 {
            return Err("内置技能不可覆盖；请新建自定义技能".into());
        }
        connection.execute("UPDATE strix_skills SET name=?1,description=?2,instructions=?3,enabled=?4,updated_at=datetime('now','localtime') WHERE id=?5", params![name,input.description.trim(),instructions,input.enabled as i64,id]).map_err(|error| error.to_string())?;
        Ok(id)
    } else {
        connection.execute("INSERT INTO strix_skills(name,description,instructions,builtin,enabled) VALUES(?1,?2,?3,0,?4)", params![name,input.description.trim(),instructions,input.enabled as i64]).map_err(|error| error.to_string())?;
        Ok(connection.last_insert_rowid())
    }
}

#[tauri::command]
pub fn delete_strix_skill(state: State<AppState>, skill_id: i64) -> Result<(), String> {
    let connection = db::open(&state.db_path)?;
    let deleted = connection
        .execute(
            "DELETE FROM strix_skills WHERE id=?1 AND builtin=0",
            [skill_id],
        )
        .map_err(|error| error.to_string())?;
    if deleted == 0 {
        return Err("内置技能不可删除，或技能不存在".into());
    }
    Ok(())
}

fn strix_trace_base(
    connection: &rusqlite::Connection,
    scan_id: &str,
) -> Result<
    (
        String,
        String,
        String,
        String,
        String,
        i64,
        i64,
        i64,
        i64,
        i64,
        String,
        String,
    ),
    String,
> {
    connection
        .query_row(
            "SELECT task_name,project_name,status,scan_type,task_path,llm_requests,input_tokens,output_tokens,cached_tokens,total_tokens,created_at,updated_at FROM sentinel_scans WHERE id=?1",
            [scan_id],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?,row.get(8)?,row.get(9)?,row.get(10)?,row.get(11)?)),
        )
        .map_err(|_| "Strix 任务不存在".to_string())
}

fn trace_run_dirs(task_path: &Path, scan_id: &str) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if task_path.is_dir() {
        roots.push(task_path.to_path_buf());
    }
    if let Some(task_parent) = task_path.parent() {
        if task_parent.file_name().and_then(|value| value.to_str()) == Some("sentinel-tasks") {
            if let Some(app_root) = task_parent.parent() {
                let job_root = app_root.join("strix-jobs").join(scan_id);
                if job_root.is_dir() {
                    roots.push(job_root);
                }
            }
        }
    }
    let mut dirs = roots
        .iter()
        .flat_map(|root| strix_run_dirs(root).unwrap_or_default())
        .collect::<Vec<_>>();
    dirs.sort();
    dirs.dedup();
    dirs
}

fn trace_hook_files(task_path: &Path, scan_id: &str) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if task_path.is_dir() {
        roots.push(task_path.to_path_buf());
    }
    if let Some(task_parent) = task_path.parent() {
        if task_parent.file_name().and_then(|value| value.to_str()) == Some("sentinel-tasks") {
            if let Some(app_root) = task_parent.parent() {
                roots.push(app_root.join("strix-jobs").join(scan_id));
            }
        }
    }
    fn walk(path: &Path, depth: usize, result: &mut Vec<PathBuf>) {
        if !path.is_dir() || depth == 0 {
            return;
        }
        let hook = path.join("llm-hook.jsonl");
        if hook.is_file() {
            result.push(hook);
        }
        if let Ok(entries) = fs::read_dir(path) {
            for child in entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
            {
                walk(&child, depth - 1, result);
            }
        }
    }
    let mut result = Vec::new();
    for root in roots {
        walk(&root, 8, &mut result);
    }
    result.sort();
    result.dedup();
    result
}

fn trace_prompt_audit(task_path: &Path, scan_id: &str) -> Option<StrixPromptAudit> {
    let mut candidates = Vec::new();
    if task_path.is_dir() {
        candidates.push(task_path.join("strix-prompt-audit.json"));
    }
    if let Some(task_parent) = task_path.parent() {
        if task_parent.file_name().and_then(|value| value.to_str()) == Some("sentinel-tasks") {
            if let Some(app_root) = task_parent.parent() {
                candidates.push(
                    app_root
                        .join("strix-jobs")
                        .join(scan_id)
                        .join("strix-prompt-audit.json"),
                );
            }
        }
    }
    candidates.into_iter().find_map(|path| {
        let mut audit = serde_json::from_slice::<StrixPromptAudit>(&fs::read(path).ok()?).ok()?;
        // Oviraptor currently captures only its generated instruction. Never present
        // a local manifest as the exact request assembled by Strix.
        audit.exact_model_request = false;
        audit.capture_level = "generated_instruction".into();
        audit.instruction = audit.instruction.as_deref().map(retained_trace_text);
        Some(audit)
    })
}

fn retained_trace_value(value: &JsonValue) -> JsonValue {
    match value {
        JsonValue::Object(values) => JsonValue::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), retained_trace_value(value)))
                .collect(),
        ),
        JsonValue::Array(values) => {
            JsonValue::Array(values.iter().map(retained_trace_value).collect())
        }
        JsonValue::String(value) => JsonValue::String(retained_trace_text(value)),
        _ => value.clone(),
    }
}

fn retained_trace_text(value: &str) -> String {
    value.to_string()
}

fn trace_preview(value: &str, limit: usize) -> (String, i64, bool) {
    let retained = retained_trace_text(value);
    let size = value.len() as i64;
    let mut preview = retained.chars().take(limit).collect::<String>();
    let truncated = retained.chars().count() > limit;
    if truncated {
        preview.push_str("\n… [preview truncated]");
    }
    (preview, size, truncated)
}

fn trace_message_detail(message: &JsonValue, event_type: &str) -> (String, i64, bool) {
    let value = match event_type {
        "function_call" => message
            .get("arguments")
            .and_then(JsonValue::as_str)
            .and_then(|value| serde_json::from_str::<JsonValue>(value).ok())
            .map(|value| retained_trace_value(&value).to_string())
            .unwrap_or_else(|| {
                message
                    .get("arguments")
                    .and_then(JsonValue::as_str)
                    .unwrap_or("")
                    .to_string()
            }),
        "function_call_output" => message
            .get("output")
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .to_string(),
        "reasoning" => message
            .get("summary")
            .map(|value| retained_trace_value(value).to_string())
            .unwrap_or_default(),
        _ => message
            .get("content")
            .map(|value| match value {
                JsonValue::String(value) => value.clone(),
                _ => retained_trace_value(value).to_string(),
            })
            .unwrap_or_default(),
    };
    trace_preview(&value, 1200)
}

fn collect_strix_trace(
    connection: &rusqlite::Connection,
    scan_id: &str,
    include_events: bool,
    latest_attempt_only: bool,
) -> Result<(StrixTraceSummary, Vec<StrixTraceEvent>), String> {
    let (
        task_name,
        project_name,
        status,
        scan_type,
        task_path,
        mut stored_requests,
        mut stored_input,
        mut stored_output,
        mut stored_cached,
        mut stored_total,
        created_at,
        updated_at,
    ) = strix_trace_base(connection, scan_id)?;
    let mut run_count = 0i64;
    let mut agent_count = 0i64;
    let mut message_count = 0i64;
    let mut reasoning_count = 0i64;
    let mut tool_call_count = 0i64;
    let mut tool_result_count = 0i64;
    let mut model = String::new();
    let mut instruction_hasher = Sha256::new();
    let mut has_instruction = false;
    let mut tools: HashMap<String, (i64, i64)> = HashMap::new();
    let mut events = Vec::new();
    let mut llm_requests = 0i64;
    let mut input_tokens = 0i64;
    let mut output_tokens = 0i64;
    let mut cached_tokens = 0i64;
    let mut total_tokens = 0i64;
    let mut usage_entry_count = 0i64;
    let mut usage_agent_ids = HashSet::new();
    let task_path = if latest_attempt_only {
        connection
            .query_row(
                "SELECT work_dir FROM sentinel_scan_attempts WHERE scan_id=?1 ORDER BY attempt_number DESC LIMIT 1",
                [scan_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(&task_path))
    } else {
        PathBuf::from(&task_path)
    };
    if latest_attempt_only {
        if let Some((requests, input, output, cached, total)) = connection
            .query_row(
                "SELECT llm_requests_delta,input_tokens_delta,output_tokens_delta,cached_tokens_delta,total_tokens_delta FROM sentinel_scan_attempts WHERE scan_id=?1 ORDER BY attempt_number DESC LIMIT 1",
                [scan_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?
        {
            stored_requests = requests;
            stored_input = input;
            stored_output = output;
            stored_cached = cached;
            stored_total = total;
        }
    }
    let run_dirs = if latest_attempt_only {
        strix_run_dirs(&task_path).unwrap_or_default()
    } else {
        trace_run_dirs(&task_path, scan_id)
    };
    let hook_files = trace_hook_files(&task_path, scan_id);
    let mut hook_usage = llm_hook::UsageTotals::default();
    let mut hook_records = Vec::new();
    let mut exact_request_capture = false;
    let mut token_usage_estimated = false;
    for path in &hook_files {
        let usage = llm_hook::usage_from_file(path);
        hook_usage.requests += usage.requests;
        hook_usage.input_tokens += usage.input_tokens;
        hook_usage.output_tokens += usage.output_tokens;
        hook_usage.cached_tokens += usage.cached_tokens;
        hook_usage.total_tokens += usage.total_tokens;
        let records = llm_hook::records_from_file(path);
        exact_request_capture |= records.iter().any(|record| record.get("request").is_some());
        token_usage_estimated |= records.iter().any(|record| {
            record
                .get("usageEstimated")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false)
        });
        if include_events {
            hook_records.extend(records);
        }
    }
    let use_hook_usage = hook_usage.requests > 0 || hook_usage.failed_requests > 0;
    if use_hook_usage {
        llm_requests = hook_usage.requests;
        input_tokens = hook_usage.input_tokens;
        output_tokens = hook_usage.output_tokens;
        cached_tokens = hook_usage.cached_tokens;
        total_tokens = hook_usage.total_tokens;
    }
    if include_events {
        let completed_request_ids = hook_records
            .iter()
            .filter(|record| record.get("kind").and_then(JsonValue::as_str) == Some("model_call"))
            .filter_map(|record| record.get("requestId").and_then(JsonValue::as_str))
            .map(str::to_string)
            .collect::<HashSet<_>>();
        hook_records.retain(|record| {
            record.get("kind").and_then(JsonValue::as_str) != Some("model_call_started")
                || record
                    .get("requestId")
                    .and_then(JsonValue::as_str)
                    .is_none_or(|request_id| !completed_request_ids.contains(request_id))
        });
    }
    for run_dir in run_dirs {
        run_count += 1;
        let mut run_target = String::new();
        if let Ok(bytes) = fs::read(run_dir.join(STRIX_RUN_ARTIFACT)) {
            if let Ok(run) = serde_json::from_slice::<JsonValue>(&bytes) {
                run_target = run
                    .pointer("/targets_info/0/original")
                    .or_else(|| run.pointer("/targets_info/0/target"))
                    .or_else(|| run.get("target"))
                    .and_then(JsonValue::as_str)
                    .unwrap_or("")
                    .to_string();
                if let Some(instruction) = run.get("instruction").and_then(JsonValue::as_str) {
                    instruction_hasher.update(instruction.as_bytes());
                    has_instruction = true;
                }
                let usage = run.get("llm_usage").unwrap_or(&JsonValue::Null);
                usage_entry_count += usage
                    .get("request_usage_entries")
                    .and_then(JsonValue::as_array)
                    .map(|items| items.len() as i64)
                    .unwrap_or(0);
                if let Some(agents) = usage.get("agents").and_then(JsonValue::as_array) {
                    for agent in agents {
                        let id = agent
                            .get("agent_id")
                            .and_then(JsonValue::as_str)
                            .unwrap_or("");
                        if !id.is_empty() {
                            usage_agent_ids.insert(id.to_string());
                        }
                    }
                }
                if !use_hook_usage {
                    llm_requests += usage_request_count(usage);
                    input_tokens += usage_input_tokens(usage);
                    output_tokens += usage_output_tokens(usage);
                    cached_tokens += usage_cached_tokens(usage);
                    total_tokens += usage_total_tokens(usage);
                }
            }
        }
        let agents_path = strix_agent_state_path(&run_dir);
        if !agents_path.is_file() {
            continue;
        }
        let Ok(agent_db) = rusqlite::Connection::open(&agents_path) else {
            continue;
        };
        agent_count += agent_db
            .query_row(STRIX_AGENT_SESSION_COUNT_QUERY, [], |row| row.get(0))
            .unwrap_or(0);
        let Ok(mut statement) = agent_db.prepare(
            STRIX_AGENT_TRACE_QUERY,
        ) else {
            continue;
        };
        let Ok(rows) = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        }) else {
            continue;
        };
        let mut call_names: HashMap<String, String> = HashMap::new();
        for row in rows.flatten() {
            message_count += 1;
            let message = json(row.2);
            let event_type = message
                .get("type")
                .and_then(JsonValue::as_str)
                .unwrap_or("message");
            let role = message
                .get("role")
                .and_then(JsonValue::as_str)
                .unwrap_or("");
            let mut name = message
                .get("name")
                .and_then(JsonValue::as_str)
                .unwrap_or("")
                .to_string();
            let call_id = message
                .get("call_id")
                .and_then(JsonValue::as_str)
                .unwrap_or("")
                .to_string();
            if event_type == "function_call" && !call_id.is_empty() && !name.is_empty() {
                call_names.insert(call_id.clone(), name.clone());
            } else if event_type == "function_call_output" && name.is_empty() {
                name = call_names.get(&call_id).cloned().unwrap_or_default();
            }
            let event_status = message
                .get("status")
                .and_then(JsonValue::as_str)
                .unwrap_or("")
                .to_string();
            if event_type == "reasoning" {
                reasoning_count += 1;
            } else if event_type == "function_call" {
                tool_call_count += 1;
                if !name.is_empty() {
                    tools.entry(name.clone()).or_default().0 += 1;
                }
            } else if event_type == "function_call_output" {
                tool_result_count += 1;
                if !name.is_empty() {
                    tools.entry(name.clone()).or_default().1 += 1;
                }
            }
            if model.is_empty() {
                model = message
                    .pointer("/provider_data/model")
                    .and_then(JsonValue::as_str)
                    .unwrap_or("")
                    .to_string();
            }
            if include_events && events.len() < 800 {
                let (detail, detail_size, detail_truncated) =
                    trace_message_detail(&message, event_type);
                events.push(StrixTraceEvent {
                    id: format!(
                        "{}:{}",
                        run_dir
                            .file_name()
                            .and_then(|value| value.to_str())
                            .unwrap_or("run"),
                        row.0
                    ),
                    session_id: row.1,
                    call_id: call_id.clone(),
                    target_url: run_target.clone(),
                    event_type: event_type.to_string(),
                    role: role.to_string(),
                    name,
                    status: event_status,
                    detail,
                    detail_size,
                    detail_truncated,
                    created_at: row.3,
                });
            }
        }
    }
    if include_events {
        for (index, record) in hook_records.iter().enumerate() {
            let call_type = record
                .get("callType")
                .and_then(JsonValue::as_str)
                .unwrap_or("scan");
            let detail_value = if exact_request_capture {
                serde_json::json!({
                    "callType": call_type,
                    "request": record.get("request").cloned().unwrap_or(JsonValue::Null),
                    "response": record.get("response").cloned().unwrap_or(JsonValue::Null),
                    "usage": record.get("usage").cloned().unwrap_or(JsonValue::Null),
                })
            } else {
                serde_json::json!({
                    "callType": call_type,
                    "requestHash": record.get("requestHash").cloned().unwrap_or(JsonValue::Null),
                    "requestChars": record.get("requestChars").cloned().unwrap_or(JsonValue::Null),
                    "requestSummary": record.get("requestSummary").cloned().unwrap_or(JsonValue::Null),
                    "usage": record.get("usage").cloned().unwrap_or(JsonValue::Null),
                })
            };
            let (detail, detail_size, detail_truncated) =
                trace_preview(&detail_value.to_string(), 20_000);
            events.push(StrixTraceEvent {
                id: format!("llm-hook:{index}"),
                session_id: "llm-hook".into(),
                call_id: record
                    .get("requestHash")
                    .and_then(JsonValue::as_str)
                    .unwrap_or("")
                    .chars()
                    .take(12)
                    .collect(),
                target_url: String::new(),
                event_type: "model_request".into(),
                role: "model".into(),
                name: record
                    .get("model")
                    .and_then(JsonValue::as_str)
                    .map(|model| match call_type {
                        "context_compaction" => format!("上下文压缩 · {model}"),
                        "health_check" => format!("模型健康检查 · {model}"),
                        _ => model.to_string(),
                    })
                    .unwrap_or_else(|| {
                        match call_type {
                            "context_compaction" => "上下文压缩 · local-llm".into(),
                            "health_check" => "模型健康检查 · local-llm".into(),
                            _ => "local-llm".into(),
                        }
                    }),
                status: record
                    .get("status")
                    .and_then(JsonValue::as_str)
                    .unwrap_or_else(|| {
                        if record.get("kind").and_then(JsonValue::as_str)
                            == Some("model_call_started")
                        {
                            "in_flight"
                        } else {
                            "recorded"
                        }
                    })
                    .to_string(),
                detail,
                detail_size,
                detail_truncated,
                created_at: record
                    .get("recordedAt")
                    .and_then(JsonValue::as_str)
                    .unwrap_or("")
                    .to_string(),
            });
        }
    }
    events.sort_by(|left, right| left.created_at.cmp(&right.created_at));
    let mut tools = tools
        .into_iter()
        .map(|(name, (calls, results))| StrixTraceToolStat {
            name,
            calls,
            results,
        })
        .collect::<Vec<_>>();
    tools.sort_by_key(|tool| std::cmp::Reverse(tool.calls + tool.results));
    let instruction_hash = if has_instruction {
        format!("{:x}", instruction_hasher.finalize())
    } else {
        String::new()
    };
    let knowledge_id = connection
        .query_row(
            "SELECT id FROM strix_knowledge_entries WHERE scan_id=?1",
            [scan_id],
            |row| row.get(0),
        )
        .optional()
        .unwrap_or(None);
    Ok((
        StrixTraceSummary {
            scan_id: scan_id.to_string(),
            task_name,
            project_name,
            status,
            scan_type,
            model,
            run_count,
            agent_count,
            message_count,
            reasoning_count,
            tool_call_count,
            tool_result_count,
            llm_requests: llm_requests.max(stored_requests),
            input_tokens: input_tokens.max(stored_input),
            output_tokens: output_tokens.max(stored_output),
            cached_tokens: cached_tokens.max(stored_cached),
            total_tokens: total_tokens.max(stored_total),
            hooked_request_count: hook_usage.requests,
            exact_request_capture,
            usage_entry_count,
            usage_agent_count: usage_agent_ids.len() as i64,
            token_usage_estimated,
            instruction_hash,
            tools,
            knowledge_id,
            created_at,
            updated_at,
        },
        events,
    ))
}

#[tauri::command]
pub fn list_strix_traces(state: State<AppState>) -> Result<Vec<StrixTraceSummary>, String> {
    let connection = db::open(&state.db_path)?;
    let mut statement = connection
        .prepare(
            "SELECT id FROM sentinel_scans WHERE task_path<>'' ORDER BY created_at DESC LIMIT 120",
        )
        .map_err(|error| error.to_string())?;
    let ids = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .flatten()
        .collect::<Vec<_>>();
    drop(statement);
    Ok(ids
        .iter()
        .filter_map(|id| {
            collect_strix_trace(&connection, id, false, false)
                .ok()
                .map(|value| value.0)
        })
        .collect())
}

#[tauri::command]
pub fn get_strix_trace(
    state: State<AppState>,
    scan_id: String,
) -> Result<StrixTraceDetail, String> {
    let connection = db::open(&state.db_path)?;
    let fallback_task_path = strix_trace_base(&connection, &scan_id)?.4;
    let task_path = connection
        .query_row(
            "SELECT work_dir FROM sentinel_scan_attempts WHERE scan_id=?1 ORDER BY attempt_number DESC LIMIT 1",
            [&scan_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(fallback_task_path);
    let (summary, events) = collect_strix_trace(&connection, &scan_id, true, true)?;
    let mut prompt_audit = trace_prompt_audit(Path::new(&task_path), &scan_id);
    if let Some(audit) = prompt_audit.as_mut() {
        audit.exact_model_request = summary.exact_request_capture;
        if summary.exact_request_capture {
            audit.capture_level = "generated_instruction_and_model_requests".into();
            audit.notice = "Oviraptor instruction 快照与本地模型 Hook 捕获的最终请求均按原文保存在本机；逐请求内容位于下方调用时间线。".into();
        }
    }
    Ok(StrixTraceDetail {
        summary,
        events,
        prompt_audit,
    })
}
