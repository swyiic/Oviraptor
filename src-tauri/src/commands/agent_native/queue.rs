/// Native agent loop (§6) and its result ingestion (§8).
///
/// Oviraptor drives the model, the tools, the budgets and the terminal state;
/// no external backend process is involved on this path.
const AGENT_SYSTEM_RULES: &str = "你是 Oviraptor 的授权资产安全调查 Agent。你只在当前任务目标与其证据记录的关联业务域名内工作，\
工具调用是唯一执行途径：不得假设未验证的响应，不得声称未做过的事。\n\
规则：\n\
1. 每轮只调用一个或一组互不重复的工具；先读证据再发请求，避免重复侦察。\n\
2. 优先执行已有的高价值契约，其次是运行时正式 API，最后才是定向发现。\n\
3. 普通 401/403 只说明权限边界，记录后继续其他分支；遇到 WAF/验证码/机器人挑战时，先在授权范围内尝试绕过——WAF：XFF/来源头伪造、大小写与编码变形、参数污染、备用原始 IP 直连；验证码：是否仅前端校验、可复用/不失效、OCR 识别、滑块或点选的可预测性与 key——仅当多次绕过均失败、确认为硬性持续拦截时，才记录 confirmed_waf_or_challenge 收口；全程不做 DoS、破坏性或不可逆操作。\n\
4. 写请求需要已授权契约、尝试次数、清理步骤与恢复条件，缺一不可。\n\
4b. 发 HTML 标记前必须先对上前端渲染点：转义或文本输出直接忽略；innerHTML/v-html/dangerouslySetInnerHTML 只记录“按 HTML 解析”，禁止自动更换绕过；只有该判断成立且操作者已确认这一次，才允许发不含事件的标记。\n\
4c. 探测被拦截时，只根据上一发请求和响应决定下一发同主机只读对比（GET/HEAD）。不要按漏洞类型套绕过清单，不要因此改发 POST。\n\
5. 只有 record_hypothesis_result 的 confirmed（含控制请求、测试请求、响应差异、影响与复现步骤）才是漏洞结论；\
   insufficient_evidence 既不是漏洞，也不算验证成功。\n\
6. 没有发现也是正常终态：必须调用 finish_target 输出覆盖账本、未覆盖原因（不得写成“安全”）、排除项与人工深入建议。\n\
7. 未覆盖的原因必须如实写入覆盖账本，不得为了结束而声称已覆盖。\n\
8. 若工具结果含 securityRelevantHeaders（如 Server/X-Powered-By/X-AspNet-Version）或匿名身份拿到业务 JSON，必须先对 information_disclosure/authorization 调用 record_hypothesis_result（能绑定控制/测试请求则 confirmed，否则 insufficient_evidence 写明缺口），再 finish_target。\n\
9. 覆盖族被标为 covered 不等于已确认漏洞；确认问题只来自 confirmed 或系统自动落库的观察型证据。\n\
10. 除登录写操作外，应优先覆盖工具结果里新挖到的同站公开路径（pendingQueue 中的 api: 项）；在仍有未访问的公开 api: 队列时不要 finish_target。\n\
11. 自检与换向：每轮先看状态块里的 noProgressStreak、coveredFamilies、confirmedFindings。若当前路数已连续数轮让 noProgressStreak 上升，而 coveredFamilies 与 confirmedFindings 都无新增，说明这条路在空转——立即放弃它，改去覆盖一个尚未覆盖的族、或对同一目标换一种技术/参数/身份，不要在同一处反复重试同一请求。只有当可及的族与 pendingQueue 都已实质尝试、仍无新进展时，才调用 finish_target 如实收口；绝不为凑轮数空耗预算。";

/// Which families a route still owes, used both for the initial queue and for a
/// derived terminal state.
fn path_is_static_asset(path: &str) -> bool {
    let path = path.split('?').next().unwrap_or(path).to_ascii_lowercase();
    [
        ".js", ".css", ".map", ".png", ".jpg", ".jpeg", ".gif", ".svg", ".ico", ".woff",
        ".woff2", ".ttf",
    ]
    .iter()
    .any(|suffix| path.ends_with(suffix))
}

fn agent_required_families() -> Vec<&'static str> {
    AGENT_COVERAGE_FAMILIES.to_vec()
}

