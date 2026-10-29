//! Minimal desktop wiring: spawn the Node agent-host as a local sidecar and
//! expose origin/session to the webview. Does not touch Nest/Asset business kernels.

use serde::Serialize;
use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{mpsc, Mutex},
    thread,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

#[path = "agent_host_lifecycle.rs"]
mod lifecycle;
use lifecycle::OwnedHostProcess;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(20);
const PACKAGED_HOST_ROOT: &str = "resources/agent-host";
const PACKAGED_HOST_CLI: &str = "packages/agent-host/src/cli.mjs";
const PACKAGED_HOST_FILES: &[&str] = &[
    PACKAGED_HOST_CLI,
    "packages/agent-host/bundled-apps.json",
    "packages/agent-host/shell-apps.json",
    "packages/agent-ui/dist/index.html",
    "packages/app-sdk/src/index.mjs",
    "packages/app-sdk/src/worker.mjs",
    "packages/runtime-acp/index.mjs",
    "packages/skills-library/src/index.mjs",
    "packages/skills-library/resources/compound-engineering.json.gz",
    "packages/skills-library/resources/COMPOUND-ENGINEERING-LICENSE",
];

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentHostEndpoint {
    pub origin: String,
    pub session: String,
    pub source: String,
}

pub struct AgentHostSidecar {
    child: Mutex<Option<OwnedHostProcess>>,
    endpoint: Mutex<Option<AgentHostEndpoint>>,
}

impl AgentHostSidecar {
    pub fn inactive() -> Self {
        Self {
            child: Mutex::new(None),
            endpoint: Mutex::new(None),
        }
    }

    pub fn endpoint(&self) -> Option<AgentHostEndpoint> {
        self.endpoint.lock().ok()?.clone()
    }

    pub fn stop(&self) {
        if let Ok(mut guard) = self.child.lock() {
            if let Some(mut child) = guard.take() {
                let outcome = child.stop();
                if outcome.forced || !outcome.host_reaped {
                    eprintln!("Oviraptor agent-host required forced shutdown; active work may have been interrupted");
                }
            }
        }
        if let Ok(mut endpoint) = self.endpoint.lock() {
            endpoint.take();
        }
    }
}

