fn scan_work_root<'a>(path: &'a Path, scan_id: &str) -> &'a Path {
    path.ancestors()
        .filter(|candidate| {
            fs::read_to_string(candidate.join(".oviraptor-scan-id"))
                .ok()
                .is_some_and(|value| value.trim() == scan_id)
        })
        .last()
        .unwrap_or(path)
}

const STRIX_CLOUD_STARTUP_IDLE_TIMEOUT_SECONDS: u64 = 90;
const STRIX_CLOUD_STARTUP_HARD_TIMEOUT_SECONDS: u64 = 300;
fn strix_startup_timeouts(environment: &StrixRuntimeEnv) -> (u64, u64) {
    if environment.deployment == "local" {
        let policy = local_model_runtime_policy(environment);
        (policy.startup_idle_seconds, policy.startup_hard_seconds)
    } else {
        (
            STRIX_CLOUD_STARTUP_IDLE_TIMEOUT_SECONDS,
            STRIX_CLOUD_STARTUP_HARD_TIMEOUT_SECONDS,
        )
    }
}
const STRIX_IMAGE_PULL_TIMEOUT_SECONDS: u64 = 900;
// The frontend worker has a single per-target watchdog. Its per-identity
// browser budget is derived below so authenticated A/B runs share this limit.
const FRONTEND_RECON_HARD_TIMEOUT_SECONDS: u64 = 900;

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrontendReconConfig {
    #[serde(default = "default_frontend_hard_timeout")]
    hard_timeout_seconds: u64,
    #[serde(default = "default_frontend_browser_timeout")]
    browser_request_timeout_seconds: u64,
    #[serde(default = "default_frontend_exploration_timeout")]
    exploration_timeout_seconds: u64,
}

fn default_frontend_hard_timeout() -> u64 { FRONTEND_RECON_HARD_TIMEOUT_SECONDS }
fn default_frontend_browser_timeout() -> u64 { 30 }
fn default_frontend_exploration_timeout() -> u64 { 600 }

fn frontend_recon_config(worker: &Path) -> FrontendReconConfig {
    let candidates = [
        worker.parent().map(|path| path.join("../config/frontend_recon.json")),
        worker.parent().map(|path| path.join("frontend_recon.json")),
    ];
    candidates.into_iter().flatten().find_map(|path| {
        fs::read_to_string(path).ok().and_then(|text| serde_json::from_str::<FrontendReconConfig>(&text).ok())
    }).unwrap_or(FrontendReconConfig {
        hard_timeout_seconds: FRONTEND_RECON_HARD_TIMEOUT_SECONDS,
        browser_request_timeout_seconds: 45,
        exploration_timeout_seconds: 600,
    })
}

/// Return the hard budget for one target URL.
///
/// Authentication identities are explored inside the same worker process; they
/// must share this URL budget rather than multiplying it. Multiplication made a
/// configured 120-second URL limit look like 240 seconds for A/B sessions and
/// allowed a single target to monopolize the pipeline.
fn frontend_recon_hard_timeout_seconds(_identity_count: usize, config: &FrontendReconConfig) -> u64 {
    config.hard_timeout_seconds.max(1).clamp(30, 1_800)
}

/// Runtime exploration happens once per authenticated identity inside the same
/// worker process. Keep each identity's browser budget below the per-URL
/// watchdog so A/B capture cannot consume 90 seconds each and get killed at
/// the shared 120-second URL limit.
fn frontend_recon_exploration_timeout_seconds(
    identity_count: usize,
    config: &FrontendReconConfig,
) -> u64 {
    let hard_timeout = frontend_recon_hard_timeout_seconds(identity_count, config);
    let identities = identity_count.max(1) as u64;
    let coordinator_reserve = 20_u64.min(hard_timeout.saturating_sub(1));
    let per_identity_budget = hard_timeout
        .saturating_sub(coordinator_reserve)
        .checked_div(identities)
        .unwrap_or(1)
        .max(15);
    config.exploration_timeout_seconds.max(1).min(per_identity_budget)
}

fn no_progress_request_threshold(bounded_frontend: bool, configured_window: i64) -> i64 {
    configured_window.max(if bounded_frontend { 3 } else { 2 })
}

fn no_progress_fuse_allowed(
    bounded_frontend: bool,
    requests: i64,
    no_progress_requests: i64,
    active_child_agents: usize,
    waiting_on_agents: bool,
    configured_window: i64,
) -> bool {
    let threshold = no_progress_request_threshold(bounded_frontend, configured_window);
    requests >= threshold
        && no_progress_requests >= threshold
        && active_child_agents == 0
        && !waiting_on_agents
}

