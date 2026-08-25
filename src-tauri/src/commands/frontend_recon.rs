#[derive(Clone, Debug)]
struct FrontendRoute {
    url: String,
    score: i64,
    mode: String,
    surface: String,
    reasons: Vec<String>,
}

impl FrontendRoute {
    fn fallback(url: &str, _adaptive: &AdaptiveStrixSettings, reason: &str) -> Self {
        Self {
            url: url.to_string(),
            score: 0,
            mode: "skip".into(),
            surface: "unknown".into(),
            reasons: vec![reason.into()],
        }
    }

    fn as_json(&self) -> JsonValue {
        serde_json::json!({
            "url": self.url,
            "valueScore": self.score,
            "scanMode": self.mode,
            "surface": self.surface,
            "reasons": self.reasons,
        })
    }

    fn reason_text(&self) -> String {
        self.reasons.join("；")
    }
}

fn annotate_local_full_power_routes(routes: &mut [FrontendRoute]) {
    for route in routes {
        if route.mode == "skip" {
            continue;
        }
        route.reasons.push(if route.surface == "framework_application" {
            "本地模型火力全开：复杂前端仍只验证最高价值机会，不恢复全站探索".into()
        } else {
            "本地模型火力全开：按自适应模式放宽时长、Token 与请求预算，同时保留无进展和绝对上限熔断".into()
        });
    }
}

fn json_array_len(value: &JsonValue, key: &str) -> usize {
    value
        .get(key)
        .and_then(JsonValue::as_array)
        .map(Vec::len)
        .unwrap_or(0)
}

fn actionable_api_candidate(value: &JsonValue) -> bool {
    let method = value_first(value, &["method"]).to_ascii_uppercase();
    if !matches!(method.as_str(), "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS")
        || value.get("candidateOnly").and_then(JsonValue::as_bool).unwrap_or(false)
    {
        return false;
    }
    let confidence = value
        .get("confidence")
        .and_then(JsonValue::as_str)
        .unwrap_or("");
    let engine = value
        .get("extractionEngine")
        .and_then(JsonValue::as_str)
        .unwrap_or("");
    let verification = value.get("verification").unwrap_or(&JsonValue::Null);
    if verification.get("sameOrigin").and_then(JsonValue::as_bool) == Some(false) {
        return false;
    }
    if matches!(
        verification.get("reason").and_then(JsonValue::as_str),
        Some("spa_fallback" | "html_response" | "candidate_not_resolved")
    ) {
        return false;
    }
    if verification.get("verified").and_then(JsonValue::as_bool) == Some(true) {
        return true;
    }
    if confidence != "high" {
        return false;
    }
    if engine == "browser-runtime" {
        return true;
    }
    if !matches!(
        engine,
        "babel-ast" | "babel-ast-xhr" | "jsluice-tree-sitter"
    ) {
        return false;
    }
    let endpoint = value_first(value, &["url", "path"]).to_ascii_lowercase();
    matches!(method.as_str(), "POST" | "PUT" | "PATCH" | "DELETE")
        || [
            "login", "signin", "auth", "oauth", "token", "session", "register", "signup", "admin",
            "upload", "payment", "order", "graphql", "export",
        ]
        .iter()
        .any(|keyword| endpoint.contains(keyword))
}

fn model_ready_opportunity(value: &JsonValue) -> bool {
    let method = value_first(value, &["method"]).to_ascii_uppercase();
    let stage = value
        .pointer("/readiness/stage")
        .and_then(JsonValue::as_str)
        .unwrap_or("");
    matches!(method.as_str(), "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS")
        && stage == "agent_ready"
        && value
            .pointer("/riskEvidence/present")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false)
        && !value.get("candidateOnly").and_then(JsonValue::as_bool).unwrap_or(false)
}