impl Drop for AgentHostSidecar {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Signal handlers only notify a watcher; Tauri's ordinary Exit event performs
/// owned-child cleanup. Window close/hide never requests application exit.
pub struct DesktopExitSignals {
    #[cfg(unix)]
    bridge: Mutex<Option<lifecycle::ExitSignalBridge>>,
}

impl DesktopExitSignals {
    pub fn stop(&self) {
        #[cfg(unix)]
        if let Ok(mut bridge) = self.bridge.lock() {
            bridge.take();
        }
    }
}

pub fn install_exit_signals(app: &AppHandle) -> Result<DesktopExitSignals, String> {
    #[cfg(unix)]
    {
        let handle = app.clone();
        let bridge = lifecycle::ExitSignalBridge::install(move |signal| {
            handle.exit(128 + signal);
        }).map_err(|error| format!("failed to install desktop exit signals: {error}"))?;
        Ok(DesktopExitSignals { bridge: Mutex::new(Some(bridge)) })
    }
    #[cfg(not(unix))]
    {
        let _ = app;
        Ok(DesktopExitSignals {})
    }
}

/// A loopback origin with an explicit port and nothing else. A prefix match
/// let `http://localhost:1@evil.example/` through, so parse and check the real
/// authority: scheme http, host 127.0.0.1/localhost, a port, no userinfo, and
/// an empty path/query/fragment.
fn valid_host_origin(origin: &str) -> bool {
    match reqwest::Url::parse(origin) {
        Ok(url) => {
            url.scheme() == "http"
                && matches!(url.host_str(), Some("127.0.0.1") | Some("localhost"))
                && url.port().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && matches!(url.path(), "" | "/")
                && url.query().is_none()
                && url.fragment().is_none()
        }
        Err(_) => false,
    }
}

fn env_endpoint() -> Option<AgentHostEndpoint> {
    let origin = std::env::var("OVIRAPTOR_HOST_ORIGIN")
        .ok()?
        .trim()
        .to_string();
    let session = std::env::var("OVIRAPTOR_HOST_SESSION")
        .ok()?
        .trim()
        .to_string();
    if origin.is_empty() || session.is_empty() {
        return None;
    }
    if !valid_host_origin(&origin) {
        return None;
    }
    if !session.chars().all(|c| c.is_ascii_hexdigit()) || !(32..=128).contains(&session.len()) {
        return None;
    }
    Some(AgentHostEndpoint {
        origin: origin.trim_end_matches('/').to_string(),
        session,
        source: "env".into(),
    })
}

fn node_major(text: &str) -> Option<u32> {
    text.trim()
        .trim_start_matches('v')
        .split('.')
        .next()?
        .parse()
        .ok()
}

fn probe_node(path: &Path) -> Option<u32> {
    if !path.is_file() {
        return None;
    }
    let output = Command::new(path).arg("-v").output().ok()?;
    if !output.status.success() {
        return None;
    }
    node_major(&String::from_utf8_lossy(&output.stdout))
}

fn require_node24(path: PathBuf) -> Result<PathBuf, String> {
    if probe_node(&path).is_some_and(|major| major >= 24) {
        return Ok(path);
    }
    Err("agent-host requires an external Node.js 24+ executable; OVIRAPTOR_NODE_EXECUTABLE is missing, invalid, or too old".into())
}

fn select_node24(mut candidates: Vec<(u32, PathBuf)>) -> Result<PathBuf, String> {
    candidates.sort_by(|a, b| b.0.cmp(&a.0));
    candidates
        .into_iter()
        .find(|(major, _)| *major >= 24)
        .map(|(_, path)| path)
        .ok_or_else(|| {
            "agent-host requires external Node.js 24+; install it or set OVIRAPTOR_NODE_EXECUTABLE"
                .into()
        })
}

fn resolve_node() -> Result<PathBuf, String> {
    if let Some(configured) = std::env::var_os("OVIRAPTOR_NODE_EXECUTABLE") {
        return require_node24(PathBuf::from(configured));
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let runtime_path = crate::worker_resources::sentinel_runtime_path(&home);
    let candidates = crate::worker_resources::helper_node_candidates(&runtime_path, None);
    let mut scored: Vec<(u32, PathBuf)> = Vec::new();
    for path in candidates {
        if let Some(major) = probe_node(&path) {
            scored.push((major, path));
        }
    }
    select_node24(scored)
}

fn packaged_cli(root: &Path) -> Result<PathBuf, String> {
    let receipt: serde_json::Value = std::fs::read(root.join("desktop-distribution.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or_else(|| {
            "packaged agent-host distribution receipt is missing or invalid".to_string()
        })?;
    if receipt["schemaVersion"].as_u64() != Some(1)
        || receipt["applicationVersion"].as_str() != Some(env!("CARGO_PKG_VERSION"))
        || receipt["minimumNodeMajor"].as_u64() != Some(24)
        || receipt["nodeRuntimeBundled"].as_bool() != Some(false)
    {
        return Err("packaged agent-host resources do not match this desktop build".into());
    }
    for path in PACKAGED_HOST_FILES {
        if !root.join(path).is_file() {
            return Err(format!("packaged agent-host resource is missing: {path}"));
        }
    }
    let bundled: serde_json::Value =
        std::fs::read(root.join("packages/agent-host/bundled-apps.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .ok_or_else(|| "packaged builtin application list is invalid".to_string())?;
    let apps = bundled["apps"]
        .as_array()
        .filter(|apps| !apps.is_empty())
        .ok_or_else(|| "packaged builtin application list is empty".to_string())?;
    for app in apps {
        let package = app["package"]
            .as_str()
            .ok_or_else(|| "packaged builtin application path is invalid".to_string())?;
        let path = Path::new(package);
        if !package.starts_with("apps/")
            || path
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
            || !root.join(path).join("manifest.json").is_file()
        {
            return Err("packaged builtin application resources are missing or invalid".into());
        }
    }
    Ok(root.join(PACKAGED_HOST_CLI))
}

fn resolve_cli_in(
    resource_dir: Option<&Path>,
    debug_root: Option<&Path>,
    debug_override: Option<&Path>,
) -> Result<PathBuf, String> {
    if let Some(resource_dir) = resource_dir {
        let root = resource_dir.join(PACKAGED_HOST_ROOT);
        if root.exists() {
            return packaged_cli(&root);
        }
    }
    if debug_root.is_some() {
        if let Some(path) = debug_override {
            return path
                .is_file()
                .then(|| path.to_path_buf())
                .ok_or_else(|| "debug agent-host CLI override does not exist".into());
        }
        let path = debug_root.unwrap().join(PACKAGED_HOST_CLI);
        if path.is_file() {
            return Ok(path);
        }
    }
    Err(
        "packaged agent-host resources are missing; rebuild with npm run build:desktop-agent-host"
            .into(),
    )
}

fn resolve_cli(app: &AppHandle) -> Result<PathBuf, String> {
    let resource_dir = app.path().resource_dir().ok();
    // Release builds deliberately contain no compiler-checkout fallback or CLI
    // environment override. A missing bundle must be reported as a packaging error.
    #[cfg(debug_assertions)]
    let debug_root = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".."));
    #[cfg(not(debug_assertions))]
    let debug_root: Option<PathBuf> = None;
    #[cfg(debug_assertions)]
    let debug_override = std::env::var_os("OVIRAPTOR_AGENT_HOST_CLI").map(PathBuf::from);
    #[cfg(not(debug_assertions))]
    let debug_override: Option<PathBuf> = None;
    resolve_cli_in(
        resource_dir.as_deref(),
        debug_root.as_deref(),
        debug_override.as_deref(),
    )
}

fn parse_ready_line(line: &str, origin: &mut Option<String>, session: &mut Option<String>) {
    let trimmed = line.trim();
    if let Some(value) = trimmed.strip_prefix("Oviraptor host:") {
        *origin = Some(value.trim().trim_end_matches('/').to_string());
    } else if let Some(value) = trimmed.strip_prefix("Session:") {
        *session = Some(value.trim().to_string());
    }
}

fn spawn_sidecar(app: &AppHandle, data_dir: &Path) -> Result<(OwnedHostProcess, AgentHostEndpoint), String> {
    let node = resolve_node()?;
    let cli = resolve_cli(app)?;
    let host_data = data_dir.join("agent-host");
    std::fs::create_dir_all(&host_data).map_err(|error| error.to_string())?;
    let session = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    // Both packaged and debug layouts retain the monorepo-relative package tree.
    let bundled_root = cli
        .parent() // src
        .and_then(|p| p.parent()) // agent-host
        .and_then(|p| p.parent()) // packages
        .and_then(|p| p.parent()) // repo or resource root
        .map(|p| p.to_path_buf())
        .ok_or_else(|| "agent-host CLI has an invalid distribution layout".to_string())?;

    let mut command = Command::new(&node);
    if let Some(bridge) = app.try_state::<crate::certificate_bridge::Bridge>() {
        command.env("OVIRAPTOR_CERTIFICATE_BRIDGE_ORIGIN", &bridge.origin).env("OVIRAPTOR_CERTIFICATE_BRIDGE_TOKEN", &bridge.token);
        command.env("OVIRAPTOR_SCAN_BRIDGE_ORIGIN", &bridge.origin).env("OVIRAPTOR_SCAN_BRIDGE_TOKEN", &bridge.token);
    }
    for (key, value) in sidecar_proxy_env(
        |key| std::env::var(key).ok().filter(|v| !v.trim().is_empty()),
        read_scutil_proxy,
    ) {
        command.env(key, value);
    }
    let mut child = OwnedHostProcess::spawn(command
        .arg(&cli)
        .arg("--data-dir")
        .arg(&host_data)
        // The token goes over stdin: argv is readable by every local user
        // through `ps` and /proc/<pid>/cmdline.
        .arg("--session-token-stdin")
        .arg("--port")
        .arg("0")
        // No --runtime override: the host restores the saved runtime (xai-api after Grok OAuth).
        .env("OVIRAPTOR_BUNDLED_APPS_ROOT", &bundled_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .stdin(Stdio::piped()))
        .map_err(|error| format!("failed to spawn agent-host sidecar: {error}"))?;
    {
        let mut stdin = child
            .take_stdin()
            .ok_or_else(|| "agent-host sidecar stdin unavailable".to_string())?;
        writeln!(stdin, "{session}")
            .map_err(|error| format!("failed to pass agent-host session: {error}"))?;
    }

    let stdout = child
        .take_stdout()
        .ok_or_else(|| "agent-host sidecar stdout unavailable".to_string())?;
    let (tx, rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().flatten() {
            if tx.send(line).is_err() {
                break;
            }
        }
    });

    let mut origin = None;
    let mut reported_session = None;
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    while origin.is_none() || reported_session.is_none() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("agent-host sidecar startup timed out".into());
        }
        match rx.recv_timeout(remaining) {
            Ok(line) => parse_ready_line(&line, &mut origin, &mut reported_session),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                return Err("agent-host sidecar startup timed out".into());
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("agent-host sidecar exited before becoming ready".into());
            }
        }
    }
    let origin = origin.unwrap();
    let session = reported_session.unwrap_or(session);
    if !session.chars().all(|c| c.is_ascii_hexdigit()) || !(32..=128).contains(&session.len()) {
        return Err("agent-host sidecar returned an invalid session token".into());
    }
    Ok((
        child,
        AgentHostEndpoint {
            origin,
            session,
            source: "sidecar".into(),
        },
    ))
}

