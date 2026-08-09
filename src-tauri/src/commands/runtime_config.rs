fn strix_skill_instructions(
    connection: &rusqlite::Connection,
    skill_ids: &[i64],
) -> Result<(String, String), String> {
    if skill_ids.is_empty() {
        return Ok((String::new(), String::new()));
    }
    let ids = serde_json::to_string(skill_ids).map_err(|error| error.to_string())?;
    let sql = "SELECT name,instructions FROM strix_skills WHERE enabled=1 AND id IN (SELECT value FROM json_each(?1)) ORDER BY builtin DESC,id";
    let mut statement = connection.prepare(sql).map_err(|error| error.to_string())?;
    let rows: Vec<(String, String)> = statement
        .query_map([ids], strix_skill_row)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    const MAX_SKILL_CONTEXT_CHARS: usize = 48_000;
    const MAX_LARGE_SKILL_CHARS: usize = 24_000;

    let row_count = rows.len();
    let names = rows
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join("、");
    let mut used = 0usize;
    let mut rendered = Vec::new();
    for (name, body) in rows {
        if used >= MAX_SKILL_CONTEXT_CHARS {
            break;
        }
        let remaining = MAX_SKILL_CONTEXT_CHARS.saturating_sub(used);
        let per_skill_limit = if name.starts_with("sec_skills ·") {
            MAX_LARGE_SKILL_CHARS.min(remaining)
        } else {
            remaining
        };
        let compacted = compact_skill_context(&body, per_skill_limit);
        used = used.saturating_add(compacted.chars().count());
        rendered.push(format!("## {name}\n{compacted}"));
    }
    if rendered.len() < row_count {
        rendered.push(
            "\n[其余 Skill 已省略：达到单次任务 Skill 上下文上限；请在任务中显式选择所需 Skill。]"
                .into(),
        );
    }
    Ok((names, rendered.join("\n\n")))
}

/// Keep large imported method packs useful without copying their entire
/// archive into every model request. Section headings and a bounded excerpt
/// are retained so the Agent can recognize the relevant method family; the
/// full text remains editable in the local Skill card.
fn compact_skill_context(body: &str, max_chars: usize) -> String {
    if body.chars().count() <= max_chars {
        return body.to_string();
    }
    let mut output = String::new();
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("##") {
            let heading = trimmed.to_string();
            if output.chars().count() + heading.chars().count() + 2 > max_chars {
                break;
            }
            output.push_str(&heading);
            output.push('\n');
            continue;
        }
        if output.chars().count() >= max_chars.saturating_sub(160) {
            break;
        }
        let remaining = max_chars.saturating_sub(output.chars().count() + 160);
        let excerpt = line.chars().take(remaining.min(720)).collect::<String>();
        if excerpt.trim().is_empty() {
            continue;
        }
        output.push_str(&excerpt);
        output.push('\n');
    }
    output.push_str(
        "\n[该 Skill 为本地大方法包；本次仅注入章节摘要，完整内容仍保存在 Oviraptor 本地。]",
    );
    output.chars().take(max_chars).collect()
}

fn default_web_strix_skill(connection: &rusqlite::Connection) -> Result<(String, String), String> {
    let mut statement = connection
        .prepare("SELECT id FROM strix_skills WHERE enabled=1 AND builtin=1 AND name='业务前端深度分析' ORDER BY id LIMIT 1")
        .map_err(|error| error.to_string())?;
    let ids = statement
        .query_map([], |row| row.get::<_, i64>(0))
        .map_err(|error| error.to_string())?
        .flatten()
        .collect::<Vec<_>>();
    strix_skill_instructions(connection, &ids)
}

fn strix_skill_row(row: &Row<'_>) -> rusqlite::Result<(String, String)> {
    Ok((row.get(0)?, row.get(1)?))
}

