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
    time::Duration,
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

pub fn usage_from_file(path: &Path) -> UsageTotals {
    let Ok(text) = fs::read_to_string(path) else {
        return UsageTotals::default();
    };
    let mut totals = UsageTotals::default();
    let mut in_flight = HashMap::new();
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let request_id = value.get("requestId").and_then(Value::as_str).unwrap_or("");
        let maintenance = value
            .get("callType")
            .and_then(Value::as_str)
            .is_some_and(is_maintenance_call_type);
        if value.get("kind").and_then(Value::as_str) == Some("model_call_started") {
            // Health checks also perform real provider work (and may load a
            // 27B model), so keep them visible as active inference until the
            // matching completion record arrives.
            if !request_id.is_empty() {
                in_flight.insert(
                    request_id.to_string(),
                    value
                        .get("estimatedInputTokens")
                        .and_then(Value::as_i64)
                        .unwrap_or(0),
                );
            }
            continue;
        }
        if !request_id.is_empty() {
            in_flight.remove(request_id);
        }
        if value
            .get("callType")
            .and_then(Value::as_str)
            .is_some_and(is_aborted_call_type)
        {
            continue;
        }
        let status = value
            .get("status")
            .and_then(Value::as_str)
            .and_then(|value| value.parse::<u16>().ok());
        // Older hook records did not persist a status field. Preserve their
        // historical accounting; only an explicit non-2xx response is a
        // failed model attempt.
        let success = status
            .map(|value| (200..300).contains(&value))
            .unwrap_or(true);
        if !success {
            if maintenance {
                totals.maintenance_failed_requests += 1;
                continue;
            }
            totals.failed_requests += 1;
            let error = value
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            let context_error = is_context_error(status.unwrap_or(0), error);
            if context_error {
                totals.context_errors += 1;
            }
            if !error.is_empty() {
                totals.last_error = error.to_string();
            }
            continue;
        }
        let usage = value.get("usage").unwrap_or(&Value::Null);
        totals.requests += 1;
        if maintenance {
            totals.maintenance_requests += 1;
        }
        totals.input_tokens += number(usage, &["input_tokens", "prompt_tokens"]);
        totals.output_tokens += number(usage, &["output_tokens", "completion_tokens"]);
        totals.cached_tokens += cached_tokens(usage);
        let total = number(usage, &["total_tokens"]);
        totals.total_tokens += if total > 0 {
            total
        } else {
            number(usage, &["input_tokens", "prompt_tokens"])
                + number(usage, &["output_tokens", "completion_tokens"])
        };
    }
    totals.in_flight_requests = in_flight.len() as i64;
    totals.in_flight_input_tokens = in_flight.values().sum();
    totals
}

pub fn records_from_file(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect()
}

fn parse_http_base(value: &str) -> Result<Option<Upstream>, String> {
    let value = value.trim();
    let (scheme, rest, default_port) = if let Some(rest) = value.strip_prefix("http://") {
        ("http", rest, 80)
    } else if let Some(rest) = value.strip_prefix("https://") {
        ("https", rest, 443)
    } else {
        return Ok(None);
    };
    let authority = rest.split('/').next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') || authority.contains('?') {
        return Err("本地 LLM Hook 只接受不含凭据的 HTTP 地址".into());
    }
    let base_path = rest
        .split_once('/')
        .map(|(_, path)| format!("/{path}"))
        .unwrap_or_default();
    let (host, port, host_header) = if let Some(raw) = authority.strip_prefix('[') {
        let Some((host, rest)) = raw.split_once(']') else {
            return Err("本地 LLM 地址的 IPv6 格式无效".into());
        };
        let port = rest
            .strip_prefix(':')
            .unwrap_or(if default_port == 443 { "443" } else { "80" })
            .parse::<u16>()
            .map_err(|_| "本地 LLM 端口无效")?;
        (host.to_string(), port, format!("[{host}]:{port}"))
    } else if let Some((host, raw_port)) = authority.rsplit_once(':') {
        let port = raw_port.parse::<u16>().map_err(|_| "本地 LLM 端口无效")?;
        (host.to_string(), port, authority.to_string())
    } else {
        (authority.to_string(), default_port, authority.to_string())
    };
    if host.is_empty() {
        return Err("本地 LLM 地址缺少主机名".into());
    }
    Ok(Some(Upstream {
        scheme: scheme.to_string(),
        host,
        port,
        host_header,
        base_path,
        proxy: None,
        api_key: String::new(),
    }))
}