fn strip_ansi_sequences(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut in_escape = false;
    for character in text.chars() {
        if in_escape {
            if character.is_ascii_alphabetic() {
                in_escape = false;
            }
            continue;
        }
        if character == '\u{1b}' {
            in_escape = true;
        } else if character != '\r' {
            output.push(character);
        }
    }
    output
}

fn strix_runner_log_tail(path: &Path, max_lines: usize) -> Vec<String> {
    let Ok(mut file) = File::open(path) else {
        return Vec::new();
    };
    let length = file.metadata().map(|value| value.len()).unwrap_or(0);
    let start = length.saturating_sub(64 * 1024);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return Vec::new();
    }
    let mut bytes = Vec::new();
    if file.read_to_end(&mut bytes).is_err() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&bytes);
    let text = if start > 0 {
        text.split_once('\n').map(|(_, tail)| tail).unwrap_or("")
    } else {
        &text
    };
    let mut lines = text
        .lines()
        .map(strip_ansi_sequences)
        .map(|line| safe_strix_log_line(line.trim()))
        .filter(|line| !line.is_empty())
        .rev()
        .take(max_lines)
        .collect::<Vec<_>>();
    lines.reverse();
    lines
}

fn append_runner_log(path: &Path, message: &str) {
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(
            file,
            "[{}] [oviraptor] {}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
            message
        );
    }
}

fn strix_startup_phase(log_path: &Path) -> String {
    let text = strix_runner_log_tail(log_path, 80).join("\n");
    let lower = text.to_ascii_lowercase();
    if lower.contains("pulling image") && !lower.contains("docker image ready") {
        "正在拉取 Strix Docker 镜像；首次准备会较慢，后续应直接复用缓存".into()
    } else if lower.contains("docker image ready") {
        "Docker 镜像已就绪，正在等待模型 warm-up".into()
    } else if lower.contains("model cost map") && lower.contains("timed out") {
        "LiteLLM 模型元数据请求超时，已回退本地缓存并继续启动".into()
    } else {
        "正在启动 Strix 进程".into()
    }
}

fn strix_failure_detail(log_path: &Path, fallback: &str) -> String {
    let lines = strix_runner_log_tail(log_path, 120);
    let text = lines.join("\n");
    let lower = text.to_ascii_lowercase();
    if lower.contains("unicodeencodeerror") && lower.contains("gbk") {
        return "Strix Windows 控制台编码失败：GBK 无法输出 Unicode；请改用 pipx 安装的 strix-agent、WSL2，或升级已修复该问题的 Strix Windows 构建".into();
    }
    if lower.contains("llm warm-up failed") {
        return "Strix 模型预热失败；请检查模型连通性、模型 ID 与 Strix CLI 兼容性".into();
    }
    if (lower.contains("invalid api key") || lower.contains("api key") && lower.contains("invalid"))
        || lower.contains("authentication_error")
        || lower.contains("authentication fails")
        || lower.contains("incorrect api key")
    {
        return "模型认证失败：API Key 无效或已失效；前端侦察结果已保留，请更新模型配置后重试 Strix".into();
    }
    let detail = lines
        .into_iter()
        .rev()
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            !lower.contains("oviraptor sandbox cleanup")
                && !lower.contains("model quality warning")
                && !line
                    .chars()
                    .all(|character| matches!(character, '|' | '+' | '-' | ' '))
        })
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join(" | ");
    if detail.is_empty() {
        fallback.to_string()
    } else {
        format!("{fallback}；{detail}").chars().take(1600).collect()
    }
}

fn strix_configuration_failure(reason: &str) -> bool {
    let lower = reason.to_ascii_lowercase();
    lower.contains("模型认证失败")
        || lower.contains("invalid api key")
        || lower.contains("authentication_error")
        || lower.contains("authentication fails")
        || lower.contains("incorrect api key")
}

