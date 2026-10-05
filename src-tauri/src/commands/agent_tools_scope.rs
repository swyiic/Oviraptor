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
