/// The one scope gate for every URL this run may touch (§5.1). HTTP replay, each
/// side of an identity comparison, discovery candidates, the browser entry, every
/// request a browser observed and every redirect target are decided here, by the
/// same rules: the frozen `allowed_origins` of the plan and nothing else.
///
/// `Ok((url, class))` means the request may be sent and what it counts as; the
/// noise screen needs the real method, because without it every candidate looks
/// like an UNKNOWN request and even the authorized target would be refused.
fn agent_scope_assess(
    context: &AgentRunContext,
    url: &str,
    method: &str,
    resource_type: &str,
    source: ScopeSource,
) -> ScopeAssessment {
    let reject = |code: &str, reason: String| ScopeAssessment {
        decision: ScopeDecision::Reject {
            code: code.to_string(),
            reason,
        },
        class: ScopeClass::OutOfScope,
        origin: String::new(),
    };
    let ignore = |class: ScopeClass, reason: String| ScopeAssessment {
        decision: ScopeDecision::Ignore { reason },
        class,
        origin: String::new(),
    };
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return reject("invalid_url", format!("URL 无效：{url}"));
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return reject("invalid_scheme", "只允许 http/https 目标".to_string());
    }
    let Some(host) = parsed.host_str() else {
        return reject("missing_host", "URL 缺少主机名".to_string());
    };
    // SSRF hard floor: a target that is itself an internal / loopback / link-local
    // IP literal (127.0.0.1, 10.x, 169.254.169.254 metadata, [::1], …) is refused
    // regardless of allowed_origins. Deterministic, never the agent's call;
    // internal targets are reachable only when the operator authorized them in the
    // signed scope contract (agent_internal_scope_authorized). DNS names that
    // resolve to internal addresses are caught at the connection pin in
    // agent_pin_public_address.
    if host_is_internal_literal(host) && !agent_internal_scope_authorized(context) {
        return reject(
            "ssrf_internal_denied",
            format!("{host} 是内网/环回/链路本地地址，SSRF 硬守卫默认拒绝；内网测试须由操作者在签名范围合约中显式授权"),
        );
    }
    let candidate = serde_json::json!({
        "url": parsed.as_str(),
        "method": method,
        "resourceType": resource_type,
    });
    if investigation_background_noise(&candidate) {
        return ignore(
            ScopeClass::TelemetryOrNoise,
            format!("{host}{} 属于遥测、埋点、静态资源或设备指纹", parsed.path()),
        );
    }
    let origin = normalize_ownership_domain(host);
    let allowed = context
        .execution_plan
        .allowed_origins
        .iter()
        .any(|rule| domain_matches(host, rule));
    if !allowed {
        // §5.4: an identity-provider jump is session collection only. It is never
        // turned into an authorization probe and never widens the scope.
        if identity_provider_target(url) {
            return ignore(
                ScopeClass::ThirdPartyRequired,
                format!("{host} 是身份提供商跳转，仅用于浏览器显示与会话采集"),
            );
        }
        return match source {
            // An observation is classified, not sent: record the dependency and
            // keep it out of formal APIs and coverage (§5.3).
            ScopeSource::BrowserObserved => ignore(
                ScopeClass::ThirdPartyRequired,
                format!("{host} 不属于本任务冻结的授权域名，只作为第三方依赖摘要"),
            ),
            _ => reject(
                "scope_denied",
                format!(
                    "{host} 不在本任务冻结的授权域名 [{}] 内",
                    context.execution_plan.allowed_origins.join(", ")
                ),
            ),
        };
    }
    let class = if is_agent_static_resource(&normalized_investigation_path(parsed.path()), resource_type)
    {
        ScopeClass::AuthorizedStatic
    } else if matches!(resource_type, "document" | "document-fragment" | "navigation")
        || (parsed.path() != "/" && parsed.path().ends_with('/'))
    {
        ScopeClass::AuthorizedDocument
    } else {
        ScopeClass::AuthorizedBusinessApi
    };
    ScopeAssessment {
        decision: ScopeDecision::Allow {
            normalized_origin: origin.clone(),
            reason: format!("{host} 属于冻结授权域 {origin}"),
        },
        class,
        origin,
    }
}

