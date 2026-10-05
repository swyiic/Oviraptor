#[allow(clippy::too_many_arguments)]
fn agent_http_request(
    context: &AgentRunContext,
    runtime: &mut AgentToolRuntime,
    identity_key: &str,
    method: &str,
    url: &str,
    extra_headers: Vec<(String, String)>,
    body: Option<String>,
    content_type: Option<String>,
    contract_key: &str,
    family: &str,
    source: ScopeSource,
    tool: &str,
) -> Result<JsonValue, JsonValue> {
    let Some(identity) = agent_identity_of(context, identity_key) else {
        return Err(serde_json::json!({
            "error": "该身份不属于当前任务",
            "code": "identity_not_found",
        }));
    };
    let mut extra_headers = extra_headers;
    // One Origin probe per attempt so CORS reflection/wildcard can be observed.
    if !runtime.cors_probed && method.eq_ignore_ascii_case("GET") {
        runtime.cors_probed = true;
        if !extra_headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("origin"))
        {
            extra_headers.push((
                "Origin".to_string(),
                "https://evil.example".to_string(),
            ));
        }
    }
    agent_http_exchange(
        context,
        runtime,
        &AgentHttpRequest {
            identity,
            method: method.to_ascii_uppercase(),
            url: url.to_string(),
            extra_headers,
            body,
            content_type,
            contract_key: contract_key.to_string(),
            family: family.to_string(),
            source,
            tool: tool.to_string(),
            timeout_seconds: 15,
        },
    )
}
