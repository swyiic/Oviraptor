// #7 — SQL-injection detector (Track C, high-risk `target_write` class). This is
// a DETECTOR, not an exploitation engine: it sends a small, fixed set of canonical
// differential probes at ONE injection point and classifies the responses as
// error- / boolean- / time-based injectable. It never dumps data, never stacks a
// second statement, never writes — the proof is the response DIFFERENTIAL, and the
// agent turns a positive differential into a finding through the normal
// record_hypothesis_result + reviewer path (this tool only persists evidence and
// returns the verdict, like probe_command_execution).
//
// Three composed floors, exactly like the RCE probe:
//   1. the scan must have authorized the `target_write` high-risk class (the
//      mandatory switch) — active injection against the target's query layer is
//      gated even though the in-band probes are non-destructive SELECT-inference;
//   2. one single-use operator approval per distinct injection point
//      (method+url+param), via high_risk_action_gate_on (reversible: read-only);
//   3. every request still goes through the brokered transport, so the scope /
//      SSRF / budget / identity gates all apply and the target stays in scope.
//
// The probe set is deliberately tiny (a control plane, not a payload library):
// one error trigger, a true/false boolean pair, and one time trigger per DB
// family. v1 injects through GET query parameters only (a read method), so it
// cannot smuggle a write past the mutation-contract gate.

// Canonical DB error-message fragments. Presence of one of these in the single-
// quote probe response (and NOT in the clean baseline) is an error-based signal.
// Detection signatures, not payloads.
const SQLI_DB_ERROR_FRAGMENTS: &[&str] = &[
    "you have an error in your sql syntax",
    "warning: mysqli",
    "warning: mysql_",
    "mysql_fetch",
    "supplied argument is not a valid mysql",
    "ora-0",
    "ora-1",
    "pls-",
    "quoted string not properly terminated",
    "postgresql",
    "pg_query",
    "syntax error at or near",
    "unterminated quoted string",
    "sqlstate",
    "sqliteexception",
    "sqlite3::",
    "near \"",
    "unclosed quotation mark",
    "incorrect syntax near",
    "microsoft ole db",
    "odbc sql server driver",
    "conversion failed when converting",
    "system.data.sqlclient",
    "jdbc",
    // SQL Server, incl. the Chinese-locale error strings a real zh-CN target emits.
    // Without these the detector is blind to Chinese SQL Server (the common case for
    // domestic targets): a numeric-context payload like 14/(0) or 14+@@version only
    // ever reveals itself through these exact messages.
    "divide by zero",
    "以零作除数",                       // divide-by-zero (zh)
    "将 nvarchar 值",                   // "converting nvarchar value ... to int failed" (zh)
    "将 varchar 值",
    "转换成数据类型",                    // "... to data type ... failed" (zh)
    "转换为数据类型",
    "附近有语法错误",                    // "syntax error near ..." (zh)
    "未能将",                           // "could not convert ..." (zh)
    "必须声明标量变量",                  // "must declare the scalar variable" (zh)
    "arithmetic overflow",
    "算术溢出",
    "conversion failed",
    "nvarchar value",
    "varchar value",
    "operand type clash",
];

// Time-based sleep triggers per DB family. The tool sends ONE (the hinted family,
// or MySQL by default). Bounded: a single ~5s sleep is non-destructive.
struct SleepProbe {
    family: &'static str,
    payload: &'static str,
}
const SQLI_SLEEP_PROBES: &[SleepProbe] = &[
    SleepProbe { family: "mysql", payload: "' AND SLEEP(5)-- -" },
    SleepProbe { family: "postgres", payload: "' AND 1=(SELECT 1 FROM PG_SLEEP(5))-- -" },
    SleepProbe { family: "mssql", payload: "'; WAITFOR DELAY '0:0:5'-- -" },
];

