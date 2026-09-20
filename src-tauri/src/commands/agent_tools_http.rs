fn agent_evidence_vocabulary(evidence: &JsonValue) -> HashSet<String> {
    let mut words = HashSet::new();
    for pointer in [
        "/apiCandidates",
        "/routeCandidates",
        "/businessEntrypoints",
        "/opportunities",
        "/investigation/apis",
        "/investigation/actions",
        "/applicationScripts",
    ] {
        let Some(items) = evidence.pointer(pointer).and_then(JsonValue::as_array) else {
            continue;
        };
        for item in items {
            let mut sources = Vec::new();
            for key in ["path", "url", "endpoint", "name", "text", "evidence"] {
                if let Some(value) = item.get(key).and_then(JsonValue::as_str) {
                    sources.push(value.to_string());
                }
            }
            for value in sources {
                for segment in value
                    .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                    .map(str::trim)
                    .filter(|part| part.len() >= 3)
                {
                    words.insert(segment.to_ascii_lowercase());
                }
            }
        }
    }
    if let Some(matches) = evidence.get("localKnowledgeMatches").and_then(JsonValue::as_array) {
        for item in matches {
            if let Some(value) = item.get("text").and_then(JsonValue::as_str) {
                for token in value.split(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
                    if token.len() >= 3 {
                        words.insert(token.to_ascii_lowercase());
                    }
                }
            }
        }
    }
    words
}

fn agent_identity_of(context: &AgentRunContext, key: &str) -> Option<AgentIdentity> {
    if context.identities.len() == 1 {
        // A single-identity task may only use that identity, whatever the model
        // calls it.
        return context.identities.first().cloned();
    }
    context
        .identities
        .iter()
        .find(|value| value.key == key)
        .cloned()
}

/// The task-scoped credential document for one identity. Tools pass the handle
/// around; the material itself never reaches the model.
fn agent_session_document(
    context: &AgentRunContext,
    identity: &AgentIdentity,
) -> Result<Option<JsonValue>, String> {
    let Some(session_id) = identity.session_id.clone() else {
        return Ok(None);
    };
    let connection = db::open(&context.db_path).map_err(|error| error.to_string())?;
    let project_id: i64 = connection
        .query_row(
            "SELECT project_id FROM sentinel_scans WHERE id=?1",
            [&context.scan_id],
            |row| row.get::<_, Option<i64>>(0),
        )
        .ok()
        .flatten()
        .unwrap_or(0);
    crate::auth_session::session_document_for_scan(&connection, &session_id, project_id)
        .map(Some)
        .map_err(|error| format!("身份 {} 的会话不可用：{error}", identity.key))
}

fn agent_identity_headers(context: &AgentRunContext, identity: &AgentIdentity) -> Result<Vec<(String, String)>, String> {
    let Some(document) = agent_session_document(context, identity)? else {
        return Ok(Vec::new());
    };
    let mut headers = Vec::new();
    let cookie_text = document
        .get("cookies")
        .and_then(JsonValue::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|item| {
                    let name = item.get("name").and_then(JsonValue::as_str)?;
                    let value = item.get("value").and_then(JsonValue::as_str)?;
                    Some(format!("{name}={value}"))
                })
                .collect::<Vec<_>>()
                .join("; ")
        })
        .unwrap_or_default();
    if !cookie_text.is_empty() {
        headers.push(("cookie".to_string(), cookie_text));
    }
    if let Some(map) = document.get("headers").and_then(JsonValue::as_object) {
        for (name, value) in map {
            let lowered = name.to_ascii_lowercase();
            if matches!(lowered.as_str(), "cookie" | "host" | "content-length") {
                continue;
            }
            if let Some(text) = value.as_str() {
                headers.push((lowered, text.to_string()));
            }
        }
    }
    Ok(headers)
}

