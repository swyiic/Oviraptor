// Per-finding deep-dive (exploitation) engine — the "深挖面" that runs AFTER the
// breadth attack sweep converges. For every non-info finding the sweep wrote, this
// reproduces it with a focused, minimal confirmation, decides whether it is really
// exploitable, and records SRC-grade evidence (full reproduction request as curl,
// impact with PII redacted, a bounded iteration trace) into agent_finding_exploits,
// always attributed to the finding_id.
//
// Convergence (the point of this phase): the finding set is finite and every
// per-finding dive is bounded (a fixed, small set of focused requests + the client's
// own per-request timeout), with a global wall-clock guard across findings. Progress
// is OUTCOME-based: a finding advances only by reaching a terminal verdict
// (exploited / not_exploitable), never by mere exploration. When no finding is left
// 'pending' the phase is complete; it cannot spin.
//
// Red lines (preserved): the whole phase is gated on the scan's target_write
// capability class (scan_capability_class_authorized) exactly like the sweep — it
// skips silently when unauthorized. It never auto-attests legal authorization, never
// does code_exec / self-written scripts (MVP uses deterministic Rust probes only),
// stays same-origin behind the SSRF floor, and uses detection-grade payloads only
// (harmless id/sleep, benign markers, one negative value, low-harm mass-assign).
// A blocked response can enter a bounded bypass loop (encoding, case, one
// documentation-network source header). CAPTCHA is not solved. Four misses
// record waf_blocked. The generated Python is a reproduction artifact; it is
// not executed here and has no network grant of its own.
//
// Included into scanning.rs AFTER attack_sweep.rs, so it reuses that unit's
// InjectionPoint, sweep_collect_points / sweep_collect_get_urls and the sweep probes.

const DEEP_DIVE_MAX_FINDINGS: usize = 50;
const DEEP_DIVE_GLOBAL_WALL_SECS: u64 = 90;
const DEEP_DIVE_IMPACT_SAMPLE_CHARS: usize = 600;

/// Severity rank for deep-dive ordering (critical first). Anything unknown sorts last.
fn deep_dive_severity_rank(severity: &str) -> u8 {
    match severity {
        "critical" => 0,
        "high" => 1,
        "medium" => 2,
        "low" => 3,
        _ => 4,
    }
}

/// Single-quote a string for a POSIX shell, so a reproduction curl is copy-pasteable
/// even when the payload contains quotes (SQLi `'))`, JSON bodies, …).
fn shell_single_quote(raw: &str) -> String {
    format!("'{}'", raw.replace('\'', "'\\''"))
}

/// Build a copy-pasteable reproduction `curl` for one request.
fn build_curl(method: &str, url: &str, headers: &[(String, String)], body: Option<&str>) -> String {
    let mut parts = vec![format!("curl -i -s -X {} {}", method.to_ascii_uppercase(), shell_single_quote(url))];
    for (name, value) in headers {
        parts.push(format!("-H {}", shell_single_quote(&format!("{name}: {value}"))));
    }
    if let Some(data) = body {
        parts.push(format!("--data {}", shell_single_quote(data)));
    }
    parts.join(" \\\n  ")
}

/// Truncate to `max` chars, flagging that the sample was cut.
fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max).collect();
    format!("{kept}…(已截断)")
}

/// Collapse a run of 7+ consecutive digits (card/phone/id-like) into a marker, so a
/// redacted impact sample never carries long numeric identifiers.
fn redact_digit_runs(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut run = String::new();
    let flush = |out: &mut String, run: &mut String| {
        if run.chars().count() >= 7 {
            out.push_str("[已脱敏数字]");
        } else {
            out.push_str(run);
        }
        run.clear();
    };
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            run.push(ch);
        } else {
            flush(&mut out, &mut run);
            out.push(ch);
        }
    }
    flush(&mut out, &mut run);
    out
}

/// Mask the local part of anything that looks like an email: `admin@juice.sh`
/// becomes `***@juice.sh`, keeping the domain (useful signal) but dropping the PII.
fn mask_email_locals(text: &str) -> String {
    let is_local = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '%' | '+' | '-');
    let mut out = String::with_capacity(text.len());
    let mut token = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if is_local(c) {
            token.push(c);
        } else if c == '@'
            && !token.is_empty()
            && chars.peek().map(|n| n.is_ascii_alphanumeric()).unwrap_or(false)
        {
            out.push_str("***@");
            token.clear();
        } else {
            out.push_str(&token);
            token.clear();
            out.push(c);
        }
    }
    out.push_str(&token);
    out
}

