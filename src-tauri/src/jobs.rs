use crate::{
    commands, db,
    models::{ImportResult, JobProgressEvent, StartJobInput},
    AppState,
};
use rusqlite::{params, OptionalExtension};
use serde_json::{Map, Value};
use std::{
    collections::{hash_map::DefaultHasher, HashMap, VecDeque},
    fs::{self, File},
    hash::{Hash, Hasher},
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc, Arc,
    },
    thread,
    time::Duration,
};
use tauri::{image::Image, AppHandle, Emitter, Manager, State};
use uuid::Uuid;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[cfg(windows)]
fn configure_child_command(command: &mut Command) {
    use std::os::windows::process::CommandExt;

    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn configure_child_command(_command: &mut Command) {}

#[cfg(windows)]
fn set_job_power_request(active: bool) {
    const ES_CONTINUOUS: u32 = 0x80000000;
    const ES_SYSTEM_REQUIRED: u32 = 0x00000001;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetThreadExecutionState(execution_state: u32) -> u32;
    }

    let execution_state = if active {
        ES_CONTINUOUS | ES_SYSTEM_REQUIRED
    } else {
        ES_CONTINUOUS
    };
    unsafe {
        let _ = SetThreadExecutionState(execution_state);
    }
}

#[cfg(not(windows))]
fn set_job_power_request(_active: bool) {}

fn remember_process_output(recent: &mut VecDeque<String>, level: &str, line: String) {
    let line = line.trim().to_string();
    if line.is_empty() {
        return;
    }
    if recent.len() >= 40 {
        recent.pop_front();
    }
    recent.push_back(format!("[{level}] {line}"));
}

fn setting_str(settings: &Value, key: &str, default: &str) -> String {
    settings
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or(default)
        .to_string()
}

fn setting_bool(settings: &Value, key: &str, default: bool) -> bool {
    settings
        .get(key)
        .and_then(Value::as_bool)
        .unwrap_or(default)
}

fn setting_i64(settings: &Value, key: &str, default: i64) -> i64 {
    settings.get(key).and_then(Value::as_i64).unwrap_or(default)
}

fn setting_f64(settings: &Value, key: &str, default: f64) -> f64 {
    settings.get(key).and_then(Value::as_f64).unwrap_or(default)
}

fn resolve_python_with_pandas(configured: &str) -> Result<String, String> {
    let mut candidates = Vec::new();
    if !configured.trim().is_empty() {
        candidates.push(configured.trim().to_string());
    }
    for candidate in [
        "python3",
        "python",
        "/opt/homebrew/bin/python3",
        "/usr/local/bin/python3",
    ] {
        if !candidates.iter().any(|item| item == candidate) {
            candidates.push(candidate.to_string());
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        let pyenv = PathBuf::from(home).join(".pyenv");
        candidates.push(
            pyenv
                .parent()
                .unwrap_or_else(|| Path::new("/"))
                .join("oviraptor/runtime/python/bin/python3")
                .to_string_lossy()
                .to_string(),
        );
        for candidate in [pyenv.join("shims/python3"), pyenv.join("shims/python")] {
            candidates.push(candidate.to_string_lossy().to_string());
        }
        if let Ok(entries) = fs::read_dir(pyenv.join("versions")) {
            for entry in entries.flatten() {
                candidates.push(
                    entry
                        .path()
                        .join("bin/python3")
                        .to_string_lossy()
                        .to_string(),
                );
            }
        }
    }
    for candidate in &candidates {
        let mut command = Command::new(candidate);
        configure_child_command(&mut command);
        let result = command
            .args(["-c", "import pandas; print(pandas.__version__)"])
            .output();
        if result.as_ref().is_ok_and(|output| output.status.success()) {
            return Ok(candidate.clone());
        }
    }
    Err(format!("找不到可导入 pandas 的 Python 解释器。当前配置为 `{}`；请在配置中心把 Python executable 改为安装 pandas 的解释器完整路径，例如虚拟环境中的 bin/python。", configured))
}

fn ini_value(value: String) -> String {
    value.replace(['\r', '\n'], " ").trim().to_string()
}

fn secure_file(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn resolve_scripts_dir(app: &AppHandle, settings: &Value) -> Result<PathBuf, String> {
    let override_dir = setting_str(settings, "scriptsDirectory", "");
    if !override_dir.trim().is_empty() {
        let path = PathBuf::from(override_dir.trim());
        if path.join("1_collect_info.py").exists() {
            return Ok(path);
        }
        return Err(format!(
            "外部脚本目录无效，未找到 1_collect_info.py：{}；清空 Scripts directory 可使用内置脚本",
            path.display()
        ));
    }

    let mut candidates = Vec::new();
    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(resource_dir.join("resources").join("workers"));
        candidates.push(resource_dir.join("workers"));
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/workers"));
    for candidate in &candidates {
        if candidate.join("1_collect_info.py").exists() {
            return Ok(candidate.clone());
        }
    }
    Err(format!(
        "应用内置采集脚本缺失；已检查：{}",
        candidates
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("、")
    ))
}

fn write_runtime_config(path: &Path, settings: &Value) -> Result<(), String> {
    let key = ini_value(setting_str(settings, "fofaKey", ""));
    let legacy_path = PathBuf::from(setting_str(settings, "configPath", ""));
    if key.is_empty() {
        if legacy_path.exists() {
            fs::copy(&legacy_path, path).map_err(|error| {
                format!("无法复制兼容配置 {}：{}", legacy_path.display(), error)
            })?;
            secure_file(path)?;
            return Ok(());
        }
        return Err(
            "未配置 FOFA Key；请在配置中心填写 FOFA account / key，或指定已有 Config path".into(),
        );
    }

    let email = ini_value(setting_str(settings, "fofaEmail", ""));
    let content = format!(
        "[fofa]\nemail = {email}\nkey = {key}\n\n[collection]\nmode = {}\nprofile = {}\npage_size = {}\nmax_pages = {}\ninterval = {}\ntimeout = {}\nfull = {}\nenable_cidr24 = {}\nmax_cidrs = {}\nmax_fingerprints = {}\nmax_derived_domains = {}\ninclude_weak_fingerprints = {}\nauto_expand_min_score = {}\ncache = {}\n\n[probe]\npriority_rate = {}\nother_rate = {}\nper_host_interval = {}\nworkers = {}\ntimeout = {}\nretries = {}\nmax_body_bytes = {}\nbatch_size = {}\ncache_hours = {}\ncontent_threshold = {}\ninclude_other = {}\ninclude_weak = {}\nallow_private = {}\nstrict_tls = {}\nscheme_fallback = {}\n",
        setting_str(settings, "collectionMode", "all"),
        setting_str(settings, "fofaProfile", "professional"),
        setting_i64(settings, "pageSize", 500),
        setting_i64(settings, "maxPages", 0),
        setting_f64(settings, "interval", 6.0),
        setting_i64(settings, "collectionTimeout", 45),
        setting_bool(settings, "fullHistory", false),
        setting_bool(settings, "enableCidr24", false),
        setting_i64(settings, "maxCidrs", 0),
        setting_i64(settings, "maxFingerprints", 0),
        setting_i64(settings, "maxDerivedDomains", 200),
        setting_bool(settings, "includeWeakFingerprints", false),
        setting_i64(settings, "autoExpandMinScore", 85),
        setting_bool(settings, "collectionCache", true),
        setting_f64(settings, "priorityRate", 20.0),
        setting_f64(settings, "otherRate", 10.0),
        setting_f64(settings, "perHostInterval", 1.5),
        setting_i64(settings, "workers", 64),
        setting_i64(settings, "probeTimeout", 6),
        setting_i64(settings, "probeRetries", 0),
        setting_i64(settings, "maxBodyBytes", 524_288),
        setting_i64(settings, "batchSize", 200),
        setting_i64(settings, "cacheHours", 24),
        setting_i64(settings, "contentThreshold", 12),
        setting_bool(settings, "includeOther", true),
        setting_bool(settings, "includeWeak", false),
        setting_bool(settings, "allowPrivate", false),
        setting_bool(settings, "strictTls", false),
        setting_bool(settings, "schemeFallback", true),
    );
    fs::write(path, content).map_err(|error| error.to_string())?;
    secure_file(path)
}

fn log_line(db_path: &Path, run_id: i64, level: &str, stage: &str, message: &str) {
    if let Ok(connection) = db::open(db_path) {
        let sanitized = message.replace("FOFA_KEY", "FOFA_***");
        let _ = connection.execute(
            "INSERT INTO logs(run_id,level,stage,message) VALUES(?1,?2,?3,?4)",
            params![run_id, level, stage, sanitized],
        );
    }
}

fn log_lines(db_path: &Path, run_id: i64, stage: &str, lines: &[(String, String)]) {
    if lines.is_empty() {
        return;
    }
    if let Ok(mut connection) = db::open(db_path) {
        if let Ok(transaction) = connection.transaction() {
            for (level, message) in lines {
                let sanitized = message.replace("FOFA_KEY", "FOFA_***");
                let _ = transaction.execute(
                    "INSERT INTO logs(run_id,level,stage,message) VALUES(?1,?2,?3,?4)",
                    params![run_id, level, stage, sanitized],
                );
            }
            let _ = transaction.commit();
        }
    }
}

fn set_progress(
    app: &AppHandle,
    db_path: &Path,
    run_id: i64,
    status: &str,
    stage: &str,
    progress: f64,
    message: &str,
) {
    if let Ok(connection) = db::open(db_path) {
        let _ = connection.execute(
            "UPDATE runs SET status=?1,stage=?2,progress=?3,started_at=COALESCE(started_at,datetime('now','localtime')) WHERE id=?4",
            params![status, stage, progress, run_id],
        );
    }
    let _ = app.emit(
        "job-progress",
        JobProgressEvent {
            run_id,
            status: status.into(),
            stage: stage.into(),
            progress,
            message: message.into(),
        },
    );
}

fn spinner_icon(frame: usize) -> Image<'static> {
    let size = 18u32;
    let center = (size as f64 - 1.0) / 2.0;
    let mut pixels = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let dx = x as f64 - center;
            let dy = y as f64 - center;
            let radius = (dx * dx + dy * dy).sqrt();
            if (5.1..=7.6).contains(&radius) {
                let angle =
                    (dy.atan2(dx) + std::f64::consts::PI * 2.0) % (std::f64::consts::PI * 2.0);
                let segment =
                    ((angle / (std::f64::consts::PI * 2.0) * 12.0).floor() as usize + frame) % 12;
                let alpha = 55 + ((11 - segment) * 18) as u8;
                let offset = ((y * size + x) * 4) as usize;
                pixels[offset] = 35;
                pixels[offset + 1] = 111;
                pixels[offset + 2] = 237;
                pixels[offset + 3] = alpha;
            }
        }
    }
    Image::new_owned(pixels, size, size)
}