fn handle_connection(
    mut client: TcpStream,
    upstream: Upstream,
    output_path: &Path,
    write_lock: &Mutex<()>,
    capture_mode: &str,
    max_output_tokens: Option<u64>,
    max_context_tokens: u64,
    stop: &Arc<AtomicBool>,
    active_upstreams: Arc<Mutex<HashMap<String, TcpStream>>>,
    concurrency_gate: &ConcurrencyGate,
) {
    let request = match read_request(&mut client) {
        Ok(request) => request,
        Err(error) => {
            write_error(&mut client, 400, &error);
            return;
        }
    };
    let mut request = request;
    let original_request_value =
        serde_json::from_slice::<Value>(&request.body).unwrap_or_else(|_| json!({}));
    if let Some(limit) = max_output_tokens {
        // Strix performs a real provider health request before the scan. A
        // large default generation allowance is pointless for “OK” and can
        // amplify first-load CPU time on reasoning-oriented 27B models.
        let effective_limit = if is_health_check_request(&original_request_value) {
            limit.min(64)
        } else {
            limit
        };
        request.body = clamp_output_tokens(&request.body, effective_limit);
    }
    let (guarded_body, context_guard) =
        guard_local_model_context(&request.body, max_context_tokens);
    request.body = guarded_body;
    let request_value =
        serde_json::from_slice::<Value>(&request.body).unwrap_or_else(|_| json!({}));
    let request_id = Uuid::new_v4().to_string();
    let call_type = call_type_for_request(&request_value);
    let Some(_permit) = concurrency_gate.acquire(stop) else {
        append_cancelled_record(output_path, write_lock, &request_id, &request_value);
        write_error(&mut client, 503, "任务已停止，本地模型请求未执行");
        return;
    };
    append_record(
        output_path,
        write_lock,
        &json!({
            "kind":"model_call_started",
            "requestId":request_id,
            "callType":call_type,
            "recordedAt":chrono::Utc::now().to_rfc3339(),
            "path":request.path,
            "model":request_value.get("model").cloned().unwrap_or(Value::Null),
            "stream":request_value.get("stream").cloned().unwrap_or(Value::Bool(false)),
            "requestChars":request.body.len(),
            "estimatedInputTokens":((request.body.len() as i64 + 3) / 4).max(1),
            // Structural telemetry contains no prompt text, credentials or
            // tool arguments, but keeps context growth diagnosable when full
            // prompt auditing is disabled.
            "requestSummary":request_summary(&request_value),
            "contextGuard":context_guard,
        }),
    );
    if upstream.scheme == "https" {
        handle_https_request(
            &mut client,
            request,
            upstream,
            output_path,
            write_lock,
            capture_mode,
            max_output_tokens,
            &request_id,
        );
        return;
    }
    let mut upstream_stream = match (upstream.host.as_str(), upstream.port)
        .to_socket_addrs()
        .ok()
        .and_then(|mut addresses| {
            addresses.find_map(|address| {
                TcpStream::connect_timeout(&address, Duration::from_secs(15)).ok()
            })
        }) {
        Some(stream) => stream,
        None => {
            append_failure_record(
                output_path,
                write_lock,
                &request_id,
                &request_value,
                502,
                "无法连接本地 LLM 上游地址",
            );
            write_error(&mut client, 502, "无法连接本地 LLM 上游地址");
            return;
        }
    };
    let _active_upstream =
        ActiveUpstream::register(&request_id, &upstream_stream, active_upstreams);
    // A short read poll lets task cancellation close an inference immediately.
    // The previous four-hour blocking timeout left detached 27B generations
    // consuming CPU after Strix and its UI task had already stopped.
    let _ = upstream_stream.set_read_timeout(Some(Duration::from_secs(1)));
    let _ = upstream_stream.set_write_timeout(Some(Duration::from_secs(60)));
    let outbound = build_request(&request, &upstream.host_header, &upstream.api_key);
    if upstream_stream.write_all(&outbound).is_err() || upstream_stream.flush().is_err() {
        append_failure_record(
            output_path,
            write_lock,
            &request_id,
            &request_value,
            502,
            "无法转发本地 LLM 请求",
        );
        write_error(&mut client, 502, "无法转发本地 LLM 请求");
        return;
    }
    let mut response = Vec::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        if stop.load(Ordering::Acquire) {
            append_cancelled_record(output_path, write_lock, &request_id, &request_value);
            return;
        }
        match upstream_stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                if client.write_all(&buffer[..count]).is_err() {
                    let _ = upstream_stream.shutdown(Shutdown::Both);
                    append_client_disconnected_record(
                        output_path,
                        write_lock,
                        &request_id,
                        &request_value,
                    );
                    return;
                }
                if response.len() < MAX_CAPTURE_BYTES {
                    response.extend_from_slice(
                        &buffer[..count.min(MAX_CAPTURE_BYTES - response.len())],
                    );
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                continue;
            }
            Err(_) => break,
        }
    }
    if stop.load(Ordering::Acquire) {
        append_cancelled_record(output_path, write_lock, &request_id, &request_value);
        return;
    }
    let _ = client.flush();
    let record = build_record(
        &request,
        &request_value,
        &response,
        capture_mode,
        &request_id,
    );
    append_record(output_path, write_lock, &record);
}

