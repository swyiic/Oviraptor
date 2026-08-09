fn hackerone_program(row: &Row<'_>) -> rusqlite::Result<HackerOneProgram> {
    Ok(HackerOneProgram {
        id: row.get(0)?,
        handle: row.get(1)?,
        name: row.get(2)?,
        icon_url: row.get(3)?,
        policy: row.get(4)?,
        submission_state: row.get(5)?,
        program_state: row.get(6)?,
        offers_bounties: row.get::<_, i64>(7)? != 0,
        open_scope: row.get::<_, i64>(8)? != 0,
        fast_payments: row.get::<_, i64>(9)? != 0,
        safe_harbor: row.get::<_, i64>(10)? != 0,
        collaboration: row.get::<_, i64>(11)? != 0,
        last_synced_at: row.get(12)?,
        bookmarked: row.get::<_, i64>(13)? != 0,
        scope_count: row.get(14)?,
    })
}

const H1_PROGRAM_SELECT: &str = r#"SELECT p.id,p.handle,p.name,p.icon_url,p.policy,p.submission_state,p.program_state,p.offers_bounties,p.open_scope,p.fast_payments,p.safe_harbor,p.collaboration,p.last_synced_at,
 COALESCE(n.bookmarked,0),(SELECT COUNT(*) FROM hackerone_scopes s WHERE s.program_handle=p.handle AND s.active=1)
 FROM hackerone_programs p LEFT JOIN hackerone_notes n ON n.program_handle=p.handle"#;

#[tauri::command]
pub fn list_hackerone_programs(
    state: State<'_, AppState>,
    search: Option<String>,
) -> Result<Vec<HackerOneProgram>, String> {
    let connection = db::open(&state.db_path)?;
    let needle = format!("%{}%", search.unwrap_or_default().trim());
    let sql=format!("{H1_PROGRAM_SELECT} WHERE (?1='%%' OR p.name LIKE ?1 OR p.handle LIKE ?1) ORDER BY COALESCE(n.bookmarked,0) DESC,p.offers_bounties DESC,p.name");
    let mut statement = connection.prepare(&sql).map_err(|e| e.to_string())?;
    let programs = statement
        .query_map([needle], hackerone_program)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(programs)
}