fn restore_tray_icon(app: &AppHandle) {
    let icon = Image::from_bytes(include_bytes!("../icons/tray-icon.png"));
    if let (Some(tray), Ok(icon)) = (app.tray_by_id("main"), icon) {
        let _ = tray.set_icon(Some(icon));
    }
}

fn begin_tray_activity(app: AppHandle, active_jobs: Arc<AtomicUsize>) {
    if active_jobs.fetch_add(1, Ordering::SeqCst) != 0 {
        return;
    }
    thread::spawn(move || {
        let mut index = 0usize;
        while active_jobs.load(Ordering::SeqCst) > 0 {
            if let Some(tray) = app.tray_by_id("main") {
                let _ = tray.set_icon(Some(spinner_icon(index)));
                let _ = tray.set_title(Some("资产更新中"));
                let _ = tray.set_tooltip(Some("Oviraptor · 正在更新资产"));
            }
            index += 1;
            thread::sleep(Duration::from_millis(420));
        }
        if let Some(tray) = app.tray_by_id("main") {
            let _ = tray.set_title(None::<&str>);
            let _ = tray.set_tooltip(Some("Oviraptor · 后台运行"));
        }
        restore_tray_icon(&app);
    });
}

fn run_process(
    app: &AppHandle,
    db_path: &Path,
    run_id: i64,
    stage: &str,
    progress: f64,
    program: &str,
    args: &[String],
    cwd: &Path,
    cancel: &AtomicBool,
    proxy_url: &str,
    no_proxy: &str,
) -> Result<(), String> {
    log_line(
        db_path,
        run_id,
        "info",
        stage,
        &format!("启动：{} {}", program, args.join(" ")),
    );
    let mut command = Command::new(program);
    configure_child_command(&mut command);
    command
        .args(args)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("PYTHONUNBUFFERED", "1");
    if !proxy_url.trim().is_empty() {
        command
            .env("HTTP_PROXY", proxy_url)
            .env("HTTPS_PROXY", proxy_url)
            .env("ALL_PROXY", proxy_url);
    }
    if !no_proxy.trim().is_empty() {
        command.env("NO_PROXY", no_proxy);
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("无法启动 {}：{}", program, error))?;

    let (sender, receiver) = mpsc::channel::<(String, String)>();
    if let Some(stdout) = child.stdout.take() {
        let sender = sender.clone();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let _ = sender.send(("info".into(), line));
            }
        });
    }
    if let Some(stderr) = child.stderr.take() {
        let sender = sender.clone();
        thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let _ = sender.send(("warning".into(), line));
            }
        });
    }
    drop(sender);

    let mut recent_output = VecDeque::new();
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("__CANCELLED__".into());
        }
        let mut pending_logs = Vec::new();
        while let Ok((level, line)) = receiver.try_recv() {
            remember_process_output(&mut recent_output, &level, line.clone());
            pending_logs.push((level.clone(), line.clone()));
            let _ = app.emit(
                "job-progress",
                JobProgressEvent {
                    run_id,
                    status: "running".into(),
                    stage: stage.into(),
                    progress,
                    message: line,
                },
            );
        }
        log_lines(db_path, run_id, stage, &pending_logs);
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            let mut final_logs = Vec::new();
            while let Ok((level, line)) = receiver.try_recv() {
                remember_process_output(&mut recent_output, &level, line.clone());
                final_logs.push((level, line));
            }
            log_lines(db_path, run_id, stage, &final_logs);
            if status.success() {
                return Ok(());
            }
            let exit_code = status
                .code()
                .map(|code| format!("exit code {code}"))
                .unwrap_or_else(|| format!("{status}"));
            let output = if recent_output.is_empty() {
                "未捕获到子进程输出；请查看任务目录中的阶段日志".to_string()
            } else {
                format!(
                    "最近输出：{}",
                    recent_output.into_iter().collect::<Vec<_>>().join("\n")
                )
            };
            return Err(format!("{stage} 阶段失败：{exit_code}；{output}"));
        }
        thread::sleep(Duration::from_millis(180));
    }
}

