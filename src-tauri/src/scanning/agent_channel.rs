//! Trusted desktop adapter. No independent service store or snapshot is adopted.
use super::*;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::Manager;

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS agent_scan_operations(project_id INTEGER NOT NULL REFERENCES projects(id),operation_id TEXT NOT NULL,action TEXT NOT NULL,arguments_sha256 TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('prepared','completed','uncertain')),receipt_json TEXT NOT NULL DEFAULT '',created_at TEXT NOT NULL DEFAULT(datetime('now')),PRIMARY KEY(project_id,operation_id));";

#[path = "agent_results.rs"]
mod results;
#[path = "agent_scan_recovery.rs"]
mod recovery;
#[path = "agent_scan_diagnostics.rs"]
mod diagnostics;
#[path = "fingerprint_reference.rs"]
mod fingerprint_reference;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FromAssets {
    asset_ids: Vec<i64>,
    scan_mode: Option<String>,
    max_model_requests: Option<i64>,
    max_model_tokens: Option<i64>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Task {
    task_id: String,
    #[serde(default)]
    required_capabilities: Vec<String>,
    #[serde(default)]
    conversation_id: String,
    #[serde(default)]
    binding_version: Option<i64>,
    #[serde(default)]
    attempt: Option<i64>,
    #[serde(default)]
    attempt_number: Option<i64>,
}
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct List {
    limit: Option<i64>,
    offset: Option<i64>,
}
/// Operator-only capability-class authorization, reached only through the trusted
/// Host bridge (`scanAuthorizationConversationAction` → internalAction
/// `authorize_capability`). It is never a model tool. The actual write delegates
/// to `set_scan_capability_authorization_on`, which keeps the legal-attestation
/// precondition, the known-class check and the immutable-record guarantee.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AuthorizeCapability {
    scan_id: String,
    #[serde(default)]
    attempt_number: Option<i64>,
    legal_attestation: bool,
    classes: Vec<String>,
    #[serde(default)]
    attested_by: Option<String>,
}

fn active(c: &rusqlite::Connection, project: i64) -> Result<(), String> {
    if project < 1
        || !c
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1 AND status='active')",
                [project],
                |r| r.get::<_, bool>(0),
            )
            .map_err(|_| "scan_project_unavailable")?
    {
        return Err("scan_project_unavailable".into());
    }
    Ok(())
}
fn present(c: &rusqlite::Connection, project: i64) -> Result<(), String> {
    if project < 1
        || !c
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1)",
                [project],
                |r| r.get::<_, bool>(0),
            )
            .map_err(|_| "scan_project_unavailable")?
    {
        return Err("scan_project_unavailable".into());
    }
    Ok(())
}
fn owned_scope(c: &rusqlite::Connection, project: i64, task: &str) -> Result<(), String> {
    if task.is_empty() || task.len() > 128 || task.chars().any(char::is_control) {
        return Err("scan_task_invalid".into());
    }
    let exists:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM sentinel_scans s WHERE id=?1 AND project_id=?2 AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans d WHERE d.scan_id=s.id))",params![task,project],|r|r.get(0)).map_err(|_|"scan_task_unavailable")?;
    if !exists {
        return Err("scan_task_unavailable".into());
    }
    Ok(())
}
fn owned(c: &rusqlite::Connection, project: i64, task: &str) -> Result<SentinelScan, String> {
    owned_scope(c, project, task)?;
    sentinel_scan_by_id(c, task)
}
fn projection(c: &rusqlite::Connection, scan: &SentinelScan) -> Result<Value, String> {
    let mut progress = serde_json::Map::new();
    let mut s = c
        .prepare("SELECT status,COUNT(*) FROM sentinel_targets WHERE scan_id=?1 GROUP BY status")
        .map_err(|_| "scan_progress_unavailable")?;
    let rows = s
        .query_map([&scan.id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })
        .map_err(|_| "scan_progress_unavailable")?;
    let mut total = 0;
    for row in rows {
        let (status, count) = row.map_err(|_| "scan_progress_unavailable")?;
        total += count;
        progress.insert(status, json!(count));
    }
    // Keep path/config/raw logs and target session material outside model context.
    let attempt_usage = match attempt_model_requests(c, &scan.id, scan.attempt_count) {
        Ok(count) => (json!(count), Value::Null),
        Err(reason) => (Value::Null, json!(reason)),
    };
    Ok(
        json!({"id":scan.id,"title":scan.task_name,"status":scan.status,"scanType":scan.scan_type,
        "attempt":scan.attempt_count,"latestAttemptStatus":scan.latest_attempt_status,
        "targetCount":total,"progress":progress,"modelRequests":scan.llm_requests,"attemptModelRequests":attempt_usage.0,"attemptUsageReason":attempt_usage.1,"tokens":scan.total_tokens,
        "bindingVersion":crate::scanning::task_binding::current_binding_version(c, scan.project_id.unwrap_or(0), &scan.id).map(|version| json!(version)).unwrap_or(Value::Null),
        "createdAt":scan.created_at,"updatedAt":scan.updated_at,"executionAvailable":scan.scan_type=="web",
        "executionStarted":scan.status=="scanning","modelLimits":saved_model_limits(c,&scan.id)?,"lastOperation":last_operation(c,scan.project_id.unwrap_or(0),&scan.id),
        "lastCommand":crate::scanning::task_binding::latest_command(c,&scan.id),
        "pendingCommandCount":crate::scanning::task_binding::pending_command_count(c,&scan.id),
        "pendingInstructionCount":crate::scanning::task_binding::pending_instruction_count(c,&scan.id),
        "pendingVerificationCount":crate::scanning::task_binding::pending_verification_count(c,&scan.id),
        "pendingCommands":crate::scanning::task_binding::pending_commands(c,&scan.id),
        "source":"desktop_scanning_owner"}),
    )
}
pub(crate) fn attempt_model_requests(c: &rusqlite::Connection, scan_id: &str, attempt: i64) -> Result<i64, String> {
    if attempt <= 0 { return Ok(0); }
    match c.query_row(
        "SELECT llm_requests_delta FROM sentinel_scan_attempts WHERE scan_id=?1 AND attempt_number=?2",
        params![scan_id, attempt], |row| row.get::<_, i64>(0),
    ) {
        Ok(count) => Ok(count),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(0),
        Err(_) => Err("attempt_usage_unreadable".into()),
    }
}
fn last_operation(c: &rusqlite::Connection, project: i64, task: &str) -> Value {
    let row = c.query_row(
        "SELECT operation_id, action, state, receipt_json FROM agent_scan_operations WHERE project_id=?1 AND (json_extract(receipt_json,'$.taskId')=?2 OR json_extract(receipt_json,'$.task.id')=?2) ORDER BY created_at DESC, operation_id DESC LIMIT 1",
        params![project, task],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?)),
    );
    let Ok((operation_id, action, state, receipt)) = row else { return Value::Null; };
    let parsed: Value = serde_json::from_str(&receipt).unwrap_or(Value::Null);
    json!({"operationId": operation_id, "action": action, "operationState": state,
        "phase": parsed.get("phase").cloned().unwrap_or(Value::Null),
        "reasonCode": parsed.get("reasonCode").cloned().unwrap_or(Value::Null),
        "outcomeUnknown": parsed.get("outcomeUnknown").cloned().unwrap_or(Value::Null),
        "attempt": parsed.pointer("/task/attempt").or_else(|| parsed.get("attemptNumber")).cloned().unwrap_or(Value::Null)})
}
fn execution_method_for(id: &str) -> &'static str {
    match id {
        "browser" | "sandbox" | "uploadRole" | "concurrencyRole" => "not_executable",
        "tools" => "builtin_http_and_ast",
        "model" => "saved_model_profile",
        "targets" => "task_targets",
        "projectAuthorization" => "project_membership",
        "recovery" => "original_ledger",
        "broker" => "deferred_until_lease",
        "cost" => "saved_limits",
        "modelNetwork" => "configured_endpoint_unverified",
        _ => "unspecified",
    }
}
fn readiness_item(id: &str, state: &str, reason_code: Option<&str>, detail: &str) -> Value {
    let required = matches!(id, "model" | "targets" | "projectAuthorization" | "modelNetwork");
    json!({"id": id, "state": state, "required": required, "executable": state == "ready", "checked": state != "unchecked", "reasonCode": reason_code, "detail": detail, "checkedAt": "", "executionMethod": execution_method_for(id), "configVersion": ""})
}
fn tag_model_endpoint(mut item: Value, host: Option<&str>, port: Option<u16>) -> Value {
    match (host, port) {
        (Some(host), Some(port)) if !host.is_empty() => {
            item["executionMethod"] = json!("configured_endpoint_unverified");
            if item.get("configVersion").and_then(Value::as_str).unwrap_or("").is_empty() {
                item["configVersion"] = json!(format!("{host}:{port}"));
            }
        }
        _ => {
            item["executionMethod"] = json!("model_endpoint_unconfigured");
        }
    }
    item
}
fn stamp_model_network_client(item: &mut Value, tls_required: bool, auth_configured: bool) {
    if item.get("id").and_then(Value::as_str) != Some("modelNetwork") {
        return;
    }
    item["tlsRequired"] = json!(tls_required);
    item["authConfigured"] = json!(auth_configured);
    item["tlsVerified"] = json!(false);
    item["authVerified"] = json!(false);
    item["executable"] = json!(false);
    if item.get("state").and_then(Value::as_str) == Some("ready") {
        item["state"] = json!("unchecked");
        item["checked"] = json!(false);
        item["reasonCode"] = json!("scan_model_tls_auth_unchecked");
    }
}
fn model_network_routes(api_base: Option<&str>, proxies: &[String]) -> Value {
    // OAuth talks to the process-owned Host bridge directly. Upstream network
    // routing belongs to Host, so an unrelated scan proxy cannot block this hop.
    if api_base.is_some_and(crate::agent_host_sidecar::is_model_bridge_endpoint) {
        return model_network_item(api_base, None);
    }
    // The executor rotates `offset % proxies.len()`. Every entry is a live hop.
    if proxies.is_empty() {
        return model_network_item(api_base, None);
    }
    let mut pending = None;
    for proxy in proxies {
        let item = model_network_item(api_base, Some(proxy));
        if item.get("state").and_then(Value::as_str) == Some("blocked") {
            return item;
        }
        pending = Some(item);
    }
    pending.unwrap_or_else(|| model_network_item(api_base, None))
}
/// Model calls use the same http/https port rules as preflight. Other schemes stop.
pub(crate) fn model_endpoint_port(endpoint: &str) -> Result<(String, u16), String> {
    let url = reqwest::Url::parse(endpoint).map_err(|_| "scan_model_endpoint_invalid".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("scan_model_endpoint_invalid".into());
    }
    let host = url.host_str().filter(|host| !host.is_empty()).ok_or_else(|| "scan_model_endpoint_invalid".to_string())?;
    let port = configured_hop_port(&url).ok_or_else(|| "scan_model_endpoint_invalid".to_string())?;
    Ok((host.to_string(), port))
}

