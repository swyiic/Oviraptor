const WEB_INVESTIGATION_POLICY_SCHEMA: i64 = 4;

fn normalized_web_scan_mode(value: Option<&str>) -> &'static str {
    match value {
        Some("quick") => "quick",
        Some("deep") => "deep",
        _ => "standard",
    }
}

fn web_mode_contract_limit(mode: &str) -> i64 {
    match mode {
        "quick" => 4,
        "deep" => 24,
        _ => 12,
    }
}

fn web_mode_discovery_passes(mode: &str) -> i64 {
    match mode {
        "quick" => 1,
        "deep" => 3,
        _ => 2,
    }
}

fn web_mode_verifier_limit(mode: &str) -> i64 {
    match mode {
        "quick" => 1,
        "deep" => 3,
        _ => 2,
    }
}

fn normalized_web_skill_ids(values: &[i64]) -> Vec<i64> {
    let mut ids = values.iter().copied().filter(|value| *value > 0).collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    ids.truncate(32);
    ids
}

fn build_web_investigation_policy(
    scan_mode: Option<&str>,
    max_budget_usd: Option<f64>,
    auth_session_ids: Vec<String>,
    skill_ids: &[i64],
    instruction: &str,
    entry_point: &str,
) -> Result<JsonValue, String> {
    let mode = normalized_web_scan_mode(scan_mode);
    let instruction = instruction.trim();
    if instruction.chars().count() > 12_000 {
        return Err("Web 任务补充要求最多 12000 个字符".into());
    }
    Ok(serde_json::json!({
        "schemaVersion": WEB_INVESTIGATION_POLICY_SCHEMA,
        "policyKind": "unified-web-investigation",
        "entryPoint": entry_point,
        "webModeCeiling": mode,
        "maxBudgetUsd": max_budget_usd,
        "authSessionId": auth_session_ids.first().cloned().unwrap_or_default(),
        "authSessionIds": auth_session_ids,
        "identityComparison": auth_session_ids.len() > 1,
        "identityIsolation": "dedicated-webview-and-distinct-auth-material",
        "workflow": "coverage-led-evidence-validation",
        "defaultWorkflowSkill": "业务前端深度分析",
        "selectedSkillIds": normalized_web_skill_ids(skill_ids),
        "additionalInstruction": instruction,
        "automation": {
            "contractLimit": web_mode_contract_limit(mode),
            "discoveryPasses": web_mode_discovery_passes(mode),
            "verifierLimit": web_mode_verifier_limit(mode),
            "continueAfterNoFinding": mode != "quick",
            "attackChainCorrelation": mode == "deep",
            "manualApprovalPerReadOnlyContract": false
        }
    }))
}

fn web_capability_manifest(_settings: &JsonValue, identity_count: usize) -> JsonValue {
    serde_json::json!({
        "browserRuntime": {"available": true, "reason": "内置 CDP 运行时采集"},
        "identityDifferential": {
            "available": identity_count >= 2,
            "reason": if identity_count >= 2 { "已绑定至少两个隔离身份" } else { "需要至少两个有效且隔离的登录身份" }
        },
        "oastCallback": {
            "available": "runtime_probe",
            "mode": "builtin-local-http",
            "reason": "每个目标启动时自动创建唯一 HTTP 回连地址；实际可达性写入 src-capabilities.json"
        },
        "rawHttpProtocol": {
            "available": true,
            "adapter": "builtin-bounded-raw-http",
            "reason": "依赖零外部包的原始 TCP/HTTP 适配器随任务挂载"
        },
        "raceScheduler": {
            "available": true,
            "maxConcurrency": 64,
            "maxAttempts": 128,
            "reason": "内置有界并发调度；写请求强制要求 cleanup 与业务不变量"
        },
        "controlledWrite": {
            "available": true,
            "mode": "contract_gated",
            "reason": "按单契约自动判断；必须具备清理、回滚、次数上限和隔离测试数据"
        },
        "attackChainCorrelation": {
            "available": true,
            "reason": "基于同任务证据关联，不把指纹或普通路径单独视为漏洞"
        }
    })
}

