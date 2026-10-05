// Share the explicitly emitted headers between the transport and its claim.
fn agent_http_claim_identity_valid(context: &AgentRunContext, request: &AgentHttpRequest) -> bool {
    context.identities.iter().any(|identity| {
        identity.key == request.identity.key
            && identity.session_id == request.identity.session_id
            && identity.anonymous == request.identity.anonymous
    })
}

fn agent_http_wire_headers(
    context: &AgentRunContext,
    request: &AgentHttpRequest,
) -> Result<Vec<(String, String)>, String> {
    let mut headers = agent_identity_headers(context, &request.identity)?;
    for (name, value) in &request.extra_headers {
        if AGENT_CREDENTIAL_HEADERS.contains(&name.as_str()) || name.eq_ignore_ascii_case("host") {
            continue;
        }
        headers.push((name.clone(), value.clone()));
    }
    if let Some(content_type) = request
        .content_type
        .as_deref()
        .filter(|v| !v.trim().is_empty())
    {
        headers.push(("content-type".to_string(), content_type.to_string()));
    }
    Ok(headers)
}

fn agent_http_valid_wire_headers(headers: &[(String, String)]) -> Vec<(String, Vec<u8>)> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            let name = reqwest::header::HeaderName::from_bytes(name.as_bytes()).ok()?;
            let value = reqwest::header::HeaderValue::from_str(value).ok()?;
            Some((name.as_str().to_string(), value.as_bytes().to_vec()))
        })
        .collect()
}

fn agent_http_upload_bytes(headers: &[(String, String)], body: Option<&str>) -> usize {
    let upload = agent_http_valid_wire_headers(headers)
        .iter()
        .any(|(name, value)| {
            name == "content-type"
                && String::from_utf8_lossy(value)
                    .trim()
                    .to_ascii_lowercase()
                    .starts_with("multipart/")
        });
    if upload {
        body.map_or(0, str::len)
    } else {
        0
    }
}