fn agent_initial_queue(context: &AgentRunContext) -> Vec<String> {
    let mut queue = Vec::new();
    // §6 rule 3: existing high-value contracts first.
    for pointer in ["/investigation/hypotheses", "/opportunities"] {
        let Some(items) = context.evidence.pointer(pointer).and_then(JsonValue::as_array) else {
            continue;
        };
        for item in items {
            let status = value_first(item, &["status"]);
            if !status.is_empty() && !matches!(status.as_str(), "ready" | "candidate" | "in_progress") {
                continue;
            }
            let category = value_first(item, &["category", "kind"]);
            let endpoint = value_first(item, &["endpoint", "path", "url"]);
            if category.is_empty() || endpoint.is_empty() {
                continue;
            }
            queue.push(format!(
                "contract:{category}|{}",
                normalized_investigation_path(&endpoint)
            ));
        }
    }
    // Then the formal runtime APIs actually observed in the browser.
    if let Some(items) = context
        .evidence
        .pointer("/apiCandidates")
        .and_then(JsonValue::as_array)
    {
        for item in items.iter().take(40) {
            if investigation_background_noise(item) {
                continue;
            }
            if !standard_investigation_api(item) {
                continue;
            }
            let verb = value_first(item, &["method"]).to_ascii_uppercase();
            let path = normalized_investigation_path(&value_first(item, &["path", "url"]));
            if verb.is_empty() || path.is_empty() {
                continue;
            }
            queue.push(format!("api:{verb}|{path}"));
        }
    }
    // Login and business forms are real work even when recon did not promote
    // them to runtime API candidates. Without this they never enter the queue,
    // and the model can finish having only replayed the page itself.
    for pointer in ["/businessEntrypoints", "/forms"] {
        let Some(items) = context.evidence.pointer(pointer).and_then(JsonValue::as_array) else {
            continue;
        };
        for item in items.iter().take(40) {
            let verb = value_first(item, &["method"]).to_ascii_uppercase();
            let path = normalized_investigation_path(&value_first(item, &["path", "url"]));
            if !matches!(verb.as_str(), "GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE")
                || path.is_empty()
                || path_is_static_asset(&path)
            {
                continue;
            }
            let key = format!("api:{verb}|{path}");
            if !queue.contains(&key) {
                queue.push(key);
            }
        }
    }
    // Then the coverage families that nothing has claimed yet.
    for family in agent_required_families() {
        queue.push(format!("family:{family}"));
    }
    queue
}

fn agent_state_block(
    context: &AgentRunContext,
    runtime: &AgentToolRuntime,
    state: &NativeAgentState,
    queue: &[String],
) -> JsonValue {
    serde_json::json!({
        "role": "user",
        "oviraptorKeep": true,
        "content": serde_json::json!({
            "targetUrl": context.target_url,
            "executionPlan": context.execution_plan.as_json(),
            "identities": context.identities.iter().map(|identity| serde_json::json!({
                "key": identity.key,
                "label": agent_identity_label(&context.identities, &identity.key),
                "anonymous": identity.anonymous,
            })).collect::<Vec<_>>(),
            "budgetUsage": {
                "modelRequests": state.token_usage.model_requests,
                "uncachedInputTokens": state.token_usage.uncached_input(),
                "totalTokens": state.token_usage.total_tokens,
                "targetRequests": runtime.target_requests,
                "discoveryRounds": runtime.discovery_rounds,
                "turns": state.turns,
                "noProgressStreak": state.no_progress_streak
            },
            "coveredFamilies": runtime.families.iter().collect::<Vec<_>>(),
            "closedContracts": state.completed_contract_keys,
            "contractAttempts": queue
                .iter()
                .filter_map(|key| key.strip_prefix("contract:"))
                .map(|contract| {
                    format!(
                        "{contract}={}/{}",
                        state.attempts_for(contract),
                        agent_evidence_contract_attempts(&context.evidence, contract)
                    )
                })
                .take(24)
                .collect::<Vec<_>>(),
            "confirmedFindings": runtime.confirmed_findings,
            "observedEndpoints": runtime.endpoints.iter().take(80).collect::<Vec<_>>(),
            "pendingQueue": queue.iter().take(60).collect::<Vec<_>>(),
            "evidenceDigest": agent_evidence_digest(&context.evidence),
            "capabilities": context.capabilities,
        })
        .to_string(),
    })
}

/// Only the parts of the bundle the model cannot already have used.
fn agent_evidence_digest(evidence: &JsonValue) -> JsonValue {
    let top = |pointer: &str, count: usize| -> Vec<JsonValue> {
        evidence
            .pointer(pointer)
            .and_then(JsonValue::as_array)
            .map(|rows| {
                rows.iter()
                    .take(count)
                    .map(agent_compact_item)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    serde_json::json!({
        "surface": evidence.get("surface"),
        "publicSurface": evidence.get("multiAgentExternalSurface"),
        "valueScore": evidence.get("valueScore"),
        "verificationPlan": evidence.get("verificationPlan"),
        "stopRule": evidence.get("stopRule"),
        "apiCandidates": top("/apiCandidates", 12),
        "opportunities": top("/opportunities", 8),
        "sensitiveCandidates": top("/sensitiveCandidates", 8),
        "routeCandidates": top("/routeCandidates", 8),
        "identityDifferences": top("/investigation/identityDifferences", 8),
        "hypotheses": top("/investigation/hypotheses", 8),
    })
}

