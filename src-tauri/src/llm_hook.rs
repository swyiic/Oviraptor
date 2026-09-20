use crate::agent_runtime::model::{
    transport, CancelInitiator, TransportError, TransportEvent, TransportRequest,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::{Read, Write},
    net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use uuid::Uuid;

const MAX_REQUEST_BYTES: usize = 64 * 1024 * 1024;
const MAX_CAPTURE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Default, Debug)]
pub struct UsageTotals {
    pub requests: i64,
    pub maintenance_requests: i64,
    pub failed_requests: i64,
    pub maintenance_failed_requests: i64,
    pub context_errors: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_tokens: i64,
    pub total_tokens: i64,
    pub last_error: String,
    pub in_flight_requests: i64,
    pub in_flight_input_tokens: i64,
}

pub struct LlmHookHandle {
    stop: Arc<AtomicBool>,
    active_upstreams: Arc<Mutex<HashMap<String, TcpStream>>>,
    concurrency_gate: ConcurrencyGate,
    address: String,
    listener_address: String,
    thread: Option<JoinHandle<()>>,
}

impl LlmHookHandle {
    pub fn base_url(&self) -> &str {
        &self.address
    }
}

impl Drop for LlmHookHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.concurrency_gate.wake_all();
        if let Ok(mut active) = self.active_upstreams.lock() {
            for (_, stream) in active.drain() {
                let _ = stream.shutdown(Shutdown::Both);
            }
        }
        if let Ok(stream) = TcpStream::connect(&self.listener_address) {
            let _ = stream.shutdown(Shutdown::Both);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Clone)]
struct ConcurrencyGate {
    state: Arc<(Mutex<usize>, Condvar)>,
    max: usize,
}

impl ConcurrencyGate {
    fn new(max: usize) -> Self {
        Self {
            state: Arc::new((Mutex::new(0), Condvar::new())),
            max: max.max(1),
        }
    }

    fn acquire(&self, stop: &AtomicBool) -> Option<ConcurrencyPermit> {
        let (lock, condition) = &*self.state;
        let mut active = lock.lock().ok()?;
        while *active >= self.max {
            if stop.load(Ordering::Acquire) {
                return None;
            }
            active = condition
                .wait_timeout(active, Duration::from_millis(250))
                .ok()?
                .0;
        }
        if stop.load(Ordering::Acquire) {
            return None;
        }
        *active += 1;
        Some(ConcurrencyPermit {
            state: Arc::clone(&self.state),
        })
    }

    fn wake_all(&self) {
        self.state.1.notify_all();
    }
}

struct ConcurrencyPermit {
    state: Arc<(Mutex<usize>, Condvar)>,
}

impl Drop for ConcurrencyPermit {
    fn drop(&mut self) {
        let (lock, condition) = &*self.state;
        if let Ok(mut active) = lock.lock() {
            *active = active.saturating_sub(1);
            condition.notify_one();
        }
    }
}

struct ActiveUpstream {
    request_id: String,
    active: Arc<Mutex<HashMap<String, TcpStream>>>,
}

impl ActiveUpstream {
    fn register(
        request_id: &str,
        stream: &TcpStream,
        active: Arc<Mutex<HashMap<String, TcpStream>>>,
    ) -> Self {
        if let (Ok(clone), Ok(mut streams)) = (stream.try_clone(), active.lock()) {
            streams.insert(request_id.to_string(), clone);
        }
        Self {
            request_id: request_id.to_string(),
            active,
        }
    }
}

impl Drop for ActiveUpstream {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active.lock() {
            active.remove(&self.request_id);
        }
    }
}

