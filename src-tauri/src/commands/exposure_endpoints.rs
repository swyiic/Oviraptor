// Deterministic common-exposure checks for a Web target (the "Category A" misses:
// exposed management/doc endpoints, middleware/version leaks, CORS wildcard). This
// module is PURE classification over an already-fetched response — it performs no
// network I/O and makes no budget claim. The recon wiring (next increment) fetches
// each catalog path through the same budgeted/scope-checked transport the public
// surface uses, and the homepage response it already captures is classified here
// for free (versions + CORS), so no extra request is spent for those two.
//
// A target page is untrusted: these are fixed signature rules, never the model's
// judgement. Every rule keys off bytes actually returned (status + headers + a
// bounded body sample), so a 404/soft-404 or an error page served at the path does
// not mint a finding.

/// One exposed-endpoint probe. `body_markers` empty means "a 2xx alone is the
/// signal" (e.g. a heapdump download); otherwise the body must contain at least
/// one marker (case-insensitive), so a generic 200 catch-all page is not a hit.
#[derive(Clone, Copy)]
#[allow(dead_code)] // Consumed by the recon wiring (next increment); exercised by tests now.
struct ExposureProbe {
    path: &'static str,
    title: &'static str,
    severity: &'static str,
    cwe: &'static str,
    body_markers: &'static [&'static str],
}

#[allow(dead_code)]
const EXPOSURE_PROBES: &[ExposureProbe] = &[
    ExposureProbe { path: "/actuator", title: "Spring Boot Actuator 端点未授权暴露", severity: "medium", cwe: "CWE-200", body_markers: &["\"_links\"", "\"health\"", "/actuator/"] },
    ExposureProbe { path: "/actuator/env", title: "Actuator env 暴露环境变量与配置", severity: "high", cwe: "CWE-200", body_markers: &["\"propertySources\"", "systemEnvironment"] },
    ExposureProbe { path: "/actuator/heapdump", title: "Actuator heapdump 可下载（内存转储泄漏）", severity: "high", cwe: "CWE-200", body_markers: &[] },
    ExposureProbe { path: "/v2/api-docs", title: "Swagger/OpenAPI 文档未授权暴露", severity: "medium", cwe: "CWE-200", body_markers: &["\"swagger\"", "\"openapi\"", "\"paths\""] },
    ExposureProbe { path: "/v3/api-docs", title: "Swagger/OpenAPI 文档未授权暴露", severity: "medium", cwe: "CWE-200", body_markers: &["\"openapi\"", "\"paths\""] },
    ExposureProbe { path: "/swagger-ui/index.html", title: "Swagger UI 未授权暴露", severity: "medium", cwe: "CWE-200", body_markers: &["swagger-ui"] },
    ExposureProbe { path: "/swagger-ui.html", title: "Swagger UI 未授权暴露", severity: "medium", cwe: "CWE-200", body_markers: &["swagger-ui"] },
    ExposureProbe { path: "/swagger-resources", title: "Swagger resources 未授权暴露", severity: "low", cwe: "CWE-200", body_markers: &["swagger", "\"location\""] },
    ExposureProbe { path: "/druid/index.html", title: "Druid 监控台未授权暴露", severity: "medium", cwe: "CWE-284", body_markers: &["Druid Stat", "druid-", "druid monitor"] },
    ExposureProbe { path: "/examples/", title: "Tomcat 示例目录暴露", severity: "low", cwe: "CWE-200", body_markers: &["Apache Tomcat Examples", "Servlets examples", "JSP Examples"] },
    ExposureProbe { path: "/.env", title: "环境变量文件 .env 未授权暴露", severity: "high", cwe: "CWE-200", body_markers: &["APP_KEY=", "DB_PASSWORD=", "SECRET", "APP_ENV="] },
    ExposureProbe { path: "/.git/config", title: "Git 仓库配置暴露", severity: "high", cwe: "CWE-527", body_markers: &["[core]", "repositoryformatversion"] },
];