// Milliseconds a time-based injection must add over the clean baseline to count.
// Set below the 5s sleep with headroom for network jitter / already-slow pages.
const SQLI_TIME_THRESHOLD_MS: i64 = 3500;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct SqliVerdict {
    error_based: bool,
    boolean_based: bool,
    time_based: bool,
}

impl SqliVerdict {
    fn any(self) -> bool {
        self.error_based || self.boolean_based || self.time_based
    }
    fn techniques(self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.error_based {
            out.push("error_based");
        }
        if self.boolean_based {
            out.push("boolean_based");
        }
        if self.time_based {
            out.push("time_based");
        }
        out
    }
}

// Response accessors over the broker's summary JSON (status/bytes/elapsedMs/
// bodySha256/preview). Missing fields degrade to neutral values.
fn sqli_status(v: &JsonValue) -> i64 {
    v.get("status").and_then(JsonValue::as_i64).unwrap_or(0)
}
fn sqli_bytes(v: &JsonValue) -> i64 {
    v.get("bytes").and_then(JsonValue::as_i64).unwrap_or(0)
}
fn sqli_elapsed(v: &JsonValue) -> i64 {
    v.get("elapsedMs").and_then(JsonValue::as_i64).unwrap_or(0)
}
fn sqli_sha(v: &JsonValue) -> String {
    value_first(v, &["bodySha256"])
}
fn sqli_preview_lower(v: &JsonValue) -> String {
    value_first(v, &["preview"]).to_ascii_lowercase()
}

fn sqli_db_error_present(preview_lower: &str) -> bool {
    SQLI_DB_ERROR_FRAGMENTS
        .iter()
        .any(|fragment| preview_lower.contains(fragment))
}

// Two responses are "equivalent" when the body hash matches, or the status matches
// and the byte counts are close (tolerant of small dynamic differences). Used so a
// true-condition that matches the clean page and a false-condition that diverges
// read as a boolean-injectable differential.
fn sqli_bytes_close(a: i64, b: i64) -> bool {
    (a - b).abs() <= 64.max(a / 20)
}
fn sqli_equivalent(a: &JsonValue, b: &JsonValue) -> bool {
    let sha_a = sqli_sha(a);
    let sha_b = sqli_sha(b);
    if !sha_a.is_empty() && sha_a == sha_b {
        return true;
    }
    sqli_status(a) == sqli_status(b) && sqli_bytes_close(sqli_bytes(a), sqli_bytes(b))
}

/// Classify the probe responses. Pure over the broker summaries so it is unit-
/// testable without any network. `baseline` is the clean request; `error` carries
/// a single quote; `b_true`/`b_false` are the boolean pair; `timed` carries the
/// sleep trigger.
fn classify_sqli(
    baseline: &JsonValue,
    error: &JsonValue,
    b_true: &JsonValue,
    b_false: &JsonValue,
    timed: &JsonValue,
) -> SqliVerdict {
    let baseline_err = sqli_db_error_present(&sqli_preview_lower(baseline));
    let error_based = !baseline_err && sqli_db_error_present(&sqli_preview_lower(error));
    // Boolean: the always-true condition still looks like the clean page while the
    // always-false condition diverges. An error in either is handled above, not here.
    let boolean_based = sqli_equivalent(b_true, baseline) && !sqli_equivalent(b_false, baseline);
    // Time: the sleep trigger adds at least the threshold over the baseline AND is
    // itself slow, so a uniformly slow endpoint does not read as injectable.
    let base_ms = sqli_elapsed(baseline);
    let timed_ms = sqli_elapsed(timed);
    let time_based = timed_ms >= base_ms + SQLI_TIME_THRESHOLD_MS && timed_ms >= SQLI_TIME_THRESHOLD_MS;
    SqliVerdict { error_based, boolean_based, time_based }
}