/// The port the model client would open. An unknown scheme has no substitute
/// port: preflight must not invent a direct connection to 443 or 80.
pub(crate) fn configured_hop_port(url: &reqwest::Url) -> Option<u16> {
    if let Some(port) = url.port() {
        return Some(port);
    }
    match url.scheme() {
        "http" => Some(80),
        "https" => Some(443),
        "socks5" | "socks5h" => Some(1080),
        _ => None,
    }
}

fn model_network_item(api_base: Option<&str>, proxy: Option<&str>) -> Value {
    let Some(api_base) = api_base.filter(|value| !value.is_empty()) else {
        return tag_model_endpoint(readiness_item("modelNetwork", "unchecked", None, "没有可探测的模型地址，目标网络也未检查"), None, None);
    };
    let Ok(model_url) = reqwest::Url::parse(api_base) else {
        return tag_model_endpoint(readiness_item("modelNetwork", "blocked", Some("scan_model_endpoint_invalid"), "模型地址无法解析，没有向扫描目标发包"), None, None);
    };
    let Some(model_host) = model_url.host_str() else {
        return tag_model_endpoint(readiness_item("modelNetwork", "blocked", Some("scan_model_endpoint_invalid"), "模型地址没有主机，没有向扫描目标发包"), None, None);
    };
    if !matches!(model_url.scheme(), "http" | "https") {
        return tag_model_endpoint(readiness_item("modelNetwork", "blocked", Some("scan_model_endpoint_invalid"), "模型地址不是 http 或 https，没有改用 443 探测，也没有向扫描目标发包"), None, None);
    }
    let Some(model_port) = configured_hop_port(&model_url) else {
        return tag_model_endpoint(readiness_item("modelNetwork", "blocked", Some("scan_model_endpoint_invalid"), "模型地址没有可执行端口，没有改用 443 探测，也没有向扫描目标发包"), None, None);
    };
    let tag = |item| tag_model_endpoint(item, Some(model_host), Some(model_port));
    let routed = if let Some(proxy) = proxy.map(str::trim).filter(|value| !value.is_empty()) {
        let Ok(proxy_url) = reqwest::Url::parse(proxy) else {
            return tag(readiness_item("modelNetwork", "blocked", Some("scan_model_proxy_invalid"), "模型代理地址无效，没有直连模型端口，也没有向扫描目标发包"));
        };
        let Some(proxy_host) = proxy_url.host_str() else {
            return tag(readiness_item("modelNetwork", "blocked", Some("scan_model_proxy_invalid"), "模型代理地址没有主机，没有直连模型端口，也没有向扫描目标发包"));
        };
        let Some(proxy_port) = configured_hop_port(&proxy_url) else {
            return tag(readiness_item("modelNetwork", "blocked", Some("scan_model_proxy_invalid"), "模型代理协议没有可执行端口，没有改用 80 或 443，也没有向扫描目标发包"));
        };
        (proxy_host.to_string(), proxy_port, true)
    } else {
        (model_host.to_string(), model_port, false)
    };
    let (host, port, through_proxy) = routed;
    if host.ends_with(".example.test") || host.ends_with(".example.invalid") || host.ends_with(".invalid") {
        let hop = if through_proxy { "代理" } else { "模型" };
        return tag(readiness_item("modelNetwork", "unchecked", None, &format!("示例{hop} {host}:{port} 不在预检里发包，目标网络未检查")));
    }
    use std::net::ToSocketAddrs;
    let Ok(mut addresses) = format!("{host}:{port}").to_socket_addrs() else {
        let code = if through_proxy { "scan_model_proxy_unreachable" } else { "scan_model_network_unreachable" };
        return tag(readiness_item("modelNetwork", "blocked", Some(code), &format!("{host}:{port} 无法解析，没有向扫描目标发包")));
    };
    let Some(address) = addresses.next() else {
        let code = if through_proxy { "scan_model_proxy_unreachable" } else { "scan_model_network_unreachable" };
        return tag(readiness_item("modelNetwork", "blocked", Some(code), &format!("{host}:{port} 没有地址，没有向扫描目标发包")));
    };
    match std::net::TcpStream::connect_timeout(&address, std::time::Duration::from_millis(400)) {
        Ok(_) => tag(model_network_transport_seen(through_proxy, &host, port, true)),
        Err(_) => {
            let code = if through_proxy { "scan_model_proxy_unreachable" } else { "scan_model_network_unreachable" };
            tag(readiness_item("modelNetwork", "blocked", Some(code), &format!("{host}:{port} 不可达，没有向扫描目标发包")))
        }
    }
}
fn model_network_transport_seen(through_proxy: bool, host: &str, port: u16, connected: bool) -> Value {
    if !connected {
        let code = if through_proxy { "scan_model_proxy_unreachable" } else { "scan_model_network_unreachable" };
        return readiness_item("modelNetwork", "blocked", Some(code), &format!("{host}:{port} 不可达，没有向扫描目标发包"));
    }
    if through_proxy {
        return readiness_item("modelNetwork", "unchecked", Some("scan_model_proxy_auth_unchecked"), &format!("已连通代理 {host}:{port}。认证和模型服务尚未验证，这次连通不是放行，也没有向扫描目标发包"));
    }
    readiness_item("modelNetwork", "unchecked", Some("scan_model_tls_auth_unchecked"), &format!("已连通模型地址 {host}:{port}。TLS 和认证还没按真实客户端核对，这次连通不是放行，也没有向扫描目标发包"))
}
fn saved_model_limits(c: &rusqlite::Connection, task: &str) -> Result<Value, String> {
    let raw: Option<String> = c
        .query_row(
            "SELECT policy_json FROM sentinel_scan_contexts WHERE scan_id=?1",
            [task],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| "scan_policy_unavailable")?;
    let policy: Value = raw
        .map(|v| serde_json::from_str(&v))
        .transpose()
        .map_err(|_| "scan_policy_unavailable")?
        .unwrap_or(Value::Null);
    Ok(
        json!({"maxModelRequests":policy.get("maxModelRequests"),"maxModelTokens":policy.get("maxModelTokens"),
        "requestedMode":match policy.get("webModeCeiling").and_then(Value::as_str){Some("quick")=>"quick",Some("deep")=>"deep",_=>"standard"},
        "scope":"per_target_per_attempt","requestedMaxBudgetUsd":policy.get("maxBudgetUsd"),"usdLimitEnforced":false}),
    )
}
fn preflight(c: &rusqlite::Connection, project: i64, task: &str) -> Result<Value, String> {
    let tx = c
        .unchecked_transaction()
        .map_err(|_| "scan_preflight_unavailable")?;
    let scan = owned(&tx, project, task)?;
    let settings = agent_policy_settings_for_scan(&tx, task)?;
    let (configured, model, network) = match model_runtime_env(&settings).and_then(|environment| {
        crate::model_configuration::gateway_profile(&environment.as_connection(), None).map(|profile| (environment, profile))
    }) {
        Ok((environment, profile)) => {
            let endpoint = reqwest::Url::parse(&profile.endpoint).ok();
            let proxies = approved_agent_proxies(&settings).into_iter().map(|(_, url)| url).collect::<Vec<_>>();
            let mut network = model_network_routes(Some(profile.endpoint.as_str()), &proxies);
            let tls_required = reqwest::Url::parse(&profile.endpoint).is_ok_and(|url| url.scheme() == "https");
            stamp_model_network_client(&mut network, tls_required, profile.bearer().is_some());
            (
                true,
                Some(json!({"name": profile.model, "deployment": environment.deployment, "endpointHost": endpoint.as_ref().and_then(|url| url.host_str()), "endpointPort": endpoint.as_ref().and_then(|url| url.port_or_known_default())})),
                network,
            )
        }
        Err(_) => {
            let mut network = model_network_item(None, None);
            stamp_model_network_client(&mut network, false, false);
            (false, None, network)
        }
    };
    let financial_resume=if scan.scan_type=="web" {
        match super::scan_resume_financial_preflight_in(&tx,task) {
            Ok(())=>json!({"checked":true,"ordinaryResumeBlocked":false,"onlyFinancialHistoryChecked":true}),
            Err(code)=>json!({"checked":true,"ordinaryResumeBlocked":true,"reasonCode":code,"onlyFinancialHistoryChecked":true,
                "reason":"普通恢复被原生费用保护策略阻止；请保留原账单与预算，使用原 owner 的费用核对或剩余额度接续流程。不是应用读取或执行授权丢失；本次没有创建执行尝试或发送请求。"}),
        }
    }else{json!({"checked":false,"ordinaryResumeBlocked":true,"reasonCode":"scan_channel_web_only"})};
    let task_view = projection(&tx, &scan)?;
    let target_count = task_view["targetCount"].as_i64().unwrap_or(0);
    let items = vec![
        if configured {
            readiness_item("model", "ready", None, "已解析保存的扫描模型配置")
        } else {
            readiness_item("model", "blocked", Some("scan_model_unconfigured"), "扫描模型连接未就绪，请到配置中心 → 模型检查")
        },
        if target_count > 0 {
            readiness_item("targets", "ready", None, "当前任务有可扫描目标")
        } else {
            readiness_item("targets", "blocked", Some("scan_no_eligible_targets"), "任务没有可扫描目标")
        },
        readiness_item("projectAuthorization", "ready", None, "任务属于当前项目"),
        if scan.scan_type == "web" {
            if financial_resume["ordinaryResumeBlocked"].as_bool() == Some(true) {
                readiness_item("recovery", "unavailable", financial_resume["reasonCode"].as_str(), "普通恢复被原费用门禁挡住；用 scan_recovery 看同一任务的原账本，不新建额度")
            } else {
                readiness_item("recovery", "ready", None, "未发现阻止普通恢复的已有费用历史")
            }
        } else {
            readiness_item("recovery", "unchecked", Some("scan_channel_web_only"), "未检查非 Web 任务的恢复条件")
        },
        readiness_item("tools", "unavailable", Some(crate::tool_supply::builtin_scan_tools::SANDBOX_RUNNER), &crate::tool_supply::builtin_scan_tools::summary()),
        readiness_item("broker", "unchecked", None, "进程内 Broker 要等执行期目标租约才准入，预检没有向目标发包"),
        network,
        readiness_item("browser", "unavailable", Some("browser_network_broker_unavailable"), "真实浏览器流量尚未进入 Broker。HTTP 回放不是点击、登录 DOM、XHR 或 WebSocket 验证"),
        readiness_item("sandbox", "unavailable", Some("macos_hard_memory_containment_unverified"), "macOS 离线解析的硬内存隔离未验证。本次没有做实机隔离测试，不代表容器沙箱不存在"),
        readiness_item("uploadRole", "unavailable", Some("upload_role_not_implemented"), "上传专家未实现，不能先规划再发现执行器不存在"),
        readiness_item("concurrencyRole", "unavailable", Some("concurrency_role_not_implemented"), "并发竞态专家未实现，不能先规划再发现执行器不存在"),
        readiness_item("cost", "ready", None, "已读取保存的 token 与请求限额。USD 值不限制供应商账单"),
    ];
    let checked_at: String = tx.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%SZ','now')", [], |row| row.get(0)).unwrap_or_default();
    let checked_at = if checked_at.is_empty() { chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string() } else { checked_at };
    let config_version = model.as_ref().map(|record| {
        let name = record["name"].as_str().unwrap_or("");
        let host = record["endpointHost"].as_str().unwrap_or("");
        let port = record["endpointPort"].as_u64().unwrap_or(0);
        if name.is_empty() || host.is_empty() || port == 0 { String::new() } else { format!("{name}@{host}:{port}") }
    }).unwrap_or_default();
    let mut items = items;
    for item in &mut items {
        item["checkedAt"] = json!(checked_at);
        item["configVersion"] = json!(config_version);
        stamp_admission_states(item);
    }
    // A model route that is unreachable or misconfigured blocks every start and
    // continuation. A route that connected but whose TLS and authentication are
    // not yet checked with the real client stays `unchecked`, reported as such.
    // The read-only probe dials directly and may miss a system proxy, so a
    // network failure there is reported, not a blocker: only a model route
    // that cannot be configured at all blocks here. Execution admission then
    // proves the route with the real client.
    let blocker = items.iter().find(|item| item["required"] == true && item["state"] != "ready"
        && !(item["id"] == "modelNetwork" && !item["reasonCode"].as_str().is_some_and(|code| code.ends_with("_invalid"))));
    let start_blocked = blocker.is_some();
    let start_reason = blocker.and_then(|item| item["reasonCode"].as_str()).unwrap_or("capability_not_executable").to_string();
    let executable: Vec<&str> = items.iter().filter(|item| item["executable"] == true).filter_map(|item| item["id"].as_str()).collect();
    let excluded: Vec<Value> = items.iter().filter(|item| item["executable"] != true).map(|item| json!({"id": item["id"], "state": item["state"], "reasonCode": item["reasonCode"]})).collect();
    Ok(
        json!({"projectId":project,"task":task_view,"modelConfigured":configured,"model":model,"resumeFinancialCheck":financial_resume,
        "readiness": items, "executableCapabilities": executable,
        "admission": {"configValid": !(start_blocked && (start_reason.ends_with("_invalid") || start_reason.ends_with("_unconfigured"))), "connectionVerified": false, "admittedForRun": false}, "excludedCapabilities": excluded, "startBlocked": start_blocked, "startBlockedReason": if start_blocked { Value::String(start_reason.clone()) } else { Value::Null },
        "configurationSource":"desktop_models_category","networkTested":false,"engineAdmissionValidated":false,
        "recovery":if scan.scan_type=="web"{Some(recovery::view(&tx,project,task)?)}else{None},
        "executionAuthorizedByThisRead":false,"reason":if start_blocked { format!("启动前已阻断：{start_reason}。本次没有发送模型或目标请求") } else if configured {"已解析保存的扫描模型；网络、Broker 和引擎准入仍是未检查，启动时会再校验".to_string()}else{"扫描模型连接未就绪，请到配置中心 → 模型检查；本次没有发送模型或目标请求".to_string()}}),
    )
}
fn blocked_requested_capability(ready: &Value, requested: &[String]) -> Option<String> {
    let items = ready.get("readiness").and_then(Value::as_array)?;
    for name in requested {
        let Some(item) = items.iter().find(|item| item.get("id").and_then(Value::as_str) == Some(name.as_str())) else {
            return Some("capability_not_executable".into());
        };
        if item.get("state").and_then(Value::as_str) != Some("ready") {
            return Some(item.get("reasonCode").and_then(Value::as_str).unwrap_or("capability_not_executable").to_string());
        }
    }
    None
}