struct AgentHttpRequest {
    identity: AgentIdentity,
    method: String,
    url: String,
    extra_headers: Vec<(String, String)>,
    body: Option<String>,
    content_type: Option<String>,
    contract_key: String,
    family: String,
    /// Which scope rule set this call is decided by (§5.2).
    source: ScopeSource,
    /// Recorded on the request so a coverage claim can say what produced it.
    tool: String,
    timeout_seconds: u64,
}

/// The native HTTP executor. Raw bytes, structure summary and timing are kept
/// for audit; the model receives the same digest fields inline.
fn agent_http_exchange(
    context: &AgentRunContext,
    runtime: &mut AgentToolRuntime,
    request: &AgentHttpRequest,
) -> Result<JsonValue, JsonValue> {
    if AGENT_DENY_METHODS.contains(&request.method.as_str()) {
        return Err(serde_json::json!({
            "error": format!("{} 不允许执行；请先从调用点、表单或实际运行时恢复 method", request.method),
            "code": "unknown_method",
        }));
    }
    let (url, scope_class) =
        agent_scope_check(context, &request.url, &request.method, request.source).map_err(|error| {
            serde_json::json!({"error": error, "code": "scope_denied"})
        })?;
    let mut headers = agent_identity_headers(context, &request.identity).map_err(|error| {
        serde_json::json!({"error": error, "code": "identity_session_unavailable"})
    })?;
    for (name, value) in &request.extra_headers {
        if AGENT_CREDENTIAL_HEADERS.contains(&name.as_str()) || name.eq_ignore_ascii_case("host") {
            continue;
        }
        headers.push((name.clone(), value.clone()));
    }
    if let Some(content_type) = request.content_type.as_deref().filter(|value| !value.trim().is_empty()) {
        headers.push(("content-type".to_string(), content_type.to_string()));
    }
    let mut client_builder = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(request.timeout_seconds.clamp(5, 30)));
    if let Some(proxy) = &context.proxy {
        client_builder = match reqwest::Proxy::all(proxy) {
            Ok(parsed) => client_builder.proxy(parsed),
            Err(error) => {
                return Err(serde_json::json!({"error": error.to_string(), "code": "proxy_invalid"}))
            }
        };
    }
    let client = client_builder
        .build()
        .map_err(|error| serde_json::json!({"error": error.to_string(), "code": "client_failed"}))?;
    let mut call = client
        .request(
            reqwest::Method::from_bytes(request.method.as_bytes())
                .map_err(|_| serde_json::json!({"error":"无效 method","code":"bad_method"}))?,
            &url,
        )
        .header(reqwest::header::USER_AGENT, "oviraptor-native-agent/1.0");
    for (name, value) in &headers {
        if let Ok(parsed) = name.parse::<reqwest::header::HeaderName>() {
            call = call.header(parsed, value.clone());
        }
    }
    if let Some(body) = request.body.as_deref().filter(|value| !value.is_empty()) {
        call = call.body(body.to_string());
    }
    runtime.spend_request();
    let started = Instant::now();
    let mut response = call.send().map_err(|error| {
        serde_json::json!({"error": error.to_string(), "code": "request_failed"})
    })?;
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let header_names: Vec<String> = response
        .headers()
        .keys()
        .map(|key| key.as_str().to_string())
        .collect();
    let redirect_target = response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| parsed_redirect(&url, value));
    let declared_bytes = response.content_length();
    // Read at most the cap: an archive or a stream that never ends must not be
    // buffered whole before it is thrown away.
    use std::io::Read;
    let mut body: Vec<u8> = Vec::new();
    let mut chunk = [0u8; AGENT_RESPONSE_CHUNK_BYTES];
    let mut read_failure: Option<String> = None;
    let mut truncated_early = false;
    loop {
        let room = AGENT_MAX_RESPONSE_BYTES.saturating_sub(body.len());
        if room == 0 {
            break;
        }
        if runtime.cancelled() {
            // Stop pulling bytes; dropping the reader closes the transfer.
            truncated_early = true;
            break;
        }
        let take = room.min(chunk.len());
        match response.read(&mut chunk[..take]) {
            Ok(0) => break,
            Ok(size) => body.extend_from_slice(&chunk[..size]),
            Err(error) => {
                read_failure = Some(error.to_string());
                break;
            }
        }
    }
    if let Some(error) = read_failure {
        return Err(serde_json::json!({"error": error, "code": "body_failed"}));
    }
    let captured = body.len();
    let truncated = truncated_early
        || match declared_bytes {
            Some(declared) => declared > captured as u64,
            // Filling the cap with no declared length means bytes may still be on
            // the wire; one probe byte tells the difference from an exact fit.
            None if captured < AGENT_MAX_RESPONSE_BYTES => false,
            None => {
                let mut probe = [0u8; 1];
                matches!(response.read(&mut probe), Ok(1))
            }
        };
    let elapsed_ms = started.elapsed().as_millis();
    let mut hasher = Sha256::new();
    hasher.update(&body);
    let digest = format!("{:x}", hasher.finalize());
    let text = String::from_utf8_lossy(&body).to_string();
    let structure = agent_response_structure(&text, &content_type);
    body.clear();
    runtime.record_request(AgentRequestTrace {
        method: request.method.to_ascii_uppercase(),
        origin: agent_origin_of(&url),
        path: normalized_investigation_path(
            &reqwest::Url::parse(&url)
                .map(|parsed| parsed.path().to_string())
                .unwrap_or_else(|_| url.clone()),
        ),
        identity: request.identity.key.clone(),
        status: status as i64,
        family: request.family.clone(),
        contract_key: request.contract_key.clone(),
        tool: request.tool.clone(),
        scope_class: scope_class.as_str().to_string(),
        parameters: agent_request_parameters(&url, request.body.as_deref()),
        ..Default::default()
    });
    // §5.4: a 3xx is only ever followed by the browser, and even there an
    // identity-provider jump is observation rather than a test. Out of the frozen
    // scope it stops the action outright.
    let redirect_code = match status {
        301 | 302 | 303 | 307 | 308 => redirect_target
            .as_deref()
            .map(|target| agent_redirect_verdict(context, target, &url))
            .unwrap_or_else(|| "redirect_without_location".to_string()),
        _ => String::new(),
    };
    if is_directory_block_signal(&format!(
        "http {status} {content_type} {}",
        text.chars().take(4_000).collect::<String>()
    )) {
        let stop = AgentStop::new(
            AGENT_STOP_WAF,
            format!("目标返回明确的 WAF、验证码或机器人挑战信号（HTTP {status}）"),
        );
        runtime.protected_stop = Some(stop.clone());
        return Err(serde_json::json!({"code": "protected", "stop": stop}));
    }
    // §11: the site keeps refusing. One 429 is a throttle to work around; two in a
    // row inside the same attempt means this run will not be answered, so it stops
    // as a protection signal rather than burning the remaining budget.
    if status == 429 {
        runtime.rate_limited_streak += 1;
        if runtime.rate_limited_streak >= AGENT_PERSISTENT_RATE_LIMITS {
            let stop = AgentStop::new(
                AGENT_STOP_RATE_LIMIT,
                format!(
                    "目标持续限流（连续 {} 次 HTTP 429），本轮不再继续消耗预算",
                    runtime.rate_limited_streak
                ),
            );
            runtime.protected_stop = Some(stop.clone());
            return Err(serde_json::json!({"code": "protected", "stop": stop}));
        }
    } else {
        runtime.rate_limited_streak = 0;
    }
    let summary = serde_json::json!({
        "identity": request.identity.key,
        "method": request.method,
        "url": url,
        "status": status,
        "contentType": content_type,
        "bytes": captured,
        "declaredBytes": declared_bytes,
        "maxResponseBytes": AGENT_MAX_RESPONSE_BYTES as u64,
        "elapsedMs": elapsed_ms,
        "bodySha256": digest,
        "truncated": truncated,
        "responseHeaderNames": header_names,
        "structureKeys": structure,
        "fields": agent_field_fingerprints(&text, &content_type, &runtime.redaction),
        "preview": agent_text_truncated(&text, AGENT_MAX_PREVIEW_CHARS),
        "contractKey": request.contract_key,
        "requestId": runtime
            .requests
            .last()
            .map(|trace| trace.id.clone())
            .unwrap_or_default(),
        "scopeClass": scope_class.as_str(),
        "redirectCode": redirect_code,
        "redirectTarget": redirect_target,
        // A cached answer is not evidence about the live authorization behaviour.
        "cacheState": agent_cache_state(&headers),
    });
    // §9.3: an input that comes back verbatim is recorded as a reflection point
    // before the field fingerprints are replaced by markers.
    let reflected = agent_reflects_input(
        request,
        summary.get("fields").and_then(JsonValue::as_object),
    );
    // §6.1: the model view is derived first, so the audit record's metadata holds
    // the same redacted facts the model saw and the untouched bytes stay in the
    // payload files next to it.
    let mut view = agent_model_view(summary, runtime);
    let artifact_id = match agent_write_http_record(
        context,
        runtime.target_requests,
        &serde_json::json!({
            "identity": request.identity.key,
            "method": request.method,
            "url": url,
            "headers": headers.iter().map(|(name, value)| serde_json::json!({"name": name, "value": value})).collect::<Vec<_>>(),
            "body": request.body,
            "contractKey": request.contract_key,
        }),
        &view,
        &text,
    ) {
        Ok(artifact) => artifact,
        Err(error) => {
            return Err(serde_json::json!({"error": error, "code": "evidence_write_failed"}))
        }
    };
    runtime.note_artifact(&artifact_id);
    runtime.attach_artifact(&artifact_id, &digest[..12]);
    // §9.2: the family a request was executed for is credited here, from the
    // ledger's own facts — never from what the model said.
    let request_id = runtime
        .requests
        .last()
        .map(|trace| trace.id.clone())
        .unwrap_or_default();
    if reflected && !request_id.is_empty() {
        runtime.reflections.insert(request_id.clone());
    }
    runtime.credit_coverage(
        &request.family,
        "request",
        &request.contract_key,
        &[request_id],
    );
    if let Some(object) = view.as_object_mut() {
        object.insert(
            "rawArtifactId".to_string(),
            JsonValue::String(artifact_id.clone()),
        );
    }
    Ok(view)
}