fn handle_https_request(
    client: &mut TcpStream,
    request: Request,
    upstream: Upstream,
    output_path: &Path,
    write_lock: &Mutex<()>,
    capture_mode: &str,
    max_output_tokens: Option<u64>,
    request_id: &str,
) {
    let mut request = request;
    if let Some(limit) = max_output_tokens {
        request.body = clamp_output_tokens(&request.body, limit);
    }
    let request_value =
        serde_json::from_slice::<Value>(&request.body).unwrap_or_else(|_| json!({}));
    let mut builder = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(14_400));
    if let Some(proxy) = upstream.proxy.as_deref() {
        match reqwest::Proxy::all(proxy) {
            Ok(proxy) => builder = builder.proxy(proxy),
            Err(error) => {
                append_failure_record(
                    output_path,
                    write_lock,
                    request_id,
                    &request_value,
                    502,
                    &format!("LLM Hook 代理配置无效：{error}"),
                );
                write_error(client, 502, &format!("LLM Hook 代理配置无效：{error}"));
                return;
            }
        }
    }
    let http = match builder.build() {
        Ok(client) => client,
        Err(error) => {
            append_failure_record(
                output_path,
                write_lock,
                request_id,
                &request_value,
                502,
                &format!("LLM Hook HTTPS 客户端初始化失败：{error}"),
            );
            write_error(
                client,
                502,
                &format!("LLM Hook HTTPS 客户端初始化失败：{error}"),
            );
            return;
        }
    };
    let method = request
        .method
        .parse::<reqwest::Method>()
        .unwrap_or(reqwest::Method::POST);
    let url = format!("https://{}{}", upstream.host_header, request.path);
    let mut outbound = http.request(method, url);
    for (key, value) in &request.headers {
        if key.eq_ignore_ascii_case("host")
            || key.eq_ignore_ascii_case("content-length")
            || key.eq_ignore_ascii_case("connection")
            || key.eq_ignore_ascii_case("accept-encoding")
            || key.eq_ignore_ascii_case("authorization")
        {
            continue;
        }
        outbound = outbound.header(key, value);
    }
    if !upstream.api_key.is_empty() {
        outbound = outbound.bearer_auth(&upstream.api_key);
    }
    let mut response = match outbound.body(request.body.clone()).send() {
        Ok(response) => response,
        Err(error) => {
            append_failure_record(
                output_path,
                write_lock,
                request_id,
                &request_value,
                502,
                &format!("无法连接云端 LLM 上游地址：{error}"),
            );
            write_error(client, 502, &format!("无法连接云端 LLM 上游地址：{error}"));
            return;
        }
    };
    let status = response.status();
    let mut response_head = format!(
        "HTTP/1.1 {} {}\r\n",
        status.as_u16(),
        status.canonical_reason().unwrap_or("")
    );
    for (key, value) in response.headers() {
        if key.as_str().eq_ignore_ascii_case("connection")
            || key.as_str().eq_ignore_ascii_case("transfer-encoding")
        {
            continue;
        }
        if let Ok(value) = value.to_str() {
            response_head.push_str(key.as_str());
            response_head.push_str(": ");
            response_head.push_str(value);
            response_head.push_str("\r\n");
        }
    }
    response_head.push_str("Connection: close\r\n\r\n");
    if client.write_all(response_head.as_bytes()).is_err() {
        return;
    }
    let mut captured = response_head.into_bytes();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        match response.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                if client.write_all(&buffer[..count]).is_err() {
                    return;
                }
                if captured.len() < MAX_CAPTURE_BYTES {
                    captured.extend_from_slice(
                        &buffer[..count.min(MAX_CAPTURE_BYTES - captured.len())],
                    );
                }
            }
            Err(_) => break,
        }
    }
    let _ = client.flush();
    let record = build_record(
        &request,
        &request_value,
        &captured,
        capture_mode,
        request_id,
    );
    append_record(output_path, write_lock, &record);
}

fn append_record(output_path: &Path, write_lock: &Mutex<()>, record: &Value) {
    if let Ok(_guard) = write_lock.lock() {
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(output_path)
        {
            let _ = serde_json::to_writer(&mut file, &record);
            let _ = file.write_all(b"\n");
        }
    }
}

fn append_failure_record(
    output_path: &Path,
    write_lock: &Mutex<()>,
    request_id: &str,
    request_value: &Value,
    status: u16,
    error: &str,
) {
    append_record(
        output_path,
        write_lock,
        &json!({
            "kind":"model_call",
            "requestId":request_id,
            "callType":call_type_for_request(request_value),
            "recordedAt":chrono::Utc::now().to_rfc3339(),
            "status":status.to_string(),
            "model":request_value.get("model").cloned().unwrap_or(Value::Null),
            "usage":{},
            "error":error.chars().take(500).collect::<String>(),
        }),
    );
}

fn append_cancelled_record(
    output_path: &Path,
    write_lock: &Mutex<()>,
    request_id: &str,
    request_value: &Value,
) {
    append_record(
        output_path,
        write_lock,
        &json!({
            "kind":"model_call",
            "requestId":request_id,
            "callType":"scan_cancelled",
            "recordedAt":chrono::Utc::now().to_rfc3339(),
            "status":"499",
            "model":request_value.get("model").cloned().unwrap_or(Value::Null),
            "usage":{},
            "error":"Oviraptor 任务已停止，已断开本地模型上游推理",
        }),
    );
}

fn append_client_disconnected_record(
    output_path: &Path,
    write_lock: &Mutex<()>,
    request_id: &str,
    request_value: &Value,
) {
    append_record(
        output_path,
        write_lock,
        &json!({
            "kind":"model_call",
            "requestId":request_id,
            "callType":"scan_client_disconnected",
            "recordedAt":chrono::Utc::now().to_rfc3339(),
            "status":"499",
            "model":request_value.get("model").cloned().unwrap_or(Value::Null),
            "usage":{},
            "error":"Strix 在模型响应返回前关闭了本次连接；常见原因是单次模型调用达到 LLM_TIMEOUT，不能记为用户停止任务",
        }),
    );
}

fn clamp_output_tokens(body: &[u8], limit: u64) -> Vec<u8> {
    let Ok(mut value) = serde_json::from_slice::<Value>(body) else {
        return body.to_vec();
    };
    let Some(object) = value.as_object_mut() else {
        return body.to_vec();
    };
    let mut changed = false;
    for key in ["max_tokens", "max_completion_tokens"] {
        if let Some(current) = object.get(key).and_then(Value::as_u64) {
            if current > limit {
                object.insert(key.to_string(), Value::from(limit));
                changed = true;
            }
        }
    }
    if !object.contains_key("max_tokens") && !object.contains_key("max_completion_tokens") {
        object.insert("max_tokens".into(), Value::from(limit));
        changed = true;
    }
    if changed {
        serde_json::to_vec(&value).unwrap_or_else(|_| body.to_vec())
    } else {
        body.to_vec()
    }
}