fn executable_works(candidate: &str, argument: &str) -> bool {
    !candidate.trim().is_empty()
        && Command::new(candidate)
            .arg(argument)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StrixCliCapabilities {
    version: String,
    mount_flag: bool,
    target_flag: bool,
    target_list_flag: bool,
    instruction_file_flag: bool,
    non_interactive_flag: bool,
    scan_mode_flag: bool,
    max_turns_flag: bool,
    max_budget_flag: Option<String>,
    scope_mode_flag: bool,
    diff_base_flag: bool,
}

fn strix_help_has_option(help: &str, option: &str) -> bool {
    help.split_whitespace().any(|token| {
        token.trim_matches(|value: char| {
            matches!(value, '[' | ']' | ',' | '(' | ')' | ':' | ';')
        }) == option
    })
}

fn parse_strix_cli_capabilities(help: &str, version: &str) -> Result<StrixCliCapabilities, String> {
    let capabilities = StrixCliCapabilities {
        version: version.trim().to_string(),
        mount_flag: strix_help_has_option(help, "--mount"),
        target_flag: strix_help_has_option(help, "--target"),
        target_list_flag: strix_help_has_option(help, "--target-list"),
        instruction_file_flag: strix_help_has_option(help, "--instruction-file"),
        non_interactive_flag: strix_help_has_option(help, "--non-interactive"),
        scan_mode_flag: strix_help_has_option(help, "--scan-mode"),
        max_turns_flag: strix_help_has_option(help, "--max-turns"),
        max_budget_flag: if strix_help_has_option(help, "--max-budget-usd") {
            Some("--max-budget-usd".into())
        } else if strix_help_has_option(help, "--max-budget") {
            Some("--max-budget".into())
        } else {
            None
        },
        scope_mode_flag: strix_help_has_option(help, "--scope-mode"),
        diff_base_flag: strix_help_has_option(help, "--diff-base"),
    };
    let mut missing = Vec::new();
    if !capabilities.target_flag && !capabilities.target_list_flag {
        missing.push("--target/--target-list");
    }
    if !capabilities.instruction_file_flag {
        missing.push("--instruction-file");
    }
    if !capabilities.non_interactive_flag {
        missing.push("--non-interactive");
    }
    if !capabilities.scan_mode_flag {
        missing.push("--scan-mode");
    }
    if !missing.is_empty() {
        return Err(format!(
            "当前 Strix CLI 不兼容 Oviraptor，缺少必要能力：{}；检测版本：{}",
            missing.join("、"),
            capabilities.version
        ));
    }
    Ok(capabilities)
}

static STRIX_CAPABILITY_CACHE: OnceLock<
    Mutex<HashMap<String, Result<StrixCliCapabilities, String>>>,
> = OnceLock::new();

fn strix_executable_cache_key(executable: &str) -> String {
    let configured = PathBuf::from(executable);
    let discovered = if configured.components().count() == 1 {
        std::env::var_os("PATH")
            .into_iter()
            .flat_map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
            .map(|directory| directory.join(executable))
            .find(|candidate| candidate.is_file())
            .unwrap_or(configured)
    } else {
        configured
    };
    let path = fs::canonicalize(&discovered).unwrap_or(discovered);
    let metadata = fs::metadata(&path).ok();
    let length = metadata.as_ref().map(|value| value.len()).unwrap_or(0);
    let modified = metadata
        .and_then(|value| value.modified().ok())
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    format!("{}|{length}|{modified}", path.to_string_lossy())
}

fn strix_cli_capabilities(executable: &str) -> Result<StrixCliCapabilities, String> {
    let cache_key = strix_executable_cache_key(executable);
    let cache = STRIX_CAPABILITY_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(guard) = cache.lock() {
        if let Some(cached) = guard.get(&cache_key) {
            return cached.clone();
        }
    }
    let inspect = || -> Result<StrixCliCapabilities, String> {
        let help = Command::new(executable)
            .arg("--help")
            .output()
            .map_err(|error| format!("无法检查 Strix CLI 能力：{error}"))?;
        let help_text = format!(
            "{}\n{}",
            String::from_utf8_lossy(&help.stdout),
            String::from_utf8_lossy(&help.stderr)
        );
        if help_text.trim().is_empty() {
            return Err("Strix --help 没有返回可解析的命令能力".into());
        }
        let version = Command::new(executable)
            .arg("--version")
            .output()
            .ok()
            .map(|output| {
                format!(
                    "{}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                )
            })
            .unwrap_or_else(|| "unknown".into());
        parse_strix_cli_capabilities(&help_text, &version)
    }();
    if let Ok(mut guard) = cache.lock() {
        guard.insert(cache_key, inspect.clone());
    }
    inspect
}

fn append_strix_local_directory(
    command: &mut Command,
    capabilities: &StrixCliCapabilities,
    directory: &Path,
) -> Result<&'static str, String> {
    if capabilities.mount_flag {
        command.arg("--mount").arg(directory);
        Ok("--mount")
    } else if capabilities.target_flag {
        command.arg("--target").arg(directory);
        Ok("--target")
    } else {
        Err(format!(
            "Strix {} 既不支持 --mount，也不支持使用 --target 传入本地证据目录",
            capabilities.version
        ))
    }
}

fn append_strix_budget(
    command: &mut Command,
    capabilities: &StrixCliCapabilities,
    budget: Option<f64>,
) {
    if let (Some(flag), Some(value)) = (capabilities.max_budget_flag.as_deref(), budget) {
        command.arg(flag).arg(format!("{value:.2}"));
    }
}

fn resolve_strix_executable(settings: &JsonValue, home: &Path) -> Result<String, String> {
    let configured = settings
        .get("strixExecutable")
        .and_then(JsonValue::as_str)
        .unwrap_or("")
        .trim();
    let mut candidates = Vec::new();
    if !configured.is_empty() {
        candidates.push(configured.to_string());
    }
    if cfg!(target_os = "windows") {
        candidates.extend([
            "strix.exe".into(),
            home.join(".strix/bin/strix.exe")
                .to_string_lossy()
                .into_owned(),
        ]);
    } else {
        candidates.extend([
            home.join(".strix/bin/strix").to_string_lossy().into_owned(),
            "strix".into(),
            "/opt/homebrew/bin/strix".into(),
            "/usr/local/bin/strix".into(),
        ]);
    }
    candidates
        .into_iter()
        .find(|candidate| executable_works(candidate, "--version"))
        .ok_or_else(|| "未找到可运行的 Strix；请在配置中心填写 Strix executable 完整路径".into())
}

fn resolve_plain_python(settings: &JsonValue, home: &Path) -> Result<String, String> {
    let configured = settings
        .get("pythonExecutable")
        .and_then(JsonValue::as_str)
        .unwrap_or("")
        .trim();
    let mut candidates = Vec::new();
    if !configured.is_empty() {
        candidates.push(configured.to_string());
    }
    candidates.extend([
        home.join(".pyenv/shims/python3")
            .to_string_lossy()
            .into_owned(),
        "python3".into(),
        "python".into(),
        "/opt/homebrew/bin/python3".into(),
        "/usr/local/bin/python3".into(),
    ]);
    candidates
        .into_iter()
        .find(|candidate| executable_works(candidate, "--version"))
        .ok_or_else(|| "未找到可运行的 Python；请在配置中心填写 Python executable 完整路径".into())
}

fn resolve_frontend_recon_worker(app: &AppHandle) -> Result<PathBuf, String> {
    let mut candidates = Vec::new();
    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(resource_dir.join("resources/workers/7_frontend_recon.py"));
        candidates.push(resource_dir.join("workers/7_frontend_recon.py"));
    }
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/workers/7_frontend_recon.py"),
    );
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| "应用内置前端侦察脚本 7_frontend_recon.py 缺失".into())
}

