const MODEL_CLOUD_STARTUP_IDLE_TIMEOUT_SECONDS: u64 = 90;
const MODEL_CLOUD_STARTUP_HARD_TIMEOUT_SECONDS: u64 = 300;
fn model_startup_timeouts(environment: &ModelRuntimeEnv) -> (u64, u64) {
    if environment.deployment == "local" {
        let policy = local_model_runtime_policy(environment);
        (policy.startup_idle_seconds, policy.startup_hard_seconds)
    } else {
        (
            MODEL_CLOUD_STARTUP_IDLE_TIMEOUT_SECONDS,
            MODEL_CLOUD_STARTUP_HARD_TIMEOUT_SECONDS,
        )
    }
}
// The frontend worker has a single per-target watchdog. Its per-identity
// browser budget is derived below so authenticated A/B runs share this limit.
const FRONTEND_RECON_HARD_TIMEOUT_SECONDS: u64 = 900;

#[derive(Debug, serde::Deserialize, serde::Serialize)]
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

#[cfg(test)]
fn no_progress_request_threshold(bounded_frontend: bool, configured_window: i64) -> i64 {
    configured_window.max(if bounded_frontend { 3 } else { 2 })
}

#[cfg(test)]
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

/// Why a runner-log read returned what it did. The reader must never collapse
/// "the file was not created", "it is empty" and "it could not be read" into one
/// empty list, because the UI would then report a real failure as "no logs yet".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RunnerLogStatus {
    Ready,
    NotCreated,
    Empty,
    ReadFailed,
}

impl RunnerLogStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::NotCreated => "not_created",
            Self::Empty => "empty",
            Self::ReadFailed => "read_failed",
        }
    }
}

struct RunnerLogRead {
    status: RunnerLogStatus,
    lines: Vec<String>,
    detail: String,
}

fn runner_log_failure(detail: impl Into<String>) -> RunnerLogRead {
    RunnerLogRead {
        status: RunnerLogStatus::ReadFailed,
        lines: Vec::new(),
        detail: detail.into(),
    }
}

/// Runner logs are shown and copied in the UI, so credential-shaped material is
/// masked there even though the on-disk evidence file stays untouched.
fn redact_runner_log_line(line: &str) -> String {
    crate::agent_runtime::secrets::redact_text_with(line, None)
}

/// A free-text diagnostic kept at a bounded length with credentials masked, so
/// storing one cannot grow without limit or leak a header value.
fn bounded_redacted_text(text: &str, max_chars: usize) -> String {
    redact_runner_log_line(text)
        .chars()
        .take(max_chars)
        .collect()
}

/// Tail of a runner log: last 64 KiB, ANSI stripped, secrets redacted.
fn read_runner_log_tail(path: &Path, max_lines: usize) -> RunnerLogRead {
    if !path.is_file() {
        return RunnerLogRead {
            status: RunnerLogStatus::NotCreated,
            lines: Vec::new(),
            detail: format!("日志文件尚未生成：{}", path.display()),
        };
    }
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) => {
            return runner_log_failure(format!("无法打开 {}：{error}", path.display()))
        }
    };
    let length = match file.metadata() {
        Ok(metadata) => metadata.len(),
        Err(error) => {
            return runner_log_failure(format!("无法读取 {} 元数据：{error}", path.display()))
        }
    };
    let start = length.saturating_sub(64 * 1024);
    if let Err(error) = file.seek(SeekFrom::Start(start)) {
        return runner_log_failure(format!(
            "无法定位 {} 第 {start} 字节：{error}",
            path.display()
        ));
    }
    let mut bytes = Vec::new();
    if let Err(error) = file.read_to_end(&mut bytes) {
        return runner_log_failure(format!("无法读取 {}：{error}", path.display()));
    }
    let text = String::from_utf8_lossy(&bytes);
    let text = if start > 0 {
        text.split_once('\n').map(|(_, tail)| tail).unwrap_or("")
    } else {
        &text
    };
    let mut lines = text
        .lines()
        .map(runner_log_display_line)
        .filter(|line| !line.is_empty())
        .rev()
        .take(max_lines)
        .collect::<Vec<_>>();
    lines.reverse();
    if lines.is_empty() {
        return RunnerLogRead {
            status: RunnerLogStatus::Empty,
            lines,
            detail: format!("{} 没有可显示的日志行", path.display()),
        };
    }
    RunnerLogRead {
        status: RunnerLogStatus::Ready,
        lines,
        detail: String::new(),
    }
}


static NEST_LOG_APP: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();

pub(crate) fn install_nest_runner_log_bus(app: tauri::AppHandle) {
    let _ = NEST_LOG_APP.set(app);
}

fn runner_log_identity(path: &Path) -> Option<(String, i64)> {
    let mut cursor = path.parent();
    while let Some(dir) = cursor {
        let name = dir.file_name()?.to_string_lossy();
        if let Some(rest) = name.strip_prefix("attempt-") {
            let attempt = rest.parse::<i64>().ok()?;
            let scan_id = dir.parent()?.file_name()?.to_string_lossy().to_string();
            if !scan_id.is_empty() {
                return Some((scan_id, attempt));
            }
        }
        cursor = dir.parent();
    }
    None
}

fn append_runner_log(path: &Path, message: &str) {
    // Filter new diagnostic writes without the UI display length bound;
    // existing log evidence and the helper's JSON stdout stay untouched.
    let line = format!(
        "[{}] [oviraptor] {}",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
        redact_runner_log_line(message)
    );
    if OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| writeln!(file, "{line}"))
        .is_err()
    {
        return;
    }
    let Some(app) = NEST_LOG_APP.get() else {
        return;
    };
    let Some((scan_id, attempt)) = runner_log_identity(path) else {
        return;
    };
    let _ = app.emit(
        "nest-runner-log",
        runner_log_notification(&scan_id, attempt, &line),
    );
}


fn model_configuration_failure(reason: &str) -> bool {
    let lower = reason.to_ascii_lowercase();
    lower.contains("模型认证失败")
        || lower.contains("invalid api key")
        || lower.contains("authentication_error")
        || lower.contains("authentication fails")
        || lower.contains("incorrect api key")
}

fn model_retryable_provider_failure(reason: &str) -> bool {
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