fn write_seeds(path: &Path, targets: &[(String, String)]) -> Result<(), String> {
    let mut file = File::create(path).map_err(|error| error.to_string())?;
    use std::io::Write;
    file.write_all(&[0xEF, 0xBB, 0xBF])
        .map_err(|error| error.to_string())?;
    let mut writer = csv::Writer::from_writer(file);
    writer
        .write_record([
            "company",
            "names",
            "aliases",
            "domains",
            "icps",
            "ip_ranges",
            "asn_orgs",
            "keywords",
        ])
        .map_err(|error| error.to_string())?;
    for (target_type, value) in targets {
        let mut record = vec![
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
        ];
        match target_type.as_str() {
            "company" => {
                record[0] = value.clone();
                record[1] = value.clone();
            }
            "domain" => record[3] = value.clone(),
            "ip" | "cidr" => record[5] = value.clone(),
            "icp" => record[4] = value.clone(),
            "asn" => record[6] = value.clone(),
            "keyword" => record[7] = value.clone(),
            other => return Err(format!("目标类型不受支持：{other}")),
        }
        writer
            .write_record(record)
            .map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|error| error.to_string())
}

fn write_content_rules(path: &Path, settings: &Value, db_path: &Path) -> Result<(), String> {
    let dynamic_keywords = if let Ok(connection) = db::open(db_path) {
        let mut keywords = Vec::new();
        if let Ok(mut statement) =
            connection.prepare("SELECT keyword FROM content_rules WHERE enabled=1 ORDER BY id")
        {
            if let Ok(rows) = statement.query_map([], |row| row.get::<_, String>(0)) {
                keywords.extend(rows.filter_map(Result::ok));
            }
        }
        keywords
    } else {
        Vec::new()
    };
    let rules = serde_json::json!({
        "version": 2,
        "replaceDefaults": settings.get("replaceDefaultContentRules").and_then(Value::as_bool).unwrap_or(false),
        "gamblingKeywords": settings.get("gamblingKeywords").cloned().unwrap_or_else(|| Value::Array(Vec::new())),
        "pornKeywords": settings.get("pornKeywords").cloned().unwrap_or_else(|| Value::Array(Vec::new())),
        "negativeKeywords": settings.get("negativeKeywords").cloned().unwrap_or_else(|| Value::Array(Vec::new())),
        "customKeywords": dynamic_keywords,
    });
    let encoded = serde_json::to_vec_pretty(&rules).map_err(|error| error.to_string())?;
    fs::write(path, encoded).map_err(|error| error.to_string())
}

fn field<'a>(
    record: &'a csv::StringRecord,
    index: &HashMap<String, usize>,
    names: &[&str],
) -> &'a str {
    for name in names {
        if let Some(position) = index.get(*name) {
            let value = record.get(*position).unwrap_or("").trim();
            if !value.is_empty() {
                return value;
            }
        }
    }
    ""
}

fn canonical_identity(
    link: &str,
    host: &str,
    protocol: &str,
    ip: &str,
    port: &str,
    fallback: &str,
) -> String {
    let endpoint = if !link.trim().is_empty() {
        link
    } else if !host.trim().is_empty() {
        host
    } else if !ip.trim().is_empty() || !port.trim().is_empty() {
        return format!(
            "{}|{}|{}",
            protocol.trim().to_lowercase(),
            ip.trim().to_lowercase(),
            port.trim()
        );
    } else {
        fallback
    };
    endpoint.trim().trim_end_matches('/').to_lowercase()
}