/// Keeps local OpenAI-compatible requests below the configured context window
/// without spending another model call on summarisation. Strix can append a
/// long prose answer and then force a recovery turn when the root agent forgets
/// its lifecycle tool. Keeping the complete tool transcript in that recovery
/// request used to make an otherwise successful scan fail at the provider's
/// hard context boundary.
fn guard_local_model_context(body: &[u8], max_context_tokens: u64) -> (Vec<u8>, Value) {
    if max_context_tokens == 0 {
        return (body.to_vec(), Value::Null);
    }
    let Ok(mut value) = serde_json::from_slice::<Value>(body) else {
        return (body.to_vec(), Value::Null);
    };
    if is_health_check_request(&value) || is_context_compaction_request(&value) {
        return (body.to_vec(), Value::Null);
    }
    let before_tokens = estimated_request_tokens(body);
    let before_messages = value
        .get("messages")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let lifecycle_recovery = contains_lifecycle_recovery(&value);
    let trigger_tokens = max_context_tokens.saturating_mul(94) / 100;
    if !lifecycle_recovery && before_tokens <= trigger_tokens {
        return (body.to_vec(), Value::Null);
    }

    let removed_messages = compact_conversation(&mut value, true, 2_400);
    let mut filtered_tools = 0usize;
    if lifecycle_recovery {
        filtered_tools = retain_lifecycle_tools(&mut value);
    }
    let mut trimmed_descriptions = 0usize;
    let target_tokens = max_context_tokens.saturating_mul(92) / 100;
    if serialized_estimated_tokens(&value) > target_tokens {
        trim_named_strings(&mut value, "description", 320, &mut trimmed_descriptions);
    }
    if serialized_estimated_tokens(&value) > target_tokens {
        // The assistant's prose is useful context but is never authoritative;
        // the original task and recovery instruction are the durable contract.
        compact_conversation(&mut value, false, 0);
    }
    if serialized_estimated_tokens(&value) > target_tokens {
        compact_user_messages(&mut value, 3_000);
        trim_named_strings(&mut value, "description", 160, &mut trimmed_descriptions);
    }

    let guarded = serde_json::to_vec(&value).unwrap_or_else(|_| body.to_vec());
    let after_tokens = estimated_request_tokens(&guarded);
    let after_messages = value
        .get("messages")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    (
        guarded,
        json!({
            "applied": true,
            "reason": if lifecycle_recovery { "lifecycle_recovery" } else { "context_headroom" },
            "maxContextTokens": max_context_tokens,
            "beforeEstimatedTokens": before_tokens,
            "afterEstimatedTokens": after_tokens,
            "beforeMessages": before_messages,
            "afterMessages": after_messages,
            "removedMessages": removed_messages,
            "filteredTools": filtered_tools,
            "trimmedToolDescriptions": trimmed_descriptions,
        }),
    )
}

fn estimated_request_tokens(body: &[u8]) -> u64 {
    ((body.len() as u64 + 3) / 4).max(1)
}

fn serialized_estimated_tokens(value: &Value) -> u64 {
    serde_json::to_vec(value)
        .map(|body| estimated_request_tokens(&body))
        .unwrap_or(u64::MAX)
}

fn contains_lifecycle_recovery(value: &Value) -> bool {
    // Strix's ordinary system prompt documents `finish_scan` and lifecycle
    // tool calls. Searching the complete request therefore classified every
    // first scan turn as recovery and removed the HTTP/browser tools before
    // the model could use them. Recovery is a protocol message emitted by
    // Strix itself, so only accept its exact marker in the latest user turn.
    let Some(latest) = value
        .get("messages")
        .or_else(|| value.get("input"))
        .and_then(Value::as_array)
        .and_then(|messages| messages.last())
        .filter(|message| message_role(message) == "user")
    else {
        return false;
    };
    let lower = message_content_text(latest).to_ascii_lowercase();
    lower.contains(
        "your previous response ended the autonomous strix run without a lifecycle tool call",
    ) && lower.contains("this is recovery attempt")
}

fn compact_conversation(
    value: &mut Value,
    keep_assistant_summary: bool,
    assistant_chars: usize,
) -> usize {
    let Some(messages) = value.get_mut("messages").and_then(Value::as_array_mut) else {
        return 0;
    };
    if messages.len() <= 2 {
        return 0;
    }
    let original_len = messages.len();
    let system = messages
        .iter()
        .position(|message| message_role(message) == "system");
    let first_user = messages
        .iter()
        .position(|message| message_role(message) == "user");
    let last_user = messages
        .iter()
        .rposition(|message| message_role(message) == "user");
    let last_assistant = last_user.and_then(|user_index| {
        messages[..user_index]
            .iter()
            .rposition(|message| message_role(message) == "assistant")
    });
    let mut selected = Vec::new();
    for index in [system, first_user]
        .into_iter()
        .flatten()
        .chain(
            keep_assistant_summary
                .then_some(last_assistant)
                .flatten()
                .into_iter(),
        )
        .chain(last_user.into_iter())
    {
        if !selected.contains(&index) {
            selected.push(index);
        }
    }
    selected.sort_unstable();
    let mut compacted = selected
        .into_iter()
        .filter_map(|index| messages.get(index).cloned())
        .collect::<Vec<_>>();
    if keep_assistant_summary {
        if let Some(message) = compacted
            .iter_mut()
            .find(|message| message_role(message) == "assistant")
        {
            let text = message_content_text(message);
            if text.trim().is_empty() {
                compacted.retain(|item| message_role(item) != "assistant");
            } else if let Some(object) = message.as_object_mut() {
                object.remove("tool_calls");
                object.remove("function_call");
                object.remove("tool_call_id");
                object.insert(
                    "content".into(),
                    Value::String(format!(
                        "[Earlier execution compacted deterministically]\n{}",
                        compact_text(&text, assistant_chars)
                    )),
                );
            }
        }
    }
    *messages = compacted;
    original_len.saturating_sub(messages.len())
}

fn message_role(message: &Value) -> &str {
    message.get("role").and_then(Value::as_str).unwrap_or("")
}