fn web_coverage_catalog(capabilities: &JsonValue) -> JsonValue {
    let available = |path: &str| {
        capabilities
            .pointer(path)
            .and_then(JsonValue::as_bool)
            .unwrap_or(false)
    };
    serde_json::json!([
        {"key":"baseline","label":"配置、信息泄露与错误处理","status":"ready","priority":"medium","standard":"ASVS 5 / WSTG","prerequisite":"浏览器与 HTTP 响应","manualFocus":"反向代理、错误页、缓存层和非默认 Host 下的差异"},
        {"key":"authentication_session","label":"认证、会话、找回与 MFA/OAuth","status":"ready","priority":"high","standard":"ASVS 5 / WSTG","prerequisite":"真实登录、找回、绑定或联合登录流程","manualFocus":"令牌生命周期、跨端登出、重放、账号枚举和流程跳步"},
        {"key":"authorization","label":"对象、属性、功能与租户权限","status":if available("/identityDifferential/available") {"ready"} else {"partial"},"priority":"critical","standard":"OWASP API1/API3/API5","prerequisite":"至少两个隔离身份；单身份仍检查匿名边界","manualFocus":"同级账号对象归属、字段级读写、管理功能和跨租户边界"},
        {"key":"injection","label":"输入验证、注入与服务端解释器","status":"ready","priority":"high","standard":"ASVS 5 / WSTG","prerequisite":"真实接口、参数和稳定控制响应","manualFocus":"二阶数据流、异步任务、导入模板及服务端渲染链路"},
        {"key":"business_flow","label":"敏感业务流、状态机与滥用防护","status":"partial","priority":"critical","standard":"OWASP API6 / ASVS 5","prerequisite":"测试数据、业务不变量和可恢复状态","manualFocus":"跳步、重复提交、跨渠道状态差、额度/次数/顺序与审核边界"},
        {"key":"resource_consumption","label":"资源消耗、批量能力与限额","status":"partial","priority":"high","standard":"OWASP API4","prerequisite":"明确的安全速率和不会影响生产的测试窗口","manualFocus":"分页上限、复杂查询、批量导出、上传解压和异步任务配额"},
        {"key":"file_handling","label":"上传、下载、导入导出与对象存储","status":if available("/controlledWrite/available") {"ready"} else {"partial"},"priority":"high","standard":"ASVS 5 / WSTG","prerequisite":"可清理样本与受控写入契约","manualFocus":"下载授权、签名链接、文件覆盖、解析器、压缩包和跨账号附件"},
        {"key":"blind_oast","label":"SSRF、XXE、Webhook 与异步回连","status":"automatic","priority":"high","standard":"OWASP API7 / WSTG","prerequisite":"任务启动时检测目标到唯一 OAST 地址的可达性","manualFocus":"重定向、异步消费、预览/转换服务及隔离网络回连"},
        {"key":"race","label":"条件竞争与并发一致性","status":if available("/raceScheduler/available") {"ready"} else {"not_configured"},"priority":"high","standard":"ASVS 5 / WSTG","prerequisite":"并发调度、可恢复状态与业务不变量","manualFocus":"领券、库存、审批、积分、绑定、幂等键和重复消费"},
        {"key":"api_inventory","label":"影子 API、旧版本与管理端点","status":"partial","priority":"high","standard":"OWASP API9","prerequisite":"Web 运行时只覆盖实际触发和源码可追溯端点","manualFocus":"移动端、旧版、合作方、内部域名、调试接口和未触发功能"},
        {"key":"third_party","label":"第三方 API 与不可信上游数据","status":"partial","priority":"medium","standard":"OWASP API10","prerequisite":"识别服务端实际消费的第三方接口和信任边界","manualFocus":"重定向、上游响应校验、超时、内容类型和错误回退"},
        {"key":"realtime","label":"GraphQL、WebSocket、SSE 与异步消息","status":"partial","priority":"high","standard":"ASVS 5 / WSTG","prerequisite":"运行时观察到协议、订阅或消息结构","manualFocus":"订阅授权、消息对象边界、内省、重连和事件顺序"},
        {"key":"browser_trust","label":"CORS、CSRF、跨窗口与客户端信任","status":"ready","priority":"high","standard":"ASVS 5 / WSTG","prerequisite":"浏览器上下文、Origin/Referer 与状态变更请求","manualFocus":"跨源凭据、登录 CSRF、postMessage、Service Worker 和前端权限开关"},
        {"key":"http_protocol","label":"HTTP 解析、缓存与代理差异","status":if available("/rawHttpProtocol/available") {"ready"} else {"not_configured"},"priority":"medium","standard":"WSTG","prerequisite":"原始 TCP/HTTP 工具与允许直连的测试路径","manualFocus":"请求走私、缓存键、Host 路由、路径规范化和协议降级"},
        {"key":"client_supply_chain","label":"前端密钥、签名信任与依赖供应链","status":"partial","priority":"medium","standard":"ASVS 5 / WSTG","prerequisite":"业务 JS、源码映射、依赖和服务端校验事实","manualFocus":"客户端签名是否被服务端过度信任、公开配置、依赖接管与 source map 数据流"},
        {"key":"attack_chain","label":"跨证据攻击链与实际影响","status":if available("/attackChainCorrelation/available") {"ready"} else {"disabled"},"priority":"high","standard":"evidence correlation","prerequisite":"至少两个独立、可重放证据节点","manualFocus":"低危信息、权限边界和业务流能否形成可重复的高影响路径"}
    ])
}