fn strix_retryable_provider_failure(reason: &str) -> bool {
    let lower = reason.to_ascii_lowercase();
    [
        "resource temporarily unavailable",
        "os error 35",
        "temporarily unavailable",
        "service unavailable",
        "upstream overloaded",
        "overloaded",
        "rate limit",
        "too many requests",
        "http 429",
        "error code: 429",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn strix_run_was_interrupted(target_dir: &Path) -> bool {
    strix_run_dirs(target_dir)
        .map(|dirs| {
            dirs.iter().any(|dir| {
                fs::read(dir.join(STRIX_RUN_ARTIFACT))
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<JsonValue>(&bytes).ok())
                    .and_then(|run| run.get("status").and_then(JsonValue::as_str).map(str::to_ascii_lowercase))
                    .is_some_and(|status| matches!(status.as_str(), "interrupted" | "cancelled" | "canceled"))
            })
        })
        .unwrap_or(false)
}

/// Strix writes the terminal run artifact during interpreter shutdown. On a
/// fast local process exit that file can become visible just after `try_wait`
/// reports the non-zero status. Reconcile that short write race before turning
/// a deliberately bounded/interrupted run into the misleading `exit status 1`.
fn wait_for_strix_interrupted_artifact(target_dir: &Path) -> bool {
    if strix_run_was_interrupted(target_dir) {
        return true;
    }
    for _ in 0..6 {
        thread::sleep(Duration::from_millis(100));
        if strix_run_was_interrupted(target_dir) {
            return true;
        }
    }
    false
}

fn prepare_strix_sandbox_image(
    db_path: &Path,
    scan_id: &str,
    docker: &Path,
    runtime_path: &OsString,
    log_path: &Path,
    image: &str,
) -> Result<(), String> {
    let mut inspect_command = Command::new(docker);
    configure_child_command(&mut inspect_command);
    let inspected = inspect_command
        .args(["image", "inspect", image])
        .env("PATH", runtime_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if inspected.is_ok_and(|status| status.success()) {
        sentinel_scan_update(
            db_path,
            scan_id,
            "scanning",
            &format!("Strix Docker 镜像已就绪：{image}"),
        );
        return Ok(());
    }
    let started = Instant::now();
    let mut last_error = String::new();
    for attempt in 1..=4 {
        sentinel_scan_update(
            db_path,
            scan_id,
            "scanning",
            &format!("正在拉取 Strix Docker 镜像 · 第 {attempt}/4 次"),
        );
        let stdout = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)
            .map_err(|error| error.to_string())?;
        let stderr = stdout.try_clone().map_err(|error| error.to_string())?;
        let mut command = Command::new(docker);
        configure_child_command(&mut command);
        let mut child = command
            .args(["pull", image])
            .env("PATH", runtime_path)
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()
            .map_err(|error| format!("Docker 镜像拉取无法启动：{error}"))?;
        let process_id = child.id();
        sentinel_process_set(db_path, scan_id, process_id, "docker-image-pull", log_path);
        let attempt_result = loop {
            if sentinel_scan_pause_requested(db_path, scan_id) {
                let _ = child.kill();
                let _ = child.wait();
                break Err("暂停请求已接收；Strix 镜像拉取已停止".into());
            }
            if !sentinel_scan_is_active(db_path, scan_id) {
                let _ = child.kill();
                let _ = child.wait();
                break Err("任务已取消".into());
            }
            match child.try_wait() {
                Ok(Some(status)) if status.success() => break Ok(()),
                Ok(Some(status)) => {
                    let detail = strix_runner_log_tail(log_path, 12).join(" | ");
                    break Err(if detail.is_empty() {
                        format!("Docker 拉取 Strix 镜像失败：{status}")
                    } else {
                        format!("Docker 拉取 Strix 镜像失败：{status}；{detail}")
                    });
                }
                Err(error) => break Err(format!("Docker 镜像拉取状态读取失败：{error}")),
                Ok(None) if started.elapsed().as_secs() >= STRIX_IMAGE_PULL_TIMEOUT_SECONDS => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(format!(
                        "Docker 拉取 Strix 镜像超过 {STRIX_IMAGE_PULL_TIMEOUT_SECONDS} 秒"
                    ));
                }
                Ok(None) => {
                    let elapsed = started.elapsed().as_secs();
                    if elapsed.is_multiple_of(5) {
                        sentinel_scan_update(
                            db_path,
                            scan_id,
                            "scanning",
                            &format!(
                                "正在拉取 Strix Docker 镜像 · 第 {attempt}/4 次 · {elapsed} 秒"
                            ),
                        );
                    }
                    thread::sleep(Duration::from_millis(500));
                }
            }
        };
        sentinel_process_clear(db_path, scan_id, process_id);
        match attempt_result {
            Ok(()) => return Ok(()),
            Err(error)
                if error.starts_with("暂停请求")
                    || error == "任务已取消"
                    || started.elapsed().as_secs() >= STRIX_IMAGE_PULL_TIMEOUT_SECONDS =>
            {
                return Err(error);
            }
            Err(error) => {
                last_error = error;
                if attempt < 4 {
                    thread::sleep(Duration::from_secs(2));
                }
            }
        }
    }
    let lower = last_error.to_ascii_lowercase();
    if lower.contains("pkg-containers.githubusercontent.com")
        && (lower.contains("eof") || lower.contains("timed out"))
    {
        Err(format!(
            "Docker daemon 下载 GHCR blob 时网络中断；请在 Docker Desktop 的 Proxies 中配置可访问 pkg-containers.githubusercontent.com 的代理。应用内代理不会传递给 Docker daemon。最后错误：{last_error}"
        ))
    } else {
        Err(last_error)
    }
}

