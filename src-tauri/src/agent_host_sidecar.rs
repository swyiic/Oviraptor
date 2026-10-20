//! Minimal desktop wiring: spawn the Node agent-host as a local sidecar and
//! expose origin/session to the webview. Does not touch Nest/Asset business kernels.

use serde::Serialize;
use std::{
    io::{BufRead, BufReader},
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
        .arg("--session-token")
        .arg(&session)
        .arg("--port")
        .arg("0")
        // No --runtime override: the host restores the saved runtime (xai-api after Grok OAuth).
        .env("OVIRAPTOR_BUNDLED_APPS_ROOT", &bundled_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .stdin(Stdio::null()))
        .map_err(|error| format!("failed to spawn agent-host sidecar: {error}"))?;

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
mod tests {
    use super::*;

    fn test_tree() -> PathBuf {
        let root = std::env::temp_dir().join(format!("oviraptor-sidecar-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn fixture_distribution(root: &Path) {
        for path in PACKAGED_HOST_FILES {
            let file = root.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, "synthetic fixture").unwrap();
        }
        std::fs::write(
            root.join("desktop-distribution.json"),
            serde_json::json!({
                "schemaVersion": 1, "applicationVersion": env!("CARGO_PKG_VERSION"),
                "minimumNodeMajor": 24, "nodeRuntimeBundled": false,
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            root.join("packages/agent-host/bundled-apps.json"),
            serde_json::json!({
                "schemaVersion": 1, "apps": [{"id": "fixture", "package": "apps/fixture-1.0.0"}],
            })
            .to_string(),
        )
        .unwrap();
        std::fs::create_dir_all(root.join("apps/fixture-1.0.0")).unwrap();
        std::fs::write(root.join("apps/fixture-1.0.0/manifest.json"), "{}").unwrap();
    }

    #[test]
    fn packaged_distribution_wins_over_debug_source_and_override() {
        let root = test_tree();
        let resources = root.join("resources");
        let packaged = resources.join(PACKAGED_HOST_ROOT);
        fixture_distribution(&packaged);
        let checkout = root.join("checkout");
        let source = checkout.join(PACKAGED_HOST_CLI);
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "debug fixture").unwrap();
        assert_eq!(
            resolve_cli_in(Some(&resources), Some(&checkout), Some(&source)).unwrap(),
            packaged.join(PACKAGED_HOST_CLI)
        );
        std::fs::remove_file(packaged.join("apps/fixture-1.0.0/manifest.json")).unwrap();
        assert!(resolve_cli_in(Some(&resources), Some(&checkout), Some(&source)).is_err());
        fixture_distribution(&packaged);
        std::fs::remove_file(packaged.join("packages/runtime-acp/index.mjs")).unwrap();
        assert!(
            resolve_cli_in(Some(&resources), Some(&checkout), Some(&source)).is_err(),
            "a partial package must not silently load the checkout"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn release_resolution_never_falls_back_to_a_checkout_or_cli_override() {
        let root = test_tree();
        let source = root.join(PACKAGED_HOST_CLI);
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "debug fixture").unwrap();
        assert!(resolve_cli_in(None, None, Some(&source)).is_err());
        assert_eq!(resolve_cli_in(None, Some(&root), None).unwrap(), source);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn node20_is_not_an_acceptable_sidecar_runtime() {
        assert_eq!(node_major("v24.12.0\n"), Some(24));
        assert!(select_node24(vec![(20, PathBuf::from("old-node"))]).is_err());
        assert_eq!(
            select_node24(vec![
                (20, PathBuf::from("old-node")),
                (24, PathBuf::from("new-node"))
            ])
            .unwrap(),
            PathBuf::from("new-node")
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_explicit_node_executable_is_version_checked_too() {
        use std::os::unix::fs::PermissionsExt;
        let root = test_tree();
        let executable = root.join("node");
        std::fs::write(&executable, "#!/bin/sh\nprintf 'v20.1.0\\n'\n").unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(require_node24(executable.clone()).is_err());
        std::fs::write(&executable, "#!/bin/sh\nprintf 'v24.1.0\\n'\n").unwrap();
        assert_eq!(require_node24(executable.clone()).unwrap(), executable);
        std::fs::remove_dir_all(root).unwrap();
    }

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
            (
                Some("http://127.0.0.1:7890".into()),
                Some("http://127.0.0.1:7890".into())
            )
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
        assert_eq!(
            env,
            vec![(
                "NO_PROXY".to_string(),
                "127.0.0.1,localhost,::1".to_string()
            )]
        );
        let off = "  HTTPSEnable : 0\n  HTTPSProxy : 1.2.3.4\n  HTTPSPort : 1\n";
        assert!(sidecar_proxy_env(|_| None, || Some(off.to_string())).is_empty());
    }
}