/// Whether this fetched response proves the endpoint is exposed. Returns the
/// finding (title, severity, cwe) or None. Only 2xx counts; a redirect, 401/403,
/// or 404 is a boundary, not a finding.
#[allow(dead_code)]
fn classify_exposure_endpoint(path: &str, status: u16, body: &str) -> Option<(&'static str, &'static str, &'static str)> {
    let probe = EXPOSURE_PROBES.iter().find(|p| p.path == path)?;
    if !(200..=299).contains(&status) {
        return None;
    }
    let hit = if probe.body_markers.is_empty() {
        !body.is_empty()
    } else {
        let lower = body.to_ascii_lowercase();
        probe.body_markers.iter().any(|m| lower.contains(&m.to_ascii_lowercase()))
    };
    hit.then_some((probe.title, probe.severity, probe.cwe))
}

/// CORS wildcard: `Access-Control-Allow-Origin: *` trusts any origin. `headers`
/// are (lowercased-name, value) pairs as captured by the public-surface probe.
#[allow(dead_code)]
fn detect_cors_wildcard(headers: &[(String, String)]) -> bool {
    headers.iter().any(|(name, value)| name == "access-control-allow-origin" && value.trim() == "*")
}

/// Extract (component, version) leaks from response headers and a bounded body
/// sample — the "连版本号都没有" gap. Header form `Name/1.2.3` and a few common
/// body banners. De-duplicated, bounded.
#[allow(dead_code)]
fn extract_server_versions(headers: &[(String, String)], body: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut push = |component: String, version: String| {
        if !version.is_empty() && !out.iter().any(|(c, v)| *c == component && *v == version) && out.len() < 16 {
            out.push((component, version));
        }
    };
    for (name, value) in headers {
        if matches!(name.as_str(), "server" | "x-powered-by" | "x-aspnet-version" | "x-aspnetmvc-version") {
            // Split each "Name/Version" token; a bare version (x-aspnet-version)
            // is attributed to the header's own component.
            for token in value.split([' ', ',', ';']) {
                if let Some((component, version)) = token.split_once('/') {
                    if version.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                        push(component.trim().to_string(), version.trim().to_string());
                    }
                } else if token.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                    push(name.clone(), token.trim().to_string());
                }
            }
        }
    }
    // A few high-signal body banners (error pages / monitor consoles).
    for (needle, component) in [
        ("apache tomcat/", "Apache Tomcat"),
        ("nginx/", "nginx"),
        ("jetty(", "Jetty"),
        ("openresty/", "openresty"),
    ] {
        let lower = body.to_ascii_lowercase();
        if let Some(pos) = lower.find(needle) {
            let tail: String = body[pos + needle.len()..]
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if !tail.is_empty() {
                push(component.to_string(), tail);
            }
        }
    }
    out
}

/// Server error / stack-trace / framework-banner disclosure in a response body
/// (both operator reports flag "报错信息泄漏"). Signature over the body only, so a
/// normal page never trips it. Returns (title, severity, cwe).
#[allow(dead_code)]
fn detect_error_disclosure(body: &str) -> Option<(&'static str, &'static str, &'static str)> {
    let lower = body.to_ascii_lowercase();
    const MARKERS: &[&str] = &[
        "whitelabel error page", "org.springframework", "com.alibaba.druid",
        "traceback (most recent call last)", "you have an error in your sql syntax",
        "org.hibernate", "java.lang.", "javax.servlet", "system.web.httpexception",
        "<b>warning</b>:", "<b>fatal error</b>", "<b>notice</b>:", "pg::", "psql:",
        "ora-0", "microsoft ole db provider", "unclosed quotation mark",
    ];
    MARKERS
        .iter()
        .any(|m| lower.contains(m))
        .then_some(("服务器报错/堆栈信息泄漏（框架、路径、SQL 细节）", "medium", "CWE-209"))
}

