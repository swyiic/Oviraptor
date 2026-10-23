//! Private loopback bridge. Only the trusted desktop bootstrap gives the Host its bearer.
use crate::asset_discovery::certificate_tasks::Manager;
use serde::Deserialize;
use tauri::Manager as _;
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};
pub(crate) struct Bridge {
    pub origin: String,
    pub token: String,
    stop: Arc<AtomicBool>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Call {
    project_id: i64,
    action: String,
    args: Value,
    operation_id: String,
}
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0, |v, (a, b)| v | (a ^ b)) == 0
}
fn read(stream: &mut TcpStream, token: &str) -> Result<(String, Call), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(|_| "bridge_io")?;
    let mut buf = Vec::new();
    let mut block = [0; 4096];
    let end = loop {
        if buf.len() > 40 * 1024 {
            return Err("bridge_frame_too_large".into());
        }
        let n = stream.read(&mut block).map_err(|_| "bridge_io")?;
        if n == 0 {
            return Err("bridge_frame_invalid".into());
        }
        buf.extend_from_slice(&block[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
    };
    let header = std::str::from_utf8(&buf[..end]).map_err(|_| "bridge_frame_invalid")?;
    let mut lines = header.split("\r\n");
    let route = match lines.next() {
        Some("POST /certificate-task HTTP/1.1") => "certificate",
        Some("POST /scan-task HTTP/1.1") => "scan",
        Some("POST /discovery-enrichment HTTP/1.1") => "enrichment",
        Some("POST /discovery-assist HTTP/1.1") => "assist",
        Some("POST /exposure-task HTTP/1.1") => "exposure",
        _ => return Err("bridge_route_invalid".into()),
    };
    let mut length = None;
    let mut authorized = false;
    for line in lines.filter(|s| !s.is_empty()) {
        let (name, value) = line.split_once(':').ok_or("bridge_frame_invalid")?;
        match name.to_ascii_lowercase().as_str() {
            "authorization" => {
                if authorized {
                    return Err("bridge_auth_invalid".into());
                }
                authorized = same(value.trim(), &format!("Bearer {token}"));
            }
            "origin" | "transfer-encoding" => return Err("bridge_origin_denied".into()),
            "content-length" => {
                if length.is_some() {
                    return Err("bridge_frame_invalid".into());
                }
                length = Some(
                    value
                        .trim()
                        .parse::<usize>()
                        .map_err(|_| "bridge_frame_invalid")?,
                );
            }
            _ => {}
        }
    }
    if !authorized {
        return Err("bridge_auth_invalid".into());
    }
    let size = length
        .filter(|n| *n <= if route == "assist" { 256 * 1024 } else { 32 * 1024 })
        .ok_or("bridge_frame_invalid")?;
    while buf.len() < end + size {
        let n = stream.read(&mut block).map_err(|_| "bridge_io")?;
        if n == 0 {
            return Err("bridge_frame_invalid".into());
        }
        buf.extend_from_slice(&block[..n]);
        if buf.len() > end + if route == "assist" { 256 * 1024 } else { 32 * 1024 } {
            return Err("bridge_frame_too_large".into());
        }
    }
    if buf.len() != end + size {
        return Err("bridge_frame_invalid".into());
    }
    let c: Call = serde_json::from_slice(&buf[end..]).map_err(|_| "bridge_frame_invalid")?;
    if c.project_id < 1 || c.operation_id.is_empty() || c.operation_id.len() > 128 {
        return Err("bridge_context_invalid".into());
    }
    Ok((route.into(), c))
}
fn rejection_receipt(error: &str) -> Value {
    // These exact validations run before any committed task admission. An
    // arbitrary dispatch/storage failure is not proof that nothing started.
    let not_started = matches!(error,
        "请先在新建查询中确认允许补采的域名范围"
        | "certificate_arguments_invalid" | "certificate_action_invalid"
        | "certificate_operation_invalid" | "certificate_limits_invalid"
        | "certificate_selection_invalid" | "certificate_selection_outside_project"
        | "bridge_frame_too_large" | "bridge_frame_invalid" | "bridge_route_invalid"
        | "bridge_auth_invalid" | "bridge_origin_denied" | "bridge_context_invalid"
        | "discovery_arguments_invalid" | "discovery_project_unavailable"
        | "discovery_review_changed" | "discovery_action_invalid"
        | "seed_ledger_query_invalid" | "seed_ledger_seed_unavailable"
        | "seed_ledger_project_unavailable" | "seed_ledger_not_initialized"
        | "exposure_agent_arguments_invalid" | "exposure_agent_project_invalid"
        | "exposure_agent_project_unavailable" | "exposure_agent_project_archived"
        | "exposure_agent_run_unavailable" | "exposure_agent_action_invalid"
        | "exposure_agent_operation_invalid" | "exposure_agent_operation_conflict"
    );
    json!({"ok":false,"error":error,"phase":if not_started {"not_started"} else {"unknown"},"outcomeUnknown":!not_started})
}
fn handle(mut stream: TcpStream, token: &str, manager: &Manager, app: Option<&tauri::AppHandle>) {
    let outcome = read(&mut stream, token).and_then(|(route, c)| match route.as_str() {
        "certificate" => manager.dispatch(c.project_id, &c.action, c.args, &c.operation_id),
        "enrichment" => app.ok_or("discovery_executor_unavailable")?.state::<crate::asset_discovery::enrichment::Manager>().dispatch(c.project_id, &c.action, c.args, &c.operation_id),
        "assist" => crate::asset_discovery::review::assist::dispatch_chat(&app.ok_or("discovery_executor_unavailable")?.state::<crate::asset_discovery::review::assist::Manager>(),c.project_id,&c.action,c.args),
        "exposure" => crate::commands::exposure::agent_channel::dispatch_at(&app.ok_or("exposure_executor_unavailable")?.state::<crate::AppState>().db_path,c.project_id,&c.action,c.args,&c.operation_id)
            .map_err(|error|crate::commands::exposure::redaction::redact_text(&crate::commands::exposure::redaction::redact_url(&error))),
        _ => crate::scanning::agent_channel::dispatch(
            app.ok_or("scan_executor_unavailable")?,
            c.project_id,
            &c.action,
            c.args,
            &c.operation_id,
        ),
    });
    let (status, value) = match outcome {
        Ok(v) => (200, json!({"ok":true,"result":v})),
        Err(e) => (400, rejection_receipt(&e)),
    };
    let body = serde_json::to_vec(&value).unwrap_or_default();
    let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));
    let _=write!(stream,"HTTP/1.1 {status} Result\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",body.len());
    let _ = stream.write_all(&body);
}
impl Bridge {
    #[cfg(test)]
    pub(crate) fn start(manager: Manager) -> Result<Self, String> {
        Self::start_inner(manager, None)
    }
    pub(crate) fn start_with_scanning(
        manager: Manager,
        app: tauri::AppHandle,
    ) -> Result<Self, String> {
        Self::start_inner(manager, Some(app))
    }
    fn start_inner(manager: Manager, app: Option<tauri::AppHandle>) -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let origin = format!(
            "http://{}",
            listener.local_addr().map_err(|e| e.to_string())?
        );
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let key = token.clone();
        thread::spawn(move || {
            let active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            while !done.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        // Engine admission can take longer than a status read.
                        // Bound concurrency without blocking the accept loop or UI.
                        if active.fetch_add(1, Ordering::SeqCst) >= 8 {
                            active.fetch_sub(1, Ordering::SeqCst);
                            let _=stream.write_all(b"HTTP/1.1 503 Busy\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                            continue;
                        }
                        let count = active.clone();
                        let secret = key.clone();
                        let tasks = manager.clone();
                        let desktop = app.clone();
                        thread::spawn(move || {
                            struct Active(Arc<std::sync::atomic::AtomicUsize>);
                            impl Drop for Active {
                                fn drop(&mut self) {
                                    self.0.fetch_sub(1, Ordering::SeqCst);
                                }
                            }
                            let _active = Active(count);
                            handle(stream, &secret, &tasks, desktop.as_ref());
                        });
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(30))
                    }
                    Err(_) => break,
                }
            }
            manager.shutdown();
        });
        Ok(Self {
            origin,
            token,
            stop,
        })
    }
}
impl Drop for Bridge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst)
    }
}
#[cfg(test)]
mod rejection_tests {
    use super::*;
    #[test]
    fn only_proven_pre_admission_validations_are_known_not_started() {
        for message in ["请先在新建查询中确认允许补采的域名范围", "certificate_limits_invalid", "bridge_auth_invalid"] {
            let receipt = rejection_receipt(message);
            assert_eq!(receipt["phase"], "not_started");
            assert_eq!(receipt["outcomeUnknown"], false);
        }
        for message in ["database is locked", "certificate_operation_unavailable", "certificate_task_unavailable"] {
            let receipt = rejection_receipt(message);
            assert_eq!(receipt["phase"], "unknown");
            assert_eq!(receipt["outcomeUnknown"], true);
        }
    }
}
