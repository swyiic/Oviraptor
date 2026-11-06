fn agent_tool_inspect_evidence(context: &AgentRunContext, arguments: &JsonValue) -> JsonValue {
    let kind_argument = value_first(arguments, &["kind"]);
    let kind = if kind_argument.is_empty() { "all" } else { kind_argument.as_str() };
    let query = value_first(arguments, &["query"]).to_ascii_lowercase();
    let limit = arguments
        .get("limit")
        .and_then(JsonValue::as_i64)
        .unwrap_or(12)
        .clamp(1, AGENT_MAX_INSPECT_ITEMS as i64) as usize;
    let buckets: &[(&str, &str)] = match kind {
        "api" => &[("apiCandidates", "/apiCandidates"), ("apis", "/investigation/apis")],
        "action" => &[("actions", "/investigation/actions")],
        "identity" => &[
            ("identityDifferences", "/investigation/identityDifferences"),
            ("identityRuns", "/investigation/identityRuns"),
        ],
        "sensitive" => &[("sensitiveCandidates", "/sensitiveCandidates")],
        "route" => &[("routeCandidates", "/routeCandidates")],
        "opportunity" => &[("opportunities", "/opportunities")],
        _ => &[
            ("apiCandidates", "/apiCandidates"),
            ("opportunities", "/opportunities"),
            ("routeCandidates", "/routeCandidates"),
            ("sensitiveCandidates", "/sensitiveCandidates"),
            ("actions", "/investigation/actions"),
            ("identityDifferences", "/investigation/identityDifferences"),
        ],
    };
    let mut output = serde_json::Map::new();
    for (name, pointer) in buckets {
        let Some(items) = context.evidence.pointer(pointer).and_then(JsonValue::as_array) else {
            continue;
        };
        let matched: Vec<JsonValue> = items
            .iter()
            .filter(|item| {
                query.is_empty() || item.to_string().to_ascii_lowercase().contains(&query)
            })
            .take(limit)
            .map(agent_compact_item)
            .collect();
        if !matched.is_empty() {
            output.insert((*name).to_string(), serde_json::json!(matched));
        }
    }
    let total = output
        .values()
        .map(|value| value.as_array().map_or(0, Vec::len))
        .sum::<usize>();
    serde_json::json!({
        "matched": total,
        "verificationPlan": context.evidence.get("verificationPlan"),
        "investigation": context.evidence.get("investigation"),
        "items": output,
    })
}

/// Keep one inspected item small so reading evidence cannot itself exhaust the
/// context window.
fn agent_compact_item(item: &JsonValue) -> JsonValue {
    let mut map = serde_json::Map::new();
    for key in [
        "path",
        "url",
        "method",
        "parameters",
        "responseKeys",
        "source",
        "confidence",
        "category",
        "title",
        "endpoint",
        "apiKey",
        "leftIdentity",
        "rightIdentity",
        "differenceType",
        "riskScore",
        "type",
        "severity",
        "name",
        "status",
        "key",
        "hypothesisKey",
    ] {
        let Some(value) = item.get(key) else { continue };
        match value {
            JsonValue::String(text) => {
                map.insert(key.to_string(), serde_json::json!(agent_text_truncated(text, 200)));
            }
            JsonValue::Array(values) => {
                let clipped = values
                    .iter()
                    .filter_map(JsonValue::as_str)
                    .take(12)
                    .collect::<Vec<_>>();
                map.insert(key.to_string(), serde_json::json!(clipped));
            }
            JsonValue::Object(_) | JsonValue::Number(_) | JsonValue::Bool(_) => {
                map.insert(key.to_string(), value.clone());
            }
            _ => {}
        }
    }
    JsonValue::Object(map)
}