#[derive(Clone)]
struct Upstream {
    scheme: String,
    host: String,
    port: u16,
    host_header: String,
    base_path: String,
    proxy: Option<String>,
    api_key: String,
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

#[allow(clippy::too_many_arguments)]
pub fn start(
    api_base: &str,
    api_key: &str,
    output_dir: &Path,
    capture_mode: &str,
    proxy: Option<&str>,
    max_output_tokens: Option<u64>,
    max_context_tokens: u64,
    max_concurrent_upstream: usize,
) -> Result<Option<LlmHookHandle>, String> {
    let Some(mut upstream) = parse_http_base(api_base)? else {
        return Ok(None);
    };
    upstream.api_key = api_key.trim().to_string();
    upstream.proxy = proxy
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    fs::create_dir_all(output_dir).map_err(|error| error.to_string())?;
    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|error| format!("LLM Hook 监听失败：{error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("LLM Hook 初始化失败：{error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    let base_path = upstream.base_path.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let active_upstreams = Arc::new(Mutex::new(HashMap::new()));
    let concurrency_gate = ConcurrencyGate::new(max_concurrent_upstream);
    let thread_stop = Arc::clone(&stop);
    let listener_active_upstreams = Arc::clone(&active_upstreams);
    let listener_concurrency_gate = concurrency_gate.clone();
    let output_path = output_dir.join("llm-hook.jsonl");
    let write_lock = Arc::new(Mutex::new(()));
    let mode = capture_mode.to_string();
    let thread = thread::spawn(move || {
        while !thread_stop.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((stream, _)) => {
                    // macOS hands the accepted socket the listener's O_NONBLOCK
                    // flag, so a read that loses the race against the client returns
                    // EAGAIN and the request used to be answered with a 400.
                    let _ = stream.set_nonblocking(false);
                    let upstream = upstream.clone();
                    let output_path = output_path.clone();
                    let write_lock = Arc::clone(&write_lock);
                    let mode = mode.clone();
                    let worker_stop = Arc::clone(&thread_stop);
                    let active_upstreams = Arc::clone(&listener_active_upstreams);
                    let concurrency_gate = listener_concurrency_gate.clone();
                    thread::spawn(move || {
                        handle_connection(
                            stream,
                            upstream,
                            &output_path,
                            &write_lock,
                            &mode,
                            max_output_tokens,
                            max_context_tokens,
                            &worker_stop,
                            active_upstreams,
                            &concurrency_gate,
                        );
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(25));
                }
                Err(_) => break,
            }
        }
    });
    Ok(Some(LlmHookHandle {
        stop,
        active_upstreams,
        concurrency_gate,
        address: format!("http://127.0.0.1:{port}{base_path}"),
        listener_address: format!("127.0.0.1:{port}"),
        thread: Some(thread),
    }))
}

fn call_type_for_request(request_value: &Value) -> &'static str {
    if is_context_compaction_request(request_value) {
        "context_compaction"
    } else if is_health_check_request(request_value) {
        "health_check"
    } else {
        "scan"
    }
}

fn is_context_compaction_request(value: &Value) -> bool {
    match value {
        Value::String(text) => {
            text.contains("You are compacting the earlier part of an autonomous security-testing agent's conversation")
                || text.contains("Conversation to summarise:")
        }
        Value::Array(items) => items.iter().any(is_context_compaction_request),
        Value::Object(map) => map.values().any(is_context_compaction_request),
        _ => false,
    }
}

fn is_maintenance_call_type(value: &str) -> bool {
    matches!(
        value,
        "context_compaction" | "health_check" | "scan_cancelled" | "scan_client_disconnected"
    )
}

fn is_aborted_call_type(value: &str) -> bool {
    matches!(value, "scan_cancelled" | "scan_client_disconnected")
}

fn is_health_check_request(value: &Value) -> bool {
    let Some(messages) = value.get("messages").and_then(Value::as_array) else {
        return false;
    };
    messages.iter().any(|message| {
        if message.get("role").and_then(Value::as_str) != Some("user") {
            return false;
        }
        let Some(content) = message.get("content").and_then(Value::as_str) else {
            return false;
        };
        matches!(
            content.trim().to_ascii_lowercase().as_str(),
            "reply with just 'ok'."
                | "reply with just \"ok\"."
                | "reply with just ok."
                | "respond with just 'ok'."
                | "respond with just \"ok\"."
                | "respond with just ok."
        )
    })
}

fn is_context_error(status: u16, error: &str) -> bool {
    if !matches!(status, 400 | 413 | 422) {
        return false;
    }
    let text = error.to_ascii_lowercase();
    [
        "context",
        "prompt",
        "token",
        "maximum length",
        "too long",
        "request too large",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

fn response_usage(value: &Value) -> Option<Value> {
    value
        .get("usage")
        .cloned()
        .or_else(|| value.pointer("/response/usage").cloned())
}

fn decode_response_body(response: &[u8]) -> (Vec<u8>, bool) {
    let Some(header_end) = response
        .windows(4)
        .position(|item| item == b"\r\n\r\n")
        .map(|position| position + 4)
    else {
        return (response.to_vec(), false);
    };
    let headers = String::from_utf8_lossy(&response[..header_end]).to_ascii_lowercase();
    let body = &response[header_end..];
    if !headers.contains("transfer-encoding: chunked") {
        return (body.to_vec(), false);
    }
    let mut decoded = Vec::new();
    let mut cursor = 0usize;
    while cursor < body.len() {
        let Some(line_end) = body[cursor..].windows(2).position(|item| item == b"\r\n") else {
            break;
        };
        let size_text = String::from_utf8_lossy(&body[cursor..cursor + line_end]);
        let Ok(size) = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16)
        else {
            break;
        };
        cursor += line_end + 2;
        if size == 0 || cursor + size > body.len() {
            break;
        }
        decoded.extend_from_slice(&body[cursor..cursor + size]);
        cursor += size + 2;
    }
    (decoded, true)
}

fn request_summary(value: &Value) -> Value {
    let messages = value
        .get("messages")
        .or_else(|| value.get("input"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let roles = messages.iter().map(|item| json!({
        "role": item.get("role").and_then(Value::as_str).unwrap_or("unknown"),
        "chars": item.get("content").map(|content| content.to_string().chars().count()).unwrap_or(0),
    })).collect::<Vec<_>>();
    json!({
        "messageCount": messages.len(),
        "messages": roles,
        "toolCount": value.get("tools").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
    })
}

fn bounded_capture(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), bounded_capture(value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(bounded_capture).collect()),
        Value::String(text) => {
            if text.chars().count() > 200_000 {
                Value::String(format!(
                    "{}\n[content truncated before persistence]",
                    text.chars().take(200_000).collect::<String>()
                ))
            } else {
                Value::String(text.clone())
            }
        }
        other => other.clone(),
    }
}

fn number(value: &Value, keys: &[&str]) -> i64 {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_i64))
        .unwrap_or(0)
}

fn cached_tokens(value: &Value) -> i64 {
    for key in ["input_tokens_details", "prompt_tokens_details"] {
        if let Some(details) = value.get(key) {
            if let Some(number) = details.get("cached_tokens").and_then(Value::as_i64) {
                return number;
            }
            if let Some(items) = details.as_array() {
                let total = items
                    .iter()
                    .filter_map(|item| item.get("cached_tokens").and_then(Value::as_i64))
                    .sum();
                if total > 0 {
                    return total;
                }
            }
        }
    }
    number(value, &["cached_tokens"])
}

fn write_error(stream: &mut TcpStream, status: u16, message: &str) {
    let body = json!({"error": message}).to_string();
    let _ = stream.write_all(format!("HTTP/1.1 {status} Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).as_bytes());
}
include!("llm_hook_record.rs");
include!("llm_hook_proxy.rs");
include!("llm_hook_context.rs");
include!("llm_hook_tests.rs");