fn score_frontend_target(
    target: &JsonValue,
    requested_url: &str,
    adaptive: &AdaptiveStrixSettings,
) -> FrontendRoute {
    let url = value_first(target, &["url", "finalUrl"]);
    let url = if url.trim().is_empty() {
        requested_url.to_string()
    } else {
        url
    };
    let status = target
        .get("statusCode")
        .and_then(JsonValue::as_i64)
        .unwrap_or(0);
    let errors = json_array_len(target, "errors");
    if status == 0 && errors > 0 {
        return FrontendRoute {
            url,
            score: 0,
            mode: "skip".into(),
            surface: "unreachable".into(),
            reasons: vec!["入口无法访问，未取得可分析前端内容".into()],
        };
    }

    let mut score = 0i64;
    let mut reasons = Vec::new();
    if (200..400).contains(&status) {
        score += 10;
        reasons.push(format!("入口可访问（HTTP {status}）"));
    } else if matches!(status, 401 | 403) {
        score += 5;
        reasons.push(format!("入口受限但在线（HTTP {status}）"));
    } else if status >= 400 {
        reasons.push(format!("入口响应 HTTP {status}"));
    }
    let opportunities = target
        .get("opportunities")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let high_opportunity_count = opportunities
        .iter()
        .filter(|item| model_ready_opportunity(item))
        .filter(|item| item.get("score").and_then(JsonValue::as_i64).unwrap_or(0) >= 70)
        .count();
    let max_opportunity_score = opportunities
        .iter()
        .filter(|item| model_ready_opportunity(item))
        .filter_map(|item| item.get("score").and_then(JsonValue::as_i64))
        .max()
        .unwrap_or(0);
    if high_opportunity_count > 0 {
        score += (high_opportunity_count as i64 * 12).min(36);
        reasons.push(format!(
            "确定性侦察形成 {high_opportunity_count} 个 70 分以上机会（最高 {max_opportunity_score} 分）"
        ));
    }

    let scripts = target
        .get("jsFiles")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let application_count = scripts
        .iter()
        .filter(|item| {
            item.get("type")
                .and_then(JsonValue::as_str)
                .is_some_and(|kind| matches!(kind, "application" | "plugin"))
                && item
                    .get("statusCode")
                    .and_then(JsonValue::as_i64)
                    .unwrap_or(0)
                    < 400
        })
        .count();
    let chunk_count = scripts
        .iter()
        .filter(|item| item.get("type").and_then(JsonValue::as_str) == Some("chunk"))
        .count();
    let source_maps = scripts
        .iter()
        .filter(|item| {
            item.pointer("/analysis/sourceMapReference")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false)
        })
        .count();
    if application_count > 0 {
        reasons.push(format!(
            "{application_count} 个自定义业务脚本（仅作前端清单，不触发 Strix）"
        ));
    }
    if chunk_count > 0 {
        reasons.push(format!("{chunk_count} 个应用分包（数量不作为扫描价值）"));
    }
    if source_maps > 0 {
        score += 20;
        reasons.push(format!("{source_maps} 个 SourceMap 线索"));
    }

    let frontend = target
        .pointer("/fingerprint/frontend")
        .unwrap_or(&JsonValue::Null);
    let framework = value_first(frontend, &["framework", "name"]);
    let confidence = value_first(frontend, &["confidence"]);
    if !framework.is_empty() && framework != "Unknown" {
        reasons.push(format!(
            "识别到 {framework}（{confidence}；框架本身不触发 Strix）"
        ));
    }
    let api_count = json_array_len(target, "apis");
    let api_candidates = target
        .get("apiCandidates")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let candidate_count = api_candidates.len();
    let mut all_api_records = target
        .get("apis")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    all_api_records.extend(api_candidates.clone());
    let actionable_candidate_count = all_api_records
        .iter()
        .filter(|candidate| actionable_api_candidate(candidate))
        .count();
    let route_count = json_array_len(target, "routes");
    if api_count > 0 {
        reasons.push(format!("{api_count} 个已验证/保留 API 线索"));
    }
    if candidate_count > 0 {
        reasons.push(format!(
            "{candidate_count} 个待验证 API 候选（低置信数量不加分）"
        ));
    }
    if actionable_candidate_count > 0 {
        score += (actionable_candidate_count as i64 * 15).min(45);
        reasons.push(format!(
            "{actionable_candidate_count} 个可执行定向验证的真实/高置信接口"
        ));
    }
    if route_count > 0 {
        reasons.push(format!("{route_count} 个前端路由（路由数量不触发 Strix）"));
    }

    let sensitive = target
        .get("sensitiveInfo")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let high_sensitive = sensitive
        .iter()
        .filter(|item| item.get("severity").and_then(JsonValue::as_str) == Some("high"))
        .count();
    let medium_sensitive = sensitive.len().saturating_sub(high_sensitive);
    if high_sensitive > 0 {
        score += (high_sensitive as i64 * 20).min(40);
        reasons.push(format!("{high_sensitive} 个高风险敏感信息线索"));
    }
    if medium_sensitive > 0 {
        reasons.push(format!(
            "{medium_sensitive} 个中风险信息线索（仅展示，不单独触发 Strix）"
        ));
    }

    let registration_count = json_array_len(target, "registrationEntrypoints");
    let form_count = json_array_len(target, "forms");

    let route_corpus = format!(
        "{} {} {} {}",
        target.get("apis").unwrap_or(&JsonValue::Null),
        target.get("apiCandidates").unwrap_or(&JsonValue::Null),
        target.get("routes").unwrap_or(&JsonValue::Null),
        target.get("forms").unwrap_or(&JsonValue::Null)
    )
    .to_ascii_lowercase();
    let has_business_entry = [
        "login",
        "oauth",
        "auth",
        "admin",
        "upload",
        "payment",
        "order",
        "graphql",
        "websocket",
        "export",
        "download",
        "token",
        "session",
        "register",
        "signup",
    ]
    .iter()
    .any(|keyword| route_corpus.contains(keyword));
    let concrete_business_entry = has_business_entry
        && (actionable_candidate_count > 0 || registration_count > 0 || form_count > 0);
    if concrete_business_entry {
        score += 15;
        reasons.push("存在接口或表单支撑的鉴权、注册、管理、上传等业务入口".into());
    } else if has_business_entry {
        reasons.push("仅发现业务名称/路由文本，没有可验证接口或表单".into());
    }
    let ai_fallback_enabled = target
        .pointer("/aiFallback/enabled")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    if ai_fallback_enabled {
        reasons.push("已准备有限代码切片；AI fallback 本身不触发 Strix".into());
    }

    let lower_url = url.to_ascii_lowercase();
    let static_host = [
        "image.",
        "images.",
        "img.",
        "static.",
        "cdn.",
        "file.",
        "files.",
        "download.",
    ]
    .iter()
    .any(|token| lower_url.contains(token));
    if static_host
        && application_count == 0
        && api_count == 0
        && candidate_count == 0
        && route_count == 0
    {
        score -= 25;
        reasons.push("静态资源/文件域名且没有业务前端证据".into());
    }
    if scripts.is_empty()
        && api_count == 0
        && candidate_count == 0
        && route_count == 0
        && sensitive.is_empty()
    {
        score -= 15;
        reasons.push("没有脚本、接口、路由或敏感线索".into());
    }
    if registration_count > 0 {
        score += (registration_count as i64 * 15).min(30);
        reasons.push(format!("{registration_count} 个注册或账户创建入口"));
    }
    let framework_name = framework.to_ascii_lowercase();
    let complex_framework = [
        "vue", "react", "angular", "svelte", "preact", "solid", "next.js", "nuxt",
    ]
    .iter()
    .any(|name| framework_name.contains(name));
    let empty_surface = scripts.is_empty()
        && api_count == 0
        && candidate_count == 0
        && route_count == 0
        && sensitive.is_empty()
        && registration_count == 0
        && json_array_len(target, "forms") == 0
        && json_array_len(target, "links") == 0
        && json_array_len(target, "runtimeSignals") == 0;
    let static_frontend = empty_surface;
    let ordinary_web = !complex_framework
        && !empty_surface
        && ((200..400).contains(&status) || matches!(status, 401 | 403))
        && !static_host;
    score = score.clamp(0, 100);
    let mut mode = adaptive.mode_for_score(score).to_string();
    let surface = if static_frontend {
        mode = "skip".into();
        reasons.push("现代前端硬门控：没有真实/高置信接口、业务表单、SourceMap 或高风险敏感线索，不启动 Strix".into());
        "static_frontend"
    } else if ordinary_web {
        // Ordinary pages follow the same evidence gate. With no high-value
        // opportunity, quick mode is enough for one bounded directory/API
        // fallback; a plain login page must never receive the main budget.
        if high_opportunity_count == 0 {
            mode = "quick".into();
        }
        reasons.push(if high_opportunity_count > 0 {
            "普通服务端 Web：按机会分值选择有限验证预算".into()
        } else {
            "普通服务端 Web：没有高价值机会，仅允许 quick 与一次性兜底发现".into()
        });
        "ordinary_web"
    } else if complex_framework {
        // The browser explorer and AST pass now produce a bounded evidence
        // packet. Framework applications may enter a short verification run,
        // but only when that packet contains an actionable opportunity.
        if high_opportunity_count == 0 {
            mode = "skip".into();
            reasons.push("复杂前端框架：没有 70 分以上机会；首次基线可做一次有界接口/目录兜底，已有基线则本地停止".into());
        } else {
            if mode == "deep" && max_opportunity_score < 85 {
                mode = "standard".into();
            }
            if mode == "skip" {
                mode = "quick".into();
            }
            reasons.push("复杂前端框架：只把高价值机会的有限证据包交给 Strix 定向验证".into());
        }
        "framework_application"
    } else {
        "application"
    };
    mode = adaptive.bounded_mode(&mode);
    if reasons.is_empty() {
        reasons.push("未发现足够的前端价值信号".into());
    }
    FrontendRoute {
        url,
        score,
        mode,
        surface: surface.into(),
        reasons,
    }
}