struct ScopeAssessment {
    decision: ScopeDecision,
    class: ScopeClass,
    origin: String,
}

// ───────────────────────── SSRF hard guard (deterministic, not agent-judged) ──
//
// The scope gate above matches on the host STRING; it never resolved a name to
// an IP, so a public `allowed_origins` host that resolves to an internal address
// (or a literal metadata IP) could be reached. These helpers close that: a hard,
// model-unreachable floor that refuses internal targets by default. Internal
// pentests stay possible, but only when the operator signs for them — that is
// `agent_internal_scope_authorized`, never the agent's decision and never a
// default.

/// True for an address the agent's HTTP egress must never reach by default:
/// loopback, RFC1918 private, carrier-grade NAT, link-local (incl. the cloud
/// metadata address 169.254.169.254), IPv6 ULA/link-local, unspecified,
/// broadcast, documentation and reserved ranges, and IPv4-mapped IPv6 forms of
/// all of the above.
#[allow(deprecated)] // Ipv6Addr::to_ipv4 intentionally catches the ::a.b.c.d compat form.
fn ip_is_internal(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => ipv4_is_internal(v4),
        std::net::IpAddr::V6(v6) => {
            // Native IPv6 checks FIRST: ::1 and :: otherwise read as the
            // IPv4-compat form ::0.0.0.1 / ::0.0.0.0 and slip past the conversions.
            let first = v6.segments()[0];
            if v6.is_loopback()
                || v6.is_unspecified()
                || (first & 0xfe00) == 0xfc00 // fc00::/7 unique-local
                || (first & 0xffc0) == 0xfe80 // fe80::/10 link-local
            {
                return true;
            }
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return ipv4_is_internal(mapped);
            }
            // Deprecated ::a.b.c.d compatibility form still names an IPv4 host
            // (e.g. ::127.0.0.1); ::1/:: are already handled above.
            if let Some(compat) = v6.to_ipv4() {
                return ipv4_is_internal(compat);
            }
            false
        }
    }
}

fn ipv4_is_internal(v4: std::net::Ipv4Addr) -> bool {
    if v4.is_loopback() // 127.0.0.0/8
        || v4.is_private() // 10/8, 172.16/12, 192.168/16
        || v4.is_link_local() // 169.254.0.0/16 (incl. 169.254.169.254 metadata)
        || v4.is_unspecified() // 0.0.0.0
        || v4.is_broadcast() // 255.255.255.255
        || v4.is_documentation()
    {
        return true;
    }
    let o = v4.octets();
    (o[0] == 100 && (o[1] & 0xc0) == 64) // 100.64.0.0/10 carrier-grade NAT
        || (o[0] == 192 && o[1] == 0 && o[2] == 0) // 192.0.0.0/24 IETF protocol
        || o[0] >= 240 // 240.0.0.0/4 reserved
}

/// A host string that is itself an internal IP literal (`host_str()` gives
/// `[::1]` for a bracketed IPv6 literal, so the brackets are stripped first).
fn host_is_internal_literal(host: &str) -> bool {
    let trimmed = host.trim_start_matches('[').trim_end_matches(']');
    trimmed
        .parse::<std::net::IpAddr>()
        .map(ip_is_internal)
        .unwrap_or(false)
}

/// Whether the operator explicitly authorized internal targets for this run.
/// Internal pentests are legitimate, but authorizing them is the operator's
/// signed decision — never the agent's, never a default. Read from the immutable
/// per-scan capability record (the `internal_scope` high-risk class, set only via
/// `set_scan_capability_authorization` with a legal attestation; the agent has no
/// tool that writes that table, so a prompt-injected agent cannot self-authorize).
/// The SSRF floor, the DNS-pin and the address checks are unchanged — this only
/// decides whether an operator-authorized internal literal may pass. Fails closed:
/// a missing authorization, a missing attestation or any storage error keeps every
/// internal target denied.
fn agent_internal_scope_authorized(context: &AgentRunContext) -> bool {
    let Ok(connection) = db::open(&context.db_path) else {
        return false;
    };
    scan_capability_class_authorized(
        &connection,
        &context.scan_id,
        context.attempt_number,
        "internal_scope",
    )
    .unwrap_or(false)
}

