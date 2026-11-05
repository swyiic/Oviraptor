// MCP (stdio) client for the native web agent. External tools are never on by
// default: each server is operator-configured, disabled until enabled, and each
// server/tool pair must match the workspace allowlist before `call_mcp_tool` runs.

const MCP_SERVER_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS agent_mcp_servers (
    server_id TEXT PRIMARY KEY,
    label TEXT NOT NULL DEFAULT '',
    transport TEXT NOT NULL DEFAULT 'stdio' CHECK(transport IN ('stdio','http','sse')),
    command TEXT NOT NULL,
    args_json TEXT NOT NULL DEFAULT '[]',
    env_json TEXT NOT NULL DEFAULT '[]',
    enabled INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
"#;

#[derive(Clone, Debug)]
struct McpServerRecord {
    transport: String,
    command: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    enabled: bool,
}

const MCP_MAX_FRAME: usize = 65_536;
const MCP_CALL_TIMEOUT: Duration = Duration::from_secs(120);

static MCP_SESSIONS: std::sync::Mutex<Option<HashMap<String, McpStdioSession>>> =
    std::sync::Mutex::new(None);
static MCP_HTTP_SESSIONS: std::sync::Mutex<Option<HashMap<String, McpHttpSession>>> =
    std::sync::Mutex::new(None);

struct McpStdioSession {
    child: std::process::Child,
    next_id: i64,
    stdout: BufReader<std::process::ChildStdout>,
}

fn ensure_mcp_storage(connection: &rusqlite::Connection) -> Result<(), String> {
    connection
        .execute_batch(MCP_SERVER_SCHEMA)
        .map_err(|_| -> String { "mcp_storage_unavailable".into() })?;
    migrate_mcp_transport_check(connection)
}

fn migrate_mcp_transport_check(connection: &rusqlite::Connection) -> Result<(), String> {
    let ddl: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='agent_mcp_servers'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_default();
    if ddl.contains("'http'") || ddl.is_empty() {
        return Ok(());
    }
    connection
        .execute_batch(
            "CREATE TABLE agent_mcp_servers_v2 (
                server_id TEXT PRIMARY KEY,
                label TEXT NOT NULL DEFAULT '',
                transport TEXT NOT NULL DEFAULT 'stdio' CHECK(transport IN ('stdio','http','sse')),
                command TEXT NOT NULL,
                args_json TEXT NOT NULL DEFAULT '[]',
                env_json TEXT NOT NULL DEFAULT '[]',
                enabled INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
            );
            INSERT INTO agent_mcp_servers_v2 SELECT * FROM agent_mcp_servers;
            DROP TABLE agent_mcp_servers;
            ALTER TABLE agent_mcp_servers_v2 RENAME TO agent_mcp_servers;",
        )
        .map_err(|_| "mcp_transport_migration_failed".to_string())
}

fn default_allowlist_json() -> &'static str {
    "[]"
}

fn mcp_allowlist(connection: &rusqlite::Connection) -> Result<Vec<(String, String)>, String> {
    let raw: String = connection
        .query_row(
            "SELECT value FROM app_settings WHERE key='agent_mcp_tool_allowlist'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| default_allowlist_json().to_string());
    let rows: Vec<JsonValue> = serde_json::from_str(&raw).unwrap_or_default();
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let pattern = row.get("pattern")?.as_str()?.trim().to_string();
            let decision = row.get("decision")?.as_str()?.trim().to_ascii_lowercase();
            if decision == "allow" && !pattern.is_empty() {
                Some((pattern, decision))
            } else {
                None
            }
        })
        .collect())
}

fn mcp_glob_match(pattern: &str, server_id: &str, tool: &str) -> bool {
    let path = format!("{server_id}/{tool}");
    let pat = pattern.trim().to_ascii_lowercase();
    let hay = path.to_ascii_lowercase();
    if !pat.contains('*') {
        return pat == hay || pat == server_id.to_ascii_lowercase();
    }
    let mut parts = pat.split('*').collect::<Vec<_>>();
    let mut cursor = hay.as_str();
    if !pat.starts_with('*') {
        let head = parts.remove(0);
        if head.is_empty() || !cursor.starts_with(head) {
            return false;
        }
        cursor = &cursor[head.len()..];
    }
    if !pat.ends_with('*') {
        let tail = parts.pop().unwrap_or("");
        if !tail.is_empty() && !cursor.ends_with(tail) {
            return false;
        }
        if !tail.is_empty() {
            cursor = &cursor[..cursor.len().saturating_sub(tail.len())];
        }
    }
    for part in parts {
        if part.is_empty() {
            continue;
        }
        let Some(index) = cursor.find(part) else {
            return false;
        };
        cursor = &cursor[index + part.len()..];
    }
    true
}