fn continuation_waiting(project: i64, operation: &str, task: &str) -> Value {
    json!({"projectId":project,"operationId":operation,"operationState":"waiting","action":"continue_targets","taskId":task,
        "phase":"not_started","reasonCode":"waiting_for_pause","outcomeUnknown":false,"executionStartedByThisOperation":false,
        "freshAllowanceMinted":false,"retryable":false,"nextAction":"scan_status",
        "reason":"暂停尚未完成，续跑在等待，没有开始执行，也不是执行失败。","receiptIsHistorical":true})
}

fn continuation_preflight_rejection(project: i64, operation: &str, task: &str, reason: &str) -> Value {
    json!({"projectId":project,"operationId":operation,"operationState":"rejected","action":"continue_targets","taskId":task,
        "phase":"not_started","reasonCode":reason,"outcomeUnknown":false,"executionStartedByThisOperation":false,
        "freshAllowanceMinted":false,"retryable":false,"nextAction":"scan_preflight",
        "proxyBypassed":false,"sandboxClosed":false,"executionModeChanged":false,
        "reason":"当前预检未通过，没有启动续跑，也没有沿用旧的预检通过。沙箱保持原状，代理没有被绕过，执行方式没有切换。","receiptIsHistorical":true})
}

pub(crate) fn persist_attempt_capability_snapshot(c: &rusqlite::Connection, scan_id: &str, attempt: i64, ready: &Value) -> Result<(), String> {
    if attempt <= 0 { return Ok(()); }
    let _ = c.execute("ALTER TABLE sentinel_scan_attempts ADD COLUMN capability_snapshot_json TEXT NOT NULL DEFAULT ''", []);
    let unfinished = ["browser", "uploadRole", "concurrencyRole", "sandbox"];
    let mut executable = Vec::new();
    if let Some(items) = ready.get("executableCapabilities").and_then(Value::as_array) {
        for item in items {
            if let Some(id) = capability_id(item) {
                if !unfinished.contains(&id.as_str()) { executable.push(id); }
            }
        }
    }
    let mut not_executable = Vec::new();
    if let Some(items) = ready.get("readiness").and_then(Value::as_array) {
        for item in items {
            let method = item.get("executionMethod").and_then(Value::as_str).unwrap_or("");
            let executable_flag = item.get("executable").and_then(Value::as_bool).unwrap_or(false);
            let forced = capability_id(item).is_some_and(|id| unfinished.contains(&id.as_str()));
            if forced || method == "not_executable" || !executable_flag {
                if let Some(id) = capability_id(item) {
                    if !not_executable.iter().any(|saved| saved == &id) { not_executable.push(id); }
                }
            }
        }
    }
    for id in unfinished {
        let named = ready.get("executableCapabilities").and_then(Value::as_array).is_some_and(|items| {
            items.iter().any(|item| capability_id(item).as_deref() == Some(id))
        });
        if named && !not_executable.iter().any(|saved| saved == id) {
            not_executable.push((*id).to_string());
        }
    }
    let config_version = ready.get("readiness").and_then(Value::as_array).and_then(|items| {
        items.iter().find_map(|item| item.get("configVersion").and_then(Value::as_str).filter(|value| !value.is_empty()).map(str::to_string))
    }).or_else(|| ready.get("configVersion").and_then(Value::as_str).filter(|value| !value.is_empty()).map(str::to_string)).unwrap_or_default();
    let route_fingerprint = ready.pointer("/admission/routeFingerprint").and_then(Value::as_str).unwrap_or("");
    let payload = json!({"executable": executable, "notExecutable": not_executable, "configVersion": config_version, "routeFingerprint": route_fingerprint}).to_string();
    c.execute(
        "UPDATE sentinel_scan_attempts SET capability_snapshot_json=?1, updated_at=datetime('now','localtime') WHERE scan_id=?2 AND attempt_number=?3",
        params![payload, scan_id, attempt],
    ).map_err(|error| error.to_string())?;
    Ok(())
}

/// Browser is not an executable capability. A stored snapshot cannot put
/// `browser_action` back into the plan, including a forged executable flag.
pub(crate) fn preflight_snapshot_excludes_browser(_db_path: &std::path::Path, _scan_id: &str, _attempt: i64) -> bool {
    true
}

fn capability_id(value: &Value) -> Option<String> {
    let id = value.as_str().or_else(|| value.get("id").and_then(Value::as_str))?;
    if id.is_empty() || id.len() > 64 || !id.chars().all(|character| character.is_ascii_alphanumeric()) { return None; }
    Some(id.to_string())
}

fn execution_preflight_block(ready: &Value) -> Option<String> {
    if ready.get("startBlocked").and_then(Value::as_bool) == Some(true) {
        Some(ready.get("startBlockedReason").and_then(Value::as_str).unwrap_or("scan_preflight_blocked").to_string())
    } else {
        None
    }
}

