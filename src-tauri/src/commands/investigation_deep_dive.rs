// Manual deep-dive planning: the read-only APIs, evidence and next steps the desk
// offers a human when automated proof is not enough. Included from investigation.rs.

fn manual_api_evidence(apis: &[(String, JsonValue)], markers: &[&str]) -> Vec<String> {
    let mut evidence = apis
        .iter()
        .filter_map(|(_, api)| {
            let raw_method = value_first(api, &["method"]).to_ascii_uppercase();
            let method = if raw_method.is_empty() {
                "HTTP".to_string()
            } else {
                raw_method
            };
            let endpoint = value_first(api, &["url", "path"]);
            let parameters = investigation_strings(api.get("parameters"));
            let searchable = format!("{} {}", endpoint, parameters.join(" ")).to_ascii_lowercase();
            markers
                .iter()
                .any(|marker| searchable.contains(marker))
                .then(|| format!("{method} {}", normalized_investigation_path(&endpoint)))
        })
        .collect::<Vec<_>>();
    evidence.sort();
    evidence.dedup();
    evidence.truncate(3);
    evidence
}

#[allow(clippy::too_many_arguments)]
fn manual_deep_dive_item(
    category: &str,
    title: &str,
    priority: &str,
    reason: &str,
    evidence: Vec<String>,
    missing_evidence: &str,
    steps: &[&str],
    stop_condition: &str,
) -> JsonValue {
    serde_json::json!({
        "category":category,
        "title":title,
        "priority":priority,
        "reason":reason,
        "evidence":evidence,
        "missingEvidence":missing_evidence,
        "steps":steps,
        "stopCondition":stop_condition,
        "classification":"coverage_gap_not_vulnerability",
        "source":"deterministic-src-coverage"
    })
}