fn sentinel_runtime_path(home: &Path) -> OsString {
    let mut paths: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|value| std::env::split_paths(&value).collect())
        .unwrap_or_default();
    for path in [
        PathBuf::from("C:\\Program Files\\Docker\\Docker\\resources\\bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/Applications/Docker.app/Contents/Resources/bin"),
        home.join(".strix/bin"),
        home.join(".pyenv/shims"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
        PathBuf::from("/usr/sbin"),
        PathBuf::from("/sbin"),
    ] {
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    std::env::join_paths(paths).unwrap_or_else(|_| OsString::from("/usr/local/bin:/usr/bin:/bin"))
}

fn docker_candidates(home: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if cfg!(target_os = "windows") {
        candidates.extend([
            PathBuf::from("docker.exe"),
            PathBuf::from("C:\\Program Files\\Docker\\Docker\\resources\\bin\\docker.exe"),
        ]);
    } else {
        candidates.extend([
            PathBuf::from("/usr/local/bin/docker"),
            PathBuf::from("/opt/homebrew/bin/docker"),
            PathBuf::from("/Applications/Docker.app/Contents/Resources/bin/docker"),
            home.join(".docker/bin/docker"),
            PathBuf::from("docker"),
        ]);
    }
    candidates
}

fn ensure_docker_ready(home: &Path, runtime_path: &OsString) -> Result<PathBuf, String> {
    for candidate in docker_candidates(home) {
        let mut command = Command::new(&candidate);
        configure_child_command(&mut command);
        let spawned = command
            .args(["info", "--format", "{{.ServerVersion}}"])
            .env("PATH", runtime_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let Ok(mut child) = spawned else { continue };
        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(candidate),
                Ok(Some(_)) | Err(_) => break,
                Ok(None) if started.elapsed() >= Duration::from_secs(12) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
                Ok(None) => thread::sleep(Duration::from_millis(100)),
            }
        }
    }
    Err("Docker CLI 或 Docker daemon 不可用；请先启动 Docker Desktop，再确认 Sentinel 扫描".into())
}