fn requested_capability_names(args: &Value) -> Vec<String> {
    args.get("requiredCapabilities")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(|item| item.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

/// Continuation uses the same two gates as start: a blocked preflight, then any
/// requested capability that is not ready. An unavailable browser does not get
/// a remaining-target launch.
fn continuation_capability_block(ready: &Value, requested: &[String]) -> Option<String> {
    execution_preflight_block(ready).or_else(|| blocked_requested_capability(ready, requested))
}

/// Resume uses the same requested-capability gate as start. An empty request
/// does not add a preflight. A named unavailable capability stops before the
/// original attempt is recovered or a new resume is launched.
fn resume_capability_refusal(c: &rusqlite::Connection, project: i64, task: &str, args: &Value) -> Result<(), String> {
    // Resume is an execution like start: the default required set applies even
    // when the caller names no capability.
    let ready = admit_execution(c, project, task, &requested_capability_names(args))?;
    let attempt: i64 = c.query_row("SELECT attempt_count FROM sentinel_scans WHERE id=?1", [task], |row| row.get(0)).unwrap_or(0);
    let _ = persist_attempt_capability_snapshot(c, task, attempt, &ready);
    Ok(())
}

/// The three questions a readiness item answers separately: is the saved
/// configuration usable, was the connection proven with the client the run
/// will use, and is it admitted for this run. A read-only preflight never
/// admits anything.
fn stamp_admission_states(item: &mut Value) {
    let state = item["state"].as_str().unwrap_or("").to_string();
    let reason = item["reasonCode"].as_str().unwrap_or("").to_string();
    let config_valid = !(state == "blocked" && (reason.ends_with("_invalid") || reason.ends_with("_unconfigured")));
    let connection_verified = item["id"] != "modelNetwork" && state == "ready";
    item["configValid"] = json!(config_valid);
    item["connectionVerified"] = json!(connection_verified);
    item["admittedForRun"] = json!(false);
}

enum ModelClientCheck {
    Verified,
    /// Reached through TLS, but the provider has no model list to prove the key.
    AuthUnverified,
}

/// One authenticated request through the same transport, proxy, TLS
/// verification and bearer the run's model calls use, against the provider's
/// model list. It sends no completion and spends no tokens.
fn verify_model_client(profile: &crate::agent_runtime::model::GatewayProfile) -> Result<ModelClientCheck, String> {
    use crate::agent_runtime::model::transport::{self, TransportError, TransportRequest};
    let base = profile.endpoint.strip_suffix("/chat/completions").ok_or("scan_model_endpoint_invalid")?;
    let mut request = TransportRequest::post(format!("{base}/models"), Vec::new());
    request.method = "GET".into();
    request.connect_timeout = std::time::Duration::from_secs(5);
    request.total_timeout = Some(std::time::Duration::from_secs(8));
    request.proxy = profile.proxy.clone();
    request.reject_redirects = true;
    request.max_response_bytes = Some(256 * 1024);
    if let Some(key) = profile.bearer() {
        request = request.header("Authorization", format!("Bearer {key}"));
    }
    match transport::send(request, std::sync::Arc::new(|| false), Box::new(|_| true)) {
        Ok(reply) if (200..300).contains(&reply.status) => Ok(ModelClientCheck::Verified),
        Ok(reply) if matches!(reply.status, 401 | 403) => Err("scan_model_auth_rejected".into()),
        Ok(reply) if matches!(reply.status, 404 | 405) => Ok(ModelClientCheck::AuthUnverified),
        Ok(reply) if reply.status == 407 => Err("scan_model_proxy_auth_rejected".into()),
        Ok(_) => Err("scan_model_unhealthy".into()),
        Err(TransportError::Setup(_)) => Err("scan_model_client_invalid".into()),
        Err(TransportError::Timeout(_)) => Err("scan_model_unreachable".into()),
        Err(TransportError::Network(message)) | Err(TransportError::Interrupted(message)) => {
            let lower = message.to_ascii_lowercase();
            Err(if lower.contains("certificate") || lower.contains("tls") || lower.contains("handshake") { "scan_model_tls_failed" } else { "scan_model_unreachable" }.into())
        }
        Err(TransportError::Cancelled(_)) => Err("scan_model_unreachable".into()),
    }
}

/// Execution admission shared by start, resume and continuation: the required
/// readiness set, any capability the caller named, then the model route proven
/// with the real client over every proxy the run may rotate through.
fn admit_execution(c: &rusqlite::Connection, project: i64, task: &str, requested: &[String]) -> Result<Value, String> {
    let mut ready = preflight(c, project, task)?;
    if let Some(reason) = execution_preflight_block(&ready) {
        return Err(reason);
    }
    if let Some(code) = blocked_requested_capability(&ready, requested) {
        return Err(code);
    }
    let settings = agent_policy_settings_for_scan(c, task)?;
    let environment = model_runtime_env(&settings)?;
    let direct_profile = crate::model_configuration::gateway_profile(&environment.as_connection(), None)?;
    let host_bridge = crate::agent_host_sidecar::is_model_bridge_endpoint(&direct_profile.endpoint);
    let mut routes: Vec<Option<String>> = if host_bridge { Vec::new() } else {
        approved_agent_proxies(&settings).into_iter().map(|(_, url)| Some(url)).take(4).collect()
    };
    if routes.is_empty() {
        routes.push(None);
    }
    let mut auth_verified = true;
    for route in &routes {
        let profile = crate::model_configuration::gateway_profile(&environment.as_connection(), route.as_deref())?;
        if let ModelClientCheck::AuthUnverified = verify_model_client(&profile)? {
            auth_verified = false;
        }
    }
    if let Some(items) = ready["readiness"].as_array_mut() {
        for item in items.iter_mut() {
            if item["id"] == "modelNetwork" {
                item["connectionVerified"] = json!(true);
                item["state"] = json!(if auth_verified { "ready" } else { "unchecked" });
                item["executable"] = json!(auth_verified);
                item["reasonCode"] = if auth_verified { Value::Null } else { json!("scan_model_auth_unverified") };
                item["detail"] = if host_bridge {
                    json!(if auth_verified { "已通过本机 Host 连接完成 Grok OAuth 鉴权；上游请求使用 Host 网络配置" } else { "本机 Host 可达；供应商未提供模型列表，Grok OAuth 鉴权未单独核对" })
                } else {
                    json!(format!("已用运行时客户端经 {} 条路由连通模型并完成 TLS 校验{}", routes.len(), if auth_verified { "与鉴权" } else { "；供应商未提供模型列表，鉴权未单独核对" }))
                };
            }
            let executable = item["executable"] == true;
            item["admittedForRun"] = json!(executable);
        }
    }
    let config_version = ready["readiness"].as_array().and_then(|items| items.iter().find_map(|item| item["configVersion"].as_str().filter(|v| !v.is_empty()).map(str::to_string))).unwrap_or_default();
    let mut proxies: Vec<String> = approved_agent_proxies(&settings).into_iter().map(|(_, url)| url).collect();
    proxies.sort();
    let no_proxy = settings.get("noProxy").and_then(Value::as_str).unwrap_or("").to_string();
    let route_fingerprint = format!("{:x}", sha2::Sha256::digest(json!([config_version, proxies, no_proxy]).to_string().as_bytes()));
    ready["admission"] = json!({"configValid": true, "connectionVerified": true, "authVerified": auth_verified, "admittedForRun": true, "routes": routes.len(), "routeFingerprint": route_fingerprint});
    Ok(ready)
}

/// The model and proxy routes the attempt was admitted with, if it recorded them.
fn admitted_route_fingerprint(c: &rusqlite::Connection, task: &str, attempt: i64) -> Option<String> {
    let snapshot: String = c.query_row(
        "SELECT capability_snapshot_json FROM sentinel_scan_attempts WHERE scan_id=?1 AND attempt_number=?2",
        params![task, attempt], |row| row.get(0),
    ).ok()?;
    serde_json::from_str::<Value>(&snapshot).ok()?["routeFingerprint"].as_str().filter(|v| !v.is_empty()).map(str::to_string)
}

pub(crate) fn fresh_execution_preflight_block(c: &rusqlite::Connection, project: i64, task: &str) -> Result<Option<String>, String> {
    let ready = preflight(c, project, task)?;
    Ok(execution_preflight_block(&ready))
}

fn arguments(action: &str, args: &Value) -> Result<(), String> {
    match action {
        "list" => {
            let input: List =
                serde_json::from_value(args.clone()).map_err(|_| "scan_arguments_invalid")?;
            if input.limit.is_some_and(|v| !(1..=20).contains(&v))
                || input.offset.is_some_and(|v| !(0..=1_000_000).contains(&v))
            {
                return Err("scan_arguments_invalid".into());
            }
        }
        "from_assets" => {
            let input: FromAssets =
                serde_json::from_value(args.clone()).map_err(|_| "scan_arguments_invalid")?;
            if input.asset_ids.is_empty()
                || input.asset_ids.len() > 128
                || input.asset_ids.iter().any(|v| *v < 1)
                || input.asset_ids.iter().collect::<HashSet<_>>().len() != input.asset_ids.len()
                || input
                    .max_model_requests
                    .is_some_and(|v| !(1..=96).contains(&v))
                || input
                    .max_model_tokens
                    .is_some_and(|v| !(512..=2_000_000).contains(&v))
                || input
                    .scan_mode
                    .as_deref()
                    .is_some_and(|v| !matches!(v, "quick" | "standard" | "deep"))
            {
                return Err("scan_arguments_invalid".into());
            }
        }
        "status" | "preflight" | "recovery" | "start" | "pause" | "resume" | "continue_targets" => {
            let input: Task =
                serde_json::from_value(args.clone()).map_err(|_| "scan_arguments_invalid")?;
            if input.task_id.is_empty()
                || input.task_id.len() > 128
                || input.task_id.chars().any(char::is_control)
            {
                return Err("scan_arguments_invalid".into());
            }
        }
        "results" => { results::Results::parse(args)?; }
        "authorize_capability" => {
            let input: AuthorizeCapability =
                serde_json::from_value(args.clone()).map_err(|_| "scan_arguments_invalid")?;
            if input.scan_id.is_empty()
                || input.scan_id.len() > 128
                || input.scan_id.chars().any(char::is_control)
                || input.classes.len() > 8
                || input.classes.iter().any(|c| c.is_empty() || c.len() > 32)
                || input.attempt_number.is_some_and(|v| v <= 0)
                || input.attested_by.as_deref().is_some_and(|v| v.len() > 64)
            {
                return Err("scan_arguments_invalid".into());
            }
        }
        "fingerprint_search" | "fingerprint_read" => { fingerprint_reference::validate_arguments(action, args)?; }
        "deliveries" => {
            if args.get("limit").is_some_and(|limit| !limit.as_i64().is_some_and(|value| (1..=64).contains(&value))) {
                return Err("scan_arguments_invalid".into());
            }
        }
        "deliveries_ack" => {
            let acks = args.get("acks").and_then(Value::as_array).ok_or("scan_arguments_invalid")?;
            if acks.is_empty() || acks.len() > 64 || acks.iter().any(|ack| {
                let event = ack.get("eventId").and_then(Value::as_str).unwrap_or("");
                event.is_empty() || event.len() > 128 || !matches!(ack.get("outcome").and_then(Value::as_str), Some("delivered" | "rejected"))
            }) {
                return Err("scan_arguments_invalid".into());
            }
        }
        "bind" | "command" | "command_ack" | "review" | "verify" | "retry_delivery" => {
            let text = |key| args.get(key).and_then(Value::as_str).unwrap_or("");
            let task = text("taskId");
            let conversation = text("conversationId");
            let needs_conversation = !matches!(action, "command_ack" | "review" | "verify" | "retry_delivery");
            if needs_conversation && (task.is_empty() || task.len() > 128 || task.chars().any(char::is_control)
                || conversation.is_empty() || conversation.len() > 128 || conversation.chars().any(char::is_control)) {
                return Err("scan_arguments_invalid".into());
            }
            if action == "command" {
                let key = text("idempotencyKey");
                let kind = text("kind");
                let payload = text("payload");
                if key.is_empty() || key.len() > 128 || kind.is_empty() || kind.len() > 64 || payload.is_empty() || payload.len() > 4000 {
                    return Err("scan_arguments_invalid".into());
                }
            }
            if matches!(action, "command_ack" | "review" | "verify" | "retry_delivery") && (text("commandId").is_empty() || text("commandId").len() > 128) {
                return Err("scan_arguments_invalid".into());
            }
            if action == "review" && (text("reviewerId").trim().is_empty() || text("reviewerId").len() > 128 || text("note").trim().is_empty() || text("note").len() > 4000 || !matches!(text("outcome"), "passed" | "failed")) {
                return Err("scan_arguments_invalid".into());
            }
        }
        "reassociate_binding" => {
            let text = |key| args.get(key).and_then(Value::as_str).unwrap_or("");
            let task = text("taskId");
            let conversation = text("conversationId");
            if task.is_empty() || task.len() > 128 || task.chars().any(char::is_control)
                || (!conversation.is_empty() && (conversation.len() > 128 || conversation.chars().any(char::is_control))) {
                return Err("scan_arguments_invalid".into());
            }
        }
        _ => return Err("scan_action_unavailable".into()),
    }
    Ok(())
}
fn creation_receipt(
    c: &rusqlite::Connection,
    project: i64,
    operation: &str,
    scan: &SentinelScan,
) -> Result<Value, String> {
    let receipt = json!({"projectId":project,"operationId":operation,"operationState":"completed","action":"from_assets","task":projection(c,scan)?,"receiptIsHistorical":true});
    let changed=c.execute("UPDATE agent_scan_operations SET state='completed',receipt_json=?3 WHERE project_id=?1 AND operation_id=?2 AND state='prepared'",params![project,operation,receipt.to_string()]).map_err(|_|"scan_receipt_unavailable")?;
    if changed != 1 {
        return Err("scan_operation_state_changed".into());
    }
    Ok(receipt)
}

fn claim_checked(
    c: &rusqlite::Connection,
    project: i64,
    operation: &str,
    action: &str,
    args: &Value,
    admission:impl FnOnce(&rusqlite::Connection)->Result<(),String>,
) -> Result<Option<Value>, String> {
    if operation.is_empty() || operation.len() > 128 || operation.chars().any(char::is_control) {
        return Err("scan_operation_invalid".into());
    }
    c.execute_batch(SCHEMA)
        .map_err(|_| "scan_operation_store_unavailable")?;
    let tx = rusqlite::Transaction::new_unchecked(c, rusqlite::TransactionBehavior::Immediate)
        .map_err(|_| "scan_operation_store_unavailable")?;
    if action == "pause" {
        present(&tx, project)?;
    } else {
        active(&tx, project)?;
    }
    let digest = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(args).map_err(|_| "scan_arguments_invalid")?)
    );
    let old:Option<(String,String,String,String)>=tx.query_row("SELECT action,arguments_sha256,state,receipt_json FROM agent_scan_operations WHERE project_id=?1 AND operation_id=?2",params![project,operation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|_|"scan_operation_store_unavailable")?;
    if let Some((verb, hash, state, receipt)) = old {
        if verb != action || hash != digest {
            return Err("scan_operation_conflict".into());
        }
        return if state == "completed" {
            serde_json::from_str(&receipt)
                .map(Some)
                .map_err(|_| "scan_receipt_unavailable".into())
        } else {
            Ok(Some(
                json!({"projectId":project,"operationId":operation,"operationState":"uncertain","outcomeUnknown":true,"retryable":false,"action":action,"taskId":args.get("taskId"),"reason":"上次操作的结果未确认；请读取真实任务状态，不自动重发"}),
            ))
        };
    }
    admission(&tx)?;
    tx.execute("INSERT INTO agent_scan_operations(project_id,operation_id,action,arguments_sha256,state) VALUES(?1,?2,?3,?4,'prepared')",params![project,operation,action,digest]).map_err(|_|"scan_operation_store_unavailable")?;
    tx.commit()
        .map_err(|_| "scan_operation_prepare_unconfirmed")?;
    Ok(None)
}

/// The Host injects the conversation of the verified run that made a bridge
/// call. It replaces whatever conversation the model named, so results can only
/// return to the conversation that asked; a `host:` id the Host did not inject
/// is refused as forged.
pub(crate) fn scope_host_conversation(action: &str, mut args: Value, host: Option<&str>) -> Result<Value, String> {
    let prefix = super::task_binding::HOST_CONVERSATION_PREFIX;
    let named = args.get("conversationId").and_then(Value::as_str).unwrap_or("").to_string();
    if named.starts_with(prefix) {
        return Err("scan_arguments_invalid".into());
    }
    let Some(host) = host else { return Ok(args) };
    if host.is_empty() || host.len() > 100 || !host.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("bridge_context_invalid".into());
    }
    let always = matches!(action, "bind" | "command" | "reassociate_binding");
    let when_named = matches!(action, "continue_targets" | "start" | "pause" | "resume");
    if args.is_object() && (always || (when_named && !named.is_empty())) {
        args["conversationId"] = json!(format!("{prefix}{host}"));
    }
    Ok(args)
}

