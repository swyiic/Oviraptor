fn path_segment_ok(word: &str) -> bool {
    let mut chars = word.chars();
    let Some(first) = chars.next() else { return false };
    first.is_ascii_alphanumeric()
        && word.len() >= 3
        && word.len() <= 64
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

fn naming_seeds(evidence: &JsonValue, target_url: &str) -> Vec<String> {
    let mut seeds = Vec::new();
    if let Ok(url) = reqwest::Url::parse(target_url) {
        if let Some(host) = url.host_str() {
            for label in host.split('.') {
                if label.len() >= 2 && !matches!(label, "www" | "com" | "cn" | "net" | "org") {
                    seeds.push(label.to_string());
                }
            }
        }
    }
    for pointer in ["/apiCandidates", "/routeCandidates", "/businessEntrypoints", "/opportunities", "/investigation/apis", "/investigation/actions"] {
        let Some(items) = evidence.pointer(pointer).and_then(JsonValue::as_array) else { continue };
        for item in items {
            for key in ["path", "url", "endpoint", "name", "text"] {
                if let Some(value) = item.get(key).and_then(JsonValue::as_str) {
                    let trimmed = value.trim();
                    if (3..=80).contains(&trimmed.chars().count()) {
                        seeds.push(trimmed.to_string());
                    }
                }
            }
        }
    }
    seeds.sort();
    seeds.dedup();
    seeds.truncate(30);
    seeds
}

fn naming_model_text(profile: &GatewayProfile, prompt: &str) -> Result<String, String> {
    let body = serde_json::json!({
        "model": profile.model,
        "messages": [
            {"role":"system","content":"你只把给定的域名或接口名改写成路径段。只输出 JSON {\"paths\":[\"...\"]}。不要发明与这些名字无关的目录。"},
            {"role":"user","content":prompt}
        ],
        "temperature": 0.1,
        "max_tokens": 400,
        "stream": false
    });
    let request_body = serde_json::to_vec(&body).map_err(|error| error.to_string())?;
    let mut request = TransportRequest::post(&profile.endpoint, request_body).header("content-type", "application/json");
    if let Some(bearer) = profile.bearer() {
        request = request.header("authorization", format!("Bearer {bearer}"));
    }
    request.total_timeout = Some(Duration::from_secs(30));
    request.proxy = profile.proxy.clone();
    request.reject_redirects = crate::agent_host_sidecar::is_model_bridge_endpoint(&profile.endpoint);
    let response = transport::send(request, std::sync::Arc::new(|| false), Box::new(|_| true)).map_err(|error| match error {
        TransportError::Setup(message)
        | TransportError::Network(message)
        | TransportError::Timeout(message)
        | TransportError::Interrupted(message) => message,
        TransportError::Cancelled(_) => "naming_model_cancelled".into(),
    })?;
    if !(200..300).contains(&response.status) {
        return Err("naming_model_http".into());
    }
    let parsed: JsonValue = serde_json::from_slice(&response.body).map_err(|error| error.to_string())?;
    match parsed.pointer("/choices/0/message/content") {
        Some(JsonValue::String(value)) if !value.trim().is_empty() => Ok(value.clone()),
        _ => Err("naming_model_empty".into()),
    }
}

fn expand_naming_paths(connection: &rusqlite::Connection, seeds: &[String]) -> Vec<String> {
    if seeds.is_empty() {
        return Vec::new();
    }
    let Ok(profile) = crate::model_configuration::gateway_profile_on(connection, None) else {
        return Vec::new();
    };
    let prompt = format!(
        "这些名字来自已授权目标的域名或已观察到的接口/页面名：{}。只把它们改写成路径段：拼音、中文首字母、英文单词。不要发明无关目录，不要解释。只输出 JSON：{{\"paths\":[\"...\"]}}",
        seeds.join(" | ")
    );
    let Ok(text) = naming_model_text(&profile, &prompt) else {
        return Vec::new();
    };
    let Some(start) = text.find('{') else { return Vec::new() };
    let Some(end) = text.rfind('}') else { return Vec::new() };
    let Ok(value) = serde_json::from_str::<JsonValue>(&text[start..=end]) else { return Vec::new() };
    value.get("paths").and_then(JsonValue::as_array).map(|rows| {
        rows.iter().filter_map(JsonValue::as_str).map(|word| word.trim().trim_matches('/').to_ascii_lowercase()).filter(|word| path_segment_ok(word)).take(24).collect()
    }).unwrap_or_default()
}

fn agent_tool_targeted_discovery(
    context: &AgentRunContext,
    runtime: &mut AgentToolRuntime,
    arguments: &JsonValue,
) -> JsonValue {
    let planned = context.execution_plan.discovery_passes.max(0) as usize;
    if runtime.discovery_rounds >= planned {
        return serde_json::json!({
            "error": format!("定向发现已用完计划内 {planned} 轮"),
            "code": "discovery_budget_exhausted",
        });
    }
    let identity_key = value_first(arguments, &["identity"]);
    let method = value_first(arguments, &["method"]).to_ascii_uppercase();
    let method = if method.is_empty() { "GET".to_string() } else { method };
    if !AGENT_READ_METHODS.contains(&method.as_str()) {
        return serde_json::json!({
            "error": "发现轮次只允许只读方法",
            "code": "mutation_requires_contract",
        });
    }
    let vocabulary = agent_evidence_vocabulary(&context.evidence);
    let mut rejected = Vec::new();
    let mut accepted = Vec::new();
    for word in arguments
        .get("words")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(JsonValue::as_str)
    {
        let cleaned = word.trim().to_ascii_lowercase();
        if cleaned.len() < 3 || cleaned.len() > 64 {
            rejected.push(format!("{word}：长度无效"));
            continue;
        }
        if !vocabulary.contains(&cleaned) {
            rejected.push(format!("{word}：不是证据中出现的业务词、路由或调用点"));
            continue;
        }
        accepted.push(cleaned);
    }
    if let Ok(connection) = db::open(&context.db_path) {
        let seeds = naming_seeds(&context.evidence, &context.target_url);
        for word in expand_naming_paths(&connection, &seeds) {
            if !accepted.contains(&word) {
                accepted.push(word);
            }
        }
        if !seeds.is_empty() {
            record_pending_observation(
                &connection,
                &context.scan_id,
                "路径命名习惯",
                &format!("由这些域名或接口名改写路径，待复核：{}", seeds.join(" | ")),
            );
        }
    }
    if accepted.is_empty() {
        return serde_json::json!({
            "status": "insufficient_evidence",
            "reason": "没有可探测的路径。需要证据里的业务词，或云端模型能根据域名和接口名改写出拼音、首字母、英文路径",
            "rejected": rejected,
        });
    }
    let Ok(base) = reqwest::Url::parse(&context.target_url) else {
        return serde_json::json!({"error": "目标 URL 无法解析", "code": "invalid_url"});
    };
    runtime.discovery_rounds += 1;
    let family = value_first(arguments, &["family"]);
    let mut added = Vec::new();
    let mut different = Vec::new();
    let mut noise = Vec::new();
    for word in accepted.iter().take(24) {
        let mut candidate = base.clone();
        candidate.set_path(&format!("/{word}"));
        let path = format!("/{word}");
        if investigation_background_noise(&serde_json::json!({"url": candidate.as_str(), "method": method}))
        {
            noise.push(format!("{method} {path}：静态资源或遥测"));
            continue;
        }
        let already_seen = runtime.touched(&method, &path);
        match agent_http_request(
            context,
            runtime,
            &identity_key,
            &method,
            candidate.as_str(),
            Vec::new(),
            None,
            None,
            "",
            &family,
            ScopeSource::DiscoveryProbe,
            "targeted_discovery",
        ) {
            Ok(response) => {
                if !already_seen {
                    added.push(format!("{method} {path}"));
                }
                if response.get("status").and_then(JsonValue::as_i64).unwrap_or(0) != 404 {
                    different.push(serde_json::json!({
                        "path": path,
                        "status": response.get("status"),
                        "contentType": value_first(&response, &["contentType"]),
                        "bytes": response.get("bytes"),
                    }));
                }
                if runtime.protected_stop.is_some() {
                    break;
                }
            }
            Err(error) => {
                if value_first(&error, &["code"]) == "protected" {
                    break;
                }
                noise.push(format!("{method} {path}：{}", value_first(&error, &["error"])));
            }
        }
    }
    if let Some(stop) = &runtime.protected_stop {
        return serde_json::json!({"code": "protected", "stop": stop, "newEndpoints": added});
    }
    runtime.last_progress.new_response_shapes += different.len();
    if runtime.credit_family(&family) {
        runtime.last_progress.new_families += 1;
    }
    // A round that found nothing new closes that round only: it never makes the
    // discovery family `covered` (§9.3).
    runtime.credit_coverage(&family, "discovery", "", &[]);
    serde_json::json!({
        "round": runtime.discovery_rounds,
        "plannedRounds": planned,
        "newEndpoints": added,
        "differentResponses": different,
        "noIncrement": added.is_empty() && different.is_empty(),
        "rejectedWords": rejected,
        "skippedNoise": noise.iter().take(12).collect::<Vec<_>>(),
        "note": "UNKNOWN method、遥测、图片、字体、埋点、广告、Sentry、设备指纹和静态资源不会进入正式 API",
    })
}

#[cfg(test)]
mod naming_path_tests {
    use super::path_segment_ok;

    #[test]
    fn accepts_pinyin_and_initials_only() {
        assert!(path_segment_ok("yonghuguanli"));
        assert!(path_segment_ok("yhgl"));
        assert!(!path_segment_ok("../etc"));
        assert!(!path_segment_ok("用户"));
    }
}
