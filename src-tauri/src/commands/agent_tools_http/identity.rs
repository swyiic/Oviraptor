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
    // Never turn a model-supplied "anonymous" (or a misspelled handle) into
    // the only authenticated account. The recorded identity must match the
    // credential actually sent on the wire, even for a single-account task.
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
    let owner_scan_id: String = connection
        .query_row(
            "SELECT owner_scan_id FROM browser_auth_sessions WHERE id=?1 AND project_id=?2",
            params![session_id, project_id],
            |row| row.get(0),
        )
        .map_err(|_| "登录会话未绑定当前任务".to_string())?;
    if owner_scan_id != context.scan_id {
        return Err("登录会话已从当前任务解绑".into());
    }
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