/// Prefer explicit env credentials; otherwise spawn a local agent-host sidecar.
pub fn start(app: &AppHandle, data_dir: &Path) -> AgentHostSidecar {
    if let Some(endpoint) = env_endpoint() {
        let _ = inject_env(app, &endpoint);
        return AgentHostSidecar {
            child: Mutex::new(None),
            endpoint: Mutex::new(Some(endpoint)),
        };
    }
    match spawn_sidecar(app, data_dir) {
        Ok((child, endpoint)) => {
            // Not exported through std::env: every later child process
            // (analyzers, workers) would inherit the session token.
            let _ = inject_env(app, &endpoint);
            AgentHostSidecar {
                child: Mutex::new(Some(child)),
                endpoint: Mutex::new(Some(endpoint)),
            }
        }
        Err(error) => {
            eprintln!("Oviraptor agent-host sidecar unavailable: {error}");
            AgentHostSidecar::inactive()
        }
    }
}

fn inject_env(app: &AppHandle, endpoint: &AgentHostEndpoint) -> Result<(), String> {
    let script = format!(
        "window.__OVIRAPTOR_ENV__=Object.assign(window.__OVIRAPTOR_ENV__||{{}},{});",
        serde_json::json!({
            "OVIRAPTOR_HOST_ORIGIN": endpoint.origin,
            "OVIRAPTOR_HOST_SESSION": endpoint.session,
            "OVIRAPTOR_APP_HOST": "agent-host",
        })
    );
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.eval(&script);
    }
    Ok(())
}