/// Resolve `url`'s host once and return the host plus one vetted address to pin
/// the connection to, so the socket reaches exactly the IP we checked — a public
/// name that resolves to an internal address, or one that rebinds between check
/// and connect, is refused. `Ok(None)` when the host is already an IP literal
/// (vetted in `agent_scope_assess`) so there is nothing to pin. `Err(code)` when
/// resolution fails or yields an internal address without operator authorization.
fn agent_pin_public_address(
    context: &AgentRunContext,
    url: &str,
) -> Result<Option<(String, std::net::SocketAddr)>, &'static str> {
    resolve_pinned_address(url, agent_internal_scope_authorized(context))
}

/// The context-free core of the pin: parse, resolve, classify. Split out so the
/// resolve-then-block behaviour is unit-testable without an `AgentRunContext`.
fn resolve_pinned_address(
    url: &str,
    allow_internal: bool,
) -> Result<Option<(String, std::net::SocketAddr)>, &'static str> {
    use std::net::ToSocketAddrs;
    let parsed = reqwest::Url::parse(url).map_err(|_| "invalid_url")?;
    let Some(host) = parsed.host_str() else {
        return Err("missing_host");
    };
    let stripped = host.trim_start_matches('[').trim_end_matches(']');
    if stripped.parse::<std::net::IpAddr>().is_ok() {
        // A literal IP was already vetted at the scope gate; nothing to pin.
        return Ok(None);
    }
    let port = parsed
        .port_or_known_default()
        .unwrap_or(if parsed.scheme() == "https" { 443 } else { 80 });
    let addrs: Vec<std::net::SocketAddr> = (host, port)
        .to_socket_addrs()
        .map_err(|_| "dns_resolution_failed")?
        .collect();
    let Some(first) = addrs.first().copied() else {
        return Err("dns_resolution_failed");
    };
    // Split-horizon or rebinding host: if ANY resolved address is internal, the
    // whole host is refused, not just the internal ones.
    if !allow_internal && addrs.iter().any(|addr| ip_is_internal(addr.ip())) {
        return Err("ssrf_resolved_internal");
    }
    Ok(Some((host.to_string(), first)))
}

fn ssrf_candidate_host(host: &str) -> bool {
    let Ok(ip) = host.parse::<std::net::Ipv4Addr>() else {
        return false;
    };
    let octets = ip.octets();
    octets[0] == 10
        || (octets[0] == 192 && octets[1] == 168)
        || (octets[0] == 169 && octets[1] == 254)
        || (octets[0] == 172 && (16..=31).contains(&octets[1]))
}

fn ssrf_callback_url(connection: &rusqlite::Connection) -> String {
    connection
        .query_row(
            "SELECT value FROM app_settings WHERE key='agent_ssrf_callback_url'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap_or_default()
}

fn valid_callback(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url.trim()) else {
        return false;
    };
    matches!(parsed.scheme(), "http" | "https")
        && parsed.host_str().is_some_and(|host| !host_is_internal_literal(host))
        && url.len() <= 300
}

fn observed_internal_hosts(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && (bytes[index].is_ascii_digit() || bytes[index] == b'.') {
            index += 1;
        }
        let token = &text[start..index];
        if ssrf_candidate_host(token) && !found.iter().any(|item: &String| item == token) {
            found.push(token.to_string());
        }
        if found.len() >= 8 {
            break;
        }
    }
    found
}