fn web_policy_skill_ids(connection: &rusqlite::Connection, policy: &JsonValue) -> Result<Vec<i64>, String> {
    let mut ids = Vec::new();
    if let Ok(id) = connection.query_row(
        "SELECT id FROM strix_skills WHERE enabled=1 AND builtin=1 AND name='业务前端深度分析' ORDER BY id LIMIT 1",
        [],
        |row| row.get::<_, i64>(0),
    ) {
        ids.push(id);
    }
    if let Some(selected) = policy.get("selectedSkillIds").and_then(JsonValue::as_array) {
        for id in selected.iter().filter_map(JsonValue::as_i64) {
            let enabled = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM strix_skills WHERE id=?1 AND enabled=1)",
                    [id],
                    |row| row.get::<_, bool>(0),
                )
                .unwrap_or(false);
            if enabled {
                ids.push(id);
            }
        }
    }
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

fn effective_web_policy(
    connection: &rusqlite::Connection,
    policy: &JsonValue,
    settings: &JsonValue,
) -> Result<(JsonValue, String, String), String> {
    let mode = normalized_web_scan_mode(policy.get("webModeCeiling").and_then(JsonValue::as_str));
    let identity_count = investigation_strings(policy.get("authSessionIds")).len();
    let capabilities = web_capability_manifest(settings, identity_count);
    let skill_ids = web_policy_skill_ids(connection, policy)?;
    let (skill_names, skill_instructions) = strix_skill_instructions(connection, &skill_ids)?;
    let mut effective = policy.clone();
    if !effective.is_object() {
        effective = serde_json::json!({});
    }
    let object = effective.as_object_mut().expect("web policy object");
    object.insert("schemaVersion".into(), WEB_INVESTIGATION_POLICY_SCHEMA.into());
    object.insert("policyKind".into(), "unified-web-investigation".into());
    object.insert("effectiveSkillNames".into(), skill_names.clone().into());
    object.insert("coverageCatalog".into(), web_coverage_catalog(&capabilities));
    object.insert("capabilities".into(), capabilities);
    object.insert(
        "automation".into(),
        serde_json::json!({
            "contractLimit": web_mode_contract_limit(mode),
            "discoveryPasses": web_mode_discovery_passes(mode),
            "verifierLimit": web_mode_verifier_limit(mode),
            "continueAfterNoFinding": mode != "quick",
            "attackChainCorrelation": mode == "deep",
            "manualApprovalPerReadOnlyContract": false
        }),
    );
    Ok((effective, skill_names, skill_instructions))
}