fn sentinel_process_set(
    db_path: &Path,
    scan_id: &str,
    process_id: u32,
    engine: &str,
    work_dir: &Path,
) {
    if let Ok(connection) = db::open(db_path) {
        let _ = connection.execute(
            "INSERT INTO sentinel_processes(scan_id,process_id,engine,work_dir) VALUES(?1,?2,?3,?4) ON CONFLICT(scan_id,process_id) DO UPDATE SET engine=excluded.engine,work_dir=excluded.work_dir,started_at=datetime('now','localtime')",
            params![scan_id, process_id as i64, engine, work_dir.to_string_lossy()],
        );
    }
}

fn sentinel_process_clear(db_path: &Path, scan_id: &str, process_id: u32) {
    if let Ok(connection) = db::open(db_path) {
        let _ = connection.execute(
            "DELETE FROM sentinel_processes WHERE scan_id=?1 AND process_id=?2",
            params![scan_id, process_id as i64],
        );
    }
}

fn force_stop_registered_sentinel_processes(db_path: &Path, scan_id: &str) -> Vec<i64> {
    let process_ids = db::open(db_path)
        .ok()
        .and_then(|connection| {
            let mut statement = connection
                .prepare("SELECT process_id FROM sentinel_processes WHERE scan_id=?1")
                .ok()?;
            let rows = statement
                .query_map([scan_id], |row| row.get::<_, i64>(0))
                .ok()?;
            Some(rows.flatten().collect::<Vec<_>>())
        })
        .unwrap_or_default();
    for process_id in &process_ids {
        force_stop_sentinel_process(*process_id);
    }
    process_ids
}

fn sentinel_scan_update(db_path: &Path, scan_id: &str, status: &str, checkpoint: &str) {
    if let Ok(connection) = db::open(db_path) {
        let deleted = connection
            .query_row(
                "SELECT COUNT(*) FROM sentinel_deleted_scans WHERE scan_id=?1",
                [scan_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0);
        if deleted == 0 {
            let _ = connection.execute(
                "UPDATE sentinel_scans SET status=CASE WHEN status='pausing' AND ?1='scanning' THEN status ELSE ?1 END,current_checkpoint=CASE WHEN status='pausing' AND ?1='scanning' THEN current_checkpoint ELSE ?2 END,updated_at=datetime('now','localtime') WHERE id=?3",
                params![status, checkpoint, scan_id],
            );
            sync_sentinel_attempt(&connection, scan_id);
        }
    }
}

fn sentinel_scan_is_active(db_path: &Path, scan_id: &str) -> bool {
    let Ok(connection) = db::open(db_path) else {
        return false;
    };
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1 AND status IN ('scanning','pausing')) AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)",
            [scan_id],
            |row| row.get::<_, bool>(0),
        )
        .unwrap_or(false)
}

fn sentinel_scan_pause_requested(db_path: &Path, scan_id: &str) -> bool {
    let Ok(connection) = db::open(db_path) else {
        return false;
    };
    connection
        .query_row(
            "SELECT status='pausing' FROM sentinel_scans WHERE id=?1",
            [scan_id],
            |row| row.get::<_, bool>(0),
        )
        .unwrap_or(false)
}

fn sentinel_scan_is_paused(db_path: &Path, scan_id: &str) -> bool {
    let Ok(connection) = db::open(db_path) else {
        return false;
    };
    connection
        .query_row(
            "SELECT status IN ('pausing','paused') FROM sentinel_scans WHERE id=?1",
            [scan_id],
            |row| row.get::<_, bool>(0),
        )
        .unwrap_or(false)
}

fn finish_sentinel_pause(db_path: &Path, scan_id: &str, checkpoint: &str) {
    sentinel_scan_update(db_path, scan_id, "paused", checkpoint);
    if let Ok(connection) = db::open(db_path) {
        let _ = connection.execute("DELETE FROM sentinel_processes WHERE scan_id=?1", [scan_id]);
    }
}

#[cfg(test)]
fn strix_batch_size(settings: &JsonValue) -> usize {
    settings
        .get("strixBatchSize")
        .and_then(JsonValue::as_u64)
        .unwrap_or(15)
        .clamp(1, 50) as usize
}