/// Redact PII from an evidence sample before it is stored as `impact`: mask emails,
/// collapse long digit runs, and bound the length. Pure, so it is unit-tested.
fn redact_pii(sample: &str, max: usize) -> String {
    truncate_chars(&mask_email_locals(&redact_digit_runs(sample)), max)
}

/// The concrete (method, url, body, content-type) a point is probed with, for a given
/// payload — mirrors sweep_send's construction so the reproduction curl is exact.
fn point_reproduction_curl(point: &InjectionPoint, payload: &str) -> String {
    if let Some(param) = &point.query_param {
        let url = sqli_inject_url(&point.url, param, "{{INJ}}", payload).unwrap_or_else(|_| point.url.clone());
        build_curl(&point.method, &url, &[], None)
    } else if let (Some(field), Some(base)) = (&point.body_field, &point.base_body) {
        let mut body = base.clone();
        if let Some(obj) = body.as_object_mut() {
            let base_val = obj.get(field).and_then(JsonValue::as_str).unwrap_or("test").to_string();
            obj.insert(field.clone(), JsonValue::String(format!("{base_val}{payload}")));
        }
        build_curl(
            &point.method,
            &point.url,
            &[("Content-Type".to_string(), "application/json".to_string())],
            Some(&body.to_string()),
        )
    } else {
        build_curl(&point.method, &point.url, &[], None)
    }
}

/// One step in a finding's bounded deep-dive trace.
fn trace_step(step: usize, action: &str, detail: JsonValue) -> JsonValue {
    serde_json::json!({ "step": step, "action": action, "detail": detail })
}

/// Everything deep_dive_one needs for one finding.
struct DeepDiveInput<'a> {
    scan_id: &'a str,
    finding_id: i64,
    kind: &'a str,
    severity: &'a str,
    record: &'a JsonValue,
    point: Option<&'a InjectionPoint>,
    url: Option<&'a str>,
    display_urls: &'a [String],
}