/// Pause and cancel stop dispatch through the scan's own pause control. A scan
/// that is not running has nothing to stop. The queued control row records the
/// outcome, so the runner does not apply it a second time.
fn apply_execution_control(app: Option<&AppHandle>, c: &rusqlite::Connection, project: i64, task: &str, receipt: &mut Value) -> Result<(), String> {
    let status: String = c.query_row("SELECT status FROM sentinel_scans WHERE id=?1 AND project_id=?2", params![task, project], |row| row.get(0))
        .map_err(|_| "scan_task_unavailable".to_string())?;
    // A queued scan has no runner to stop yet, and a failed pause request may be
    // transient: both leave the control queued for the runner, which claims it
    // at its first check. Only a control that took effect is marked applied.
    let outcome = match status.as_str() {
        "scanning" => match app {
            Some(app) => pause_sentinel_scan(app.state(), task.into()).map(|_| "pause_requested"),
            None => Err("scan_executor_unavailable".to_string()),
        },
        "queued" => Err("queued_for_runner".to_string()),
        _ => Ok("not_running"),
    };
    let command = receipt["commandId"].as_str().unwrap_or("").to_string();
    if outcome.is_ok() {
        c.execute(
            "UPDATE nest_execution_controls SET state='applied' WHERE command_id=?1 AND state='queued'",
            [&command],
        ).map_err(|error| error.to_string())?;
    }
    receipt["dispatchStopped"] = json!(outcome.is_ok());
    receipt["engineStopped"] = json!(outcome.is_ok());
    receipt["engineControl"] = json!(match &outcome { Ok(kind) => kind.to_string(), Err(error) => error.clone() });
    Ok(())
}

fn require_named_conversation_binding(c: &rusqlite::Connection, project: i64, args: &Value) -> Result<(), String> {
    let conversation = args.get("conversationId").and_then(Value::as_str).unwrap_or("");
    if conversation.is_empty() { return Ok(()); }
    let task = args.get("taskId").and_then(Value::as_str).unwrap_or("");
    super::task_binding::require_existing_binding(c, project, conversation, task)
}

fn reassociate_revoked_in(c: &rusqlite::Connection, project: i64, args: &Value) -> Result<Value, String> {
    let task = args.get("taskId").and_then(Value::as_str).unwrap_or("");
    let to = args.get("conversationId").and_then(Value::as_str).unwrap_or("");
    let mut statement = match c.prepare(
        "SELECT conversation_id FROM nest_session_task_bindings WHERE project_id=?1 AND task_id=?2 ORDER BY conversation_id",
    ) {
        Ok(statement) => statement,
        Err(error) => {
            let message = error.to_string();
            if message.contains("no such table") { return Err("task_binding_missing".into()); }
            return Err(message);
        }
    };
    let sources = statement.query_map(params![project, task], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let from = match sources.len() {
        1 => sources[0].as_str(),
        0 => return Err("task_binding_missing".into()),
        _ => return Err("task_not_unique".into()),
    };
    // Only the Host can tell whether one of its conversations is gone; it names
    // that conversation when it has checked. Without it the binding stays put.
    if let Some(source) = super::task_binding::host_conversation(from) {
        if args.get("hostSourceGone").and_then(Value::as_str) != Some(source) {
            return Err("reassociation_source_still_available".into());
        }
    }
    let (binding_version, attempt) = observed_write_versions(args)?;
    super::task_binding::reassociate_revoked_binding_at(c, project, task, from, to, binding_version, attempt)
}

fn observed_write_versions(args: &Value) -> Result<(Option<i64>, Option<i64>), String> {
    let version = |key: &str| match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_i64().filter(|number| *number >= 0).map(Some).ok_or_else(|| "scan_arguments_invalid".to_string()),
    };
    let binding_version = version("bindingVersion")?;
    let attempt = match (version("attempt")?, version("attemptNumber")?) {
        (Some(left), Some(right)) if left != right => return Err("scan_arguments_invalid".into()),
        (Some(value), _) | (_, Some(value)) => Some(value),
        (None, None) => None,
    };
    Ok((binding_version, attempt))
}