fn agent_tool_plan_ssrf_channel(
    context: &AgentRunContext,
    runtime: &mut AgentToolRuntime,
    arguments: &JsonValue,
) -> JsonValue {
    let Ok(connection) = db::open(&context.db_path) else {
        return agent_tool_error("无法读取 SSRF 通道配置", "ssrf_plan_unavailable");
    };
    let callback = ssrf_callback_url(&connection);
    let callback_ok = valid_callback(&callback);
    let internal = agent_internal_scope_authorized(context);
    let response = value_first(arguments, &["response", "body", "error", "reason"]);
    let mut seen = observed_internal_hosts(&response);
    for host in observed_internal_hosts(&context.evidence.to_string()) {
        if !seen.contains(&host) && seen.len() < 8 {
            seen.push(host);
        }
    }
    if !callback_ok && (!internal || seen.is_empty()) {
        return serde_json::json!({
            "accepted": true,
            "mode": "stop",
            "observedHosts": seen,
            "reason": "没有回连配置，响应里也没有可跟进的内网地址。不会按网段盲举。",
        });
    }
    let prompt = format!(
        "根据这次响应决定 SSRF 通道，不要使用预先指定的地址。响应片段：{}。响应或证据中出现的内网地址：{}。已配置回连：{}。内网跟进已授权：{internal}。只输出 JSON {{\"mode\":\"callback\"|\"internal\"|\"stop\",\"host\":\"\",\"port\":80,\"reason\":\"\"}}。internal 的 host 必须是上面出现过的某一个地址。没有回连不能选 callback。没有内网授权或没有出现内网地址就不能选 internal。不要输出地址清单，不要建议扫描网段。",
        response.chars().take(800).collect::<String>(),
        if seen.is_empty() { "无".to_string() } else { seen.join(",") },
        if callback_ok { callback.as_str() } else { "无" }
    );
    let choice = readonly_model_text_choice(&connection, &prompt);
    let mut mode = choice.0;
    let mut host = choice.1;
    let port = choice.2;
    let mut why = choice.3;
    if mode == "callback" && !callback_ok {
        mode = "stop".into();
        why = "没有配置回连".into();
    }
    if mode == "internal" && (!internal || !seen.iter().any(|item| item == &host) || !(1..=65535).contains(&port)) {
        mode = "stop".into();
        host.clear();
        why = "内网地址必须来自这次响应或已有证据，且任务已授权内网跟进".into();
    }
    record_pending_observation(
        &connection,
        &context.scan_id,
        "SSRF 通道",
        &format!("mode={mode} host={host} port={port} fromResponse={}", seen.join(",")),
    );
    let mut sent = JsonValue::Null;
    let target = value_first(arguments, &["url"]);
    let param = value_first(arguments, &["param"]);
    let identity = value_first(arguments, &["identity"]);
    if !target.is_empty() && !param.is_empty() && !identity.is_empty() && mode != "stop" {
        let payload = if mode == "callback" {
            callback.clone()
        } else {
            format!("http://{host}:{port}/")
        };
        if let Ok(injected) = sqli_inject_url(&target, &param, "{{INJ}}", &payload) {
            if same_host(&context.target_url, &injected) {
                if let Ok(response) = agent_http_request(
                    context, runtime, &identity, "GET", &injected, Vec::new(), None, None, "",
                    "ssrf", ScopeSource::HttpReplay, "plan_ssrf_channel",
                ) {
                    let _ = persist_agent_evidence(context, &response, "plan_ssrf_channel");
                    sent = serde_json::json!({"url": injected, "status": response.get("status")});
                }
            }
        }
    }
    serde_json::json!({
        "accepted": true,
        "mode": mode,
        "callbackUrl": if mode == "callback" { callback } else { String::new() },
        "host": if mode == "internal" { host } else { String::new() },
        "port": if mode == "internal" { port } else { 0 },
        "observedHosts": seen,
        "sent": sent,
        "reason": why,
    })
}