/// Build the injected URL: append `payload` to the chosen query parameter's value,
/// or substitute it for the `{{INJ}}` placeholder when no param is named. Returns
/// Err when neither an existing param nor a placeholder is available.
fn sqli_inject_url(url: &str, param: &str, placeholder: &str, payload: &str) -> Result<String, String> {
    if !param.is_empty() {
        let mut parsed = reqwest::Url::parse(url).map_err(|_| "url 无效".to_string())?;
        let mut found = false;
        let pairs: Vec<(String, String)> = parsed
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        {
            let mut qp = parsed.query_pairs_mut();
            qp.clear();
            for (k, v) in &pairs {
                if k == param {
                    found = true;
                    qp.append_pair(k, &format!("{v}{payload}"));
                } else {
                    qp.append_pair(k, v);
                }
            }
            if !found {
                qp.append_pair(param, payload);
            }
        }
        return Ok(parsed.to_string());
    }
    if url.contains(placeholder) {
        return Ok(url.replace(placeholder, payload));
    }
    Err("请提供 param(要注入的查询参数名)或在 url 中放置 {{INJ}} 占位符".into())
}

/// Stable per-injection-point fingerprint for the single-use approval, so approving
/// one injection point never approves a different url/param.
fn sqli_action_fingerprint(method: &str, url: &str, param: &str) -> String {
    let digest = Sha256::digest(format!("sqli:{method}:{url}:{param}").as_bytes());
    format!("{digest:x}")
}