fn submit_command_in(c: &rusqlite::Connection, project: i64, args: &Value) -> Result<Value, String> {
    require_named_conversation_binding(c, project, args)?;
    super::task_binding::submit(
        c,
        project,
        args["conversationId"].as_str().unwrap(),
        args["taskId"].as_str().unwrap(),
        args["idempotencyKey"].as_str().unwrap(),
        args["kind"].as_str().unwrap(),
        args["payload"].as_str().unwrap(),
    )
}

fn continue_targets_in(c: &rusqlite::Connection, db_path: &std::path::Path, project: i64, operation: &str, args: &Value) -> Result<Value, String> {
    let task = args["taskId"].as_str().unwrap();
    let conversation = args.get("conversationId").and_then(Value::as_str).unwrap_or("");
    if !conversation.is_empty() {
        super::task_binding::require_existing_binding(c, project, conversation, task)?;
    }
    let submitted = if !conversation.is_empty() {
        let attempt: i64 = c.query_row("SELECT attempt_count FROM sentinel_scans WHERE id=?1 AND project_id=?2", params![task, project], |row| row.get(0)).unwrap_or(0);
        Some(super::task_binding::submit(c, project, conversation, task, operation, "continue_targets", &format!("continue:{task}:{attempt}"))?)
    } else {
        None
    };
    let before = diagnostics::execution_snapshot(c, project, task).ok();
    let scan_status: String = c.query_row("SELECT status FROM sentinel_scans WHERE id=?1 AND project_id=?2", params![task, project], |row| row.get(0)).unwrap_or_default();
    let (phase, receipt) = if scan_status == "pausing" {
        if let Some(command_id) = submitted.as_ref().and_then(|value| value.get("commandId")).and_then(Value::as_str) {
            let _ = super::task_binding::mark_waiting(c, project, command_id);
        }
        ("completed", continuation_waiting(project, operation, task))
    } else {
        let requested = requested_capability_names(args);
        let current_attempt: i64 = c.query_row("SELECT attempt_count FROM sentinel_scans WHERE id=?1 AND project_id=?2", params![task, project], |row| row.get(0)).unwrap_or(0);
        // A continuation runs inside the admitted attempt, so it must use the
        // model and proxy routes that attempt was admitted with.
        let preflight_outcome = admit_execution(c, project, task, &requested).and_then(|ready| {
            let admitted = admitted_route_fingerprint(c, task, current_attempt);
            let now = ready.pointer("/admission/routeFingerprint").and_then(Value::as_str).map(str::to_string);
            match (admitted, now) {
                (Some(admitted), Some(now)) if admitted != now => Err("continuation_configuration_changed".to_string()),
                _ => Ok(ready),
            }
        });
        let preflight_reason = match &preflight_outcome {
            Ok(ready) => continuation_capability_block(ready, &requested),
            Err(reason) => Some(reason.clone()),
        };
        if let Some(reason) = preflight_reason {
            if let Some(command_id) = submitted.as_ref().and_then(|value| value.get("commandId")).and_then(Value::as_str) {
                let _ = super::task_binding::mark_rejected(c, project, command_id, &reason);
            }
            ("completed", continuation_preflight_rejection(project, operation, task, &reason))
        } else {
            if let Ok(ready) = &preflight_outcome {
                let attempt: i64 = c.query_row("SELECT attempt_count FROM sentinel_scans WHERE id=?1 AND project_id=?2", params![task, project], |row| row.get(0)).unwrap_or(0);
                let _ = persist_attempt_capability_snapshot(c, task, attempt, ready);
            }
            match begin_remaining_target_continuation(db_path, task) {
                Ok(value) => (
                    "completed",
                    json!({"projectId":project,"operationId":operation,"operationState":"completed","action":"continue_targets","taskId":task,
                        "continuation":value,"freshAllowanceMinted":false,"executionStartedByThisOperation":value.get("executionStarted").and_then(Value::as_bool).unwrap_or(false),"receiptIsHistorical":true}),
                ),
                Err(reason) => {
                    if let Some(command_id) = submitted.as_ref().and_then(|value| value.get("commandId")).and_then(Value::as_str) {
                        let _ = super::task_binding::mark_execution_failed(c, project, command_id, &reason);
                    }
                    diagnostics::failed_receipt(c, project, operation, "continue_targets", args, before.as_ref(), &reason)
                }
            }
        }
    };
    let changed = c.execute("UPDATE agent_scan_operations SET state=?3,receipt_json=?4 WHERE project_id=?1 AND operation_id=?2 AND state='prepared'", params![project, operation, phase, receipt.to_string()]).map_err(|_| "scan_operation_outcome_unknown")?;
    if changed != 1 { return Err("scan_operation_outcome_unknown".into()); }
    Ok(receipt)
}

pub(crate) fn dispatch(
    app: &AppHandle,
    project: i64,
    action: &str,
    args: Value,
    operation: &str,
) -> Result<Value, String> {
    let state = app.state::<AppState>();
    dispatch_on(Some(app), &state.db_path, &state.app_data_dir, project, action, args, operation)
}

/// Engine start, pause and resume need the application; everything else runs
/// on the desktop database alone.
pub(crate) fn dispatch_on(
    app: Option<&AppHandle>,
    db_path: &std::path::Path,
    app_data_dir: &std::path::Path,
    project: i64,
    action: &str,
    args: Value,
    operation: &str,
) -> Result<Value, String> {
    // The Host refuses any result that does not name the project it asked
    // about, so every receipt carries the project this call ran in.
    let mut value = dispatch_in(app, db_path, app_data_dir, project, action, args, operation)?;
    if let Some(object) = value.as_object_mut() {
        match object.get("projectId").and_then(Value::as_i64) {
            Some(named) if named != project => return Err("scan_project_binding_changed".into()),
            Some(_) => {}
            None => { object.insert("projectId".into(), json!(project)); }
        }
    }
    Ok(value)
}