/// One A/B comparison written out as a difference record: the file a confirmed
/// finding has to cite, so the claim points at two real responses (§10.2).
fn agent_write_diff_record(
    context: &AgentRunContext,
    index: usize,
    record: &JsonValue,
) -> Result<String, String> {
    let directory = context.target_dir.join(AGENT_DIFF_DIRECTORY);
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let mut slot = index.max(1);
    while directory.join(format!("diff-{slot:04}.json")).exists() {
        slot += 1;
    }
    let name = format!("diff-{slot:04}.json");
    write_private_temp_file(
        &directory.join(&name),
        serde_json::json!({"index": slot, "record": record})
            .to_string()
            .as_bytes(),
    )?;
    Ok(name)
}

/// Read a difference record back. `None` means the cited artifact does not exist,
/// which is enough to refuse a confirmed finding (§10.2).
fn agent_read_diff_record(context: &AgentRunContext, artifact_id: &str) -> Option<JsonValue> {
    if artifact_id.is_empty()
        || artifact_id.contains('/')
        || artifact_id.contains("..")
        || !artifact_id.starts_with("diff-")
    {
        return None;
    }
    let path = context
        .target_dir
        .join(AGENT_DIFF_DIRECTORY)
        .join(artifact_id);
    let text = fs::read_to_string(path).ok()?;
    let parsed = serde_json::from_str::<JsonValue>(&text).ok()?;
    crate::agent_runtime::secrets::redact_json(&parsed)
        .get("record")
        .cloned()
}

