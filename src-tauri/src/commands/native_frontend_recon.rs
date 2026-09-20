#[allow(clippy::too_many_arguments)]
fn native_runtime_probe(
    helper: &Path,
    url: &str,
    auth_session: Option<&JsonValue>,
    timeout_seconds: u64,
    exploration_seconds: u64,
    deployment: &str,
    proxy: Option<&str>,
    no_proxy: &str,
    runtime_path: &OsString,
    comparison_requests: &[JsonValue],
    comparison_only: bool,
    target_action: Option<&str>,
    control: &NativeReconControl,
) -> Result<JsonValue, String> {
    let payload = serde_json::json!({
        "url": url,
        "authSession": auth_session.cloned().unwrap_or(JsonValue::Null),
        "timeoutMs": timeout_seconds.saturating_mul(1000),
        "explorationTimeoutMs": exploration_seconds.saturating_mul(1000),
        "maxActions": if deployment == "local" { 18 } else { 32 },
        "maxStates": if deployment == "local" { 10 } else { 18 },
        "maxDepth": 3,
        "maxRequests": 1200,
        "comparisonRequests": comparison_requests,
        "comparisonOnly": comparison_only,
        "targetAction": target_action.unwrap_or_default(),
    });
    run_native_json_helper(
        helper,
        &payload,
        runtime_path,
        proxy,
        no_proxy,
        Duration::from_secs(
            timeout_seconds
                .saturating_add(exploration_seconds)
                .saturating_add(20)
                .max(30),
        ),
        control,
    )
}

fn recon_header_map(headers: &reqwest::header::HeaderMap) -> JsonValue {
    JsonValue::Object(
        headers
            .iter()
            .filter_map(|(key, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (key.to_string(), JsonValue::String(value.to_string())))
            })
            .collect(),
    )
}

fn run_ast_helper(
    helper: &Path,
    source_url: &str,
    source: &str,
    runtime_path: &OsString,
    control: &NativeReconControl,
) -> Result<JsonValue, String> {
    run_native_json_helper(
        helper,
        &serde_json::json!({"url": source_url, "source": source}),
        runtime_path,
        None,
        "",
        Duration::from_secs(30),
        control,
    )
}

fn sensitive_findings(source: &str, source_url: &str, rules_path: &Path) -> Vec<JsonValue> {
    native_sensitive::scan(source, source_url, rules_path)
}

/// scripts, api candidates, route candidates, sensitive findings, runtime actions.
type StaticIntelligence = (
    Vec<JsonValue>,
    Vec<JsonValue>,
    Vec<JsonValue>,
    Vec<JsonValue>,
    Vec<JsonValue>,
);