fn readonly_model_text_choice(connection: &rusqlite::Connection, prompt: &str) -> (String, String, u16, String) {
    let Ok(profile) = crate::model_configuration::gateway_profile_on(connection, None) else {
        return ("stop".into(), String::new(), 0, String::new());
    };
    let body = serde_json::json!({
        "model": profile.model,
        "messages": [
            {"role":"system","content":"你只根据给出的响应选择一个 SSRF 通道。内网地址必须是响应里出现过的。只输出 JSON。"},
            {"role":"user","content":prompt}
        ],
        "temperature": 0,
        "max_tokens": 180,
        "stream": false
    });
    let Ok(bytes) = serde_json::to_vec(&body) else { return ("stop".into(), String::new(), 0, String::new()) };
    let mut request = TransportRequest::post(&profile.endpoint, bytes).header("content-type", "application/json");
    if let Some(bearer) = profile.bearer() {
        request = request.header("authorization", format!("Bearer {bearer}"));
    }
    request.total_timeout = Some(Duration::from_secs(30));
    let Ok(response) = transport::send(request, std::sync::Arc::new(|| false), Box::new(|_| true)) else {
        return ("stop".into(), String::new(), 0, String::new());
    };
    let Ok(parsed) = serde_json::from_slice::<JsonValue>(&response.body) else {
        return ("stop".into(), String::new(), 0, String::new());
    };
    let Some(text) = parsed.pointer("/choices/0/message/content").and_then(JsonValue::as_str) else {
        return ("stop".into(), String::new(), 0, String::new());
    };
    let Some(start) = text.find('{') else { return ("stop".into(), String::new(), 0, String::new()) };
    let Some(end) = text.rfind('}') else { return ("stop".into(), String::new(), 0, String::new()) };
    let Ok(value) = serde_json::from_str::<JsonValue>(&text[start..=end]) else {
        return ("stop".into(), String::new(), 0, String::new());
    };
    (
        value.get("mode").and_then(JsonValue::as_str).unwrap_or("stop").to_string(),
        value.get("host").and_then(JsonValue::as_str).unwrap_or("").trim().to_string(),
        value.get("port").and_then(JsonValue::as_u64).unwrap_or(80) as u16,
        value.get("reason").and_then(JsonValue::as_str).unwrap_or("").chars().take(240).collect(),
    )
}

#[tauri::command]
pub fn get_agent_ssrf_callback_url(state: State<AppState>) -> Result<String, String> {
    let connection = db::open(&state.db_path)?;
    Ok(ssrf_callback_url(&connection))
}