/// The business parameter names a request carried: query keys plus form or JSON
/// body keys, sorted. Values never enter the ledger (§10.2).
fn agent_request_parameters(url: &str, body: Option<&str>) -> Vec<String> {
    let mut names: Vec<String> = reqwest::Url::parse(url)
        .map(|parsed| parsed.query_pairs().map(|(key, _)| key.to_string()).collect())
        .unwrap_or_default();
    if let Some(body) = body.filter(|value| !value.trim().is_empty()) {
        if let Ok(parsed) = serde_json::from_str::<JsonValue>(body) {
            names.extend(
                parsed
                    .as_object()
                    .map(|map| map.keys().cloned().collect::<Vec<_>>())
                    .unwrap_or_default(),
            );
        } else {
            names.extend(
                body.split('&')
                    .filter_map(|pair| pair.split_once('=').map(|(key, _)| key.to_string())),
            );
        }
    }
    names.sort();
    names.dedup();
    names.iter().take(40).cloned().collect()
}

/// `hit` when a cache in front of the target answered this response (§10.2).
fn agent_cache_state(headers: &[(String, String)]) -> &'static str {
    for (name, value) in headers {
        let lowered = value.trim().to_ascii_lowercase();
        let answered_from_cache = match name.to_ascii_lowercase().as_str() {
            "age" => lowered.parse::<u64>().unwrap_or(0) > 0,
            "x-cache" | "cf-cache-status" | "x-boomerang-delivery-method" => {
                lowered.contains("hit")
            }
            _ => false,
        };
        if answered_from_cache {
            return "hit";
        }
    }
    "miss"
}