fn import_csv(
    app: &AppHandle,
    db_path: &Path,
    run_id: i64,
    project_id: i64,
    csv_path: &Path,
    stage_progress: f64,
) -> Result<ImportResult, String> {
    if !csv_path.exists() {
        return Err(format!("结果文件不存在：{}", csv_path.display()));
    }
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .from_path(csv_path)
        .map_err(|error| error.to_string())?;
    let headers = reader.headers().map_err(|error| error.to_string())?.clone();
    let index = headers
        .iter()
        .enumerate()
        .map(|(position, name)| (name.to_string(), position))
        .collect::<HashMap<_, _>>();
    let mut connection = db::open(db_path)?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    let content_rules = {
        let mut statement = transaction
            .prepare("SELECT normalized_keyword FROM content_rules WHERE enabled=1 ORDER BY id")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?
    };
    let mut inserted = 0i64;
    let mut updated = 0i64;
    let mut invalid = 0i64;

    for (offset, row) in reader.records().enumerate() {
        let record = match row {
            Ok(value) => value,
            Err(_) => {
                invalid += 1;
                continue;
            }
        };
        let company = field(&record, &index, &["company"]);
        let source_key = field(&record, &index, &["asset_key"]);
        let host = field(&record, &index, &["host"]);
        let link = field(&record, &index, &["link"]);
        let ip = field(&record, &index, &["ip"]);
        let port = field(&record, &index, &["port"]);
        let protocol = field(&record, &index, &["protocol"]);
        let fallback = format!("{}|{}|{}|{}", link, host, ip, port);
        let raw_key = if source_key.is_empty() {
            fallback.as_str()
        } else {
            source_key
        };
        if raw_key.trim_matches('|').is_empty() {
            invalid += 1;
            continue;
        }
        let canonical_key = canonical_identity(link, host, protocol, ip, port, raw_key);
        let existing_asset_key: Option<String> = transaction
            .query_row(
                "SELECT a.asset_key FROM project_assets pa JOIN assets a ON a.id=pa.asset_id WHERE pa.project_id=?1 AND a.canonical_key=?2 AND pa.is_deleted=0 ORDER BY pa.asset_id LIMIT 1",
                params![project_id, canonical_key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let asset_key =
            existing_asset_key.unwrap_or_else(|| format!("{}\u{1f}{}", company, raw_key));
        let domain = field(&record, &index, &["domain"]);
        let title = field(&record, &index, &["probe_title", "title"]);
        let status_code = field(&record, &index, &["probe_status_code", "status_code"]);
        let probe_outcome = field(&record, &index, &["probe_outcome"]);
        let probe_entry_state = field(&record, &index, &["probe_entry_state"]);
        let review_tier = field(&record, &index, &["review_tier"]);
        let content_category = field(&record, &index, &["content_category"]);
        let normalized_title = title.to_lowercase();
        let matched_content_rule = content_rules
            .iter()
            .find(|keyword| !keyword.is_empty() && normalized_title.contains(keyword.as_str()));
        let effective_content_category = if matched_content_rule.is_some() {
            "custom_rule"
        } else {
            content_category
        };
        let score = field(&record, &index, &["score"]);

        let mut extra = Map::new();
        for (position, name) in headers.iter().enumerate() {
            let value = record.get(position).unwrap_or("");
            if !value.is_empty() {
                extra.insert(name.to_string(), Value::String(value.to_string()));
            }
        }
        let extra_json = Value::Object(extra).to_string();
        let is_probe = index.contains_key("probe_outcome");
        let mut hasher = DefaultHasher::new();
        let hash_values: Vec<&str> = if is_probe {
            vec![
                probe_outcome,
                probe_entry_state,
                status_code,
                title,
                effective_content_category,
            ]
        } else {
            vec![
                host,
                link,
                ip,
                port,
                protocol,
                domain,
                title,
                status_code,
                review_tier,
                score,
            ]
        };
        for value in hash_values {
            value.hash(&mut hasher);
        }
        let state_hash = format!("{:016x}", hasher.finish());
        let previous: Option<(i64, String)> = transaction
            .query_row(
                if is_probe {
                    "SELECT id,probe_hash FROM assets WHERE asset_key=?1"
                } else {
                    "SELECT id,state_hash FROM assets WHERE asset_key=?1"
                },
                [&asset_key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let was_associated = if let Some((asset_id, _)) = &previous {
            transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM project_assets WHERE project_id=?1 AND asset_id=?2)",
                params![project_id, asset_id], |row| row.get::<_, i64>(0),
            ).map_err(|error| error.to_string())? != 0
        } else {
            false
        };

        transaction.execute(r#"
            INSERT INTO assets(asset_key,company,host,link,ip,port,protocol,domain,title,status_code,probe_outcome,probe_entry_state,review_tier,content_category,score,state_hash,probe_hash,canonical_key,extra_json,last_alive)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,CASE WHEN ?11 IN ('web_alive','web_restricted','browser_render_required','virtual_host_required','alive_clean') THEN datetime('now','localtime') ELSE NULL END)
            ON CONFLICT(asset_key) DO UPDATE SET company=CASE WHEN assets.company='' THEN excluded.company ELSE assets.company END,host=excluded.host,link=excluded.link,ip=excluded.ip,port=excluded.port,
              protocol=excluded.protocol,domain=excluded.domain,
              title=CASE WHEN excluded.probe_hash<>'' OR assets.probe_outcome='' THEN excluded.title ELSE assets.title END,
              status_code=CASE WHEN excluded.probe_hash<>'' OR assets.probe_outcome='' THEN excluded.status_code ELSE assets.status_code END,
              probe_outcome=CASE WHEN excluded.probe_outcome='' THEN assets.probe_outcome ELSE excluded.probe_outcome END,
              probe_entry_state=CASE WHEN excluded.probe_entry_state='' THEN assets.probe_entry_state ELSE excluded.probe_entry_state END,
              review_tier=CASE WHEN excluded.review_tier='' THEN assets.review_tier ELSE excluded.review_tier END,
              content_category=CASE WHEN excluded.content_category='' THEN assets.content_category ELSE excluded.content_category END,
              score=CASE WHEN excluded.score='' THEN assets.score ELSE excluded.score END,
              state_hash=CASE WHEN excluded.state_hash='' THEN assets.state_hash ELSE excluded.state_hash END,
              probe_hash=CASE WHEN excluded.probe_hash='' THEN assets.probe_hash ELSE excluded.probe_hash END,
              canonical_key=excluded.canonical_key,
              extra_json=json_patch(assets.extra_json,excluded.extra_json),last_seen=datetime('now','localtime'),
              last_alive=CASE WHEN excluded.probe_outcome IN ('web_alive','web_restricted','browser_render_required','virtual_host_required','alive_clean') THEN datetime('now','localtime') ELSE assets.last_alive END
        "#, params![asset_key,company,host,link,ip,port,protocol,domain,title,status_code,probe_outcome,probe_entry_state,review_tier,effective_content_category,score,
            if is_probe { "" } else { state_hash.as_str() }, if is_probe { state_hash.as_str() } else { "" }, canonical_key, extra_json])
            .map_err(|error| error.to_string())?;
        let asset_id: i64 = transaction
            .query_row(
                "SELECT id FROM assets WHERE asset_key=?1",
                [&asset_key],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        transaction.execute(r#"
            INSERT INTO project_assets(project_id,asset_id,last_run_id) VALUES(?1,?2,?3)
            ON CONFLICT(project_id,asset_id) DO UPDATE SET last_seen=datetime('now','localtime'),last_run_id=excluded.last_run_id
        "#, params![project_id,asset_id,run_id]).map_err(|error| error.to_string())?;

        let auto_excluded = field(&record, &index, &["auto_excluded"]) == "1";
        let exclude_reason = field(&record, &index, &["exclude_reason"]);
        if auto_excluded {
            transaction.execute(
                "UPDATE project_assets SET decision='rejected',note=?1,is_deleted=1,deleted_at=datetime('now','localtime') WHERE project_id=?2 AND asset_id=?3",
                params![format!("系统自动SRC清洗：{}", exclude_reason),project_id,asset_id],
            ).map_err(|error| error.to_string())?;
        } else if let Some(keyword) = matched_content_rule {
            transaction.execute(
                "UPDATE project_assets SET decision=CASE WHEN decision IN ('pending','uncertain','','not_applicable') THEN 'rejected' ELSE decision END,note=CASE WHEN decision IN ('pending','uncertain','','not_applicable','rejected') THEN ?1 ELSE note END,is_deleted=0,deleted_at=NULL WHERE project_id=?2 AND asset_id=?3",
                params![format!("系统自动内容规则：标题包含「{}」", keyword),project_id,asset_id],
            ).map_err(|error| error.to_string())?;
        } else if is_probe {
            if matches!(
                probe_outcome,
                "tcp_alive_non_http" | "web_abnormal" | "unreachable" | "skipped"
            ) {
                let classification = match probe_outcome {
                    "tcp_alive_non_http" => "TCP端口开放，但未识别到HTTP服务",
                    "web_abnormal" => "存在HTTP响应，但入口异常或无有效页面",
                    "unreachable" => "当前无法建立Web连接",
                    _ => "目标格式或网络范围不适合Web探测",
                };
                transaction.execute(
                    "UPDATE project_assets SET decision='not_applicable',note=?1,is_deleted=0,deleted_at=NULL WHERE project_id=?2 AND asset_id=?3 AND decision IN ('pending','uncertain','','not_applicable')",
                    params![format!("系统自动Web分类：{}",classification),project_id,asset_id],
                ).map_err(|error| error.to_string())?;
            } else if probe_outcome == "blocked_content" {
                transaction.execute(
                    "UPDATE project_assets SET decision='rejected',note='系统自动Web分类：违规内容隔离',is_deleted=0,deleted_at=NULL WHERE project_id=?1 AND asset_id=?2 AND decision IN ('pending','uncertain','','rejected')",
                    params![project_id,asset_id],
                ).map_err(|error| error.to_string())?;
            } else if matches!(
                probe_outcome,
                "web_alive"
                    | "web_restricted"
                    | "browser_render_required"
                    | "virtual_host_required"
            ) {
                transaction.execute(
                    "UPDATE project_assets SET decision='pending',note='',is_deleted=0,deleted_at=NULL WHERE project_id=?1 AND asset_id=?2 AND (note LIKE '系统自动SRC清洗：%' OR note LIKE '系统自动Web分类：%')",
                    params![project_id,asset_id],
                ).map_err(|error| error.to_string())?;
            }
        }

        if !was_associated {
            transaction.execute(
                "INSERT INTO asset_events(project_id,asset_id,run_id,event_type,summary) VALUES(?1,?2,?3,'new','首次在项目中发现')",
                params![project_id,asset_id,run_id],
            ).map_err(|error| error.to_string())?;
            inserted += 1;
        } else if previous
            .as_ref()
            .is_some_and(|(_, old_hash)| old_hash != &state_hash)
        {
            transaction.execute(
                "INSERT INTO asset_events(project_id,asset_id,run_id,event_type,summary) VALUES(?1,?2,?3,'changed','资产关键字段发生变化')",
                params![project_id,asset_id,run_id],
            ).map_err(|error| error.to_string())?;
            updated += 1;
        }
        if offset > 0 && offset % 2000 == 0 {
            let _ = app.emit(
                "job-progress",
                JobProgressEvent {
                    run_id,
                    status: "running".into(),
                    stage: "import".into(),
                    progress: stage_progress,
                    message: format!("已导入 {} 条", offset),
                },
            );
        }
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(ImportResult {
        inserted,
        updated,
        invalid,
    })
}

fn mark_missing(db_path: &Path, project_id: i64, run_id: i64) -> Result<(), String> {
    let connection = db::open(db_path)?;
    connection.execute(r#"
        INSERT INTO asset_events(project_id,asset_id,run_id,event_type,summary)
        SELECT pa.project_id,pa.asset_id,?2,'missing','本次完整采集未再次发现；不等同于资产已下线'
        FROM project_assets pa
        WHERE pa.project_id=?1 AND pa.is_deleted=0 AND COALESCE(pa.last_run_id,0)<>?2
          AND NOT EXISTS(SELECT 1 FROM asset_events e WHERE e.run_id=?2 AND e.asset_id=pa.asset_id AND e.event_type='missing')
    "#, params![project_id,run_id]).map_err(|error| error.to_string())?;
    Ok(())
}

fn write_reprobe_sources(
    db_path: &Path,
    project_id: i64,
    refined_dir: &Path,
) -> Result<i64, String> {
    fs::create_dir_all(refined_dir).map_err(|error| error.to_string())?;
    let primary = refined_dir.join("P1_active_strong.csv");
    // 中断后必须保留原始文件的大小和 mtime，探测脚本才能沿用 checkpoint。
    if primary.exists() {
        let count = BufReader::new(File::open(&primary).map_err(|error| error.to_string())?)
            .lines()
            .count()
            .saturating_sub(1) as i64;
        return Ok(count);
    }

    let headers = [
        "review_tier",
        "company",
        "asset_key",
        "host",
        "link",
        "ip",
        "port",
        "protocol",
        "domain",
        "title",
        "status_code",
        "score",
    ];
    let connection = db::open(db_path)?;
    let mut statement = connection
        .prepare(
            r#"SELECT a.review_tier,a.company,a.asset_key,a.host,a.link,a.ip,a.port,
                      a.protocol,a.domain,a.title,a.status_code,a.score
               FROM project_assets pa JOIN assets a ON a.id=pa.asset_id
               WHERE pa.project_id=?1 AND pa.is_deleted=0
               ORDER BY pa.asset_id"#,
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([project_id], |row| {
            Ok((0..headers.len())
                .map(|index| row.get::<_, String>(index).unwrap_or_default())
                .collect::<Vec<_>>())
        })
        .map_err(|error| error.to_string())?;
    let mut writer = csv::Writer::from_path(&primary).map_err(|error| error.to_string())?;
    writer
        .write_record(headers)
        .map_err(|error| error.to_string())?;
    let mut count = 0i64;
    for row in rows {
        writer
            .write_record(row.map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
        count += 1;
    }
    writer.flush().map_err(|error| error.to_string())?;
    for name in ["P2_strong_needs_validation.csv", "P3_name_candidates.csv"] {
        let mut empty =
            csv::Writer::from_path(refined_dir.join(name)).map_err(|error| error.to_string())?;
        empty
            .write_record(headers)
            .map_err(|error| error.to_string())?;
        empty.flush().map_err(|error| error.to_string())?;
    }
    Ok(count)
}

fn execute_reprobe_job(
    app: &AppHandle,
    db_path: &Path,
    run_id: i64,
    project_id: i64,
    settings: &Value,
    run_dir: &Path,
    cancel: &AtomicBool,
) -> Result<(), String> {
    fs::create_dir_all(run_dir).map_err(|error| error.to_string())?;
    let scripts_dir = resolve_scripts_dir(app, settings)?;
    let script = scripts_dir.join("4_probe_alive.py");
    if !script.exists() {
        return Err(format!("探测脚本不存在：{}", script.display()));
    }
    let python = resolve_python_with_pandas(&setting_str(settings, "pythonExecutable", "python3"))?;
    let config_path = run_dir.join(".runtime_config.ini");
    write_runtime_config(&config_path, settings)?;
    let refined_dir = run_dir.join("refined");
    let total = write_reprobe_sources(db_path, project_id, &refined_dir)?;
    if total == 0 {
        return Err("当前项目没有可复测的资产".into());
    }
    if let Ok(connection) = db::open(db_path) {
        let _ = connection.execute(
            "UPDATE runs SET total=?1 WHERE id=?2",
            params![total, run_id],
        );
    }
    let probe_dir = run_dir.join("probe");
    let content_rules = run_dir.join("content_rules.json");
    write_content_rules(&content_rules, settings, db_path)?;
    set_progress(
        app,
        db_path,
        run_id,
        "running",
        "reprobe",
        5.0,
        &format!("开始复测现有资产，共 {} 条；支持断点续跑", total),
    );
    let args = vec![
        script.to_string_lossy().to_string(),
        "--config".into(),
        config_path.to_string_lossy().to_string(),
        "--refined-dir".into(),
        refined_dir.to_string_lossy().to_string(),
        "--other-input".into(),
        refined_dir
            .join("P1_active_strong.csv")
            .to_string_lossy()
            .to_string(),
        "--output-dir".into(),
        probe_dir.to_string_lossy().to_string(),
        "--priority-rate".into(),
        setting_f64(settings, "priorityRate", 20.0).to_string(),
        "--other-rate".into(),
        setting_f64(settings, "otherRate", 10.0).to_string(),
        "--workers".into(),
        setting_i64(settings, "workers", 64).to_string(),
        "--timeout".into(),
        setting_i64(settings, "probeTimeout", 6).to_string(),
        "--retries".into(),
        setting_i64(settings, "probeRetries", 0).to_string(),
        "--content-threshold".into(),
        setting_i64(settings, "contentThreshold", 12).to_string(),
        "--content-rules".into(),
        content_rules.to_string_lossy().to_string(),
        "--no-include-other".into(),
        if setting_bool(settings, "includeWeak", false) {
            "--include-weak"
        } else {
            "--no-include-weak"
        }
        .into(),
    ];
    run_process(
        app,
        db_path,
        run_id,
        "reprobe",
        55.0,
        &python,
        &args,
        &scripts_dir,
        cancel,
        &setting_str(settings, "proxyUrl", ""),
        &setting_str(settings, "noProxy", "127.0.0.1,localhost"),
    )?;
    set_progress(
        app,
        db_path,
        run_id,
        "running",
        "import",
        92.0,
        "正在回写最新存活状态",
    );
    let mut processed = 0i64;
    for outcome in [
        "web_alive",
        "web_restricted",
        "browser_render_required",
        "virtual_host_required",
        "web_abnormal",
        "tcp_alive_non_http",
        "blocked_content",
        "unreachable",
        "skipped",
    ] {
        let path = probe_dir.join(format!("P1_{}.csv", outcome));
        if path.exists() {
            let result = import_csv(app, db_path, run_id, project_id, &path, 95.0)?;
            processed += result.inserted + result.updated;
        }
    }
    if let Ok(connection) = db::open(db_path) {
        let _ = connection.execute(
            "UPDATE runs SET processed=?1 WHERE id=?2",
            params![processed, run_id],
        );
    }
    Ok(())
}

fn execute_job(
    app: AppHandle,
    db_path: PathBuf,
    run_id: i64,
    project_id: i64,
    pipeline: String,
    settings: Value,
    targets: Vec<(String, String)>,
    run_dir: PathBuf,
    cancel: Arc<AtomicBool>,
) -> Result<(), String> {
    fs::create_dir_all(&run_dir).map_err(|error| error.to_string())?;
    if pipeline == "reprobe" {
        return execute_reprobe_job(
            &app, &db_path, run_id, project_id, &settings, &run_dir, &cancel,
        );
    }
    let seeds_path = run_dir.join("seeds.csv");
    write_seeds(&seeds_path, &targets)?;
    let scripts_dir = resolve_scripts_dir(&app, &settings)?;
    let python =
        resolve_python_with_pandas(&setting_str(&settings, "pythonExecutable", "python3"))?;
    let proxy_url = setting_str(&settings, "proxyUrl", "");
    let no_proxy = setting_str(&settings, "noProxy", "127.0.0.1,localhost");
    let config_path = run_dir.join(".runtime_config.ini");
    write_runtime_config(&config_path, &settings)?;
    let fofa_key = setting_str(&settings, "fofaKey", "");
    if !fofa_key.trim().is_empty() {
        let fofa_proxy = setting_str(&settings, "proxyUrl", "");
        set_progress(
            &app,
            &db_path,
            run_id,
            "running",
            "collect",
            3.0,
            "正在预检 FOFA 网络、代理与 Key",
        );
        commands::request_fofa_account(fofa_key.trim(), fofa_proxy.trim()).map_err(|error| {
            log_line(
                &db_path,
                run_id,
                "error",
                "collect",
                &format!("FOFA 预检失败：{error}"),
            );
            format!("FOFA 预检失败：{error}。采集尚未开始，请修正配置后重试")
        })?;
        log_line(
            &db_path,
            run_id,
            "info",
            "collect",
            "FOFA 预检通过：网络、代理与 Key 可用",
        );
    }
    let collect_script = scripts_dir.join("1_collect_info.py");
    if !collect_script.exists() {
        return Err(format!("采集脚本不存在：{}", collect_script.display()));
    }

    let collect_dir = run_dir.join("collection");
    set_progress(
        &app,
        &db_path,
        run_id,
        "running",
        "collect",
        5.0,
        "开始查询网络测绘数据",
    );
    let mut collect_args = vec![
        collect_script.to_string_lossy().to_string(),
        "--config".into(),
        config_path.to_string_lossy().to_string(),
        "--seeds".into(),
        seeds_path.to_string_lossy().to_string(),
        "--output-dir".into(),
        collect_dir.to_string_lossy().to_string(),
        "--mode".into(),
        setting_str(&settings, "collectionMode", "all"),
        "--profile".into(),
        setting_str(&settings, "fofaProfile", "professional"),
        "--page-size".into(),
        setting_i64(&settings, "pageSize", 500).to_string(),
        "--max-pages".into(),
        setting_i64(&settings, "maxPages", 0).to_string(),
        "--interval".into(),
        setting_f64(&settings, "interval", 6.0).to_string(),
        "--timeout".into(),
        setting_i64(&settings, "collectionTimeout", 45).to_string(),
        "--max-derived-domains".into(),
        setting_i64(&settings, "maxDerivedDomains", 200).to_string(),
    ];
    collect_args.push(
        if setting_bool(&settings, "fullHistory", false) {
            "--full"
        } else {
            "--no-full"
        }
        .into(),
    );
    collect_args.push(
        if setting_bool(&settings, "enableCidr24", false) {
            "--enable-cidr24"
        } else {
            "--no-enable-cidr24"
        }
        .into(),
    );
    collect_args.push(
        if setting_bool(&settings, "includeWeakFingerprints", false) {
            "--include-weak-fingerprints"
        } else {
            "--no-include-weak-fingerprints"
        }
        .into(),
    );
    run_process(
        &app,
        &db_path,
        run_id,
        "collect",
        35.0,
        &python,
        &collect_args,
        &scripts_dir,
        &cancel,
        &proxy_url,
        &no_proxy,
    )?;

    set_progress(
        &app,
        &db_path,
        run_id,
        "running",
        "import",
        60.0,
        "正在将候选资产增量写入数据库",
    );
    let candidates = collect_dir.join("candidates.csv");
    let imported = import_csv(&app, &db_path, run_id, project_id, &candidates, 65.0)?;
    log_line(
        &db_path,
        run_id,
        "info",
        "import",
        &format!(
            "候选导入完成：新增 {}，变化 {}，无效 {}",
            imported.inserted, imported.updated, imported.invalid
        ),
    );

    // FOFA 返回的是历史测绘结果。只要配置未明确关闭，任何新采集都必须经过
    // 分层和存活探测，避免“仅采集”把历史失效资产直接交给人工处理。
    let should_probe = setting_bool(&settings, "runProbe", true);
    let should_refine = setting_bool(&settings, "runRefine", true) || should_probe;
    let refined_dir = run_dir.join("refined");
    if should_refine {
        let script = scripts_dir.join("3_refine_candidates.py");
        if !script.exists() {
            return Err(format!("分层脚本不存在：{}", script.display()));
        }
        set_progress(
            &app,
            &db_path,
            run_id,
            "running",
            "refine",
            70.0,
            "正在生成 P1/P2/P3 分层",
        );
        let args = vec![
            script.to_string_lossy().to_string(),
            "--input".into(),
            candidates.to_string_lossy().to_string(),
            "--query-log".into(),
            collect_dir
                .join("query_log.csv")
                .to_string_lossy()
                .to_string(),
            "--output-dir".into(),
            refined_dir.to_string_lossy().to_string(),
        ];
        run_process(
            &app,
            &db_path,
            run_id,
            "refine",
            72.0,
            &python,
            &args,
            &scripts_dir,
            &cancel,
            &proxy_url,
            &no_proxy,
        )?;
        // Persist the tiering result before probing/optimisation.  Optimised
        // output may legitimately leave review_tier blank; importing only that
        // file made every asset lose P1/P2/P3 even though refinement succeeded.
        for name in [
            "P1_active_strong.csv",
            "P2_strong_needs_validation.csv",
            "P3_name_candidates.csv",
        ] {
            let path = refined_dir.join(name);
            if path.exists() {
                let _ = import_csv(&app, &db_path, run_id, project_id, &path, 76.0)?;
            }
        }
    }

    if should_probe {
        let script = scripts_dir.join("4_probe_alive.py");
        if !script.exists() {
            return Err(format!("探测脚本不存在：{}", script.display()));
        }
        let probe_dir = run_dir.join("probe");
        let content_rules = run_dir.join("content_rules.json");
        write_content_rules(&content_rules, &settings, &db_path)?;
        set_progress(
            &app,
            &db_path,
            run_id,
            "running",
            "probe",
            78.0,
            "正在执行存活和内容探测",
        );
        let mut args = vec![
            script.to_string_lossy().to_string(),
            "--config".into(),
            config_path.to_string_lossy().to_string(),
            "--refined-dir".into(),
            refined_dir.to_string_lossy().to_string(),
            "--other-input".into(),
            candidates.to_string_lossy().to_string(),
            "--output-dir".into(),
            probe_dir.to_string_lossy().to_string(),
            "--priority-rate".into(),
            setting_f64(&settings, "priorityRate", 20.0).to_string(),
            "--other-rate".into(),
            setting_f64(&settings, "otherRate", 10.0).to_string(),
            "--workers".into(),
            setting_i64(&settings, "workers", 64).to_string(),
            "--timeout".into(),
            setting_i64(&settings, "probeTimeout", 6).to_string(),
            "--retries".into(),
            setting_i64(&settings, "probeRetries", 0).to_string(),
            "--content-threshold".into(),
            setting_i64(&settings, "contentThreshold", 12).to_string(),
            "--content-rules".into(),
            content_rules.to_string_lossy().to_string(),
        ];
        args.push(
            if setting_bool(&settings, "includeOther", true) {
                "--include-other"
            } else {
                "--no-include-other"
            }
            .into(),
        );
        args.push(
            if setting_bool(&settings, "includeWeak", false) {
                "--include-weak"
            } else {
                "--no-include-weak"
            }
            .into(),
        );
        run_process(
            &app,
            &db_path,
            run_id,
            "probe",
            85.0,
            &python,
            &args,
            &scripts_dir,
            &cancel,
            &proxy_url,
            &no_proxy,
        )?;
        let optimizer = scripts_dir.join("5_optimize_src_assets.py");
        let optimized_dir = run_dir.join("optimized");
        if !optimizer.exists() {
            return Err(format!("SRC清洗脚本不存在：{}", optimizer.display()));
        }
        set_progress(
            &app,
            &db_path,
            run_id,
            "running",
            "import",
            95.0,
            "正在执行SRC去重、5xx和失败入口软隔离",
        );
        let optimize_args = vec![
            optimizer.to_string_lossy().to_string(),
            "--input-dir".into(),
            probe_dir.to_string_lossy().to_string(),
            "--output-dir".into(),
            optimized_dir.to_string_lossy().to_string(),
        ];
        run_process(
            &app,
            &db_path,
            run_id,
            "optimize",
            94.0,
            &python,
            &optimize_args,
            &scripts_dir,
            &cancel,
            &proxy_url,
            &no_proxy,
        )?;
        for name in ["optimized_assets.csv", "auto_excluded.csv"] {
            let path = optimized_dir.join(name);
            if path.exists() {
                let _ = import_csv(&app, &db_path, run_id, project_id, &path, 96.0)?;
            }
        }
    }

    mark_missing(&db_path, project_id, run_id)?;
    Ok(())
}

fn launch_run(
    app: AppHandle,
    db_path: PathBuf,
    cancellations: Arc<std::sync::Mutex<HashMap<i64, Arc<AtomicBool>>>>,
    active_jobs: Arc<AtomicUsize>,
    run_id: i64,
    project_id: i64,
    pipeline: String,
    settings: Value,
    targets: Vec<(String, String)>,
    run_dir: PathBuf,
    cancel: Arc<AtomicBool>,
) {
    let cleanup_config = run_dir.join(".runtime_config.ini");
    begin_tray_activity(app.clone(), active_jobs.clone());
    thread::spawn(move || {
        set_job_power_request(true);
        let result = execute_job(
            app.clone(),
            db_path.clone(),
            run_id,
            project_id,
            pipeline,
            settings,
            targets,
            run_dir,
            cancel,
        );
        let _ = fs::remove_file(&cleanup_config);
        let connection = db::open(&db_path);
        match result {
            Ok(()) => {
                if let Ok(connection) = connection {
                    let _ = connection.execute("UPDATE runs SET status='completed',stage='completed',progress=100,finished_at=datetime('now','localtime'),error='' WHERE id=?1",[run_id]);
                }
                let _ = app.emit(
                    "job-progress",
                    JobProgressEvent {
                        run_id,
                        status: "completed".into(),
                        stage: "completed".into(),
                        progress: 100.0,
                        message: "任务完成".into(),
                    },
                );
            }
            Err(error) if error == "__CANCELLED__" => {
                if let Ok(connection) = connection {
                    let _ = connection.execute("UPDATE runs SET status='cancelled',stage='cancelled',finished_at=datetime('now','localtime') WHERE id=?1",[run_id]);
                }
                let _ = app.emit(
                    "job-progress",
                    JobProgressEvent {
                        run_id,
                        status: "cancelled".into(),
                        stage: "cancelled".into(),
                        progress: 0.0,
                        message: "任务已取消；复测任务可从断点继续".into(),
                    },
                );
            }
            Err(error) => {
                log_line(&db_path, run_id, "error", "failed", &error);
                if let Ok(connection) = connection {
                    let _ = connection.execute("UPDATE runs SET status='failed',stage='failed',error=?1,finished_at=datetime('now','localtime') WHERE id=?2",params![error,run_id]);
                }
                let _ = app.emit(
                    "job-progress",
                    JobProgressEvent {
                        run_id,
                        status: "failed".into(),
                        stage: "failed".into(),
                        progress: 0.0,
                        message: error,
                    },
                );
            }
        }
        if let Ok(mut flags) = cancellations.lock() {
            flags.remove(&run_id);
        }
        set_job_power_request(false);
        active_jobs.fetch_sub(1, Ordering::SeqCst);
    });
}

#[tauri::command]
pub fn start_job(
    app: AppHandle,
    state: State<AppState>,
    input: StartJobInput,
) -> Result<i64, String> {
    let connection = db::open(&state.db_path)?;
    let _project_name: String = connection
        .query_row(
            "SELECT name FROM projects WHERE id=?1 AND status='active'",
            [input.project_id],
            |row| row.get(0),
        )
        .map_err(|_| "项目不存在或已归档；恢复工作空间后才能启动资产任务".to_string())?;
    let settings_text: String = connection
        .query_row(
            "SELECT settings_json FROM config_profiles WHERE id=?1",
            [input.profile_id],
            |row| row.get(0),
        )
        .map_err(|_| "配置方案不存在".to_string())?;
    let settings: Value =
        serde_json::from_str(&settings_text).map_err(|error| error.to_string())?;
    let mut snapshot = settings.clone();
    if let Some(object) = snapshot.as_object_mut() {
        let configured = object
            .get("fofaKey")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty());
        object.insert(
            "fofaKey".into(),
            Value::String(if configured { "__CONFIGURED__" } else { "" }.into()),
        );
        let h1_configured = object
            .get("hackerOneToken")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty());
        object.insert(
            "hackerOneToken".into(),
            Value::String(if h1_configured { "__CONFIGURED__" } else { "" }.into()),
        );
    }
    let snapshot_text = snapshot.to_string();
    let targets = if input.pipeline == "reprobe" {
        Vec::new()
    } else {
        let mut statement = connection
            .prepare("SELECT target_type,value FROM targets WHERE project_id=?1 AND enabled=1 ORDER BY id")
            .map_err(|error| error.to_string())?;
        let result = statement
            .query_map([input.project_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<(String, String)>, _>>()
            .map_err(|error| error.to_string())?;
        result
    };
    if targets.is_empty() && input.pipeline != "reprobe" {
        return Err("项目中没有可用目标，请先导入公司名、域名、IP或CIDR".into());
    }
    let total = if input.pipeline == "reprobe" {
        connection
            .query_row(
                "SELECT COUNT(*) FROM project_assets WHERE project_id=?1 AND is_deleted=0",
                [input.project_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| error.to_string())?
    } else {
        targets.len() as i64
    };
    if input.pipeline == "reprobe" && total == 0 {
        return Err("当前项目没有可复测的资产".into());
    }
    let run_uuid = Uuid::new_v4().to_string();
    let run_dir = state.app_data_dir.join("jobs").join(&run_uuid);
    connection.execute(
        "INSERT INTO runs(project_id,profile_id,name,pipeline,total,config_snapshot,output_dir) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![input.project_id,input.profile_id,input.name,input.pipeline,total,snapshot_text,run_dir.to_string_lossy()],
    ).map_err(|error| error.to_string())?;
    let run_id = connection.last_insert_rowid();
    let cancel = Arc::new(AtomicBool::new(false));
    state
        .cancellations
        .lock()
        .map_err(|_| "任务锁异常")?
        .insert(run_id, cancel.clone());
    launch_run(
        app,
        state.db_path.clone(),
        state.cancellations.clone(),
        state.active_jobs.clone(),
        run_id,
        input.project_id,
        input.pipeline,
        settings,
        targets,
        run_dir,
        cancel,
    );
    Ok(run_id)
}

#[tauri::command]
pub fn resume_job(app: AppHandle, state: State<AppState>, run_id: i64) -> Result<i64, String> {
    let connection = db::open(&state.db_path)?;
    let (project_id, profile_id, pipeline, output_dir, status): (i64, i64, String, String, String) = connection
        .query_row(
            "SELECT r.project_id,COALESCE(r.profile_id,0),r.pipeline,r.output_dir,r.status FROM runs r JOIN projects p ON p.id=r.project_id WHERE r.id=?1",
            [run_id],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)),
        ).map_err(|_| "任务不存在".to_string())?;
    if pipeline != "reprobe" {
        return Err("只有现有资产复测任务支持原地断点续跑".into());
    }
    if !matches!(status.as_str(), "interrupted" | "cancelled" | "failed") {
        return Err("该任务当前不能继续".into());
    }
    let settings_text: String = connection
        .query_row(
            "SELECT settings_json FROM config_profiles WHERE id=?1",
            [profile_id],
            |row| row.get(0),
        )
        .map_err(|_| "原配置方案不存在".to_string())?;
    let settings: Value =
        serde_json::from_str(&settings_text).map_err(|error| error.to_string())?;
    connection
        .execute(
            "UPDATE runs SET status='queued',stage='reprobe',error='',finished_at=NULL WHERE id=?1",
            [run_id],
        )
        .map_err(|error| error.to_string())?;
    let cancel = Arc::new(AtomicBool::new(false));
    state
        .cancellations
        .lock()
        .map_err(|_| "任务锁异常")?
        .insert(run_id, cancel.clone());
    launch_run(
        app,
        state.db_path.clone(),
        state.cancellations.clone(),
        state.active_jobs.clone(),
        run_id,
        project_id,
        pipeline,
        settings,
        Vec::new(),
        PathBuf::from(output_dir),
        cancel,
    );
    Ok(run_id)
}

#[tauri::command]
pub fn cancel_job(state: State<AppState>, run_id: i64) -> Result<(), String> {
    if let Some(flag) = state
        .cancellations
        .lock()
        .map_err(|_| "任务锁异常")?
        .get(&run_id)
    {
        flag.store(true, Ordering::Relaxed);
        let connection = db::open(&state.db_path)?;
        connection
            .execute(
                "UPDATE runs SET status='cancel_requested' WHERE id=?1",
                [run_id],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    } else {
        Err("任务不在运行中".into())
    }
}

#[cfg(test)]
mod tests {
    use super::{remember_process_output, write_seeds};
    use std::collections::VecDeque;

    #[test]
    fn keeps_only_the_latest_process_output_lines() {
        let mut recent = VecDeque::new();
        for index in 0..45 {
            remember_process_output(&mut recent, "warning", format!("line {index}"));
        }

        assert_eq!(recent.len(), 40);
        assert_eq!(recent.front().map(String::as_str), Some("[warning] line 5"));
        assert_eq!(recent.back().map(String::as_str), Some("[warning] line 44"));
    }

    #[test]
    fn ignores_blank_process_output() {
        let mut recent = VecDeque::new();
        remember_process_output(&mut recent, "info", "  \n".into());
        assert!(recent.is_empty());
    }

    #[test]
    fn domain_seed_does_not_inherit_the_project_name_as_company() {
        let path =
            std::env::temp_dir().join(format!("oviraptor-seed-{}.csv", uuid::Uuid::new_v4()));
        write_seeds(&path, &[("domain".into(), "*.Example.com".into())]).unwrap();
        let mut reader = csv::Reader::from_path(&path).unwrap();
        let row = reader.records().next().unwrap().unwrap();
        assert_eq!(row.get(0), Some(""));
        assert_eq!(row.get(1), Some(""));
        assert_eq!(row.get(3), Some("*.Example.com"));
        let _ = std::fs::remove_file(path);
    }
}