pub(crate) fn mcp_tool_allowed(
    connection: &rusqlite::Connection,
    server_id: &str,
    tool: &str,
) -> Result<bool, String> {
    let list = mcp_allowlist(connection)?;
    if list.is_empty() {
        return Ok(false);
    }
    Ok(list
        .iter()
        .any(|(pattern, _)| mcp_glob_match(pattern, server_id, tool)))
}

fn parse_env_rows(env_json: &str) -> Vec<(String, String)> {
    let env_rows: Vec<JsonValue> = serde_json::from_str(env_json).unwrap_or_default();
    env_rows
        .into_iter()
        .filter_map(|item| {
            let name = item.get("name")?.as_str()?.to_string();
            let value = item.get("value")?.as_str()?.to_string();
            Some((name, value))
        })
        .collect()
}

fn read_server(connection: &rusqlite::Connection, server_id: &str) -> Result<McpServerRecord, String> {
    ensure_mcp_storage(connection)?;
    connection
        .query_row(
            "SELECT transport, command, args_json, env_json, enabled FROM agent_mcp_servers WHERE server_id=?1",
            [server_id],
            |row| {
                Ok(McpServerRecord {
                    transport: row.get::<_, String>(0)?,
                    command: row.get::<_, String>(1)?,
                    args: serde_json::from_str(&row.get::<_, String>(2)?).unwrap_or_default(),
                    env: parse_env_rows(&row.get::<_, String>(3)?),
                    enabled: row.get::<_, i64>(4)? != 0,
                })
            },
        )
        .map_err(|_| "mcp_server_not_found".into())
}

fn normalize_transport(raw: &str) -> Result<&'static str, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "stdio" => Ok("stdio"),
        "http" | "streamable-http" => Ok("http"),
        "sse" => Ok("sse"),
        _ => Err("mcp_transport_invalid".into()),
    }
}

fn validate_remote_endpoint(url: &str) -> Result<(), String> {
    let trimmed = url.trim();
    if trimmed.len() < 8 || trimmed.len() > 2048 {
        return Err("mcp_endpoint_invalid".into());
    }
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        return Err("mcp_endpoint_invalid".into());
    }
    Ok(())
}

impl McpStdioSession {
    fn spawn(command: &str, args: &[String], env: &[(String, String)]) -> Result<Self, String> {
        if command.trim().is_empty() {
            return Err("mcp_command_empty".into());
        }
        let mut child_cmd = Command::new(command);
        child_cmd.args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        for (key, value) in env {
            child_cmd.env(key, value);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            child_cmd.creation_flags(0x08000000);
        }
        let mut child = child_cmd.spawn().map_err(|error| error.to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "mcp_stdout_unavailable".to_string())?;
        let mut session = Self {
            child,
            next_id: 1,
            stdout: BufReader::new(stdout),
        };
        session.initialize()?;
        Ok(session)
    }