/// The model-visible projection of a tool result (§6.1, §6.3). Structure, status
/// codes, field paths, types, hashes and lengths survive; credential and personal
/// values become stable, comparable markers. The untouched bytes live in the raw
/// artifact the caller points at, never here.
fn agent_model_view(summary: JsonValue, runtime: &AgentToolRuntime) -> JsonValue {
    use crate::agent_runtime::secrets::redact_json_with;
    // The HTTP path projects its payload first because the audit record embeds
    // that same view; a second pass would re-scan the markers it just wrote.
    if summary
        .get("redaction")
        .and_then(|value| value.get("applied"))
        .and_then(JsonValue::as_bool)
        == Some(true)
    {
        return summary;
    }
    let mut view = summary;
    if let Some(object) = view.as_object_mut() {
        object.insert(
            "redaction".to_string(),
            serde_json::json!({"applied": true, "mode": "per-run-stable-fingerprint"}),
        );
    }
    redact_json_with(&view, Some(&runtime.redaction))
}

fn agent_response_structure(text: &str, content_type: &str) -> Vec<String> {
    if content_type.contains("json") || text.trim_start().starts_with('{') || text.trim_start().starts_with('[') {
        if let Ok(parsed) = serde_json::from_str::<JsonValue>(text) {
            let mut keys = Vec::new();
            match &parsed {
                JsonValue::Object(map) => keys.extend(map.keys().cloned()),
                JsonValue::Array(rows) => {
                    for row in rows.iter().take(3) {
                        if let Some(map) = row.as_object() {
                            keys.extend(map.keys().cloned());
                        }
                    }
                }
                _ => {}
            }
            keys.sort();
            keys.dedup();
            keys.truncate(40);
            return keys;
        }
    }
    if content_type.contains("html") || text.trim_start().starts_with("<!DOCTYPE") {
        return vec!["html".to_string()];
    }
    Vec::new()
}

