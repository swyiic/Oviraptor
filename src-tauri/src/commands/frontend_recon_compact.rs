// Budgeted evidence compaction: what the frontend recon hands to the model when the
// raw capture would blow the context window. Included from frontend_recon.rs.

fn bounded_text(value: &JsonValue, keys: &[&str], limit: usize) -> String {
    value_first(value, keys).chars().take(limit).collect()
}

fn compact_parameter(value: &JsonValue) -> JsonValue {
    if let Some(text) = value.as_str() {
        return JsonValue::String(text.chars().take(180).collect());
    }
    serde_json::json!({
        "name": bounded_text(value, &["name", "key", "parameter", "param"], 180),
        "type": bounded_text(value, &["type", "kind"], 80),
        "location": bounded_text(value, &["in", "location", "source"], 80),
        "required": value.get("required").and_then(JsonValue::as_bool).unwrap_or(false),
        "value": bounded_text(value, &["value", "example", "default"], 240)
    })
}

fn compact_verification(value: &JsonValue) -> JsonValue {
    serde_json::json!({
        "status": bounded_text(value, &["status", "statusText"], 80),
        "statusCode": value.get("statusCode").or_else(|| value.get("httpStatus")).and_then(JsonValue::as_i64).unwrap_or(0),
        "method": bounded_text(value, &["method", "probeMethod"], 12),
        "url": bounded_text(value, &["url", "endpoint", "resolvedUrl"], 700),
        "parameter": bounded_text(value, &["parameter", "param"], 180),
        "evidence": bounded_text(value, &["evidence", "response", "body"], 900)
    })
}

