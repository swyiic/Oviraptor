fn rule_pack_row(row: &Row<'_>) -> rusqlite::Result<SecurityRulePack> {
    Ok(SecurityRulePack {
        id: row.get(0)?,
        key: row.get(1)?,
        name: row.get(2)?,
        engine: row.get(3)?,
        repository: row.get(4)?,
        reference: row.get(5)?,
        local_path: row.get(6)?,
        previous_version: row.get(7)?,
        version: row.get(8)?,
        enabled: row.get::<_, i64>(9)? != 0,
        builtin: row.get::<_, i64>(10)? != 0,
        status: row.get(11)?,
        last_sync_at: row.get(12)?,
        error: row.get(13)?,
        added_count: row.get(14)?,
        modified_count: row.get(15)?,
        deleted_count: row.get(16)?,
        change_summary: json(row.get(17)?),
        progress: row.get(18)?,
        progress_stage: row.get(19)?,
        progress_message: row.get(20)?,
    })
}

const RULE_PACK_COLUMNS: &str = "id,key,name,engine,repository,reference,local_path,previous_version,version,enabled,builtin,status,last_sync_at,error,added_count,modified_count,deleted_count,change_summary,progress,progress_stage,progress_message";

fn builtin_rule_pack_source(key: &str) -> Option<(&'static str, &'static str)> {
    match key {
        "semgrep-rules" => Some(("https://github.com/semgrep/semgrep-rules.git", "develop")),
        "codeql-queries" => Some(("https://github.com/github/codeql.git", "main")),
        "owasp-benchmark" => Some(("https://github.com/OWASP/Benchmark.git", "master")),
        _ => None,
    }
}

