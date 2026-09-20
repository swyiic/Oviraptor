    /// Read one HTTP request from a mock connection. A client announcing
    /// `Expect: 100-continue` gets the interim response so it sends the body, and
    /// a request is served as soon as its header block is complete — dropping the
    /// connection instead makes the client retry, which silently shifts the
    /// scripted model rounds.
    /// One-shot redirect server. `/api/hijack` leaves the frozen scope and
    /// `/api/sso` jumps to an identity provider; both answer with a `location`
    /// header, which the generic mock cannot express.
    fn spawn_redirect_server(hijack_to: &'static str, sso_to: &'static str) -> u16 {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let Some(request) = read_http_request(&mut stream) else {
                    continue;
                };
                let line = request.lines().next().unwrap_or_default().to_string();
                let target = if line.starts_with("GET /api/hijack") {
                    hijack_to
                } else if line.starts_with("GET /api/sso") {
                    sso_to
                } else {
                    ""
                };
                let response = if target.is_empty() {
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 11\r\nconnection: close\r\n\r\n{\"ok\":true}"
                        .to_string()
                } else {
                    format!(
                        "HTTP/1.1 302 Found\r\nlocation: {target}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                    )
                };
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        port
    }

    fn read_http_request(stream: &mut std::net::TcpStream) -> Option<String> {
        use std::io::{Read, Write};
        let _ = stream.set_read_timeout(Some(Duration::from_secs(20)));
        let mut buffer: Vec<u8> = Vec::new();
        let mut chunk = [0u8; 8192];
        let mut header_end: Option<usize> = None;
        let mut want = 0usize;
        let mut continued = false;
        loop {
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(size) => {
                    buffer.extend_from_slice(&chunk[..size]);
                    if header_end.is_none() {
                        header_end = String::from_utf8_lossy(&buffer).find("\r\n\r\n");
                        if let Some(index) = header_end {
                            let head =
                                String::from_utf8_lossy(&buffer[..index]).to_ascii_lowercase();
                            want = head
                                .lines()
                                .find_map(|line| {
                                    line.strip_prefix("content-length:")
                                        .map(|rest| rest.trim().parse::<usize>().unwrap_or(0))
                                })
                                .unwrap_or(0);
                        }
                    }
                    let Some(index) = header_end else { continue };
                    if buffer.len() >= index + 4 + want {
                        break;
                    }
                    if !continued
                        && String::from_utf8_lossy(&buffer[..index])
                            .to_ascii_lowercase()
                            .contains("expect: 100-continue")
                    {
                        let _ = stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n");
                        continued = true;
                    }
                }
                Err(_) => break,
            }
        }
        header_end?;
        Some(String::from_utf8_lossy(&buffer).to_string())
    }

    fn spawn_endpoint(
        handler: std::sync::Arc<
            dyn Fn(String) -> (u16, &'static str, String) + Send + Sync + 'static,
        >,
    ) -> (u16, Seen, std::sync::Arc<std::sync::atomic::AtomicBool>) {
        use std::net::TcpListener;
        use std::sync::atomic::{AtomicBool, Ordering};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let seen: Seen = std::sync::Arc::new(Mutex::new(Vec::new()));
        let loop_stop = stop.clone();
        let loop_seen = seen.clone();
        thread::spawn(move || {
            while !loop_stop.load(Ordering::SeqCst) {
                let Ok((stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(1));
                    continue;
                };
                // The accepted socket inherits the listener's O_NONBLOCK on
                // macOS; a non-blocking read that arrives first would return
                // WouldBlock and drop the request.
                let _ = stream.set_nonblocking(false);
                let seen = loop_seen.clone();
                let stop = loop_stop.clone();
                let handler = handler.clone();
                thread::spawn(move || {
                    if stop.load(Ordering::SeqCst) {
                        return;
                    }
                    let mut stream = stream;
                let Some(request) = read_http_request(&mut stream) else {
                    return;
                };
                let (status, content_type, body) = (handler)(request.clone());
                seen.lock().unwrap().push(request);
                let response = format!(
                    "HTTP/1.1 {status} OK\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
                });
            }
        });
        (port, seen, stop)
    }

    pub(super) fn model_round(calls: &[(&str, JsonValue)], prompt_tokens: i64) -> String {
        let list: Vec<JsonValue> = calls
            .iter()
            .enumerate()
            .map(|(index, (name, arguments))| {
                serde_json::json!({
                    "id": format!("call-{index}"),
                    "type": "function",
                    "function": {"name": name, "arguments": arguments.to_string()}
                })
            })
            .collect();
        serde_json::json!({
            "choices": [{
                "message": {"content": "", "tool_calls": list},
                "finish_reason": if list.is_empty() { "stop" } else { "tool_calls" }
            }],
            "usage": {
                "prompt_tokens": prompt_tokens,
                "completion_tokens": 40,
                "total_tokens": prompt_tokens + 40,
                "prompt_tokens_details": {"cached_tokens": 0}
            }
        })
        .to_string()
    }

    fn spawn_model(
        script: Vec<String>,
    ) -> (u16, std::sync::Arc<std::sync::atomic::AtomicBool>, Seen) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let counter = std::sync::Arc::new(AtomicUsize::new(0));
        let rows = script.clone();
        let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(move |request| {
            if !request.contains("POST /v1/chat/completions") {
                return (404, "text/plain", "unexpected path".to_string());
            }
            let index = counter.fetch_add(1, Ordering::SeqCst);
            let body = rows
                .get(index)
                .cloned()
                .unwrap_or_else(|| model_round(&[], 10));
            (200, "application/json", body)
        }));
        (port, stop, seen)
    }

    pub(super) struct AgentHarness {
        pub(super) root: PathBuf,
        pub(super) db_path: PathBuf,
        pub(super) context: AgentRunContext,
        pub(super) site_seen: Seen,
        pub(super) model_seen: Seen,
    }

    pub(super) fn agent_harness(
        tag: &str,
        site: impl Fn(String) -> (u16, &'static str, String) + Send + Sync + 'static,
        identities: Vec<AgentIdentity>,
    ) -> AgentHarness {
        let (site_port, site_seen, _site_stop) =
            spawn_endpoint(std::sync::Arc::new(site));
        let (model_port, _model_stop, model_seen) = spawn_model(vec![]);
        let root = std::env::temp_dir().join(format!("oviraptor-agent-{tag}-{}", Uuid::new_v4()));
        let db_path = db::initialize(&root).unwrap();
        seed_scan(&db_path, "agent-scan", "scanning");
        let target_url = format!("http://127.0.0.1:{site_port}");
        seed_target(&db_path, &target_url);
        let mut context = test_context(&db_path, &target_url, identities);
        context.environment.api_base = format!("http://127.0.0.1:{model_port}/v1");
        context.environment.api_key = "mock-key".into();
        context.target_dir = root.join("target");
        context.log_path = root.join("runner.log");
        fs::create_dir_all(&context.target_dir).unwrap();
        context.evidence["apiCandidates"] = serde_json::json!([
            {"path": "/api/orders", "url": format!("{target_url}/api/orders"), "method": "GET", "parameters": ["page"], "source": "browser-runtime"}
        ]);
        AgentHarness {
            root,
            db_path,
            context,
            site_seen,
            model_seen,
        }
    }

    /// Point the run at the real scripted model rounds.
    pub(super) fn retarget_model(harness: &mut AgentHarness, script: Vec<String>) {
        let (model_port, _stop, seen) = spawn_model(script);
        harness.context.environment.api_base = format!("http://127.0.0.1:{model_port}/v1");
        harness.model_seen = seen;
    }

    /// What `agent_runs` holds for this scan's single coordinator run.
    #[derive(Debug)]
    struct RunRow {
        status: String,
        soft_token_budget: i64,
        hard_token_budget: i64,
        soft_request_budget: i64,
        hard_request_budget: i64,
        used_tokens: i64,
        used_requests: i64,
        terminal_code: String,
        pending_contracts: i64,
        rows: i64,
    }

    fn read_run_row(db_path: &Path) -> RunRow {
        let connection = db::open(db_path).unwrap();
        let snapshot: String = connection
            .query_row(
                "SELECT s.snapshot_json FROM agent_snapshots s
                 JOIN agent_runs r ON r.id=s.run_id WHERE r.scan_id='agent-scan'",
                [],
                |row| row.get(0),
            )
            .unwrap_or_default();
        let pending = json(snapshot)
            .get("pendingContracts")
            .and_then(JsonValue::as_array)
            .map(|rows| rows.len() as i64)
            .unwrap_or(0);
        let (rows, status, soft, hard, soft_r, hard_r, used, reqs, code) = connection
            .query_row(
                "SELECT COUNT(*),MAX(status),MAX(soft_token_budget),MAX(hard_token_budget),MAX(soft_request_budget),MAX(hard_request_budget),MAX(used_tokens),MAX(used_requests),MAX(terminal_code) FROM agent_runs WHERE scan_id='agent-scan'",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, i64>(3)?, row.get::<_, i64>(4)?, row.get::<_, i64>(5)?, row.get::<_, i64>(6)?, row.get::<_, i64>(7)?, row.get::<_, String>(8)?)),
            )
            .unwrap();
        RunRow {
            status,
            soft_token_budget: soft,
            hard_token_budget: hard,
            soft_request_budget: soft_r,
            hard_request_budget: hard_r,
            used_tokens: used,
            used_requests: reqs,
            terminal_code: code,
            pending_contracts: pending,
            rows,
        }
    }

    pub(super) fn findings_for(db_path: &Path, stage: &str) -> Vec<(String, String)> {
        let connection = db::open(db_path).unwrap();
        let mut statement = connection
            .prepare("SELECT kind,record_json FROM sentinel_findings WHERE stage=?1 ORDER BY id")
            .unwrap();
        statement
            .query_map([stage], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .flatten()
            .collect()
    }

    /// A closing coverage ledger: `covered` must be backed by requests this run
    /// actually executed; every other family is honestly declared not applicable,
    /// which is what lets a zero-finding run reach Completed without lying.
    pub(super) fn closing_ledger(covered: &[&str]) -> Vec<JsonValue> {
        AGENT_COVERAGE_FAMILIES
            .iter()
            .map(|family| {
                if covered.contains(family) {
                    serde_json::json!({"family":family,"status":"covered","reason":"本轮已执行该族请求"})
                } else {
                    serde_json::json!({"family":family,"status":"not_applicable","reason":"该目标没有可执行的同类入口"})
                }
            })
            .collect()
    }

    /// The mock site: an anonymous SPA, a cookie-aware API and a gate that
    /// answers with a Cloudflare-style challenge.
    fn mock_site(request: String) -> (u16, &'static str, String) {
        let line = request.lines().next().unwrap_or_default().to_string();
        let cookie = request
            .lines()
            .find(|row| row.to_ascii_lowercase().starts_with("cookie:"))
            .unwrap_or_default()
            .to_string();
        if line.starts_with("GET / ") {
            return (200, "text/html", "<!DOCTYPE html><html><body><div id=app>orders SPA</div><a href=/orders>订单</a><script src=/static/app.js></script></body></html>".to_string());
        }
        if line.contains("/static/app.js") {
            return (
                200,
                "application/javascript",
                "fetch('/api/orders?page=1')".to_string(),
            );
        }
        if line.contains("/api/orders") {
            let who = if cookie.contains("cookie-alpha") {
                "alpha"
            } else if cookie.contains("cookie-beta") {
                "beta"
            } else {
                "anonymous"
            };
            return (
                200,
                "application/json",
                format!("{{\"who\":\"{who}\",\"items\":[{{\"id\":1}}]}}"),
            );
        }
        if line.contains("/api/gated") {
            return (
                403,
                "text/html",
                "<html>Attention Required! | Cloudflare Ray ID: 7f3b</html>".to_string(),
            );
        }
        (404, "text/plain", "not found".to_string())
    }