fn render_web_investigation_instruction(
    policy: &JsonValue,
    skill_instructions: &str,
    local_model: bool,
) -> String {
    let mode = normalized_web_scan_mode(policy.get("webModeCeiling").and_then(JsonValue::as_str));
    let contract_limit = web_mode_contract_limit(mode);
    let discovery_passes = web_mode_discovery_passes(mode);
    let verifier_limit = web_mode_verifier_limit(mode);
    let additional = policy
        .get("additionalInstruction")
        .and_then(JsonValue::as_str)
        .unwrap_or("")
        .trim();
    let capabilities = policy.get("capabilities").cloned().unwrap_or_default();
    if local_model {
        return render_local_web_investigation_instruction(
            mode,
            contract_limit,
            discovery_passes,
            verifier_limit,
            additional,
            skill_instructions,
        );
    }
    format!(
        r#"These targets are explicitly authorized for internal defensive security testing. Oviraptor owns collection, scope, coverage state, budgets and stop decisions; Strix is the evidence-driven verifier. Preserve reproducible request/response evidence, impact, CVSS/CWE, remediation and PoC details. A finding is confirmed only when it has replayable evidence; an untested surface is never reported as safe.

Use the authoritative execution packet appended by Oviraptor before making any request. It contains the compact runtime requests, parameters, identity matrix and investigation decision; the mounted frontend-evidence.json is a fallback for delegated verifiers that do not receive the inline packet. Do not list the workspace or reread the complete recon bundle. Do not repeat framework inventory or turn public JavaScript, fingerprints, ordinary API paths or missing headers into vulnerabilities without demonstrated impact. Inferred routes remain candidates until a real request/response verifies them.

This task uses {mode} mode. Process stable eligible contracts in descending value order, up to {contract_limit} contracts for this attempt. Continue after an exhausted or no-finding branch until the cap, budget, coverage queue or a hard stop is reached. Use at most {verifier_limit} verifier agents and partition contracts between them; never let multiple agents repeat the same endpoint and hypothesis. Deep mode may correlate evidence into attack chains, but every link must remain independently evidenced. Perform up to {discovery_passes} targeted fallback discovery passes only when the coverage ledger shows an untested high-value business surface; derive words from observed business routes and never recursively brute-force unrelated paths.

The execution packet may include `investigation.manualDeepDive`. These are deterministic coverage gaps, not suspected vulnerabilities and not new autonomous discovery tasks. After completing the automatic contracts, preserve up to three highest-priority rows as human follow-up: state the observed evidence, missing prerequisite, concrete manual steps and stop condition. Never turn a missing identity, untriggered function or unavailable environment into a security finding or a claim that the surface passed.

Read-only and non-destructive contract actions are automatically authorized and require no per-request operator approval. Controlled writes are allowed only when capabilities.controlledWrite.available is true, the contract defines cleanup and rollback, and the exact endpoint and attempt count are bounded. Never perform irreversible deletion, financial settlement, external messaging, persistent account/permission changes or denial of service. Treat routine 401/403 as boundary evidence and continue other in-scope contracts. Stop active requests on confirmed WAF/bot challenge/CAPTCHA, sustained 429, or homogeneous blocking; ordinary no-difference and exhausted branches are completion states, not reasons to pause the whole task.

Read the mounted `src-capabilities.json` for the target-specific adapter paths and runtime OAST state. Treat the adapter as an executable interface: never print or read the complete `src-assurance-adapter.py` source; invoke only the exact manifest command when an eligible contract requires it. Use OAST-dependent SSRF/XXE/blind validation only when its `oast.available` is true; after sending the exact callback URL, poll `oast.pollUrl` at least twice within the contract timeout and do not wait more than 15 seconds. The built-in raw HTTP and race adapters require no package installation, but remain limited to eligible evidence contracts; race writes require cleanup and a reversible business invariant. For a runtime-unreachable capability, record `not_tested` with the network or evidence prerequisite instead of guessing, silently skipping, or claiming the surface passed.

Effective capability manifest:
{capabilities}

{skill_instructions}

## Task-specific requirements
{additional}
"#,
        capabilities = serde_json::to_string_pretty(&capabilities).unwrap_or_else(|_| "{}".into()),
        additional = if additional.is_empty() { "No additional operator requirements." } else { additional },
    )
}