#[tauri::command]
pub fn list_security_rule_packs(state: State<AppState>) -> Result<Vec<SecurityRulePack>, String> {
    let connection = db::open(&state.db_path)?;
    let mut stmt = connection
        .prepare(&format!(
            "SELECT {RULE_PACK_COLUMNS} FROM security_rule_packs ORDER BY builtin DESC,name"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], rule_pack_row)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

#[tauri::command]
pub fn save_security_rule_pack(
    state: State<AppState>,
    input: SecurityRulePackInput,
) -> Result<i64, String> {
    let key = input.key.trim().to_lowercase();
    let engine = input.engine.trim().to_lowercase();
    if key.is_empty() || input.name.trim().is_empty() {
        return Err("规则库必须填写标识和名称".into());
    }
    let builtin_source = builtin_rule_pack_source(&key);
    let repository = if input.repository.trim().is_empty() {
        builtin_source
            .map(|source| source.0)
            .ok_or_else(|| "自定义规则库必须填写 HTTPS 仓库地址".to_string())?
    } else {
        input.repository.trim()
    };
    let reference = if input.reference.trim().is_empty() {
        builtin_source.map(|source| source.1).unwrap_or("")
    } else {
        input.reference.trim()
    };
    if !repository.starts_with("https://") {
        return Err("规则库仓库地址只允许使用 HTTPS".into());
    }
    if !["semgrep", "codeql", "benchmark"].contains(&engine.as_str()) {
        return Err("规则库引擎只支持 semgrep、codeql 或 benchmark".into());
    }
    let connection = db::open(&state.db_path)?;
    if let Some(id) = connection
        .query_row(
            "SELECT id FROM security_rule_packs WHERE key=?1",
            [&key],
            |r| r.get::<_, i64>(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
    {
        connection.execute("UPDATE security_rule_packs SET name=?1,engine=?2,repository=?3,reference=?4,enabled=?5,updated_at=datetime('now','localtime') WHERE id=?6",params![input.name.trim(),engine,repository,reference,input.enabled as i64,id]).map_err(|e|e.to_string())?;
        Ok(id)
    } else {
        connection.execute("INSERT INTO security_rule_packs(key,name,engine,repository,reference,enabled,builtin) VALUES(?1,?2,?3,?4,?5,?6,0)",params![key,input.name.trim(),engine,repository,reference,input.enabled as i64]).map_err(|e|e.to_string())?;
        Ok(connection.last_insert_rowid())
    }
}

#[tauri::command]
pub fn delete_security_rule_pack(state: State<AppState>, pack_id: i64) -> Result<(), String> {
    let connection = db::open(&state.db_path)?;
    let builtin: i64 = connection
        .query_row(
            "SELECT builtin FROM security_rule_packs WHERE id=?1",
            [pack_id],
            |r| r.get(0),
        )
        .map_err(|_| "规则库不存在".to_string())?;
    if builtin != 0 {
        return Err("内置规则库不能删除，请改为停用".into());
    }
    connection
        .execute("DELETE FROM security_rule_packs WHERE id=?1", [pack_id])
        .map_err(|e| e.to_string())
        .map(|_| ())
}

fn rule_pack_root(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .unwrap_or(Path::new("."))
        .join("security-rule-packs")
}

fn run_git(path: Option<&Path>, args: &[&str]) -> Result<String, String> {
    let mut command = Command::new("git");
    if let Some(path) = path {
        command.current_dir(path);
    }
    let output = command
        .args(args)
        .output()
        .map_err(|error| format!("无法执行 git：{error}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if detail.is_empty() {
            format!("git {} 执行失败", args.first().copied().unwrap_or("命令"))
        } else {
            detail
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn update_rule_pack_progress(
    db_path: &Path,
    pack_id: i64,
    progress: i64,
    stage: &str,
    message: &str,
) {
    if let Ok(connection) = db::open(db_path) {
        let safe_message: String = message.chars().take(320).collect();
        let _ = connection.execute(
            "UPDATE security_rule_packs SET progress=?1,progress_stage=?2,progress_message=?3,updated_at=datetime('now','localtime') WHERE id=?4",
            params![progress.clamp(0, 100), stage, safe_message, pack_id],
        );
    }
}

fn git_progress_percent(line: &str) -> Option<i64> {
    line.split_whitespace().find_map(|part| {
        part.trim_matches(|value: char| !value.is_ascii_digit() && value != '%')
            .strip_suffix('%')
            .and_then(|value| value.parse::<i64>().ok())
            .filter(|value| (0..=100).contains(value))
    })
}

fn run_git_with_progress(
    db_path: &Path,
    pack_id: i64,
    path: Option<&Path>,
    args: &[String],
    stage: &str,
    base: i64,
    span: i64,
) -> Result<(), String> {
    let mut command = Command::new("git");
    if let Some(path) = path {
        command.current_dir(path);
    }
    command
        .args([
            "-c",
            "filter.lfs.smudge=",
            "-c",
            "filter.lfs.process=",
            "-c",
            "filter.lfs.required=false",
        ])
        .args(args)
        .env("GIT_LFS_SKIP_SMUDGE", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("无法执行 git：{error}"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "无法读取 git 同步进度".to_string())?;
    let mut last_line = String::new();
    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        last_line = line.chars().take(320).collect();
        let progress = git_progress_percent(line)
            .map(|value| base + value * span / 100)
            .unwrap_or(base);
        update_rule_pack_progress(db_path, pack_id, progress, stage, &last_line);
    }
    let status = child.wait().map_err(|error| error.to_string())?;
    if status.success() {
        Ok(())
    } else if last_line.is_empty() {
        Err(format!(
            "git {} 执行失败",
            args.first().map(String::as_str).unwrap_or("命令")
        ))
    } else {
        Err(last_line)
    }
}

fn rule_pack_changes(path: &Path, old_version: &str) -> Result<(i64, i64, i64, String), String> {
    let output = if old_version.is_empty() {
        run_git(Some(path), &["ls-files"])?
            .lines()
            .map(|file| format!("A\t{file}"))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        let current = run_git(Some(path), &["rev-parse", "HEAD"])?;
        if current.starts_with(old_version) || old_version.starts_with(&current) {
            String::new()
        } else {
            run_git(Some(path), &["diff", "--name-status", old_version, "HEAD"])?
        }
    };
    let mut added = 0;
    let mut modified = 0;
    let mut deleted = 0;
    let mut changes = Vec::new();
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        let parts = line.split('\t').collect::<Vec<_>>();
        let status = parts.first().copied().unwrap_or("M");
        let kind = match status.chars().next().unwrap_or('M') {
            'A' => {
                added += 1;
                "added"
            }
            'D' => {
                deleted += 1;
                "deleted"
            }
            _ => {
                modified += 1;
                "modified"
            }
        };
        if changes.len() < 200 {
            let path = if status.starts_with('R') && parts.len() > 2 {
                format!("{} -> {}", parts[1], parts[2])
            } else {
                parts.get(1).copied().unwrap_or(line).to_string()
            };
            changes.push(serde_json::json!({"status":kind,"path":path}));
        }
    }
    Ok((
        added,
        modified,
        deleted,
        serde_json::to_string(&changes).map_err(|error| error.to_string())?,
    ))
}

#[tauri::command]
pub fn sync_security_rule_pack(
    state: State<AppState>,
    pack_id: i64,
) -> Result<SecurityRulePack, String> {
    let connection = db::open(&state.db_path)?;
    let (key, mut repo, mut reference, old_version, current_status): (
        String,
        String,
        String,
        String,
        String,
    ) = connection
        .query_row(
            "SELECT key,repository,reference,version,status FROM security_rule_packs WHERE id=?1",
            [pack_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .map_err(|_| "规则库不存在".to_string())?;
    if current_status == "syncing" {
        return connection
            .query_row(
                &format!("SELECT {RULE_PACK_COLUMNS} FROM security_rule_packs WHERE id=?1"),
                [pack_id],
                rule_pack_row,
            )
            .map_err(|error| error.to_string());
    }
    if let Some((builtin_repo, builtin_reference)) = builtin_rule_pack_source(&key) {
        if repo.trim().is_empty() {
            repo = builtin_repo.into();
        }
        if reference.trim().is_empty() {
            reference = builtin_reference.into();
        }
    }
    if repo.trim().is_empty() || !repo.starts_with("https://") {
        return Err("规则库缺少有效的 HTTPS 仓库地址".into());
    }
    let path = rule_pack_root(&state.db_path).join(&key);
    fs::create_dir_all(rule_pack_root(&state.db_path)).map_err(|e| e.to_string())?;
    connection
        .execute(
            "UPDATE security_rule_packs SET status='syncing',error='',progress=2,progress_stage='prepare',progress_message='准备规则仓库和轻量检出环境' WHERE id=?1",
            [pack_id],
        )
        .map_err(|e| e.to_string())?;
    let db_path = state.db_path.clone();
    thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            if path.join(".git").exists() {
                let mut fetch = vec![
                    "fetch".into(),
                    "--progress".into(),
                    "--depth".into(),
                    "1".into(),
                    "origin".into(),
                ];
                if !reference.trim().is_empty() {
                    fetch.push(reference.clone());
                }
                run_git_with_progress(&db_path, pack_id, Some(&path), &fetch, "download", 5, 55)?;
                update_rule_pack_progress(
                    &db_path,
                    pack_id,
                    65,
                    "checkout",
                    "正在应用最新规则，Git LFS 文件保持为指针以避免阻塞",
                );
                run_git_with_progress(
                    &db_path,
                    pack_id,
                    Some(&path),
                    &["reset".into(), "--hard".into(), "FETCH_HEAD".into()],
                    "checkout",
                    65,
                    20,
                )?;
            } else {
                let destination = path.to_string_lossy().to_string();
                let mut clone = vec![
                    "clone".into(),
                    "--progress".into(),
                    "--depth".into(),
                    "1".into(),
                ];
                if !reference.trim().is_empty() {
                    clone.extend(["--branch".into(), reference.clone()]);
                }
                clone.extend([repo.clone(), destination]);
                run_git_with_progress(&db_path, pack_id, None, &clone, "download", 5, 80)?;
            }
            update_rule_pack_progress(&db_path, pack_id, 88, "index", "正在计算规则版本和变更清单");
            let version = run_git(Some(&path), &["rev-parse", "--short", "HEAD"])?;
            let (added, modified, deleted, changes) = rule_pack_changes(&path, &old_version)?;
            let connection = db::open(&db_path)?;
            connection.execute("UPDATE security_rule_packs SET repository=?1,reference=?2,local_path=?3,previous_version=?4,version=?5,status='ready',last_sync_at=datetime('now','localtime'),error='',added_count=?6,modified_count=?7,deleted_count=?8,change_summary=?9,progress=100,progress_stage='complete',progress_message='规则库同步完成',updated_at=datetime('now','localtime') WHERE id=?10",params![repo,reference,path.to_string_lossy(),old_version,version,added,modified,deleted,changes,pack_id]).map_err(|e|e.to_string())?;
            Ok(())
        })();
        if let Err(message) = result {
            if let Ok(connection) = db::open(&db_path) {
                let _ = connection.execute(
                    "UPDATE security_rule_packs SET status='error',error=?1,progress_stage='error',progress_message=?1,updated_at=datetime('now','localtime') WHERE id=?2",
                    params![message, pack_id],
                );
            }
        }
    });
    connection
        .query_row(
            &format!("SELECT {RULE_PACK_COLUMNS} FROM security_rule_packs WHERE id=?1"),
            [pack_id],
            rule_pack_row,
        )
        .map_err(|e| e.to_string())
}
