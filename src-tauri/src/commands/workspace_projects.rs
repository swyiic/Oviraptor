#[tauri::command]
pub async fn dashboard_stats(
    state: State<'_, AppState>,
    project_id: Option<i64>,
) -> Result<DashboardStats, String> {
    let connection = db::open(&state.db_path)?;
    let project_count = connection
        .query_row(
            "SELECT COUNT(*) FROM projects WHERE status='active'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let running_jobs = connection
        .query_row(
            "SELECT COUNT(*) FROM runs WHERE status IN ('queued','running','cancel_requested')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let (asset_count, alive_count, pending_count, blocked_count) = connection
        .query_row(
            r#"SELECT COUNT(*),
                      COALESCE(SUM(CASE WHEN a.probe_outcome IN ('web_alive','web_restricted','browser_render_required','virtual_host_required') AND COALESCE(a.content_category,'') NOT IN ('gambling','porn','custom_rule') THEN 1 ELSE 0 END),0),
                      COALESCE(SUM(CASE WHEN pa.decision IN ('pending','uncertain','') THEN 1 ELSE 0 END),0),
                      COALESCE(SUM(CASE WHEN a.probe_outcome='blocked_content' OR a.content_category IN ('gambling','porn','custom_rule') THEN 1 ELSE 0 END),0)
               FROM project_assets pa JOIN assets a ON a.id=pa.asset_id
               WHERE pa.is_deleted=0 AND (?1 IS NULL OR pa.project_id=?1)"#,
            [project_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|error| error.to_string())?;
    let (new_count, changed_count) = connection
        .query_row(
            r#"SELECT COALESCE(SUM(CASE WHEN event_type='new' THEN 1 ELSE 0 END),0),
                      COALESCE(SUM(CASE WHEN event_type='changed' THEN 1 ELSE 0 END),0)
               FROM asset_events
               WHERE created_at>=datetime('now','-30 day') AND (?1 IS NULL OR project_id=?1)"#,
            [project_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| error.to_string())?;
    Ok(DashboardStats {
        project_count,
        asset_count,
        alive_count,
        pending_count,
        new_count,
        changed_count,
        blocked_count,
        running_jobs,
    })
}

#[tauri::command]
pub async fn list_projects(state: State<'_, AppState>) -> Result<Vec<Project>, String> {
    let connection = db::open(&state.db_path)?;
    let mut statement = connection.prepare(r#"
        SELECT p.id,p.name,p.description,p.status,p.created_at,p.updated_at,
               (SELECT COUNT(*) FROM project_assets pa WHERE pa.project_id=p.id AND pa.is_deleted=0),
               (SELECT COUNT(*) FROM project_assets pa WHERE pa.project_id=p.id AND pa.is_deleted=0 AND pa.decision IN ('pending','uncertain','')),
               (SELECT COUNT(*) FROM targets t WHERE t.project_id=p.id),
               (SELECT COUNT(*) FROM runs r WHERE r.project_id=p.id),
               (SELECT COUNT(*) FROM sentinel_scans s WHERE s.project_id=p.id),
               (SELECT COUNT(*) FROM sentinel_findings f JOIN sentinel_scans s ON s.id=f.scan_id WHERE s.project_id=p.id AND f.kind='vulnerability'),
               (SELECT COUNT(*) FROM sentinel_validations v JOIN sentinel_scans s ON s.id=v.scan_id WHERE s.project_id=p.id AND v.verdict<>'pending'),
               (SELECT COUNT(*) FROM sentinel_fuse_zone z WHERE z.project_id=p.id AND z.archived=0),
               (SELECT MAX(COALESCE(r.finished_at,r.created_at)) FROM runs r WHERE r.project_id=p.id),
               (SELECT MAX(s.updated_at) FROM sentinel_scans s WHERE s.project_id=p.id)
        FROM projects p ORDER BY p.updated_at DESC, p.id DESC
    "#).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok(Project {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                status: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
                asset_count: row.get(6)?,
                pending_count: row.get(7)?,
                target_count: row.get(8)?,
                asset_run_count: row.get(9)?,
                scan_count: row.get(10)?,
                vulnerability_count: row.get(11)?,
                validation_count: row.get(12)?,
                active_fuse_count: row.get(13)?,
                last_run_at: row.get(14)?,
                last_scan_at: row.get(15)?,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

fn project_impact_for_connection(
    connection: &rusqlite::Connection,
    project_id: i64,
) -> Result<ProjectImpact, String> {
    let exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1)",
            [project_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if !exists {
        return Err("工作空间不存在或已被删除".into());
    }
    let count = |sql: &str| -> Result<i64, String> {
        connection
            .query_row(sql, [project_id], |row| row.get(0))
            .map_err(|error| error.to_string())
    };
    let asset_count = count("SELECT COUNT(*) FROM project_assets WHERE project_id=?1")?;
    let asset_event_count = count("SELECT COUNT(*) FROM asset_events WHERE project_id=?1")?;
    let target_count = count("SELECT COUNT(*) FROM targets WHERE project_id=?1")?;
    let asset_run_count = count("SELECT COUNT(*) FROM runs WHERE project_id=?1")?;
    let saved_view_count = count("SELECT COUNT(*) FROM saved_views WHERE project_id=?1")?;
    let sentinel_scan_count = count("SELECT COUNT(*) FROM sentinel_scans WHERE project_id=?1")?;
    let sentinel_target_count = count("SELECT COUNT(*) FROM sentinel_targets WHERE project_id=?1")?;
    let finding_count = count("SELECT COUNT(*) FROM sentinel_findings f JOIN sentinel_scans s ON s.id=f.scan_id WHERE s.project_id=?1")?;
    let validation_count = count("SELECT COUNT(*) FROM sentinel_validations v JOIN sentinel_scans s ON s.id=v.scan_id WHERE s.project_id=?1")?;
    let opportunity_count =
        count("SELECT COUNT(*) FROM sentinel_opportunities WHERE project_id=?1")?;
    let fuse_count = count("SELECT COUNT(*) FROM sentinel_fuse_zone WHERE project_id=?1")?;
    let appsec_vulnerability_count =
        count("SELECT COUNT(*) FROM appsec_vulnerabilities WHERE project_id=?1")?;
    let knowledge_count =
        count("SELECT COUNT(*) FROM strix_knowledge_entries WHERE project_id=?1")?;
    let learning_candidate_count =
        count("SELECT COUNT(*) FROM strix_learning_candidates WHERE project_id=?1")?;
    let browser_auth_session_count =
        count("SELECT COUNT(*) FROM browser_auth_sessions WHERE project_id=?1")?;
    let total_records = asset_count
        + asset_event_count
        + target_count
        + asset_run_count
        + saved_view_count
        + sentinel_scan_count
        + sentinel_target_count
        + finding_count
        + validation_count
        + opportunity_count
        + fuse_count
        + appsec_vulnerability_count
        + knowledge_count
        + learning_candidate_count
        + browser_auth_session_count;
    Ok(ProjectImpact {
        asset_count,
        asset_event_count,
        target_count,
        asset_run_count,
        saved_view_count,
        sentinel_scan_count,
        sentinel_target_count,
        finding_count,
        validation_count,
        opportunity_count,
        fuse_count,
        appsec_vulnerability_count,
        knowledge_count,
        learning_candidate_count,
        browser_auth_session_count,
        total_records,
    })
}

#[tauri::command]
pub fn project_impact(
    state: State<'_, AppState>,
    project_id: i64,
) -> Result<ProjectImpact, String> {
    let connection = db::open(&state.db_path)?;
    project_impact_for_connection(&connection, project_id)
}

#[tauri::command]
pub fn save_project(state: State<AppState>, input: ProjectInput) -> Result<i64, String> {
    let connection = db::open(&state.db_path)?;
    let name = input.name.trim();
    if name.is_empty() {
        return Err("项目名称不能为空".into());
    }
    let duplicate: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE lower(trim(name))=lower(?1) AND (?2 IS NULL OR id<>?2))",
            params![name, input.id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if duplicate {
        return Err(format!("项目名称“{name}”已存在，请使用其他名称"));
    }
    if let Some(id) = input.id {
        let updated = connection.execute(
            "UPDATE projects SET name=?1,description=?2,updated_at=datetime('now','localtime') WHERE id=?3",
            params![name, input.description.trim(), id],
        ).map_err(|error| error.to_string())?;
        if updated == 0 {
            return Err("工作空间不存在或已被删除".into());
        }
        Ok(id)
    } else {
        connection
            .execute(
                "INSERT INTO projects(name,description) VALUES(?1,?2)",
                params![name, input.description.trim()],
            )
            .map_err(|error| error.to_string())?;
        Ok(connection.last_insert_rowid())
    }
}

#[tauri::command]
pub fn archive_project(
    state: State<'_, AppState>,
    project_id: i64,
    archived: bool,
) -> Result<(), String> {
    let connection = db::open(&state.db_path)?;
    let updated = connection
        .execute(
            "UPDATE projects SET status=?1,updated_at=datetime('now','localtime') WHERE id=?2",
            params![if archived { "archived" } else { "active" }, project_id],
        )
        .map_err(|error| error.to_string())?;
    if updated == 0 {
        return Err("工作空间不存在或已被删除".into());
    }
    Ok(())
}

#[tauri::command]
pub fn delete_project(state: State<AppState>, project_id: i64) -> Result<(), String> {
    let mut connection = db::open(&state.db_path)?;
    let impact = project_impact_for_connection(&connection, project_id)?;
    if impact.total_records > 0 {
        return Err(format!(
            "该工作空间仍关联 {} 条记录（{} 条资产、{} 个 Strix 任务、{} 条证据/结论），不能删除；请归档以保留完整历史",
            impact.total_records,
            impact.asset_count,
            impact.sentinel_scan_count,
            impact.finding_count + impact.validation_count
        ));
    }
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    let deleted = transaction
        .execute("DELETE FROM projects WHERE id=?1", [project_id])
        .map_err(|error| error.to_string())?;
    if deleted == 0 {
        return Err("项目不存在".into());
    }
    // 项目在上方已经确认没有关联资产。这里不能顺带扫描并清理整个 assets 表：
    // 数据量较大时，删除一个空项目会退化成昂贵的全库维护操作。
    // 孤儿资产清理由独立的数据库维护流程负责。
    transaction.commit().map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_app_settings(state: State<AppState>) -> Result<AppSettings, String> {
    let connection = db::open(&state.db_path)?;
    let value = |key: &str, fallback: &str| -> String {
        connection
            .query_row(
                "SELECT value FROM app_settings WHERE key=?1",
                [key],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| fallback.to_string())
    };
    Ok(AppSettings {
        reminder_days: value("reminder_days", "7").parse().unwrap_or(7),
        custom_icon: value("custom_icon", "false") == "true",
        deduplicated_assets: value("last_deduplicated", "0").parse().unwrap_or(0),
    })
}

#[tauri::command]
pub fn get_app_icon_data_url(state: State<AppState>) -> Result<String, String> {
    let custom = state.app_data_dir.join("custom-app-icon.png");
    let bytes = if custom.is_file() {
        let bytes = fs::read(custom).map_err(|error| error.to_string())?;
        if Image::from_bytes(&bytes).is_ok() {
            bytes
        } else {
            include_bytes!("../../icons/brand-icon.png").to_vec()
        }
    } else {
        include_bytes!("../../icons/brand-icon.png").to_vec()
    };
    // A single base64 string is materially cheaper to serialize across Tauri
    // IPC than hundreds of thousands of JSON array elements.
    Ok(format!(
        "data:image/png;base64,{}",
        BASE64_STANDARD.encode(bytes)
    ))
}

#[tauri::command]
pub fn save_app_settings(state: State<AppState>, input: AppSettingsInput) -> Result<(), String> {
    if !(1..=365).contains(&input.reminder_days) {
        return Err("提醒天数必须在 1 到 365 之间".into());
    }
    db::open(&state.db_path)?
        .execute(
            "INSERT INTO app_settings(key,value) VALUES('reminder_days',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [input.reminder_days.to_string()],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn save_app_icon(state: State<AppState>, bytes: Vec<u8>) -> Result<(), String> {
    if bytes.is_empty() || bytes.len() > 5 * 1024 * 1024 {
        return Err("图标必须是小于 5MB 的 PNG 文件".into());
    }
    let icon = Image::from_bytes(&bytes).map_err(|_| "无法解析 PNG 图标".to_string())?;
    if icon.width() < 32 || icon.height() < 32 {
        return Err("图标尺寸至少为 32×32".into());
    }
    let path = state.app_data_dir.join("custom-app-icon.png");
    fs::write(&path, bytes).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    db::open(&state.db_path)?
        .execute(
            "INSERT INTO app_settings(key,value) VALUES('custom_icon','true') ON CONFLICT(key) DO UPDATE SET value='true'",
            [],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn reset_app_icon(state: State<AppState>) -> Result<(), String> {
    let _ = fs::remove_file(state.app_data_dir.join("custom-app-icon.png"));
    for directory in &state.legacy_icon_dirs {
        let _ = fs::remove_file(directory.join("custom-app-icon.png"));
    }
    db::open(&state.db_path)?
        .execute(
            "INSERT INTO app_settings(key,value) VALUES('custom_icon','false') ON CONFLICT(key) DO UPDATE SET value='false'",
            [],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn startup_status(state: State<AppState>) -> Result<StartupStatus, String> {
    let connection = db::open(&state.db_path)?;
    connection
        .execute(
            "UPDATE runs SET status='interrupted',stage='interrupted',error='上次运行时应用或电脑被关闭，任务未完成',finished_at=datetime('now','localtime') WHERE status IN ('queued','running','cancel_requested')",
            [],
        )
        .map_err(|error| error.to_string())?;
    let reminder_days: i64 = connection
        .query_row(
            "SELECT value FROM app_settings WHERE key='reminder_days'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap_or_else(|_| "7".into())
        .parse()
        .unwrap_or(7);

    let mut stale_statement = connection.prepare(r#"
        SELECT p.id,p.name,
               MAX(CASE WHEN r.status IN ('completed','historical_import') OR r.stage='completed' THEN COALESCE(r.finished_at,r.created_at) END) AS last_run,
               CAST(julianday('now','localtime')-julianday(MAX(CASE WHEN r.status IN ('completed','historical_import') OR r.stage='completed' THEN COALESCE(r.finished_at,r.created_at) END)) AS INTEGER) AS days_old
        FROM projects p LEFT JOIN runs r ON r.project_id=p.id
        WHERE p.status='active'
          AND (EXISTS(SELECT 1 FROM targets t WHERE t.project_id=p.id AND t.enabled=1)
               OR EXISTS(SELECT 1 FROM project_assets pa WHERE pa.project_id=p.id AND pa.is_deleted=0))
        GROUP BY p.id,p.name
        HAVING last_run IS NULL OR days_old>=?1
        ORDER BY COALESCE(days_old,999999) DESC,p.name
    "#).map_err(|error| error.to_string())?;
    let stale_projects = stale_statement
        .query_map([reminder_days], |row| {
            Ok(StaleProject {
                project_id: row.get(0)?,
                project_name: row.get(1)?,
                last_run_at: row.get(2)?,
                days_since_update: row.get(3)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;

    let mut interrupted_statement = connection
        .prepare(
            r#"
        SELECT r.id,r.project_id,p.name,r.profile_id,r.name,r.pipeline,r.created_at
        FROM runs r JOIN projects p ON p.id=r.project_id
        WHERE r.status='interrupted' ORDER BY r.id DESC LIMIT 30
    "#,
        )
        .map_err(|error| error.to_string())?;
    let interrupted_jobs = interrupted_statement
        .query_map([], |row| {
            Ok(InterruptedJob {
                run_id: row.get(0)?,
                project_id: row.get(1)?,
                project_name: row.get(2)?,
                profile_id: row.get(3)?,
                name: row.get(4)?,
                pipeline: row.get(5)?,
                created_at: row.get(6)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(StartupStatus {
        reminder_days,
        stale_projects,
        interrupted_jobs,
    })
}

#[tauri::command]
pub fn acknowledge_interrupted_run(state: State<AppState>, run_id: i64) -> Result<(), String> {
    db::open(&state.db_path)?
        .execute(
            "UPDATE runs SET status='restarted',stage='restarted' WHERE id=?1 AND status='interrupted'",
            [run_id],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn list_config_profiles(state: State<AppState>) -> Result<Vec<ConfigProfile>, String> {
    let connection = db::open(&state.db_path)?;
    let mut statement = connection.prepare(
        "SELECT id,name,description,is_default,settings_json,created_at,updated_at FROM config_profiles ORDER BY is_default DESC, updated_at DESC"
    ).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok(ConfigProfile {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                is_default: row.get::<_, i64>(3)? != 0,
                settings: json(row.get(4)?),
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn save_config_profile(
    state: State<'_, AppState>,
    input: ConfigProfileInput,
) -> Result<i64, String> {
    if input.name.trim().is_empty() {
        return Err("配置名称不能为空".into());
    }
    let mut connection = db::open(&state.db_path)?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    if input.is_default {
        transaction
            .execute("UPDATE config_profiles SET is_default=0", [])
            .map_err(|error| error.to_string())?;
    }
    let id = if let Some(id) = input.id {
        transaction.execute(
            "UPDATE config_profiles SET name=?1,description=?2,is_default=?3,settings_json=?4,updated_at=datetime('now','localtime') WHERE id=?5",
            params![input.name.trim(), input.description.trim(), input.is_default as i64, input.settings.to_string(), id],
        ).map_err(|error| error.to_string())?;
        id
    } else {
        transaction.execute(
            "INSERT INTO config_profiles(name,description,is_default,settings_json) VALUES(?1,?2,?3,?4)",
            params![input.name.trim(), input.description.trim(), input.is_default as i64, input.settings.to_string()],
        ).map_err(|error| error.to_string())?;
        transaction.last_insert_rowid()
    };
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(id)
}

#[tauri::command]
pub fn delete_config_profile(state: State<AppState>, profile_id: i64) -> Result<(), String> {
    let connection = db::open(&state.db_path)?;
    let is_default = connection
        .query_row(
            "SELECT is_default FROM config_profiles WHERE id=?1",
            [profile_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "配置方案不存在".to_string())?;
    if is_default != 0 {
        return Err("系统默认配置不能删除".into());
    }
    connection
        .execute("DELETE FROM config_profiles WHERE id=?1", [profile_id])
        .map_err(|error| error.to_string())?;
    Ok(())
}
