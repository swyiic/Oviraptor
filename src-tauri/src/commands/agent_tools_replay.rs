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
    response
}

/// §5: a write request needs an existing contract, an attempt number, a cleanup
/// step and a recovery condition — plus a live human approval row.
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
    match agent_mutation_approved(context, method, url) {
        Ok(()) => None,
        Err(reason) => Some(serde_json::json!({"error": reason, "code": "mutation_not_approved"})),
    }
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
    lookup("/opportunities")
        .or_else(|| lookup("/investigation/hypotheses"))
        .unwrap_or(2)
        .clamp(1, 3)
}

fn agent_mutation_approved(
    context: &AgentRunContext,
    method: &str,
    url: &str,
) -> Result<(), String> {
    let path = reqwest::Url::parse(url)
        .ok()
        .map(|parsed| parsed.path().to_string())
        .unwrap_or_else(|| url.to_string());
    let connection = db::open(&context.db_path).map_err(|error| error.to_string())?;
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
            "{method} {path} 没有未过期的人工授权，原生后端不会执行写请求"
        )),
    }
}