/// Strix ships a large fixed tool schema and currently injects its instruction
/// twice into the first chat context.  Reusing the full Markdown skill on a
/// 49K-65K local window leaves almost no room for the first request/response
/// and tool result.  Keep the same execution contract in a dense form and
/// leave the full skill in Oviraptor's local database/UI for humans and cloud
/// models.  Task-specific operator requirements are retained verbatim within
/// a bounded tail so a local profile never silently loses per-task direction.
fn render_local_web_investigation_instruction(
    mode: &str,
    contract_limit: i64,
    discovery_passes: i64,
    verifier_limit: i64,
    additional: &str,
    skill_instructions: &str,
) -> String {
    const MAX_ADDITIONAL_CHARS: usize = 4_000;
    const MAX_SKILL_DIGEST_CHARS: usize = 1_600;
    let additional = if additional.trim().is_empty() {
        "No additional operator requirements.".to_string()
    } else {
        additional.chars().take(MAX_ADDITIONAL_CHARS).collect()
    };
    let skill_digest = local_skill_heading_digest(skill_instructions, MAX_SKILL_DIGEST_CHARS);
    format!(
        r#"Authorized internal defensive SRC assessment. Oviraptor owns scope, collection, budgets and stop decisions; Strix only verifies evidence. Never report a vulnerability without replayable request/response evidence and demonstrated impact. Never describe untested coverage as safe.

Execution plan ({mode}): use the authoritative execution packet appended by Oviraptor. The mounted `frontend-evidence.json` and `src-capabilities.json` are fallback inputs for a delegated verifier that lacks the inline packet. Do not list or search `/workspace`, read `oviraptor_recon.json`, re-crawl the site, re-enumerate bundles, repeat fingerprinting, or print adapter source. Use the exact observed method, URL, sanitized request template, baseline response and current authorized session. Runtime requests and validated AST call sites are facts; inferred strings and routes remain candidates until a real response confirms them.

Process eligible contracts by score, at most {contract_limit}; use at most {verifier_limit} non-overlapping verifiers. For each contract perform one control request and only the bounded comparison required by its evidence contract. Read-only checks are automatically authorized. A controlled write is allowed only when the capability manifest permits it and the exact contract includes cleanup, rollback and an attempt cap. Never perform irreversible deletion, real settlement, external messaging, persistent account/permission changes or denial of service.

Identity testing requires the same method, URL, request shape and business-object context from isolated A/B sessions with distinct authentication material and complete responses on both sides. Status-only or one-sided differences are not authorization findings. Anonymous scans must not invent account state. Treat ordinary 401/403 as boundary evidence and continue other contracts.

If no eligible hypothesis exists but standard investigation is allowed, validate only the strongest observed business API or exact source-mapped GET/HEAD, record its status and response structure, then close the target as bounded-complete. A targeted fallback pass is allowed only for an explicit high-value coverage gap, at most {discovery_passes} pass(es), using observed business terms and no recursive brute force. Telemetry, static assets, frontend routes, UNKNOWN methods and string-only URL combinations are not formal APIs.

`investigation.manualDeepDive` is a deterministic list of coverage gaps, not vulnerabilities. Do not spend a model turn rediscovering it. Preserve the highest-priority rows in the final coverage summary with observed evidence, missing prerequisite, concise manual steps and stop condition. Never call an untriggered or untested surface safe.

Use the built-in adapter only through commands declared in `src-capabilities.json`. OAST-dependent conclusions require `oast.available=true` and observed callbacks; otherwise record the missing prerequisite as not tested. Stop active requests on a confirmed WAF/bot challenge/CAPTCHA, sustained 429 or homogeneous blocking. No difference, no finding and an exhausted bounded branch are completion states, not reasons to pause or request another run.

Keep intermediate prose brief. The root agent must finish by calling `finish_scan` exactly once; never return a prose-only final answer. Keep every `finish_scan` field concise: tested function and endpoint, control/test evidence, impact or normal/exhausted result, untested prerequisite, cost/stop reason, and the single highest-value remaining manual lead. A confirmed finding also needs PoC, scope, CVSS/CWE and remediation.

Selected skill section index (full text remains local and must not be reloaded):
{skill_digest}

Task-specific requirements:
{additional}
"#,
    )
}

fn local_skill_heading_digest(skill_instructions: &str, max_chars: usize) -> String {
    let headings = skill_instructions
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with('#'))
        .map(|line| line.trim_start_matches('#').trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" -> ");
    if headings.is_empty() {
        "Built-in business frontend and SRC workflow.".into()
    } else {
        headings.chars().take(max_chars).collect()
    }
}