/// Focused, bounded deep-dive of ONE finding. Reuses the sweep's tested probes to
/// re-confirm, builds the reproduction curl, redacts the impact sample, and returns a
/// FindingExploit ready to persist. `requests_used` is a nominal upper bound for the
/// focused confirmation (the reused probes do not report an exact count); the real
/// bound on the phase is the finite finding set + the global wall-clock guard.
fn deep_dive_one(connection: &rusqlite::Connection, client: &reqwest::blocking::Client, input: &DeepDiveInput) -> FindingExploit {
    let mut record = FindingExploit::pending(input.scan_id, input.finding_id, input.severity);
    record.deep_status = DEEP_STATUS_IN_PROGRESS.to_string();
    let mut trace: Vec<JsonValue> = Vec::new();
    let sleep = SQLI_SLEEP_PROBES[0].payload;

    let (status, impact, reproduction, requests, poc_output) = match input.kind {
        "sql_injection" => {
            let payload = input
                .record
                .pointer("/evidence/errorContext")
                .and_then(JsonValue::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or("' AND '1'='1")
                .to_string();
            match input.point {
                Some(point) => {
                    let reproduction = point_reproduction_curl(point, &payload);
                    let confirmed = sweep_probe_point(client, point, sleep)
                        .map(|(verdict, ev)| (verdict.any(), ev))
                        .unwrap_or((false, JsonValue::Null));
                    trace.push(trace_step(1, "replay_confirm", serde_json::json!({"reProbed": confirmed.1})));
                    if confirmed.0 {
                        let techniques = input.record.pointer("/techniques").cloned().unwrap_or(JsonValue::Null);
                        (
                            DEEP_STATUS_EXPLOITED,
                            format!("SQL 注入可复现（技术：{techniques}）。注入上下文 {payload} 触发数据库差异，可进一步取数据库版本/数据。"),
                            reproduction,
                            14,
                            confirmed.1.to_string(),
                        )
                    } else {
                        (DEEP_STATUS_NOT_EXPLOITABLE, "重放未复现 SQL 注入差异，判定当前不可利用。".to_string(), reproduction, 14, String::new())
                    }
                }
                None => (DEEP_STATUS_ERROR, "无法从 recon 定位该注入点（接口可能已变化）。".to_string(), String::new(), 0, String::new()),
            }
        }
        "broken_access" | "sensitive_exposure" => match input.url {
            Some(url) => {
                let reproduction = build_curl("GET", url, &[("User-Agent".to_string(), "oviraptor-deepdive/1.0".to_string())], None);
                match client.get(url).header(reqwest::header::USER_AGENT, "oviraptor-deepdive/1.0").send() {
                    Ok(resp) => {
                        let status_code = resp.status().as_u16();
                        let mut buf = Vec::new();
                        let _ = std::io::Read::take(resp, 262_144).read_to_end(&mut buf);
                        let body = String::from_utf8_lossy(&buf);
                        trace.push(trace_step(1, "anonymous_get", serde_json::json!({"status": status_code, "bytes": body.len()})));
                        if status_code == 200 {
                            let leaks_email = sweep_contains_email(&body);
                            let sample = redact_pii(&body, DEEP_DIVE_IMPACT_SAMPLE_CHARS);
                            (
                                DEEP_STATUS_EXPLOITED,
                                format!("匿名（无凭据）即可 200 读取。含邮箱={leaks_email}。脱敏样本：{sample}"),
                                reproduction,
                                1,
                                String::new(),
                            )
                        } else {
                            (DEEP_STATUS_NOT_EXPLOITABLE, format!("匿名重放返回 {status_code}，访问控制生效，判定不可利用。"), reproduction, 1, String::new())
                        }
                    }
                    Err(_) => (DEEP_STATUS_ERROR, "匿名重放请求失败（网络/超时）。".to_string(), reproduction, 1, String::new()),
                }
            }
            None => (DEEP_STATUS_ERROR, "发现记录缺少可复现 URL。".to_string(), String::new(), 0, String::new()),
        },
        "mass_assignment" => match input.point {
            Some(point) => {
                let reproduction = point.base_body.as_ref().map(|base| {
                    let body = mass_assign_probe_body(base);
                    build_curl(&point.method, &point.url, &[("Content-Type".to_string(), "application/json".to_string())], Some(&body.to_string()))
                }).unwrap_or_default();
                match sweep_probe_mass_assignment(client, point, agent_post_write_enabled(connection, input.scan_id)) {
                    Some(ev) => {
                        trace.push(trace_step(1, "mass_assign_confirm", ev.clone()));
                        (DEEP_STATUS_EXPLOITED, "创建/更新接口接受并回显未暴露的权限字段（role/isAdmin…），可权限提升/越权。".to_string(), reproduction, 1, ev.to_string())
                    }
                    None => (DEEP_STATUS_NOT_EXPLOITABLE, "重放未再接受权限字段，判定不可利用。".to_string(), reproduction, 1, String::new()),
                }
            }
            None => (DEEP_STATUS_ERROR, "无法定位批量赋值的写接口。".to_string(), String::new(), 0, String::new()),
        },
        "stored_xss" => match input.point {
            Some(point) => {
                let reproduction = point_reproduction_curl(point, "<ovrx…>");
                let bounded: Vec<String> = input.display_urls.iter().take(8).cloned().collect();
                match sweep_probe_stored_xss(client, point, &bounded, agent_post_write_enabled(connection, input.scan_id)) {
                    Some(ev) => {
                        trace.push(trace_step(1, "stored_xss_confirm", ev.clone()));
                        (DEEP_STATUS_EXPLOITED, "注入标记被持久化且未编码渲染在展示面，存在存储型 XSS。".to_string(), reproduction, 9, ev.to_string())
                    }
                    None => (DEEP_STATUS_NOT_EXPLOITABLE, "重放未复现持久化未编码标记，判定不可利用。".to_string(), reproduction, 9, String::new()),
                }
            }
            None => (DEEP_STATUS_ERROR, "无法定位存储型 XSS 的写接口。".to_string(), String::new(), 0, String::new()),
        },
        "reflected_xss" => match input.point {
            Some(point) => {
                let reproduction = point_reproduction_curl(point, "<ovrx…>");
                match sweep_probe_xss(client, point, agent_post_write_enabled(connection, input.scan_id)) {
                    Some(ev) => {
                        trace.push(trace_step(1, "reflected_xss_confirm", ev.clone()));
                        (DEEP_STATUS_EXPLOITED, "注入标记在同一响应未编码回显，反射型 XSS（低危）。".to_string(), reproduction, 1, ev.to_string())
                    }
                    None => (DEEP_STATUS_NOT_EXPLOITABLE, "重放未复现未编码回显，判定不可利用。".to_string(), reproduction, 1, String::new()),
                }
            }
            None => (DEEP_STATUS_ERROR, "无法定位反射型 XSS 的注入点。".to_string(), String::new(), 0, String::new()),
        },
        "command_injection" => match input.point {
            Some(point) => {
                let reproduction = point_reproduction_curl(point, ";id");
                match sweep_probe_cmdi(client, point, agent_post_write_enabled(connection, input.scan_id)) {
                    Some(ev) => {
                        trace.push(trace_step(1, "cmdi_confirm", ev.clone()));
                        (DEEP_STATUS_EXPLOITED, "命令注入可复现（id 回显或注入 sleep 造成时延），等同远程命令执行。".to_string(), reproduction, 7, ev.to_string())
                    }
                    None => (DEEP_STATUS_NOT_EXPLOITABLE, "重放未复现命令注入信号，判定不可利用。".to_string(), reproduction, 7, String::new()),
                }
            }
            None => (DEEP_STATUS_ERROR, "无法定位命令注入点。".to_string(), String::new(), 0, String::new()),
        },
        "account_enumeration" => match input.point {
            Some(point) => {
                let reproduction = point_reproduction_curl(point, "<标识>");
                match sweep_probe_auth_enum(client, point, agent_post_write_enabled(connection, input.scan_id)) {
                    Some(ev) => {
                        trace.push(trace_step(1, "auth_enum_confirm", ev.clone()));
                        (DEEP_STATUS_EXPLOITED, "存在/不存在标识的响应差异可复现，可枚举账号存在性。".to_string(), reproduction, 2, ev.to_string())
                    }
                    None => (DEEP_STATUS_NOT_EXPLOITABLE, "重放未复现枚举差异，判定不可利用。".to_string(), reproduction, 2, String::new()),
                }
            }
            None => (DEEP_STATUS_ERROR, "无法定位账号枚举接口。".to_string(), String::new(), 0, String::new()),
        },
        "business_logic" => match input.point {
            Some(point) => {
                let reproduction = point.base_body.as_ref().map(|base| {
                    let mut body = base.clone();
                    if let (Some(field), Some(o)) = (&point.body_field, body.as_object_mut()) {
                        o.insert(field.clone(), serde_json::json!(-1));
                    }
                    build_curl(&point.method, &point.url, &[("Content-Type".to_string(), "application/json".to_string())], Some(&body.to_string()))
                }).unwrap_or_default();
                match sweep_probe_business_numeric(client, point, agent_post_write_enabled(connection, input.scan_id)) {
                    Some(ev) => {
                        trace.push(trace_step(1, "business_numeric_confirm", ev.clone()));
                        (DEEP_STATUS_EXPLOITED, "负值仍被接受（2xx），业务逻辑缺陷候选——是否可利用需业务判断。".to_string(), reproduction, 1, ev.to_string())
                    }
                    None => (DEEP_STATUS_NOT_EXPLOITABLE, "重放未复现负值被接受，判定不可利用。".to_string(), reproduction, 1, String::new()),
                }
            }
            None => (DEEP_STATUS_ERROR, "无法定位业务逻辑写接口。".to_string(), String::new(), 0, String::new()),
        },
        other => (
            DEEP_STATUS_NOT_EXPLOITABLE,
            format!("暂无针对 {other} 的确定性深挖手法（MVP），保留原始发现。"),
            String::new(),
            0,
            String::new(),
        ),
    };

    let mut status = status.to_string();
    let mut impact = impact;
    let mut reproduction = reproduction;
    let mut requests = requests;
    let target = input.point.map(|point| point.url.as_str()).or(input.url);
    if status == DEEP_STATUS_NOT_EXPLOITABLE {
        if let Some(url) = target {
            if let Some(sample) = client.get(url).send().ok().and_then(|response| {
                let code = response.status().as_u16();
                response.text().ok().map(|body| (code, body))
            }) {
                if looks_like_waf(sample.0, &sample.1) {
                    trace.push(trace_step(trace.len() + 1, "waf_detected", serde_json::json!({"status": sample.0})));
                    requests += 1;
                    let outcome = bounded_waf_bypass(connection, client, url, sample.0, &sample.1);
                    if let Some(hit) = outcome.hit {
                        trace.extend(hit.trace);
                        requests += hit.requests;
                        status = DEEP_STATUS_EXPLOITED.to_string();
                        impact = format!("模型第 {} 轮同主机建议得到非拦截响应。{}", hit.attempt, outcome.note);
                        reproduction = hit.curl;
                    } else {
                        status = DEEP_STATUS_WAF_BLOCKED.to_string();
                        impact = outcome.note;
                    }
                }
            }
        }
    }
    record.poc_script = python_poc(&reproduction);
    record.deep_status = status;
    record.impact = impact;
    record.reproduction_request = reproduction;
    record.poc_output = poc_output;
    record.requests_used = requests;
    record.iterations_used = trace.len() as i64;
    record.exploit_trace = JsonValue::Array(trace);
    record
}

fn looks_like_waf(status: u16, body: &str) -> bool {
    if !matches!(status, 403 | 406 | 429 | 503) { return false; }
    let lower = body.to_ascii_lowercase();
    ["captcha", "cloudflare", "attention required", "access denied", "waf", "blocked", "bot"].iter().any(|marker| lower.contains(marker))
}

fn header_map(pairs: &[(&str, &str)]) -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();
    for (name, value) in pairs {
        if let (Ok(name), Ok(value)) = (
            reqwest::header::HeaderName::from_bytes(name.as_bytes()),
            reqwest::header::HeaderValue::from_str(value),
        ) {
            headers.insert(name, value);
        }
    }
    headers
}

