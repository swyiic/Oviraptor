// Surface and opportunity classification for the investigation desk: which related
// services a target implies, how an opportunity is keyed and de-duplicated, what the
// verification contract demands, and how identity runs are summarised. Included from
// investigation.rs.

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
            ("identityKeys", (!identity.is_empty()).then_some(JsonValue::String(identity))),
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
    if !value_first(api, &["confidence"]).eq_ignore_ascii_case("high") {
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
    // Readiness establishes that a hypothesis has real risk evidence and a
    // concrete request contract. Mode-specific ranking happens later when the
    // packet is built (quick 65 / standard 50 / deep 35), so this floor must
    // not silently erase deep-mode candidates before ranking can see them.
    if score < 35 {
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

fn runtime_diagnostic_for<'a>(target: &'a JsonValue, identity_key: &str) -> Option<&'a JsonValue> {
    target
        .get("runtimeDiagnostics")
        .and_then(JsonValue::as_array)?
        .iter()
        .find(|item| value_first(item, &["identityKey"]) == identity_key)
}

fn anonymous_identity(identity_key: &str) -> bool {
    identity_key.trim().eq_ignore_ascii_case("anonymous")
}

fn identity_node_payload(target: &JsonValue, identity_key: &str, index: usize) -> JsonValue {
    let summary = identity_run_summary(target, identity_key);
    let diagnostics = runtime_diagnostic_for(target, identity_key);
    let pick = |keys: &[&str]| -> String {
        diagnostics
            .map(|item| value_first(item, keys))
            .filter(|value| !value.trim().is_empty() && value != "unknown")
            .or_else(|| {
                summary
                    .map(|run| value_first(run, keys))
                    .filter(|value| !value.trim().is_empty())
            })
            .unwrap_or_default()
    };
    let capture_status = {
        let from_diag = diagnostics
            .map(|item| value_first(item, &["captureStatus"]))
            .filter(|value| !value.trim().is_empty() && value != "unknown");
        let from_run = summary
            .map(|run| value_first(run, &["effectiveCaptureStatus", "captureStatus"]))
            .filter(|value| !value.trim().is_empty());
        from_diag.or(from_run).unwrap_or_else(|| "unknown".into())
    };
    serde_json::json!({
        "identityKey": identity_key,
        "identityLabel": summary.map(|run| value_first(run, &["identityLabel"])).filter(|value| !value.is_empty()).unwrap_or_else(|| format!("账号 {}", char::from(b'A' + (index.min(25) as u8)))),
        "sessionValid": summary.and_then(|run| run.get("sessionValid")).cloned().unwrap_or(JsonValue::Null),
        "valid": summary.and_then(|run| run.get("valid")).cloned().unwrap_or(JsonValue::Null),
        "captureStatus": capture_status,
        "runtimeProbeAvailable": diagnostics
            .and_then(|item| item.get("available"))
            .and_then(JsonValue::as_bool)
            .or_else(|| summary.and_then(|run| run.get("runtimeProbeAvailable")).and_then(JsonValue::as_bool))
            .unwrap_or(false),
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
        "captureError": pick(&["captureError"]),
        "runtimeStopReason": pick(&["runtimeStopReason"]),
        "failedStage": pick(&["failedStage", "probeStage"]),
        "cdpTransport": pick(&["cdpTransport"]),
        "browserVersion": pick(&["browserVersion"]),
        "nodeVersion": pick(&["nodeVersion"]),
        "browserExitCode": diagnostics
            .and_then(|item| item.get("browserExitCode"))
            .cloned()
            .unwrap_or(JsonValue::Null),
        "browserSignal": diagnostics
            .and_then(|item| item.get("browserSignal"))
            .cloned()
            .unwrap_or(JsonValue::Null),
        "browserStderr": pick(&["browserStderr"]),
        "runtimeDiagnostics": diagnostics.cloned().unwrap_or(JsonValue::Null),
    })
}