fn strix_progress_idle_timeout(route: &FrontendRoute) -> u64 {
    match route.mode.as_str() {
        "deep" => 300,
        "standard" => 180,
        _ => 120,
    }
}

fn collect_private_artifacts(root: &Path, path: &Path, depth: usize, files: &mut Vec<JsonValue>) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let child = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&child) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = if metadata.is_dir() { 0o700 } else { 0o600 };
            let _ = fs::set_permissions(&child, fs::Permissions::from_mode(mode));
        }
        if metadata.is_dir() {
            collect_private_artifacts(root, &child, depth - 1, files);
        } else if metadata.is_file()
            && child.file_name().and_then(|value| value.to_str()) != Some("manifest.json")
        {
            files.push(serde_json::json!({
                "file": child.strip_prefix(root).unwrap_or(&child).to_string_lossy(),
                "bytes": metadata.len(),
            }));
        }
    }
}

fn finalize_strix_tool_output_archive(root: &Path) -> usize {
    let mut files = Vec::new();
    collect_private_artifacts(root, root, 8, &mut files);
    files.sort_by(|left, right| left["file"].as_str().cmp(&right["file"].as_str()));
    if files.is_empty() {
        let _ = fs::remove_dir(root);
        return 0;
    }
    let count = files.len();
    let manifest = serde_json::json!({
        "schemaVersion": 1,
        "description": "Full Strix tool outputs archived before sandbox cleanup. Files may contain sensitive scan evidence.",
        "archivedAt": chrono::Utc::now().to_rfc3339(),
        "files": files,
    });
    if let Ok(bytes) = serde_json::to_vec_pretty(&manifest) {
        let manifest_path = root.join("manifest.json");
        let _ = fs::write(&manifest_path, bytes);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&manifest_path, fs::Permissions::from_mode(0o600));
            let _ = fs::set_permissions(root, fs::Permissions::from_mode(0o700));
        }
    }
    count
}

fn cleanup_strix_sandboxes(
    work_dir: &Path,
    docker: &Path,
    runtime_path: &OsString,
    runner_log: &Path,
) -> usize {
    let mut container_ids = HashSet::new();
    if let Ok(run_dirs) = strix_run_dirs(work_dir) {
        for dir in run_dirs {
            let Ok(log) = fs::read_to_string(dir.join("strix.log")) else {
                continue;
            };
            for line in log.lines() {
                let Some(raw) = line.split("Sandbox container created: id=").nth(1) else {
                    continue;
                };
                let id = raw
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase();
                if (12..=64).contains(&id.len())
                    && id.chars().all(|value| value.is_ascii_hexdigit())
                {
                    container_ids.insert(id);
                }
            }
        }
    }
    let mut removed = 0usize;
    let archive_root = work_dir.join("strix-tool-output");
    for id in container_ids {
        let mut inspect_command = Command::new(docker);
        configure_child_command(&mut inspect_command);
        let inspected = inspect_command
            .args(["inspect", "--format", "{{.Config.Image}}", &id])
            .env("PATH", runtime_path)
            .output();
        let Ok(inspected) = inspected else { continue };
        if !inspected.status.success() {
            continue;
        }
        let image = String::from_utf8_lossy(&inspected.stdout);
        if !image.trim().to_ascii_lowercase().contains("strix-sandbox") {
            continue;
        }
        if fs::create_dir_all(&archive_root).is_ok() {
            let mut copy_command = Command::new(docker);
            configure_child_command(&mut copy_command);
            let _ = copy_command
                .args([
                    "cp",
                    &format!("{id}:/workspace/.strix/tool-output/."),
                    archive_root.to_string_lossy().as_ref(),
                ])
                .env("PATH", runtime_path)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let mut remove_command = Command::new(docker);
        configure_child_command(&mut remove_command);
        let status = remove_command
            .args(["rm", "-f", &id])
            .env("PATH", runtime_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if status.is_ok_and(|value| value.success()) {
            removed += 1;
        }
    }
    let archived = finalize_strix_tool_output_archive(&archive_root);
    if let Ok(mut log) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(runner_log)
    {
        let _ = writeln!(
            log,
            "Oviraptor sandbox cleanup: archived {archived} full tool output file(s), removed {removed} Strix container(s) for {}",
            work_dir.display()
        );
        if archived > 0 {
            let _ = writeln!(
                log,
                "Oviraptor full tool output archive: {} (local-only, permission restricted)",
                archive_root.display()
            );
        }
    }
    removed
}
