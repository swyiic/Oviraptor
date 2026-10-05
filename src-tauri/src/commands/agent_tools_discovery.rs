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
    if accepted.is_empty() {
        return serde_json::json!({
            "status": "insufficient_evidence",
            "reason": "词表必须来自同源链接、表单、JS 调用点、已观察业务词、产品路由或本地知识库",
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