#[tauri::command]
pub fn agent_host_endpoint(state: State<'_, AgentHostSidecar>) -> Option<AgentHostEndpoint> {
    state.endpoint()
}

const PROXY_ENV_KEYS: [&str; 6] = [
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "ALL_PROXY",
    "all_proxy",
];

/// Node's fetch ignores the macOS system proxy. When the parent process has no proxy env,
/// read `scutil --proxy` and inject HTTPS_PROXY/HTTP_PROXY (+ loopback NO_PROXY) for the child.
fn sidecar_proxy_env(
    get_env: impl Fn(&str) -> Option<String>,
    read_system: impl FnOnce() -> Option<String>,
) -> Vec<(String, String)> {
    if PROXY_ENV_KEYS.iter().any(|key| get_env(key).is_some()) {
        // Parent already configured a proxy; only make sure loopback bypasses it.
        if get_env("NO_PROXY").is_none() && get_env("no_proxy").is_none() {
            return vec![("NO_PROXY".into(), "127.0.0.1,localhost,::1".into())];
        }
        return Vec::new();
    }
    let Some(output) = read_system() else {
        return Vec::new();
    };
    let (https, http) = parse_scutil_proxy(&output);
    let mut env = Vec::new();
    if let Some(url) = https.clone().or_else(|| http.clone()) {
        env.push(("HTTPS_PROXY".to_string(), url));
    }
    if let Some(url) = http.or(https) {
        env.push(("HTTP_PROXY".to_string(), url));
    }
    if !env.is_empty() {
        let mut no_proxy = vec![
            "127.0.0.1".to_string(),
            "localhost".to_string(),
            "::1".to_string(),
        ];
        if let Some(extra) = get_env("NO_PROXY").or_else(|| get_env("no_proxy")) {
            no_proxy.push(extra);
        }
        env.push(("NO_PROXY".to_string(), no_proxy.join(",")));
    }
    env
}

fn read_scutil_proxy() -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let output = Command::new("/usr/sbin/scutil")
        .arg("--proxy")
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Parse `scutil --proxy` output into (https_proxy_url, http_proxy_url).
fn parse_scutil_proxy(output: &str) -> (Option<String>, Option<String>) {
    let mut map = std::collections::HashMap::new();
    for line in output.lines() {
        if let Some((key, value)) = line.split_once(" : ") {
            map.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    let pick = |prefix: &str| -> Option<String> {
        if map.get(&format!("{prefix}Enable")).map(String::as_str) != Some("1") {
            return None;
        }
        let host = map.get(&format!("{prefix}Proxy"))?.trim();
        let port = map.get(&format!("{prefix}Port"))?.trim();
        (!host.is_empty() && !port.is_empty()).then(|| format!("http://{host}:{port}"))
    };
    (pick("HTTPS"), pick("HTTP"))
}

#[cfg(test)]
mod host_origin_tests {
    use super::valid_host_origin;

    #[test]
    fn only_a_bare_loopback_origin_with_a_port_is_accepted() {
        assert!(valid_host_origin("http://127.0.0.1:57531"));
        assert!(valid_host_origin("http://localhost:1420/"));
    }

    #[test]
    fn userinfo_wrong_host_or_missing_port_is_rejected() {
        for origin in [
            "http://localhost:1@evil.example/",   // userinfo bypass the prefix match allowed
            "http://127.0.0.1@evil.example:80/",
            "http://localhost",                    // no port
            "http://evil.example:80/",
            "https://127.0.0.1:443/",              // not http
            "http://127.0.0.1:80/api",             // non-empty path
            "http://127.0.0.1:80/?x=1",
            "http://127.0.0.1:80/#f",
            "not a url",
        ] {
            assert!(!valid_host_origin(origin), "must reject {origin}");
        }
    }
}