/// The raw request/response pair is written to the target directory for audit.
///
/// §6.4 splits the record: `<slot>.json` holds metadata plus the same redacted
/// view the model received, while `<slot>.body` (and `<slot>.request` when a
/// request body exists) holds the untouched bytes. Only the metadata file is ever
/// read back into a prompt or shown by default.
fn agent_write_http_record(
    context: &AgentRunContext,
    index: usize,
    request: &JsonValue,
    response: &JsonValue,
    body_text: &str,
) -> Result<String, String> {
    use crate::agent_runtime::secrets::redact_json;
    let directory = context.target_dir.join(AGENT_HTTP_DIRECTORY);
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    // The per-run sequence restarts on a fresh attempt over the same target
    // directory, so the slot is chosen by what is free rather than by the counter
    // — otherwise the second attempt would fail to record anything at all.
    let mut slot = index.max(1);
    while directory.join(format!("{slot:04}.json")).exists() {
        slot += 1;
    }
    let name = format!("{slot:04}.json");
    let raw_request_body = request
        .get("body")
        .and_then(JsonValue::as_str)
        .filter(|body| !body.is_empty())
        .map(str::to_string);
    let header_names: Vec<String> = request
        .get("headers")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|row| row.get("name").and_then(JsonValue::as_str))
        .map(str::to_string)
        .collect();
    let metadata = serde_json::json!({
        "identity": request.get("identity"),
        "method": request.get("method"),
        "url": request.get("url"),
        "headerNames": header_names,
        "contractKey": request.get("contractKey"),
        "bodyBytes": raw_request_body.as_ref().map(String::len),
    });
    // TECH DEBT (§6.4): the payload files are unredacted and rely on 0600
    // permissions plus the private target directory; they are not yet encrypted
    // with a task-level key or a system-keychain-derived one.
    let record = serde_json::json!({
        "index": slot,
        "request": redact_json(&metadata),
        "response": response,
        "payloadFile": format!("{slot:04}.body"),
        "payloadBytes": body_text.len(),
        "requestPayloadFile": raw_request_body.as_ref().map(|_| format!("{slot:04}.request")),
    });
    write_private_temp_file(&directory.join(&name), record.to_string().as_bytes())?;
    write_private_temp_file(
        &directory.join(format!("{slot:04}.body")),
        body_text.as_bytes(),
    )?;
    if let Some(body) = raw_request_body {
        // The complete header set — including the credential the identity carried —
        // is what makes the raw request replayable for reproduction.
        let headers = request
            .get("headers")
            .and_then(JsonValue::as_array)
            .cloned()
            .unwrap_or_default();
        let dump = format!(
            "{} {}\n{}\n\n{}",
            request
                .get("method")
                .and_then(JsonValue::as_str)
                .unwrap_or("GET"),
            request
                .get("url")
                .and_then(JsonValue::as_str)
                .unwrap_or_default(),
            headers
                .iter()
                .filter_map(|row| Some(format!(
                    "{}: {}",
                    row.get("name")?.as_str()?,
                    row.get("value")?.as_str()?
                )))
                .collect::<Vec<_>>()
                .join("\n"),
            body
        );
        write_private_temp_file(&directory.join(format!("{slot:04}.request")), dump.as_bytes())?;
    }
    Ok(name)
}

#[allow(clippy::too_many_arguments)]
fn agent_http_request(
    context: &AgentRunContext,
    runtime: &mut AgentToolRuntime,
    identity_key: &str,
    method: &str,
    url: &str,
    extra_headers: Vec<(String, String)>,
    body: Option<String>,
    content_type: Option<String>,
    contract_key: &str,
    family: &str,
    source: ScopeSource,
    tool: &str,
) -> Result<JsonValue, JsonValue> {
    let Some(identity) = agent_identity_of(context, identity_key) else {
        return Err(serde_json::json!({
            "error": "该身份不属于当前任务",
            "code": "identity_not_found",
        }));
    };
    agent_http_exchange(
        context,
        runtime,
        &AgentHttpRequest {
            identity,
            method: method.to_ascii_uppercase(),
            url: url.to_string(),
            extra_headers,
            body,
            content_type,
            contract_key: contract_key.to_string(),
            family: family.to_string(),
            source,
            tool: tool.to_string(),
            timeout_seconds: 15,
        },
    )
}
