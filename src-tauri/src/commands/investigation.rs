fn investigation_hash(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    format!("{digest:x}").chars().take(24).collect()
}

fn investigation_json(text: String) -> JsonValue {
    serde_json::from_str(&text).unwrap_or(JsonValue::Null)
}

fn investigation_strings(value: Option<&JsonValue>) -> Vec<String> {
    let mut values = value
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| match item {
            JsonValue::String(text) if !text.trim().is_empty() => Some(text.trim().to_string()),
            JsonValue::Object(_) => item
                .get("name")
                .or_else(|| item.get("key"))
                .or_else(|| item.get("path"))
                .and_then(JsonValue::as_str)
                .filter(|text| !text.trim().is_empty())
                .map(|text| text.trim().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
}

fn sanitized_investigation_response_keys(value: Option<&JsonValue>) -> Vec<String> {
    investigation_strings(value)
        .into_iter()
        .filter(|text| {
            let trimmed = text.trim();
            !trimmed.is_empty()
                && trimmed.len() <= 160
                && !trimmed.starts_with('{')
                && !trimmed.starts_with('[')
                && !trimmed.contains('\n')
                && !trimmed.contains('\r')
                && !trimmed.starts_with("http://")
                && !trimmed.starts_with("https://")
        })
        .take(80)
        .collect()
}

fn investigation_background_noise(api: &JsonValue) -> bool {
    let url = value_first(api, &["url", "path", "apiKey"]).to_ascii_lowercase();
    let path = url
        .split('#').next().unwrap_or(&url)
        .split('?').next().unwrap_or(&url);
    let content_type = value_first(api, &["contentType", "mimeType"]).to_ascii_lowercase();
    let resource_type = value_first(api, &["resourceType"]).to_ascii_lowercase();
    let method = value_first(api, &["method"]).to_ascii_uppercase();
    let source = value_first(api, &["source", "extractionEngine"]).to_ascii_lowercase();
    let static_suffixes = [
        ".avif", ".bmp", ".css", ".eot", ".gif", ".ico", ".jpeg", ".jpg",
        ".map", ".mp3", ".mp4", ".pdf", ".png", ".svg", ".ttf", ".webp",
        ".woff", ".woff2",
    ];
    if static_suffixes.iter().any(|suffix| path.ends_with(suffix))
        || ["image", "media", "font", "stylesheet"].contains(&resource_type.as_str())
        || ["image/", "audio/", "video/", "font/"].iter().any(|prefix| content_type.starts_with(prefix))
    {
        return true;
    }
    if [
        "data_report_web", "sentry", "/envelope", "deviceprofile", "telemetry",
        "/pixel", "/beacon", "/heartbeat", "/healthz", "__webpack_hmr", "sockjs",
    ].iter().any(|marker| path.contains(marker))
    {
        return true;
    }
    if matches!(method.as_str(), "" | "UNKNOWN")
        && !source.contains("browser-runtime")
        && api.get("statusCode").or_else(|| api.get("status")).is_none()
    {
        return true;
    }
    matches!(method.as_str(), "GET" | "HEAD")
        && ["/categories", "/banner", "/feeds", "/feed", "/search/found"]
            .iter().any(|suffix| path.ends_with(suffix))
        || matches!(method.as_str(), "GET" | "HEAD") && path.contains("/welcome_page")
}

fn investigation_site_suffix(host: &str) -> String {
    let parts = host.trim_matches('.').split('.').filter(|part| !part.is_empty()).collect::<Vec<_>>();
    if parts.len() < 2 { return host.to_ascii_lowercase() }
    parts[parts.len() - 2..].join(".").to_ascii_lowercase()
}

fn investigation_related_services_from_target(target_url: &str, target: &JsonValue) -> Vec<JsonValue> {
    let target_host = reqwest::Url::parse(target_url).ok()
        .and_then(|url| url.host_str().map(str::to_string)).unwrap_or_default();
    let target_suffix = investigation_site_suffix(&target_host);
    let requests = target.get("runtimeExploration").and_then(|value| value.get("requests"))
        .and_then(JsonValue::as_array).cloned().unwrap_or_default();
    let mut grouped: HashMap<String, JsonValue> = HashMap::new();
    let mut observed = HashSet::new();
    for request in requests {
        let resource_type = value_first(&request, &["resourceType", "transport"]).to_ascii_lowercase();
        if !["xhr", "fetch", "eventsource", "websocket"].contains(&resource_type.as_str()) { continue }
        if !investigation_background_noise(&request) { continue }
        let url_text = value_first(&request, &["url", "path"]);
        let Ok(url) = reqwest::Url::parse(&url_text) else { continue };
        let host = url.host_str().unwrap_or("").to_ascii_lowercase();
        if host.is_empty() { continue }
        let path = url.path().to_string();
        let lower = format!("{}{}", host, path).to_ascii_lowercase();
        let classification = if ["monitor", "sentry", "envelope", "telemetry", "data_report", "beacon", "pixel"]
            .iter().any(|marker| lower.contains(marker)) {
            "monitoring_telemetry"
        } else if ["deviceprofile", "fingerprint"].iter().any(|marker| lower.contains(marker)) {
            "device_fingerprint"
        } else if ["/categories", "/banner", "/feeds", "/feed", "/welcome_page", "/search/found"]
            .iter().any(|marker| path.to_ascii_lowercase().contains(marker)) {
            "page_bootstrap"
        } else {
            "background_service"
        };
        let identity = value_first(&request, &["identityKey"]);
        let method = {
            let value = value_first(&request, &["method"]).to_ascii_uppercase();
            if value.is_empty() { "UNKNOWN".to_string() } else { value }
        };
        let action = value_first(&request, &["actionId"]);
        let observation_key = format!("{host}|{classification}|{identity}|{method}|{url_text}|{action}");
        if !observed.insert(observation_key) { continue }
        let key = format!("{host}|{classification}");
        let query_keys = url.query_pairs().map(|(name, _)| JsonValue::String(name.into_owned())).collect::<Vec<_>>();
        let status = request.get("statusCode").or_else(|| request.get("status")).and_then(JsonValue::as_i64);
        let relation = if !target_suffix.is_empty() && investigation_site_suffix(&host) == target_suffix { "same_party" } else { "third_party" };
        let row = grouped.entry(key).or_insert_with(|| serde_json::json!({
            "host":host,"classification":classification,"relation":relation,"requestCount":0,
            "methods":[],"paths":[],"queryKeys":[],"identityKeys":[],"resourceTypes":[],"sources":[],"statuses":[],
            "firstUrl":url_text,"evidenceSource":"CDP 运行时网络证据"
        }));
        let Some(object) = row.as_object_mut() else { continue };
        object.insert("requestCount".into(), JsonValue::from(object.get("requestCount").and_then(JsonValue::as_i64).unwrap_or(0) + 1));
        for (field, value) in [
            ("methods", Some(JsonValue::String(method))),
            ("paths", Some(JsonValue::String(path))),
            ("identityKeys", (!identity.is_empty()).then(|| JsonValue::String(identity))),
            ("resourceTypes", Some(JsonValue::String(resource_type))),
            ("sources", Some(JsonValue::String(value_first(&request, &["source", "captureSource"])))),
            ("statuses", status.map(JsonValue::from)),
        ] {
            let Some(value) = value else { continue };
            let values = object.entry(field).or_insert_with(|| JsonValue::Array(Vec::new())).as_array_mut().unwrap();
            if value != JsonValue::String(String::new()) && !values.contains(&value) { values.push(value) }
        }
        if let Some(values) = object.get_mut("queryKeys").and_then(JsonValue::as_array_mut) {
            for value in query_keys { if !values.contains(&value) { values.push(value) } }
        }
    }
    let mut values = grouped.into_values().collect::<Vec<_>>();
    values.sort_by(|left, right| {
        let left_class = value_first(left, &["classification"]);
        let right_class = value_first(right, &["classification"]);
        left_class.cmp(&right_class).then_with(|| value_first(left, &["host"]).cmp(&value_first(right, &["host"])))
    });
    values
}

fn read_investigation_related_services(connection: &rusqlite::Connection, scan_id: &str, target_url: &str) -> Result<Vec<JsonValue>, String> {
    let checkpoint = connection.query_row(
        "SELECT raw_json FROM sentinel_checkpoints WHERE scan_id=?1 AND (?2='' OR url=?2) AND stage='frontend_recon' ORDER BY updated_at DESC,rowid DESC LIMIT 1",
        params![scan_id, target_url],
        |row| row.get::<_, String>(0),
    ).optional().map_err(|error| error.to_string())?;
    Ok(checkpoint.and_then(|raw| serde_json::from_str::<JsonValue>(&raw).ok())
        .map(|target| investigation_related_services_from_target(target_url, &target)).unwrap_or_default())
}

fn sanitize_identity_matrix(matrix: &mut JsonValue) {
    let Some(entries) = matrix.as_object_mut() else { return };
    for observation in entries.values_mut() {
        let Some(object) = observation.as_object_mut() else { continue };
        let keys = sanitized_investigation_response_keys(object.get("responseKeys"));
        object.insert("responseKeys".into(), serde_json::json!(keys));
    }
}

fn normalized_investigation_path(value: &str) -> String {
    let without_fragment = value.split('#').next().unwrap_or(value);
    let without_query = without_fragment.split('?').next().unwrap_or(without_fragment);
    let path = if let Some(scheme) = without_query.find("://") {
        without_query[scheme + 3..]
            .find('/')
            .map(|index| &without_query[scheme + 3 + index..])
            .unwrap_or("/")
    } else {
        without_query
    };
    let mut normalized = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    while normalized.len() > 1 && normalized.ends_with('/') {
        normalized.pop();
    }
    normalized
}

fn identity_diff_endpoint_key(api_key: &str) -> String {
    let parts = api_key.split('|').collect::<Vec<_>>();
    let method = parts.first().copied().unwrap_or("GET").to_ascii_uppercase();
    let path = parts
        .iter()
        .find(|part| part.starts_with('/'))
        .copied()
        .unwrap_or(api_key);
    format!("{method}|{}", normalized_investigation_path(path))
}

fn stable_opportunity_key(opportunity: &JsonValue) -> String {
    let category = value_first(opportunity, &["category"])
        .trim()
        .to_ascii_lowercase();
    let method = {
        let value = value_first(opportunity, &["method"]).to_ascii_uppercase();
        if value.is_empty() { "GET".to_string() } else { value }
    };
    let endpoint = value_first(opportunity, &["endpoint", "url", "path"]);
    if !endpoint.is_empty() {
        return format!("{category}|{method}|{}", normalized_investigation_path(&endpoint));
    }
    let title = value_first(opportunity, &["title"])
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    format!("{category}|{method}|{title}")
}

fn deduplicated_actionable_opportunities(opportunities: &[JsonValue]) -> Vec<JsonValue> {
    let mut grouped: HashMap<String, JsonValue> = HashMap::new();
    for opportunity in opportunities.iter().filter(|item| {
        !opportunity_is_low_value(item) && !opportunity_is_unresolved_static_clue(item)
    }) {
        let key = stable_opportunity_key(opportunity);
        let score = opportunity.get("score").and_then(JsonValue::as_i64).unwrap_or(0);
        let replace = grouped
            .get(&key)
            .map(|current| score > current.get("score").and_then(JsonValue::as_i64).unwrap_or(0))
            .unwrap_or(true);
        if replace {
            grouped.insert(key, opportunity.clone());
        }
    }
    let mut values = grouped.into_values().collect::<Vec<_>>();
    values.sort_by(|left, right| {
        right.get("score").and_then(JsonValue::as_i64).unwrap_or(0)
            .cmp(&left.get("score").and_then(JsonValue::as_i64).unwrap_or(0))
            .then_with(|| stable_opportunity_key(left).cmp(&stable_opportunity_key(right)))
    });
    // Keep all raw evidence in frontend-evidence.json, but keep the model queue
    // bounded and readable instead of creating hundreds of nonce variants.
    values.truncate(24);
    values
}

fn query_parameter_names(value: &str) -> Vec<String> {
    let Some(query) = value.split('?').nth(1).map(|part| part.split('#').next().unwrap_or(part)) else {
        return Vec::new();
    };
    let mut names = query
        .split('&')
        .filter_map(|pair| pair.split('=').next())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
}

fn investigation_node(
    connection: &rusqlite::Connection,
    project_id: Option<i64>,
    scan_id: &str,
    target_url: &str,
    node_key: &str,
    node_type: &str,
    label: &str,
    confidence: &str,
    value_score: i64,
    status: &str,
    payload: &JsonValue,
) -> Result<(), String> {
    connection.execute(
        "INSERT INTO investigation_nodes(project_id,scan_id,target_url,node_key,node_type,label,confidence,value_score,status,payload_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(scan_id,target_url,node_key) DO UPDATE SET project_id=excluded.project_id,node_type=excluded.node_type,label=excluded.label,confidence=excluded.confidence,value_score=excluded.value_score,status=excluded.status,payload_json=excluded.payload_json,last_seen=datetime('now','localtime')",
        params![project_id, scan_id, target_url, node_key, node_type, label, confidence, value_score, status, payload.to_string()],
    ).map_err(|error| error.to_string())?;
    Ok(())
}

fn investigation_edge(
    connection: &rusqlite::Connection,
    project_id: Option<i64>,
    scan_id: &str,
    target_url: &str,
    source_key: &str,
    relation: &str,
    target_key: &str,
    confidence: &str,
    evidence: &JsonValue,
) -> Result<(), String> {
    connection.execute(
        "INSERT OR REPLACE INTO investigation_edges(project_id,scan_id,target_url,source_key,relation,target_key,confidence,evidence_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![project_id, scan_id, target_url, source_key, relation, target_key, confidence, evidence.to_string()],
    ).map_err(|error| error.to_string())?;
    Ok(())
}

fn decorate_verification_contract(mut contract: JsonValue, opportunity: &JsonValue) -> JsonValue {
    if let Some(object) = contract.as_object_mut() {
        let identity_keys = opportunity.get("identityKeys").cloned().unwrap_or_else(|| serde_json::json!([]));
        let identity_runs = opportunity.get("identityRuns").cloned().unwrap_or_else(|| serde_json::json!([]));
        let identity_comparisons = opportunity.get("identityComparisons").cloned().unwrap_or_else(|| serde_json::json!([]));
        object.insert("identityKeys".into(), identity_keys);
        object.insert("identityRuns".into(), identity_runs);
        object.insert("identityComparisons".into(), identity_comparisons);
        object.insert("comparisonRule".into(), serde_json::json!("仅在 A/B 两侧 captureStatus=complete 时允许判定权限差异；否则显示不可比较"));
    }
    contract
}

fn verification_contract(category: &str, opportunity: &JsonValue) -> JsonValue {
    let normalized = category.to_ascii_lowercase();
    let common_stop = serde_json::json!([
        "confirmed_waf_or_challenge",
        "rate_limit_detected",
        "scope_boundary",
        "two_consecutive_no_information_gain"
    ]);
    let endpoint = value_first(opportunity, &["endpoint", "url", "path"]);
    let method = value_first(opportunity, &["method"]).to_ascii_uppercase();
    let parameters = investigation_strings(opportunity.get("parameters"));
    if normalized.contains("idor")
        || normalized.contains("author")
        || normalized.contains("permission")
        || normalized.contains("access")
    {
        return decorate_verification_contract(serde_json::json!({
            "kind":"authorization-differential",
            "objective":"比较至少两个身份对同一业务对象的授权边界，不以单个 401/403 判定会话失效",
            "preconditions":["two_valid_identities_or_authenticated_plus_anonymous","stable_object_reference","same_request_shape"],
            "requiredEvidence":["control_response","cross_identity_response","object_ownership_context","status_body_or_field_difference"],
            "endpoint":endpoint,"method":method,"parameters":parameters,"maxAttempts":3,
            "mutationPolicy":"automatic_bounded_same_contract","successRule":"unauthorized_identity_obtains_protected_object_or_action",
            "stopRules":common_stop
        }), opportunity);
    }
    if normalized.contains("auth") || normalized.contains("session") || normalized.contains("login") {
        return decorate_verification_contract(serde_json::json!({
            "kind":"session-boundary-differential",
            "objective":"比较匿名与有效会话的可达页面、请求头和响应结构",
            "preconditions":["validated_session","anonymous_control"],
            "requiredEvidence":["authenticated_request","anonymous_control","redirect_or_response_difference","session_validity_signal"],
            "endpoint":endpoint,"method":method,"parameters":parameters,"maxAttempts":3,
            "mutationPolicy":"read_only","successRule":"protected_data_or_action_available_without_required_identity",
            "stopRules":common_stop
        }), opportunity);
    }
    if normalized.contains("upload") || normalized.contains("file") {
        return decorate_verification_contract(serde_json::json!({
            "kind":"safe-upload-contract",
            "objective":"仅使用无害标记文件验证类型、存储和访问控制",
            "preconditions":["upload_endpoint_observed","test_artifact_is_benign","no_overwrite"],
            "requiredEvidence":["control_upload_policy","server_response","retrieval_or_rejection_result","cleanup_result"],
            "endpoint":endpoint,"method":method,"parameters":parameters,"maxAttempts":2,
            "mutationPolicy":"automatic_benign_marker_and_cleanup","successRule":"policy_allows_disallowed_type_or_cross_identity_access",
            "stopRules":common_stop
        }), opportunity);
    }
    if normalized.contains("inject") || normalized.contains("sql") || normalized.contains("xss") {
        return decorate_verification_contract(serde_json::json!({
            "kind":"bounded-input-differential",
            "objective":"使用控制值与无害探测值比较确定性响应差异",
            "preconditions":["parameter_observed","stable_control_response"],
            "requiredEvidence":["control_request","test_request","status_timing_or_schema_difference","parameter_source"],
            "endpoint":endpoint,"method":method,"parameters":parameters,"maxAttempts":3,
            "mutationPolicy":"non_destructive_payloads_only","successRule":"repeatable_security_relevant_response_difference",
            "stopRules":common_stop
        }), opportunity);
    }
    if normalized.contains("register") || normalized.contains("account") {
        return decorate_verification_contract(serde_json::json!({
            "kind":"registration-entry-contract",
            "objective":"确认注册入口、字段和前置约束，不自动创建真实账户",
            "preconditions":["registration_entry_observed"],
            "requiredEvidence":["entry_source","field_schema","request_method","server_precondition"],
            "endpoint":endpoint,"method":method,"parameters":parameters,"maxAttempts":2,
            "mutationPolicy":"automatic_discovery_no_account_creation","successRule":"registration_surface_and_constraints_are_reproducible",
            "stopRules":common_stop
        }), opportunity);
    }
    decorate_verification_contract(serde_json::json!({
        "kind":"bounded-evidence-validation",
        "objective":"只验证已有证据指向的安全假设",
        "preconditions":["concrete_endpoint_or_feature","reproducible_control"],
        "requiredEvidence":["source_evidence","control_result","test_result","impact_explanation"],
        "endpoint":endpoint,"method":method,"parameters":parameters,"maxAttempts":2,
            "mutationPolicy":"automatic_bounded_non_destructive","successRule":"repeatable_security_impact_with_request_evidence",
        "stopRules":common_stop
    }), opportunity)
}

fn opportunity_is_low_value(opportunity: &JsonValue) -> bool {
    let method = value_first(opportunity, &["method"]).to_ascii_uppercase();
    let endpoint = value_first(opportunity, &["endpoint", "url", "path"]).to_ascii_lowercase();
    let category = value_first(opportunity, &["category"]).to_ascii_lowercase();
    let context = format!(
        "{} {} {}",
        endpoint,
        category,
        value_first(opportunity, &["title", "source", "feature"]).to_ascii_lowercase()
    );
    if method == "OPTIONS"
        || [
            "sentry", "telemetry", "heartbeat", "healthz", "health-check",
            "deviceprofile", "data_report_web", "report/envelope", "/envelope",
            "tracking", "analytics", "pixel", "beacon", "hot-update",
            "/banner", "/feeds", "welcome_page", "/categories", "get_qrcode_url",
        ]
        .iter()
        .any(|marker| context.contains(marker))
    {
        return true;
    }
    if matches!(method.as_str(), "GET" | "HEAD") {
        let security_markers = [
            "admin", "permission", "privilege", "role", "member", "tenant",
            "auth", "login", "logout", "register", "signup", "oauth", "token",
            "session", "account", "profile", "user", "ownership", "object",
            "upload", "download", "import", "export", "attachment", "file",
            "order", "invoice", "payment", "refund", "coupon", "balance",
            "config", "setting", "system", "audit", "backup", "task",
            "detail", "权限", "账户", "用户", "角色", "对象",
        ];
        let has_object_id = endpoint
            .split(['?', '#'])
            .next()
            .unwrap_or("")
            .split('/')
            .any(|segment| {
                let segment = segment.trim();
                (segment.len() >= 2 && segment.chars().all(|c| c.is_ascii_digit()))
                    || (segment.len() >= 16
                        && segment.chars().all(|c| c.is_ascii_hexdigit() || c == '-'))
            });
        if !security_markers.iter().any(|marker| context.contains(marker)) && !has_object_id {
            return true;
        }
    }
    false
}

/// Decide whether a concrete API is useful as a bounded, read-only baseline
/// even when it does not yet justify a security hypothesis. This deliberately
/// accepts only browser-observed request contracts; AST/string candidates stay
/// in deterministic reconnaissance and never open the model gate by themselves.
fn standard_investigation_api(api: &JsonValue) -> bool {
    let method = value_first(api, &["method"]).to_ascii_uppercase();
    if !matches!(method.as_str(), "GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE") {
        return false;
    }
    let source = value_first(api, &["source", "extractionEngine"]).to_ascii_lowercase();
    let transport = value_first(api, &["resourceType", "transport"]).to_ascii_lowercase();
    let runtime_observed = source.contains("runtime")
        || api.get("runtimeObservation").is_some()
        || matches!(transport.as_str(), "xhr" | "fetch" | "eventsource" | "websocket");
    if !runtime_observed {
        return false;
    }
    let endpoint = value_first(api, &["url", "path"]).to_ascii_lowercase();
    ![
        "sentry", "telemetry", "heartbeat", "healthz", "deviceprofile",
        "data_report_web", "report/envelope", "/envelope", "analytics",
        "pixel", "beacon", "hot-update",
    ]
    .iter()
    .any(|marker| endpoint.contains(marker))
}

/// A source map can preserve an exact client call even when the anonymous
/// landing page does not naturally issue the request. This is stronger than a
/// string/route guess, but weaker than CDP evidence, so only exact read-only
/// calls may open a bounded source-guided investigation.
fn source_mapped_readonly_api(api: &JsonValue) -> bool {
    let method = value_first(api, &["method"]).to_ascii_uppercase();
    if !matches!(method.as_str(), "GET" | "HEAD") {
        return false;
    }
    let source = value_first(api, &["source"]).to_ascii_lowercase();
    let extraction = value_first(api, &["extractionEngine"]).to_ascii_lowercase();
    if !source.contains(".js.map#") && !extraction.contains("babel-ast") {
        return false;
    }
    if value_first(api, &["confidence"]).to_ascii_lowercase() != "high" {
        return false;
    }
    let endpoint = value_first(api, &["url", "path"]);
    if endpoint.is_empty()
        || endpoint.contains('<')
        || endpoint.contains('>')
        || endpoint.contains("${")
        || endpoint.contains("{{")
        || endpoint.to_ascii_lowercase().contains("logout")
    {
        return false;
    }
    // Source maps frequently contain SDK examples and documentation URLs
    // (for example api.github.com). Keep those in the evidence inventory, but
    // never let an unrelated absolute host open an automatic investigation.
    if let Ok(endpoint_url) = reqwest::Url::parse(&endpoint) {
        let source_url = source.split('#').next().and_then(|value| reqwest::Url::parse(value).ok());
        if source_url.as_ref().and_then(reqwest::Url::host_str)
            != endpoint_url.host_str()
        {
            return false;
        }
    }
    !investigation_background_noise(api)
}

fn requested_web_mode_ceiling(connection: &rusqlite::Connection, scan_id: &str) -> String {
    connection
        .query_row(
            "SELECT COALESCE(json_extract(policy_json,'$.webModeCeiling'),'standard') FROM sentinel_scan_contexts WHERE scan_id=?1",
            [scan_id],
            |row| row.get::<_, String>(0),
        )
        .unwrap_or_else(|_| "standard".into())
}

fn opportunity_is_unresolved_static_clue(opportunity: &JsonValue) -> bool {
    let method = value_first(opportunity, &["method"]).to_ascii_uppercase();
    let known_method = matches!(
        method.as_str(),
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS"
    );
    let runtime_observed = value_first(opportunity, &["source"]) == "runtime-request"
        || opportunity
            .pointer("/requestContext/status")
            .and_then(JsonValue::as_i64)
            .is_some_and(|status| status > 0);
    let probe_verified = opportunity
        .pointer("/verification/verified")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    !known_method && !runtime_observed && !probe_verified
}

fn opportunity_agent_readiness(opportunity: &JsonValue) -> (bool, &'static str) {
    let score = opportunity
        .get("score")
        .and_then(JsonValue::as_i64)
        .unwrap_or(0);
    if score < 65 {
        return (false, "score_below_verification_gate");
    }
    if opportunity
        .pointer("/riskEvidence/present")
        .and_then(JsonValue::as_bool)
        != Some(true)
    {
        return (false, "formal_api_without_security_risk_signal");
    }
    let category = value_first(opportunity, &["category"]).to_ascii_lowercase();
    if matches!(category.as_str(), "frontend_feature" | "product_match" | "fallback_discovery") {
        return (false, "discovery_or_template_signal_requires_runtime_evidence");
    }
    if opportunity
        .get("candidateOnly")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        return (false, "inferred_candidate_requires_request_contract");
    }
    let endpoint = value_first(opportunity, &["endpoint", "url", "path"]);
    if endpoint.is_empty() {
        return (false, "missing_endpoint");
    }
    let method = value_first(opportunity, &["method"]).to_ascii_uppercase();
    let endpoint_lower = endpoint.to_ascii_lowercase();
    if method == "OPTIONS"
        || endpoint_lower.contains("data_report_web")
        || endpoint_lower.contains("sentry")
        || endpoint_lower.contains("envelope")
        || endpoint_lower.contains("deviceprofile")
    {
        return (false, "preflight_or_telemetry_request_is_not_a_security_hypothesis");
    }
    if !matches!(
        method.as_str(),
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS"
    ) {
        return (false, "missing_verified_http_method");
    }
    let source = value_first(opportunity, &["source"]);
    let runtime_observed = source == "runtime-request"
        || opportunity
            .pointer("/requestContext/status")
            .and_then(JsonValue::as_i64)
            .is_some_and(|status| status > 0);
    let probe_verified = opportunity
        .pointer("/verification/verified")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let parameters = investigation_strings(opportunity.get("parameters"));
    let direct_contract = source.starts_with("babel-ast")
        && (!parameters.is_empty() || matches!(method.as_str(), "GET" | "HEAD" | "OPTIONS"));
    if runtime_observed || probe_verified || direct_contract {
        (true, "fresh_runtime_or_concrete_request_contract")
    } else {
        (false, "missing_fresh_request_response_evidence")
    }
}

fn scan_identity_keys(
    connection: &rusqlite::Connection,
    scan_id: &str,
) -> Result<Vec<String>, String> {
    let policy = connection
        .query_row(
            "SELECT policy_json FROM sentinel_scan_contexts WHERE scan_id=?1",
            [scan_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .map(investigation_json)
        .unwrap_or(JsonValue::Null);
    let mut ids = investigation_strings(policy.get("authSessionIds"));
    if let Some(primary) = policy
        .get("authSessionId")
        .and_then(JsonValue::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        ids.push(primary.trim().to_string());
    }
    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        return Ok(vec!["anonymous".into()]);
    }
    let mut identities = Vec::new();
    for id in ids {
        let name = connection
            .query_row(
                "SELECT name FROM browser_auth_sessions WHERE id=?1",
                [&id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .unwrap_or_default();
        identities.push(if name.trim().is_empty() {
            format!("session:{id}")
        } else {
            format!("session:{id}:{name}")
        });
    }
    Ok(identities)
}

fn identity_run_summary<'a>(target: &'a JsonValue, identity_key: &str) -> Option<&'a JsonValue> {
    target
        .get("identityRuns")
        .and_then(JsonValue::as_array)?
        .iter()
        .find(|run| value_first(run, &["identityKey"]) == identity_key)
}

fn anonymous_identity(identity_key: &str) -> bool {
    identity_key.trim().eq_ignore_ascii_case("anonymous")
}

fn identity_node_payload(target: &JsonValue, identity_key: &str, index: usize) -> JsonValue {
    let summary = identity_run_summary(target, identity_key);
    serde_json::json!({
        "identityKey": identity_key,
        "identityLabel": summary.map(|run| value_first(run, &["identityLabel"])).filter(|value| !value.is_empty()).unwrap_or_else(|| format!("账号 {}", char::from(b'A' + (index.min(25) as u8)))),
        "sessionValid": summary.and_then(|run| run.get("sessionValid")).cloned().unwrap_or(JsonValue::Null),
        "valid": summary.and_then(|run| run.get("valid")).cloned().unwrap_or(JsonValue::Null),
        "captureStatus": summary.map(|run| value_first(run, &["effectiveCaptureStatus", "captureStatus"])).filter(|value| !value.is_empty()).unwrap_or_else(|| "unknown".into()),
        "runtimeProbeAvailable": summary.and_then(|run| run.get("runtimeProbeAvailable")).and_then(JsonValue::as_bool).unwrap_or(false),
        "validationReason": summary.map(|run| value_first(run, &["validationReason"])).unwrap_or_default(),
        "statusCode": summary.and_then(|run| run.get("statusCode")).cloned().unwrap_or(JsonValue::Null),
        "finalUrl": summary.map(|run| value_first(run, &["finalUrl"])).unwrap_or_default(),
        "stateCount": summary.and_then(|run| run.get("stateCount")).and_then(JsonValue::as_i64).unwrap_or(0),
        "actionCount": summary.and_then(|run| run.get("actionCount")).and_then(JsonValue::as_i64).unwrap_or(0),
        "apiCount": summary.and_then(|run| run.get("apiCount")).and_then(JsonValue::as_i64).unwrap_or(0),
        "replayPlannedCount": summary.and_then(|run| run.get("replayPlannedCount")).and_then(JsonValue::as_i64).unwrap_or(0),
        "replayObservedCount": summary.and_then(|run| run.get("replayObservedCount")).and_then(JsonValue::as_i64).unwrap_or(0),
        "replayCaptureStatus": summary.map(|run| value_first(run, &["replayCaptureStatus"])).unwrap_or_default(),
        "authSessionCapturedRequestCount": summary.and_then(|run| run.get("authSessionCapturedRequestCount")).and_then(JsonValue::as_i64).unwrap_or(0),
        "captureError": summary.map(|run| value_first(run, &["captureError"])).unwrap_or_default(),
        "runtimeStopReason": summary.map(|run| value_first(run, &["runtimeStopReason"])).unwrap_or_default(),
    })
}

fn persist_knowledge_layers(
    connection: &rusqlite::Connection,
    project_id: Option<i64>,
    scan_id: &str,
    target_url: &str,
    target: &JsonValue,
    apis: &[(String, JsonValue)],
    hypotheses: &[(String, String)],
    stop_reason: &str,
) -> Result<(), String> {
    let Some(project_id) = project_id else { return Ok(()) };
    for (api_key, api) in apis {
        let method = value_first(api, &["method"]);
        let path = normalized_investigation_path(&value_first(api, &["url", "path"]));
        let fact_key = format!("api:{}", investigation_hash(&format!("{method}|{path}")));
        let evidence_hash = investigation_hash(&api.to_string());
        connection.execute(
            "INSERT INTO knowledge_facts(project_id,fact_key,fact_type,subject,predicate,object_json,confidence,source_scan_id,target_url,evidence_hash) VALUES(?1,?2,'api',?3,'exposes',?4,?5,?6,?7,?8) ON CONFLICT(project_id,fact_key,source_scan_id,target_url) DO UPDATE SET object_json=excluded.object_json,confidence=excluded.confidence,evidence_hash=excluded.evidence_hash,last_seen=datetime('now','localtime')",
            params![project_id, fact_key, target_url, serde_json::json!({"apiKey":api_key,"method":method,"path":path,"parameters":investigation_strings(api.get("parameters"))}).to_string(), value_first(api, &["confidence"]), scan_id, target_url, evidence_hash],
        ).map_err(|error| error.to_string())?;
    }
    if let Some(fingerprint) = target.get("fingerprint") {
        for layer in ["frontend", "backend", "server", "waf", "cdn"] {
            let Some(value) = fingerprint.get(layer) else { continue };
            let name = value_first(value, &["name"]);
            if name.is_empty() || name.eq_ignore_ascii_case("unknown") {
                continue;
            }
            let fact_key = format!("technology:{}", investigation_hash(&format!("{layer}|{name}")));
            connection.execute(
                "INSERT INTO knowledge_facts(project_id,fact_key,fact_type,subject,predicate,object_json,confidence,source_scan_id,target_url,evidence_hash) VALUES(?1,?2,'technology',?3,?4,?5,?6,?7,?3,?8) ON CONFLICT(project_id,fact_key,source_scan_id,target_url) DO UPDATE SET object_json=excluded.object_json,confidence=excluded.confidence,evidence_hash=excluded.evidence_hash,last_seen=datetime('now','localtime')",
                params![project_id, fact_key, target_url, layer, value.to_string(), value_first(value, &["confidence"]), scan_id, investigation_hash(&value.to_string())],
            ).map_err(|error| error.to_string())?;
        }
    }
    for (hypothesis_key, category) in hypotheses {
        let strategy_key = format!("bounded:{}", category.to_ascii_lowercase());
        connection.execute(
            "INSERT INTO knowledge_outcomes(project_id,scan_id,target_url,hypothesis_key,strategy_key,outcome,stop_reason,evidence_json) VALUES(?1,?2,?3,?4,?5,'not_executed',?6,'{}') ON CONFLICT(scan_id,target_url,hypothesis_key,strategy_key) DO UPDATE SET stop_reason=excluded.stop_reason",
            params![project_id, scan_id, target_url, hypothesis_key, strategy_key, stop_reason],
        ).map_err(|error| error.to_string())?;
        let contract = verification_contract(category, &JsonValue::Null);
        connection.execute(
            "INSERT INTO knowledge_strategies(project_id,strategy_key,category,title,conditions_json,playbook_json) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(project_id,strategy_key) DO UPDATE SET playbook_json=excluded.playbook_json,updated_at=datetime('now','localtime')",
            params![project_id, strategy_key, category, format!("{} 的有界验证", category), serde_json::json!({"requiresDeterministicEvidence":true,"requiresIndependentSupport":2}).to_string(), contract.to_string()],
        ).map_err(|error| error.to_string())?;
        connection.execute(
            "UPDATE knowledge_strategies SET support_count=(SELECT COUNT(DISTINCT scan_id||'|'||target_url) FROM knowledge_outcomes WHERE project_id=?1 AND strategy_key=?2),success_count=(SELECT COUNT(*) FROM knowledge_outcomes WHERE project_id=?1 AND strategy_key=?2 AND outcome IN ('validated','confirmed')),failure_count=(SELECT COUNT(*) FROM knowledge_outcomes WHERE project_id=?1 AND strategy_key=?2 AND outcome IN ('rejected','failed','exhausted')),promoted=CASE WHEN (SELECT COUNT(DISTINCT scan_id||'|'||target_url) FROM knowledge_outcomes WHERE project_id=?1 AND strategy_key=?2)>=2 OR (SELECT COUNT(*) FROM knowledge_outcomes WHERE project_id=?1 AND strategy_key=?2 AND outcome IN ('validated','confirmed'))>0 THEN 1 ELSE 0 END,updated_at=datetime('now','localtime') WHERE project_id=?1 AND strategy_key=?2",
            params![project_id, strategy_key],
        ).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn persist_identity_differences(
    connection: &rusqlite::Connection,
    project_id: Option<i64>,
    scan_id: &str,
    target_url: &str,
    identity_keys: &[String],
) -> Result<(), String> {
    let Some(project_id) = project_id else { return Ok(()) };
    if identity_keys.len() < 2 {
        return Ok(());
    }
    let mut existing_endpoints = HashSet::new();
    {
        let mut statement = connection
            .prepare("SELECT api_key FROM investigation_identity_diffs WHERE scan_id=?1 AND target_url=?2")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![scan_id, target_url], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;
        for row in rows {
            existing_endpoints.insert(identity_diff_endpoint_key(&row.map_err(|error| error.to_string())?));
        }
    }
    let mut baselines: HashMap<String, HashSet<String>> = HashMap::new();
    for identity in identity_keys {
        let raw = connection
            .query_row(
                "SELECT api_signatures_json FROM investigation_baselines WHERE project_id=?1 AND target_url=?2 AND identity_key=?3 ORDER BY created_at DESC,id DESC LIMIT 1",
                params![project_id, target_url, identity],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .unwrap_or_else(|| "[]".into());
        baselines.insert(
            identity.clone(),
            investigation_strings(Some(&investigation_json(raw)))
                .into_iter()
                .collect(),
        );
    }
    for left_index in 0..identity_keys.len() {
        for right_index in left_index + 1..identity_keys.len() {
            let left = &identity_keys[left_index];
            let right = &identity_keys[right_index];
            let left_apis = baselines.get(left).cloned().unwrap_or_default();
            let right_apis = baselines.get(right).cloned().unwrap_or_default();
            for api_signature in left_apis.symmetric_difference(&right_apis) {
                let endpoint_key = identity_diff_endpoint_key(api_signature);
                if existing_endpoints.contains(&endpoint_key) {
                    continue;
                }
                let present_left = left_apis.contains(api_signature);
                let matrix = serde_json::json!({
                    "left":{"identity":left,"observed":present_left},
                    "right":{"identity":right,"observed":!present_left},
                    "note":"可达性差异是权限边界候选，不会直接判定为漏洞"
                });
                connection.execute(
                    "INSERT OR REPLACE INTO investigation_identity_diffs(project_id,scan_id,target_url,api_key,left_identity_key,right_identity_key,difference_type,risk_score,status,matrix_json) VALUES(?1,?2,?3,?4,?5,?6,'reachability',55,'observed',?7)",
                    params![project_id, scan_id, target_url, api_signature, left, right, matrix.to_string()],
                ).map_err(|error| error.to_string())?;
                existing_endpoints.insert(endpoint_key);
            }
        }
    }
    Ok(())
}

fn manual_api_evidence(apis: &[(String, JsonValue)], markers: &[&str]) -> Vec<String> {
    let mut evidence = apis
        .iter()
        .filter_map(|(_, api)| {
            let raw_method = value_first(api, &["method"]).to_ascii_uppercase();
            let method = if raw_method.is_empty() {
                "HTTP".to_string()
            } else {
                raw_method
            };
            let endpoint = value_first(api, &["url", "path"]);
            let parameters = investigation_strings(api.get("parameters"));
            let searchable = format!("{} {}", endpoint, parameters.join(" ")).to_ascii_lowercase();
            markers
                .iter()
                .any(|marker| searchable.contains(marker))
                .then(|| format!("{method} {}", normalized_investigation_path(&endpoint)))
        })
        .collect::<Vec<_>>();
    evidence.sort();
    evidence.dedup();
    evidence.truncate(3);
    evidence
}

fn manual_deep_dive_item(
    category: &str,
    title: &str,
    priority: &str,
    reason: &str,
    evidence: Vec<String>,
    missing_evidence: &str,
    steps: &[&str],
    stop_condition: &str,
) -> JsonValue {
    serde_json::json!({
        "category":category,
        "title":title,
        "priority":priority,
        "reason":reason,
        "evidence":evidence,
        "missingEvidence":missing_evidence,
        "steps":steps,
        "stopCondition":stop_condition,
        "classification":"coverage_gap_not_vulnerability",
        "source":"deterministic-src-coverage"
    })
}

/// Produce high-yield human follow-up from deterministic coverage gaps. These
/// rows are never vulnerability findings: they explain what the bounded Web
/// run did not reach and the exact business evidence needed to continue.
fn manual_deep_dive_plan(
    target: &JsonValue,
    apis: &[(String, JsonValue)],
    actions: &[JsonValue],
    identity_keys: &[String],
    mode: &str,
) -> JsonValue {
    let mut surface_parts = Vec::new();
    for (_, api) in apis {
        surface_parts.push(value_first(api, &["method"]));
        surface_parts.push(value_first(api, &["url", "path"]));
        surface_parts.extend(investigation_strings(api.get("parameters")));
    }
    for action in actions {
        surface_parts.push(value_first(
            action,
            &["label", "role", "outcome", "afterUrl"],
        ));
    }
    for key in [
        "businessEntrypoints",
        "registrationEntrypoints",
        "routeCandidates",
    ] {
        surface_parts.extend(investigation_strings(target.get(key)));
    }
    let surface = surface_parts.join(" ").to_ascii_lowercase();
    let has_any = |markers: &[&str]| markers.iter().any(|marker| surface.contains(marker));
    let authenticated_identities = identity_keys
        .iter()
        .filter(|identity| !anonymous_identity(identity))
        .count();
    let write_api_count = apis
        .iter()
        .filter(|(_, api)| {
            matches!(
                value_first(api, &["method"])
                    .to_ascii_uppercase()
                    .as_str(),
                "POST" | "PUT" | "PATCH" | "DELETE"
            )
        })
        .count();
    let mut leads = Vec::new();

    let object_markers = [
        "user_id", "uid", "/user/", "/users/", "account", "member", "tenant", "org_id",
        "role", "owner", "project_id", "team_id", "/detail", "/admin",
    ];
    if !apis.is_empty() && (authenticated_identities < 2 || has_any(&object_markers)) {
        leads.push(manual_deep_dive_item(
            "authorization",
            "同级账号、对象归属与字段级权限",
            "critical",
            if authenticated_identities < 2 {
                "自动化没有两套独立有效身份，无法覆盖同级账号与跨租户对象边界。"
            } else {
                "已观察到对象、用户、租户或角色相关契约，仍需结合真实对象归属确认字段级读写边界。"
            },
            manual_api_evidence(apis, &object_markers),
            if authenticated_identities < 2 {
                "两个独立平权账号、各自拥有的对象以及完整双侧响应"
            } else {
                "明确的对象所有者、非所有者和字段修改前后状态"
            },
            &[
                "为两个账号各准备一个可识别测试对象",
                "固定同一方法和请求形状，仅替换对象引用",
                "分别比较状态、字段、对象归属和实际副作用",
            ],
            "完成主要对象类型的所有者/非所有者对照，或连续三个代表性对象均无权限差异",
        ));
    }

    let auth_markers = [
        "login", "logout", "register", "signup", "reset", "recover", "password", "captcha",
        "qrcode", "oauth", "mfa", "bind", "token", "session",
    ];
    if has_any(&auth_markers) {
        leads.push(manual_deep_dive_item(
            "authentication_session",
            "登录、找回、绑定与令牌生命周期",
            "high",
            "身份流程包含一次性状态、跨页面跳转或客户端令牌，自动重放无法完整理解所有生命周期约束。",
            manual_api_evidence(apis, &auth_markers),
            "可控测试账号、旧/新令牌、跨浏览器状态和完整找回或绑定流程",
            &[
                "记录登录前后、退出后和改密后的令牌有效性",
                "检查找回/绑定步骤能否跳步、重放或跨账号复用",
                "比较错误账号、错误验证码和不存在账号的可观察差异",
            ],
            "关键令牌均按预期失效，流程步骤不可跨账号复用且无稳定枚举差异",
        ));
    }

    let business_markers = [
        "order", "pay", "payment", "refund", "coupon", "balance", "points", "credit",
        "invite", "claim", "redeem", "approve", "audit", "workflow", "/status", "_status",
        "quota", "stock", "subscribe",
    ];
    if has_any(&business_markers) {
        leads.push(manual_deep_dive_item(
            "business_flow",
            "业务状态机、次数/额度与流程跳步",
            "critical",
            "发现订单、权益、审核、额度或状态相关入口；这类风险依赖公司业务不变量，不能仅凭通用模型判定。",
            manual_api_evidence(apis, &business_markers),
            "正常状态图、允许的顺序/次数/额度、隔离测试数据和回滚方式",
            &[
                "画出正常状态转换和服务端认可的最终状态",
                "验证跳步、重复提交、乱序调用和跨入口操作",
                "检查数量、金额、次数、时间和边界值是否由服务端统一校验",
            ],
            "关键状态转换、额度和幂等性均由服务端约束，且测试数据已恢复",
        ));
    }

    let file_markers = [
        "upload", "download", "file", "attachment", "avatar", "import", "export", "archive",
        "template", "document", "image", "pdf",
    ];
    if has_any(&file_markers) {
        leads.push(manual_deep_dive_item(
            "file_handling",
            "文件、导入导出与对象存储边界",
            "high",
            "文件处理通常跨越上传、解析、存储、下载和异步任务，单次只读验证覆盖不完整。",
            manual_api_evidence(apis, &file_markers),
            "无害样本、下载对象归属、清理接口以及解析完成后的结果",
            &[
                "先验证跨账号下载与签名链接有效期",
                "用无害样本检查类型、文件名、压缩包和解析结果",
                "确认上传对象不可覆盖他人资源并完成清理",
            ],
            "下载授权、存储隔离和解析边界均有证据，所有测试文件已删除",
        ));
    }

    let integration_markers = [
        "url", "uri", "callback", "webhook", "redirect", "preview", "fetch", "proxy",
        "remote", "rss", "convert", "source", "image_url",
    ];
    if has_any(&integration_markers) {
        leads.push(manual_deep_dive_item(
            "server_side_integration",
            "服务端取 URL、Webhook 与不可信上游",
            "high",
            "发现 URL、回调、预览或远程资源参数，可能存在异步消费、重定向或第三方响应信任边界。",
            manual_api_evidence(apis, &integration_markers),
            "目标可达的唯一回连地址、异步任务结果和服务端实际取回证据",
            &[
                "确认参数是否由服务端而非浏览器访问",
                "分别观察直接地址、受控重定向和异步消费结果",
                "记录上游内容类型、大小、超时和错误回退行为",
            ],
            "没有服务端访问证据，或所有可控地址均被稳定拒绝且异步任务已结束",
        ));
    }

    let realtime_markers = [
        "graphql", "websocket", "wss", "eventsource", "sse", "subscribe", "subscription",
        "socket", "stream",
    ];
    if has_any(&realtime_markers) {
        leads.push(manual_deep_dive_item(
            "realtime_api",
            "GraphQL、订阅与实时消息授权",
            "high",
            "实时协议和 GraphQL 的授权边界位于消息、字段或订阅层，普通 HTTP 接口清单无法完整表达。",
            manual_api_evidence(apis, &realtime_markers),
            "握手请求、消息结构、订阅对象、断线重连和双身份消息证据",
            &[
                "记录握手与首个业务消息",
                "使用两个账号比较订阅对象和字段范围",
                "验证断线重连、令牌失效与取消订阅后的消息边界",
            ],
            "主要订阅和字段均完成双身份对照，失效会话不能继续收到受保护消息",
        ));
    }

    let client_trust_markers = [
        "nonce", "hkey", "signature", "_sign", "sign=", "/sign", "client_type",
        "client_version", "device_id", "x_app", "timestamp",
    ];
    if has_any(&client_trust_markers) {
        leads.push(manual_deep_dive_item(
            "client_trust",
            "客户端签名、版本与设备信任",
            "medium",
            "请求包含客户端签名、版本、设备或时间字段；需要确认服务端校验的是安全边界还是仅兼容性参数。",
            manual_api_evidence(apis, &client_trust_markers),
            "字段生成位置、服务端失败响应和当前会话重新签名能力",
            &[
                "分别移除、固定和正常更新非认证字段",
                "比较服务端是否只依赖客户端可控版本/设备标识",
                "避免把公开算法本身当作漏洞，只记录可重复的服务端信任缺陷",
            ],
            "字段缺失和篡改均被服务端按预期处理，或确认其不承担安全边界",
        ));
    }

    let resource_markers = [
        "search", "query", "list", "batch", "export", "report", "page", "size", "limit",
        "offset", "count",
    ];
    if has_any(&resource_markers) {
        leads.push(manual_deep_dive_item(
            "resource_consumption",
            "分页、批量、复杂查询与资源配额",
            "medium",
            "发现列表、搜索、报表或批量参数；自动化不会在不清楚生产容量时扩大负载。",
            manual_api_evidence(apis, &resource_markers),
            "安全测试窗口、可接受速率、最大分页/导出规模和任务配额",
            &[
                "先确认单请求的服务端上限",
                "在批准的低速窗口检查分页、批量和异步任务配额",
                "观察同账号、同 IP 和跨账号限制是否一致",
            ],
            "达到约定安全上限或确认服务端存在稳定的容量与并发限制",
        ));
    }

    if write_api_count > 0 && has_any(&business_markers) {
        leads.push(manual_deep_dive_item(
            "race_condition",
            "幂等、重复消费与并发一致性",
            "high",
            "存在状态变更接口和敏感业务对象，但缺少可恢复业务不变量时自动化不会并发写入。",
            manual_api_evidence(apis, &business_markers),
            "隔离测试对象、期望不变量、幂等键、并发前后状态和清理方案",
            &[
                "先以顺序请求建立正常结果",
                "在安全窗口对同一测试对象做小规模同步并发",
                "核对最终状态、次数、余额/库存及重复副作用",
            ],
            "业务不变量保持成立且测试对象恢复；出现异常立即停止并保存前后状态",
        ));
    }

    if apis.len() < 6 || actions.len() < 2 || mode == "deep" {
        leads.push(manual_deep_dive_item(
            "api_inventory",
            "未触发功能、旧版/移动端与影子 API",
            if apis.len() < 3 { "high" } else { "medium" },
            "Web 自动探索只证明当前页面状态实际触发的接口，不能代表移动端、合作方、旧版本或隐藏管理入口。",
            vec![format!(
                "当前仅形成 {} 个正式接口、{} 个页面动作",
                apis.len(),
                actions.len()
            )],
            "其它客户端流量、接口文档、旧版本路径、内部域名或更多业务角色",
            &[
                "优先查看页面未触发菜单和真实移动端/旧版客户端请求",
                "对已观察公共前缀查找版本、管理、批量和调试分支",
                "只保留有真实响应和业务归属的接口，避免字符串拼接噪音",
            ],
            "新增入口不再产生正式业务接口，或已覆盖所有在用客户端与角色",
        ));
    }

    if leads.is_empty() {
        leads.push(manual_deep_dive_item(
            "business_semantics",
            "业务语义与跨渠道一致性复核",
            "medium",
            "当前自动契约没有形成额外高价值风险信号，但业务规则、跨渠道状态和异常恢复仍无法由通用模型完整推断。",
            vec![format!(
                "已形成 {} 个正式接口、{} 个页面动作",
                apis.len(),
                actions.len()
            )],
            "业务规则、角色矩阵、异常状态和其它客户端行为",
            &[
                "选择最重要的一个业务结果并写出服务端不变量",
                "比较正常、越界、重复和中断恢复路径",
                "确认不同客户端和角色得到一致的服务端约束",
            ],
            "核心不变量均有对照证据，且没有新的接口、状态或对象边界出现",
        ));
    }

    let limit = match mode {
        "quick" => 3,
        "deep" => 8,
        _ => 5,
    };
    leads.truncate(limit);
    for (index, lead) in leads.iter_mut().enumerate() {
        if let Some(object) = lead.as_object_mut() {
            object.insert("rank".into(), ((index + 1) as i64).into());
        }
    }
    JsonValue::Array(leads)
}

pub(crate) fn persist_investigation_graph(
    connection: &rusqlite::Connection,
    project_id: Option<i64>,
    scan_id: &str,
    target_url: &str,
    target: &JsonValue,
) -> Result<InvestigationMetrics, String> {
    for table in [
        "investigation_edges",
        "investigation_nodes",
        "investigation_actions",
        "investigation_api_models",
        "investigation_identity_diffs",
    ] {
        connection.execute(
            &format!("DELETE FROM {table} WHERE scan_id=?1 AND target_url=?2"),
            params![scan_id, target_url],
        ).map_err(|error| error.to_string())?;
    }

    let root_key = format!("target:{}", investigation_hash(target_url));
    investigation_node(connection, project_id, scan_id, target_url, &root_key, "target", target_url, "high", 100, "observed", &serde_json::json!({"url":target_url}))?;
    let identity_keys = scan_identity_keys(connection, scan_id)?;
    for (identity_index, identity) in identity_keys
        .iter()
        .filter(|identity| !anonymous_identity(identity))
        .enumerate()
    {
        let identity_node_key = format!("identity:{}", investigation_hash(identity));
        let payload = identity_node_payload(target, identity, identity_index);
        let label = value_first(&payload, &["identityLabel"]);
        let status = if payload.get("sessionValid").and_then(JsonValue::as_bool) == Some(true) {
            "active"
        } else if payload.get("sessionValid").and_then(JsonValue::as_bool) == Some(false) {
            "invalid"
        } else {
            "unknown"
        };
        investigation_node(connection, project_id, scan_id, target_url, &identity_node_key, "identity", &label, "high", 70, status, &payload)?;
        investigation_edge(connection, project_id, scan_id, target_url, &root_key, "observed_as", &identity_node_key, "high", &serde_json::json!({"source":"scan-policy"}))?;
    }

    let exploration = target.get("runtimeExploration").unwrap_or(&JsonValue::Null);
    let states = exploration.get("states").and_then(JsonValue::as_array).cloned().unwrap_or_default();
    let actions = exploration.get("actions").and_then(JsonValue::as_array).cloned().unwrap_or_default();
    let mut runtime_requests = exploration.get("requests").and_then(JsonValue::as_array).cloned().unwrap_or_default();
    if runtime_requests.is_empty() {
        if let Some(auth_requests) = exploration.get("authSessionRequests").and_then(JsonValue::as_array) {
            runtime_requests = auth_requests.clone();
        }
    }
    let coverage = exploration.get("coverage").cloned().unwrap_or_else(|| serde_json::json!({}));

    for state in &states {
        let state_id = value_first(state, &["id"]);
        if state_id.is_empty() { continue }
        let state_key = format!("state:{state_id}");
        let label = value_first(state, &["title", "url"]);
        let score = state.get("highValueLabels").and_then(JsonValue::as_array).map(|items| (items.len() as i64 * 8).min(40) + 35).unwrap_or(35);
        investigation_node(connection, project_id, scan_id, target_url, &state_key, "page_state", &label, "high", score, "observed", state)?;
        investigation_edge(connection, project_id, scan_id, target_url, &root_key, "contains_state", &state_key, "high", &serde_json::json!({"discoveredFrom":value_first(state, &["discoveredFrom"])}))?;
    }

    for action in &actions {
        let action_id = value_first(action, &["id"]);
        if action_id.is_empty() { continue }
        let action_key = format!("action:{action_id}");
        let state_id = value_first(action, &["stateId"]);
        let state_key = if state_id.is_empty() { String::new() } else { format!("state:{state_id}") };
        let label = value_first(action, &["label", "role"]);
        let request_count = action.get("requestCount").and_then(JsonValue::as_i64).unwrap_or(0);
        let state_changed = action.get("stateChanged").and_then(JsonValue::as_bool).unwrap_or(false);
        let value_score = (action.get("score").and_then(JsonValue::as_i64).unwrap_or(0) + request_count * 8 + if state_changed { 15 } else { 0 }).clamp(0, 100);
        let action_type = if value_first(action, &["role"]).contains("link") { "navigate" } else { "interact" };
        let protocol = serde_json::json!({
            "version":1,
            "preconditions":{"stateKey":state_key,"identities":identity_keys},
            "operation":{"type":action_type,"label":label,"role":value_first(action, &["role"]),"destructive":false},
            "observations":{"beforeUrl":value_first(action, &["beforeUrl"]),"afterUrl":value_first(action, &["afterUrl"]),"requestCount":request_count,"stateChanged":state_changed},
            "outcome":value_first(action, &["outcome"]),
            "stopRules":["confirmed_waf_or_challenge","rate_limit_detected","mutation_blocked"],
            "raw":action
        });
        connection.execute(
            "INSERT INTO investigation_actions(project_id,scan_id,target_url,action_key,state_key,action_type,label,outcome,value_score,protocol_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![project_id, scan_id, target_url, action_key, state_key, action_type, label, value_first(action, &["outcome"]), value_score, protocol.to_string()],
        ).map_err(|error| error.to_string())?;
        investigation_node(connection, project_id, scan_id, target_url, &action_key, "action", &label, "high", value_score, value_first(action, &["outcome"]).as_str(), &protocol)?;
        if !state_key.is_empty() {
            investigation_edge(connection, project_id, scan_id, target_url, &state_key, "offers_action", &action_key, "high", action)?;
        }
        let after_url = value_first(action, &["afterUrl"]);
        if !after_url.is_empty() {
            if let Some(next_state) = states.iter().find(|state| value_first(state, &["url"]) == after_url) {
                let next_id = value_first(next_state, &["id"]);
                if !next_id.is_empty() {
                    investigation_edge(connection, project_id, scan_id, target_url, &action_key, "transitions_to", &format!("state:{next_id}"), "medium", &serde_json::json!({"afterUrl":after_url}))?;
                }
            }
        }
    }

    let requested_mode_ceiling = requested_web_mode_ceiling(connection, scan_id);
    let source_guided_api_limit = match requested_mode_ceiling.as_str() {
        "deep" => 20,
        "standard" => 8,
        _ => 0,
    };
    let mut api_records: HashMap<String, JsonValue> = HashMap::new();
    for api in target.get("apis").and_then(JsonValue::as_array).into_iter().flatten() {
        if investigation_background_noise(api) { continue }
        let method = value_first(api, &["method"]).to_ascii_uppercase();
        let api_url = value_first(api, &["url", "path"]);
        if api_url.is_empty() { continue }
        let path = normalized_investigation_path(&api_url);
        let key = format!("api:{}", investigation_hash(&format!("{}|{}", if method.is_empty() { "UNKNOWN" } else { &method }, path)));
        api_records.insert(key, api.clone());
    }
    for api in target
        .get("apiCandidates")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter(|api| source_mapped_readonly_api(api))
        .take(source_guided_api_limit)
    {
        let method = value_first(api, &["method"]).to_ascii_uppercase();
        let api_url = value_first(api, &["url", "path"]);
        let path = normalized_investigation_path(&api_url);
        let key = format!(
            "api:{}",
            investigation_hash(&format!("{method}|{path}"))
        );
        api_records.entry(key).or_insert_with(|| api.clone());
    }
    for request in &runtime_requests {
        let resource_type = value_first(request, &["resourceType", "transport"]).to_ascii_lowercase();
        if !["xhr", "fetch", "eventsource", "websocket"].contains(&resource_type.as_str()) { continue }
        if investigation_background_noise(request) { continue }
        let method = value_first(request, &["method"]).to_ascii_uppercase();
        let api_url = value_first(request, &["url"]);
        if api_url.is_empty() { continue }
        let path = normalized_investigation_path(&api_url);
        let key = format!("api:{}", investigation_hash(&format!("{}|{}", if method.is_empty() { "GET" } else { &method }, path)));
        api_records.entry(key).and_modify(|current| {
            if let Some(object) = current.as_object_mut() {
                object.insert("runtimeObservation".into(), request.clone());
                object.insert("source".into(), JsonValue::String("browser-runtime".into()));
                object.insert("confidence".into(), JsonValue::String("high".into()));
            }
        }).or_insert_with(|| request.clone());
    }

    let identity_key = identity_keys.first().cloned().unwrap_or_else(|| "anonymous".into());
    let previous = if let Some(project_id) = project_id {
        connection.query_row(
            "SELECT api_signatures_json,parameter_signatures_json,signature FROM investigation_baselines WHERE project_id=?1 AND target_url=?2 AND identity_key=?3 AND source_scan_id<>?4 ORDER BY created_at DESC,id DESC LIMIT 1",
            params![project_id, target_url, identity_key, scan_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
        ).optional().map_err(|error| error.to_string())?
    } else { None };
    let previous_apis = previous.as_ref().map(|row| investigation_strings(Some(&investigation_json(row.0.clone()))).into_iter().collect::<HashSet<_>>()).unwrap_or_default();
    let previous_params = previous.as_ref().map(|row| investigation_strings(Some(&investigation_json(row.1.clone()))).into_iter().collect::<HashSet<_>>()).unwrap_or_default();

    let mut api_signatures = Vec::new();
    let mut parameter_signatures = Vec::new();
    let mut persisted_apis = Vec::new();
    let mut identity_api_signatures: HashMap<String, Vec<String>> = identity_keys
        .iter()
        .map(|identity| (identity.clone(), Vec::new()))
        .collect();
    let mut identity_parameter_signatures: HashMap<String, Vec<String>> = identity_keys
        .iter()
        .map(|identity| (identity.clone(), Vec::new()))
        .collect();
    for (api_key, api) in &api_records {
        let method = {
            let value = value_first(api, &["method"]).to_ascii_uppercase();
            if value.is_empty() { "GET".to_string() } else { value }
        };
        let api_url = value_first(api, &["url", "path"]);
        let path = normalized_investigation_path(&api_url);
        let signature = format!("{method}|{path}");
        api_signatures.push(signature.clone());
        let mut api_identity_keys = investigation_strings(api.get("identityKeys"));
        if api_identity_keys.is_empty() {
            let identity = value_first(api, &["identityKey"]);
            if !identity.is_empty() {
                api_identity_keys.push(identity);
            }
        }
        if api_identity_keys.is_empty() {
            api_identity_keys = identity_keys.clone();
        }
        let mut parameters = investigation_strings(api.get("parameters"));
        parameters.extend(investigation_strings(api.get("queryKeys")));
        parameters.extend(investigation_strings(api.get("bodyKeys")));
        parameters.extend(query_parameter_names(&api_url));
        if let Some(observation) = api.get("runtimeObservation") {
            parameters.extend(investigation_strings(observation.get("queryKeys")));
            parameters.extend(investigation_strings(observation.get("bodyKeys")));
        }
        parameters.sort(); parameters.dedup();
        parameter_signatures.extend(parameters.iter().map(|parameter| format!("{signature}|{parameter}")));
        for identity in &api_identity_keys {
            identity_api_signatures.entry(identity.clone()).or_default().push(signature.clone());
            identity_parameter_signatures.entry(identity.clone()).or_default().extend(
                parameters.iter().map(|parameter| format!("{signature}|{parameter}")),
            );
        }
        let baseline_status = if !previous_apis.contains(&signature) { "new" } else if parameters.iter().any(|parameter| !previous_params.contains(&format!("{signature}|{parameter}"))) { "changed" } else { "unchanged" };
        let source = value_first(api, &["source", "extractionEngine"]);
        let confidence = {
            let value = value_first(api, &["confidence"]);
            if value.is_empty() { if api.get("runtimeObservation").is_some() { "high".into() } else { "medium".into() } } else { value }
        };
        let state_id = value_first(api, &["stateId"]);
        let action_id = value_first(api, &["actionId"]);
        let state_keys = if state_id.is_empty() { Vec::new() } else { vec![format!("state:{state_id}")] };
        let action_keys = if action_id.is_empty() || action_id == "initial-load" || action_id == "navigation" { Vec::new() } else { vec![format!("action:{action_id}")] };
        let response_keys = sanitized_investigation_response_keys(api.get("responseKeys"));
        let request_schema = serde_json::json!({"parameters":parameters,"headers":investigation_strings(api.get("requestHeaderNames")),"contentType":value_first(api, &["requestContentType"])});
        let response_schema = serde_json::json!({"status":api.get("statusCode").or_else(|| api.get("status")).cloned().unwrap_or(JsonValue::Null),"contentType":value_first(api, &["contentType"]),"keys":response_keys});
        let auth_scope = if api_identity_keys.iter().all(|identity| identity == "anonymous") { "anonymous_or_unknown" } else if api_identity_keys.iter().any(|identity| identity == "anonymous") { "mixed_identity_observation" } else { "authenticated_observation" };
        connection.execute(
            "INSERT INTO investigation_api_models(project_id,scan_id,target_url,api_key,method,url,normalized_path,source,confidence,auth_scope,parameters_json,request_schema_json,response_schema_json,state_keys_json,action_keys_json,identity_keys_json,observed_count,baseline_status,payload_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,1,?17,?18)",
            params![project_id, scan_id, target_url, api_key, method, api_url, path, source, confidence, auth_scope, serde_json::to_string(&parameters).unwrap_or_else(|_| "[]".into()), request_schema.to_string(), response_schema.to_string(), serde_json::to_string(&state_keys).unwrap_or_else(|_| "[]".into()), serde_json::to_string(&action_keys).unwrap_or_else(|_| "[]".into()), serde_json::to_string(&api_identity_keys).unwrap_or_else(|_| "[]".into()), baseline_status, api.to_string()],
        ).map_err(|error| error.to_string())?;
        let value_score = (45 + if source.contains("runtime") { 25 } else { 0 } + (parameters.len() as i64 * 3).min(18)).min(100);
        investigation_node(connection, project_id, scan_id, target_url, api_key, "api", &format!("{method} {path}"), &confidence, value_score, baseline_status, api)?;
        investigation_edge(connection, project_id, scan_id, target_url, &root_key, "exposes_api", api_key, &confidence, &serde_json::json!({"source":source}))?;
        for state_key in &state_keys {
            investigation_edge(connection, project_id, scan_id, target_url, state_key, "issued_request", api_key, "high", &serde_json::json!({"source":"browser-runtime"}))?;
        }
        for action_key in &action_keys {
            investigation_edge(connection, project_id, scan_id, target_url, action_key, "triggered_request", api_key, "high", &serde_json::json!({"source":"browser-runtime"}))?;
        }
        for parameter in &parameters {
            let parameter_key = format!("parameter:{}", investigation_hash(&format!("{api_key}|{parameter}")));
            investigation_node(connection, project_id, scan_id, target_url, &parameter_key, "parameter", parameter, &confidence, 50, baseline_status, &serde_json::json!({"name":parameter,"apiKey":api_key,"source":source}))?;
            investigation_edge(connection, project_id, scan_id, target_url, api_key, "has_parameter", &parameter_key, &confidence, &serde_json::json!({"name":parameter}))?;
        }
        for identity in &api_identity_keys {
            investigation_edge(connection, project_id, scan_id, target_url, api_key, "observed_with_identity", &format!("identity:{}", investigation_hash(identity)), "high", &serde_json::json!({"identityKey":identity}))?;
        }
        persisted_apis.push((api_key.clone(), api.clone()));
    }
    api_signatures.sort(); api_signatures.dedup();
    parameter_signatures.sort(); parameter_signatures.dedup();

    let opportunities = target.get("opportunities").and_then(JsonValue::as_array).cloned().unwrap_or_default();
    // Keep low-value transport/telemetry in raw runtime evidence, but never
    // turn it into a graph hypothesis or a manual validation queue item.
    let actionable_opportunities = deduplicated_actionable_opportunities(&opportunities);
    // A retry may ingest the same endpoint with a fresh nonce/timestamp. Drop
    // only unstarted queue rows before rebuilding stable contracts; terminal
    // decisions and explicit authorization states remain audit history.
    connection.execute(
        "DELETE FROM investigation_edges WHERE scan_id=?1 AND target_url=?2 AND (source_key IN (SELECT hypothesis_key FROM investigation_hypotheses WHERE scan_id=?1 AND target_url=?2 AND status IN ('candidate','ready','needs_more_evidence')) OR target_key IN (SELECT hypothesis_key FROM investigation_hypotheses WHERE scan_id=?1 AND target_url=?2 AND status IN ('candidate','ready','needs_more_evidence')))",
        params![scan_id, target_url],
    ).map_err(|error| error.to_string())?;
    connection.execute(
        "DELETE FROM investigation_nodes WHERE scan_id=?1 AND target_url=?2 AND node_key IN (SELECT hypothesis_key FROM investigation_hypotheses WHERE scan_id=?1 AND target_url=?2 AND status IN ('candidate','ready','needs_more_evidence'))",
        params![scan_id, target_url],
    ).map_err(|error| error.to_string())?;
    connection.execute(
        "DELETE FROM investigation_hypotheses WHERE scan_id=?1 AND target_url=?2 AND status IN ('candidate','ready','needs_more_evidence')",
        params![scan_id, target_url],
    ).map_err(|error| error.to_string())?;
    let mut hypothesis_records = Vec::new();
    for opportunity in &actionable_opportunities {
        let category = value_first(opportunity, &["category"]);
        let title = value_first(opportunity, &["title"]);
        let source_key = stable_opportunity_key(opportunity);
        let hypothesis_key = format!("hypothesis:{}", investigation_hash(&source_key));
        let score = opportunity.get("score").and_then(JsonValue::as_i64).unwrap_or(0).clamp(0, 100);
        let confidence = value_first(opportunity, &["confidence"]);
        let evidence = opportunity.get("evidenceRefs").cloned().unwrap_or_else(|| serde_json::json!([]));
        let endpoint = value_first(opportunity, &["endpoint", "url", "path"]);
        let (ready, readiness_reason) = opportunity_agent_readiness(opportunity);
        let status = if ready { "ready" } else { "candidate" };
        let contract = verification_contract(&category, opportunity);
        let decision = serde_json::json!({
            "eligibleForModel":ready,
            "reason":readiness_reason,
            "requiresHuman":false,
            "authorizationMode":"automatic_bounded",
            "verificationMode":if ready { "ai_auto" } else { "needs_evidence" },
            "humanReviewStage":if ready { "final_verdict_only" } else { "evidence_collection" },
            "suspiciousOnlyEscalation":true,
        });
        connection.execute(
            "INSERT INTO investigation_hypotheses(project_id,scan_id,target_url,hypothesis_key,category,title,status,score,confidence,contract_json,evidence_json,decision_json,source_opportunity_key) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13) ON CONFLICT(scan_id,target_url,hypothesis_key) DO UPDATE SET project_id=excluded.project_id,category=excluded.category,title=excluded.title,status=CASE WHEN investigation_hypotheses.status IN ('in_progress','validated','rejected','exhausted') THEN investigation_hypotheses.status ELSE excluded.status END,score=excluded.score,confidence=excluded.confidence,contract_json=excluded.contract_json,evidence_json=excluded.evidence_json,decision_json=excluded.decision_json,source_opportunity_key=excluded.source_opportunity_key,updated_at=datetime('now','localtime')",
            params![project_id, scan_id, target_url, hypothesis_key, category, title, status, score, confidence, contract.to_string(), evidence.to_string(), decision.to_string(), source_key],
        ).map_err(|error| error.to_string())?;
        investigation_node(connection, project_id, scan_id, target_url, &hypothesis_key, "hypothesis", &title, &confidence, score, status, opportunity)?;
        investigation_edge(connection, project_id, scan_id, target_url, &root_key, "raises_hypothesis", &hypothesis_key, &confidence, &evidence)?;
        if !endpoint.is_empty() {
            let path = normalized_investigation_path(&endpoint);
            if let Some((api_key, _)) = persisted_apis.iter().find(|(_, api)| normalized_investigation_path(&value_first(api, &["url", "path"])) == path) {
                investigation_edge(connection, project_id, scan_id, target_url, api_key, "supports_hypothesis", &hypothesis_key, &confidence, opportunity)?;
            }
        }
        if ready {
            hypothesis_records.push((hypothesis_key, category));
        }
    }

    let current_api_set = api_signatures.iter().cloned().collect::<HashSet<_>>();
    let current_param_set = parameter_signatures.iter().cloned().collect::<HashSet<_>>();
    let added_api_count = current_api_set.difference(&previous_apis).count() as i64;
    let removed_api_count = previous_apis.difference(&current_api_set).count() as i64;
    let added_parameter_count = current_param_set.difference(&previous_params).count() as i64;
    let removed_parameter_count = previous_params.difference(&current_param_set).count() as i64;
    let ready_hypothesis_count = connection.query_row(
        "SELECT COUNT(*) FROM investigation_hypotheses WHERE scan_id=?1 AND target_url=?2 AND status IN ('ready','in_progress')",
        params![scan_id, target_url],
        |row| row.get::<_, i64>(0),
    ).map_err(|error| error.to_string())?;
    let duplicate_count = coverage.get("deduplicatedStateCount").and_then(JsonValue::as_i64).unwrap_or(0) + coverage.get("lowValueStateSkipped").and_then(JsonValue::as_i64).unwrap_or(0);
    let has_baseline = previous.is_some();
    let information_gain = if has_baseline {
        (added_api_count * 12 + added_parameter_count * 6 + ready_hypothesis_count * 8 + actions.len() as i64 * 2 - duplicate_count.min(20)).clamp(0, 100)
    } else {
        ((states.len() as i64 * 3) + (actions.len() as i64 * 3) + (api_signatures.len() as i64 * 5) + (parameter_signatures.len() as i64 * 2) + ready_hypothesis_count * 8).clamp(0, 100)
    };
    let runtime_stop_reason = value_first(exploration, &["stopReason"]);
    let waf_detected = target.get("authSessionValidation").and_then(|value| value.get("wafDetected")).and_then(JsonValue::as_bool).unwrap_or(false) || runtime_stop_reason == "confirmed_waf_or_challenge";
    let token_worthy = !waf_detected && ready_hypothesis_count > 0 && (!has_baseline || added_api_count > 0 || added_parameter_count > 0 || information_gain >= 35);
    let verified_runtime_api_count = persisted_apis
        .iter()
        .filter(|(_, api)| standard_investigation_api(api))
        .count() as i64;
    let source_mapped_readonly_api_count = persisted_apis
        .iter()
        .filter(|(_, api)| source_mapped_readonly_api(api))
        .count() as i64;
    let source_guided_investigation_allowed = !waf_detected
        && !token_worthy
        && requested_mode_ceiling != "quick"
        && verified_runtime_api_count == 0
        && source_mapped_readonly_api_count > 0
        && information_gain >= 30;
    // A real browser request contract is enough for one bounded standard
    // investigation. It is not promoted to a vulnerability hypothesis and it
    // does not weaken the stricter evidence gate used for deep validation.
    let standard_investigation_allowed = source_guided_investigation_allowed || (!waf_detected
        && !token_worthy
        && verified_runtime_api_count > 0
        && (information_gain >= 20
            || verified_runtime_api_count >= 2
            || !actions.is_empty()
            || identity_keys.len() >= 2));
    let stop_reason = if waf_detected {
        "confirmed_waf_or_challenge"
    } else if source_guided_investigation_allowed {
        "source_mapped_readonly_contracts"
    } else if !runtime_requests.is_empty() && api_signatures.is_empty() {
        "request_evidence_present_no_api_contract"
    } else if has_baseline && added_api_count == 0 && added_parameter_count == 0 {
        "incremental_no_new_value"
    } else if ready_hypothesis_count == 0 && information_gain < 25 {
        "no_high_value_hypothesis"
    } else if runtime_stop_reason.is_empty() {
        "evidence_collection_complete"
    } else {
        runtime_stop_reason.as_str()
    }.to_string();
    let node_count = connection.query_row("SELECT COUNT(*) FROM investigation_nodes WHERE scan_id=?1 AND target_url=?2", params![scan_id,target_url], |row| row.get::<_,i64>(0)).map_err(|error| error.to_string())?;
    let edge_count = connection.query_row("SELECT COUNT(*) FROM investigation_edges WHERE scan_id=?1 AND target_url=?2", params![scan_id,target_url], |row| row.get::<_,i64>(0)).map_err(|error| error.to_string())?;
    let automation_tier = if token_worthy {
        "evidence_deep_validation"
    } else if standard_investigation_allowed {
        "runtime_standard_investigation"
    } else {
        "recon_only"
    };
    let manual_deep_dive = manual_deep_dive_plan(
        target,
        &persisted_apis,
        &actions,
        &identity_keys,
        &requested_mode_ceiling,
    );
    let decision = serde_json::json!({
        "schemaVersion":3,"eligibleForModel":token_worthy,
        "standardInvestigationAllowed":standard_investigation_allowed,
        "automationTier":automation_tier,"informationGain":information_gain,
        "baseline":{"available":has_baseline,"addedApis":added_api_count,"removedApis":removed_api_count,"addedParameters":added_parameter_count,"removedParameters":removed_parameter_count},
        "coverage":coverage,"readyHypotheses":ready_hypothesis_count,"identityCount":identity_keys.len(),
        "observedRequestCount":runtime_requests.len(),
        "verifiedRuntimeApiCount":verified_runtime_api_count,
        "sourceMappedReadOnlyApiCount":source_mapped_readonly_api_count,
        "sourceGuidedInvestigationAllowed":source_guided_investigation_allowed,
        "requestedModeCeiling":requested_mode_ceiling,
        "apiEvidenceSource": if exploration.get("authSessionFallbackUsed").and_then(JsonValue::as_bool).unwrap_or(false) { "auth-session-fallback" } else if exploration.get("runtimeProbeAvailable").and_then(JsonValue::as_bool).unwrap_or(false) { "browser-runtime" } else { "deterministic-or-static" },
        "runtimeProbeAvailable":exploration.get("runtimeProbeAvailable").and_then(JsonValue::as_bool).unwrap_or(false),
        "authSessionCaptureAvailable":exploration.get("authSessionCapture").and_then(|value| value.get("available")).and_then(JsonValue::as_bool).unwrap_or(false),
        "manualDeepDive":manual_deep_dive,
        "coverageSemantics":{"completed":"listed contract executed with usable evidence","notFound":"executed without security impact","notTested":"missing identity, state, data, protocol or environment","neverAssumeSafe":true},
        "stopReason":stop_reason,"rules":{"wafStopsImmediately":true,"authorizationStatusDoesNotInvalidateSession":true,"minimumHypothesisScore":70,"unresolvedStaticCandidatesNeverOpenStandardGate":true,"sourceMappedReadOnlyContractsMayOpenBoundedGate":true}
    });
    connection.execute(
        "INSERT INTO investigation_metrics(scan_id,target_url,project_id,node_count,edge_count,state_count,action_count,api_count,parameter_count,hypothesis_count,added_count,changed_count,removed_count,duplicate_count,information_gain,token_worthy,stop_reason,decision_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18) ON CONFLICT(scan_id,target_url) DO UPDATE SET project_id=excluded.project_id,node_count=excluded.node_count,edge_count=excluded.edge_count,state_count=excluded.state_count,action_count=excluded.action_count,api_count=excluded.api_count,parameter_count=excluded.parameter_count,hypothesis_count=excluded.hypothesis_count,added_count=excluded.added_count,changed_count=excluded.changed_count,removed_count=excluded.removed_count,duplicate_count=excluded.duplicate_count,information_gain=excluded.information_gain,token_worthy=excluded.token_worthy,stop_reason=excluded.stop_reason,decision_json=excluded.decision_json,updated_at=datetime('now','localtime')",
        params![scan_id,target_url,project_id,node_count,edge_count,states.len() as i64,actions.len() as i64,api_signatures.len() as i64,parameter_signatures.len() as i64,actionable_opportunities.len() as i64,added_api_count+added_parameter_count,added_parameter_count+removed_parameter_count,removed_api_count+removed_parameter_count,duplicate_count,information_gain,token_worthy as i64,stop_reason,decision.to_string()],
    ).map_err(|error| error.to_string())?;
    if let Some(project_id) = project_id {
        let signature = investigation_hash(&format!("{}|{}", api_signatures.join("\n"), parameter_signatures.join("\n")));
        for identity in &identity_keys {
            let mut identity_apis = identity_api_signatures.remove(identity).unwrap_or_default();
            let mut identity_parameters = identity_parameter_signatures.remove(identity).unwrap_or_default();
            identity_apis.sort(); identity_apis.dedup();
            identity_parameters.sort(); identity_parameters.dedup();
            let identity_signature = if identity_keys.len() == 1 { signature.clone() } else { investigation_hash(&format!("{}|{}", identity_apis.join("\n"), identity_parameters.join("\n"))) };
            connection.execute(
                "INSERT INTO investigation_baselines(project_id,target_url,identity_key,source_scan_id,signature,api_signatures_json,parameter_signatures_json,metrics_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(project_id,target_url,identity_key,source_scan_id) DO UPDATE SET signature=excluded.signature,api_signatures_json=excluded.api_signatures_json,parameter_signatures_json=excluded.parameter_signatures_json,metrics_json=excluded.metrics_json",
                params![project_id,target_url,identity,scan_id,identity_signature,serde_json::to_string(&identity_apis).unwrap_or_else(|_| "[]".into()),serde_json::to_string(&identity_parameters).unwrap_or_else(|_| "[]".into()),decision.to_string()],
            ).map_err(|error| error.to_string())?;
        }
    }
    if let Some(comparisons) = target.get("identityComparisons").and_then(JsonValue::as_array) {
        for comparison in comparisons {
            let api_key = value_first(comparison,&["apiKey"]);
            if value_first(comparison,&["differenceType"]) == "feature_surface" || api_key.to_ascii_lowercase().starts_with("feature:") { continue }
            let endpoint = api_key.split('|').find(|part| part.starts_with('/')).unwrap_or(&api_key);
            if investigation_background_noise(&serde_json::json!({"url":endpoint,"method":api_key.split('|').next().unwrap_or("GET")})) { continue }
            let mut matrix = comparison.get("matrix").cloned().unwrap_or_else(|| serde_json::json!({}));
            sanitize_identity_matrix(&mut matrix);
            let identities = matrix.as_object().map(|value| value.keys().cloned().collect::<Vec<_>>()).unwrap_or_default();
            if identities.len() < 2 { continue }
            connection.execute(
                "INSERT OR REPLACE INTO investigation_identity_diffs(project_id,scan_id,target_url,api_key,left_identity_key,right_identity_key,difference_type,risk_score,status,matrix_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'observed',?9)",
                params![project_id,scan_id,target_url,api_key,identities[0],identities[1],value_first(comparison,&["differenceType"]),comparison.get("riskScore").and_then(JsonValue::as_i64).unwrap_or(0),matrix.to_string()],
            ).map_err(|error| error.to_string())?;
        }
    }
    persist_identity_differences(connection, project_id, scan_id, target_url, &identity_keys)?;
    persist_knowledge_layers(connection, project_id, scan_id, target_url, target, &persisted_apis, &hypothesis_records, &stop_reason)?;
    insert_finding(connection, scan_id, target_url, "investigation", "investigation_decision", "information-gain", "调查决策", if token_worthy { "medium" } else { "info" }, &decision)?;
    connection.execute(
        "UPDATE sentinel_targets SET value_score=MAX(value_score,?1),scan_mode=CASE WHEN ?2=1 AND scan_mode<>'deep' THEN 'evidence_guided' WHEN ?3=1 AND scan_mode<>'deep' THEN 'standard' ELSE scan_mode END,routing_reason=?4,updated_at=datetime('now','localtime') WHERE scan_id=?5 AND url=?6",
        params![information_gain,token_worthy as i64,standard_investigation_allowed as i64,if token_worthy { format!("调查图谱存在风险证据，进入证据后深挖；信息增益 {information_gain}/100") } else if source_guided_investigation_allowed { format!("已从源码映射还原 {source_mapped_readonly_api_count} 个高置信度只读接口，进入有界目标调查；信息增益 {information_gain}/100") } else if standard_investigation_allowed { format!("已采集 {verified_runtime_api_count} 个真实运行时接口，进入一次有界标准调查；信息增益 {information_gain}/100") } else { format!("本地调查停止：{stop_reason}；信息增益 {information_gain}/100") },scan_id,target_url],
    ).map_err(|error| error.to_string())?;

    Ok(InvestigationMetrics {
        scan_id: scan_id.into(), target_url: target_url.into(), node_count, edge_count,
        state_count: states.len() as i64, action_count: actions.len() as i64,
        api_count: api_signatures.len() as i64, parameter_count: parameter_signatures.len() as i64,
        hypothesis_count: actionable_opportunities.len() as i64, added_count: added_api_count + added_parameter_count,
        changed_count: added_parameter_count + removed_parameter_count, removed_count: removed_api_count + removed_parameter_count,
        duplicate_count, information_gain, token_worthy, stop_reason, decision,
        updated_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    })
}

fn read_investigation_nodes(connection: &rusqlite::Connection, scan_id: &str, target_url: &str) -> Result<Vec<InvestigationNode>, String> {
    let mut statement = connection.prepare("SELECT id,scan_id,target_url,node_key,node_type,label,confidence,value_score,status,payload_json,first_seen,last_seen FROM investigation_nodes WHERE scan_id=?1 AND (?2='' OR target_url=?2) ORDER BY CASE node_type WHEN 'target' THEN 0 WHEN 'identity' THEN 1 WHEN 'page_state' THEN 2 WHEN 'action' THEN 3 WHEN 'api' THEN 4 WHEN 'parameter' THEN 5 ELSE 6 END,value_score DESC,id").map_err(|error| error.to_string())?;
    let rows = statement.query_map(params![scan_id,target_url], |row| Ok(InvestigationNode { id:row.get(0)?,scan_id:row.get(1)?,target_url:row.get(2)?,node_key:row.get(3)?,node_type:row.get(4)?,label:row.get(5)?,confidence:row.get(6)?,value_score:row.get(7)?,status:row.get(8)?,payload:investigation_json(row.get(9)?),first_seen:row.get(10)?,last_seen:row.get(11)? })).map_err(|error| error.to_string())?;
    let mut nodes = rows.collect::<Result<Vec<_>,_>>().map_err(|error| error.to_string())?;
    drop(statement);
    // `anonymous` is an API observation scope, not a browser login identity.
    // Older graphs persisted it as an identity node and the UI consequently
    // rendered a fictitious "账号 A / 会话已失效" on public scans.
    nodes.retain(|node| {
        node.node_type != "identity"
            || !anonymous_identity(&value_first(&node.payload, &["identityKey"]))
    });
    // Repair pre-1.1.21 graphs at read time. Their checkpoint already contains
    // the complete identityRuns data, but the old graph node persisted only an
    // identityKey and therefore rendered every account as unknown.
    let checkpoint = connection.query_row(
        "SELECT raw_json FROM sentinel_checkpoints WHERE scan_id=?1 AND url=?2 AND stage='frontend_recon'",
        params![scan_id, target_url],
        |row| row.get::<_, String>(0),
    ).optional().map_err(|error| error.to_string())?
        .and_then(|raw| serde_json::from_str::<JsonValue>(&raw).ok());
    if let Some(target) = checkpoint {
        let mut identity_index = 0usize;
        for node in nodes.iter_mut().filter(|node| node.node_type == "identity") {
            let identity_key = value_first(&node.payload, &["identityKey"]);
            if identity_key.is_empty() || identity_run_summary(&target, &identity_key).is_none() {
                identity_index += 1;
                continue;
            }
            let payload = identity_node_payload(&target, &identity_key, identity_index);
            node.label = value_first(&payload, &["identityLabel"]);
            node.status = match payload.get("sessionValid").and_then(JsonValue::as_bool) {
                Some(true) => "active".into(),
                Some(false) => "invalid".into(),
                None => "unknown".into(),
            };
            node.payload = payload;
            identity_index += 1;
        }
    }
    Ok(nodes)
}

fn read_investigation_edges(connection: &rusqlite::Connection, scan_id: &str, target_url: &str) -> Result<Vec<InvestigationEdge>, String> {
    let mut statement = connection.prepare("SELECT id,scan_id,target_url,source_key,relation,target_key,confidence,evidence_json,created_at FROM investigation_edges WHERE scan_id=?1 AND (?2='' OR target_url=?2) ORDER BY id").map_err(|error| error.to_string())?;
    let rows = statement.query_map(params![scan_id,target_url], |row| Ok(InvestigationEdge { id:row.get(0)?,scan_id:row.get(1)?,target_url:row.get(2)?,source_key:row.get(3)?,relation:row.get(4)?,target_key:row.get(5)?,confidence:row.get(6)?,evidence:investigation_json(row.get(7)?),created_at:row.get(8)? })).map_err(|error| error.to_string())?;
    let anonymous_key = format!("identity:{}", investigation_hash("anonymous"));
    let mut values = rows.collect::<Result<Vec<_>,_>>().map_err(|error| error.to_string())?;
    values.retain(|edge| edge.source_key != anonymous_key && edge.target_key != anonymous_key);
    Ok(values)
}

fn read_investigation_actions(connection: &rusqlite::Connection, scan_id: &str, target_url: &str) -> Result<Vec<InvestigationAction>, String> {
    let mut statement = connection.prepare("SELECT id,scan_id,target_url,action_key,state_key,action_type,label,outcome,value_score,protocol_json,created_at,updated_at FROM investigation_actions WHERE scan_id=?1 AND (?2='' OR target_url=?2) ORDER BY value_score DESC,id").map_err(|error| error.to_string())?;
    let rows = statement.query_map(params![scan_id,target_url], |row| Ok(InvestigationAction { id:row.get(0)?,scan_id:row.get(1)?,target_url:row.get(2)?,action_key:row.get(3)?,state_key:row.get(4)?,action_type:row.get(5)?,label:row.get(6)?,outcome:row.get(7)?,value_score:row.get(8)?,protocol:investigation_json(row.get(9)?),created_at:row.get(10)?,updated_at:row.get(11)? })).map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>,_>>().map_err(|error| error.to_string())
}

fn read_investigation_apis(connection: &rusqlite::Connection, scan_id: &str, target_url: &str) -> Result<Vec<InvestigationApiModel>, String> {
    let mut statement = connection.prepare("SELECT id,scan_id,target_url,api_key,method,url,normalized_path,source,confidence,auth_scope,parameters_json,request_schema_json,response_schema_json,state_keys_json,action_keys_json,identity_keys_json,observed_count,baseline_status,payload_json,updated_at FROM investigation_api_models WHERE scan_id=?1 AND (?2='' OR target_url=?2) ORDER BY CASE baseline_status WHEN 'new' THEN 0 WHEN 'changed' THEN 1 ELSE 2 END,method,normalized_path").map_err(|error| error.to_string())?;
    let rows = statement.query_map(params![scan_id,target_url], |row| Ok(InvestigationApiModel { id:row.get(0)?,scan_id:row.get(1)?,target_url:row.get(2)?,api_key:row.get(3)?,method:row.get(4)?,url:row.get(5)?,normalized_path:row.get(6)?,source:row.get(7)?,confidence:row.get(8)?,auth_scope:row.get(9)?,parameters:investigation_json(row.get(10)?),request_schema:investigation_json(row.get(11)?),response_schema:investigation_json(row.get(12)?),state_keys:investigation_json(row.get(13)?),action_keys:investigation_json(row.get(14)?),identity_keys:investigation_json(row.get(15)?),observed_count:row.get(16)?,baseline_status:row.get(17)?,payload:investigation_json(row.get(18)?),updated_at:row.get(19)? })).map_err(|error| error.to_string())?;
    let mut values = rows.collect::<Result<Vec<_>,_>>().map_err(|error| error.to_string())?;
    values.retain(|api| !investigation_background_noise(&serde_json::json!({
        "url":api.url,"method":api.method,"source":api.source,"contentType":api.payload.get("contentType"),"resourceType":api.payload.get("resourceType")
    })));
    for api in &mut values {
        let keys = api.response_schema.get("keys").cloned().unwrap_or_else(|| serde_json::json!([]));
        if let Some(schema) = api.response_schema.as_object_mut() {
            schema.insert("keys".into(), serde_json::json!(sanitized_investigation_response_keys(Some(&keys))));
        }
        if let Some(payload) = api.payload.as_object_mut() {
            let keys = payload.get("responseKeys").cloned().unwrap_or_else(|| serde_json::json!([]));
            payload.insert("responseKeys".into(), serde_json::json!(sanitized_investigation_response_keys(Some(&keys))));
            if let Some(observations) = payload.get_mut("identityObservations").and_then(JsonValue::as_array_mut) {
                for observation in observations {
                    if let Some(object) = observation.as_object_mut() {
                        let keys = object.get("responseKeys").cloned().unwrap_or_else(|| serde_json::json!([]));
                        object.insert("responseKeys".into(), serde_json::json!(sanitized_investigation_response_keys(Some(&keys))));
                    }
                }
            }
        }
    }
    Ok(values)
}

fn read_investigation_hypotheses(connection: &rusqlite::Connection, scan_id: &str, target_url: &str, status: &str) -> Result<Vec<InvestigationHypothesis>, String> {
    let mut statement = connection.prepare("SELECT h.id,h.project_id,h.scan_id,h.target_url,h.hypothesis_key,h.category,h.title,h.status,h.score,h.confidence,h.contract_json,h.evidence_json,h.decision_json,h.source_opportunity_key,h.created_at,h.updated_at,COALESCE(a.approved,0),COALESCE(a.scope_json,'{}'),COALESCE(a.max_attempts,1),COALESCE(a.note,''),COALESCE(a.expires_at,''),COALESCE(a.updated_at,''),CASE WHEN COALESCE(a.approved,0)=1 AND datetime(a.expires_at)>datetime('now','localtime') THEN 1 ELSE 0 END FROM investigation_hypotheses h LEFT JOIN investigation_mutation_approvals a ON a.hypothesis_id=h.id WHERE (?1='' OR h.scan_id=?1) AND (?2='' OR h.target_url=?2) AND (?3='' OR h.status=?3) ORDER BY h.score DESC,h.updated_at DESC").map_err(|error| error.to_string())?;
    let rows = statement.query_map(params![scan_id,target_url,status], |row| {
        let scope = investigation_json(row.get(17)?);
        Ok(InvestigationHypothesis {
            id:row.get(0)?,project_id:row.get(1)?,scan_id:row.get(2)?,target_url:row.get(3)?,hypothesis_key:row.get(4)?,category:row.get(5)?,title:row.get(6)?,status:row.get(7)?,score:row.get(8)?,confidence:row.get(9)?,contract:investigation_json(row.get(10)?),evidence:investigation_json(row.get(11)?),decision:investigation_json(row.get(12)?),source_opportunity_key:row.get(13)?,created_at:row.get(14)?,updated_at:row.get(15)?,
            mutation_approval: serde_json::json!({
                "approved":row.get::<_,i64>(16)?!=0,
                "active":row.get::<_,i64>(22)?!=0,
                "scope":scope,
                "maxAttempts":row.get::<_,i64>(18)?,
                "note":row.get::<_,String>(19)?,
                "expiresAt":row.get::<_,String>(20)?,
                "updatedAt":row.get::<_,String>(21)?,
            }),
        })
    }).map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>,_>>().map_err(|error| error.to_string())
}

fn read_investigation_identity_diffs(connection: &rusqlite::Connection, scan_id: &str, target_url: &str) -> Result<Vec<InvestigationIdentityDiff>, String> {
    let mut statement = connection.prepare("SELECT id,scan_id,target_url,api_key,left_identity_key,right_identity_key,difference_type,risk_score,status,matrix_json,created_at FROM investigation_identity_diffs WHERE scan_id=?1 AND (?2='' OR target_url=?2) ORDER BY risk_score DESC,id").map_err(|error| error.to_string())?;
    let rows = statement.query_map(params![scan_id,target_url], |row| Ok(InvestigationIdentityDiff { id:row.get(0)?,scan_id:row.get(1)?,target_url:row.get(2)?,api_key:row.get(3)?,left_identity_key:row.get(4)?,right_identity_key:row.get(5)?,difference_type:row.get(6)?,risk_score:row.get(7)?,status:row.get(8)?,matrix:investigation_json(row.get(9)?),created_at:row.get(10)? })).map_err(|error| error.to_string())?;
    let mut values = rows.collect::<Result<Vec<_>,_>>().map_err(|error| error.to_string())?;
    values.retain(|diff| {
        if diff.difference_type == "feature_surface" || diff.api_key.to_ascii_lowercase().starts_with("feature:") {
            return false;
        }
        let endpoint = diff.api_key.split('|').find(|part| part.starts_with('/')).unwrap_or(&diff.api_key);
        !investigation_background_noise(&serde_json::json!({"url":endpoint,"method":diff.api_key.split('|').next().unwrap_or("GET")}))
    });
    for diff in &mut values {
        sanitize_identity_matrix(&mut diff.matrix);
    }
    Ok(values)
}

fn read_investigation_metrics(connection: &rusqlite::Connection, scan_id: &str, target_url: &str) -> Result<Option<InvestigationMetrics>, String> {
    connection.query_row("SELECT scan_id,target_url,node_count,edge_count,state_count,action_count,api_count,parameter_count,hypothesis_count,added_count,changed_count,removed_count,duplicate_count,information_gain,token_worthy,stop_reason,decision_json,updated_at FROM investigation_metrics WHERE scan_id=?1 AND target_url=?2", params![scan_id,target_url], |row| Ok(InvestigationMetrics { scan_id:row.get(0)?,target_url:row.get(1)?,node_count:row.get(2)?,edge_count:row.get(3)?,state_count:row.get(4)?,action_count:row.get(5)?,api_count:row.get(6)?,parameter_count:row.get(7)?,hypothesis_count:row.get(8)?,added_count:row.get(9)?,changed_count:row.get(10)?,removed_count:row.get(11)?,duplicate_count:row.get(12)?,information_gain:row.get(13)?,token_worthy:row.get::<_,i64>(14)?!=0,stop_reason:row.get(15)?,decision:investigation_json(row.get(16)?),updated_at:row.get(17)? })).optional().map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_investigation_graph(state: State<AppState>, scan_id: String, target_url: Option<String>) -> Result<InvestigationGraph, String> {
    let connection = db::open(&state.db_path)?;
    let target_url = target_url.unwrap_or_default();
    let apis = read_investigation_apis(&connection,&scan_id,&target_url)?;
    let actions = read_investigation_actions(&connection,&scan_id,&target_url)?;
    let nodes = read_investigation_nodes(&connection,&scan_id,&target_url)?;
    let identity_diffs = read_investigation_identity_diffs(&connection,&scan_id,&target_url)?;
    let mut metrics = if target_url.is_empty() { None } else { read_investigation_metrics(&connection, &scan_id, &target_url)? };
    if let Some(value) = &mut metrics {
        // Historical rows retain their raw audit records. Keep the visible KPI
        // aligned with the sanitized formal API list returned by this read.
        value.api_count = apis.len() as i64;
        if value.decision.get("manualDeepDive").and_then(JsonValue::as_array).is_none() {
            let persisted_apis = apis.iter().map(|api| (
                api.api_key.clone(),
                serde_json::json!({
                    "method":api.method,"url":api.url,"path":api.normalized_path,
                    "parameters":api.parameters,
                    "responseKeys":api.response_schema.get("keys").cloned().unwrap_or_default()
                }),
            )).collect::<Vec<_>>();
            let raw_actions = actions.iter().map(|action| serde_json::json!({
                "label":action.label,"type":action.action_type,"outcome":action.outcome
            })).collect::<Vec<_>>();
            let mut identity_keys = nodes.iter()
                .filter(|node| node.node_type == "identity")
                .filter_map(|node| node.payload.get("identityKey").and_then(JsonValue::as_str))
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            if identity_keys.is_empty() { identity_keys.push("anonymous".into()); }
            let mode = requested_web_mode_ceiling(&connection, &scan_id);
            let manual = manual_deep_dive_plan(
                &JsonValue::Null,
                &persisted_apis,
                &raw_actions,
                &identity_keys,
                &mode,
            );
            if let Some(decision) = value.decision.as_object_mut() {
                decision.insert("manualDeepDive".into(), manual);
                decision.insert("coverageSemantics".into(), serde_json::json!({
                    "completed":"listed contract executed with usable evidence",
                    "notFound":"executed without security impact",
                    "notTested":"missing identity, state, data, protocol or environment",
                    "neverAssumeSafe":true
                }));
            }
        }
    }
    let related_services = read_investigation_related_services(&connection, &scan_id, &target_url)?;
    Ok(InvestigationGraph { scan_id:scan_id.clone(),target_url:target_url.clone(),nodes,edges:read_investigation_edges(&connection,&scan_id,&target_url)?,actions,apis,related_services,hypotheses:read_investigation_hypotheses(&connection,&scan_id,&target_url,"")?,identity_diffs,metrics })
}

#[tauri::command]
pub fn list_investigation_hypotheses(state: State<AppState>, scan_id: Option<String>, status: Option<String>) -> Result<Vec<InvestigationHypothesis>, String> {
    let connection = db::open(&state.db_path)?;
    read_investigation_hypotheses(&connection, scan_id.as_deref().unwrap_or(""), "", status.as_deref().unwrap_or(""))
}

#[tauri::command]
pub fn update_investigation_hypothesis(state: State<AppState>, input: InvestigationHypothesisUpdateInput) -> Result<(), String> {
    let allowed = ["candidate","ready","in_progress","validated","rejected","exhausted"];
    if !allowed.contains(&input.status.as_str()) { return Err("不支持的假设状态".into()) }
    let connection = db::open(&state.db_path)?;
    let changed = connection.execute("UPDATE investigation_hypotheses SET status=?1,updated_at=datetime('now','localtime') WHERE id=?2", params![input.status,input.hypothesis_id]).map_err(|error| error.to_string())?;
    if changed == 0 { return Err("调查假设不存在".into()) }
    connection.execute("UPDATE knowledge_outcomes SET outcome=?1 WHERE hypothesis_key=(SELECT hypothesis_key FROM investigation_hypotheses WHERE id=?2) AND scan_id=(SELECT scan_id FROM investigation_hypotheses WHERE id=?2)", params![input.status,input.hypothesis_id]).map_err(|error| error.to_string())?;
    let strategies = {
        let mut statement = connection.prepare("SELECT DISTINCT project_id,strategy_key FROM knowledge_outcomes WHERE hypothesis_key=(SELECT hypothesis_key FROM investigation_hypotheses WHERE id=?1) AND scan_id=(SELECT scan_id FROM investigation_hypotheses WHERE id=?1)").map_err(|error| error.to_string())?;
        let rows = statement.query_map([input.hypothesis_id], |row| Ok((row.get::<_,Option<i64>>(0)?,row.get::<_,String>(1)?))).map_err(|error| error.to_string())?;
        rows.flatten().collect::<Vec<_>>()
    };
    for (project_id, strategy_key) in strategies {
        let Some(project_id) = project_id else { continue };
        connection.execute(
            "UPDATE knowledge_strategies SET support_count=(SELECT COUNT(DISTINCT scan_id||'|'||target_url) FROM knowledge_outcomes WHERE project_id=?1 AND strategy_key=?2),success_count=(SELECT COUNT(*) FROM knowledge_outcomes WHERE project_id=?1 AND strategy_key=?2 AND outcome IN ('validated','confirmed')),failure_count=(SELECT COUNT(*) FROM knowledge_outcomes WHERE project_id=?1 AND strategy_key=?2 AND outcome IN ('rejected','failed','exhausted')),promoted=CASE WHEN (SELECT COUNT(DISTINCT scan_id||'|'||target_url) FROM knowledge_outcomes WHERE project_id=?1 AND strategy_key=?2)>=2 OR (SELECT COUNT(*) FROM knowledge_outcomes WHERE project_id=?1 AND strategy_key=?2 AND outcome IN ('validated','confirmed'))>0 THEN 1 ELSE 0 END,updated_at=datetime('now','localtime') WHERE project_id=?1 AND strategy_key=?2",
            params![project_id,strategy_key],
        ).map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn set_investigation_mutation_approval(
    state: State<AppState>,
    input: InvestigationMutationApprovalInput,
) -> Result<(), String> {
    if input.hypothesis_id <= 0 {
        return Err("调查假设 ID 无效".into());
    }
    let max_attempts = input.max_attempts.unwrap_or(1).clamp(1, 3);
    let expires_minutes = input.expires_minutes.unwrap_or(30).clamp(5, 240);
    let note = input.note.unwrap_or_default();
    if note.chars().count() > 500 {
        return Err("授权说明不能超过 500 个字符".into());
    }
    let connection = db::open(&state.db_path)?;
    let hypothesis = connection
        .query_row(
            "SELECT target_url,contract_json FROM investigation_hypotheses WHERE id=?1",
            [input.hypothesis_id],
            |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "调查假设不存在".to_string())?;
    let contract = investigation_json(hypothesis.1);
    let scope = serde_json::json!({
        "targetUrl":hypothesis.0,
        "endpoint":value_first(&contract,&["endpoint"]),
        "method":value_first(&contract,&["method"]),
        "contractKind":value_first(&contract,&["kind"]),
        "mutationPolicy":value_first(&contract,&["mutationPolicy"]),
    });
    if input.approved && value_first(&contract, &["endpoint"]).trim().is_empty() {
        return Err("该假设没有具体端点，不能授予状态变更权限".into());
    }
    connection.execute(
        "INSERT INTO investigation_mutation_approvals(hypothesis_id,approved,scope_json,max_attempts,note,expires_at) VALUES(?1,?2,?3,?4,?5,datetime('now','localtime',?6)) ON CONFLICT(hypothesis_id) DO UPDATE SET approved=excluded.approved,scope_json=excluded.scope_json,max_attempts=excluded.max_attempts,note=excluded.note,expires_at=excluded.expires_at,updated_at=datetime('now','localtime')",
        params![input.hypothesis_id,input.approved as i64,scope.to_string(),max_attempts,note,format!("+{expires_minutes} minutes")],
    ).map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn investigation_overview(state: State<AppState>, project_id: Option<i64>) -> Result<InvestigationOverview, String> {
    let connection = db::open(&state.db_path)?;
    let filter = project_id.map(|value| format!("={value}")).unwrap_or_else(|| "IS NOT NULL".into());
    if project_id.is_some_and(|value| value <= 0) { return Err("工作空间 ID 无效".into()) }
    let metric_sql = format!("SELECT COUNT(*),COALESCE(SUM(node_count),0),COALESCE(SUM(edge_count),0),COALESCE(SUM(api_count),0),COALESCE(SUM(parameter_count),0),COALESCE(SUM(hypothesis_count),0),COALESCE(SUM(token_worthy),0),COALESCE(AVG(information_gain),0) FROM investigation_metrics WHERE project_id {filter}");
    let metrics = connection.query_row(&metric_sql, [], |row| Ok((row.get::<_,i64>(0)?,row.get::<_,i64>(1)?,row.get::<_,i64>(2)?,row.get::<_,i64>(3)?,row.get::<_,i64>(4)?,row.get::<_,i64>(5)?,row.get::<_,i64>(6)?,row.get::<_,f64>(7)? as i64))).map_err(|error| error.to_string())?;
    let hypothesis_filter = if let Some(value)=project_id { format!("project_id={value}") } else { "project_id IS NOT NULL".into() };
    let ready = connection.query_row(&format!("SELECT COUNT(*) FROM investigation_hypotheses WHERE {hypothesis_filter} AND status IN ('ready','in_progress')"), [], |row| row.get::<_,i64>(0)).map_err(|error| error.to_string())?;
    let diffs = connection.query_row(&format!("SELECT COUNT(*) FROM investigation_identity_diffs WHERE {hypothesis_filter}"), [], |row| row.get::<_,i64>(0)).map_err(|error| error.to_string())?;
    let facts = connection.query_row(&format!("SELECT COUNT(*) FROM knowledge_facts WHERE {hypothesis_filter}"), [], |row| row.get::<_,i64>(0)).map_err(|error| error.to_string())?;
    let strategies = connection.query_row(&format!("SELECT COUNT(*) FROM knowledge_strategies WHERE {hypothesis_filter} AND promoted=1"), [], |row| row.get::<_,i64>(0)).map_err(|error| error.to_string())?;
    Ok(InvestigationOverview { target_count:metrics.0,node_count:metrics.1,edge_count:metrics.2,api_count:metrics.3,parameter_count:metrics.4,hypothesis_count:metrics.5,ready_hypothesis_count:ready,identity_diff_count:diffs,token_worthy_count:metrics.6,average_information_gain:metrics.7,fact_count:facts,promoted_strategy_count:strategies })
}

#[cfg(test)]
mod investigation_tests {
    use super::*;

    #[test]
    fn verification_contracts_are_bounded() {
        let contract = verification_contract("idor", &serde_json::json!({"endpoint":"/api/users/1","parameters":["id"]}));
        assert_eq!(contract["maxAttempts"], 3);
        assert_eq!(contract["mutationPolicy"], "automatic_bounded_same_contract");
        assert!(contract["stopRules"].as_array().unwrap().iter().any(|value| value == "confirmed_waf_or_challenge"));
    }

    #[test]
    fn paths_and_queries_are_stable() {
        assert_eq!(normalized_investigation_path("https://example.test/api/users/?q=1#x"), "/api/users");
        assert_eq!(query_parameter_names("/api?a=1&b=2&a=3"), vec!["a", "b"]);
        assert_eq!(identity_diff_endpoint_key("GET|api.example.test|/api/users|id"), "GET|/api/users");
        assert_eq!(identity_diff_endpoint_key("GET|/api/users"), "GET|/api/users");
    }

    #[test]
    fn identity_nodes_keep_runtime_validity_and_capture_status() {
        let target = serde_json::json!({"identityRuns":[{
            "identityKey":"session:a","identityLabel":"账号 A","sessionValid":true,
            "captureStatus":"complete","statusCode":200,"apiCount":7,
            "validationReason":"session_active"
        }]});
        let payload = identity_node_payload(&target, "session:a", 0);
        assert_eq!(payload["identityLabel"], "账号 A");
        assert_eq!(payload["sessionValid"], true);
        assert_eq!(payload["captureStatus"], "complete");
        assert_eq!(payload["apiCount"], 7);
    }

    #[test]
    fn generic_read_only_gets_do_not_enter_high_value_queue() {
        let generic = serde_json::json!({
            "method":"GET",
            "endpoint":"https://example.test/bbs/app/feeds",
            "category":"data_query",
            "title":"接口测试面"
        });
        assert!(opportunity_is_low_value(&generic));
        let suspicious = serde_json::json!({
            "method":"GET",
            "endpoint":"https://example.test/api/account/profile",
            "category":"identity_surface",
            "title":"账户资料"
        });
        assert!(!opportunity_is_low_value(&suspicious));
        let object = serde_json::json!({
            "method":"GET",
            "endpoint":"https://example.test/api/items/12345",
            "category":"api_surface",
            "title":"详情"
        });
        assert!(!opportunity_is_low_value(&object));
    }

    #[test]
    fn graph_excludes_background_transport_and_serialized_response_keys() {
        for value in [
            serde_json::json!({"method":"GET","url":"https://cdn.test/avatar/user.jpeg","resourceType":"Fetch"}),
            serde_json::json!({"method":"POST","url":"https://fp-it.portal101.cn/deviceprofile/v4"}),
            serde_json::json!({"method":"GET","url":"https://example.test/bbs/app/topic/categories"}),
            serde_json::json!({"method":"UNKNOWN","url":"/bbs/app/api/general/search/v1/web","source":"string-heuristic"}),
        ] {
            assert!(investigation_background_noise(&value));
        }
        let keys = serde_json::json!(["{\"msg\":\"\",\"result\":{}}", "msg", "result"]);
        assert_eq!(sanitized_investigation_response_keys(Some(&keys)), vec!["msg", "result"]);
    }

    #[test]
    fn telemetry_is_retained_as_related_service_with_identity_evidence() {
        let target = serde_json::json!({"runtimeExploration":{"requests":[
            {"method":"POST","url":"https://monitor.example.test/api/34/envelope/?sentry_version=7&sentry_key=public","resourceType":"XHR","source":"browser-runtime","identityKey":"account-a","actionId":"action-1"},
            {"method":"POST","url":"https://monitor.example.test/api/34/envelope/?sentry_version=7&sentry_key=public","resourceType":"Fetch","source":"browser-runtime-intercept","identityKey":"account-a","actionId":"action-1"},
            {"method":"POST","url":"https://monitor.example.test/api/34/envelope/?sentry_version=7&sentry_key=public","resourceType":"XHR","source":"browser-runtime","identityKey":"account-b","actionId":"action-2"}
        ]}});
        let services = investigation_related_services_from_target("https://www.example.test/app", &target);
        assert_eq!(services.len(), 1);
        assert_eq!(services[0]["host"], "monitor.example.test");
        assert_eq!(services[0]["classification"], "monitoring_telemetry");
        assert_eq!(services[0]["relation"], "same_party");
        assert_eq!(services[0]["requestCount"], 2);
        assert_eq!(services[0]["identityKeys"].as_array().unwrap().len(), 2);
        assert!(services[0]["queryKeys"].as_array().unwrap().iter().any(|value| value == "sentry_key"));
    }

    #[test]
    fn related_services_read_from_checkpoint_table_without_an_id_column() {
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE sentinel_checkpoints(scan_id TEXT,url TEXT,stage TEXT,raw_json TEXT,updated_at TEXT);").unwrap();
        let raw = serde_json::json!({"runtimeExploration":{"requests":[{
            "method":"POST","url":"https://monitor.example.test/api/1/envelope/","resourceType":"XHR","source":"browser-runtime"
        }]}}).to_string();
        connection.execute(
            "INSERT INTO sentinel_checkpoints(scan_id,url,stage,raw_json,updated_at) VALUES('scan-1','https://www.example.test','frontend_recon',?1,'2026-08-23 00:00:00')",
            [raw],
        ).unwrap();
        let services = read_investigation_related_services(&connection, "scan-1", "https://www.example.test").unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0]["host"], "monitor.example.test");
    }

    #[test]
    fn volatile_query_values_share_one_stable_contract() {
        let first = serde_json::json!({
            "method":"GET","category":"identity_surface",
            "endpoint":"https://example.test/api/account?nonce=one&ts=1","score":72
        });
        let second = serde_json::json!({
            "method":"GET","category":"identity_surface",
            "endpoint":"https://example.test/api/account?nonce=two&ts=2","score":88
        });
        assert_eq!(stable_opportunity_key(&first), stable_opportunity_key(&second));
        let grouped = deduplicated_actionable_opportunities(&[first, second]);
        assert_eq!(grouped.len(), 1);
        assert_eq!(grouped[0]["score"], 88);
    }

    #[test]
    fn opportunity_card_merges_both_identity_observations() {
        let mut left = serde_json::json!({
            "identityKeys":["account-a","account-b"],
            "identityScopeKeys":["account-a","account-b"],
            "identityRuns":[
                {"identityKey":"account-a","observed":true,"statusCode":200},
                {"identityKey":"account-b","observed":false}
            ]
        });
        let right = serde_json::json!({
            "identityKeys":["account-b"],
            "identityRuns":[
                {"identityKey":"account-a","observed":false},
                {"identityKey":"account-b","observed":true,"statusCode":200}
            ]
        });
        merge_opportunity_record(&mut left, &right);
        let runs = left["identityRuns"].as_array().unwrap();
        assert_eq!(runs.len(), 2);
        assert!(runs.iter().all(|run| run["observed"] == true));
    }

    #[test]
    fn only_concrete_requests_enter_the_agent_verification_queue() {
        let inferred = serde_json::json!({"score":88,"category":"identity_surface","endpoint":"/login","method":"UNKNOWN","candidateOnly":true,"source":"evidence-reconstruction"});
        assert!(!opportunity_agent_readiness(&inferred).0);
        let route = serde_json::json!({"score":80,"category":"frontend_feature","route":"/admin","source":"babel-ast"});
        assert!(!opportunity_agent_readiness(&route).0);
        let concrete = serde_json::json!({"score":82,"category":"identity_surface","endpoint":"/api/account","method":"GET","parameters":["id"],"source":"runtime-request","requestContext":{"status":200},"riskEvidence":{"present":true,"signals":[{"type":"object_boundary_parameter"}]}});
        assert!(opportunity_agent_readiness(&concrete).0);
        let ordinary = serde_json::json!({"score":86,"category":"identity_surface","endpoint":"/account/restore_login","method":"GET","source":"runtime-request","requestContext":{"status":200}});
        assert!(!opportunity_agent_readiness(&ordinary).0);
    }

    #[test]
    fn runtime_and_exact_source_mapped_reads_can_open_bounded_investigation() {
        let runtime = serde_json::json!({
            "method":"GET","url":"https://example.test/api/search?q=one",
            "source":"browser-runtime","statusCode":200
        });
        assert!(standard_investigation_api(&runtime));
        let static_candidate = serde_json::json!({
            "method":"GET","url":"/api/search","source":"babel-ast"
        });
        assert!(!standard_investigation_api(&static_candidate));
        assert!(!source_mapped_readonly_api(&static_candidate));
        let source_mapped_read = serde_json::json!({
            "method":"GET","url":"https://example.test/api/oauth/state",
            "source":"https://example.test/static/js/main.js.map#components/Login.js",
            "confidence":"high"
        });
        assert!(source_mapped_readonly_api(&source_mapped_read));
        let unrelated_source_mapped_read = serde_json::json!({
            "method":"GET","url":"https://api.github.com/repos/example/demo",
            "source":"https://example.test/static/js/main.js.map#vendor/example.js",
            "confidence":"high"
        });
        assert!(!source_mapped_readonly_api(&unrelated_source_mapped_read));
        let source_mapped_write = serde_json::json!({
            "method":"POST","url":"https://example.test/api/user/manage",
            "source":"https://example.test/static/js/main.js.map#components/Users.js",
            "confidence":"high"
        });
        assert!(!source_mapped_readonly_api(&source_mapped_write));
        let source_mapped_placeholder = serde_json::json!({
            "method":"GET","url":"https://example.test/api/user/<id>",
            "source":"https://example.test/static/js/main.js.map#components/Users.js",
            "confidence":"high"
        });
        assert!(!source_mapped_readonly_api(&source_mapped_placeholder));
        let telemetry = serde_json::json!({
            "method":"POST","url":"https://example.test/account/data_report_web",
            "source":"browser-runtime","statusCode":200
        });
        assert!(!standard_investigation_api(&telemetry));
        let unknown = serde_json::json!({
            "method":"UNKNOWN","url":"/api/general/search/v1/web",
            "source":"string-heuristic"
        });
        assert!(!standard_investigation_api(&unknown));
    }

    #[test]
    fn manual_deep_dive_is_target_specific_bounded_and_never_a_finding() {
        let apis = vec![
            (
                "GET|example.test|/api/orders/42".into(),
                serde_json::json!({
                    "method":"GET","url":"https://example.test/api/orders/42",
                    "parameters":["order_id","user_id"],"responseKeys":["id","status"]
                }),
            ),
            (
                "POST|example.test|/api/orders/export".into(),
                serde_json::json!({
                    "method":"POST","url":"https://example.test/api/orders/export",
                    "parameters":["order_id","callback_url"],"responseKeys":["task_id"]
                }),
            ),
        ];
        let actions = vec![serde_json::json!({"label":"导出订单","outcome":"clicked"})];
        let plan = manual_deep_dive_plan(
            &serde_json::json!({"businessEntrypoints":["订单详情","导出"]}),
            &apis,
            &actions,
            &["anonymous".into()],
            "standard",
        );
        let rows = plan.as_array().unwrap();
        assert!(!rows.is_empty());
        assert!(rows.len() <= 5);
        assert_eq!(rows[0]["category"], "authorization");
        assert!(rows.iter().any(|row| row["category"] == "business_flow"));
        assert!(rows.iter().any(|row| row["category"] == "file_handling"));
        assert!(rows.iter().any(|row| row["category"] == "server_side_integration"));
        assert!(rows.iter().all(|row| {
            row["classification"] == "coverage_gap_not_vulnerability"
                && row["steps"].as_array().is_some_and(|steps| !steps.is_empty())
                && !row["stopCondition"].as_str().unwrap_or_default().is_empty()
        }));
    }

    #[test]
    fn graph_persistence_builds_incremental_and_learning_layers() {
        let directory = std::env::temp_dir().join(format!(
            "oviraptor-investigation-test-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&directory).unwrap();
        let database = db::initialize(&directory).unwrap();
        let connection = db::open(&database).unwrap();
        connection.execute("INSERT INTO projects(name) VALUES('investigation-test')", []).unwrap();
        let project_id = connection.last_insert_rowid();
        let target = serde_json::json!({
            "url":"https://example.test/app",
            "finalUrl":"https://example.test/app",
            "fingerprint":{"backend":{"name":"Django","confidence":"medium"}},
            "apis":[{"method":"GET","url":"https://example.test/api/users/1?id=1","parameters":["id"],"source":"browser-runtime","confidence":"high","stateId":"state-1","actionId":"action-1","statusCode":200,"responseKeys":["id","name"]}],
            "opportunities":[{"opportunityKey":"idor-users","category":"idor","title":"用户对象权限差异","score":85,"confidence":"high","endpoint":"/api/users/1","method":"GET","parameters":["id"],"source":"runtime-request","requestContext":{"status":200},"riskEvidence":{"present":true,"signals":[{"type":"object_boundary_parameter"}]},"evidenceRefs":[{"type":"runtime_request"}]}],
            "runtimeExploration":{
                "states":[{"id":"state-1","url":"https://example.test/app","title":"Users","highValueLabels":["用户详情"]}],
                "actions":[{"id":"action-1","stateId":"state-1","label":"用户详情","role":"button","score":60,"outcome":"clicked","stateChanged":true,"requestCount":1,"beforeUrl":"https://example.test/app","afterUrl":"https://example.test/app"}],
                "requests":[{"method":"GET","url":"https://example.test/api/users/1?id=1","resourceType":"Fetch","stateId":"state-1","actionId":"action-1","queryKeys":["id"],"status":200,"responseKeys":["id","name"]}],
                "coverage":{"deduplicatedStateCount":0},"stopReason":"no_more_valuable_states"
            },
            "authSessionValidation":{"wafDetected":false}
        });
        for scan_id in ["scan-one", "scan-two"] {
            connection.execute("INSERT INTO sentinel_scans(id,project_id,project_name,status) VALUES(?1,?2,'investigation-test','completed')", params![scan_id,project_id]).unwrap();
            connection.execute("INSERT INTO sentinel_scan_contexts(scan_id,policy_json) VALUES(?1,'{}')", [scan_id]).unwrap();
            connection.execute("INSERT INTO sentinel_targets(project_id,scan_id,company,url,status) VALUES(?1,?2,'investigation-test','https://example.test/app','completed')", params![project_id,scan_id]).unwrap();
            let metrics = persist_investigation_graph(&connection, Some(project_id), scan_id, "https://example.test/app", &target).unwrap();
            let login_identity_nodes: i64 = connection.query_row(
                "SELECT COUNT(*) FROM investigation_nodes WHERE scan_id=?1 AND node_type='identity'",
                [scan_id],
                |row| row.get(0),
            ).unwrap();
            assert_eq!(login_identity_nodes, 0, "anonymous scope must not become a login identity");
            if scan_id == "scan-one" {
                assert!(metrics.token_worthy);
                assert!(metrics.node_count >= 6);
            } else {
                assert!(!metrics.token_worthy);
                assert_eq!(metrics.stop_reason, "incremental_no_new_value");
            }
        }
        let fact_count: i64 = connection.query_row("SELECT COUNT(*) FROM knowledge_facts", [], |row| row.get(0)).unwrap();
        let promoted: i64 = connection.query_row("SELECT promoted FROM knowledge_strategies WHERE category='idor'", [], |row| row.get(0)).unwrap();
        assert!(fact_count >= 4);
        assert_eq!(promoted, 1);
        let hypothesis_id: i64 = connection.query_row("SELECT id FROM investigation_hypotheses WHERE scan_id='scan-one' LIMIT 1", [], |row| row.get(0)).unwrap();
        connection.execute("INSERT INTO investigation_mutation_approvals(hypothesis_id,approved,scope_json,max_attempts,expires_at) VALUES(?1,1,'{\"method\":\"GET\",\"endpoint\":\"/api/users/1\"}',1,datetime('now','localtime','+30 minutes'))", [hypothesis_id]).unwrap();
        let approved = read_investigation_hypotheses(&connection, "scan-one", "", "").unwrap();
        assert_eq!(approved[0].contract["method"], "GET");
        assert_eq!(approved[0].mutation_approval["active"], true);
        connection.execute("UPDATE investigation_mutation_approvals SET expires_at=datetime('now','localtime','-1 minute') WHERE hypothesis_id=?1", [hypothesis_id]).unwrap();
        let expired = read_investigation_hypotheses(&connection, "scan-one", "", "").unwrap();
        assert_eq!(expired[0].mutation_approval["active"], false);
        drop(connection);
        fs::remove_dir_all(&directory).unwrap();
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationReplayInput {
    pub url: String,
    pub method: String,
    #[serde(default)]
    pub headers: JsonValue,
    #[serde(default)]
    pub body: String,
    #[serde(default = "default_replay_timeout")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub allow_mutation: bool,
    #[serde(default)]
    pub identity_id: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationReplayResult {
    pub status: u16,
    pub status_text: String,
    pub headers: JsonValue,
    pub body: String,
    pub decoded_body: String,
    pub content_type: String,
    pub content_encoding: String,
    pub body_is_json: bool,
    pub elapsed_ms: u128,
    pub identity_id: String,
}

fn default_replay_timeout() -> u64 { 120000 }

fn decode_replay_body(bytes: &[u8], encoding: &str) -> Result<Vec<u8>, String> {
    use std::io::Read;
    match encoding.split(',').next().unwrap_or("").trim() {
        "gzip" => { let mut decoder = flate2::read::GzDecoder::new(bytes); let mut output = Vec::new(); decoder.read_to_end(&mut output).map_err(|e| e.to_string())?; Ok(output) }
        "deflate" => { let mut decoder = flate2::read::ZlibDecoder::new(bytes); let mut output = Vec::new(); decoder.read_to_end(&mut output).map_err(|e| e.to_string())?; Ok(output) }
        "br" => { let mut decoder = brotli::Decompressor::new(bytes, 4096); let mut output = Vec::new(); decoder.read_to_end(&mut output).map_err(|e| e.to_string())?; Ok(output) }
        _ => Ok(bytes.to_vec()),
    }
}

#[tauri::command]
pub async fn replay_investigation_request(input: InvestigationReplayInput) -> Result<InvestigationReplayResult, String> {
    let identity_id = input.identity_id.clone();
    let parsed = reqwest::Url::parse(input.url.trim()).map_err(|error| format!("请求 URL 无效：{error}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("只允许发送 HTTP/HTTPS 请求".into());
    }
    let method = input.method.trim().to_ascii_uppercase();
    if method.is_empty() || method.len() > 12 || !method.bytes().all(|value| value.is_ascii_alphabetic()) {
        return Err("请求方法无效".into());
    }
    if !input.allow_mutation && !matches!(method.as_str(), "GET" | "HEAD" | "OPTIONS") {
        return Err("当前为只读重放模式；POST/PUT/PATCH/DELETE 需要显式授权后再发送".into());
    }
    if input.body.len() > 512 * 1024 {
        return Err("请求体超过 512 KB 限制".into());
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(input.timeout_ms.clamp(1000, 120000)))
        .redirect(reqwest::redirect::Policy::limited(3))
        .user_agent("Oviraptor-Repeater/1.0")
        .build()
        .map_err(|error| format!("创建重放客户端失败：{error}"))?;
    let mut request = client.request(
        reqwest::Method::from_bytes(method.as_bytes()).map_err(|error| error.to_string())?,
        parsed,
    );
    if let Some(headers) = input.headers.as_object() {
        for (key, value) in headers {
            if key.starts_with(':') || matches!(key.to_ascii_lowercase().as_str(), "host" | "content-length" | "connection" | "transfer-encoding" | "accept-encoding") { continue; }
            let Some(value) = value.as_str() else { continue; };
            let name = reqwest::header::HeaderName::from_bytes(key.as_bytes()).map_err(|error| format!("请求头无效：{error}"))?;
            let value = reqwest::header::HeaderValue::from_str(value).map_err(|error| format!("请求头值无效：{error}"))?;
            request = request.header(name, value);
        }
    }
    if !input.body.is_empty() { request = request.body(input.body); }
    let started = std::time::Instant::now();
    let response = request.send().await.map_err(|error| format!("发送请求失败：{error}"))?;
    let status = response.status();
    let status_text = status.canonical_reason().unwrap_or("").to_string();
    let headers = serde_json::Value::Object(response.headers().iter().map(|(key, value)| (key.to_string(), serde_json::Value::String(value.to_str().unwrap_or("").to_string()))).collect());
    let content_type = response.headers().get(reqwest::header::CONTENT_TYPE).and_then(|value| value.to_str().ok()).unwrap_or("").to_string();
    let content_encoding = response.headers().get(reqwest::header::CONTENT_ENCODING).and_then(|value| value.to_str().ok()).unwrap_or("").to_ascii_lowercase();
    let raw_bytes = response.bytes().await.map_err(|error| format!("读取响应失败：{error}"))?.to_vec();
    let decoded_bytes = decode_replay_body(&raw_bytes, &content_encoding).unwrap_or_else(|_| raw_bytes.clone());
    let body = String::from_utf8_lossy(&raw_bytes).chars().take(2_000_000).collect::<String>();
    let decoded_body = String::from_utf8_lossy(&decoded_bytes).chars().take(2_000_000).collect::<String>();
    let body_is_json = serde_json::from_str::<JsonValue>(&decoded_body).is_ok() || content_type.to_ascii_lowercase().contains("json");
    Ok(InvestigationReplayResult { status: status.as_u16(), status_text, headers, body, decoded_body, content_type, content_encoding, body_is_json, elapsed_ms: started.elapsed().as_millis(), identity_id })
}