/// Produce high-yield human follow-up from deterministic coverage gaps. These
/// rows are never vulnerability findings: they explain what the bounded Web
/// run did not reach and the exact business evidence needed to continue.
fn manual_deep_dive_plan(
    target: &JsonValue,
    apis: &[(String, JsonValue)],
    actions: &[JsonValue],
    identity_keys: &[String],
    mode: &str,
) -> JsonValue {
    let mut surface_parts = Vec::new();
    for (_, api) in apis {
        surface_parts.push(value_first(api, &["method"]));
        surface_parts.push(value_first(api, &["url", "path"]));
        surface_parts.extend(investigation_strings(api.get("parameters")));
    }
    for action in actions {
        surface_parts.push(value_first(
            action,
            &["label", "role", "outcome", "afterUrl"],
        ));
    }
    for key in [
        "businessEntrypoints",
        "registrationEntrypoints",
        "routeCandidates",
    ] {
        surface_parts.extend(investigation_strings(target.get(key)));
    }
    let surface = surface_parts.join(" ").to_ascii_lowercase();
    let has_any = |markers: &[&str]| markers.iter().any(|marker| surface.contains(marker));
    let authenticated_identities = identity_keys
        .iter()
        .filter(|identity| !anonymous_identity(identity))
        .count();
    let write_api_count = apis
        .iter()
        .filter(|(_, api)| {
            matches!(
                value_first(api, &["method"])
                    .to_ascii_uppercase()
                    .as_str(),
                "POST" | "PUT" | "PATCH" | "DELETE"
            )
        })
        .count();
    let mut leads = Vec::new();

    let object_markers = [
        "user_id", "uid", "/user/", "/users/", "account", "member", "tenant", "org_id",
        "role", "owner", "project_id", "team_id", "/detail", "/admin",
    ];
    if !apis.is_empty() && (authenticated_identities < 2 || has_any(&object_markers)) {
        leads.push(manual_deep_dive_item(
            "authorization",
            "同级账号、对象归属与字段级权限",
            "critical",
            if authenticated_identities < 2 {
                "自动化没有两套独立有效身份，无法覆盖同级账号与跨租户对象边界。"
            } else {
                "已观察到对象、用户、租户或角色相关契约，仍需结合真实对象归属确认字段级读写边界。"
            },
            manual_api_evidence(apis, &object_markers),
            if authenticated_identities < 2 {
                "两个独立平权账号、各自拥有的对象以及完整双侧响应"
            } else {
                "明确的对象所有者、非所有者和字段修改前后状态"
            },
            &[
                "为两个账号各准备一个可识别测试对象",
                "固定同一方法和请求形状，仅替换对象引用",
                "分别比较状态、字段、对象归属和实际副作用",
            ],
            "完成主要对象类型的所有者/非所有者对照，或连续三个代表性对象均无权限差异",
        ));
    }

    let auth_markers = [
        "login", "logout", "register", "signup", "reset", "recover", "password", "captcha",
        "qrcode", "oauth", "mfa", "bind", "token", "session",
    ];
    if has_any(&auth_markers) {
        leads.push(manual_deep_dive_item(
            "authentication_session",
            "登录、找回、绑定与令牌生命周期",
            "high",
            "身份流程包含一次性状态、跨页面跳转或客户端令牌，自动重放无法完整理解所有生命周期约束。",
            manual_api_evidence(apis, &auth_markers),
            "可控测试账号、旧/新令牌、跨浏览器状态和完整找回或绑定流程",
            &[
                "记录登录前后、退出后和改密后的令牌有效性",
                "检查找回/绑定步骤能否跳步、重放或跨账号复用",
                "比较错误账号、错误验证码和不存在账号的可观察差异",
            ],
            "关键令牌均按预期失效，流程步骤不可跨账号复用且无稳定枚举差异",
        ));
    }

    let business_markers = [
        "order", "pay", "payment", "refund", "coupon", "balance", "points", "credit",
        "invite", "claim", "redeem", "approve", "audit", "workflow", "/status", "_status",
        "quota", "stock", "subscribe",
    ];
    if has_any(&business_markers) {
        leads.push(manual_deep_dive_item(
            "business_flow",
            "业务状态机、次数/额度与流程跳步",
            "critical",
            "发现订单、权益、审核、额度或状态相关入口；这类风险依赖公司业务不变量，不能仅凭通用模型判定。",
            manual_api_evidence(apis, &business_markers),
            "正常状态图、允许的顺序/次数/额度、隔离测试数据和回滚方式",
            &[
                "画出正常状态转换和服务端认可的最终状态",
                "验证跳步、重复提交、乱序调用和跨入口操作",
                "检查数量、金额、次数、时间和边界值是否由服务端统一校验",
            ],
            "关键状态转换、额度和幂等性均由服务端约束，且测试数据已恢复",
        ));
    }

    let file_markers = [
        "upload", "download", "file", "attachment", "avatar", "import", "export", "archive",
        "template", "document", "image", "pdf",
    ];
    if has_any(&file_markers) {
        leads.push(manual_deep_dive_item(
            "file_handling",
            "文件、导入导出与对象存储边界",
            "high",
            "文件处理通常跨越上传、解析、存储、下载和异步任务，单次只读验证覆盖不完整。",
            manual_api_evidence(apis, &file_markers),
            "无害样本、下载对象归属、清理接口以及解析完成后的结果",
            &[
                "先验证跨账号下载与签名链接有效期",
                "用无害样本检查类型、文件名、压缩包和解析结果",
                "确认上传对象不可覆盖他人资源并完成清理",
            ],
            "下载授权、存储隔离和解析边界均有证据，所有测试文件已删除",
        ));
    }

    let integration_markers = [
        "url", "uri", "callback", "webhook", "redirect", "preview", "fetch", "proxy",
        "remote", "rss", "convert", "source", "image_url",
    ];
    if has_any(&integration_markers) {
        leads.push(manual_deep_dive_item(
            "server_side_integration",
            "服务端取 URL、Webhook 与不可信上游",
            "high",
            "发现 URL、回调、预览或远程资源参数，可能存在异步消费、重定向或第三方响应信任边界。",
            manual_api_evidence(apis, &integration_markers),
            "目标可达的唯一回连地址、异步任务结果和服务端实际取回证据",
            &[
                "确认参数是否由服务端而非浏览器访问",
                "分别观察直接地址、受控重定向和异步消费结果",
                "记录上游内容类型、大小、超时和错误回退行为",
            ],
            "没有服务端访问证据，或所有可控地址均被稳定拒绝且异步任务已结束",
        ));
    }

    let realtime_markers = [
        "graphql", "websocket", "wss", "eventsource", "sse", "subscribe", "subscription",
        "socket", "stream",
    ];
    if has_any(&realtime_markers) {
        leads.push(manual_deep_dive_item(
            "realtime_api",
            "GraphQL、订阅与实时消息授权",
            "high",
            "实时协议和 GraphQL 的授权边界位于消息、字段或订阅层，普通 HTTP 接口清单无法完整表达。",
            manual_api_evidence(apis, &realtime_markers),
            "握手请求、消息结构、订阅对象、断线重连和双身份消息证据",
            &[
                "记录握手与首个业务消息",
                "使用两个账号比较订阅对象和字段范围",
                "验证断线重连、令牌失效与取消订阅后的消息边界",
            ],
            "主要订阅和字段均完成双身份对照，失效会话不能继续收到受保护消息",
        ));
    }

    let client_trust_markers = [
        "nonce", "hkey", "signature", "_sign", "sign=", "/sign", "client_type",
        "client_version", "device_id", "x_app", "timestamp",
    ];
    if has_any(&client_trust_markers) {
        leads.push(manual_deep_dive_item(
            "client_trust",
            "客户端签名、版本与设备信任",
            "medium",
            "请求包含客户端签名、版本、设备或时间字段；需要确认服务端校验的是安全边界还是仅兼容性参数。",
            manual_api_evidence(apis, &client_trust_markers),
            "字段生成位置、服务端失败响应和当前会话重新签名能力",
            &[
                "分别移除、固定和正常更新非认证字段",
                "比较服务端是否只依赖客户端可控版本/设备标识",
                "避免把公开算法本身当作漏洞，只记录可重复的服务端信任缺陷",
            ],
            "字段缺失和篡改均被服务端按预期处理，或确认其不承担安全边界",
        ));
    }

    let resource_markers = [
        "search", "query", "list", "batch", "export", "report", "page", "size", "limit",
        "offset", "count",
    ];
    if has_any(&resource_markers) {
        leads.push(manual_deep_dive_item(
            "resource_consumption",
            "分页、批量、复杂查询与资源配额",
            "medium",
            "发现列表、搜索、报表或批量参数；自动化不会在不清楚生产容量时扩大负载。",
            manual_api_evidence(apis, &resource_markers),
            "安全测试窗口、可接受速率、最大分页/导出规模和任务配额",
            &[
                "先确认单请求的服务端上限",
                "在批准的低速窗口检查分页、批量和异步任务配额",
                "观察同账号、同 IP 和跨账号限制是否一致",
            ],
            "达到约定安全上限或确认服务端存在稳定的容量与并发限制",
        ));
    }

    if write_api_count > 0 && has_any(&business_markers) {
        leads.push(manual_deep_dive_item(
            "race_condition",
            "幂等、重复消费与并发一致性",
            "high",
            "存在状态变更接口和敏感业务对象，但缺少可恢复业务不变量时自动化不会并发写入。",
            manual_api_evidence(apis, &business_markers),
            "隔离测试对象、期望不变量、幂等键、并发前后状态和清理方案",
            &[
                "先以顺序请求建立正常结果",
                "在安全窗口对同一测试对象做小规模同步并发",
                "核对最终状态、次数、余额/库存及重复副作用",
            ],
            "业务不变量保持成立且测试对象恢复；出现异常立即停止并保存前后状态",
        ));
    }

    if apis.len() < 6 || actions.len() < 2 || mode == "deep" {
        leads.push(manual_deep_dive_item(
            "api_inventory",
            "未触发功能、旧版/移动端与影子 API",
            if apis.len() < 3 { "high" } else { "medium" },
            "Web 自动探索只证明当前页面状态实际触发的接口，不能代表移动端、合作方、旧版本或隐藏管理入口。",
            vec![format!(
                "当前仅形成 {} 个正式接口、{} 个页面动作",
                apis.len(),
                actions.len()
            )],
            "其它客户端流量、接口文档、旧版本路径、内部域名或更多业务角色",
            &[
                "优先查看页面未触发菜单和真实移动端/旧版客户端请求",
                "对已观察公共前缀查找版本、管理、批量和调试分支",
                "只保留有真实响应和业务归属的接口，避免字符串拼接噪音",
            ],
            "新增入口不再产生正式业务接口，或已覆盖所有在用客户端与角色",
        ));
    }

    if leads.is_empty() {
        leads.push(manual_deep_dive_item(
            "business_semantics",
            "业务语义与跨渠道一致性复核",
            "medium",
            "当前自动契约没有形成额外高价值风险信号，但业务规则、跨渠道状态和异常恢复仍无法由通用模型完整推断。",
            vec![format!(
                "已形成 {} 个正式接口、{} 个页面动作",
                apis.len(),
                actions.len()
            )],
            "业务规则、角色矩阵、异常状态和其它客户端行为",
            &[
                "选择最重要的一个业务结果并写出服务端不变量",
                "比较正常、越界、重复和中断恢复路径",
                "确认不同客户端和角色得到一致的服务端约束",
            ],
            "核心不变量均有对照证据，且没有新的接口、状态或对象边界出现",
        ));
    }

    let limit = match mode {
        "quick" => 3,
        "deep" => 8,
        _ => 5,
    };
    leads.truncate(limit);
    for (index, lead) in leads.iter_mut().enumerate() {
        if let Some(object) = lead.as_object_mut() {
            object.insert("rank".into(), ((index + 1) as i64).into());
        }
    }
    JsonValue::Array(leads)
}