fn message_content_text(message: &Value) -> String {
    match message.get("content") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| {
                item.get("text")
                    .and_then(Value::as_str)
                    .or_else(|| item.as_str())
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn compact_text(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    if max_chars < 160 {
        return text.chars().take(max_chars).collect();
    }
    let tail = (max_chars / 4).max(80);
    let head = max_chars.saturating_sub(tail);
    format!(
        "{}\n[... compacted ...]\n{}",
        text.chars().take(head).collect::<String>(),
        text.chars()
            .skip(count.saturating_sub(tail))
            .collect::<String>()
    )
}

fn compact_user_messages(value: &mut Value, max_chars: usize) {
    let Some(messages) = value.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    for message in messages
        .iter_mut()
        .filter(|message| message_role(message) == "user")
    {
        let text = message_content_text(message);
        if text.chars().count() > max_chars {
            if let Some(object) = message.as_object_mut() {
                object.insert(
                    "content".into(),
                    Value::String(compact_text(&text, max_chars)),
                );
            }
        }
    }
}

fn retain_lifecycle_tools(value: &mut Value) -> usize {
    let Some(tools) = value.get_mut("tools").and_then(Value::as_array_mut) else {
        return 0;
    };
    let before = tools.len();
    let has_finish = tools.iter().any(|tool| tool_name(tool) == "finish_scan");
    if !has_finish {
        return 0;
    }
    tools.retain(|tool| {
        matches!(
            tool_name(tool),
            "finish_scan"
                | "wait_for_message"
                | "view_agent_graph"
                | "send_message_to_agent"
                | "stop_agent"
        )
    });
    before.saturating_sub(tools.len())
}

fn tool_name(tool: &Value) -> &str {
    tool.pointer("/function/name")
        .or_else(|| tool.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("")
}

fn trim_named_strings(value: &mut Value, key: &str, max_chars: usize, changed: &mut usize) {
    match value {
        Value::Object(map) => {
            for (name, child) in map.iter_mut() {
                if name == key {
                    if let Value::String(text) = child {
                        if text.chars().count() > max_chars {
                            *text = compact_text(text, max_chars);
                            *changed += 1;
                        }
                    }
                } else {
                    trim_named_strings(child, key, max_chars, changed);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                trim_named_strings(item, key, max_chars, changed);
            }
        }
        _ => {}
    }
}

fn read_request(stream: &mut TcpStream) -> Result<Request, String> {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 32 * 1024];
    let header_end = loop {
        let count = stream
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if count == 0 {
            return Err("LLM 请求提前关闭".into());
        }
        bytes.extend_from_slice(&buffer[..count]);
        if bytes.len() > MAX_REQUEST_BYTES {
            return Err("LLM 请求过大".into());
        }
        if let Some(position) = bytes.windows(4).position(|item| item == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let header_text = String::from_utf8_lossy(&bytes[..header_end]);
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().ok_or("缺少 LLM 请求行")?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("POST").to_string();
    let path = parts.next().unwrap_or("/").to_string();
    let mut headers = Vec::new();
    for line in lines {
        if let Some((key, value)) = line.split_once(':') {
            headers.push((key.trim().to_string(), value.trim().to_string()));
        }
    }
    let length = headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.parse::<usize>().ok())
        .unwrap_or(0);
    if length > MAX_REQUEST_BYTES {
        return Err("LLM 请求正文过大".into());
    }
    while bytes.len() < header_end + length {
        let count = stream
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if count == 0 {
            return Err("LLM 请求正文不完整".into());
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    Ok(Request {
        method,
        path,
        headers,
        body: bytes[header_end..header_end + length].to_vec(),
    })
}

fn build_request(request: &Request, host: &str, api_key: &str) -> Vec<u8> {
    let mut output = format!("{} {} HTTP/1.1\r\n", request.method, request.path).into_bytes();
    for (key, value) in &request.headers {
        if key.eq_ignore_ascii_case("host")
            || key.eq_ignore_ascii_case("content-length")
            || key.eq_ignore_ascii_case("connection")
            || key.eq_ignore_ascii_case("accept-encoding")
            || key.eq_ignore_ascii_case("authorization")
        {
            continue;
        }
        output.extend_from_slice(format!("{key}: {value}\r\n").as_bytes());
    }
    if !api_key.is_empty() {
        output.extend_from_slice(format!("Authorization: Bearer {api_key}\r\n").as_bytes());
    }
    output.extend_from_slice(
        format!(
            "Host: {host}\r\nAccept-Encoding: identity\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            request.body.len()
        )
        .as_bytes(),
    );
    output.extend_from_slice(&request.body);
    output
}

fn build_record(
    request: &Request,
    request_value: &Value,
    response: &[u8],
    capture_mode: &str,
    request_id: &str,
) -> Value {
    let response_text = String::from_utf8_lossy(response);
    let status = response_text
        .lines()
        .next()
        .unwrap_or("")
        .split_whitespace()
        .nth(1)
        .unwrap_or("");
    let (body_bytes, _) = decode_response_body(response);
    let body = String::from_utf8_lossy(&body_bytes);
    let response_values = body
        .lines()
        .filter_map(|line| {
            let data = line.strip_prefix("data: ")?.trim();
            if data == "[DONE]" {
                None
            } else {
                serde_json::from_str::<Value>(data).ok()
            }
        })
        .collect::<Vec<_>>();
    let response_json = serde_json::from_str::<Value>(&body)
        .ok()
        .or_else(|| response_values.last().cloned())
        .unwrap_or_else(|| json!({}));
    let status_code = status.parse::<u16>().unwrap_or(0);
    let mut usage = response_values
        .iter()
        .rev()
        .find_map(response_usage)
        .or_else(|| response_usage(&response_json))
        .unwrap_or_else(|| json!({}));
    let mut estimated = number(&usage, &["total_tokens"]) <= 0
        && number(&usage, &["input_tokens", "prompt_tokens"]) <= 0
        && number(&usage, &["output_tokens", "completion_tokens"]) <= 0;
    if !(200..300).contains(&status_code) {
        // A rejected request did not consume a valid model turn. Do not turn
        // its request bytes into fake token usage or no-progress evidence.
        usage = json!({});
        estimated = false;
    } else if estimated {
        let input = ((request.body.len() as i64 + 3) / 4).max(1);
        let output = ((body.len() as i64 + 3) / 4).max(1);
        usage = json!({
            "input_tokens": input,
            "output_tokens": output,
            "total_tokens": input + output,
            "estimated": true,
        });
    }
    let mut hasher = Sha256::new();
    hasher.update(&request.body);
    let request_hash = format!("{:x}", hasher.finalize());
    let call_type = call_type_for_request(request_value);
    let mut record = json!({
        "kind": "model_call",
        "requestId": request_id,
        "callType": call_type,
        "recordedAt": chrono::Utc::now().to_rfc3339(),
        "path": request.path,
        "status": status,
        "model": request_value.get("model").cloned().unwrap_or(Value::Null),
        "stream": request_value.get("stream").cloned().unwrap_or(Value::Bool(false)),
        "usage": usage,
        "usageEstimated": estimated,
    });
    if !(200..300).contains(&status_code) {
        let error = response_json
            .pointer("/error/message")
            .or_else(|| response_json.get("error"))
            .and_then(Value::as_str)
            .unwrap_or_else(|| body.trim())
            .chars()
            .take(500)
            .collect::<String>();
        record["error"] = Value::String(error);
    }
    if capture_mode != "off" {
        record["requestHash"] = Value::String(request_hash);
        record["requestChars"] = Value::from(request.body.len() as i64);
        record["requestSummary"] = request_summary(request_value);
    }
    if capture_mode == "full" {
        record["request"] = bounded_capture(request_value);
        record["response"] = bounded_capture(&response_json);
    }
    record
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use uuid::Uuid;

    #[test]
    fn accepts_https_upstreams_for_cloud_usage_accounting() {
        let upstream = parse_http_base("https://api.example.invalid/v1")
            .unwrap()
            .unwrap();
        assert_eq!(upstream.scheme, "https");
        assert_eq!(upstream.host, "api.example.invalid");
        assert_eq!(upstream.port, 443);
        assert_eq!(upstream.base_path, "/v1");
    }

    #[test]
    fn proxies_local_openai_requests_and_persists_full_local_usage() {
        let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
        let upstream_port = upstream.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut stream, _) = upstream.accept().unwrap();
            let request = read_request(&mut stream).unwrap();
            assert_eq!(request.path, "/v1/chat/completions");
            assert!(request.headers.iter().any(|(name, value)| {
                name.eq_ignore_ascii_case("authorization") && value == "Bearer upstream-test-key"
            }));
            assert!(!request
                .headers
                .iter()
                .any(|(_, value)| value.contains("strix-internal-key")));
            let body = json!({
                "model":"local-test",
                "choices":[{"message":{"role":"assistant","content":"OK"}}],
                "usage":{"prompt_tokens":12,"completion_tokens":3,"total_tokens":15}
            })
            .to_string();
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).as_bytes()).unwrap();
        });
        let root = std::env::temp_dir().join(format!("oviraptor-llm-hook-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let hook = start(
            &format!("http://127.0.0.1:{upstream_port}/v1"),
            "upstream-test-key",
            &root,
            "full",
            None,
            None,
            0,
            1,
        )
        .unwrap()
        .unwrap();
        assert!(hook.base_url().ends_with("/v1"));
        let authority = hook
            .base_url()
            .trim_start_matches("http://")
            .split('/')
            .next()
            .unwrap();
        let body = json!({
            "model":"local-test",
            "messages":[{"role":"user","content":"Authorization: Bearer top-secret"}],
            "stream":false
        })
        .to_string();
        let mut client = TcpStream::connect(authority).unwrap();
        client.write_all(format!("POST /v1/chat/completions HTTP/1.1\r\nHost: {authority}\r\nAuthorization: Bearer strix-internal-key\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).as_bytes()).unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        assert!(response.contains("200 OK"));
        server.join().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !root.join("llm-hook.jsonl").is_file() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        let records = records_from_file(&root.join("llm-hook.jsonl"));
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["kind"], "model_call_started");
        let completed = records
            .iter()
            .find(|record| record["kind"] == "model_call")
            .unwrap();
        assert_eq!(completed["usage"]["total_tokens"], 15);
        assert_eq!(
            completed["request"]["messages"][0]["content"],
            "Authorization: Bearer top-secret"
        );
        let usage = usage_from_file(&root.join("llm-hook.jsonl"));
        assert_eq!(usage.requests, 1);
        assert_eq!(usage.input_tokens, 12);
        assert_eq!(usage.output_tokens, 3);
        assert_eq!(usage.in_flight_requests, 0);
        drop(hook);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn dropping_hook_disconnects_an_in_flight_local_generation() {
        let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
        let upstream_port = upstream.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut stream, _) = upstream.accept().unwrap();
            let _ = read_request(&mut stream).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut byte = [0u8; 1];
            stream.read(&mut byte).unwrap_or(0)
        });
        let root =
            std::env::temp_dir().join(format!("oviraptor-llm-hook-cancel-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let hook = start(
            &format!("http://127.0.0.1:{upstream_port}/v1"),
            "local",
            &root,
            "off",
            None,
            Some(4_096),
            49_152,
            1,
        )
        .unwrap()
        .unwrap();
        let authority = hook
            .base_url()
            .trim_start_matches("http://")
            .split('/')
            .next()
            .unwrap()
            .to_string();
        let client = thread::spawn(move || {
            let body = r#"{"model":"local-27b","messages":[{"role":"user","content":"slow"}]}"#;
            let mut stream = TcpStream::connect(&authority).unwrap();
            let _ = stream.write_all(format!("POST /v1/chat/completions HTTP/1.1\r\nHost: {authority}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes());
            let mut response = Vec::new();
            let _ = stream.read_to_end(&mut response);
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        while hook
            .active_upstreams
            .lock()
            .map(|active| active.is_empty())
            .unwrap_or(true)
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(hook.active_upstreams.lock().unwrap().len(), 1);
        drop(hook);
        assert_eq!(server.join().unwrap(), 0);
        client.join().unwrap();
        assert!(
            records_from_file(&root.join("llm-hook.jsonl"))
                .iter()
                .any(|record| record.get("callType").and_then(Value::as_str)
                    == Some("scan_cancelled"))
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let usage = usage_from_file(&root.join("llm-hook.jsonl"));
            if usage.in_flight_requests == 0 || Instant::now() >= deadline {
                assert_eq!(usage.in_flight_requests, 0);
                assert_eq!(usage.failed_requests, 0);
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn estimates_usage_when_the_local_provider_omits_it() {
        let request = Request {
            method: "POST".into(),
            path: "/v1/chat/completions".into(),
            headers: Vec::new(),
            body: br#"{"model":"local","messages":[{"role":"user","content":"hello"}]}"#.to_vec(),
        };
        let response_body = br#"{"choices":[{"message":{"content":"world"}}]}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
            response_body.len()
        )
        .into_bytes()
        .into_iter()
        .chain(response_body.iter().copied())
        .collect::<Vec<_>>();
        let record = build_record(
            &request,
            &serde_json::from_slice(&request.body).unwrap(),
            &response,
            "metadata",
            "request-1",
        );
        assert_eq!(record["usageEstimated"], true);
        assert!(record["usage"]["total_tokens"].as_i64().unwrap() > 0);
        assert!(record.get("request").is_none());
    }

    #[test]
    fn classifies_context_compaction_without_misclassifying_checkpointed_scan_turns() {
        let compaction = json!({
            "messages": [{"role":"system","content":"You are compacting the earlier part of an autonomous security-testing agent's conversation.\n\nConversation to summarise:"}]
        });
        let resumed_scan = json!({
            "messages": [{"role":"user","content":"<conversation-checkpoint>earlier summary</conversation-checkpoint>\nContinue validating /api/register"}]
        });
        assert!(is_context_compaction_request(&compaction));
        assert!(!is_context_compaction_request(&resumed_scan));
    }

    #[test]
    fn classifies_provider_health_checks_without_misclassifying_scan_prompts() {
        let health_check = json!({
            "messages": [
                {"role":"system","content":"You are a helpful assistant."},
                {"role":"user","content":"Reply with just 'OK'."}
            ]
        });
        let scan = json!({
            "messages": [{"role":"user","content":"Check whether /health replies with just OK."}]
        });
        assert!(is_health_check_request(&health_check));
        assert!(!is_health_check_request(&scan));
    }

    #[test]
    fn lifecycle_recovery_discards_tool_history_and_keeps_finish_scan() {
        let tools = (0..34)
            .map(|index| {
                let name = if index == 33 {
                    "finish_scan".to_string()
                } else {
                    format!("tool_{index}")
                };
                json!({
                    "type":"function",
                    "function":{
                        "name":name,
                        "description":"x".repeat(2_000),
                        "parameters":{"type":"object"}
                    }
                })
            })
            .collect::<Vec<_>>();
        let request = json!({
            "model":"local-9b",
            "messages":[
                {"role":"system","content":"system".repeat(30_000)},
                {"role":"user","content":"authorized task"},
                {"role":"assistant","tool_calls":[{"id":"call-1"}],"content":""},
                {"role":"tool","tool_call_id":"call-1","content":"evidence".repeat(2_000)},
                {"role":"assistant","content":"long conclusion".repeat(2_000)},
                {"role":"user","content":"Your previous response ended the autonomous Strix run without a lifecycle tool call. That is invalid in non-interactive mode; plain text final answers are ignored. Continue immediately and call exactly one tool. If your work is complete, call finish_scan. This is recovery attempt 1/3."}
            ],
            "tools":tools,
            "max_tokens":4096
        });
        let body = serde_json::to_vec(&request).unwrap();
        let (guarded, summary) = guard_local_model_context(&body, 49_152);
        let guarded: Value = serde_json::from_slice(&guarded).unwrap();
        let messages = guarded["messages"].as_array().unwrap();
        assert!(messages.len() <= 4);
        assert!(!messages.iter().any(|message| message["role"] == "tool"));
        assert_eq!(guarded["tools"].as_array().unwrap().len(), 1);
        assert_eq!(tool_name(&guarded["tools"][0]), "finish_scan");
        assert_eq!(guarded["max_tokens"], 4096);
        assert_eq!(summary["applied"], true);
        assert_eq!(summary["reason"], "lifecycle_recovery");
        assert!(summary["afterEstimatedTokens"].as_u64().unwrap() < 49_152);
    }

    #[test]
    fn context_guard_leaves_normal_in_window_requests_unchanged() {
        let request = json!({
            "model":"local-9b",
            "messages":[{"role":"user","content":"inspect the supplied evidence"}],
            "tools":[]
        });
        let body = serde_json::to_vec(&request).unwrap();
        let (guarded, summary) = guard_local_model_context(&body, 49_152);
        assert_eq!(guarded, body);
        assert!(summary.is_null());
    }

    #[test]
    fn lifecycle_documentation_in_system_prompt_does_not_strip_scan_tools() {
        let tools = (0..34)
            .map(|index| {
                let name = if index == 33 {
                    "finish_scan".to_string()
                } else {
                    format!("tool_{index}")
                };
                json!({
                    "type":"function",
                    "function":{
                        "name":name,
                        "description":"ordinary scan tool",
                        "parameters":{"type":"object"}
                    }
                })
            })
            .collect::<Vec<_>>();
        let request = json!({
            "model":"local-9b",
            "messages":[
                {"role":"system","content":"Autonomous runs must finish with a lifecycle tool call such as finish_scan."},
                {"role":"user","content":"Validate the supplied target evidence."}
            ],
            "tools":tools
        });
        let body = serde_json::to_vec(&request).unwrap();
        let (guarded, summary) = guard_local_model_context(&body, 49_152);
        let guarded: Value = serde_json::from_slice(&guarded).unwrap();
        assert_eq!(guarded["tools"].as_array().unwrap().len(), 34);
        assert!(summary.is_null());
    }

    #[test]
    fn oversized_first_scan_turn_compacts_headroom_without_removing_tools() {
        let tools = (0..34)
            .map(|index| {
                let name = if index == 33 {
                    "finish_scan".to_string()
                } else {
                    format!("tool_{index}")
                };
                json!({
                    "type":"function",
                    "function":{
                        "name":name,
                        "description":"tool documentation ".repeat(300),
                        "parameters":{"type":"object","properties":{"url":{"type":"string"}}}
                    }
                })
            })
            .collect::<Vec<_>>();
        let request = json!({
            "model":"local-9b",
            "messages":[
                {"role":"system","content":format!("{} finish_scan is the lifecycle completion tool", "system ".repeat(10_000))},
                {"role":"user","content":"Validate the inline request/response contract."}
            ],
            "tools":tools
        });
        let body = serde_json::to_vec(&request).unwrap();
        let (guarded, summary) = guard_local_model_context(&body, 49_152);
        let guarded: Value = serde_json::from_slice(&guarded).unwrap();
        assert_eq!(guarded["tools"].as_array().unwrap().len(), 34);
        assert_eq!(summary["applied"], true);
        assert_eq!(summary["reason"], "context_headroom");
        assert_eq!(summary["filteredTools"], 0);
    }

    #[test]
    fn user_text_that_merely_mentions_finish_scan_is_not_protocol_recovery() {
        let request = json!({
            "model":"local-9b",
            "messages":[
                {"role":"system","content":"system"},
                {"role":"user","content":"Explain why a lifecycle tool call named finish_scan exists."}
            ],
            "tools":[{"type":"function","function":{"name":"finish_scan","parameters":{"type":"object"}}}]
        });
        let body = serde_json::to_vec(&request).unwrap();
        let (guarded, summary) = guard_local_model_context(&body, 49_152);
        assert_eq!(guarded, body);
        assert!(summary.is_null());
    }

    #[test]
    fn maintenance_calls_keep_token_accounting_but_do_not_become_scan_failures() {
        let root =
            std::env::temp_dir().join(format!("oviraptor-llm-hook-maintenance-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let records = [
            json!({"status":"200","callType":"scan","usage":{"input_tokens":100,"output_tokens":10,"total_tokens":110}}),
            json!({"status":"200","callType":"context_compaction","usage":{"input_tokens":80,"output_tokens":20,"total_tokens":100}}),
            json!({"status":"200","callType":"health_check","usage":{"input_tokens":30,"output_tokens":1,"total_tokens":31}}),
            json!({"status":"500","callType":"context_compaction","error":"summary failed","usage":{}}),
            json!({"status":"500","callType":"health_check","error":"provider warming up","usage":{}}),
        ];
        fs::write(
            root.join("llm-hook.jsonl"),
            records
                .into_iter()
                .map(|record| record.to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        let usage = usage_from_file(&root.join("llm-hook.jsonl"));
        assert_eq!(usage.requests, 3);
        assert_eq!(usage.maintenance_requests, 2);
        assert_eq!(usage.maintenance_failed_requests, 2);
        assert_eq!(usage.failed_requests, 0);
        assert_eq!(usage.total_tokens, 241);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn records_an_unfinished_scan_request_as_active_inference() {
        let root =
            std::env::temp_dir().join(format!("oviraptor-llm-hook-in-flight-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("llm-hook.jsonl"),
            json!({
                "kind":"model_call_started",
                "requestId":"slow-local-prefill",
                "callType":"scan",
                "status":""
            })
            .to_string(),
        )
        .unwrap();
        let usage = usage_from_file(&root.join("llm-hook.jsonl"));
        assert_eq!(usage.requests, 0);
        assert_eq!(usage.in_flight_requests, 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejected_context_requests_are_not_counted_as_model_turns() {
        let root =
            std::env::temp_dir().join(format!("oviraptor-llm-hook-error-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let record = json!({
            "kind": "model_call",
            "status": "400",
            "error": "request exceeds the available context size",
            "usage": {"input_tokens": 34000, "total_tokens": 34000}
        });
        fs::write(root.join("llm-hook.jsonl"), format!("{}\n", record)).unwrap();
        let usage = usage_from_file(&root.join("llm-hook.jsonl"));
        assert_eq!(usage.requests, 0);
        assert_eq!(usage.failed_requests, 1);
        assert_eq!(usage.context_errors, 1);
        assert_eq!(usage.total_tokens, 0);
        let _ = fs::remove_dir_all(root);
    }
}