#[tauri::command]
pub fn set_agent_ssrf_callback_url(state: State<AppState>, url: String) -> Result<(), String> {
    let trimmed = url.trim();
    if !trimmed.is_empty() && !valid_callback(trimmed) {
        return Err("回连地址必须是公网 http(s) URL，不能是 10/172/192/169 或本机".into());
    }
    let connection = db::open(&state.db_path)?;
    connection
        .execute(
            "INSERT INTO app_settings(key,value) VALUES('agent_ssrf_callback_url',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [trimmed],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(test)]
mod ssrf_guard_tests {
    use super::{host_is_internal_literal, ip_is_internal, resolve_pinned_address};
    use std::net::IpAddr;

    #[test]
    fn ssrf_candidate_is_one_private_literal() {
        assert!(super::ssrf_candidate_host("10.1.2.3"));
        assert!(super::ssrf_candidate_host("172.16.0.1"));
        assert!(super::ssrf_candidate_host("192.168.1.1"));
        assert!(super::ssrf_candidate_host("169.254.169.254"));
        assert!(!super::ssrf_candidate_host("127.0.0.1"));
        assert!(!super::ssrf_candidate_host("8.8.8.8"));
        let seen = super::observed_internal_hosts("upstream 10.0.0.8:8080 refused, ignore 8.8.8.8");
        assert_eq!(seen, vec!["10.0.0.8".to_string()]);
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn internal_addresses_are_refused() {
        for s in [
            "127.0.0.1", "10.0.0.5", "172.16.3.4", "172.31.255.255", "192.168.1.1",
            "169.254.169.254", // cloud metadata
            "0.0.0.0", "100.64.0.1", "192.0.0.1", "240.0.0.1", "255.255.255.255",
            "::1", "fe80::1", "fc00::1", "fd12:3456::1",
            "::ffff:127.0.0.1", // IPv4-mapped loopback
            "::ffff:10.0.0.1",  // IPv4-mapped private
        ] {
            assert!(ip_is_internal(ip(s)), "{s} must be internal");
        }
    }

    #[test]
    fn public_addresses_are_allowed() {
        for s in ["8.8.8.8", "1.1.1.1", "93.184.216.34", "2606:4700:4700::1111"] {
            assert!(!ip_is_internal(ip(s)), "{s} must be public");
        }
    }

    #[test]
    fn host_literals_are_classified() {
        for s in ["127.0.0.1", "[::1]", "169.254.169.254", "10.1.2.3"] {
            assert!(host_is_internal_literal(s), "{s} literal must be internal");
        }
        for s in ["example.com", "8.8.8.8", "[2606:4700:4700::1111]", "target.test"] {
            assert!(!host_is_internal_literal(s), "{s} must not be internal literal");
        }
    }

    #[test]
    fn hostname_resolving_internal_is_never_pinned() {
        // `localhost` resolves to 127.0.0.1 / ::1 via the hosts file, no network.
        // Whether it resolves (→ ssrf_resolved_internal) or cannot (→ a dns error),
        // it must NEVER come back as a pinned internal address.
        assert!(
            !matches!(resolve_pinned_address("http://localhost/", false), Ok(Some(_))),
            "a host resolving to internal must not be pinned",
        );
        // Literal IPs defer to the scope gate, so the pin step returns None.
        assert_eq!(resolve_pinned_address("http://127.0.0.1/", false), Ok(None));
        assert_eq!(resolve_pinned_address("http://[::1]/", false), Ok(None));
        // A malformed URL is refused, not silently allowed.
        assert!(resolve_pinned_address("not a url", false).is_err());
    }
}

/// The registrable domain a URL points at, as the ledger records it.
fn agent_origin_of(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(normalize_ownership_domain))
        .unwrap_or_default()
}

/// Resolve a `location` value against the URL that produced it.
fn parsed_redirect(from: &str, location: &str) -> Option<String> {
    let base = reqwest::Url::parse(from).ok()?;
    base.join(location).ok().map(|resolved| resolved.to_string())
}

/// How a redirect target is treated: same origin, inside the frozen scope, an
/// identity-provider jump, or a stop (§5.4).
fn agent_redirect_verdict(context: &AgentRunContext, target: &str, from: &str) -> String {
    if identity_provider_target(target) {
        return "identity_provider_redirect".to_string();
    }
    let assessment =
        agent_scope_assess(context, target, "GET", "document", ScopeSource::RedirectTarget);
    match assessment.decision {
        ScopeDecision::Allow { .. } => {
            let same_origin = reqwest::Url::parse(target)
                .ok()
                .zip(reqwest::Url::parse(from).ok())
                .map(|(left, right)| left.host_str() == right.host_str())
                .unwrap_or(false);
            if same_origin {
                String::new()
            } else {
                "redirect_within_scope".to_string()
            }
        }
        ScopeDecision::Ignore { .. } | ScopeDecision::Reject { .. } => {
            "redirect_out_of_scope".to_string()
        }
    }
}

fn is_agent_static_resource(path: &str, resource_type: &str) -> bool {
    let lowered = path.to_ascii_lowercase();
    AGENT_STATIC_SUFFIXES.iter().any(|suffix| lowered.ends_with(suffix))
        || matches!(
            resource_type.to_ascii_lowercase().as_str(),
            "script" | "stylesheet" | "image" | "font" | "media"
        )
}

/// `Ok((url, class))` means the request may be sent and what it counts as. The
/// noise screen needs the real method: without it every candidate looks like an
/// UNKNOWN request and even the authorized target would be refused.
fn agent_scope_check(
    context: &AgentRunContext,
    url: &str,
    method: &str,
    source: ScopeSource,
) -> Result<(String, ScopeClass), String> {
    let assessment = agent_scope_assess(context, url, method, "", source);
    match assessment.decision {
        ScopeDecision::Allow { .. } => Ok((
            reqwest::Url::parse(url)
                .map(|parsed| parsed.as_str().to_string())
                .unwrap_or_else(|_| url.to_string()),
            assessment.class,
        )),
        ScopeDecision::Reject { code, reason } if code == "scope_denied" => {
            if let Some(host) = reqwest::Url::parse(url)
                .ok()
                .and_then(|parsed| parsed.host_str().map(str::to_string))
            {
                record_discovered_host(context, &host);
            }
            Err(format!("{reason}。已记录该主机，当前任务不自动打开"))
        }
        other => Err(other.reason().to_string()),
    }
}

fn record_discovered_host(context: &AgentRunContext, host: &str) {
    let note = serde_json::json!({
        "host": host,
        "note": "发现了这个主机。当前任务不自动打开；下一次任务里明确加入后才能调查。",
    });
    let _ = stage_agent_finding(
        context,
        "native-agent",
        "discovered_host",
        &format!("discovered-host:{host}"),
        host,
        "info",
        &note,
    );
}