    fn initialize(&mut self) -> Result<(), String> {
        let id = self.next_id;
        self.next_id += 1;
        let _ = self.request(
            id,
            "initialize",
            serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "oviraptor", "version": env!("CARGO_PKG_VERSION") },
            }),
        )?;
        let _ = self.notify(
            "notifications/initialized",
            serde_json::json!({}),
        )?;
        Ok(())
    }

    fn notify(&mut self, method: &str, params: JsonValue) -> Result<(), String> {
        let stdin = self
            .child
            .stdin
            .as_mut()
            .ok_or_else(|| "mcp_stdin_unavailable".to_string())?;
        let frame = serde_json::json!({ "jsonrpc": "2.0", "method": method, "params": params });
        let line = serde_json::to_string(&frame).map_err(|error| error.to_string())?;
        if line.len() > MCP_MAX_FRAME {
            return Err("mcp_frame_too_large".into());
        }
        stdin
            .write_all(line.as_bytes())
            .and_then(|_| stdin.write_all(b"\n"))
            .map_err(|error| error.to_string())
    }

    fn request(&mut self, id: i64, method: &str, params: JsonValue) -> Result<JsonValue, String> {
        let stdin = self
            .child
            .stdin
            .as_mut()
            .ok_or_else(|| "mcp_stdin_unavailable".to_string())?;
        let frame = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let line = serde_json::to_string(&frame).map_err(|error| error.to_string())?;
        if line.len() > MCP_MAX_FRAME {
            return Err("mcp_frame_too_large".into());
        }
        stdin
            .write_all(line.as_bytes())
            .and_then(|_| stdin.write_all(b"\n"))
            .map_err(|error| error.to_string())?;
        let started = Instant::now();
        loop {
            if started.elapsed() > MCP_CALL_TIMEOUT {
                let _ = self.child.kill();
                return Err("mcp_timeout".into());
            }
            let mut line = String::new();
            let read = self.stdout.read_line(&mut line).map_err(|error| error.to_string())?;
            if read == 0 {
                return Err("mcp_server_closed".into());
            }
            if line.len() > MCP_MAX_FRAME {
                return Err("mcp_response_too_large".into());
            }
            let value: JsonValue = serde_json::from_str(line.trim()).map_err(|_| "mcp_invalid_json".to_string())?;
            if value.get("id").and_then(JsonValue::as_i64) == Some(id) {
                if let Some(error) = value.get("error") {
                    return Err(error.to_string());
                }
                return value
                    .get("result")
                    .cloned()
                    .ok_or_else(|| "mcp_missing_result".to_string());
            }
        }
    }

    fn tools_list(&mut self) -> Result<Vec<JsonValue>, String> {
        let id = self.next_id;
        self.next_id += 1;
        let result = self.request(id, "tools/list", serde_json::json!({}))?;
        Ok(result
            .get("tools")
            .and_then(JsonValue::as_array)
            .cloned()
            .unwrap_or_default())
    }

    fn tools_call(&mut self, tool: &str, arguments: &JsonValue) -> Result<JsonValue, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.request(
            id,
            "tools/call",
            serde_json::json!({ "name": tool, "arguments": arguments }),
        )
    }

    fn named_list(&mut self, method: &str, key: &str) -> Result<Vec<JsonValue>, String> {
        let id = self.next_id;
        self.next_id += 1;
        let result = self.request(id, method, serde_json::json!({}))?;
        Ok(result.get(key).and_then(JsonValue::as_array).cloned().unwrap_or_default())
    }
}

struct McpHttpSession {
    client: reqwest::blocking::Client,
    endpoint: String,
    post_endpoint: String,
    headers: Vec<(String, String)>,
    next_id: i64,
    initialized: bool,
    accept_sse: bool,
    session_id: String,
    sse_reader: Option<BufReader<reqwest::blocking::Response>>,
}

fn resolve_mcp_url(base: &str, data: &str) -> String {
    let data = data.trim();
    if data.starts_with("http://") || data.starts_with("https://") {
        return data.to_string();
    }
    if let Some(scheme) = base.find("://") {
        let rest = &base[scheme + 3..];
        if let Some(slash) = rest.find('/') {
            let origin_end = scheme + 3 + slash;
            if data.starts_with('/') {
                return format!("{}{data}", &base[..origin_end]);
            }
        } else if data.starts_with('/') {
            return format!("{base}{data}");
        }
    }
    data.to_string()
}

impl McpHttpSession {
    fn new(endpoint: &str, headers: &[(String, String)], accept_sse: bool) -> Result<Self, String> {
        validate_remote_endpoint(endpoint)?;
        let client = reqwest::blocking::Client::builder()
            .timeout(MCP_CALL_TIMEOUT)
            .redirect(reqwest::redirect::Policy::limited(4))
            .build()
            .map_err(|error| error.to_string())?;
        let endpoint = endpoint.trim().to_string();
        Ok(Self {
            client,
            post_endpoint: endpoint.clone(),
            endpoint,
            headers: headers.to_vec(),
            next_id: 1,
            initialized: false,
            accept_sse,
            session_id: String::new(),
            sse_reader: None,
        })
    }

    fn apply_headers(&self, mut request: reqwest::blocking::RequestBuilder) -> reqwest::blocking::RequestBuilder {
        for (key, value) in &self.headers {
            request = request.header(key.as_str(), value.as_str());
        }
        if !self.session_id.is_empty() {
            request = request.header("Mcp-Session-Id", self.session_id.as_str());
        }
        request
    }

