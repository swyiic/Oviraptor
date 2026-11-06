// WAF efficacy follow-up for deep-dive. The cloud model already configured for
// scans proposes the next same-host request. This file does not apply a fixed
// encoding/header catalog.

use crate::agent_runtime::model::{transport, TransportError, TransportRequest};
use std::sync::Arc;

// Bounded WAF-efficacy attempts (design C5). A hard cap on same-host bypass
// tries; the loop never exceeds it and never auto-continues past it — more
// attempts are a separate operator-authorized action, not something this loop
// decides. Rate-limited between tries so the probe can never become rapid-fire /
// DoS-like. Default ≤8 per design §4b.
const WAF_BYPASS_ATTEMPT_CAP: usize = 8;
const WAF_BYPASS_RATE_LIMIT: Duration = Duration::from_millis(1500);

struct BypassHit {
    curl: String,
    attempt: usize,
    requests: i64,
    trace: Vec<JsonValue>,
}

struct WafBypassOutcome {
    hit: Option<BypassHit>,
    note: String,
}

struct ProposedRequest {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    stop: bool,
    reason: String,
}

fn same_host(original: &str, proposed: &str) -> bool {
    let Ok(left) = reqwest::Url::parse(original) else { return false };
    let Ok(right) = reqwest::Url::parse(proposed) else { return false };
    matches!(right.scheme(), "http" | "https")
        && left.host_str() == right.host_str()
        && left.port_or_known_default() == right.port_or_known_default()
}

fn parse_proposal(text: &str) -> Option<ProposedRequest> {
    let trimmed = text.trim().trim_matches('`');
    let value: JsonValue = serde_json::from_str(trimmed).ok().or_else(|| {
        let start = trimmed.find('{')?;
        let end = trimmed.rfind('}')?;
        serde_json::from_str::<JsonValue>(&trimmed[start..=end]).ok()
    })?;
    let stop = value.get("stop").and_then(JsonValue::as_bool).unwrap_or(false);
    let method = value.get("method").and_then(JsonValue::as_str).unwrap_or("GET").trim().to_ascii_uppercase();
    if !matches!(method.as_str(), "GET" | "HEAD" | "POST") {
        return None;
    }
    let url = value.get("url").and_then(JsonValue::as_str)?.trim().to_string();
    if url.len() > 2048 || url.chars().any(|ch| ch.is_control()) {
        return None;
    }
    let mut headers = Vec::new();
    if let Some(rows) = value.get("headers").and_then(JsonValue::as_array) {
        for row in rows.iter().take(6) {
            let name = row.get("name").and_then(JsonValue::as_str).unwrap_or("").trim();
            let header_value = row.get("value").and_then(JsonValue::as_str).unwrap_or("").trim();
            let lower = name.to_ascii_lowercase();
            let value_limit = if matches!(lower.as_str(), "cookie" | "authorization") { 512 } else { 180 };
            if name.is_empty() || header_value.is_empty() || name.len() > 64 || header_value.len() > value_limit {
                continue;
            }
            if header_value.chars().any(|ch| ch.is_control()) || lower == "proxy-authorization" {
                continue;
            }
            headers.push((name.to_string(), header_value.to_string()));
        }
    }
    Some(ProposedRequest {
        method,
        url,
        headers,
        stop,
        reason: value.get("reason").and_then(JsonValue::as_str).unwrap_or("").chars().take(240).collect(),
    })
}

fn model_text(response: &JsonValue) -> Option<String> {
    match response.pointer("/choices/0/message/content") {
        Some(JsonValue::String(value)) => Some(value.clone()),
        Some(JsonValue::Array(parts)) => Some(parts.iter().filter_map(|part| part.get("text").and_then(JsonValue::as_str)).collect()),
        _ => None,
    }
}

fn ask_model(profile: &GatewayProfile, prompt: &str) -> Result<String, String> {
    let body = serde_json::json!({
        "model": profile.model,
        "messages": [
            {"role":"system","content":"你在已授权的 WAF 效力测试中分析拦截响应。只输出一个 JSON 对象，不要 Markdown。字段：stop(boolean)、method(GET|HEAD|POST)、url(必须与原请求同主机)、headers([{name,value}])、reason。Cookie 和 Authorization 只能复用上一响应里已经出现的值。不要破解验证码，不要换主机，不要给出第二发请求。"},
            {"role":"user","content":prompt}
        ],
        "temperature": 0.2,
        "max_tokens": 500,
        "stream": false
    });
    let request_body = serde_json::to_vec(&body).map_err(|error| error.to_string())?;
    let mut request = TransportRequest::post(&profile.endpoint, request_body).header("content-type", "application/json");
    if let Some(bearer) = profile.bearer() {
        request = request.header("authorization", format!("Bearer {bearer}"));
    }
    request.total_timeout = Some(Duration::from_secs(45));
    request.proxy = profile.proxy.clone();
    request.reject_redirects = crate::agent_host_sidecar::is_model_bridge_endpoint(&profile.endpoint);
    let response = transport::send(request, Arc::new(|| false), Box::new(|_| true)).map_err(|error| match error {
        TransportError::Setup(message)
        | TransportError::Network(message)
        | TransportError::Timeout(message)
        | TransportError::Interrupted(message) => message,
        TransportError::Cancelled(_) => "waf_model_cancelled".into(),
    })?;
    if !(200..300).contains(&response.status) {
        return Err(format!("waf_model_http_{}", response.status));
    }
    let parsed: JsonValue = serde_json::from_slice(&response.body).map_err(|_| "waf_model_response_invalid".to_string())?;
    model_text(&parsed).filter(|text| !text.trim().is_empty()).ok_or_else(|| "waf_model_empty".into())
}

