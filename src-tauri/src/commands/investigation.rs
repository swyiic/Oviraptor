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









#[allow(clippy::too_many_arguments)]
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

#[allow(clippy::too_many_arguments)]
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
include!("investigation_surface.rs");
include!("investigation_persist.rs");
include!("investigation_deep_dive.rs");
include!("investigation_read.rs");
include!("investigation_replay.rs");
include!("investigation_tests.rs");