fn agent_tool_replay_http(
    context: &AgentRunContext,
    runtime: &mut AgentToolRuntime,
    arguments: &JsonValue,
) -> JsonValue {
    let identity_key = value_first(arguments, &["identity"]);
    let method = value_first(arguments, &["method"]).to_ascii_uppercase();
    let url = value_first(arguments, &["url"]);
    let readonly = AGENT_READ_METHODS.contains(&method.as_str());
    let contract_key = value_first(arguments, &["contractKey"]);
    if !readonly {
        if let Some(gate) = agent_write_gate(context, runtime, arguments, &method, &url) {
            return gate;
        }
    }
    let mut extra_headers = Vec::new();
    if let Some(map) = arguments.get("headers").and_then(JsonValue::as_object) {
        for (name, value) in map {
            if let Some(text) = value.as_str() {
                extra_headers.push((name.to_ascii_lowercase(), text.to_string()));
            }
        }
    }
    let family = value_first(arguments, &["family"]);
    let response = match agent_http_request(
        context,
        runtime,
        &identity_key,
        &method,
        &url,
        extra_headers,
        arguments.get("body").and_then(JsonValue::as_str).map(str::to_string),
        arguments
            .get("contentType")
            .and_then(JsonValue::as_str)
            .map(str::to_string),
        &contract_key,
        &family,
        ScopeSource::HttpReplay,
        "replay_http",
    ) {
        Ok(value) => value,
        Err(error) => return error,
    };
    let path = reqwest::Url::parse(&url)
        .ok()
        .map(|parsed| parsed.path().to_string())
        .unwrap_or_else(|| url.clone());
    let new_shape = runtime.record_response_shape(&method, &path, &response);
    runtime.last_progress.new_response_shapes += usize::from(new_shape);
    // A response that only bounced off the frozen scope, or into an identity
    // provider, is boundary evidence: it cannot credit a coverage family (§5.4).
    if agent_redirect_credits_coverage(&response) && runtime.credit_family(&family) {
        runtime.last_progress.new_families += 1;
    }
    let names: Vec<String> = response
        .get("structureKeys")
        .and_then(JsonValue::as_array)
        .map(|rows| rows.iter().filter_map(JsonValue::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    runtime.last_progress.new_parameters += runtime.record_parameters(&method, &path, &names);
    if let Err(error) = persist_agent_evidence(context, &response, "replay_http") {
        return serde_json::json!({"error": error, "code": "evidence_write_failed"});
    }
    if let Some(body) = arguments.get("body").and_then(JsonValue::as_str) {
        if looks_like_html_marker(body) {
            let preview = value_first(&response, &["preview", "body"]);
            let verdict = if preview.contains('<') && preview.contains('>') && !preview.contains("&lt;") {
                "响应里标记仍是原样标签，待复核，不能据此打开 POST"
            } else if preview.contains("&lt;") || preview.contains("&gt;") {
                "响应里标记已被转义，待复核"
            } else {
                "响应里没有看到标记，待复核"
            };
            if let Ok(connection) = db::open(&context.db_path) {
                record_pending_observation(&connection, &context.scan_id, "标记对照", verdict);
            }
        }
    }
    response
}

enum RenderSink { Escaped, Html, Unknown }

fn looks_like_html_marker(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains('<') && lower.contains('>')
}

fn marker_field(body: &str) -> String {
    serde_json::from_str::<JsonValue>(body)
        .ok()
        .and_then(|value| value.as_object().and_then(|map| map.keys().next().cloned()))
        .unwrap_or_default()
}

fn frontend_corpus(evidence: &JsonValue, target_dir: &Path) -> String {
    let mut text = evidence.to_string();
    if text.len() > 200_000 {
        text.truncate(200_000);
    }
    if let Ok(extra) = fs::read_to_string(target_dir.join("frontend-evidence.json")) {
        text.push('\n');
        text.extend(extra.chars().take(200_000));
    }
    text.to_ascii_lowercase()
}

fn browser_page_text(context: &AgentRunContext, field: &str) -> String {
    let Some(browser) = context.browser.clone() else {
        return String::new();
    };
    let control = NativeReconControl::new(Duration::from_secs(20));
    match native_runtime_probe(
        &browser.helper,
        &context.target_url,
        None,
        8,
        6,
        &context.environment.deployment,
        context.proxy.as_deref(),
        &browser.no_proxy,
        &browser.runtime_path,
        &[],
        false,
        Some(field),
        true,
        &control,
    ) {
        Ok(value) => value.to_string().chars().take(80_000).collect(),
        Err(_) => String::new(),
    }
}

fn field_snippet(corpus: &str, field: &str) -> String {
    let field = field.trim().to_ascii_lowercase();
    if field.is_empty() {
        return String::new();
    }
    let Some(index) = corpus.find(&field) else {
        return String::new();
    };
    let from = index.saturating_sub(500);
    let to = (index + field.len() + 700).min(corpus.len());
    corpus[from..to].to_string()
}

fn parse_render_sink(text: &str) -> RenderSink {
    let Some(start) = text.find('{') else { return RenderSink::Unknown };
    let Some(end) = text.rfind('}') else { return RenderSink::Unknown };
    let Ok(value) = serde_json::from_str::<JsonValue>(&text[start..=end]) else {
        return RenderSink::Unknown;
    };
    match value.get("sink").and_then(JsonValue::as_str).unwrap_or("unknown") {
        "escaped" | "text" => RenderSink::Escaped,
        "html" => RenderSink::Html,
        _ => RenderSink::Unknown,
    }
}

fn classify_render_sink(connection: &rusqlite::Connection, corpus: &str, field: &str) -> RenderSink {
    let snippet = field_snippet(corpus, field);
    if snippet.is_empty() {
        return RenderSink::Unknown;
    }
    let Ok(profile) = crate::model_configuration::gateway_profile_on(connection, None) else {
        return RenderSink::Unknown;
    };
    let prompt = format!(
        "只根据下面这段前端代码判断字段 {field} 的输出方式。不要套关键词清单。只输出 JSON {{\"sink\":\"escaped\"|\"html\"|\"unknown\"}}。escaped 表示文本或转义后输出；html 表示会按 HTML 解析；看不出来就是 unknown。\n{snippet}"
    );
    let body = serde_json::json!({
        "model": profile.model,
        "messages": [
            {"role":"system","content":"你只判断一个字段的前端输出方式。只输出 JSON。"},
            {"role":"user","content":prompt}
        ],
        "temperature": 0,
        "max_tokens": 80,
        "stream": false
    });
    let Ok(bytes) = serde_json::to_vec(&body) else { return RenderSink::Unknown };
    let mut request = TransportRequest::post(&profile.endpoint, bytes).header("content-type", "application/json");
    if let Some(bearer) = profile.bearer() {
        request = request.header("authorization", format!("Bearer {bearer}"));
    }
    request.total_timeout = Some(Duration::from_secs(30));
    let Ok(response) = transport::send(request, std::sync::Arc::new(|| false), Box::new(|_| true)) else {
        return RenderSink::Unknown;
    };
    let Ok(parsed) = serde_json::from_slice::<JsonValue>(&response.body) else {
        return RenderSink::Unknown;
    };
    parsed.pointer("/choices/0/message/content").and_then(JsonValue::as_str).map(parse_render_sink).unwrap_or(RenderSink::Unknown)
}

fn record_pending_observation(connection: &rusqlite::Connection, scan_id: &str, title: &str, summary: &str) {
    let project_id: Option<i64> = connection
        .query_row("SELECT project_id FROM sentinel_scans WHERE id=?1", [scan_id], |row| row.get(0))
        .ok();
    let source_hash = format!("{:x}", Sha256::digest(format!("{title}\n{summary}").as_bytes()));
    let payload = serde_json::json!({
        "kind": "observation",
        "cannotEnablePostWrite": true,
        "summary": summary,
    });
    let _ = connection.execute(
        "INSERT OR IGNORE INTO agent_learning_candidates(scan_id,project_id,scan_type,title,summary,candidate_json,status,source_hash) VALUES(?1,?2,'web',?3,?4,?5,'pending',?6)",
        params![scan_id, project_id, title, summary, payload.to_string(), source_hash],
    );
}

fn comparison_attempt_limit(configured: Option<i64>) -> i64 {
    configured.unwrap_or(6).clamp(1, 8)
}

const SCAN_WRITE_PATH_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS scan_post_write (
    scan_id TEXT NOT NULL,
    attempt_number INTEGER NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (scan_id, attempt_number)
);
CREATE TABLE IF NOT EXISTS scan_write_confirmations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    scan_id TEXT NOT NULL,
    attempt_number INTEGER NOT NULL,
    method TEXT NOT NULL,
    endpoint TEXT NOT NULL,
    contract_key TEXT NOT NULL DEFAULT '',
    cleanup TEXT NOT NULL DEFAULT '',
    recovery TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','approved','rejected')),
    expires_at TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
"#;

fn ensure_scan_write_path(connection: &rusqlite::Connection) {
    let _ = connection.execute_batch(SCAN_WRITE_PATH_DDL);
}

fn scan_attempt(connection: &rusqlite::Connection, scan_id: &str) -> i64 {
    connection
        .query_row("SELECT attempt_count FROM sentinel_scans WHERE id=?1", [scan_id], |row| row.get::<_, i64>(0))
        .unwrap_or(1)
        .max(1)
}

fn agent_post_write_enabled(connection: &rusqlite::Connection, scan_id: &str) -> bool {
    ensure_scan_write_path(connection);
    let attempt = scan_attempt(connection, scan_id);
    if connection
        .query_row(
            "SELECT enabled FROM scan_post_write WHERE scan_id=?1 AND attempt_number=?2",
            params![scan_id, attempt],
            |row| row.get::<_, i64>(0),
        )
        .ok()
        .is_some_and(|value| value == 1)
    {
        return true;
    }
    scan_capability_class_authorized(connection, scan_id, attempt, "post_write").unwrap_or(false)
}

/// §5: a write request needs an existing contract, an attempt number, a cleanup
/// step and a recovery condition — plus one live human approval for the series.
/// POST stays off until the operator opens the switch. Full authorization does
/// not open it. Confidence is not a prerequisite: comparison requests are what
/// produce the evidence, and one refusal does not stop the rest of the scan.
fn agent_write_gate(
    context: &AgentRunContext,
    runtime: &mut AgentToolRuntime,
    arguments: &JsonValue,
    method: &str,
    url: &str,
) -> Option<JsonValue> {
    let contract_key = value_first(arguments, &["contractKey"]);
    let cleanup = value_first(arguments, &["cleanup"]);
    let recovery = value_first(arguments, &["recoveryCondition"]);
    if contract_key.is_empty()
        || cleanup.is_empty()
        || recovery.is_empty()
        || arguments.get("attempt").and_then(JsonValue::as_i64).is_none()
    {
        return Some(serde_json::json!({
            "error": "写请求必须提供 contractKey、attempt、cleanup 与 recoveryCondition",
            "code": "mutation_contract_required",
        }));
    }
    let known = agent_evidence_contract_keys(&context.evidence);
    if !known.iter().any(|value| value == &contract_key) {
        return Some(serde_json::json!({
            "error": format!("contractKey {contract_key} 不在本次任务的证据契约中"),
            "code": "contract_unknown",
            "knownContracts": known.iter().take(12).collect::<Vec<_>>(),
        }));
    }
    let used = runtime.contract_attempts.get(&contract_key).copied().unwrap_or(0);
    let limit = agent_evidence_contract_attempts(&context.evidence, &contract_key);
    if used >= limit {
        return Some(serde_json::json!({
            "error": format!("契约 {contract_key} 已用满 {limit} 次尝试"),
            "code": "contract_attempts_exhausted",
        }));
    }
    let Ok(connection) = db::open(&context.db_path) else {
        return Some(serde_json::json!({"error": "无法读取写入开关", "code": "post_write_unavailable"}));
    };
    if !agent_post_write_enabled(&connection, &context.scan_id) {
        return Some(serde_json::json!({
            "error": "POST/上传等写请求默认关闭。全部权限授权也不会自动放开。打开开关并经真人确认后，同一契约可以连续对比多次，单次被拒不会终止扫描。",
            "code": "post_write_disabled",
        }));
    }
    let body = arguments.get("body").and_then(JsonValue::as_str).unwrap_or("");
    if looks_like_html_marker(body) {
        let field = marker_field(body);
        let corpus = frontend_corpus(&context.evidence, &context.target_dir);
        match classify_render_sink(&connection, &corpus, &field) {
            RenderSink::Escaped => {
                record_pending_observation(&connection, &context.scan_id, "渲染点：转义", &format!("{field} 在前端按文本或转义输出，不发标记"));
                return Some(serde_json::json!({
                    "error": "前端对该字段是转义或文本输出，忽略，不发标记。",
                    "code": "render_sink_escaped",
                    "field": field,
                }));
            }
            RenderSink::Unknown => {
                let refreshed = browser_page_text(context, &field);
                let again = if refreshed.is_empty() {
                    RenderSink::Unknown
                } else {
                    classify_render_sink(&connection, &format!("{corpus}\n{refreshed}"), &field)
                };
                match again {
                    RenderSink::Escaped => {
                        record_pending_observation(&connection, &context.scan_id, "渲染点：转义", &format!("{field} 在浏览器页面中按文本或转义输出，不发标记"));
                        return Some(serde_json::json!({
                            "error": "浏览器打开页面后，该字段仍是转义或文本输出，忽略，不发标记。",
                            "code": "render_sink_escaped",
                            "field": field,
                            "browserOpened": true,
                        }));
                    }
                    RenderSink::Html => {
                        record_pending_observation(&connection, &context.scan_id, "渲染点：HTML 解析", &format!("{field} 在浏览器页面中按 HTML 解析。只记录现象，不自动换绕过"));
                    }
                    RenderSink::Unknown => return Some(serde_json::json!({
                        "error": "已尝试打开浏览器查看该字段，仍没有对上渲染点。不能发标记。",
                        "code": "render_sink_unmapped",
                        "field": field,
                        "browserOpened": context.browser.is_some(),
                    })),
                }
            }
            RenderSink::Html => {
                record_pending_observation(&connection, &context.scan_id, "渲染点：HTML 解析", &format!("{field} 按 HTML 解析。只记录现象，不自动换绕过，不能打开 POST"));
            }
        }
    }
    match agent_mutation_approved(context, method, url) {
        Ok(()) => None,
        Err(reason) => {
            let confirmation_id = queue_write_confirmation(
                &connection,
                &context.scan_id,
                method,
                url,
                &contract_key,
                &cleanup,
                &recovery,
            );
            Some(serde_json::json!({
                "error": if looks_like_html_marker(body) {
                    format!("前端按 HTML 解析该字段。只记录现象，不自动换绕过。{reason}")
                } else {
                    reason
                },
                "code": "mutation_not_approved",
                "confirmationId": confirmation_id,
                "renderSink": if looks_like_html_marker(body) { "html" } else { "" },
            }))
        }
    }
}

fn request_endpoint(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .map(|parsed| parsed.path().to_string())
        .unwrap_or_else(|| url.to_string())
}

fn queue_write_confirmation(
    connection: &rusqlite::Connection,
    scan_id: &str,
    method: &str,
    url: &str,
    contract_key: &str,
    cleanup: &str,
    recovery: &str,
) -> Option<i64> {
    ensure_scan_write_path(connection);
    let attempt = scan_attempt(connection, scan_id);
    let endpoint = request_endpoint(url);
    if let Ok(existing) = connection.query_row(
        "SELECT id FROM scan_write_confirmations WHERE scan_id=?1 AND attempt_number=?2 AND method=?3 AND endpoint=?4 AND status='pending'",
        params![scan_id, attempt, method, endpoint],
        |row| row.get::<_, i64>(0),
    ) {
        return Some(existing);
    }
    connection
        .execute(
            "INSERT INTO scan_write_confirmations(scan_id,attempt_number,method,endpoint,contract_key,cleanup,recovery,status) VALUES(?1,?2,?3,?4,?5,?6,?7,'pending')",
            params![scan_id, attempt, method, endpoint, contract_key, cleanup, recovery],
        )
        .ok()?;
    Some(connection.last_insert_rowid())
}

fn agent_evidence_contract_keys(evidence: &JsonValue) -> Vec<String> {
    let mut keys = Vec::new();
    let mut collect = |pointer: &str| {
        let Some(items) = evidence.pointer(pointer).and_then(JsonValue::as_array) else {
            return;
        };
        for item in items {
            let category = value_first(item, &["category", "kind"]);
            let endpoint = value_first(item, &["endpoint", "path", "url"]);
            if category.is_empty() || endpoint.is_empty() {
                continue;
            }
            keys.push(format!(
                "{category}|{}",
                normalized_investigation_path(&endpoint)
            ));
        }
    };
    collect("/opportunities");
    collect("/investigation/hypotheses");
    keys.sort();
    keys.dedup();
    keys
}

fn agent_evidence_contract_attempts(evidence: &JsonValue, contract_key: &str) -> i64 {
    let lookup = |pointer: &str| -> Option<i64> {
        let items = evidence.pointer(pointer)?.as_array()?;
        items.iter().find_map(|item| {
            let category = value_first(item, &["category", "kind"]);
            let endpoint = value_first(item, &["endpoint", "path", "url"]);
            let candidate = format!("{category}|{}", normalized_investigation_path(&endpoint));
            if candidate != contract_key {
                return None;
            }
            item.get("maxAttempts")
                .or_else(|| item.pointer("/contract/maxAttempts"))
                .and_then(JsonValue::as_i64)
        })
    };
    comparison_attempt_limit(
        lookup("/opportunities").or_else(|| lookup("/investigation/hypotheses")),
    )
}

fn agent_mutation_approved(
    context: &AgentRunContext,
    method: &str,
    url: &str,
) -> Result<(), String> {
    let path = request_endpoint(url);
    let connection = db::open(&context.db_path).map_err(|error| error.to_string())?;
    ensure_scan_write_path(&connection);
    let attempt = scan_attempt(&connection, &context.scan_id);
    let series: Option<i64> = connection
        .query_row(
            "SELECT id FROM scan_write_confirmations WHERE scan_id=?1 AND attempt_number=?2 AND method=?3 AND endpoint=?4 AND status='approved' AND datetime(expires_at)>datetime('now','localtime')",
            params![context.scan_id, attempt, method, path],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .flatten();
    if series.is_some() {
        return Ok(());
    }
    let approved: Option<i64> = connection
        .query_row(
            "SELECT a.hypothesis_id FROM investigation_mutation_approvals a JOIN investigation_hypotheses h ON h.id=a.hypothesis_id WHERE h.scan_id=?1 AND a.approved=1 AND datetime(a.expires_at)>datetime('now','localtime') AND json_extract(a.scope_json,'$.method')=?2 AND json_extract(a.scope_json,'$.endpoint')=?3",
            params![context.scan_id, method, path],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .flatten();
    match approved {
        Some(_) => Ok(()),
        None => Err(format!(
            "{method} {path} 没有未过期的人工确认。即使开关已开、权限已全授权，写请求仍要真人当场确认这一次"
        )),
    }
}

#[tauri::command]
pub fn get_scan_post_write_enabled(state: State<AppState>, scan_id: String) -> Result<bool, String> {
    let connection = db::open(&state.db_path)?;
    Ok(agent_post_write_enabled(&connection, &scan_id))
}

#[tauri::command]
pub fn set_scan_post_write_enabled(state: State<AppState>, scan_id: String, enabled: bool) -> Result<(), String> {
    let connection = db::open(&state.db_path)?;
    ensure_scan_write_path(&connection);
    let attempt = scan_attempt(&connection, &scan_id);
    connection
        .execute(
            "INSERT INTO scan_post_write(scan_id,attempt_number,enabled) VALUES(?1,?2,?3) ON CONFLICT(scan_id,attempt_number) DO UPDATE SET enabled=excluded.enabled",
            params![scan_id, attempt, i64::from(enabled)],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn list_scan_write_confirmations(state: State<AppState>, scan_id: String) -> Result<Vec<JsonValue>, String> {
    let connection = db::open(&state.db_path)?;
    ensure_scan_write_path(&connection);
    let attempt = scan_attempt(&connection, &scan_id);
    let mut statement = connection
        .prepare(
            "SELECT id,method,endpoint,contract_key,cleanup,recovery,status,expires_at,created_at FROM scan_write_confirmations WHERE scan_id=?1 AND attempt_number=?2 ORDER BY id DESC LIMIT 20",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![scan_id, attempt], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, i64>(0)?,
                "method": row.get::<_, String>(1)?,
                "endpoint": row.get::<_, String>(2)?,
                "contractKey": row.get::<_, String>(3)?,
                "cleanup": row.get::<_, String>(4)?,
                "recovery": row.get::<_, String>(5)?,
                "status": row.get::<_, String>(6)?,
                "expiresAt": row.get::<_, String>(7)?,
                "createdAt": row.get::<_, String>(8)?,
            }))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

#[tauri::command]
pub fn decide_scan_write_confirmation(state: State<AppState>, confirmation_id: i64, approved: bool) -> Result<(), String> {
    if confirmation_id <= 0 {
        return Err("confirmation_invalid".into());
    }
    let connection = db::open(&state.db_path)?;
    ensure_scan_write_path(&connection);
    let status = if approved { "approved" } else { "rejected" };
    let changed = connection
        .execute(
            "UPDATE scan_write_confirmations SET status=?1, expires_at=CASE WHEN ?1='approved' THEN datetime('now','localtime','+2 hours') ELSE '' END WHERE id=?2 AND status='pending'",
            params![status, confirmation_id],
        )
        .map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err("confirmation_not_pending".into());
    }
    Ok(())
}

#[cfg(test)]
mod post_write_tests {
    use super::{comparison_attempt_limit, looks_like_html_marker, parse_render_sink, RenderSink};

    #[test]
    fn escaped_sink_is_ignored_and_html_is_not_a_bypass() {
        assert!(matches!(parse_render_sink("{\"sink\":\"escaped\"}"), RenderSink::Escaped));
        assert!(matches!(parse_render_sink("{\"sink\":\"html\"}"), RenderSink::Html));
        assert!(matches!(parse_render_sink("no field here"), RenderSink::Unknown));
        assert!(!looks_like_html_marker("plain comment"));
        assert!(looks_like_html_marker("<ovrx1>"));
    }

    #[test]
    fn comparison_series_is_bounded_but_not_a_single_shot() {
        assert_eq!(comparison_attempt_limit(None), 6);
        assert_eq!(comparison_attempt_limit(Some(1)), 1);
        assert_eq!(comparison_attempt_limit(Some(20)), 8);
    }
}