/// A JWT in the response (header or body) is worth flagging so the agent checks
/// its algorithm (alg=none) and secret strength with run_analysis_script. Shape
/// heuristic: a `eyJ…` header segment, a dot, then a `eyJ…` payload segment.
#[allow(dead_code)]
fn detect_jwt_present(headers: &[(String, String)], body: &str) -> bool {
    let looks_like_jwt = |text: &str| {
        let bytes = text.as_bytes();
        let mut i = 0;
        while let Some(pos) = text[i..].find("eyJ") {
            let start = i + pos;
            // Walk the first base64url segment to a '.', then require another "eyJ".
            let mut j = start;
            while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || matches!(bytes[j], b'-' | b'_')) {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'.' && text[j + 1..].starts_with("eyJ") && j - start >= 8 {
                return true;
            }
            i = start + 3;
        }
        false
    };
    headers.iter().any(|(_, value)| looks_like_jwt(value)) || looks_like_jwt(body)
}

/// Emit deterministic exposure findings from the homepage response the public-
/// surface probe already fetched — version/middleware leaks and a CORS wildcard —
/// costing no extra request. Best-effort and idempotent (ON CONFLICT upsert); the
/// caller ignores the result so a findings-write hiccup never affects the capture.
fn emit_homepage_exposure_findings(
    connection: &rusqlite::Connection,
    scan_id: &str,
    target_url: &str,
    status: u16,
    headers: &[(String, String)],
    body: &str,
) {
    let insert = |kind: &str, record_key: &str, title: &str, severity: &str, record: JsonValue| {
        let _ = connection.execute(
            "INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title,severity,record_json) \
             VALUES(?1,?2,'exposure',?3,?4,?5,?6,?7) \
             ON CONFLICT(scan_id,target_url,stage,kind,record_key) DO UPDATE SET \
             title=excluded.title,severity=excluded.severity,record_json=excluded.record_json,updated_at=datetime('now','localtime')",
            params![scan_id, target_url, kind, record_key, title, severity, record.to_string()],
        );
    };
    for (component, version) in extract_server_versions(headers, body) {
        insert(
            "version_leak",
            &format!("version:{component}"),
            &format!("中间件/版本号泄漏：{component} {version}"),
            "low",
            serde_json::json!({"component": component, "version": version, "source": "homepage", "status": status}),
        );
    }
    if detect_cors_wildcard(headers) {
        insert(
            "cors",
            "cors:wildcard",
            "CORS 配置不当：Access-Control-Allow-Origin 为通配 *",
            "medium",
            serde_json::json!({"header": "access-control-allow-origin", "value": "*", "source": "homepage", "status": status}),
        );
    }
    if let Some((title, severity, cwe)) = detect_error_disclosure(body) {
        insert(
            "error_disclosure",
            "error:banner",
            title,
            severity,
            serde_json::json!({"cwe": cwe, "source": "homepage", "status": status}),
        );
    }
    if detect_jwt_present(headers, body) {
        insert(
            "jwt_present",
            "jwt:homepage",
            "检测到 JWT：建议用 run_analysis_script 核验签名算法(alg=none/弱算法)与密钥强度",
            "low",
            serde_json::json!({"source": "homepage", "status": status, "cwe": "CWE-345"}),
        );
    }
}

#[cfg(test)]
mod exposure_endpoints_tests {
    use super::{classify_exposure_endpoint, detect_cors_wildcard, detect_error_disclosure, detect_jwt_present, emit_homepage_exposure_findings, extract_server_versions};
    use rusqlite::Connection;

    #[test]
    fn actuator_200_with_links_is_a_finding() {
        let body = r#"{"_links":{"self":{"href":"/actuator"},"health":{"href":"/actuator/health"}}}"#;
        let found = classify_exposure_endpoint("/actuator", 200, body).unwrap();
        assert_eq!(found.1, "medium");
    }

    #[test]
    fn actuator_404_or_login_redirect_is_not_a_finding() {
        assert!(classify_exposure_endpoint("/actuator", 404, "Whitelabel Error Page").is_none());
        assert!(classify_exposure_endpoint("/actuator", 302, "").is_none());
        // 200 but a generic SPA shell without any actuator marker: not a hit.
        assert!(classify_exposure_endpoint("/actuator", 200, "<!doctype html><title>App</title>").is_none());
    }