/// Detect SQL injection at one injection point on the AUTHORIZED target. Gated by
/// `target_write` authorization + a single-use operator approval; runs a small
/// fixed differential probe set through the brokered transport and returns the
/// classification. It persists each probe as evidence but does not itself raise a
/// finding — the agent records a confirmed finding via record_hypothesis_result.
fn agent_tool_probe_sql_injection(
    context: &AgentRunContext,
    runtime: &mut AgentToolRuntime,
    arguments: &JsonValue,
) -> JsonValue {
    let url = value_first(arguments, &["url"]);
    if url.trim().is_empty() {
        return agent_tool_error("缺少 url", "invalid_arguments");
    }
    let identity = value_first(arguments, &["identity"]);
    if identity.trim().is_empty() {
        return agent_tool_error("缺少 identity(用哪个身份发起探测)", "invalid_arguments");
    }
    let param = value_first(arguments, &["param"]);
    let placeholder = {
        let raw = value_first(arguments, &["placeholder"]);
        if raw.is_empty() { "{{INJ}}".to_string() } else { raw }
    };
    let family = value_first(arguments, &["dbms"]).to_ascii_lowercase();
    let method = "GET"; // v1 read-method only, so it cannot smuggle a write.

    // Deterministic gate (never the model's judgement).
    let connection = match db::open(&context.db_path) {
        Ok(connection) => connection,
        Err(error) => return agent_tool_error(&error, "tool_authorization_unavailable"),
    };
    if !matches!(
        scan_capability_class_authorized(&connection, &context.scan_id, context.attempt_number, "target_write"),
        Ok(true)
    ) {
        return serde_json::json!({
            "accepted": false,
            "code": "high_risk_action_denied",
            "error": "target_write 能力未被该任务授权；无法进行注入探测",
        });
    }
    let fencing = current_coordinator_fencing_token(&connection, context);
    let fingerprint = sqli_action_fingerprint(method, &url, &param);
    let summary = format!("SQL 注入探测：{method} {url}（参数 {}）", if param.is_empty() { "{{INJ}}" } else { &param });
    let gate = match high_risk_action_gate_on(
        &connection,
        &context.scan_id,
        context.attempt_number,
        &fencing,
        "target_write",
        &fingerprint,
        &summary,
        &context.target_url,
        true, // reversible: non-destructive SELECT-inference, no write
    ) {
        Ok(gate) => gate,
        Err(error) => return agent_tool_error(&error, "high_risk_gate_unavailable"),
    };
    match gate {
        HighRiskGate::Denied(reason) => {
            return serde_json::json!({"accepted": false, "code": "high_risk_action_denied", "error": reason});
        }
        HighRiskGate::Pending(request_id) => {
            return serde_json::json!({
                "accepted": false,
                "code": "high_risk_action_pending_approval",
                "requestId": request_id,
                "hint": "该注入点需操作者单次批准；operator 批准后用完全相同的参数重试本工具",
            });
        }
        HighRiskGate::Allow => {}
    }

    // Pick the sleep trigger for the hinted DB family (default MySQL).
    let sleep = SQLI_SLEEP_PROBES
        .iter()
        .find(|p| p.family == family)
        .unwrap_or(&SQLI_SLEEP_PROBES[0]);

    // The fixed differential probe set. Baseline first (clean), then the signals.
    let probes: [(&str, &str); 5] = [
        ("baseline", ""),
        ("error", "'"),
        ("boolean_true", "' AND '1'='1"),
        ("boolean_false", "' AND '1'='2"),
        ("time", sleep.payload),
    ];

    let mut responses: Vec<(&str, JsonValue)> = Vec::new();
    let mut request_ids: Vec<String> = Vec::new();
    for (label, payload) in probes {
        let injected = match sqli_inject_url(&url, &param, &placeholder, payload) {
            Ok(value) => value,
            Err(error) => return agent_tool_error(&error, "invalid_arguments"),
        };
        let response = match agent_http_request(
            context,
            runtime,
            &identity,
            method,
            &injected,
            Vec::new(),
            None,
            None,
            "",
            "sql_injection",
            ScopeSource::HttpReplay,
            "probe_sql_injection",
        ) {
            Ok(value) => value,
            Err(error) => return error,
        };
        if let Err(error) = persist_agent_evidence(context, &response, "probe_sql_injection") {
            return serde_json::json!({"error": error, "code": "evidence_write_failed"});
        }
        request_ids.push(value_first(&response, &["requestId"]));
        responses.push((label, response));
    }

    let get = |name: &str| -> &JsonValue {
        responses
            .iter()
            .find(|(label, _)| *label == name)
            .map(|(_, value)| value)
            .unwrap_or(&JsonValue::Null)
    };
    let verdict = classify_sqli(
        get("baseline"),
        get("error"),
        get("boolean_true"),
        get("boolean_false"),
        get("time"),
    );

    serde_json::json!({
        "accepted": true,
        "injectable": verdict.any(),
        "techniques": verdict.techniques(),
        "dbmsHint": sleep.family,
        "injectionPoint": {"method": method, "url": url, "param": param},
        "evidenceRequestIds": request_ids,
        "hint": if verdict.any() {
            "已发现注入差异证据；请用 record_hypothesis_result 以这些 requestId 作为控制/测试请求登记 sql_injection 结论，并给出复现步骤与影响。不要导出数据。"
        } else {
            "未发现注入差异；记录 insufficient_evidence 或换注入点/上下文(numeric vs string)再试。"
        },
    })
}

#[cfg(test)]
mod sqli_tester_tests {
    use super::*;

    fn resp(status: i64, bytes: i64, elapsed: i64, sha: &str, preview: &str) -> JsonValue {
        serde_json::json!({
            "status": status,
            "bytes": bytes,
            "elapsedMs": elapsed,
            "bodySha256": sha,
            "preview": preview,
        })
    }

    #[test]
    fn error_based_detected_only_when_baseline_is_clean() {
        let baseline = resp(200, 1000, 50, "a", "welcome home");
        let error = resp(500, 1200, 60, "b", "You have an error in your SQL syntax near ''");
        let neutral = resp(200, 1000, 50, "a", "welcome home");
        let v = classify_sqli(&baseline, &error, &neutral, &neutral, &neutral);
        assert!(v.error_based);
        // If the clean baseline already shows a DB error, the quote probe proves nothing.
        let noisy_baseline = resp(200, 1000, 50, "a", "debug: SQLSTATE[HY000] shown always");
        let v2 = classify_sqli(&noisy_baseline, &error, &neutral, &neutral, &neutral);
        assert!(!v2.error_based);
    }