fn setting_u64(settings: &JsonValue, key: &str, fallback: u64, min: u64, max: u64) -> u64 {
    settings
        .get(key)
        .and_then(JsonValue::as_u64)
        .unwrap_or(fallback)
        .clamp(min, max)
}

/// The configurable uncached-token budget uses zero to disable that one layer.
/// Absolute cumulative-context and loop fuses are applied per target later.
fn token_limit(settings: &JsonValue, key: &str, fallback: u64, min: u64, max: u64) -> i64 {
    let Some(value) = settings.get(key) else {
        return fallback as i64;
    };
    if value.as_i64() == Some(0) || value.as_u64() == Some(0) {
        return 0;
    }
    value
        .as_u64()
        .map(|number| number.clamp(min, max) as i64)
        .unwrap_or(fallback as i64)
}

#[derive(Clone, Debug)]
struct AdaptiveStrixSettings {
    quick_score: i64,
    standard_score: i64,
    deep_score: i64,
    quick_timeout: u64,
    standard_timeout: u64,
    deep_timeout: u64,
    quick_tokens: i64,
    standard_tokens: i64,
    deep_tokens: i64,
    quick_requests: i64,
    standard_requests: i64,
    deep_requests: i64,
    no_tool_turn_limit: i64,
    max_mode: String,
    max_budget_usd: Option<f64>,
}

impl AdaptiveStrixSettings {
    fn from_json(settings: &JsonValue) -> Self {
        let quick_score = setting_u64(settings, "strixQuickScore", 30, 1, 90) as i64;
        let standard_score = setting_u64(
            settings,
            "strixStandardScore",
            55,
            (quick_score + 1) as u64,
            95,
        ) as i64;
        let deep_score = setting_u64(
            settings,
            "strixDeepScore",
            80,
            (standard_score + 1) as u64,
            100,
        ) as i64;
        Self {
            quick_score,
            standard_score,
            deep_score,
            quick_timeout: setting_u64(settings, "strixQuickTimeout", 120, 30, 3600),
            standard_timeout: setting_u64(settings, "strixStandardTimeout", 300, 60, 7200),
            deep_timeout: setting_u64(settings, "strixDeepTimeout", 600, 120, 14400),
            quick_tokens: token_limit(settings, "strixQuickTokenLimit", 50_000, 10_000, 10_000_000),
            standard_tokens: token_limit(
                settings,
                "strixStandardTokenLimit",
                120_000,
                20_000,
                20_000_000,
            ),
            deep_tokens: token_limit(settings, "strixDeepTokenLimit", 250_000, 50_000, 50_000_000),
            quick_requests: setting_u64(settings, "strixQuickRequestLimit", 4, 1, 100) as i64,
            standard_requests: setting_u64(settings, "strixStandardRequestLimit", 8, 1, 200) as i64,
            deep_requests: setting_u64(settings, "strixDeepRequestLimit", 12, 1, 300) as i64,
            no_tool_turn_limit: setting_u64(settings, "strixNoToolTurnLimit", 4, 1, 100) as i64,
            max_mode: "deep".into(),
            max_budget_usd: None,
        }
    }

    fn apply_web_policy(&mut self, policy: &JsonValue) {
        self.max_mode = match policy.get("webModeCeiling").and_then(JsonValue::as_str) {
            Some("quick") => "quick",
            Some("deep") => "deep",
            _ => "standard",
        }
        .into();
        self.max_budget_usd = policy
            .get("maxBudgetUsd")
            .and_then(JsonValue::as_f64)
            .filter(|value| *value > 0.0 && *value <= 10_000.0);
    }

    fn bounded_mode(&self, mode: &str) -> String {
        let rank = |value: &str| match value {
            "deep" => 3,
            "standard" => 2,
            "quick" => 1,
            _ => 0,
        };
        if rank(mode) > rank(&self.max_mode) {
            self.max_mode.clone()
        } else {
            mode.to_string()
        }
    }

    fn mode_for_score(&self, score: i64) -> &'static str {
        if score < self.quick_score {
            "skip"
        } else if score < self.standard_score {
            "quick"
        } else if score < self.deep_score {
            "standard"
        } else {
            "deep"
        }
    }

    fn limits(&self, mode: &str) -> (u64, i64, i64) {
        match mode {
            "deep" => (self.deep_timeout, self.deep_tokens, self.deep_requests),
            "standard" => (
                self.standard_timeout,
                self.standard_tokens,
                self.standard_requests,
            ),
            _ => (self.quick_timeout, self.quick_tokens, self.quick_requests),
        }
    }
}