    #[test]
    fn swagger_and_env_and_examples_match_their_markers() {
        assert!(classify_exposure_endpoint("/v2/api-docs", 200, r#"{"swagger":"2.0","paths":{}}"#).is_some());
        assert!(classify_exposure_endpoint("/.env", 200, "APP_KEY=base64:xxx\nDB_PASSWORD=secret").is_some());
        assert!(classify_exposure_endpoint("/examples/", 200, "<h1>Apache Tomcat Examples</h1>").is_some());
        // heapdump: empty markers → any non-empty 2xx body is the signal.
        assert!(classify_exposure_endpoint("/actuator/heapdump", 200, "\u{0}\u{1}JAVA PROFILE").is_some());
    }

    #[test]
    fn cors_wildcard_detected_only_on_star() {
        assert!(detect_cors_wildcard(&[("access-control-allow-origin".into(), "*".into())]));
        assert!(!detect_cors_wildcard(&[("access-control-allow-origin".into(), "https://app.example.com".into())]));
        assert!(!detect_cors_wildcard(&[("content-type".into(), "text/html".into())]));
    }

    #[test]
    fn versions_extracted_from_headers_and_body() {
        let headers = vec![
            ("server".to_string(), "Apache-Tomcat/9.0.1 (Unix)".to_string()),
            ("x-powered-by".to_string(), "Servlet/3.1 JSP/2.3".to_string()),
            ("x-aspnet-version".to_string(), "4.0.30319".to_string()),
        ];
        let got = extract_server_versions(&headers, "powered by nginx/1.18.0 here");
        assert!(got.iter().any(|(c, v)| c == "Apache-Tomcat" && v == "9.0.1"));
        assert!(got.iter().any(|(c, v)| c == "Servlet" && v == "3.1"));
        assert!(got.iter().any(|(c, v)| c == "x-aspnet-version" && v == "4.0.30319"));
        assert!(got.iter().any(|(c, v)| c == "nginx" && v == "1.18.0"));
    }

    #[test]
    fn no_versions_when_headers_are_plain() {
        assert!(extract_server_versions(&[("server".into(), "cloudflare".into())], "hello").is_empty());
    }

    #[test]
    fn emit_writes_version_and_cors_findings_idempotently() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE sentinel_findings(scan_id TEXT,target_url TEXT,stage TEXT,kind TEXT,record_key TEXT,title TEXT,severity TEXT,record_json TEXT,updated_at TEXT DEFAULT '', PRIMARY KEY(scan_id,target_url,stage,kind,record_key));",
        )
        .unwrap();
        let headers = vec![
            ("server".to_string(), "Apache-Tomcat/9.0.1".to_string()),
            ("access-control-allow-origin".to_string(), "*".to_string()),
        ];
        emit_homepage_exposure_findings(&c, "scan-a", "http://t/", 200, &headers, "");
        // A second call upserts, never duplicates (idempotent per record_key).
        emit_homepage_exposure_findings(&c, "scan-a", "http://t/", 200, &headers, "");
        let n: i64 = c.query_row("SELECT COUNT(*) FROM sentinel_findings", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 2, "one version_leak + one cors, deduped");
        let cors: i64 = c.query_row("SELECT COUNT(*) FROM sentinel_findings WHERE kind='cors'", [], |r| r.get(0)).unwrap();
        assert_eq!(cors, 1);
    }

    #[test]
    fn error_disclosure_detected_on_banners_not_normal_pages() {
        assert!(detect_error_disclosure("Whitelabel Error Page ... org.springframework.web").is_some());
        assert!(detect_error_disclosure("You have an error in your SQL syntax near ...").is_some());
        assert!(detect_error_disclosure("<b>Warning</b>: mysqli_query() failed in /var/www").is_some());
        assert!(detect_error_disclosure("<!doctype html><title>Welcome</title><p>Hello</p>").is_none());
    }

    #[test]
    fn jwt_presence_detected_in_header_or_body() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJ1aWQiOjF9.abc123";
        assert!(detect_jwt_present(&[("authorization".into(), format!("Bearer {jwt}"))], ""));
        assert!(detect_jwt_present(&[], &format!("token={jwt}")));
        assert!(!detect_jwt_present(&[("x".into(), "eyJonly-no-dot".into())], "nothing here"));
    }
}