#[tauri::command]
pub fn get_hackerone_detail(
    state: State<'_, AppState>,
    handle: String,
) -> Result<HackerOneDetail, String> {
    let connection = db::open(&state.db_path)?;
    let program = connection
        .query_row(
            &format!("{H1_PROGRAM_SELECT} WHERE p.handle=?1"),
            [handle.as_str()],
            hackerone_program,
        )
        .map_err(|e| e.to_string())?;
    let mut scope_statement=connection.prepare("SELECT id,asset_type,asset_identifier,eligible_for_submission,eligible_for_bounty,max_severity,instruction,updated_at FROM hackerone_scopes WHERE program_handle=?1 AND active=1 ORDER BY eligible_for_submission DESC,eligible_for_bounty DESC,asset_type,asset_identifier").map_err(|e|e.to_string())?;
    let scopes = scope_statement
        .query_map([handle.as_str()], |row| {
            Ok(HackerOneScope {
                id: row.get(0)?,
                asset_type: row.get(1)?,
                asset_identifier: row.get(2)?,
                eligible_for_submission: row.get::<_, i64>(3)? != 0,
                eligible_for_bounty: row.get::<_, i64>(4)? != 0,
                max_severity: row.get(5)?,
                instruction: row.get(6)?,
                updated_at: row.get(7)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let mut exclusion_statement=connection.prepare("SELECT id,category,details,updated_at FROM hackerone_exclusions WHERE program_handle=?1 AND active=1 ORDER BY category").map_err(|e|e.to_string())?;
    let exclusions = exclusion_statement
        .query_map([handle.as_str()], |row| {
            Ok(HackerOneExclusion {
                id: row.get(0)?,
                category: row.get(1)?,
                details: row.get(2)?,
                updated_at: row.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(HackerOneDetail {
        program,
        scopes,
        exclusions,
    })
}

#[tauri::command]
pub fn set_hackerone_bookmark(
    state: State<'_, AppState>,
    handle: String,
    bookmarked: bool,
) -> Result<(), String> {
    db::open(&state.db_path)?.execute("INSERT INTO hackerone_notes(program_handle,bookmarked) VALUES(?1,?2) ON CONFLICT(program_handle) DO UPDATE SET bookmarked=excluded.bookmarked",params![handle,bookmarked as i64]).map_err(|e|e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn list_hackerone_events(
    state: State<AppState>,
    limit: Option<i64>,
) -> Result<Vec<HackerOneEvent>, String> {
    let connection = db::open(&state.db_path)?;
    let mut statement=connection.prepare("SELECT id,program_handle,event_type,summary,created_at FROM hackerone_events ORDER BY id DESC LIMIT ?1").map_err(|e|e.to_string())?;
    let events = statement
        .query_map([limit.unwrap_or(100).clamp(1, 500)], |row| {
            Ok(HackerOneEvent {
                id: row.get(0)?,
                program_handle: row.get(1)?,
                event_type: row.get(2)?,
                summary: row.get(3)?,
                created_at: row.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(events)
}

#[tauri::command]
pub fn sync_hackerone(
    app: AppHandle,
    state: State<AppState>,
    profile_id: i64,
    handle: Option<String>,
) -> Result<String, String> {
    let connection = db::open(&state.db_path)?;
    let settings_text: String = connection
        .query_row(
            "SELECT settings_json FROM config_profiles WHERE id=?1",
            [profile_id],
            |row| row.get(0),
        )
        .map_err(|_| "配置方案不存在".to_string())?;
    let settings: JsonValue = serde_json::from_str(&settings_text).map_err(|e| e.to_string())?;
    let get = |key: &str| {
        settings
            .get(key)
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    };
    let username = get("hackerOneUsername");
    let token = get("hackerOneToken");
    if username.is_empty() || token.is_empty() {
        return Err("请先在配置中心填写 HackerOne API identifier 和 token".into());
    }
    let override_dir = get("scriptsDirectory");
    let mut candidates = Vec::new();
    if !override_dir.is_empty() {
        candidates.push(PathBuf::from(override_dir));
    }
    if let Ok(dir) = app.path().resource_dir() {
        candidates.push(dir.join("resources/workers"));
        candidates.push(dir.join("workers"));
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/workers"));
    let script = candidates
        .into_iter()
        .map(|p| p.join("6_hackerone_sync.py"))
        .find(|p| p.exists())
        .ok_or("HackerOne同步脚本不存在")?;
    let python = get("pythonExecutable");
    let mut command = Command::new(if python.is_empty() {
        "python3"
    } else {
        &python
    });
    command
        .arg(script)
        .arg("--db")
        .arg(&state.db_path)
        .env("H1_API_USERNAME", username)
        .env("H1_API_TOKEN", token);
    if let Some(handle) = handle {
        command.arg("--handle").arg(handle);
    }
    let proxy = get("proxyUrl");
    if !proxy.is_empty() {
        command
            .env("HTTP_PROXY", &proxy)
            .env("HTTPS_PROXY", &proxy)
            .env("ALL_PROXY", &proxy);
    }
    let no_proxy = get("noProxy");
    if !no_proxy.is_empty() {
        command.env("NO_PROXY", no_proxy);
    }
    let output = command.output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[tauri::command]
pub fn add_hackerone_scopes_to_project(
    state: State<AppState>,
    handle: String,
    project_id: i64,
) -> Result<i64, String> {
    let mut connection = db::open(&state.db_path)?;
    let transaction = connection.transaction().map_err(|e| e.to_string())?;
    let mut statement=transaction.prepare("SELECT asset_type,asset_identifier FROM hackerone_scopes WHERE program_handle=?1 AND active=1 AND eligible_for_submission=1").map_err(|e|e.to_string())?;
    let rows = statement
        .query_map([handle], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(statement);
    let mut added = 0;
    for (kind, raw) in rows {
        let lower = kind.to_lowercase();
        let (target_type, value) = if lower.contains("cidr") {
            ("cidr", raw)
        } else if lower.contains("ip address") {
            ("ip", raw)
        } else if lower.contains("domain") || lower.contains("wildcard") || lower == "url" {
            let without_scheme = raw.split("://").nth(1).unwrap_or(&raw);
            (
                "domain",
                without_scheme
                    .split('/')
                    .next()
                    .unwrap_or("")
                    .trim_start_matches("*.")
                    .to_string(),
            )
        } else {
            continue;
        };
        if value.is_empty() {
            continue;
        }
        added+=transaction.execute("INSERT OR IGNORE INTO targets(project_id,target_type,value,normalized_value) VALUES(?1,?2,?3,lower(?3))",params![project_id,target_type,value]).map_err(|e|e.to_string())? as i64;
    }
    transaction.commit().map_err(|e| e.to_string())?;
    Ok(added)
}
