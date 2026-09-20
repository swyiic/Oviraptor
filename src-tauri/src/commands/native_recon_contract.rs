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
        "parameters":request.get("queryKeys").or_else(||request.get("parameters")).cloned().unwrap_or_else(||serde_json::json!([])),
        "requestHeaders":request.get("effectiveRequestHeaders").or_else(||request.get("requestHeaders")).or_else(||request.get("headers")).cloned().unwrap_or_else(||serde_json::json!({})),
        "responseHeaders":request.get("effectiveResponseHeaders").or_else(||request.get("responseHeaders")).cloned().unwrap_or_else(||serde_json::json!({})),
        "requestBody":request_body.chars().take(24000).collect::<String>(),
        "responseBody":response.chars().take(24000).collect::<String>(),
        "responseBytes":response.len(),"responseBytesBasis":"captured_preview",
        "responseBodyCaptured":request.get("responseBodyCaptured").and_then(JsonValue::as_bool).unwrap_or(!response.is_empty()),
        "responseTruncated":request.get("responseTruncated").cloned().unwrap_or(JsonValue::Null),
        "source":if replayed {"cross-identity-replay"}else{"browser-runtime"},
    })
}

#[cfg(test)]
mod native_recon_contract_tests {
    use super::*;

    #[test]
    fn native_static_api_respects_explicit_client_prefix_without_guessing() {
        let page = reqwest::Url::parse("https://web.example.test/app/").unwrap();
        for path in ["/users", "/gateway/users"] {
            let api =
                serde_json::json!({"path":path,"clientBaseUrl":"https://api.example.test/gateway"});
            assert_eq!(
                resolve_native_static_api_url(&page, &api).unwrap().as_str(),
                "https://api.example.test/gateway/users"
            );
        }
        assert!(
            resolve_native_static_api_url(&page, &serde_json::json!({"path":"/users/${id}"}))
                .is_none()
        );
        assert!(resolve_native_static_api_url(
            &page,
            &serde_json::json!({"path":"javascript:alert(1)"})
        )
        .is_none());
    }

    #[test]
    fn native_identity_observation_preserves_each_accounts_replay_baseline() {
        let a = native_identity_observation(
            &serde_json::json!({"url":"https://example.test/api?nonce=a","method":"GET","status":200,"effectiveRequestHeaders":{"x-fixture":"A"},"responsePreview":"{\"name\":\"A\"}"}),
            false,
        );
        let b = native_identity_observation(
            &serde_json::json!({"url":"https://example.test/api?nonce=b","method":"GET","status":200,"outcome":"completed","effectiveRequestHeaders":{"x-fixture":"B"},"responsePreview":"{\"name\":\"B\"}"}),
            true,
        );
        assert_eq!(a["requestHeaders"]["x-fixture"], "A");
        assert_eq!(b["requestHeaders"]["x-fixture"], "B");
        assert_ne!(a["responseBody"], b["responseBody"]);
        assert_eq!(b["replayed"], true);
        assert_eq!(
            native_identity_observation(&serde_json::json!({"outcome":"not_sent"}), true)
                ["observed"],
            false
        );
    }

    #[test]
    fn native_identity_uses_session_id_not_a_shared_display_name() {
        assert_eq!(native_identity_key(&JsonValue::Null), "anonymous");
        assert_eq!(
            native_identity_key(&serde_json::json!({"id":"a","label":"登录会话"})),
            "session:a"
        );
        assert_ne!(
            native_identity_key(&serde_json::json!({"id":"a","label":"登录会话"})),
            native_identity_key(&serde_json::json!({"id":"b","label":"登录会话"}))
        );
    }

    #[test]
    fn native_merge_retains_second_identity_and_partial_status() {
        let merged = merge_native_identity_runtime(&[
            serde_json::json!({"identityKey":"A","runtimeExploration":{"available":true,"captureStatus":"complete","scripts":["common.js"],"features":[{"id":"f1"}],"requests":[]}}),
            serde_json::json!({"identityKey":"B","runtimeExploration":{"available":true,"captureStatus":"partial","scripts":["common.js","b.js"],"features":[{"id":"f1"}],"requests":[{"url":"https://example.test/b"}]}}),
        ]);
        assert_eq!(merged["scripts"].as_array().unwrap().len(), 2);
        assert_eq!(merged["features"].as_array().unwrap().len(), 2);
        assert_eq!(merged["features"][1]["identityKey"], "B");
        assert_eq!(merged["captureStatus"], "partial");
        assert_eq!(merged["runtimeProbeAvailable"], false);
        assert_eq!(merged["identityCaptures"].as_array().unwrap().len(), 2);
        assert_eq!(
            merge_native_identity_runtime(&[])["captureStatus"],
            "failed"
        );
    }

    #[test]
    fn native_path_keyword_is_not_risk_evidence() {
        let findings = native_surface_opportunities(&[
            serde_json::json!({"path":"/api/login","method":"GET","url":"https://example.test/api/login","statusCode":200}),
        ]);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0]["riskEvidence"]["present"], false);
        assert_ne!(findings[0]["readiness"]["stage"], "agent_ready");
        assert!(native_surface_opportunities(&[
            serde_json::json!({"path":"/home","url":"https://login.example.test/home"})
        ])
        .is_empty());
    }

    #[test]
    fn native_unknown_method_is_not_invented_and_hosts_do_not_merge() {
        assert!(api_from_runtime(&serde_json::json!({"url":"https://example.test/api","resourceType":"Fetch","method":"UNKNOWN"})).is_none());
        let a = serde_json::json!({"url":"https://a.example.test/api?id=1&nonce=x","method":"GET"});
        let b = serde_json::json!({"url":"https://a.example.test/api?nonce=y&id=2","method":"GET"});
        let c = serde_json::json!({"url":"https://b.example.test/api?id=1&nonce=x","method":"GET"});
        assert_eq!(runtime_api_identity(&a), runtime_api_identity(&b));
        assert_ne!(runtime_api_identity(&a), runtime_api_identity(&c));
    }
}
