/// Aggregate evidence without treating the first identity's successful capture
/// as success for every identity. Tag provenance without duplicating large
/// response bodies in a second copy of each runtime capture.
fn native_identity_key(session: &JsonValue) -> String {
    if session.is_null() {
        "anonymous".into()
    } else {
        format!("session:{}", value_first(session, &["id"]))
    }
}

fn resolve_native_static_api_url(page: &reqwest::Url, api: &JsonValue) -> Option<reqwest::Url> {
    let raw = value_first(api, &["path", "url"]).replace("\\/", "/");
    if raw.trim().is_empty()
        || ["${", "{{", "<", ">"]
            .iter()
            .any(|marker| raw.contains(marker))
    {
        return None;
    }
    let client_base = value_first(api, &["clientBaseUrl"]);
    let resolved =
        if raw.starts_with("http://") || raw.starts_with("https://") || raw.starts_with("//") {
            page.join(&raw).ok()?
        } else if !client_base.is_empty() {
            if ["${", "{{", "<", ">"]
                .iter()
                .any(|marker| client_base.contains(marker))
            {
                return None;
            }
            let mut base = page.join(&client_base).ok()?;
            base.set_query(None);
            base.set_fragment(None);
            let prefix = base.path().trim_end_matches('/');
            if !prefix.is_empty()
                && raw.starts_with('/')
                && (raw == prefix || raw.starts_with(&format!("{prefix}/")))
            {
                base.join(&raw).ok()?
            } else {
                reqwest::Url::parse(&format!(
                    "{}/{}",
                    base.as_str().trim_end_matches('/'),
                    raw.trim_start_matches('/')
                ))
                .ok()?
            }
        } else {
            page.join(&raw).ok()?
        };
    matches!(resolved.scheme(), "http" | "https").then_some(resolved)
}

fn merge_native_identity_runtime(runs: &[JsonValue]) -> JsonValue {
    let mut merged = serde_json::json!({});
    let complete = !runs.is_empty()
        && runs.iter().all(|run| {
            run.pointer("/runtimeExploration/available")
                .and_then(JsonValue::as_bool)
                == Some(true)
                && run
                    .pointer("/runtimeExploration/captureStatus")
                    .and_then(JsonValue::as_str)
                    == Some("complete")
        });
    let available = runs.iter().any(|run| {
        run.pointer("/runtimeExploration/available")
            .and_then(JsonValue::as_bool)
            == Some(true)
    });
    for key in [
        "scripts",
        "links",
        "navigations",
        "frameworks",
        "routes",
        "linkRecords",
        "forms",
        "states",
        "actions",
        "features",
        "requests",
        "blockedRequests",
        "comparisonReplays",
    ] {
        let mut values = Vec::new();
        let mut seen = HashSet::new();
        for run in runs {
            let identity = value_first(run, &["identityKey"]);
            for value in run
                .get("runtimeExploration")
                .and_then(|runtime| runtime.get(key))
                .and_then(JsonValue::as_array)
                .into_iter()
                .flatten()
            {
                let mut item = value.clone();
                if let Some(object) = item.as_object_mut() {
                    object.insert("identityKey".into(), JsonValue::String(identity.clone()));
                }
                if seen.insert(item.to_string()) {
                    values.push(item);
                }
            }
        }
        merged[key] = JsonValue::Array(values);
    }
    merged["available"] = JsonValue::Bool(available);
    merged["runtimeProbeAvailable"] = JsonValue::Bool(complete);
    merged["captureStatus"] = JsonValue::String(
        if complete {
            "complete"
        } else if available {
            "partial"
        } else {
            "failed"
        }
        .into(),
    );
    merged["identityCaptures"] = JsonValue::Array(
        runs.iter()
            .map(|run| {
                serde_json::json!({
                    "identityKey": value_first(run, &["identityKey"]),
                    "captureStatus": run.pointer("/runtimeExploration/captureStatus"),
                    "captureError": run.pointer("/runtimeExploration/captureError"),
                    "runtimeStopReason": run.pointer("/runtimeExploration/runtimeStopReason"),
                    "authSessionValidation": run.get("authSessionValidation"),
                })
            })
            .collect(),
    );
    merged["runtimeStopReason"] = JsonValue::String(
        if complete {
            "all_identity_captures_complete"
        } else {
            "identity_capture_incomplete"
        }
        .into(),
    );
    merged["browser"] = runs
        .first()
        .and_then(|run| run.pointer("/runtimeExploration/browser"))
        .cloned()
        .unwrap_or(JsonValue::Null);
    merged["coverage"] = serde_json::json!({
        "stateCount": merged["states"].as_array().map(Vec::len).unwrap_or(0),
        "actionCount": merged["actions"].as_array().map(Vec::len).unwrap_or(0),
        "requestCount": merged["requests"].as_array().map(Vec::len).unwrap_or(0),
        "identityCount": runs.len(),
    });
    merged
}