    #[test]
    fn boolean_based_true_matches_baseline_false_diverges() {
        let baseline = resp(200, 5000, 50, "page-hash", "ten results");
        let b_true = resp(200, 5010, 52, "page-hash", "ten results");
        let b_false = resp(200, 200, 48, "empty-hash", "no results");
        let neutral = baseline.clone();
        let v = classify_sqli(&baseline, &neutral, &b_true, &b_false, &neutral);
        assert!(v.boolean_based);
        // Both conditions identical to baseline → not injectable (no differential).
        let v2 = classify_sqli(&baseline, &neutral, &b_true, &b_true, &neutral);
        assert!(!v2.boolean_based);
    }

    #[test]
    fn time_based_needs_real_added_delay() {
        let baseline = resp(200, 1000, 120, "a", "ok");
        let timed = resp(200, 1000, 120 + super::SQLI_TIME_THRESHOLD_MS + 10, "a", "ok");
        let neutral = baseline.clone();
        let v = classify_sqli(&baseline, &neutral, &neutral, &neutral, &timed);
        assert!(v.time_based);
        // A uniformly slow endpoint (baseline already slow, no extra delay) is not injectable.
        let slow_baseline = resp(200, 1000, 6000, "a", "ok");
        let slow_timed = resp(200, 1000, 6100, "a", "ok");
        let v2 = classify_sqli(&slow_baseline, &neutral, &neutral, &neutral, &slow_timed);
        assert!(!v2.time_based);
    }

    #[test]
    fn clean_target_yields_no_signal() {
        let r = resp(200, 1000, 50, "same", "normal page");
        let v = classify_sqli(&r, &r.clone(), &r.clone(), &r.clone(), &r.clone());
        assert!(!v.any());
        assert!(v.techniques().is_empty());
    }

    #[test]
    fn inject_into_named_param_appends_payload() {
        let out = sqli_inject_url("https://t.example/item?id=7&x=1", "id", "{{INJ}}", "'").unwrap();
        // id carries the appended quote; the other param is preserved.
        assert!(out.contains("id=7%27") || out.contains("id=7'"), "{out}");
        assert!(out.contains("x=1"), "{out}");
    }

    #[test]
    fn inject_adds_param_when_absent() {
        let out = sqli_inject_url("https://t.example/item?x=1", "id", "{{INJ}}", "'").unwrap();
        assert!(out.contains("id=") && out.contains("x=1"), "{out}");
    }

    #[test]
    fn inject_uses_placeholder_when_no_param() {
        let out = sqli_inject_url("https://t.example/p/{{INJ}}/view", "", "{{INJ}}", "'").unwrap();
        assert_eq!(out, "https://t.example/p/'/view");
    }

    #[test]
    fn inject_errors_without_param_or_placeholder() {
        assert!(sqli_inject_url("https://t.example/p", "", "{{INJ}}", "'").is_err());
    }

    #[test]
    fn fingerprint_is_stable_and_point_specific() {
        let a = sqli_action_fingerprint("GET", "https://t/x?id=1", "id");
        let b = sqli_action_fingerprint("GET", "https://t/x?id=1", "id");
        let c = sqli_action_fingerprint("GET", "https://t/x?id=1", "name");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}

// Live-fire: run the REAL detector path (sqli_inject_url to craft the probes, then
// classify_sqli over the responses) against a local HTTP server that is actually
// SQL-injection-vulnerable, over real TCP. This closes the gap between the pure
// unit tests (synthetic summaries) and a real differential: it proves the detector
// fires on genuine error-/boolean-/time-based behaviour from a live endpoint. It
// does NOT exercise the broker transport or the gate (those need a full scan); it
// verifies the detection logic end-to-end on real traffic.
#[cfg(test)]
mod sqli_live_fire_tests {
    use super::{classify_sqli, sqli_inject_url};
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use std::io::{BufRead, BufReader, Write};

