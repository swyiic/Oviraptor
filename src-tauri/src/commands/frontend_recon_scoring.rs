// Which frontend routes and API candidates are worth investigating, and in what
// order: route scoring, evidence priority and the actionable-candidate filters.
// Included from frontend_recon.rs.

#[derive(Clone, Debug)]
struct FrontendRoute {
    url: String,
    score: i64,
    mode: String,
    surface: String,
    reasons: Vec<String>,
}

impl FrontendRoute {
    fn fallback(url: &str, _adaptive: &AgentBudgetSettings, reason: &str) -> Self {
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
    adaptive: &AgentBudgetSettings,
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
            "{application_count} 个自定义业务脚本（仅作前端清单，不单独启动调查）"
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
            "识别到 {framework}（{confidence}；框架名称本身不启动调查）"
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
        reasons.push(format!("{route_count} 个前端路由（路由数量不单独启动调查）"));
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
            "{medium_sensitive} 个中风险信息线索（仅展示，不单独触发智能体调查）"
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
        reasons.push("已准备有限代码切片；代码切片本身不单独启动调查".into());
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
        && form_count == 0
        && json_array_len(target, "links") == 0
        && json_array_len(target, "runtimeSignals") == 0;
    // A framework name and an application bundle are inventory, not an
    // executable investigation lead.  Complex SPAs only enter the model loop
    // when recon produced a verified/high-confidence API, a business form, a
    // source map, or a high-risk sensitive signal.
    let static_frontend = empty_surface
        || (complex_framework
            && api_count == 0
            && actionable_candidate_count == 0
            && registration_count == 0
            && form_count == 0
            && source_maps == 0
            && high_sensitive == 0);
    let ordinary_web = !complex_framework
        && !empty_surface
        && ((200..400).contains(&status) || matches!(status, 401 | 403))
        && !static_host;
    score = score.clamp(0, 100);
    let mut mode = adaptive.mode_for_score(score).to_string();
    let surface = if static_frontend {
        mode = "skip".into();
        reasons.push("现代前端硬门控：没有真实/高置信接口、业务表单、SourceMap 或高风险敏感线索，不启动自动调查".into());
        "static_frontend"
    } else if ordinary_web {
        // A submitted web target is investigated. Value score only orders the
        // work; it no longer refuses to start.
        if matches!(mode.as_str(), "skip" | "quick") {
            mode = "standard".into();
        }
        reasons.push("已提交的 Web 目标直接进入调查，不因价值分跳过".into());
        "ordinary_web"
    } else if complex_framework {
        if (high_opportunity_count == 0 && matches!(mode.as_str(), "skip" | "quick"))
            || (mode == "deep"
                && max_opportunity_score < 85
                && actionable_candidate_count == 0)
        {
            mode = "standard".into();
        }
        reasons.push("复杂前端直接进入有界调查，高价值机会只决定先做哪一条".into());
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
    adaptive: &AgentBudgetSettings,
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
                    FrontendRoute::fallback(url, adaptive, "前端解析结果缺失，本次不启动自动调查")
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