fn run_generated_poc(connection: &rusqlite::Connection, scan_id: &str, script: &str) -> Option<String> {
    if script.trim().is_empty() { return None; }
    let attempt: i64 = connection.query_row(
        "SELECT attempt_count FROM sentinel_scans WHERE id=?1",
        [scan_id],
        |row| row.get(0),
    ).unwrap_or(0);
    if attempt <= 0 || !scan_capability_class_authorized(connection, scan_id, attempt, "code_exec").unwrap_or(false) {
        return None;
    }
    let fingerprint = format!("{:x}", Sha256::digest(format!("analysis_script:{}", script.trim()).as_bytes()));
    match high_risk_action_gate_on(connection, scan_id, attempt, "deep-dive", "code_exec", &fingerprint, "深挖生成的本地复现脚本（沙箱、禁网）", "local-sandbox", true, Some("run_analysis_script")) {
        Ok(HighRiskGate::Allow) => {}
        Ok(HighRiskGate::Pending(_)) => return Some("脚本已生成，等待操作者批准后才会在沙箱执行。".into()),
        _ => return None,
    }
    let dir = std::env::temp_dir().join(format!("oviraptor-poc-{}", Uuid::new_v4()));
    if std::fs::create_dir_all(&dir).is_err() || std::fs::write(dir.join("script.py"), script.as_bytes()).is_err() {
        let _ = std::fs::remove_dir_all(&dir);
        return Some("沙箱目录不可用，脚本只保留正文。".into());
    }
    let result = crate::tool_supply::macos_static_parser::run_sandboxed(
        std::path::Path::new("/usr/bin/python3"),
        &["script.py".to_string()],
        &dir,
        &dir,
        &crate::tool_supply::macos_static_parser::SandboxLimits::default(),
        &|| false,
    );
    let _ = std::fs::remove_dir_all(&dir);
    Some(match result {
        Ok(run) => format!("exit={:?}\n{}", run.exit_code, String::from_utf8_lossy(&run.stdout).chars().take(4_000).collect::<String>()),
        Err(error) => format!("沙箱未执行：{}", error.describe()),
    })
}