fn frontend_routes(
    recon_path: &Path,
    urls: &[String],
    adaptive: &AdaptiveStrixSettings,
) -> Vec<FrontendRoute> {
    let recon = fs::read(recon_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<JsonValue>(&bytes).ok())
        .unwrap_or_default();
    let targets = recon
        .get("targets")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    urls.iter()
        .map(|url| {
            targets
                .iter()
                .find(|target| {
                    let candidate = value_first(target, &["url", "finalUrl"]);
                    asset_match_keys(&candidate)
                        .iter()
                        .any(|key| asset_match_keys(url).contains(key))
                })
                .map(|target| score_frontend_target(target, url, adaptive))
                .unwrap_or_else(|| {
                    FrontendRoute::fallback(url, adaptive, "前端解析结果缺失，本次不启动 Strix")
                })
        })
        .collect()
}

fn evidence_priority(value: &JsonValue) -> i64 {
    let text = value
        .get("url")
        .or_else(|| value.get("path"))
        .and_then(JsonValue::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    let keyword_score = [
        "admin",
        "auth",
        "login",
        "oauth",
        "token",
        "session",
        "upload",
        "export",
        "download",
        "payment",
        "order",
        "graphql",
        "websocket",
        "debug",
    ]
    .iter()
    .filter(|keyword| text.contains(**keyword))
    .count() as i64
        * 10;
    let method_score = match value
        .get("method")
        .and_then(JsonValue::as_str)
        .unwrap_or("")
    {
        "POST" | "PUT" | "PATCH" | "DELETE" => 6,
        _ => 0,
    };
    let confidence_score = match value
        .get("confidence")
        .and_then(JsonValue::as_str)
        .unwrap_or("")
    {
        "high" => 4,
        "medium" => 2,
        _ => 0,
    };
    keyword_score + method_score + confidence_score
}

fn script_evidence_priority(value: &JsonValue) -> i64 {
    let url = value_first(value, &["url"]).to_ascii_lowercase();
    let business_score = value
        .pointer("/analysis/businessScore")
        .and_then(JsonValue::as_i64)
        .unwrap_or(0);
    let source_map = value
        .pointer("/analysis/sourceMapReference")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let named_entry = [
        "main.", "main-", "app.", "app-", "index.", "index-", "entry.", "entry-",
    ]
    .iter()
    .any(|part| url.contains(part));
    business_score * 10 + i64::from(source_map) * 20 + i64::from(named_entry) * 10
}

fn runtime_signal_priority(value: &JsonValue) -> i64 {
    match value.get("type").and_then(JsonValue::as_str).unwrap_or("") {
        "runtime_hook_plan" => 100,
        "network_runtime" => 40,
        "browser_storage" | "route_runtime" => 30,
        "anti_debug" => 10,
        _ => 0,
    }
}

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
                "/workspace/strix-evidence-input/auth-session.json"
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
    let configured = settings
        .get("strixFrontendPacketBudgetKb")
        .and_then(JsonValue::as_u64)
        .unwrap_or(24)
        .clamp(4, 64) as usize;
    let budget = match settings
        .get("strixFrontendPacketMode")
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
    while serde_json::to_vec(evidence).map_or(false, |bytes| bytes.len() > max_bytes) {
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
    if serde_json::to_vec(evidence).map_or(false, |bytes| bytes.len() > max_bytes) {
        if let Some(fallback) = evidence
            .get_mut("aiFallback")
            .and_then(JsonValue::as_object_mut)
        {
            fallback.insert("snippets".into(), serde_json::json!([]));
            fallback.insert("sliceIndex".into(), serde_json::json!([]));
        }
    }
    if serde_json::to_vec(evidence).map_or(false, |bytes| bytes.len() > max_bytes) {
        if let Some(object) = evidence.as_object_mut() {
            object.insert("localAnalysis".into(), serde_json::json!({}));
            object.insert("techStack".into(), serde_json::json!({}));
            object.insert("fingerprint".into(), serde_json::json!({}));
        }
    }
    if serde_json::to_vec(evidence).map_or(false, |bytes| bytes.len() > max_bytes) {
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
    if serde_json::to_vec(evidence).map_or(false, |bytes| bytes.len() > max_bytes) {
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
/// on disk for the result viewer; Strix receives only high-value candidates.
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
        .map_or(false, |plan| !plan.is_empty());
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

fn frontend_recon_target(recon_path: &Path, url: &str) -> Option<JsonValue> {
    let recon = fs::read(recon_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<JsonValue>(&bytes).ok())?;
    recon
        .get("targets")
        .and_then(JsonValue::as_array)
        .and_then(|targets| {
            targets.iter().find(|target| {
                let candidate = value_first(target, &["url", "finalUrl"]);
                asset_match_keys(&candidate)
                    .iter()
                    .any(|key| asset_match_keys(url).contains(key))
            })
        })
        .cloned()
}

fn bounded_utf8_bytes(value: &str, limit: usize) -> &str {
    if value.len() <= limit {
        return value;
    }
    let mut end = limit;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn frontend_slice_manifest(
    index: &[JsonValue],
    max_total_bytes: usize,
    available_slice_bytes: usize,
) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "policy": {
            "sliceArtifactMaxBytes": max_total_bytes,
            "maxReads": 3,
            "maxBytesPerRead": 14_000,
            "maxCumulativeReadBytes": max_total_bytes,
            "availableSliceBytes": available_slice_bytes,
            "stopAfterUnproductiveReads": 2,
            "readOnly": true,
            "completeBundleAvailable": false,
            "preferredKinds": ["dependency-definition", "http-client", "network-call", "business-flow", "marker-window"]
        },
        "slices": index,
    }))
    .unwrap_or_default()
}

fn write_frontend_code_slices(
    target: &JsonValue,
    target_dir: &Path,
    max_total_bytes: usize,
) -> usize {
    let Some(slices) = target
        .pointer("/aiFallback/codeSlices")
        .and_then(JsonValue::as_array)
    else {
        return 0;
    };
    let slices_dir = target_dir.join("frontend-code-slices");
    let mut index = Vec::new();
    let mut files = Vec::<(String, Vec<u8>)>::new();
    let mut seen_ids = HashSet::new();
    let mut total_slice_bytes = 0usize;
    for slice in slices.iter().take(8) {
        let id = bounded_text(slice, &["id"], 32)
            .chars()
            .filter(|value| value.is_ascii_alphanumeric() || matches!(value, '-' | '_'))
            .collect::<String>();
        let context = value_first(slice, &["context"]);
        if id.is_empty()
            || !seen_ids.insert(id.clone())
            || context.trim().is_empty()
            || total_slice_bytes >= max_total_bytes
        {
            continue;
        }
        let file_name = format!("{id}.js");
        let header = format!(
            "/* Oviraptor bounded code slice; source: {}; range: {}-{} */\n",
            bounded_text(slice, &["source"], 700),
            slice.get("start").and_then(JsonValue::as_i64).unwrap_or(0),
            slice.get("end").and_then(JsonValue::as_i64).unwrap_or(0),
        );
        let mut body = bounded_utf8_bytes(&context, context.len().min(14_000)).to_string();
        let mut accepted = None;
        while body.len() >= 200 {
            let content = format!("{header}{body}\n").into_bytes();
            let entry = serde_json::json!({
                "id": id,
                "file": format!("frontend-code-slices/{file_name}"),
                "source": bounded_text(slice, &["source"], 700),
                "kind": bounded_text(slice, &["kind"], 80),
                "marker": bounded_text(slice, &["marker"], 120),
                "start": slice.get("start").and_then(JsonValue::as_i64).unwrap_or(0),
                "end": slice.get("end").and_then(JsonValue::as_i64).unwrap_or(0),
                "bytes": body.len(),
            });
            let mut trial_index = index.clone();
            trial_index.push(entry.clone());
            let manifest = frontend_slice_manifest(
                &trial_index,
                max_total_bytes,
                total_slice_bytes + content.len(),
            );
            let projected = total_slice_bytes + content.len() + manifest.len();
            if projected <= max_total_bytes {
                accepted = Some((content, entry));
                break;
            }
            let overflow = projected.saturating_sub(max_total_bytes).max(1);
            if body.len() <= 200 + overflow {
                break;
            }
            let next_len = body.len() - overflow;
            body = bounded_utf8_bytes(&body, next_len).to_string();
        }
        if let Some((content, entry)) = accepted {
            total_slice_bytes += content.len();
            files.push((file_name, content));
            index.push(entry);
        }
    }
    if index.is_empty() || fs::create_dir_all(&slices_dir).is_err() {
        return 0;
    }
    let manifest = frontend_slice_manifest(&index, max_total_bytes, total_slice_bytes);
    let mut written = 0usize;
    for (file_name, content) in files {
        if fs::write(slices_dir.join(file_name), &content).is_ok() {
            written += content.len();
        }
    }
    if fs::write(target_dir.join("frontend-code-index.json"), &manifest).is_ok() {
        written += manifest.len();
    }
    written
}

fn write_frontend_evidence(
    recon_path: &Path,
    url: &str,
    target_dir: &Path,
    route: &FrontendRoute,
    packet_budget: usize,
    db_path: Option<&Path>,
    scan_id: &str,
) {
    let target = frontend_recon_target(recon_path, url).unwrap_or_else(
        || serde_json::json!({"url": url, "errors": ["frontend recon result unavailable"]}),
    );
    let local_knowledge_matches = db_path
        .and_then(|path| db::open(path).ok())
        .map(|connection| {
            let project_id = connection
                .query_row(
                    "SELECT project_id FROM sentinel_scans WHERE id=?1",
                    [scan_id],
                    |row| row.get::<_, Option<i64>>(0),
                )
                .optional()
                .ok()
                .flatten()
                .flatten();
            let mut matches = Vec::new();
            let mut seen = HashSet::new();
            for opportunity in target
                .get("opportunities")
                .and_then(JsonValue::as_array)
                .into_iter()
                .flatten()
            {
                if let Ok(items) =
                    opportunity_knowledge_matches(&connection, project_id, opportunity)
                {
                    for item in items {
                        let id = item.get("id").and_then(JsonValue::as_i64).unwrap_or(0);
                        if id > 0 && seen.insert(id) {
                            matches.push(item);
                        }
                        if matches.len() >= 6 {
                            break;
                        }
                    }
                }
                if matches.len() >= 6 {
                    break;
                }
            }
            matches
        })
        .unwrap_or_default();
    let evidence_budget = packet_budget.saturating_mul(2) / 3;
    let slice_budget = packet_budget.saturating_sub(evidence_budget);
    let mut evidence = compact_frontend_evidence(&target, url, route, evidence_budget);
    let contract_limit = web_mode_contract_limit(&route.mode) as usize;
    let fallback_api_limit = match route.mode.as_str() {
        "quick" => 3,
        "deep" => 10,
        _ => 6,
    };
    if let Some(path) = db_path {
        if let Ok(connection) = db::open(path) {
            let metrics = read_investigation_metrics(&connection, scan_id, url)
                .ok()
                .flatten();
            let hypotheses = read_investigation_hypotheses(&connection, scan_id, url, "")
                .unwrap_or_default()
                .into_iter()
                // The verifier receives only concrete, model-eligible contracts.
                // Candidate/template rows remain in the local graph until the
                // deterministic collector can obtain a real request contract.
                .filter(|item| {
                    item.score >= 65
                        && matches!(item.status.as_str(), "ready" | "in_progress")
                        && item
                            .decision
                            .get("eligibleForModel")
                            .and_then(JsonValue::as_bool)
                            .unwrap_or(false)
                })
                .take(contract_limit)
                .map(|item| serde_json::json!({
                    "hypothesisKey":item.hypothesis_key,
                    "category":item.category,
                    "title":item.title,
                    "score":item.score,
                    "confidence":item.confidence,
                    "status":item.status,
                    "contract":item.contract,
                    "mutationApproval":item.mutation_approval,
                    "evidence":item.evidence,
                    "decision":item.decision,
                }))
                .collect::<Vec<_>>();
            let api_models = read_investigation_apis(&connection, scan_id, url)
                .unwrap_or_default()
                .into_iter()
                .filter(|item| item.baseline_status != "unchanged" || item.source.contains("runtime"))
                .take(8)
                .map(|item| serde_json::json!({
                    "apiKey":item.api_key,"method":item.method,"path":item.normalized_path,
                    "source":item.source,"confidence":item.confidence,"authScope":item.auth_scope,
                    "parameters":item.parameters,"requestSchema":item.request_schema,
                    "responseSchema":item.response_schema,"stateKeys":item.state_keys,
                    "actionKeys":item.action_keys,"identityKeys":item.identity_keys,
                    "baselineStatus":item.baseline_status,
                }))
                .collect::<Vec<_>>();
            let actions = read_investigation_actions(&connection, scan_id, url)
                .unwrap_or_default()
                .into_iter()
                .take(6)
                .map(|item| serde_json::json!({
                    "actionKey":item.action_key,"stateKey":item.state_key,"type":item.action_type,
                    "label":item.label,"outcome":item.outcome,"valueScore":item.value_score,
                    "protocol":item.protocol,
                }))
                .collect::<Vec<_>>();
            let identity_differences = read_investigation_identity_diffs(&connection, scan_id, url)
                .unwrap_or_default()
                .into_iter()
                .take(6)
                .map(|item| serde_json::json!({
                    "apiKey":item.api_key,"leftIdentity":item.left_identity_key,
                    "rightIdentity":item.right_identity_key,"differenceType":item.difference_type,
                    "riskScore":item.risk_score,"matrix":item.matrix,
                    "classification":"authorization_candidate_not_vulnerability",
                }))
                .collect::<Vec<_>>();
            if let Some(object) = evidence.as_object_mut() {
                let standard_allowed = metrics
                    .as_ref()
                    .and_then(|item| item.decision.get("standardInvestigationAllowed"))
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(false);
                let automation_tier = metrics
                    .as_ref()
                    .and_then(|item| item.decision.get("automationTier"))
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!("recon_only"));
                let source_guided = metrics
                    .as_ref()
                    .and_then(|item| item.decision.get("sourceGuidedInvestigationAllowed"))
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(false);
                let incremental_decision = compact_incremental_decision(
                    metrics.as_ref().map(|item| &item.decision),
                );
                let manual_deep_dive = compact_manual_deep_dive(
                    metrics.as_ref().map(|item| &item.decision),
                );
                let agent_rule = if metrics.as_ref().map(|item| item.token_worthy).unwrap_or(false) {
                    "Execute every listed eligible hypothesis sequentially without waiting for an operator. Oviraptor grants automatic bounded authorization for the contract's exact endpoint, method and maxAttempts. Perform read-only and non-destructive control/test requests directly; benign marker uploads must be cleaned up. Never perform irreversible deletion, financial transactions, external messaging or persistent account/permission changes. Mark ordinary/no-impact outcomes exhausted and continue to the next contract. Finish the target after the bounded queue is exhausted, even when no finding is confirmed."
                } else if source_guided {
                    "No risk hypothesis is asserted. The anonymous page did not naturally issue a business XHR/fetch, but Oviraptor recovered exact high-confidence GET/HEAD calls from source-map call sites. Validate only those listed source-guided apiModels with bounded read-only requests, preserve control responses, and stop at the task limit. Never execute inferred writes, placeholder URLs, or arbitrary string combinations."
                } else if standard_allowed {
                    "No risk hypothesis is asserted. Perform a bounded coverage investigation using only the listed browser-observed apiModels and current authorized session. Prefer meaningful non-telemetry APIs, obtain read-only control responses, compare status/schema/identity scope when available, and summarize covered functions. Do not expand into whole-site reconnaissance. Finish the target after this plan even if all results are normal."
                } else {
                    "The local evidence gate is closed. Preserve the deterministic reconnaissance result and finish without model-side discovery."
                };
                object.insert("investigation".into(), serde_json::json!({
                    "modelGate":metrics.as_ref().map(|item| item.token_worthy).unwrap_or(false),
                    "standardInvestigationAllowed":standard_allowed,
                    "sourceGuidedInvestigationAllowed":source_guided,
                    "automationTier":automation_tier,
                    "informationGain":metrics.as_ref().map(|item| item.information_gain).unwrap_or(0),
                    "stopReason":metrics.as_ref().map(|item| item.stop_reason.clone()).unwrap_or_default(),
                    "incrementalDecision":incremental_decision,
                    "manualDeepDive":manual_deep_dive,
                    "hypotheses":hypotheses,"apiModels":api_models,"actions":actions,
                    "identityDifferences":identity_differences,
                    "automationPolicy":{
                        "mode":"ai_auto_sequential",
                        "maxContracts":contract_limit,
                        "fallbackApiLimit":fallback_api_limit,
                        "authorizationMode":"automatic_bounded",
                        "humanReviewStage":"optional_final_review",
                        "suspiciousOnlyEscalation":true,
                    },
                    "manualReviewRule":"manualDeepDive contains deterministic coverage gaps, not findings. After the automatic queue, report the highest-priority untested leads with their missing evidence and stop condition. Never claim they are vulnerabilities and never spend model turns rediscovering them.",
                    "agentRule":agent_rule
                }));
            }
        }
    }
    if !local_knowledge_matches.is_empty() {
        if let Some(object) = evidence.as_object_mut() {
            object.insert(
                "localKnowledgeMatches".into(),
                JsonValue::Array(local_knowledge_matches),
            );
            if let Some(plan) = object
                .get_mut("verificationPlan")
                .and_then(JsonValue::as_object_mut)
            {
                plan.insert("localKnowledgeMatched".into(), JsonValue::Bool(true));
            }
        }
        trim_evidence_to_budget(&mut evidence, evidence_budget.clamp(1024, 64 * 1024));
    }
    if route.surface != "static_frontend" {
        write_frontend_code_slices(&target, target_dir, slice_budget);
    }
    if let Ok(bytes) = serde_json::to_vec(&evidence) {
        let _ = fs::write(target_dir.join("frontend-evidence.json"), bytes);
    }
}

fn approved_strix_proxies(settings: &JsonValue) -> Vec<(String, String)> {
    if !settings
        .get("strixProxyEnabled")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        return Vec::new();
    }
    settings
        .get("authorizedProxyPool")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let raw = item.as_str()?.trim();
            let (tag, url) = raw.split_once('|').unwrap_or(("ALL", raw));
            let url = url.trim();
            if !(url.starts_with("http://")
                || url.starts_with("https://")
                || url.starts_with("socks5://")
                || url.starts_with("socks5h://"))
            {
                return None;
            }
            Some((tag.trim().to_ascii_uppercase(), url.to_string()))
        })
        .collect()
}