    fn capture_session(&mut self, response: &reqwest::blocking::Response) {
        if let Some(value) = response.headers().get("mcp-session-id").and_then(|value| value.to_str().ok()) {
            if !value.is_empty() {
                self.session_id = value.to_string();
            }
        }
    }

    fn open_legacy_sse(&mut self) -> Result<(), String> {
        if self.sse_reader.is_some() {
            return Ok(());
        }
        let response = self
            .apply_headers(self.client.get(&self.endpoint))
            .header("Accept", "text/event-stream")
            .send()
            .map_err(|error| error.to_string())?;
        self.capture_session(&response);
        let mut reader = BufReader::new(response);
        let mut event_name = String::new();
        let mut post = String::new();
        for _ in 0..48 {
            let mut line = String::new();
            let read = reader.read_line(&mut line).map_err(|error| error.to_string())?;
            if read == 0 {
                break;
            }
            let trimmed = line.trim();
            if let Some(name) = trimmed.strip_prefix("event:") {
                event_name = name.trim().to_string();
            } else if let Some(data) = trimmed.strip_prefix("data:") {
                let data = data.trim();
                if event_name == "endpoint" || data.starts_with('/') || data.starts_with("http://") || data.starts_with("https://") {
                    post = resolve_mcp_url(&self.endpoint, data);
                    break;
                }
            } else if trimmed.is_empty() {
                event_name.clear();
            }
        }
        if post.is_empty() {
            return Err("mcp_sse_endpoint_missing".into());
        }
        self.post_endpoint = post;
        self.sse_reader = Some(reader);
        Ok(())
    }

    fn read_sse_result(reader: &mut BufReader<reqwest::blocking::Response>, expected_id: Option<i64>) -> Result<JsonValue, String> {
        let mut data = String::new();
        for _ in 0..400 {
            let mut line = String::new();
            let read = reader.read_line(&mut line).map_err(|error| error.to_string())?;
            if read == 0 {
                break;
            }
            let trimmed = line.trim();
            if let Some(payload) = trimmed.strip_prefix("data:") {
                data.push_str(payload.trim());
            } else if trimmed.is_empty() && !data.is_empty() {
                if let Ok(value) = serde_json::from_str::<JsonValue>(&data) {
                    let matches = match expected_id {
                        Some(id) => value.get("id").and_then(JsonValue::as_i64) == Some(id),
                        None => value.get("id").is_some(),
                    };
                    if matches {
                        if let Some(error) = value.get("error") {
                            return Err(error.to_string());
                        }
                        return value.get("result").cloned().ok_or_else(|| "mcp_missing_result".to_string());
                    }
                }
                data.clear();
            }
        }
        Err("mcp_sse_no_matching_event".into())
    }