fn python_poc(curl: &str) -> String {
    if curl.trim().is_empty() { return String::new(); }
    format!(
        "# Generated reproduction. Does not run by itself and does not bypass the capability gate.\nprint({})\n",
        serde_json::to_string(curl).unwrap_or_else(|_| "\"\"".into())
    )
}

/// A finding row to deep-dive, as read from sentinel_findings.
struct DeepDiveFinding {
    finding_id: i64,
    kind: String,
    severity: String,
    record_key: String,
    record: JsonValue,
}

/// Non-info attack-stage findings for a scan, severity-ordered (critical first),
/// bounded to DEEP_DIVE_MAX_FINDINGS.
fn deep_dive_collect_findings(connection: &rusqlite::Connection, scan_id: &str) -> Vec<DeepDiveFinding> {
    let mut statement = match connection.prepare(
        "SELECT id, kind, severity, record_key, record_json FROM sentinel_findings \
         WHERE scan_id=?1 AND stage='attack' AND severity IN ('critical','high','medium','low')",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = statement
        .query_map([scan_id], |row| {
            Ok(DeepDiveFinding {
                finding_id: row.get(0)?,
                kind: row.get(1)?,
                severity: row.get(2)?,
                record_key: row.get(3)?,
                record: serde_json::from_str(&row.get::<_, String>(4)?).unwrap_or(JsonValue::Null),
            })
        })
        .map(|it| it.flatten().collect::<Vec<_>>())
        .unwrap_or_default();
    let mut rows = rows;
    rows.sort_by(|a, b| {
        deep_dive_severity_rank(&a.severity)
            .cmp(&deep_dive_severity_rank(&b.severity))
            .then(a.finding_id.cmp(&b.finding_id))
    });
    rows.truncate(DEEP_DIVE_MAX_FINDINGS);
    rows
}

/// Resolve the InjectionPoint / URL a finding maps back to, by kind + record_key.
fn deep_dive_resolve<'a>(
    finding: &DeepDiveFinding,
    points_by_label: &'a HashMap<String, &'a InjectionPoint>,
    points_by_url: &'a HashMap<String, &'a InjectionPoint>,
    get_urls_by_path: &'a HashMap<String, String>,
) -> (Option<&'a InjectionPoint>, Option<&'a str>) {
    match finding.kind.as_str() {
        "broken_access" | "sensitive_exposure" => (
            None,
            get_urls_by_path.get(&finding.record_key).map(String::as_str),
        ),
        "mass_assignment" => (points_by_url.get(&finding.record_key).copied(), None),
        _ => (points_by_label.get(&finding.record_key).copied(), None),
    }
}

