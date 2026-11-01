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

/// Whether the operator explicitly authorized internal targets for this run in
/// the signed scope contract. Internal pentests are legitimate, but authorizing
/// them is the operator's signed decision — never the agent's, never a default.
/// Hard-wired to `false` until that signed-contract field exists (design §5);
/// until then every internal target is denied.
fn agent_internal_scope_authorized(_context: &AgentRunContext) -> bool {
    false
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

#[cfg(test)]
mod ssrf_guard_tests {
    use super::{host_is_internal_literal, ip_is_internal, resolve_pinned_address};
    use std::net::IpAddr;

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