fn static_frontend_intelligence(
    client: &reqwest::blocking::Client,
    final_url: &str,
    html: &str,
    runtime: &JsonValue,
    runtime_helper: &Path,
    runtime_path: &OsString,
    control: &NativeReconControl,
) -> StaticIntelligence {
    let Ok(base_url) = reqwest::Url::parse(final_url) else {
        return (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    };
    let pattern = regex::Regex::new(r#"(?is)<script[^>]+src\s*=\s*[\"']([^\"']+)[\"']"#)
        .expect("script pattern");
    let mut script_urls = pattern
        .captures_iter(html)
        .filter_map(|capture| capture.get(1))
        .filter_map(|value| base_url.join(value.as_str()).ok())
        .collect::<Vec<_>>();
    script_urls.extend(
        runtime
            .get("scripts")
            .and_then(JsonValue::as_array)
            .into_iter()
            .flatten()
            .filter_map(JsonValue::as_str)
            .filter_map(|value| base_url.join(value).ok()),
    );
    let mut seen = HashSet::new();
    script_urls.retain(|url| seen.insert(url.as_str().to_string()));
    let ast_helper = runtime_helper.with_file_name("8_js_ast_analyzer.cjs");
    let rules_path = runtime_helper
        .parent()
        .and_then(Path::parent)
        .map(|root| root.join("rules/sensitive.json"))
        .unwrap_or_default();
    let mut files = Vec::new();
    let mut candidates = Vec::new();
    let mut headers = Vec::new();
    let mut code_slices = Vec::new();
    let mut sensitive = Vec::new();
    let mut script_queue = std::collections::VecDeque::from(script_urls);
    let mut analyzed_scripts = 0;
    while let Some(script_url) = script_queue.pop_front() {
        if analyzed_scripts >= 8 {
            files.push(serde_json::json!({"url":script_url,"analysisStatus":"deferred","reason":"script_budget_reached"}));
            files.extend(script_queue.into_iter().map(|url|serde_json::json!({"url":url,"analysisStatus":"deferred","reason":"script_budget_reached"})));
            break;
        }
        analyzed_scripts += 1;
        if control.check().is_err() {
            break;
        }
        let external = script_url.host_str() != base_url.host_str();
        let response = match client.get(script_url.clone()).send() {
            Ok(response) => response,
            Err(error) => {
                files.push(serde_json::json!({"url":script_url,"external":external,"analysisStatus":"fetch_failed","error":error.to_string()}));
                continue;
            }
        };
        let status = response.status().as_u16();
        let source = match read_native_http_body(response, 1_000_000) {
            Ok(source) => source,
            Err(error) => {
                files.push(serde_json::json!({"url":script_url,"external":external,"statusCode":status,"analysisStatus":"read_failed","error":error.to_string()}));
                continue;
            }
        };
        let size = source.len();
        let analysis = run_ast_helper(
            &ast_helper,
            script_url.as_str(),
            &source,
            runtime_path,
            control,
        );
        match analysis {
            Ok(analysis) => {
                for imported in analysis.get("imports").and_then(JsonValue::as_array).into_iter().flatten().filter_map(JsonValue::as_str) {
                    if !(imported.starts_with('.') || imported.starts_with('/') || imported.starts_with("http://") || imported.starts_with("https://")) { continue; }
                    if let Ok(url) = script_url.join(imported) {
                        if matches!(url.scheme(), "http" | "https") && seen.insert(url.to_string()) { script_queue.push_back(url); }
                    }
                }
                for api in analysis.get("apis").and_then(JsonValue::as_array).into_iter().flatten() {
                    let method = value_first(api, &["method"]).to_ascii_uppercase();
                    if !matches!(method.as_str(), "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS") {
                        continue;
                    }
                    let raw = value_first(api, &["path", "url"]);
                    if raw.is_empty() || raw.contains('<') || raw.contains('>') {
                        continue;
                    }
                    let resolved = resolve_native_static_api_url(&base_url, api);
                    let Some(resolved) = resolved else { continue };
                    if !matches!(resolved.scheme(), "http" | "https") {
                        continue;
                    }
                    let mut candidate = api.clone();
                    if let Some(object) = candidate.as_object_mut() {
                        object.insert("url".into(), JsonValue::String(resolved.to_string()));
                        object.insert("path".into(), JsonValue::String(resolved.path().to_string()));
                        object.insert("candidateOnly".into(), JsonValue::Bool(true));
                        object.insert("source".into(), JsonValue::String("javascript-static".into()));
                        object.insert("verification".into(), serde_json::json!({"verified":false,"sameOrigin":resolved.origin()==base_url.origin(),"reason":"static_candidate_requires_runtime_observation"}));
                    }
                    candidates.push(candidate);
                }
                headers.extend(analysis.get("headerEvidence").and_then(JsonValue::as_array).cloned().unwrap_or_default());
                code_slices.extend(analysis.get("codeSlices").and_then(JsonValue::as_array).cloned().unwrap_or_default());
                files.push(serde_json::json!({"url":script_url,"external":external,"statusCode":status,"size":size,"analysisStatus":"complete","analysis":{"extractionEngine":"babel-ast","apiCandidates":analysis.get("apis").and_then(JsonValue::as_array).map(Vec::len).unwrap_or(0),"routes":analysis.get("routes").cloned().unwrap_or_else(||serde_json::json!([])),"stringEvidence":analysis.get("stringEvidence").cloned().unwrap_or_else(||serde_json::json!({})),"parseErrors":analysis.get("parseErrors").cloned().unwrap_or_else(||serde_json::json!([]))}}));
            }
            Err(error) => files.push(serde_json::json!({"url":script_url,"external":external,"statusCode":status,"size":size,"analysisStatus":"parse_failed","error":error})),
        }
        sensitive.extend(sensitive_findings(
            &source,
            script_url.as_str(),
            &rules_path,
        ));
    }
    let mut candidate_seen = HashSet::new();
    candidates.retain(|api| {
        candidate_seen.insert(format!(
            "{}|{}",
            value_first(api, &["method"]),
            value_first(api, &["url"])
        ))
    });
    let mut sensitive_seen = HashSet::new();
    sensitive.retain(|item| {
        sensitive_seen.insert(format!(
            "{}|{}|{}",
            value_first(item, &["type"]),
            value_first(item, &["source"]),
            value_first(item, &["maskedValue"])
        ))
    });
    (files, candidates, headers, code_slices, sensitive)
}

fn runtime_api_identity(request: &JsonValue) -> String {
    let url = value_first(request, &["url"]);
    let Ok(parsed) = reqwest::Url::parse(&url) else {
        return String::new();
    };
    let keys = parsed
        .query_pairs()
        .map(|(key, _)| key.to_string())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{}|{}|{}|{}",
        value_first(request, &["method"]).to_ascii_uppercase(),
        parsed.origin().ascii_serialization(),
        parsed.path(),
        keys
    )
}

fn api_from_runtime(request: &JsonValue) -> Option<JsonValue> {
    let resource = value_first(request, &["resourceType"]).to_ascii_lowercase();
    if !matches!(
        resource.as_str(),
        "xhr" | "fetch" | "eventsource" | "websocket"
    ) {
        return None;
    }
    let url = value_first(request, &["url"]);
    if !url.starts_with("http://")
        && !url.starts_with("https://")
        && !url.starts_with("ws://")
        && !url.starts_with("wss://")
    {
        return None;
    }
    let method = value_first(request, &["method"]).to_ascii_uppercase();
    if !matches!(
        method.as_str(),
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS" | "CONNECT"
    ) {
        return None;
    }
    Some(serde_json::json!({
        "path": reqwest::Url::parse(&url).ok().map(|value| value.path().to_string()).unwrap_or_else(|| url.clone()),
        "url": url, "method": method,
        "parameters": request.get("queryKeys").cloned().unwrap_or_else(|| serde_json::json!([])),
        "source": "browser-runtime", "confidence": "high", "extractionEngine": "browser-runtime",
        "statusCode": request.get("status").cloned().unwrap_or(JsonValue::Null),
        "contentType": value_first(request, &["contentType"]), "stateId": value_first(request, &["stateId"]),
        "actionId": value_first(request, &["actionId"]), "feature": value_first(request, &["feature"]),
        "postData": value_first(request, &["postData"]),
        "requestHeaders": request.get("effectiveRequestHeaders").or_else(|| request.get("headers")).cloned().unwrap_or_else(|| serde_json::json!({})),
        "requestHeaderNames": request.get("effectiveRequestHeaderNames").or_else(|| request.get("headerNames")).cloned().unwrap_or_else(|| serde_json::json!([])),
        "responseHeaders": request.get("effectiveResponseHeaders").or_else(|| request.get("responseHeaders")).cloned().unwrap_or_else(|| serde_json::json!({})),
        "responseHeaderNames": request.get("effectiveResponseHeaderNames").or_else(|| request.get("responseHeaderNames")).cloned().unwrap_or_else(|| serde_json::json!([])),
        "responseKeys": request.get("responseKeys").cloned().unwrap_or_else(|| serde_json::json!([])),
        "responsePreview": value_first(request, &["responsePreview"]),
        "identityKey": value_first(request, &["identityKey"]),
        "identityKeys":request.get("identityKeys").cloned().unwrap_or_else(||serde_json::json!([])),
        "verification": {"verified": request.get("status").and_then(JsonValue::as_i64).is_some(), "sameOrigin": request.get("sameOrigin").cloned().unwrap_or(JsonValue::Null), "reason": "browser_observed"}
    }))
}

#[allow(clippy::too_many_arguments)]
fn run_native_frontend_recon(
    company: &str,
    requested_url: &str,
    output_path: &Path,
    runtime_helper: &Path,
    auth_session_path: Option<&Path>,
    timeout_seconds: u64,
    exploration_seconds: u64,
    deployment: &str,
    proxy: Option<&str>,
    no_proxy: &str,
    runtime_path: &OsString,
    control: &NativeReconControl,
) -> Result<(), String> {
    control.check()?;
    let started = Instant::now();
    let mut builder = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(timeout_seconds.max(5)))
        .danger_accept_invalid_certs(true)
        .redirect(reqwest::redirect::Policy::limited(5));
    if let Some(proxy) = proxy.filter(|value| !value.trim().is_empty()) {
        builder = builder.proxy(reqwest::Proxy::all(proxy).map_err(|error| error.to_string())?);
    }
    let client = builder.build().map_err(|error| error.to_string())?;
    let mut collection_errors = Vec::<String>::new();
    let (status, final_url, headers, html) = match client
        .get(requested_url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (compatible; OviraptorFrontendRecon/3.0)",
        )
        .send()
    {
        Ok(response) => {
            let status = response.status().as_u16();
            let url = response.url().to_string();
            let headers = response.headers().clone();
            let body = read_native_http_body(response, 2_000_000).unwrap_or_else(|error| {
                collection_errors.push(error);
                String::new()
            });
            (status, url, headers, body)
        }
        Err(error) => {
            collection_errors.push(format!(
                "静态入口访问失败，继续浏览器采集：{}",
                error.without_url()
            ));
            (
                0,
                requested_url.to_string(),
                reqwest::header::HeaderMap::new(),
                String::new(),
            )
        }
    };
    let sessions = if let Some(path) = auth_session_path {
        let bytes = fs::read(path).map_err(|error| format!("无法读取本任务会话：{error}"))?;
        let document: JsonValue = serde_json::from_slice(&bytes)
            .map_err(|error| format!("本任务会话 JSON 无效：{error}"))?;
        let sessions = document
            .get("sessions")
            .and_then(JsonValue::as_array)
            .cloned()
            .unwrap_or_else(|| vec![document]);
        if sessions.is_empty()
            || sessions
                .iter()
                .any(|session| !session.is_object() || value_first(session, &["id"]).is_empty())
        {
            return Err("本任务会话缺少身份 ID，未静默降级为匿名扫描".into());
        }
        if sessions
            .iter()
            .map(native_identity_key)
            .collect::<HashSet<_>>()
            .len()
            != sessions.len()
        {
            return Err("本任务重复绑定了同一会话，无法形成独立账号对照".into());
        }
        sessions
    } else {
        vec![JsonValue::Null]
    };
    let mut raw_identity_runs = Vec::new();
    let mut identity_runs = Vec::new();
    for (index, session) in sessions.iter().enumerate() {
        control.check()?;
        let mut runtime = match native_runtime_probe(
            runtime_helper,
            &final_url,
            session.as_object().map(|_| session),
            timeout_seconds,
            exploration_seconds,
            deployment,
            proxy,
            no_proxy,
            runtime_path,
            &[],
            false,
            None,
            control,
        ) {
            Ok(runtime) => runtime,
            Err(error) => {
                control.check()?;
                collection_errors.push(format!(
                    "{} 采集失败：{error}",
                    native_identity_key(session)
                ));
                serde_json::json!({"available":false,"captureStatus":"failed","captureError":error,"requests":[]})
            }
        };
        let identity_key = native_identity_key(session);
        if let Some(requests) = runtime
            .get_mut("requests")
            .and_then(JsonValue::as_array_mut)
        {
            for request in requests {
                if let Some(object) = request.as_object_mut() {
                    object.insert(
                        "identityKey".into(),
                        JsonValue::String(identity_key.clone()),
                    );
                    object.insert("identityKeys".into(), serde_json::json!([identity_key]));
                }
            }
        }
        let validation = runtime
            .get("authSessionValidation")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let capture_status = value_first(&runtime, &["captureStatus"]);
        let available = runtime
            .get("available")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false);
        let request_count = runtime
            .get("requests")
            .and_then(JsonValue::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        identity_runs.push(serde_json::json!({"identityKey":identity_key,"identityLabel":if identity_key=="anonymous"{"匿名访问".to_string()}else{if index < 26 { format!("账号 {}", (b'A' + index as u8) as char) } else { format!("账号 {}", index + 1) }},"finalUrl":final_url,"statusCode":status,"valid":validation.get("valid").and_then(JsonValue::as_bool).unwrap_or(false)&&capture_status=="complete","sessionValid":if !session.is_null() && capture_status=="complete"{validation.get("valid").filter(|value| value.is_boolean()).cloned().unwrap_or(JsonValue::Null)}else{JsonValue::Null},"captureStatus":capture_status,"effectiveCaptureStatus":capture_status,"runtimeProbeAvailable":available,"captureError":value_first(&runtime,&["captureError"]),"runtimeStopReason":value_first(&runtime,&["runtimeStopReason"]),"validationReason":value_first(&validation,&["reason"]),"stateCount":runtime.get("states").and_then(JsonValue::as_array).map(Vec::len).unwrap_or(0),"actionCount":runtime.get("actions").and_then(JsonValue::as_array).map(Vec::len).unwrap_or(0),"apiCount":request_count,"observed":request_count>0}));
        raw_identity_runs.push(serde_json::json!({"identityKey": identity_key, "runtimeExploration": runtime, "authSessionValidation": validation}));
    }
    let comparison_requests = raw_identity_runs
        .iter()
        .flat_map(|run| {
            run.pointer("/runtimeExploration/requests")
                .and_then(JsonValue::as_array)
                .into_iter()
                .flatten()
        })
        .filter(|request| {
            matches!(
                value_first(request, &["method"])
                    .to_ascii_uppercase()
                    .as_str(),
                "GET" | "HEAD" | "OPTIONS"
            )
        })
        .filter(|request| api_from_runtime(request).is_some())
        .fold(Vec::<JsonValue>::new(), |mut items, request| {
            let key = format!(
                "{}|{}",
                value_first(request, &["method"]).to_ascii_uppercase(),
                value_first(request, &["url"])
            );
            if items.iter().all(|item| value_first(item, &["_key"]) != key) && items.len() < 24 {
                let mut candidate = request.clone();
                if let Some(object) = candidate.as_object_mut() {
                    object.insert("_key".into(), JsonValue::String(key));
                    object.insert(
                        "headers".into(),
                        request
                            .get("effectiveRequestHeaders")
                            .or_else(|| request.get("headers"))
                            .cloned()
                            .unwrap_or_else(|| serde_json::json!({})),
                    );
                }
                items.push(candidate);
            }
            items
        });
    if sessions.len() > 1 && !comparison_requests.is_empty() {
        for (index, session) in sessions.iter().enumerate() {
            let replay = native_runtime_probe(
                runtime_helper,
                &final_url,
                session.as_object().map(|_| session),
                timeout_seconds,
                exploration_seconds.min(90),
                deployment,
                proxy,
                no_proxy,
                runtime_path,
                &comparison_requests,
                true,
                None,
                control,
            );
            control.check()?;
            if let Some(runtime) = raw_identity_runs
                .get_mut(index)
                .and_then(|run| run.get_mut("runtimeExploration"))
                .and_then(JsonValue::as_object_mut)
            {
                match replay {
                    Ok(replay) => {
                        runtime.insert(
                            "comparisonReplays".into(),
                            replay
                                .get("comparisonReplays")
                                .cloned()
                                .unwrap_or_else(|| serde_json::json!([])),
                        );
                        if replay.get("available").and_then(JsonValue::as_bool) != Some(true)
                            || value_first(&replay, &["captureStatus"]) != "complete"
                        {
                            runtime.insert(
                                "captureStatus".into(),
                                JsonValue::String("partial".into()),
                            );
                            runtime.insert(
                                "comparisonCaptureError".into(),
                                replay
                                    .get("captureError")
                                    .cloned()
                                    .unwrap_or(JsonValue::Null),
                            );
                        }
                    }
                    Err(error) => {
                        collection_errors.push(format!(
                            "{} 对照采集失败：{error}",
                            native_identity_key(session)
                        ));
                        runtime.insert("captureStatus".into(), JsonValue::String("partial".into()));
                        runtime.insert("comparisonCaptureError".into(), JsonValue::String(error));
                    }
                }
            }
        }
    }
    let base_runtime = merge_native_identity_runtime(&raw_identity_runs);
    let requests = base_runtime["requests"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut api_map = HashMap::<String, JsonValue>::new();
    for request in &requests {
        if let Some(mut api) = api_from_runtime(request) {
            let same_origin = reqwest::Url::parse(&value_first(&api, &["url"]))
                .ok()
                .zip(reqwest::Url::parse(&final_url).ok())
                .is_some_and(|(left, right)| left.origin() == right.origin());
            api["verification"]["sameOrigin"] = JsonValue::Bool(same_origin);
            let key = runtime_api_identity(request);
            let identity = value_first(request, &["identityKey"]);
            let entry = api_map.entry(key).or_insert_with(|| api.clone());
            if native_observation_quality(&api) > native_observation_quality(entry) {
                let identities = entry
                    .get("identityKeys")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!([]));
                *entry = api;
                entry["identityKeys"] = identities;
            }
            if let Some(identities) = entry
                .get_mut("identityKeys")
                .and_then(JsonValue::as_array_mut)
            {
                if !identities
                    .iter()
                    .any(|value| value.as_str() == Some(&identity))
                {
                    identities.push(JsonValue::String(identity));
                }
            }
        }
    }
    let mut apis = api_map.into_values().collect::<Vec<_>>();
    apis.sort_by_key(runtime_api_identity);
    let identity_keys = identity_runs
        .iter()
        .map(|run| value_first(run, &["identityKey"]))
        .collect::<Vec<_>>();
    let mut identity_comparisons = Vec::new();
    let mut identity_api_rows = Vec::new();
    for api in &mut apis {
        let api_key = runtime_api_identity(api);
        let api_url = value_first(api, &["url"]);
        let method = value_first(api, &["method"]);
        let mut matrix = serde_json::Map::new();
        for identity in &identity_keys {
            let observed = requests
                .iter()
                .filter(|request| {
                    value_first(request, &["identityKey"]) == *identity
                        && runtime_api_identity(request) == api_key
                        && request.get("status").and_then(JsonValue::as_i64).is_some()
                })
                .max_by_key(|request| native_observation_quality(request));
            let replayed = raw_identity_runs
                .iter()
                .find(|run| value_first(run, &["identityKey"]) == *identity)
                .and_then(|run| run.pointer("/runtimeExploration/comparisonReplays"))
                .and_then(JsonValue::as_array)
                .and_then(|items| {
                    items
                        .iter()
                        .find(|item| runtime_api_identity(item) == api_key)
                });
            let evidence = observed.map(|request| native_identity_observation(request, false))
                .or_else(|| replayed.map(|request| native_identity_observation(request, true)))
                .unwrap_or_else(||serde_json::json!({"observed":false,"replayed":false,"status":null,"responseKeys":[]}));
            matrix.insert(identity.clone(), evidence);
        }
        let comparable_count = matrix
            .values()
            .filter(|value| {
                value
                    .get("observed")
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(false)
                    && value.get("status").and_then(JsonValue::as_i64).is_some()
            })
            .count();
        let difference_type = if comparable_count < identity_keys.len() {
            "insufficient_evidence"
        } else {
            let signatures = matrix
                .values()
                .map(|value| {
                    format!(
                        "{}|{}|{}",
                        value
                            .get("status")
                            .and_then(JsonValue::as_i64)
                            .unwrap_or_default(),
                        value_first(value, &["contentType"]),
                        value
                            .get("responseKeys")
                            .cloned()
                            .unwrap_or_else(|| serde_json::json!([]))
                    )
                })
                .collect::<HashSet<_>>();
            if signatures.len() == 1 {
                "same_surface"
            } else {
                "identity_response_difference"
            }
        };
        api["identityObservations"] = JsonValue::Array(
            matrix
                .iter()
                .filter(|(_, value)| {
                    value.get("observed").and_then(JsonValue::as_bool) == Some(true)
                })
                .map(|(identity, value)| {
                    let mut observation = value.clone();
                    observation["identityKey"] = JsonValue::String(identity.clone());
                    observation
                })
                .collect(),
        );
        if identity_keys.len() > 1 {
            identity_comparisons.push(serde_json::json!({"apiKey":api_key,"differenceType":difference_type,"comparisonBasis":"status_content_type_field_names","dataAccessConclusion":"not_verified","riskScore":if difference_type=="identity_response_difference"{35}else{0},"matrix":matrix.clone()}));
        }
        identity_api_rows.push(
            serde_json::json!({"apiKey":api_key,"method":method,"url":api_url,"matrix":matrix}),
        );
    }
    let (js_files, api_candidates, header_evidence, code_slices, sensitive_info) =
        static_frontend_intelligence(
            &client,
            &final_url,
            &html,
            &base_runtime,
            runtime_helper,
            runtime_path,
            control,
        );
    let external_scripts = js_files
        .iter()
        .filter(|item| {
            item.get("external")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false)
        })
        .cloned()
        .collect::<Vec<_>>();
    let server = headers
        .get(reqwest::header::SERVER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let powered = headers
        .get("x-powered-by")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let framework = if html.contains("__NEXT_DATA__") {
        "Next.js"
    } else if html.contains("__NUXT__") {
        "Nuxt"
    } else if html.contains("ng-version") {
        "Angular"
    } else if html.contains("data-v-") {
        "Vue"
    } else {
        "Unknown"
    };
    let opportunities = native_surface_opportunities(&apis);
    let all_complete = raw_identity_runs.iter().all(|run| {
        run.pointer("/runtimeExploration/captureStatus")
            .and_then(JsonValue::as_str)
            == Some("complete")
            && run
                .pointer("/runtimeExploration/available")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false)
    });
    let auth_applied = raw_identity_runs.iter().any(|run| {
        run.pointer("/authSessionValidation/applied")
            .and_then(JsonValue::as_bool)
            == Some(true)
    });
    let auth_valid = raw_identity_runs
        .iter()
        .filter(|run| {
            run.pointer("/authSessionValidation/applied")
                .and_then(JsonValue::as_bool)
                == Some(true)
        })
        .all(|run| {
            run.pointer("/authSessionValidation/valid")
                .and_then(JsonValue::as_bool)
                == Some(true)
        });
    let clear_invalid = raw_identity_runs.iter().any(|run| {
        run.pointer("/authSessionValidation/clearSessionInvalid")
            .and_then(JsonValue::as_bool)
            == Some(true)
    });
    let invalid_identity_keys = raw_identity_runs
        .iter()
        .filter(|run| {
            run.pointer("/authSessionValidation/clearSessionInvalid")
                .and_then(JsonValue::as_bool)
                == Some(true)
        })
        .map(|run| value_first(run, &["identityKey"]))
        .collect::<Vec<_>>();
    let waf_detected = raw_identity_runs.iter().any(|run| {
        run.pointer("/authSessionValidation/wafDetected")
            .and_then(JsonValue::as_bool)
            == Some(true)
    });
    let target = serde_json::json!({
        "url": requested_url, "company": company, "finalUrl": final_url, "statusCode": status, "errors": collection_errors,
        "fingerprint":{"frontend":{"name":framework,"framework":framework,"confidence":if framework=="Unknown"{"low"}else{"medium"}},"backend":{"name":if powered.is_empty(){"Unknown"}else{powered},"confidence":if powered.is_empty(){"low"}else{"medium"}},"server":{"name":if server.is_empty(){"Unknown"}else{server},"confidence":if server.is_empty(){"low"}else{"high"}},"waf":{"name":"Unknown","confidence":"low"},"cdn":{"name":"Unknown","confidence":"low"}},
        "techStack":{"framework":framework,"server":server,"poweredBy":powered,"baseUrls":[]}, "responseHeaders":recon_header_map(&headers),
        "jsFiles":js_files,"apis":apis,"apiCandidates":api_candidates,"blockedRequestCandidates":base_runtime.get("blockedRequests").cloned().unwrap_or_else(||serde_json::json!([])),
        "routes":base_runtime.get("routes").cloned().unwrap_or_else(||serde_json::json!([])),"features":base_runtime.get("features").cloned().unwrap_or_else(||serde_json::json!([])),"opportunities":opportunities,
        "runtimeExploration":base_runtime,"identityRuns":identity_runs,"identityComparisons":identity_comparisons,"identityMatrix":{"identities":identity_keys,"apis":identity_api_rows},"identityFeatureMatrix":{"identities":identity_keys,"features":[]},
        "authSessionValidation":{"applied":auth_applied,"valid":if auth_applied {JsonValue::Bool(auth_valid)}else{JsonValue::Null},"clearSessionInvalid":clear_invalid,"invalidIdentityKeys":invalid_identity_keys,"wafDetected":waf_detected,"reason":if !auth_applied{"anonymous_capture"}else if clear_invalid{"session_invalid"}else if all_complete{"complete"}else{"runtime_probe_incomplete"}},
        "sensitiveInfo":sensitive_info,"headerEvidence":header_evidence,"codeSlices":code_slices,"runtimeSignals":[],"cryptoSignals":[],"registrationEntrypoints":[],"realtimeEndpoints":[],"externalScripts":external_scripts,"metaTags":{},"links":base_runtime.get("links").cloned().unwrap_or_else(||serde_json::json!([])),"forms":base_runtime.get("forms").cloned().unwrap_or_else(||serde_json::json!([])),
        "analysisSummary":{"reconCacheVersion":4,"engine":"rust-native+cdp-node+babel-ast","runtimeBrowserAvailable":base_runtime.get("available").and_then(JsonValue::as_bool).unwrap_or(false),"runtimeProbeAvailable":base_runtime.get("available").and_then(JsonValue::as_bool).unwrap_or(false),"runtimeCaptureStatus":value_first(&base_runtime,&["captureStatus"]),"runtimeRequests":requests.len(),"identityCount":sessions.len(),"verifiedApis":apis.len(),"apiCandidates":api_candidates.len()},
        "durationMs":started.elapsed().as_millis()
    });
    let document = serde_json::json!({"schemaVersion":4,"analysisPipeline":["rust-native-http","cdp-browser-exploration","runtime-request-capture","identity-isolation","cross-identity-read-replay","babel-ast-static-candidates","sensitive-rule-pass","deterministic-contract-construction"],"generatedAt":chrono::Utc::now().to_rfc3339(),"targets":[target]});
    control.check()?;
    fs::write(
        output_path,
        serde_json::to_vec_pretty(&document).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    if !all_complete {
        return Err("CDP 运行时探测未完整成功；原生侦察证据已保留".into());
    }
    Ok(())
}