    fn ensure_initialized(&mut self) -> Result<(), String> {
        if self.initialized {
            return Ok(());
        }
        let id = self.next_id;
        self.next_id += 1;
        let _ = self.exchange(
            Some(id),
            "initialize",
            serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "oviraptor", "version": env!("CARGO_PKG_VERSION") },
            }),
        )?;
        let _ = self.notify("notifications/initialized", serde_json::json!({}))?;
        self.initialized = true;
        Ok(())
    }

    fn notify(&mut self, method: &str, params: JsonValue) -> Result<(), String> {
        if self.accept_sse {
            self.open_legacy_sse()?;
        }
        let frame = serde_json::json!({ "jsonrpc": "2.0", "method": method, "params": params });
        self.apply_headers(self.client.post(&self.post_endpoint).json(&frame))
            .header("Accept", "application/json, text/event-stream")
            .send()
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn exchange(&mut self, id: Option<i64>, method: &str, params: JsonValue) -> Result<JsonValue, String> {
        if self.accept_sse {
            self.open_legacy_sse()?;
        }
        let frame = if let Some(id) = id {
            serde_json::json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
        } else {
            serde_json::json!({ "jsonrpc": "2.0", "method": method, "params": params })
        };
        let accept = "application/json, text/event-stream";
        let response = self
            .apply_headers(self.client.post(&self.post_endpoint).json(&frame))
            .header("Accept", accept)
            .send()
            .map_err(|error| error.to_string())?;
        self.capture_session(&response);
        if self.sse_reader.is_some() {
            let reader = self.sse_reader.as_mut().ok_or_else(|| "mcp_sse_reader_missing".to_string())?;
            return Self::read_sse_result(reader, id);
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        let body = response.text().map_err(|error| error.to_string())?;
        if body.len() > MCP_MAX_FRAME.saturating_mul(8) {
            return Err("mcp_response_too_large".into());
        }
        if content_type.contains("text/event-stream") || body.contains("data:") {
            return parse_sse_json_rpc(&body, id);
        }
        if body.trim().is_empty() {
            return Err("mcp_empty_response".into());
        }
        let value: JsonValue = serde_json::from_str(body.trim()).map_err(|_| "mcp_invalid_json".to_string())?;
        if let Some(expected) = id {
            if value.get("id").and_then(JsonValue::as_i64) != Some(expected) {
                return Err("mcp_unexpected_response_id".into());
            }
        }
        if let Some(error) = value.get("error") {
            return Err(error.to_string());
        }
        value
            .get("result")
            .cloned()
            .ok_or_else(|| "mcp_missing_result".to_string())
    }

    fn tools_list(&mut self) -> Result<Vec<JsonValue>, String> {
        self.ensure_initialized()?;
        let id = self.next_id;
        self.next_id += 1;
        let result = self.exchange(Some(id), "tools/list", serde_json::json!({}))?;
        Ok(result
            .get("tools")
            .and_then(JsonValue::as_array)
            .cloned()
            .unwrap_or_default())
    }

    fn tools_call(&mut self, tool: &str, arguments: &JsonValue) -> Result<JsonValue, String> {
        self.ensure_initialized()?;
        let id = self.next_id;
        self.next_id += 1;
        self.exchange(
            Some(id),
            "tools/call",
            serde_json::json!({ "name": tool, "arguments": arguments }),
        )
    }

    fn named_list(&mut self, method: &str, key: &str) -> Result<Vec<JsonValue>, String> {
        self.ensure_initialized()?;
        let id = self.next_id;
        self.next_id += 1;
        let result = self.exchange(Some(id), method, serde_json::json!({}))?;
        Ok(result.get(key).and_then(JsonValue::as_array).cloned().unwrap_or_default())
    }
}

fn parse_sse_json_rpc(body: &str, expected_id: Option<i64>) -> Result<JsonValue, String> {
    for line in body.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("data:") {
            continue;
        }
        let payload = trimmed.trim_start_matches("data:").trim();
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }
        let value: JsonValue = serde_json::from_str(payload).map_err(|_| "mcp_invalid_json".to_string())?;
        if let Some(expected) = expected_id {
            if value.get("id").and_then(JsonValue::as_i64) != Some(expected) {
                continue;
            }
        } else if value.get("id").is_none() {
            continue;
        }
        if let Some(error) = value.get("error") {
            return Err(error.to_string());
        }
        return value
            .get("result")
            .cloned()
            .ok_or_else(|| "mcp_missing_result".to_string());
    }
    Err("mcp_sse_no_matching_event".into())
}

fn with_http_session<R>(
    server_id: &str,
    endpoint: &str,
    headers: &[(String, String)],
    accept_sse: bool,
    f: impl FnOnce(&mut McpHttpSession) -> Result<R, String>,
) -> Result<R, String> {
    let mut guard = MCP_HTTP_SESSIONS
        .lock()
        .map_err(|_| "mcp_session_lock".to_string())?;
    if guard.is_none() {
        *guard = Some(HashMap::new());
    }
    let map = guard.as_mut().unwrap();
    if !map.contains_key(server_id) {
        map.insert(
            server_id.to_string(),
            McpHttpSession::new(endpoint, headers, accept_sse)?,
        );
    }
    let session = map
        .get_mut(server_id)
        .ok_or_else(|| "mcp_session_missing".to_string())?;
    f(session)
}

fn with_session<R>(
    server_id: &str,
    command: &str,
    args: &[String],
    env: &[(String, String)],
    f: impl FnOnce(&mut McpStdioSession) -> Result<R, String>,
) -> Result<R, String> {
    let mut guard = MCP_SESSIONS
        .lock()
        .map_err(|_| "mcp_session_lock".to_string())?;
    if guard.is_none() {
        *guard = Some(HashMap::new());
    }
    let map = guard.as_mut().unwrap();
    let needs_respawn = match map.get_mut(server_id) {
        Some(session) => session.child.try_wait().ok().flatten().is_some(),
        None => true,
    };
    if needs_respawn {
        map.insert(
            server_id.to_string(),
            McpStdioSession::spawn(command, args, env)?,
        );
    }
    let session = map
        .get_mut(server_id)
        .ok_or_else(|| "mcp_session_missing".to_string())?;
    f(session)
}