fn native_surface_opportunities(apis: &[JsonValue]) -> Vec<JsonValue> {
    apis.iter().filter(|api| {
        let path = value_first(api, &["path"]).to_ascii_lowercase();
        ["auth", "login", "admin", "upload", "export", "order", "payment", "graphql"]
            .iter().any(|word| path.contains(word))
    }).map(|api| serde_json::json!({
        "type":"api_contract",
        "title":format!("{} {}",value_first(api,&["method"]),value_first(api,&["path"])),
        "score":25,"method":value_first(api,&["method"]),"url":value_first(api,&["url"]),
        "readiness":{"stage":"needs_contract","reason":"observed_surface_without_risk_evidence"},
        "riskEvidence":{"present":false},
        "summary":"已观察到业务接口，仅为调查线索；路径名称和 HTTP 成功响应不能证明漏洞。"
    })).collect()
}

fn native_observation_quality(value: &JsonValue) -> (bool, bool, usize) {
    (
        value
            .get("status")
            .or_else(|| value.get("statusCode"))
            .and_then(JsonValue::as_u64)
            .is_some(),
        !value_first(value, &["responsePreview"]).is_empty(),
        value
            .get("responseKeys")
            .and_then(JsonValue::as_array)
            .map(Vec::len)
            .unwrap_or(0),
    )
}

fn captured_header_map(request: &JsonValue) -> JsonValue {
    let mut merged = serde_json::Map::new();
    // `effectiveRequestHeaders` is often `{}` when ExtraInfo never arrived.
    // The request that was actually sent still lives in `headers`.
    for key in ["headers", "requestHeaders", "effectiveRequestHeaders"] {
        let Some(object) = request.get(key).and_then(JsonValue::as_object) else {
            continue;
        };
        for (name, value) in object {
            if name.starts_with(':') || merged.contains_key(name) {
                continue;
            }
            let Some(text) = value.as_str().filter(|text| !text.is_empty()) else {
                continue;
            };
            merged.insert(name.clone(), JsonValue::String(text.to_string()));
        }
    }
    JsonValue::Object(merged)
}

fn captured_parameter_names(request: &JsonValue) -> JsonValue {
    let mut names = Vec::new();
    for key in ["parameters", "queryKeys", "bodyKeys"] {
        if let Some(items) = request.get(key).and_then(JsonValue::as_array) {
            for item in items {
                if let Some(name) = item.as_str().filter(|name| !name.is_empty()) {
                    if !names.iter().any(|existing| existing == name) {
                        names.push(name.to_string());
                    }
                }
            }
        }
    }
    JsonValue::Array(names.into_iter().map(JsonValue::String).collect())
}

fn native_identity_observation(request: &JsonValue, replayed: bool) -> JsonValue {
    let status = request
        .get("status")
        .or_else(|| request.get("statusCode"))
        .cloned()
        .unwrap_or(JsonValue::Null);
    let response = value_first(request, &["responsePreview", "bodyPreview", "responseBody"]);
    let request_body = value_first(request, &["postData", "requestBody"]);
    let observed = status.as_u64().is_some()
        && (!replayed || value_first(request, &["outcome"]) == "completed");
    serde_json::json!({
        "observed":observed,"naturallyObserved":!replayed && observed,"replayed":replayed,
        "method":value_first(request,&["method"]),"url":value_first(request,&["url"]),"status":status,
        "outcome":value_first(request,&["outcome"]),"error":value_first(request,&["error","reason"]),
        "responseKeys":request.get("responseKeys").cloned().unwrap_or_else(||serde_json::json!([])),
        "contentType":value_first(request,&["contentType"]),
        "parameters":captured_parameter_names(request),
        "requestHeaders":captured_header_map(request),
        "responseHeaders":request.get("effectiveResponseHeaders").or_else(||request.get("responseHeaders")).cloned().unwrap_or_else(||serde_json::json!({})),
        "requestBody":request_body.chars().take(24000).collect::<String>(),
        "responseBody":response.chars().take(24000).collect::<String>(),
        "responseBytes":response.len(),"responseBytesBasis":"captured_preview",
        "responseBodyCaptured":request.get("responseBodyCaptured").and_then(JsonValue::as_bool).unwrap_or(!response.is_empty()),
        "responseTruncated":request.get("responseTruncated").cloned().unwrap_or(JsonValue::Null),
        "source":if replayed {"cross-identity-replay"}else{"browser-runtime"},
    })
}