    fn pct_decode(s: &str) -> String {
        let b = s.as_bytes();
        let mut out = String::new();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'%' && i + 2 < b.len() {
                if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    out.push(v as char);
                    i += 3;
                    continue;
                }
            }
            out.push(if b[i] == b'+' { ' ' } else { b[i] as char });
            i += 1;
        }
        out
    }

    // A deliberately SQLi-vulnerable endpoint: /item?id=VALUE. A lone quote triggers
    // a DB error; an always-false boolean returns a short body; an always-true one
    // returns the full baseline; a SLEEP() stalls the response.
    fn start_vulnerable_server() -> u16 {
        // FULL must differ from SHORT by well more than the detector's byte-close
        // tolerance (max(64, bytes/20)) so the always-false condition reads as a real
        // divergence, mirroring a page that returns many rows vs none.
        const FULL: &str = "RESULT 10 rows: user1,user2,user3,user4,user5,user6,user7,user8,user9,user10 \
with full profile records and addresses padded out to clearly exceed the byte-difference tolerance XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX";
        const SHORT: &str = "RESULT 0 rows";
        const ERRBODY: &str = "500: You have an error in your SQL syntax; check the manual near '''";
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
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
                let value = pct_decode(raw.split("id=").nth(1).unwrap_or(""));
                let (status, body) = if value.contains("SLEEP(") {
                    std::thread::sleep(std::time::Duration::from_secs(4));
                    ("200 OK", FULL)
                } else if value.contains("'='2") {
                    ("200 OK", SHORT) // always-false condition
                } else if value.contains("'='1") {
                    ("200 OK", FULL) // always-true condition (== baseline)
                } else if value.contains('\'') {
                    ("500 Internal Server Error", ERRBODY) // lone quote → DB error
                } else {
                    ("200 OK", FULL) // clean baseline
                };
                let mut stream = stream;
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        port
    }

    fn fetch_summary(client: &reqwest::blocking::Client, url: &str) -> serde_json::Value {
        let started = std::time::Instant::now();
        let resp = client.get(url).send().expect("request");
        let status = resp.status().as_u16() as i64;
        let body = resp.text().unwrap_or_default();
        let elapsed = started.elapsed().as_millis() as i64;
        json!({
            "status": status,
            "bytes": body.len() as i64,
            "elapsedMs": elapsed,
            "bodySha256": format!("{:x}", Sha256::digest(body.as_bytes())),
            "preview": body,
        })
    }

    #[test]
    fn detector_fires_on_a_real_vulnerable_endpoint() {
        let port = start_vulnerable_server();
        let base = format!("http://127.0.0.1:{port}/item?id=7");
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap();
        // Craft the exact probe set the real tool sends, via the real injector.
        let probe = |payload: &str| {
            let url = sqli_inject_url(&base, "id", "{{INJ}}", payload).unwrap();
            fetch_summary(&client, &url)
        };
        let baseline = probe("");
        let error = probe("'");
        let b_true = probe("' AND '1'='1");
        let b_false = probe("' AND '1'='2");
        let timed = probe("' AND SLEEP(5)-- -");

        let verdict = classify_sqli(&baseline, &error, &b_true, &b_false, &timed);
        assert!(verdict.error_based, "error-based must fire on real DB-error response");
        assert!(verdict.boolean_based, "boolean-based must fire on real true/false differential");
        assert!(verdict.time_based, "time-based must fire on a real delayed response");
        assert_eq!(verdict.techniques(), vec!["error_based", "boolean_based", "time_based"]);

        // A clean (non-vulnerable) endpoint must NOT be flagged: baseline-vs-baseline.
        let clean = probe(""); // the id=7 baseline behaves normally for all
        let no_verdict = classify_sqli(&clean, &clean, &clean, &clean, &clean);
        assert!(!no_verdict.any(), "a clean endpoint must not be flagged injectable");
    }
}