fn mcp_named_list(record: &McpServerRecord, server_id: &str, method: &str, key: &str) -> Result<Vec<JsonValue>, String> {
    match normalize_transport(&record.transport)? {
        "stdio" => with_session(server_id, &record.command, &record.args, &record.env, |session| session.named_list(method, key)),
        "http" => with_http_session(server_id, &record.command, &record.env, false, |session| session.named_list(method, key)),
        "sse" => with_http_session(server_id, &record.command, &record.env, true, |session| session.named_list(method, key)),
        _ => Err("mcp_transport_invalid".into()),
    }
}

fn mcp_tools_list(record: &McpServerRecord, server_id: &str) -> Result<Vec<JsonValue>, String> {
    match normalize_transport(&record.transport)? {
        "stdio" => with_session(
            server_id,
            &record.command,
            &record.args,
            &record.env,
            |session| session.tools_list(),
        ),
        "http" => with_http_session(
            server_id,
            &record.command,
            &record.env,
            false,
            |session| session.tools_list(),
        ),
        "sse" => with_http_session(
            server_id,
            &record.command,
            &record.env,
            true,
            |session| session.tools_list(),
        ),
        _ => Err("mcp_transport_invalid".into()),
    }
}

fn mcp_tools_call(
    record: &McpServerRecord,
    server_id: &str,
    tool: &str,
    arguments: &JsonValue,
) -> Result<JsonValue, String> {
    match normalize_transport(&record.transport)? {
        "stdio" => with_session(
            server_id,
            &record.command,
            &record.args,
            &record.env,
            |session| session.tools_call(tool, arguments),
        ),
        "http" => with_http_session(
            server_id,
            &record.command,
            &record.env,
            false,
            |session| session.tools_call(tool, arguments),
        ),
        "sse" => with_http_session(
            server_id,
            &record.command,
            &record.env,
            true,
            |session| session.tools_call(tool, arguments),
        ),
        _ => Err("mcp_transport_invalid".into()),
    }
}

fn agent_tool_call_mcp_tool(
    context: &AgentRunContext,
    _runtime: &mut AgentToolRuntime,
    arguments: &JsonValue,
) -> JsonValue {
    let server_id = value_first(arguments, &["serverId"]);
    let tool = value_first(arguments, &["tool"]);
    if server_id.trim().is_empty() || tool.trim().is_empty() {
        return agent_tool_error("缺少 serverId 或 tool", "invalid_arguments");
    }
    let connection = match db::open(&context.db_path) {
        Ok(connection) => connection,
        Err(error) => return agent_tool_error(&error, "tool_authorization_unavailable"),
    };
    if !mcp_tool_allowed(&connection, &server_id, &tool).unwrap_or(false) {
        return agent_tool_error(
            "该 MCP 工具未在 allowlist 中；操作者需先在 MCP 集成里启用并放行",
            "mcp_tool_not_allowed",
        );
    }
    let record = match read_server(&connection, &server_id) {
        Ok(row) => row,
        Err(error) => return agent_tool_error(&error, "mcp_server_not_found"),
    };
    if !record.enabled {
        return agent_tool_error("MCP 服务器未启用", "mcp_server_disabled");
    }
    if matches!(
        tool_permission_decision(&connection, "call_mcp_tool"),
        ToolPermissionDecision::Ask
    ) {
        let fencing = current_coordinator_fencing_token(&connection, context);
        let fingerprint = format!(
            "{:x}",
            Sha256::digest(format!("mcp:{server_id}:{tool}:{}", arguments).as_bytes())
        );
        match high_risk_action_gate_on(
            &connection,
            &context.scan_id,
            context.attempt_number,
            &fencing,
            "code_exec",
            &fingerprint,
            &format!("MCP 调用 {server_id}/{tool}"),
            &server_id,
            true,
            Some("call_mcp_tool"),
        ) {
            Ok(HighRiskGate::Allow) => {}
            Ok(HighRiskGate::Pending(request_id)) => {
                return serde_json::json!({
                    "accepted": false,
                    "code": "high_risk_action_pending_approval",
                    "requestId": request_id,
                    "hint": "MCP 调用需操作者单次批准；批准后用相同参数重试",
                });
            }
            Ok(HighRiskGate::Denied(reason)) => {
                return serde_json::json!({"accepted": false, "code": "high_risk_action_denied", "error": reason});
            }
            Err(error) => return agent_tool_error(&error, "high_risk_gate_unavailable"),
        }
    }
    let tool_args = arguments
        .get("arguments")
        .cloned()
        .filter(|value| value.is_object())
        .unwrap_or_else(|| serde_json::json!({}));
    let result = mcp_tools_call(&record, &server_id, &tool, &tool_args);
    match result {
        Ok(value) => {
            let _ = connection.execute(
                "INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json) \
                 VALUES(?1,?2,'mcp_tool',?3,'mcp_tool',?4)",
                params![
                    &context.scan_id,
                    context.attempt_number,
                    format!("{server_id}/{tool}"),
                    serde_json::json!({ "serverId": server_id, "tool": tool, "status": "completed" }).to_string(),
                ],
            );
            serde_json::json!({ "accepted": true, "result": value })
        }
        Err(error) => serde_json::json!({
            "accepted": false,
            "code": "mcp_tool_failed",
            "error": error,
        }),
    }
}