fn compact_manual_deep_dive(decision: Option<&JsonValue>) -> Vec<JsonValue> {
    decision
        .and_then(|value| value.get("manualDeepDive"))
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .take(3)
        .map(|item| {
            let compact_strings = |key: &str, limit: usize, chars: usize| {
                item.get(key)
                    .and_then(JsonValue::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(JsonValue::as_str)
                    .take(limit)
                    .map(|value| value.chars().take(chars).collect::<String>())
                    .collect::<Vec<_>>()
            };
            serde_json::json!({
                "rank":item.get("rank").and_then(JsonValue::as_i64).unwrap_or(0),
                "category":bounded_text(item, &["category"], 80),
                "title":bounded_text(item, &["title"], 160),
                "priority":bounded_text(item, &["priority"], 20),
                "reason":bounded_text(item, &["reason"], 260),
                "evidence":compact_strings("evidence", 2, 220),
                "missingEvidence":bounded_text(item, &["missingEvidence"], 260),
                "steps":compact_strings("steps", 2, 220),
                "stopCondition":bounded_text(item, &["stopCondition"], 220),
                "classification":"coverage_gap_not_vulnerability"
            })
        })
        .collect()
}

fn compact_incremental_decision(decision: Option<&JsonValue>) -> JsonValue {
    let Some(value) = decision else {
        return serde_json::json!({});
    };
    serde_json::json!({
        "schemaVersion":value.get("schemaVersion").and_then(JsonValue::as_i64).unwrap_or(0),
        "eligibleForModel":value.get("eligibleForModel").and_then(JsonValue::as_bool).unwrap_or(false),
        "standardInvestigationAllowed":value.get("standardInvestigationAllowed").and_then(JsonValue::as_bool).unwrap_or(false),
        "baselineInvestigationAllowed":value.get("baselineInvestigationAllowed").and_then(JsonValue::as_bool).unwrap_or(false),
        "sourceGuidedInvestigationAllowed":value.get("sourceGuidedInvestigationAllowed").and_then(JsonValue::as_bool).unwrap_or(false),
        "automationTier":bounded_text(value, &["automationTier"], 80),
        "baseline":value.get("baseline").cloned().unwrap_or_default(),
        "readyHypotheses":value.get("readyHypotheses").and_then(JsonValue::as_i64).unwrap_or(0),
        "identityCount":value.get("identityCount").and_then(JsonValue::as_i64).unwrap_or(0),
        "observedRequestCount":value.get("observedRequestCount").and_then(JsonValue::as_i64).unwrap_or(0),
        "verifiedRuntimeApiCount":value.get("verifiedRuntimeApiCount").and_then(JsonValue::as_i64).unwrap_or(0),
        "sourceMappedReadOnlyApiCount":value.get("sourceMappedReadOnlyApiCount").and_then(JsonValue::as_i64).unwrap_or(0),
        "apiEvidenceSource":bounded_text(value, &["apiEvidenceSource"], 80),
        "runtimeProbeAvailable":value.get("runtimeProbeAvailable").and_then(JsonValue::as_bool).unwrap_or(false),
        "authSessionCaptureAvailable":value.get("authSessionCaptureAvailable").and_then(JsonValue::as_bool).unwrap_or(false),
        "coverageSemantics":value.get("coverageSemantics").cloned().unwrap_or_default(),
        "stopReason":bounded_text(value, &["stopReason"], 120),
    })
}

fn replay_header_is_safe(name: &str) -> bool {
    let lower = name.trim().to_ascii_lowercase();
    !lower.is_empty()
        && !lower.starts_with(':')
        && !matches!(
            lower.as_str(),
            "authorization"
                | "proxy-authorization"
                | "cookie"
                | "set-cookie"
                | "content-length"
                | "host"
                | "user-agent"
                | "accept-encoding"
                | "connection"
        )
        && !lower.contains("token")
        && !lower.contains("secret")
        && !lower.contains("session")
        && !lower.contains("csrf")
        && !lower.contains("signature")
        && !lower.contains("api-key")
        && !lower.contains("apikey")
}

fn compact_replay_headers(value: &JsonValue) -> JsonValue {
    let Some(headers) = value.as_object() else {
        return serde_json::json!({});
    };
    JsonValue::Object(
        headers
            .iter()
            .filter(|(name, _)| replay_header_is_safe(name))
            .take(16)
            .map(|(name, value)| {
                (
                    name.clone(),
                    JsonValue::String(
                        value
                            .as_str()
                            .unwrap_or_default()
                            .chars()
                            .take(500)
                            .collect(),
                    ),
                )
            })
            .collect(),
    )
}

fn redact_replay_body(value: &str) -> String {
    let sensitive_key = |key: &str| {
        let lower = key.trim().to_ascii_lowercase();
        ["password", "passwd", "pwd", "token", "secret", "authorization", "session", "csrf", "signature", "api_key", "apikey"]
            .iter()
            .any(|marker| lower.contains(marker))
    };
    let structured = matches!(value.trim_start().chars().next(), Some('{') | Some('['));
    if value.contains('=') && !structured {
        return value
            .split('&')
            .take(64)
            .map(|part| {
                let Some((key, raw)) = part.split_once('=') else {
                    return part.to_string();
                };
                if sensitive_key(key) && !raw.is_empty() {
                    format!("{key}=<auth-session>")
                } else {
                    part.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("&")
            .chars()
            .take(2_000)
            .collect();
    }
    value.chars().take(2_000).collect()
}

fn compact_api_observations(value: &JsonValue) -> Vec<JsonValue> {
    value
        .get("identityObservations")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("observed").and_then(JsonValue::as_bool).unwrap_or(false))
        .take(2)
        .map(|item| {
            let identity = bounded_text(item, &["identityKey"], 120);
            let anonymous = identity.is_empty() || identity == "anonymous";
            let identity_label = if anonymous { "anonymous" } else { identity.as_str() };
            let auth_material_ref = if anonymous {
                ""
            } else {
                "auth-session.json"
            };
            serde_json::json!({
                "identity": identity_label,
                "observed": true,
                "replayed": item.get("replayed").and_then(JsonValue::as_bool).unwrap_or(false),
                "method": bounded_text(item, &["method"], 12),
                "url": bounded_text(item, &["url"], 700),
                "status": item.get("status").and_then(JsonValue::as_i64).unwrap_or(0),
                "contentType": bounded_text(item, &["contentType"], 120),
                "request": {
                    "headers": compact_replay_headers(item.get("requestHeaders").unwrap_or(&JsonValue::Null)),
                    "body": redact_replay_body(&bounded_text(item, &["requestBody"], 2_000)),
                    "authMaterialRef": auth_material_ref,
                },
                "response": {
                    "body": bounded_text(item, &["responseBody"], 2_000),
                    "keys": item.get("responseKeys").and_then(JsonValue::as_array).map(|items| items.iter().take(20).cloned().collect::<Vec<_>>()).unwrap_or_default(),
                    "objectReferences": item.get("objectReferences").and_then(JsonValue::as_array).map(|items| items.iter().take(20).cloned().collect::<Vec<_>>()).unwrap_or_default(),
                    "bytes": item.get("responseBytes").and_then(JsonValue::as_i64).unwrap_or(0),
                },
                "evidenceClass": "browser_observed_request_response"
            })
        })
        .collect()
}

fn compact_api_candidate(value: JsonValue) -> JsonValue {
    let parameters = value
        .get("parameters")
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .take(8)
                .map(compact_parameter)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut request_header_names = value
        .get("requestHeaders")
        .and_then(JsonValue::as_object)
        .map(|headers| headers.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    request_header_names.extend(
        value
            .get("requestHeaderNames")
            .and_then(JsonValue::as_array)
            .into_iter()
            .flatten()
            .filter_map(JsonValue::as_str)
            .map(ToString::to_string),
    );
    request_header_names.extend(
        value
            .get("declaredHeaders")
            .and_then(JsonValue::as_array)
            .into_iter()
            .flatten()
            .filter_map(|item| item.get("name").and_then(JsonValue::as_str))
            .map(ToString::to_string),
    );
    request_header_names.sort_by_key(|name| name.to_ascii_lowercase());
    request_header_names.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    request_header_names.truncate(16);
    let observations = compact_api_observations(&value);
    serde_json::json!({
        "path": bounded_text(&value, &["path"], 500),
        "url": bounded_text(&value, &["url"], 700),
        "method": bounded_text(&value, &["method"], 12),
        "parameters": parameters,
        "responseKeys": value.get("responseKeys").and_then(JsonValue::as_array).map(|items| items.iter().take(12).cloned().collect::<Vec<_>>()).unwrap_or_default(),
        "requestHeaderNames": request_header_names,
        "extraRequestHeaderNames": value.get("extraRequestHeaderNames").and_then(JsonValue::as_array).map(|items| items.iter().take(12).cloned().collect::<Vec<_>>()).unwrap_or_default(),
        "extraInfoRequestHeaderNames": value.get("extraInfoRequestHeaderNames").and_then(JsonValue::as_array).map(|items| items.iter().take(16).cloned().collect::<Vec<_>>()).unwrap_or_default(),
        "initiator": value.get("initiator").map(|item| serde_json::json!({
            "type": bounded_text(item, &["type"], 40),
            "url": bounded_text(item, &["url"], 700),
            "lineNumber": item.get("lineNumber").and_then(JsonValue::as_i64).unwrap_or(-1),
            "functionName": bounded_text(item, &["functionName"], 240),
        })).unwrap_or_default(),
        "source": bounded_text(&value, &["source"], 700),
        "confidence": bounded_text(&value, &["confidence"], 20),
        "extractionEngine": bounded_text(&value, &["extractionEngine"], 32),
        "dynamic": value.get("dynamic").and_then(JsonValue::as_bool).unwrap_or(false),
        "candidateOnly": value.get("candidateOnly").and_then(JsonValue::as_bool).unwrap_or(false),
        "origin": bounded_text(&value, &["origin"], 300),
        "apiPrefix": bounded_text(&value, &["apiPrefix"], 240),
        "businessEndpoint": bounded_text(&value, &["businessEndpoint"], 500),
        "normalizedPath": bounded_text(&value, &["normalizedPath"], 500),
        "splitReason": bounded_text(&value, &["splitReason"], 80),
        "reconstructionConfidence": value.get("reconstructionConfidence").and_then(JsonValue::as_f64).unwrap_or(0.0),
        "evidenceLineage": value.get("evidenceLineage").and_then(JsonValue::as_array).map(|items| items.iter().take(3).cloned().collect::<Vec<_>>()).unwrap_or_default(),
        "evidence": bounded_text(&value, &["evidence"], 240),
        "verification": value.get("verification").map(compact_verification).unwrap_or_default(),
        "observations": observations,
    })
}

fn compact_request_header_intelligence(target: &JsonValue) -> JsonValue {
    let Some(value) = target.get("headerIntelligence") else {
        return serde_json::json!({});
    };
    let compact_rows = |key: &str, limit: usize| {
        value
            .get(key)
            .and_then(JsonValue::as_array)
            .map(|items| {
                items
                    .iter()
                    .take(limit)
                    .map(|item| {
                        serde_json::json!({
                            "name": bounded_text(item, &["name"], 160),
                            "observed": item.get("observed").and_then(JsonValue::as_bool).unwrap_or(false),
                            "declared": item.get("declared").and_then(JsonValue::as_bool).unwrap_or(false),
                            "sources": item.get("sources").and_then(JsonValue::as_array).map(|sources| sources.iter().take(4).cloned().collect::<Vec<_>>()).unwrap_or_default(),
                            "occurrences": item.get("occurrences").and_then(JsonValue::as_i64).unwrap_or(0),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let possible = value
        .get("possibleBrowserManaged")
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .take(8)
                .map(|item| {
                    serde_json::json!({
                        "name": bounded_text(item, &["name"], 160),
                        "reason": bounded_text(item, &["reason"], 240),
                        "possibleOnly": true,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    serde_json::json!({
        "observed": compact_rows("observed", 24),
        "declared": compact_rows("declared", 24),
        "possibleBrowserManaged": possible,
        "summary": value.get("summary").cloned().unwrap_or_default(),
        "policy": value.get("policy").cloned().unwrap_or_default(),
        "valuesOmittedFromModelPacket": true,
    })
}

fn compact_route_candidate(value: JsonValue) -> JsonValue {
    let parameters = value
        .get("parameters")
        .or_else(|| value.get("params"))
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .take(12)
                .map(compact_parameter)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    serde_json::json!({
        "path": bounded_text(&value, &["path"], 500),
        "url": bounded_text(&value, &["url", "href"], 700),
        "method": bounded_text(&value, &["method"], 12),
        "parameters": parameters,
        "source": bounded_text(&value, &["source", "file"], 500),
        "confidence": bounded_text(&value, &["confidence"], 20),
        "evidence": bounded_text(&value, &["evidence", "context"], 500)
    })
}

fn compact_sensitive_candidate(value: JsonValue) -> JsonValue {
    serde_json::json!({
        "type": bounded_text(&value, &["type", "kind"], 80),
        "severity": bounded_text(&value, &["severity", "risk"], 20),
        "name": bounded_text(&value, &["name", "key", "parameter"], 160),
        "value": bounded_text(&value, &["value", "match", "context"], 700),
        "source": bounded_text(&value, &["source", "url", "file"], 700),
        "evidence": bounded_text(&value, &["evidence", "reason"], 500)
    })
}

fn frontend_packet_budget(settings: &JsonValue, deployment: &str) -> usize {
    let normalized = db::normalize_settings(settings);
    let settings = &normalized;
    let configured = settings
        .get("agentFrontendPacketBudgetKb")
        .and_then(JsonValue::as_u64)
        .unwrap_or(24)
        .clamp(4, 64) as usize;
    let budget = match settings
        .get("agentFrontendPacketMode")
        .and_then(JsonValue::as_str)
        .unwrap_or("balanced")
    {
        "compact" => 6 * 1024,
        "custom" => configured * 1024,
        _ => configured * 1024,
    };
    if deployment == "local" {
        budget.min(12 * 1024)
    } else {
        budget
    }
}

fn compact_ai_fallback(target: &JsonValue) -> JsonValue {
    let Some(value) = target.get("aiFallback").filter(|value| {
        value
            .get("enabled")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false)
    }) else {
        return serde_json::json!({});
    };
    let snippets = value
        .get("snippets")
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .take(6)
                .map(|item| {
                    serde_json::json!({
                        "sliceId": bounded_text(item, &["sliceId"], 32),
                        "source": bounded_text(item, &["source"], 700),
                        "marker": bounded_text(item, &["marker"], 80),
                        "context": bounded_text(item, &["context"], 900),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let slice_index = value
        .get("codeSlices")
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .take(8)
                .map(|item| {
                    serde_json::json!({
                        "id": bounded_text(item, &["id"], 32),
                        "source": bounded_text(item, &["source"], 700),
                        "kind": bounded_text(item, &["kind"], 80),
                        "marker": bounded_text(item, &["marker"], 120),
                        "start": item.get("start").and_then(JsonValue::as_i64).unwrap_or(0),
                        "end": item.get("end").and_then(JsonValue::as_i64).unwrap_or(0),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    serde_json::json!({
        "enabled": !snippets.is_empty(),
        "framework": bounded_text(value, &["framework"], 80),
        "reason": bounded_text(value, &["reason"], 240),
        "instructions": "Infer concrete request candidates from the previews first. Only when a specific dependency is missing, read frontend-code-index.json, choose dependency-definition/http-client slices before related network-call slices, and read at most three files under frontend-code-slices/. Never read every slice or request the complete minified bundle. Verify every inferred endpoint with a request/response tool. If two supplemental reads produce no verifiable candidate, stop static analysis and use the recommended runtime hook.",
        "sliceIndexFile": if slice_index.is_empty() { "" } else { "frontend-code-index.json" },
        "maxSliceReads": value.get("maxSliceReads").and_then(JsonValue::as_i64).unwrap_or(0).min(3),
        "maxCumulativeSliceChars": value.get("maxCumulativeSliceChars").and_then(JsonValue::as_i64).unwrap_or(0).min(42_000),
        "sliceIndex": slice_index,
        "snippets": snippets,
    })
}

fn trim_evidence_to_budget(evidence: &mut JsonValue, max_bytes: usize) {
    let limits = [
        ("opportunities", 1usize),
        ("localKnowledgeMatches", 1usize),
        ("runtimeSignals", 2usize),
        ("applicationScripts", 3),
        ("routeCandidates", 2),
        ("apiCandidates", 2),
        ("sensitiveCandidates", 2),
    ];
    while serde_json::to_vec(evidence).is_ok_and(|bytes| bytes.len() > max_bytes) {
        let mut removed = false;
        for (key, minimum) in limits {
            if let Some(items) = evidence.get_mut(key).and_then(JsonValue::as_array_mut) {
                if items.len() > minimum {
                    items.pop();
                    removed = true;
                    break;
                }
            }
        }
        if !removed {
            break;
        }
    }
    if serde_json::to_vec(evidence).is_ok_and(|bytes| bytes.len() > max_bytes) {
        if let Some(fallback) = evidence
            .get_mut("aiFallback")
            .and_then(JsonValue::as_object_mut)
        {
            fallback.insert("snippets".into(), serde_json::json!([]));
            fallback.insert("sliceIndex".into(), serde_json::json!([]));
        }
    }
    if serde_json::to_vec(evidence).is_ok_and(|bytes| bytes.len() > max_bytes) {
        if let Some(object) = evidence.as_object_mut() {
            object.insert("localAnalysis".into(), serde_json::json!({}));
            object.insert("techStack".into(), serde_json::json!({}));
            object.insert("fingerprint".into(), serde_json::json!({}));
        }
    }
    if serde_json::to_vec(evidence).is_ok_and(|bytes| bytes.len() > max_bytes) {
        let reasons = evidence
            .get("routingReasons")
            .and_then(JsonValue::as_array)
            .map(|items| {
                items
                    .iter()
                    .take(3)
                    .filter_map(JsonValue::as_str)
                    .map(|value| value.chars().take(180).collect::<String>())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let first = |key: &str| {
            evidence
                .get(key)
                .and_then(JsonValue::as_array)
                .and_then(|items| items.first())
                .cloned()
                .into_iter()
                .collect::<Vec<_>>()
        };
        *evidence = serde_json::json!({
            "schemaVersion": 2,
            "url": evidence.get("url").and_then(JsonValue::as_str).unwrap_or("").chars().take(1000).collect::<String>(),
            "valueScore": evidence.get("valueScore").and_then(JsonValue::as_i64).unwrap_or(0),
            "routingReasons": reasons,
            "opportunities": first("opportunities"),
            "apiCandidates": first("apiCandidates"),
            "routeCandidates": first("routeCandidates"),
            "sensitiveCandidates": first("sensitiveCandidates"),
            "runtimeSignals": first("runtimeSignals"),
            "runtimeHookRecommended": evidence.get("runtimeHookRecommended").and_then(JsonValue::as_bool).unwrap_or(false),
            "runtimeHookPlan": evidence.get("runtimeHookPlan").cloned().unwrap_or_default(),
            "investigation": evidence.get("investigation").cloned().unwrap_or_default(),
            "evidenceTruncated": true,
            "stopRule": "Validate the strongest candidate only; request local evidence by ID instead of reading a complete bundle."
        });
    }
    if serde_json::to_vec(evidence).is_ok_and(|bytes| bytes.len() > max_bytes) {
        let url = evidence
            .get("url")
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .chars()
            .take(1000)
            .collect::<String>();
        let score = evidence
            .get("valueScore")
            .and_then(JsonValue::as_i64)
            .unwrap_or(0);
        *evidence = serde_json::json!({
            "schemaVersion": 2,
            "url": url,
            "valueScore": score,
            "investigation": evidence.get("investigation").cloned().unwrap_or_default(),
            "evidenceTruncated": true,
            "stopRule": "Local evidence exceeded the strict packet budget. Use deterministic runtime validation and do not read a complete bundle."
        });
    }
}

/// Keep the model input small and deterministic. The complete recon JSON stays
/// on disk for the result viewer; the Native Agent receives only high-value candidates.
fn compact_frontend_evidence(
    target: &JsonValue,
    requested_url: &str,
    route: &FrontendRoute,
    max_bytes: usize,
) -> JsonValue {
    let expanded_cloud_packet = max_bytes >= 20 * 1024;
    let (api_limit, route_limit, sensitive_limit, script_limit, runtime_limit) =
        match (route.mode.as_str(), expanded_cloud_packet) {
            ("quick", true) => (2, 2, 2, 2, 3),
            ("standard", true) => (4, 4, 3, 3, 5),
            (_, true) => (6, 6, 4, 4, 8),
            ("quick", false) => (1, 1, 1, 1, 2),
            ("standard", false) => (2, 2, 2, 2, 3),
            _ => (3, 3, 3, 3, 4),
        };
    let mut apis = target
        .get("apis")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    apis.extend(
        target
            .get("apiCandidates")
            .and_then(JsonValue::as_array)
            .into_iter()
            .flatten()
            .filter(|value| actionable_api_candidate(value))
            .cloned(),
    );
    let mut seen_apis = HashSet::new();
    apis.retain(|value| {
        seen_apis.insert(format!(
            "{}|{}",
            value_first(value, &["method"]),
            value_first(value, &["url", "path"])
        ))
    });
    apis.sort_by_key(|value| std::cmp::Reverse(evidence_priority(value)));
    apis.retain(actionable_api_candidate);
    let mut routes = target
        .get("routes")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    routes.sort_by_key(|value| std::cmp::Reverse(evidence_priority(value)));
    let mut sensitive = target
        .get("sensitiveInfo")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    sensitive.sort_by_key(|value| {
        let severity = value
            .get("severity")
            .and_then(JsonValue::as_str)
            .unwrap_or("");
        (
            severity != "high",
            std::cmp::Reverse(evidence_priority(value)),
        )
    });
    let mut js_files = target
        .get("jsFiles")
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|item| {
                    matches!(
                        item.get("type").and_then(JsonValue::as_str),
                        Some("application") | Some("chunk") | Some("plugin")
                    )
                })
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    js_files.sort_by_key(|value| std::cmp::Reverse(script_evidence_priority(value)));
    let js_files = js_files
        .into_iter()
        .take(script_limit)
        .map(|item| {
            serde_json::json!({
                "url": value_first(&item, &["url"]),
                "type": value_first(&item, &["type"]),
                "statusCode": item.get("statusCode").and_then(JsonValue::as_i64).unwrap_or(0),
                "size": item.get("size").and_then(JsonValue::as_i64).unwrap_or(0),
                "discoveredFrom": value_first(&item, &["discoveredFrom"]),
                "analysis": item.get("analysis").cloned().unwrap_or_default(),
            })
        })
        .collect::<Vec<_>>();
    let business_entries = target
        .get("registrationEntrypoints")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .take(api_limit)
        .map(|item| {
            serde_json::json!({
                "type": "registration",
                "url": bounded_text(item, &["url", "path"], 700),
                "method": bounded_text(item, &["method"], 16),
                "title": bounded_text(item, &["title", "label"], 160),
                "confidence": bounded_text(item, &["confidence"], 20),
                "verification": item.get("verification").cloned().unwrap_or_default(),
            })
        })
        .chain(
            target
                .get("forms")
                .and_then(JsonValue::as_array)
                .into_iter()
                .flatten()
                .filter(|item| {
                    let text = item.to_string().to_ascii_lowercase();
                    [
                        "login", "signin", "auth", "register", "signup", "upload", "登录", "注册",
                        "上传",
                    ]
                    .iter()
                    .any(|keyword| text.contains(keyword))
                })
                .take(api_limit)
                .map(|item| {
                    serde_json::json!({
                        "type": "form",
                        "url": bounded_text(item, &["action", "url"], 700),
                        "method": bounded_text(item, &["method"], 16),
                        "title": bounded_text(item, &["text", "name", "id"], 160),
                    })
                }),
        )
        .take(api_limit)
        .collect::<Vec<_>>();
    let mut runtime_signals = target
        .get("runtimeSignals")
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|value| {
                    let crypto_type = matches!(
                        value.get("type").and_then(JsonValue::as_str),
                        Some("cryptojs")
                            | Some("jsencrypt")
                            | Some("sm_crypto")
                            | Some("web_crypto")
                    );
                    let crypto_hook = matches!(
                        value.get("hook").and_then(JsonValue::as_str),
                        Some("cryptojs")
                            | Some("jsencrypt")
                            | Some("sm_crypto")
                            | Some("web_crypto")
                    );
                    !crypto_type && !crypto_hook
                })
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    runtime_signals.sort_by_key(|value| std::cmp::Reverse(runtime_signal_priority(value)));
    let runtime_hook_plan = target
        .get("runtimeHookPlan")
        .filter(|plan| {
            !matches!(
                plan.get("hook").and_then(JsonValue::as_str),
                Some("cryptojs") | Some("jsencrypt") | Some("sm_crypto") | Some("web_crypto")
            )
        })
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let runtime_hook_recommended = runtime_hook_plan
        .as_object()
        .is_some_and(|plan| !plan.is_empty());
    let ai_fallback = if route.surface == "static_frontend" {
        serde_json::json!({})
    } else {
        compact_ai_fallback(target)
    };
    let stop_rule = if route.surface == "static_frontend" {
        "This is a static framework surface with no actionable API or business entry. Do not read frontend code slices. Use at most two narrowly scoped verification tools; if neither produces a new endpoint or distinct response, finish the target immediately."
    } else if route.surface == "framework_application" {
        "This is a bounded complex-frontend evidence packet. If evidence-deep hypotheses exist, validate them first. Otherwise, when standardInvestigationAllowed is true, inspect and replay only the top browser-observed API contracts as read-only controls and summarize coverage. Do not crawl the whole SPA, enumerate unrelated routes, or reread complete bundles. Stop after the configured attempts and finish the target even when no security impact is confirmed."
    } else if route.surface == "ordinary_web" {
        "Validate the strongest opportunity first. If none is actionable, run only one bounded directory/API discovery pass. Record isolated 401/403 responses as auth or role boundaries and continue other functions. Stop on confirmed WAF/bot challenge/CAPTCHA, sustained rate limiting, repeated homogeneous blocking, or when the pass produces no new valuable endpoint."
    } else {
        "Validate the strongest candidates only; stop after three requests without new evidence, endpoint, or verification result."
    };
    let compact_apis = apis
        .into_iter()
        .take(api_limit)
        .map(compact_api_candidate)
        .collect::<Vec<_>>();
    let compact_routes = routes
        .into_iter()
        .take(route_limit)
        .map(compact_route_candidate)
        .collect::<Vec<_>>();
    let compact_sensitive = sensitive
        .into_iter()
        .take(sensitive_limit)
        .map(compact_sensitive_candidate)
        .collect::<Vec<_>>();
    let api_intelligence = target
        .get("apiIntelligence")
        .map(|value| serde_json::json!({
            "clients": value.get("clients").and_then(JsonValue::as_array).map(|items| items.iter().take(4).cloned().collect::<Vec<_>>()).unwrap_or_default(),
            "reconstructions": value.get("reconstructions").and_then(JsonValue::as_array).map(|items| items.iter().filter(|item| !item.get("validated").and_then(JsonValue::as_bool).unwrap_or(false)).take(6).cloned().collect::<Vec<_>>()).unwrap_or_default(),
            "policy": value.get("policy").cloned().unwrap_or_default(),
        }))
        .unwrap_or_default();
    let request_header_intelligence = compact_request_header_intelligence(target);
    let mut compact_opportunities = target
        .get("opportunities")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    compact_opportunities
        .retain(|item| model_ready_opportunity(item) && item.get("score").and_then(JsonValue::as_i64).unwrap_or(0) >= 65);
    compact_opportunities.sort_by_key(|item| {
        std::cmp::Reverse(item.get("score").and_then(JsonValue::as_i64).unwrap_or(0))
    });
    let compact_opportunities = compact_opportunities
        .into_iter()
        .take(api_limit)
        .map(|item| {
            serde_json::json!({
                "key": bounded_text(&item, &["opportunityKey"], 40),
                "category": bounded_text(&item, &["category"], 80),
                "title": bounded_text(&item, &["title"], 240),
                "score": item.get("score").and_then(JsonValue::as_i64).unwrap_or(0),
                "endpoint": bounded_text(&item, &["endpoint", "route", "targetUrl"], 700),
                "method": bounded_text(&item, &["method"], 16),
                "parameters": item.get("parameters").and_then(JsonValue::as_array).map(|items| items.iter().take(12).cloned().collect::<Vec<_>>()).unwrap_or_default(),
                "whyValuable": item.get("whyValuable").and_then(JsonValue::as_array).map(|items| items.iter().take(3).cloned().collect::<Vec<_>>()).unwrap_or_default(),
                "evidenceRefs": item.get("evidenceRefs").and_then(JsonValue::as_array).map(|items| items.iter().take(2).cloned().collect::<Vec<_>>()).unwrap_or_default(),
                "recommendedAction": item.get("recommendedAction").cloned().unwrap_or_default(),
                "verificationMode": item.get("verificationMode").cloned().unwrap_or_else(|| serde_json::json!("ai_auto")),
                "humanReviewStage": item.get("humanReviewStage").cloned().unwrap_or_else(|| serde_json::json!("final_verdict_only")),
            })
        })
        .collect::<Vec<_>>();
    let primary_candidate = compact_opportunities
        .first()
        .or_else(|| compact_apis.first())
        .or_else(|| business_entries.first())
        .or_else(|| compact_sensitive.first())
        .or_else(|| js_files.first())
        .cloned()
        .unwrap_or_default();
    let mut evidence = serde_json::json!({
        "schemaVersion": 2,
        "url": if requested_url.trim().is_empty() { route.url.clone() } else { requested_url.to_string() },
        "valueScore": route.score,
        "surface": route.surface,
        "routingReasons": route.reasons.clone(),
        "fingerprint": target.get("fingerprint").cloned().unwrap_or_default(),
        "techStack": target.get("techStack").cloned().unwrap_or_default(),
        "localAnalysis": target.get("analysisSummary").cloned().unwrap_or_default(),
        "applicationScripts": js_files,
        "opportunities": compact_opportunities,
        "apiCandidates": compact_apis,
        "apiIntelligence": api_intelligence,
        "requestHeaderIntelligence": request_header_intelligence,
        "businessEntrypoints": business_entries,
        "routeCandidates": compact_routes,
        "sensitiveCandidates": compact_sensitive,
        "runtimeSignals": runtime_signals.into_iter().take(runtime_limit).collect::<Vec<_>>(),
        "runtimeHookRecommended": runtime_hook_recommended,
        "runtimeHookPlan": runtime_hook_plan,
        "aiFallback": ai_fallback,
        "verificationPlan": {
            "strategy": "opportunity-guided-bounded-validation",
            "primaryCandidate": primary_candidate,
            "maxApiCandidates": api_limit,
            "maxAttemptsPerCandidate": if route.surface == "framework_application" { 2 } else { 3 },
            // A skipped/manual route is deterministic recon-only. Do not
            // advertise a fallback that can start model-side discovery after
            // the investigation gate has closed.
            "boundedFallbackDiscoveryAllowed": route.surface != "static_frontend" && route.mode != "skip" && route.mode != "manual_review",
            "maxFallbackDiscoveryPasses": if route.surface != "static_frontend" && route.mode != "skip" && route.mode != "manual_review" { 1 } else { 0 },
            "completionPolicy": "finish_after_bounded_plan_even_without_confirmed_finding",
            "requireFreshRequestResponseEvidence": true,
            "frameworkInventoryAlreadyComplete": route.surface == "framework_application"
        },
        "stopRule": stop_rule
    });
    trim_evidence_to_budget(&mut evidence, max_bytes.clamp(1024, 64 * 1024));
    evidence
}
