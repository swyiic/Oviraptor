// The native HTTP executor. Raw bytes, structure summary and timing are kept
// for audit; the model receives the same digest fields inline.

/// Security-relevant response headers the model must see as values (not names
/// only), and that can become deterministic observation findings.
fn agent_security_relevant_headers(
    headers: &reqwest::header::HeaderMap,
) -> Vec<(String, String)> {
    const NAMES: &[&str] = &[
        "server",
        "x-powered-by",
        "x-aspnet-version",
        "x-aspnetmvc-version",
        "x-generator",
        "x-drupal-cache",
        "x-framework",
        "x-runtime",
        "x-version",
    ];
    let mut out = Vec::new();
    for name in NAMES {
        if let Some(value) = headers.get(*name).and_then(|v| v.to_str().ok()) {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                continue;
            }
            // Bare product tokens without a version still leak stack family when
            // paired with other headers; keep them. Skip generic placeholders.
            if trimmed.eq_ignore_ascii_case("null") || trimmed == "-" {
                continue;
            }
            out.push((name.to_string(), trimmed.to_string()));
        }
    }
    out
}

fn agent_header_looks_versioned(name: &str, value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    if name.eq_ignore_ascii_case("x-aspnet-version")
        || name.eq_ignore_ascii_case("x-aspnetmvc-version")
        || name.eq_ignore_ascii_case("x-powered-by")
    {
        return true;
    }
    if name.eq_ignore_ascii_case("server") {
        return lower.contains('/')
            || lower.contains("iis")
            || lower.contains("apache")
            || lower.contains("nginx")
            || lower.contains("tomcat")
            || lower.contains("jetty");
    }
    lower.chars().any(|c| c.is_ascii_digit())
}


fn agent_detect_stack_trace_leak(body: &str) -> Option<String> {
    let lower = body.to_ascii_lowercase();
    let needles = [
        "server error in '/' application",
        "stack trace:",
        "at system.",
        "system.web.",
        "system.data.",
        "[httpexception",
        "yellow screen of death",
        "aspxerrorpath=",
    ];
    for needle in needles {
        if lower.contains(needle) {
            let excerpt: String = body.chars().take(240).collect();
            return Some(excerpt.replace('\n', " "));
        }
    }
    None
}

fn agent_outdated_jquery_from_url(url: &str) -> Option<(String, String)> {
    let lower = url.to_ascii_lowercase();
    let idx = lower.find("jquery-")?;
    let rest = &lower[idx + "jquery-".len()..];
    let ver: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect::<String>()
        .trim_matches('.')
        .to_string();
    if ver.is_empty() || !ver.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return None;
    }
    // jQuery < 3.5.0 has multiple publicly documented XSS issues; 1.x/2.x are EOL.
    let parts: Vec<u32> = ver
        .split('.')
        .filter_map(|p| p.parse().ok())
        .collect();
    let major = *parts.first().unwrap_or(&0);
    let minor = *parts.get(1).unwrap_or(&0);
    if major < 3 || (major == 3 && minor < 5) {
        Some((ver, url.to_string()))
    } else {
        None
    }
}

#[allow(dead_code)]
fn agent_cors_misconfig(headers: &reqwest::header::HeaderMap, sent_origin: &str) -> Option<String> {
    let acao = headers
        .get("access-control-allow-origin")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .trim()
        .to_string();
    if acao.is_empty() {
        return None;
    }
    let acac = headers
        .get("access-control-allow-credentials")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .eq_ignore_ascii_case("true");
    if acao == "*" {
        return Some(format!("Access-Control-Allow-Origin: * (probe Origin={sent_origin})"));
    }
    if !sent_origin.is_empty() && acao == sent_origin {
        return Some(format!(
            "Access-Control-Allow-Origin reflects {acao}; Allow-Credentials={acac}"
        ));
    }
    None
}



fn agent_detect_client_md5_password(body: &str) -> Option<String> {
    let lower = body.to_ascii_lowercase();
    let has_md5_lib = lower.contains("jquery.md5")
        || lower.contains("/jquery.md5.js")
        || lower.contains("$.md5(")
        || lower.contains("hex_md5(")
        || lower.contains("cryptojs.md5(");
    if !has_md5_lib {
        return None;
    }
    let passwordish = lower.contains("password")
        || lower.contains("passwd")
        || lower.contains("pass_word")
        || body.contains("PassWord");
    let assigns_hash = lower.contains("$.md5(")
        || lower.contains("hex_md5(")
        || lower.contains("cryptojs.md5(")
        || (lower.contains(".val(") && lower.contains("md5"));
    if passwordish && assigns_hash {
        let excerpt: String = body
            .lines()
            .map(str::trim)
            .find(|line| {
                let l = line.to_ascii_lowercase();
                l.contains("md5") && (l.contains("pass") || l.contains("password") || line.contains("PassWord"))
            })
            .unwrap_or("client MD5 of password field before submit")
            .chars()
            .take(200)
            .collect();
        return Some(excerpt);
    }
    // Library present next to a login password field is enough to flag.
    if passwordish && lower.contains("jquery.md5") {
        return Some("page loads jquery.md5.js alongside a password field".into());
    }
    None
}