fn bounded_waf_bypass(
    connection: &rusqlite::Connection,
    client: &reqwest::blocking::Client,
    url: &str,
    status: u16,
    body: &str,
) -> WafBypassOutcome {
    let profile = match crate::model_configuration::gateway_profile_on(connection, None) {
        Ok(profile) => profile,
        Err(error) => return WafBypassOutcome { hit: None, note: format!("云端模型不可用，未做写死绕过：{error}") },
    };
    let snippet: String = body.chars().take(800).collect();
    let mut transcript = format!("原始请求被拦截。url={url} status={status} body={snippet}");
    let mut trace = Vec::new();
    for round in 1..=WAF_BYPASS_ATTEMPT_CAP {
        // Rate-limit: never fire bypass tries back-to-back (anti-DoS, design C5).
        if round > 1 {
            std::thread::sleep(WAF_BYPASS_RATE_LIMIT);
        }
        let prompt = format!("{transcript}\n请给出下一发同主机请求。若应停止，stop=true。");
        let text = match ask_model(&profile, &prompt) {
            Ok(text) => text,
            Err(error) => return WafBypassOutcome { hit: None, note: format!("模型分析失败：{error}"), },
        };
        let Some(proposal) = parse_proposal(&text) else {
            trace.push(trace_step(round, "waf_model_invalid", serde_json::json!({"round": round})));
            transcript.push_str("\n上一轮模型输出不是可用 JSON。");
            continue;
        };
        trace.push(trace_step(round, "waf_model_proposal", serde_json::json!({"round": round, "reason": proposal.reason, "method": proposal.method})));
        if proposal.stop {
            return WafBypassOutcome { hit: None, note: format!("模型建议停止：{}", proposal.reason) };
        }
        if !same_host(url, &proposal.url) {
            transcript.push_str("\n上一轮 URL 被拒绝：必须保持同主机。");
            continue;
        }
        let mut request = match proposal.method.as_str() {
            "HEAD" => client.head(&proposal.url),
            "POST" => client.post(&proposal.url),
            _ => client.get(&proposal.url),
        };
        for (name, value) in &proposal.headers {
            request = request.header(name.as_str(), value.as_str());
        }
        let Ok(response) = request.send() else {
            transcript.push_str("\n上一轮请求发送失败。");
            continue;
        };
        let code = response.status().as_u16();
        let next_body = response.text().unwrap_or_default();
        let preview: String = next_body.chars().take(400).collect();
        trace.push(trace_step(round, "waf_model_result", serde_json::json!({"status": code})));
        if !looks_like_waf(code, &next_body) && (200..400).contains(&code) {
            return WafBypassOutcome {
                hit: Some(BypassHit {
                    curl: build_curl(&proposal.method, &proposal.url, &proposal.headers, None),
                    attempt: round,
                    requests: round as i64,
                    trace,
                }),
                note: proposal.reason,
            };
        }
        transcript = format!("上一轮仍被拦截。method={} url={} status={code} body={preview} reason={}", proposal.method, proposal.url, proposal.reason);
    }
    WafBypassOutcome {
        hit: None,
        note: format!(
            "已达有界 WAF 尝试上限（{WAF_BYPASS_ATTEMPT_CAP} 次）仍被拦截，或模型主动停止。不自动继续、不使用写死规则、不破解验证码；如需更多尝试须 operator 另行授权。"
        ),
    }
}

#[cfg(test)]
mod waf_model_tests {
    use super::{parse_proposal, same_host, WAF_BYPASS_ATTEMPT_CAP, WAF_BYPASS_RATE_LIMIT};

    #[test]
    fn attempt_cap_is_bounded_and_rate_limited() {
        // Design C5: a small bounded cap (≤8) and a non-zero rate limit so the
        // efficacy probe can never become unbounded or rapid-fire / DoS-like.
        assert!(WAF_BYPASS_ATTEMPT_CAP >= 1 && WAF_BYPASS_ATTEMPT_CAP <= 8);
        assert!(!WAF_BYPASS_RATE_LIMIT.is_zero());
    }

    #[test]
    fn rejects_other_hosts() {
        assert!(same_host("https://app.example/login", "https://app.example/login?x=1"));
        assert!(!same_host("https://app.example/login", "https://evil.example/login"));
    }

    #[test]
    fn keeps_cookie_and_authorization() {
        let proposal = parse_proposal(r#"{"stop":false,"method":"GET","url":"https://app.example/a","headers":[{"name":"Cookie","value":"a=b"},{"name":"X-Test","value":"1"}]}"#).unwrap();
        assert_eq!(proposal.headers.len(), 2);
        assert!(proposal.headers.iter().any(|(name, _)| name == "Cookie"));
    }
}
