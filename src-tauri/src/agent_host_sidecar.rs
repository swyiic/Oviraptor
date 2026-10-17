//! Minimal desktop wiring: spawn the Node agent-host as a local sidecar and
//! expose origin/session to the webview. Does not touch Nest/Asset business kernels.

use serde::Serialize;
use std::{
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{mpsc, Mutex},
    thread,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentHostEndpoint {
    pub origin: String,
    pub session: String,
    pub source: String,
}

pub struct AgentHostSidecar {
    child: Mutex<Option<Child>>,
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
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

impl Drop for AgentHostSidecar {
    fn drop(&mut self) {
        self.stop();
    }
}

fn env_endpoint() -> Option<AgentHostEndpoint> {
    let origin = std::env::var("OVIRAPTOR_HOST_ORIGIN").ok()?.trim().to_string();
    let session = std::env::var("OVIRAPTOR_HOST_SESSION").ok()?.trim().to_string();
    if origin.is_empty() || session.is_empty() {
        return None;
    }
    if !(origin.starts_with("http://127.0.0.1:") || origin.starts_with("http://localhost:")) {
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

fn resolve_node() -> Result<PathBuf, String> {
    if let Ok(configured) = std::env::var("OVIRAPTOR_NODE_EXECUTABLE") {
        let path = PathBuf::from(configured);
        if path.is_file() {
            return Ok(path);
        }
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let runtime_path = crate::worker_resources::sentinel_runtime_path(&home);
    let candidates = crate::worker_resources::helper_node_candidates(&runtime_path, None);
    let mut scored: Vec<(u32, PathBuf)> = Vec::new();
    for path in candidates.into_iter().filter(|path| path.is_file()) {
        let Ok(output) = Command::new(&path).arg("-v").output() else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let version = text.trim().trim_start_matches('v');
        let major = version
            .split('.')
            .next()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0);
        scored.push((major, path));
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    if let Some((_, path)) = scored.iter().find(|(major, _)| *major >= 24) {
        return Ok(path.clone());
    }
    if let Some((_, path)) = scored.iter().find(|(major, _)| *major >= 20) {
        return Ok(path.clone());
    }
    Err("agent-host sidecar needs Node.js 24+ (20+ accepted as fallback)".into())
}

fn resolve_cli(app: &AppHandle) -> Result<PathBuf, String> {
    if let Ok(configured) = std::env::var("OVIRAPTOR_AGENT_HOST_CLI") {
        let path = PathBuf::from(configured);
        if path.is_file() {
            return Ok(path);
        }
    }
    let mut candidates = Vec::new();
    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(resource_dir.join("resources/agent-host/cli.mjs"));
        candidates.push(resource_dir.join("agent-host/cli.mjs"));
        candidates.push(resource_dir.join("packages/agent-host/src/cli.mjs"));
    }
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../packages/agent-host/src/cli.mjs"),
    );
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| "agent-host cli.mjs was not found for sidecar launch".into())
}

fn parse_ready_line(line: &str, origin: &mut Option<String>, session: &mut Option<String>) {
    let trimmed = line.trim();
    if let Some(value) = trimmed.strip_prefix("Oviraptor host:") {
        *origin = Some(value.trim().trim_end_matches('/').to_string());
    } else if let Some(value) = trimmed.strip_prefix("Session:") {
        *session = Some(value.trim().to_string());
    }
}

fn spawn_sidecar(app: &AppHandle, data_dir: &Path) -> Result<(Child, AgentHostEndpoint), String> {
    let node = resolve_node()?;
    let cli = resolve_cli(app)?;
    let host_data = data_dir.join("agent-host");
    std::fs::create_dir_all(&host_data).map_err(|error| error.to_string())?;
    let session = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    // Point ensureBundledApps at the monorepo / resource root that holds
    // packages/agent-host/bundled-apps.json and .agent-build/apps.
    let bundled_root = cli
        .parent() // src
        .and_then(|p| p.parent()) // agent-host
        .and_then(|p| p.parent()) // packages
        .and_then(|p| p.parent()) // repo or resource root
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".."));

    let mut command = Command::new(&node);
    for (key, value) in sidecar_proxy_env(
        |key| std::env::var(key).ok().filter(|v| !v.trim().is_empty()),
        read_scutil_proxy,
    ) {
        command.env(key, value);
    }
    let mut child = command
        .arg(&cli)
        .arg("--data-dir")
        .arg(&host_data)
        .arg("--session-token")
        .arg(&session)
        .arg("--port")
        .arg("0")
        // No --runtime override: the host restores the saved runtime (xai-api after Grok OAuth).
        .env("OVIRAPTOR_BUNDLED_APPS_ROOT", &bundled_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .stdin(Stdio::null())
        .spawn()
        .map_err(|error| format!("failed to spawn agent-host sidecar: {error}"))?;

    let stdout = child
        .stdout
        .take()
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
            let _ = child.kill();
            return Err("agent-host sidecar startup timed out".into());
        }
        match rx.recv_timeout(remaining) {
            Ok(line) => parse_ready_line(&line, &mut origin, &mut reported_session),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let _ = child.kill();
                return Err("agent-host sidecar startup timed out".into());
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = child.kill();
                return Err("agent-host sidecar exited before becoming ready".into());
            }
        }
    }
    let origin = origin.unwrap();
    let session = reported_session.unwrap_or(session);
    if !session.chars().all(|c| c.is_ascii_hexdigit()) || !(32..=128).contains(&session.len()) {
        let _ = child.kill();
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
            std::env::set_var("OVIRAPTOR_HOST_ORIGIN", &endpoint.origin);
            std::env::set_var("OVIRAPTOR_HOST_SESSION", &endpoint.session);
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
        let mut no_proxy = vec!["127.0.0.1".to_string(), "localhost".to_string(), "::1".to_string()];
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
    let output = Command::new("/usr/sbin/scutil").arg("--proxy").output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
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
mod tests {
    use super::*;

    #[test]
    fn parses_host_ready_lines() {
        let mut origin = None;
        let mut session = None;
        parse_ready_line(
            "Oviraptor host: http://127.0.0.1:43123",
            &mut origin,
            &mut session,
        );
        parse_ready_line(
            "Session: abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
            &mut origin,
            &mut session,
        );
        assert_eq!(origin.as_deref(), Some("http://127.0.0.1:43123"));
        assert_eq!(
            session.as_deref(),
            Some("abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789")
        );
    }

    #[test]
    fn parses_scutil_proxy_and_injects_env() {
        let out = "<dictionary> {\n  HTTPEnable : 1\n  HTTPPort : 7890\n  HTTPProxy : 127.0.0.1\n  HTTPSEnable : 1\n  HTTPSPort : 7890\n  HTTPSProxy : 127.0.0.1\n  SOCKSEnable : 1\n  SOCKSPort : 7890\n  SOCKSProxy : 127.0.0.1\n}\n";
        assert_eq!(
            parse_scutil_proxy(out),
            (Some("http://127.0.0.1:7890".into()), Some("http://127.0.0.1:7890".into()))
        );
        let env = sidecar_proxy_env(|_| None, || Some(out.to_string()));
        assert!(env.contains(&("HTTPS_PROXY".into(), "http://127.0.0.1:7890".into())));
        assert!(env.contains(&("HTTP_PROXY".into(), "http://127.0.0.1:7890".into())));
        assert!(env.contains(&("NO_PROXY".into(), "127.0.0.1,localhost,::1".into())));
    }

    #[test]
    fn parent_proxy_env_wins_and_disabled_system_proxy_is_ignored() {
        let env = sidecar_proxy_env(
            |k| (k == "HTTPS_PROXY").then(|| "http://corp:8080".to_string()),
            || panic!("must not read system proxy"),
        );
        assert_eq!(env, vec![("NO_PROXY".to_string(), "127.0.0.1,localhost,::1".to_string())]);
        let off = "  HTTPSEnable : 0\n  HTTPSProxy : 1.2.3.4\n  HTTPSPort : 1\n";
        assert!(sidecar_proxy_env(|_| None, || Some(off.to_string())).is_empty());
    }
}