fn dispatch_in(
    app: Option<&AppHandle>,
    db_path: &std::path::Path,
    app_data_dir: &std::path::Path,
    project: i64,
    action: &str,
    args: Value,
    operation: &str,
) -> Result<Value, String> {
    arguments(action, &args)?;
    if app.is_none() && matches!(action, "start" | "pause" | "resume") {
        return Err("scan_executor_unavailable".into());
    }
    let c = db::open(db_path)?;
    if matches!(action, "list" | "status" | "preflight" | "recovery" | "results" | "fingerprint_search" | "fingerprint_read" | "pause" | "deliveries" | "deliveries_ack") {
        present(&c, project)?;
    } else {
        active(&c, project)?;
    }
    if matches!(action, "fingerprint_search" | "fingerprint_read") {
        return fingerprint_reference::dispatch(&c, project, action, &args);
    }
    if action == "authorize_capability" {
        // Operator path only: the Host's authorization channel reaches this, never
        // a model tool. The scan must belong to this project, then the immutable
        // operator record is written (attestation enforced by the writer).
        let input: AuthorizeCapability =
            serde_json::from_value(args).map_err(|_| "scan_arguments_invalid")?;
        owned_scope(&c, project, &input.scan_id)?;
        let attempt = match input.attempt_number {
            Some(value) if value > 0 => value,
            Some(_) => return Err("scan_arguments_invalid".into()),
            None => c
                .query_row(
                    "SELECT attempt_count FROM sentinel_scans WHERE id=?1",
                    [&input.scan_id],
                    |r| r.get::<_, i64>(0),
                )
                .map_err(|_| "scan_task_unavailable")?
                .max(1),
        };
        let attested_by = input.attested_by.as_deref().unwrap_or("operator");
        super::set_scan_capability_authorization_on(
            &c,
            &input.scan_id,
            attempt,
            input.legal_attestation,
            &input.classes,
            attested_by,
        )
        .map_err(|error| {
            // Stable, fail-closed codes for the Host. Each is a pre-commit
            // validation or the immutability guard: nothing new started.
            if error.contains("合法授权") {
                "scan_authorization_attestation_required".to_string()
            } else if error.contains("未知的高危能力类") {
                "scan_authorization_class_invalid".to_string()
            } else if error.contains("不可更改") {
                "scan_authorization_immutable".to_string()
            } else if error.contains("任务不存在") {
                "scan_task_unavailable".to_string()
            } else {
                error
            }
        })?;
        let mut classes = input.classes.clone();
        classes.sort();
        classes.dedup();
        return Ok(json!({
            "projectId": project,
            "scanId": input.scan_id,
            "attemptNumber": attempt,
            "authorized": true,
            "classes": classes,
            "attestedBy": attested_by,
        }));
    }
    if action == "list" {
        let input: List = serde_json::from_value(args).map_err(|_| "scan_arguments_invalid")?;
        let limit = input.limit.unwrap_or(20);
        let offset = input.offset.unwrap_or(0);
        let mut statement=c.prepare("SELECT id FROM sentinel_scans s WHERE project_id=?1 AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans d WHERE d.scan_id=s.id) ORDER BY updated_at DESC,id DESC LIMIT ?2 OFFSET ?3").map_err(|_|"scan_list_unavailable")?;
        let ids: Vec<String> = statement
            .query_map(params![project, limit, offset], |r| r.get(0))
            .map_err(|_| "scan_list_unavailable")?
            .collect::<rusqlite::Result<_>>()
            .map_err(|_| "scan_list_unavailable")?;
        let total:i64=c.query_row("SELECT COUNT(*) FROM sentinel_scans s WHERE project_id=?1 AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans d WHERE d.scan_id=s.id)",[project],|r|r.get(0)).map_err(|_|"scan_list_unavailable")?;
        let tasks: Vec<Value> = ids
            .iter()
            .map(|id| owned(&c, project, id).and_then(|s| projection(&c, &s)))
            .collect::<Result<_, _>>()?;
        return Ok(
            json!({"projectId":project,"tasks":tasks,"total":total,"offset":offset,"nextOffset":if offset+limit<total{Some(offset+limit)}else{None},"source":"desktop_scanning_owner"}),
        );
    }
    if action == "status" {
        let task: Task = serde_json::from_value(args).map_err(|_| "scan_arguments_invalid")?;
        return Ok(
            json!({"projectId":project,"task":projection(&c,&owned(&c,project,&task.task_id)?)?}),
        );
    }
    if action == "preflight" {
        let task: Task = serde_json::from_value(args).map_err(|_| "scan_arguments_invalid")?;
        return preflight(&c, project, &task.task_id);
    }
    if action == "recovery" {
        let task: Task = serde_json::from_value(args).map_err(|_| "scan_arguments_invalid")?;
        return recovery::read(&c, project, &task.task_id);
    }
    if action == "deliveries" {
        // The Host pulls on a timer, so this is also the recurring pass that
        // hands saved supplements to a Coordinator that became live later.
        let _ = super::task_binding::deliver_supplements(&c, project, None);
        return super::task_binding::host_outbox_pull(&c, project, args.get("limit").and_then(Value::as_i64).unwrap_or(32));
    }
    if action == "deliveries_ack" {
        let acks = args["acks"].as_array().into_iter().flatten()
            .map(|ack| (ack["eventId"].as_str().unwrap_or("").to_string(), ack["outcome"].as_str().unwrap_or("").to_string()))
            .collect::<Vec<_>>();
        return super::task_binding::host_outbox_ack(&c, project, &acks);
    }
    if action == "bind" {
        let (binding_version, attempt) = observed_write_versions(&args)?;
        super::task_binding::refuse_stale_page_write(&c, project, args["taskId"].as_str().unwrap(), binding_version, attempt)?;
        return super::task_binding::bind(&c, project, args["conversationId"].as_str().unwrap(), args["taskId"].as_str().unwrap());
    }
    if action == "command" {
        let (binding_version, attempt) = observed_write_versions(&args)?;
        let task = args["taskId"].as_str().unwrap();
        super::task_binding::refuse_stale_page_write(&c, project, task, binding_version, attempt)?;
        let mut receipt = submit_command_in(&c, project, &args)?;
        let kind = args["kind"].as_str().unwrap();
        if matches!(kind, "pause" | "cancel") && receipt["replayed"] != json!(true) {
            apply_execution_control(app, &c, project, task, &mut receipt)?;
        }
        if kind == "supplement" && receipt["state"] == json!("accepted") {
            // Hand it to a live Coordinator now; otherwise the next delivery
            // pass or the runner does, and it stays accepted until then.
            let _ = super::task_binding::deliver_supplements(&c, project, Some(task));
            if let Some(id) = receipt["commandId"].as_str() {
                if let Ok(state) = c.query_row("SELECT state FROM nest_task_commands WHERE command_id=?1", [id], |row| row.get::<_, String>(0)) {
                    receipt["state"] = json!(state);
                }
            }
        }
        return Ok(receipt);
    }
    if action == "command_ack" {
        return super::task_binding::mark_delivered(&c, project, args["commandId"].as_str().unwrap());
    }
    if action == "review" {
        return super::task_binding::record_review(&c, project, args["commandId"].as_str().unwrap(), args["reviewerId"].as_str().unwrap(), args["note"].as_str().unwrap(), args["outcome"].as_str().unwrap());
    }
    if action == "verify" {
        return super::task_binding::mark_verified(&c, project, args["commandId"].as_str().unwrap());
    }
    if action == "retry_delivery" {
        return super::task_binding::retry_pending_delivery(&c, project, args["commandId"].as_str().unwrap());
    }
    if action == "reassociate_binding" {
        return reassociate_revoked_in(&c, project, &args);
    }
    if action == "results" {
        return results::read(&c, project, results::Results::parse(&args)?);
    }
    // Validate scope before storing a mutation receipt. The original owner also
    // checks admission after this adapter yields; this is no replacement gate.
    if action == "from_assets" {
        let input: FromAssets =
            serde_json::from_value(args.clone()).map_err(|_| "scan_arguments_invalid")?;
        for id in input.asset_ids {
            let url:Option<String>=c.query_row("SELECT COALESCE(NULLIF(a.link,''),a.host) FROM assets a JOIN project_assets pa ON pa.asset_id=a.id WHERE pa.project_id=?1 AND pa.asset_id=?2 AND pa.is_deleted=0",params![project,id],|r|r.get(0)).optional().map_err(|_|"scan_asset_unavailable")?;
            let url = url
                .and_then(|u| reqwest::Url::parse(&u).ok())
                .ok_or("scan_asset_unavailable")?;
            if !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.fragment().is_some()
            {
                return Err("scan_asset_url_invalid".into());
            }
        }
    } else {
        let task: Task =
            serde_json::from_value(args.clone()).map_err(|_| "scan_arguments_invalid")?;
        let scan = owned(&c, project, &task.task_id)?;
        if scan.scan_type != "web" {
            return Err("scan_channel_web_only".into());
        }
        let observed_attempt = match (task.attempt, task.attempt_number) {
            (Some(left), Some(right)) if left != right => return Err("scan_arguments_invalid".into()),
            (Some(value), _) | (_, Some(value)) => Some(value),
            (None, None) => None,
        };
        // The engine resume, continuation, and preflight paths keep working when
        // the caller has no version. A carried older version stops before they run.
        super::task_binding::refuse_stale_page_write(&c, project, &task.task_id, task.binding_version, observed_attempt)?;
    }
    if matches!(action, "start" | "pause" | "resume") {
        require_named_conversation_binding(&c, project, &args)?;
    }
    let claimed = claim_checked(&c, project, operation, action, &args,|snapshot|{
        if action=="resume" {super::scan_resume_financial_preflight_in(snapshot,args["taskId"].as_str().unwrap())}else{Ok(())}
    });
    let receipt = match claimed {
        Ok(receipt) => receipt,
        Err(code) if action == "resume" && recovery::financial_reason(&code) => {
            return recovery::refusal(&c, project, operation, &args, &code);
        }
        Err(reason) => return Err(reason),
    };
    if let Some(receipt) = receipt {
        return Ok(receipt);
    }
    if action == "continue_targets" {
        return continue_targets_in(&c, db_path, project, operation, &args);
    }
    let before = args.get("taskId").and_then(Value::as_str)
        .and_then(|id| diagnostics::execution_snapshot(&c, project, id).ok());
    let original_dispatch = if action=="resume" {
        let task=args["taskId"].as_str().unwrap();
        let scan=owned(&c,project,task)?;
        super::web_recovery_available_in(&c,task,scan.attempt_count)
            .then_some(scan.attempt_count)
    }else{None};
    let outcome = match action {
        "from_assets" => {
            let input: FromAssets =
                serde_json::from_value(args.clone()).map_err(|_| "scan_arguments_invalid")?;
            create_sentinel_asset_scan_with_receipt_in(
                &c,
                project,
                input.asset_ids,
                input.scan_mode,
                (input.max_model_requests, input.max_model_tokens),
                |tx, scan| creation_receipt(tx, project, operation, scan).map(|_| ()),
            )
            .and_then(|scan| {
                super::scan_detail_store::provision_new_scan(db_path, app_data_dir, &scan.id)?;
                Ok(scan)
            })
        }
        "start" => {
            let task = args["taskId"].as_str().unwrap();
            let requested = serde_json::from_value::<Task>(args.clone()).map_err(|_| "scan_arguments_invalid".to_string())?;
            match admit_execution(&c, project, task, &requested.required_capabilities) {
                Err(reason) => Err(reason),
                Ok(ready) => match app.ok_or_else(|| "scan_executor_unavailable".to_string()).and_then(|app| confirm_sentinel_scan(app.clone(), app.state(), task.into())) {
                    Ok(scan) => {
                        let _ = persist_attempt_capability_snapshot(&c, task, scan.attempt_count, &ready);
                        Ok(scan)
                    }
                    Err(error) => Err(error),
                },
            }
        }
        "pause" => app.ok_or_else(|| "scan_executor_unavailable".to_string()).and_then(|app| pause_sentinel_scan(app.state(), args["taskId"].as_str().unwrap().into())),
        "resume" => {
            let task = args["taskId"].as_str().unwrap();
            match resume_capability_refusal(&c, project, task, &args) {
                Err(code) => Err(code),
                Ok(()) => match app {
                    None => Err("scan_executor_unavailable".to_string()),
                    Some(app) => if let Some(attempt) = original_dispatch {
                        resolve_frontend_recon_worker(app).and_then(|worker|
                            super::recover_web_attempt_impl(db_path,app_data_dir,task,attempt,true,&worker)
                        ).and_then(|_|sentinel_scan_by_id(&c,task))
                    } else {
                        resume_sentinel_scan(app.clone(), app.state(), task.into())
                    },
                },
            }
        }
        _ => unreachable!(),
    };
    let (phase, receipt) = match outcome {
        Ok(scan) => (
            "completed",
            json!({"projectId":project,"operationId":operation,"operationState":"completed","action":action,"task":projection(&c,&scan)?,
                "executionRoute":if original_dispatch.is_some(){Some("same_attempt_never_dispatched")}else{None},
                "receiptIsHistorical":true}),
        ),
        Err(reason) => diagnostics::failed_receipt(&c, project, operation, action, &args, before.as_ref(), &reason),
    };
    let changed=c.execute("UPDATE agent_scan_operations SET state=?3,receipt_json=?4 WHERE project_id=?1 AND operation_id=?2 AND state='prepared'",params![project,operation,phase,receipt.to_string()]).map_err(|_|"scan_operation_outcome_unknown")?;
    if changed!=1 {return Err("scan_operation_outcome_unknown".into());}
    Ok(receipt)
}


#[cfg(test)]
mod host_conversation_tests {
    use super::scope_host_conversation;
    use serde_json::json;