#[tauri::command]
pub fn list_agent_mcp_servers(state: State<AppState>) -> Result<Vec<JsonValue>, String> {
    let connection = db::open(&state.db_path)?;
    ensure_mcp_storage(&connection)?;
    let mut statement = connection
        .prepare(
            "SELECT server_id, label, transport, command, args_json, env_json, enabled, created_at, updated_at \
             FROM agent_mcp_servers ORDER BY server_id",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok(serde_json::json!({
                "serverId": row.get::<_, String>(0)?,
                "label": row.get::<_, String>(1)?,
                "transport": row.get::<_, String>(2)?,
                "command": row.get::<_, String>(3)?,
                "args": serde_json::from_str::<Vec<String>>(&row.get::<_, String>(4)?).unwrap_or_default(),
                "env": serde_json::from_str::<Vec<JsonValue>>(&row.get::<_, String>(5)?).unwrap_or_default(),
                "enabled": row.get::<_, i64>(6)? != 0,
                "createdAt": row.get::<_, String>(7)?,
                "updatedAt": row.get::<_, String>(8)?,
            }))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

#[tauri::command]
pub fn upsert_agent_mcp_server(
    state: State<AppState>,
    server_id: String,
    label: String,
    command: String,
    args: Vec<String>,
    env: Vec<JsonValue>,
    enabled: bool,
    transport: Option<String>,
) -> Result<(), String> {
    let id = server_id.trim();
    if id.is_empty() || id.len() > 64 || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
        return Err("mcp_server_id_invalid".into());
    }
    if command.trim().is_empty() || command.len() > 2048 {
        return Err("mcp_command_invalid".into());
    }
    let transport = normalize_transport(transport.as_deref().unwrap_or("stdio"))?;
    if transport == "stdio" && command.len() > 512 {
        return Err("mcp_command_invalid".into());
    }
    if transport == "http" || transport == "sse" {
        validate_remote_endpoint(&command)?;
    }
    let connection = db::open(&state.db_path)?;
    ensure_mcp_storage(&connection)?;
    connection
        .execute(
            "INSERT INTO agent_mcp_servers(server_id,label,transport,command,args_json,env_json,enabled) \
             VALUES(?1,?2,?3,?4,?5,?6,?7) \
             ON CONFLICT(server_id) DO UPDATE SET \
               label=excluded.label, transport=excluded.transport, command=excluded.command, \
               args_json=excluded.args_json, env_json=excluded.env_json, enabled=excluded.enabled, \
               updated_at=datetime('now','localtime')",
            params![
                id,
                label.trim(),
                transport,
                command.trim(),
                serde_json::to_string(&args).map_err(|error| error.to_string())?,
                serde_json::to_string(&env).map_err(|error| error.to_string())?,
                i64::from(enabled),
            ],
        )
        .map_err(|error| error.to_string())?;
    if let Ok(mut guard) = MCP_SESSIONS.lock() {
        if let Some(map) = guard.as_mut() {
            map.remove(id);
        }
    }
    if let Ok(mut guard) = MCP_HTTP_SESSIONS.lock() {
        if let Some(map) = guard.as_mut() {
            map.remove(id);
        }
    }
    Ok(())
}

#[tauri::command]
pub fn delete_agent_mcp_server(state: State<AppState>, server_id: String) -> Result<(), String> {
    let connection = db::open(&state.db_path)?;
    ensure_mcp_storage(&connection)?;
    connection
        .execute("DELETE FROM agent_mcp_servers WHERE server_id=?1", [&server_id])
        .map_err(|error| error.to_string())?;
    if let Ok(mut guard) = MCP_SESSIONS.lock() {
        if let Some(map) = guard.as_mut() {
            map.remove(&server_id);
        }
    }
    if let Ok(mut guard) = MCP_HTTP_SESSIONS.lock() {
        if let Some(map) = guard.as_mut() {
            map.remove(&server_id);
        }
    }
    Ok(())
}

#[tauri::command]
pub fn get_agent_mcp_tool_allowlist(state: State<AppState>) -> Result<Vec<JsonValue>, String> {
    let connection = db::open(&state.db_path)?;
    let list = mcp_allowlist(&connection)?;
    Ok(list
        .into_iter()
        .map(|(pattern, decision)| serde_json::json!({ "pattern": pattern, "decision": decision }))
        .collect())
}

#[tauri::command]
pub fn set_agent_mcp_tool_allowlist(state: State<AppState>, rules: Vec<JsonValue>) -> Result<(), String> {
    let mut normalized = Vec::new();
    for rule in rules {
        let pattern = rule
            .get("pattern")
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or("mcp_allow_pattern_invalid")?;
        let decision = rule
            .get("decision")
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|value| *value == "allow")
            .ok_or("mcp_allow_decision_invalid")?;
        normalized.push(serde_json::json!({ "pattern": pattern, "decision": decision }));
    }
    let connection = db::open(&state.db_path)?;
    connection
        .execute(
            "INSERT INTO app_settings(key,value) VALUES('agent_mcp_tool_allowlist',?1) \
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [serde_json::to_string(&normalized).map_err(|error| error.to_string())?],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn list_agent_mcp_server_tools(state: State<AppState>, server_id: String) -> Result<Vec<JsonValue>, String> {
    let connection = db::open(&state.db_path)?;
    let record = read_server(&connection, &server_id)?;
    if !record.enabled {
        return Err("mcp_server_disabled".into());
    }
    mcp_tools_list(&record, &server_id)
}

#[tauri::command]
pub fn list_agent_mcp_server_catalog(state: State<AppState>, server_id: String) -> Result<JsonValue, String> {
    let connection = db::open(&state.db_path)?;
    let record = read_server(&connection, &server_id)?;
    if !record.enabled {
        return Err("mcp_server_disabled".into());
    }
    let tools = mcp_tools_list(&record, &server_id)?;
    let resources = mcp_named_list(&record, &server_id, "resources/list", "resources").unwrap_or_default();
    let prompts = mcp_named_list(&record, &server_id, "prompts/list", "prompts").unwrap_or_default();
    Ok(serde_json::json!({
        "tools": tools,
        "resources": resources,
        "prompts": prompts,
        "sampling": "disabled",
    }))
}

pub(crate) fn scan_has_mcp_surface(connection: &rusqlite::Connection) -> bool {
    if ensure_mcp_storage(connection).is_err() {
        return false;
    }
    let enabled: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM agent_mcp_servers WHERE enabled=1",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if enabled == 0 {
        return false;
    }
    !mcp_allowlist(connection).unwrap_or_default().is_empty()
}

#[cfg(test)]
mod mcp_client_tests {
    use super::{mcp_glob_match, mcp_tool_allowed};
    use rusqlite::Connection;

    #[test]
    fn glob_matches_server_tool() {
        assert!(mcp_glob_match("nuclei/*", "nuclei", "scan"));
        assert!(!mcp_glob_match("nuclei/*", "other", "scan"));
    }

    #[test]
    fn resolves_sse_endpoint_against_origin() {
        assert_eq!(
            super::resolve_mcp_url("https://mcp.example/sse", "/messages/?session_id=abc"),
            "https://mcp.example/messages/?session_id=abc"
        );
    }

    #[test]
    fn empty_allowlist_denies() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE app_settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE agent_mcp_servers(server_id TEXT PRIMARY KEY, label TEXT, transport TEXT, command TEXT, args_json TEXT, env_json TEXT, enabled INTEGER, created_at TEXT, updated_at TEXT);",
        )
        .unwrap();
        assert!(!mcp_tool_allowed(&c, "nuclei", "scan").unwrap());
    }
}