/// Run the deep-dive phase for a scan. Returns a summary; `complete` is true when no
/// non-info finding is left 'pending'. Opens its own connection, never panics, and
/// SKIPS silently when the scan did not authorize target_write.
pub(crate) fn deep_dive_for_scan(db_path: &std::path::Path, scan_id: &str, base_url: &str) -> JsonValue {
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
        return skip("target_write 未授权，跳过深挖");
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

    let points = sweep_collect_points(&connection, scan_id, &base);
    let get_urls = sweep_collect_get_urls(&connection, scan_id, &base);
    let mut points_by_label: HashMap<String, &InjectionPoint> = HashMap::new();
    let mut points_by_url: HashMap<String, &InjectionPoint> = HashMap::new();
    for point in &points {
        points_by_label.entry(point.label.clone()).or_insert(point);
        points_by_url.entry(point.url.clone()).or_insert(point);
    }
    let mut get_urls_by_path: HashMap<String, String> = HashMap::new();
    for url in &get_urls {
        if let Ok(parsed) = reqwest::Url::parse(url) {
            get_urls_by_path
                .entry(parsed.path().to_ascii_lowercase())
                .or_insert_with(|| url.clone());
        }
    }

    let findings = deep_dive_collect_findings(&connection, scan_id);
    let total = findings.len();
    // Give every finding a pending row up front, so the UI can attribute the phase to
    // each vulnerability even before (or if we never reach) its dive.
    for finding in &findings {
        if read_finding_exploit(&connection, scan_id, finding.finding_id).ok().flatten().is_none() {
            let _ = upsert_finding_exploit(&connection, &FindingExploit::pending(scan_id, finding.finding_id, &finding.severity));
        }
    }

    let started = Instant::now();
    let (mut exploited, mut not_exploitable, mut errored, mut skipped_terminal) = (0usize, 0, 0, 0);
    let mut budget_stopped = false;
    for finding in &findings {
        if started.elapsed().as_secs() >= DEEP_DIVE_GLOBAL_WALL_SECS {
            budget_stopped = true;
            break; // remaining findings stay 'pending'; the phase still returns (no spin)
        }
        // Already-terminal findings are not re-dived (outcome already produced).
        if let Ok(Some(existing)) = read_finding_exploit(&connection, scan_id, finding.finding_id) {
            if existing.is_terminal() {
                skipped_terminal += 1;
                continue;
            }
        }
        let (point, url) = deep_dive_resolve(finding, &points_by_label, &points_by_url, &get_urls_by_path);
        let input = DeepDiveInput {
            scan_id,
            finding_id: finding.finding_id,
            kind: &finding.kind,
            severity: &finding.severity,
            record: &finding.record,
            point,
            url,
            display_urls: &get_urls,
        };
        let mut record = deep_dive_one(&connection, &client, &input);
        if record.poc_output.is_empty() {
            if let Some(output) = run_generated_poc(&connection, scan_id, &record.poc_script) {
                record.poc_output = output;
            }
        }
        match record.deep_status.as_str() {
            DEEP_STATUS_EXPLOITED => exploited += 1,
            DEEP_STATUS_NOT_EXPLOITABLE => not_exploitable += 1,
            _ => errored += 1,
        }
        let _ = upsert_finding_exploit(&connection, &record);
    }

    let pending: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM agent_finding_exploits WHERE scan_id=?1 AND deep_status IN ('pending','in_progress')",
            [scan_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    serde_json::json!({
        "ran": true,
        "findings": total,
        "exploited": exploited,
        "notExploitable": not_exploitable,
        "errored": errored,
        "skippedTerminal": skipped_terminal,
        "pending": pending,
        "budgetStopped": budget_stopped,
        "complete": pending == 0,
        "elapsedMs": started.elapsed().as_millis() as i64,
    })
}

/// Operator-triggered deep-dive. With `finding_id`, that one finding is reset to
/// pending and re-dived (the "再深挖一次" path); without it, the whole scan is dived.
/// Same target_write gate and SSRF floor as the sweep.
#[tauri::command]
pub fn run_finding_deep_dive(
    state: State<AppState>,
    scan_id: String,
    base_url: String,
    finding_id: Option<i64>,
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
    let attempt = connection
        .query_row("SELECT attempt_count FROM sentinel_scans WHERE id=?1", [&scan_id], |r| r.get::<_, i64>(0))
        .unwrap_or(1)
        .max(1);
    if !scan_capability_class_authorized(&connection, &scan_id, attempt, "target_write").unwrap_or(false) {
        return Err("深挖需先为该任务授权 target_write 能力并完成合法授权声明(强制开关)".into());
    }
    // Reset the one finding so it is re-dived; dropping the row lets the phase remake
    // a pending row and run it again.
    if let Some(fid) = finding_id {
        connection
            .execute("DELETE FROM agent_finding_exploits WHERE scan_id=?1 AND finding_id=?2", params![scan_id, fid])
            .map_err(|e| e.to_string())?;
    }
    drop(connection);
    Ok(deep_dive_for_scan(&state.db_path, &scan_id, &base_url))
}

#[cfg(test)]
mod agent_finding_deep_dive_tests {
    use super::*;

    #[test]
    fn severity_rank_orders_critical_first() {
        assert!(deep_dive_severity_rank("critical") < deep_dive_severity_rank("high"));
        assert!(deep_dive_severity_rank("high") < deep_dive_severity_rank("medium"));
        assert!(deep_dive_severity_rank("medium") < deep_dive_severity_rank("low"));
        assert!(deep_dive_severity_rank("low") < deep_dive_severity_rank("info"));
        assert!(deep_dive_severity_rank("low") < deep_dive_severity_rank(""));
    }

    #[test]
    fn curl_is_copy_pasteable_and_quotes_payloads() {
        let c = build_curl("GET", "http://t/x?q=1'))", &[], None);
        assert!(c.starts_with("curl -i -s -X GET "));
        // a single quote in the URL is shell-escaped, not left raw
        assert!(c.contains("'\\''"), "{c}");
        let p = build_curl(
            "POST",
            "http://t/login",
            &[("Content-Type".to_string(), "application/json".to_string())],
            Some(r#"{"email":"a'b"}"#),
        );
        assert!(p.contains("-H 'Content-Type: application/json'"), "{p}");
        assert!(p.contains("--data "), "{p}");
    }

    #[test]
    fn redact_masks_emails_and_long_digits() {
        let out = redact_pii("user admin@juice.sh card 4111111111111111 pin 123", 1000);
        assert!(out.contains("***@juice.sh"), "email local masked: {out}");
        assert!(!out.contains("admin@"), "{out}");
        assert!(out.contains("[已脱敏数字]"), "long digit run masked: {out}");
        // a short number (<7 digits) is kept
        assert!(out.contains("123"), "{out}");
    }

    #[test]
    fn redact_truncates_to_bound() {
        let long = "a".repeat(5000);
        let out = redact_pii(&long, 100);
        assert!(out.chars().count() <= 100 + "…(已截断)".chars().count());
        assert!(out.ends_with("…(已截断)"));
    }

    #[test]
    fn point_curl_for_query_param_injects_payload() {
        let point = InjectionPoint {
            method: "GET".into(),
            url: "http://t/rest/products/search?q=apple".into(),
            body_field: None,
            base_body: None,
            query_param: Some("q".into()),
            label: "GET /rest/products/search?q".into(),
        };
        let curl = point_reproduction_curl(&point, "'))");
        assert!(curl.contains("curl -i -s -X GET"), "{curl}");
        // the payload is appended to the q value (url-encoded by sqli_inject_url)
        assert!(curl.contains("q=apple"), "{curl}");
    }

    fn findings_db() -> rusqlite::Connection {
        let c = rusqlite::Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE sentinel_findings(id INTEGER PRIMARY KEY AUTOINCREMENT, scan_id TEXT, target_url TEXT, \
             stage TEXT, kind TEXT, record_key TEXT, title TEXT, severity TEXT, record_json TEXT);",
        )
        .unwrap();
        c
    }

    #[test]
    fn collect_findings_skips_info_and_orders_by_severity() {
        let c = findings_db();
        for (kind, sev) in [
            ("reflected_xss", "low"),
            ("sql_injection", "critical"),
            ("note", "info"),
            ("broken_access", "high"),
            ("misc", ""),
        ] {
            c.execute(
                "INSERT INTO sentinel_findings(scan_id,stage,kind,record_key,severity,record_json) \
                 VALUES('s','attack',?1,?1,?2,'{}')",
                params![kind, sev],
            )
            .unwrap();
        }
        let found = deep_dive_collect_findings(&c, "s");
        let kinds: Vec<&str> = found.iter().map(|f| f.kind.as_str()).collect();
        // info and empty-severity are excluded; critical before high before low
        assert_eq!(kinds, vec!["sql_injection", "broken_access", "reflected_xss"]);
    }

    #[test]
    fn resolve_maps_finding_to_point_by_kind() {
        let point_q = InjectionPoint {
            method: "GET".into(),
            url: "http://t/s?q=1".into(),
            body_field: None,
            base_body: None,
            query_param: Some("q".into()),
            label: "GET /s?q".into(),
        };
        let point_ma = InjectionPoint {
            method: "POST".into(),
            url: "http://t/users".into(),
            body_field: Some("role".into()),
            base_body: Some(serde_json::json!({"role": "x"})),
            query_param: None,
            label: "POST /users body.role".into(),
        };
        let mut by_label = HashMap::new();
        by_label.insert(point_q.label.clone(), &point_q);
        let mut by_url = HashMap::new();
        by_url.insert(point_ma.url.clone(), &point_ma);
        let mut by_path = HashMap::new();
        by_path.insert("/admin".to_string(), "http://t/admin".to_string());

        let sqli = DeepDiveFinding { finding_id: 1, kind: "sql_injection".into(), severity: "critical".into(), record_key: "GET /s?q".into(), record: JsonValue::Null };
        let (p, u) = deep_dive_resolve(&sqli, &by_label, &by_url, &by_path);
        assert!(p.is_some() && u.is_none());

        let ma = DeepDiveFinding { finding_id: 2, kind: "mass_assignment".into(), severity: "high".into(), record_key: "http://t/users".into(), record: JsonValue::Null };
        let (p, _) = deep_dive_resolve(&ma, &by_label, &by_url, &by_path);
        assert!(p.is_some(), "mass_assignment resolves by url");

        let acc = DeepDiveFinding { finding_id: 3, kind: "broken_access".into(), severity: "high".into(), record_key: "/admin".into(), record: JsonValue::Null };
        let (p, u) = deep_dive_resolve(&acc, &by_label, &by_url, &by_path);
        assert!(p.is_none() && u == Some("http://t/admin"));
    }
}