    #[test]
    fn the_host_conversation_replaces_the_model_supplied_one() {
        let scoped = scope_host_conversation("bind", json!({"taskId":"t","conversationId":"elsewhere"}), Some("c-1")).unwrap();
        assert_eq!(scoped["conversationId"], json!("host:c-1"));
        let scoped = scope_host_conversation("command", json!({"taskId":"t"}), Some("c-1")).unwrap();
        assert_eq!(scoped["conversationId"], json!("host:c-1"));
        let unnamed = scope_host_conversation("start", json!({"taskId":"t"}), Some("c-1")).unwrap();
        assert!(unnamed.get("conversationId").is_none());
    }

    #[test]
    fn a_host_conversation_the_host_did_not_inject_is_refused() {
        assert!(scope_host_conversation("bind", json!({"taskId":"t","conversationId":"host:c-2"}), None).is_err());
        assert!(scope_host_conversation("bind", json!({"taskId":"t","conversationId":"host:c-2"}), Some("c-1")).is_err());
        assert!(scope_host_conversation("bind", json!({"taskId":"t"}), Some("../x")).is_err());
        let desktop = scope_host_conversation("bind", json!({"taskId":"t","conversationId":"d-1"}), None).unwrap();
        assert_eq!(desktop["conversationId"], json!("d-1"));
    }
}

#[cfg(test)]
mod model_client_tests {
    use super::{verify_model_client, ModelClientCheck};
    use crate::agent_runtime::model::{GatewayProfile, LocalResourcePolicy, ResolvedModelProfile};
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        sync::{atomic::{AtomicUsize, Ordering}, Arc},
        thread,
    };

    fn read_head(stream: &mut TcpStream) -> String {
        let mut bytes = Vec::new();
        let mut byte = [0u8; 1];
        while !bytes.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
            bytes.push(byte[0]);
        }
        String::from_utf8_lossy(&bytes).to_string()
    }

    /// A model service: `/v1/models` answers 200 for the right bearer and 401
    /// otherwise; with `missing` it has no model list (404).
    fn model_server(missing: bool) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                let head = read_head(&mut stream).to_ascii_lowercase();
                let status = if missing {
                    "404 Not Found"
                } else if head.starts_with("get /v1/models ") && head.contains("authorization: bearer good-key") {
                    "200 OK"
                } else {
                    "401 Unauthorized"
                };
                let body = r#"{"data":[]}"#;
                let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        port
    }

    /// A plain HTTP forward proxy that counts what it forwards.
    fn forward_proxy(count: Arc<AtomicUsize>) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut client = stream;
                let head = read_head(&mut client);
                let mut lines = head.split("\r\n");
                let request = lines.next().unwrap_or("").to_string();
                let mut parts = request.split(' ');
                let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
                let Some(rest) = target.strip_prefix("http://") else { continue };
                let (authority, path) = rest.split_once('/').map(|(a, p)| (a.to_string(), format!("/{p}"))).unwrap_or((rest.to_string(), "/".into()));
                count.fetch_add(1, Ordering::SeqCst);
                let mut origin = TcpStream::connect(&authority).unwrap();
                let mut forwarded = format!("{method} {path} HTTP/1.1\r\n");
                for line in lines.filter(|line| !line.is_empty()) {
                    let lower = line.to_ascii_lowercase();
                    if !lower.starts_with("proxy-") && !lower.starts_with("connection:") {
                        forwarded.push_str(line);
                        forwarded.push_str("\r\n");
                    }
                }
                forwarded.push_str("Connection: close\r\n\r\n");
                origin.write_all(forwarded.as_bytes()).unwrap();
                let mut response = Vec::new();
                let _ = origin.read_to_end(&mut response);
                let _ = client.write_all(&response);
            }
        });
        port
    }

    fn profile(port: u16, key: &str, proxy: Option<String>) -> GatewayProfile {
        GatewayProfile::resolve(
            &ResolvedModelProfile { model: "m".into(), api_base: format!("http://127.0.0.1:{port}/v1"), api_key: key.into(), deployment: "local".into() },
            &LocalResourcePolicy { max_output_tokens: None, max_context_tokens: 0 },
            proxy.as_deref(),
        ).unwrap()
    }

    #[test]
    fn the_model_route_is_proven_through_the_configured_proxy_with_the_real_key() {
        let server = model_server(false);
        let forwarded = Arc::new(AtomicUsize::new(0));
        let proxy = forward_proxy(forwarded.clone());
        let checked = verify_model_client(&profile(server, "good-key", Some(format!("http://127.0.0.1:{proxy}"))));
        assert!(matches!(checked, Ok(ModelClientCheck::Verified)));
        assert_eq!(forwarded.load(Ordering::SeqCst), 1, "the check went through the proxy the run uses");
    }

    #[test]
    fn a_rejected_key_or_dead_route_blocks_and_a_missing_model_list_stays_unverified() {
        let server = model_server(false);
        assert_eq!(verify_model_client(&profile(server, "bad-key", None)).err().as_deref(), Some("scan_model_auth_rejected"));
        assert!(matches!(verify_model_client(&profile(model_server(true), "good-key", None)), Ok(ModelClientCheck::AuthUnverified)));
        let closed = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        assert_eq!(verify_model_client(&profile(closed, "good-key", None)).err().as_deref(), Some("scan_model_unreachable"));
        let dead_proxy = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        assert_eq!(verify_model_client(&profile(server, "good-key", Some(format!("http://127.0.0.1:{dead_proxy}")))).err().as_deref(), Some("scan_model_unreachable"));
    }
}

#[cfg(test)]
mod authorize_capability_tests {
    use super::*;
    use serde_json::json;

    /// A fully initialized desktop database with two projects and two scans.
    fn seeded() -> (std::path::PathBuf, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("oviraptor-authz-{}", uuid::Uuid::new_v4()));
        let desktop = root.join("desktop");
        let db_path = crate::db::initialize(&desktop).unwrap();
        {
            let c = crate::db::open(&db_path).unwrap();
            c.execute_batch(
                "INSERT INTO projects(id,name) VALUES(1,'one'),(2,'two');
                 INSERT INTO sentinel_scans(id,project_id,status,attempt_count,scan_type) VALUES('scan-1',1,'completed',1,'web'),('foreign',2,'completed',1,'web');",
            )
            .unwrap();
        }
        (db_path, desktop)
    }

    fn cleanup(desktop: &std::path::Path) {
        let _ = std::fs::remove_dir_all(desktop.parent().unwrap());
    }

    #[test]
    fn operator_authorize_writes_the_immutable_record_scoped_to_the_project() {
        let (db_path, dir) = seeded();
        let result = dispatch_on(
            None, &db_path, &dir, 1, "authorize_capability",
            json!({"scanId":"scan-1","legalAttestation":true,"classes":["code_exec","internal_scope"]}),
            "op-1",
        )
        .unwrap();
        assert_eq!(result["authorized"], true);
        assert_eq!(result["attemptNumber"], 1);
        assert_eq!(result["attestedBy"], "operator");
        let c = crate::db::open(&db_path).unwrap();
        assert!(super::scan_capability_class_authorized(&c, "scan-1", 1, "code_exec").unwrap());
        assert!(super::scan_capability_class_authorized(&c, "scan-1", 1, "internal_scope").unwrap());
        // A class outside the authorized set stays refused — the floor is closed.
        assert!(!super::scan_capability_class_authorized(&c, "scan-1", 1, "egress").unwrap());
        cleanup(&dir);
    }

    #[test]
    fn operator_standing_attribution_is_recorded() {
        let (db_path, dir) = seeded();
        dispatch_on(
            None, &db_path, &dir, 1, "authorize_capability",
            json!({"scanId":"scan-1","legalAttestation":true,"classes":["egress"],"attestedBy":"operator_standing"}),
            "op-1",
        )
        .unwrap();
        let c = crate::db::open(&db_path).unwrap();
        let who: String = c
            .query_row("SELECT attested_by FROM scan_capability_authorizations WHERE scan_id='scan-1' AND attempt_number=1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(who, "operator_standing");
        cleanup(&dir);
    }

    #[test]
    fn authorize_refuses_a_scan_in_another_project() {
        let (db_path, dir) = seeded();
        let err = dispatch_on(
            None, &db_path, &dir, 1, "authorize_capability",
            json!({"scanId":"foreign","legalAttestation":true,"classes":["code_exec"]}),
            "op-1",
        )
        .unwrap_err();
        assert_eq!(err, "scan_task_unavailable");
        cleanup(&dir);
    }

    #[test]
    fn authorize_refuses_high_risk_without_attestation() {
        let (db_path, dir) = seeded();
        let err = dispatch_on(
            None, &db_path, &dir, 1, "authorize_capability",
            json!({"scanId":"scan-1","legalAttestation":false,"classes":["code_exec"]}),
            "op-1",
        )
        .unwrap_err();
        assert_eq!(err, "scan_authorization_attestation_required");
        // Nothing was written: the gate stays closed.
        let c = crate::db::open(&db_path).unwrap();
        assert!(!super::scan_capability_class_authorized(&c, "scan-1", 1, "code_exec").unwrap());
        cleanup(&dir);
    }

    #[test]
    fn authorize_is_immutable_once_written() {
        let (db_path, dir) = seeded();
        dispatch_on(
            None, &db_path, &dir, 1, "authorize_capability",
            json!({"scanId":"scan-1","legalAttestation":true,"classes":["code_exec"]}),
            "op-1",
        )
        .unwrap();
        let err = dispatch_on(
            None, &db_path, &dir, 1, "authorize_capability",
            json!({"scanId":"scan-1","legalAttestation":true,"classes":["egress"]}),
            "op-2",
        )
        .unwrap_err();
        assert_eq!(err, "scan_authorization_immutable");
        cleanup(&dir);
    }

    #[test]
    fn authorize_rejects_an_unknown_class() {
        let (db_path, dir) = seeded();
        let err = dispatch_on(
            None, &db_path, &dir, 1, "authorize_capability",
            json!({"scanId":"scan-1","legalAttestation":true,"classes":["make_coffee"]}),
            "op-1",
        )
        .unwrap_err();
        assert_eq!(err, "scan_authorization_class_invalid");
        cleanup(&dir);
    }
}
