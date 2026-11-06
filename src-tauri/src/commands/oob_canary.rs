// C3 — DNSlog / OOB out-of-band canary (设计稿 §4b). The operator configures their
// OWN DNSlog / collaborator base domain (bound to the scan). The agent mints a
// unique per-probe SUBDOMAIN to embed in a blind probe (SSRF / blind RCE / JNDI),
// and may encode a TINY command output into a label as proof-of-exfil. This is the
// `exfil` high-risk class in CANARY form: tiny output only, never batch exfil of
// real data.
//
// Minting SENDS nothing — it only generates a name + records it. The OOB-triggering
// request still goes through the existing gated probes (probe_command_execution /
// probe_sql_injection / run_target_poc). Detecting the DNS callback is operator-side
// (their DNSlog service); the operator feeds a confirmed hit back via
// `record_oob_canary_callback`, which becomes the confirmed finding. Live polling of
// a provider's DNSlog API is a follow-up (provider-specific, needs operator creds).
//
// Floors: minting requires the `exfil` class authorized (mandatory switch + legal
// attestation); the base domain is operator-set and never internal/loopback; the
// encoded output is hard-capped (tiny), base32, DNS-label-safe — no raw-data egress.

const OOB_CANARY_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS oob_canary_domains (
    scan_id TEXT PRIMARY KEY,
    attempt_number INTEGER NOT NULL,
    base_domain TEXT NOT NULL,
    provider_label TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE TABLE IF NOT EXISTS oob_canary_probes (
    id TEXT PRIMARY KEY,
    scan_id TEXT NOT NULL,
    attempt_number INTEGER NOT NULL,
    subdomain TEXT NOT NULL,
    label TEXT NOT NULL DEFAULT '',
    encoded_output TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    callback_seen_at TEXT NOT NULL DEFAULT '',
    UNIQUE(scan_id, subdomain)
);
"#;

// Tiny canary only. 24 raw bytes is enough for a `whoami` / short token and keeps
// the encoded label well under the 63-char DNS label limit; anything larger is a
// batch-exfil attempt and is refused.
const OOB_MAX_OUTPUT_BYTES: usize = 24;

/// A plausible operator DNSlog/collaborator base domain: a dotted DNS name, not a
/// URL, not an internal/loopback literal. (The operator owns this domain; we only
/// sanity-check its shape so a probe-page can never smuggle a scheme/path/IP here.)
fn valid_oob_base_domain(domain: &str) -> bool {
    let d = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    if d.is_empty() || d.len() > 253 || d.contains("://") || d.contains('/') || !d.contains('.') {
        return false;
    }
    if d == "localhost" || host_is_internal_literal(&d) {
        return false;
    }
    d.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            && !label.starts_with('-')
            && !label.ends_with('-')
    })
}

/// RFC4648 base32 (lowercase, no padding) → a DNS-label-safe encoding of a tiny
/// output. No external crate; input is hard-capped by the caller.
fn oob_base32_label(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut out = String::new();
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for &byte in bytes {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buffer >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    out
}

fn oob_scan_exists(connection: &rusqlite::Connection, scan_id: &str) -> Result<bool, String> {
    connection
        .query_row("SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1)", [scan_id], |r| r.get(0))
        .map_err(|e| e.to_string())
}

fn oob_configured_domain(connection: &rusqlite::Connection, scan_id: &str) -> Option<String> {
    connection
        .query_row(
            "SELECT base_domain FROM oob_canary_domains WHERE scan_id=?1",
            [scan_id],
            |r| r.get::<_, String>(0),
        )
        .ok()
}

/// Mint one OOB canary subdomain. Pure name generation + audit — sends nothing.
/// Fail-closed: the `exfil` class must be authorized AND a base domain configured.
fn mint_oob_canary_in(
    connection: &rusqlite::Connection,
    scan_id: &str,
    attempt_number: i64,
    label: &str,
    tiny_output: Option<&str>,
) -> Result<JsonValue, String> {
    if !scan_capability_class_authorized(connection, scan_id, attempt_number, "exfil").unwrap_or(false) {
        return Err("铸造 OOB canary 需先为该任务授权 exfil 能力并完成合法授权声明（强制开关）".into());
    }
    let base = oob_configured_domain(connection, scan_id)
        .ok_or("未配置 OOB 域名：先用 set_oob_canary_domain 配置 operator 自有的 DNSlog/collaborator 域名")?;
    let encoded = match tiny_output.map(str::as_bytes).filter(|b| !b.is_empty()) {
        Some(bytes) => {
            if bytes.len() > OOB_MAX_OUTPUT_BYTES {
                return Err(format!(
                    "canary 输出超限（仅允许极小输出 ≤{OOB_MAX_OUTPUT_BYTES} 字节证明可外带，非批量外带真实数据）"
                ));
            }
            oob_base32_label(bytes)
        }
        None => String::new(),
    };
    let token = Uuid::new_v4().simple().to_string()[..16].to_string();
    let subdomain = if encoded.is_empty() {
        format!("{token}.{base}")
    } else {
        format!("{encoded}.{token}.{base}")
    };
    connection
        .execute_batch(OOB_CANARY_SCHEMA)
        .map_err(|_| "oob_canary_storage_unavailable")?;
    connection
        .execute(
            "INSERT INTO oob_canary_probes(id,scan_id,attempt_number,subdomain,label,encoded_output) \
             VALUES(?1,?2,?3,?4,?5,?6)",
            params![Uuid::new_v4().to_string(), scan_id, attempt_number, subdomain, label, encoded],
        )
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "subdomain": subdomain,
        "token": token,
        "encodedOutput": encoded,
        "baseDomain": base,
        "note": "把该子域埋进已授权的盲洞探测（SSRF/盲 RCE/JNDI）；回连检测在 operator 的 DNSlog 侧，确认命中后用 record_oob_canary_callback 回填。"
    }))
}

/// Operator: configure the DNSlog / collaborator base domain for a scan (their own
/// domain). Validated to a dotted DNS name, never internal/loopback.
#[tauri::command]
pub fn set_oob_canary_domain(
    state: State<AppState>,
    scan_id: String,
    base_domain: String,
    provider_label: Option<String>,
) -> Result<JsonValue, String> {
    if scan_id.trim().is_empty() || scan_id.contains('/') || scan_id.contains("..") {
        return Err("任务 ID 非法".into());
    }
    if !valid_oob_base_domain(&base_domain) {
        return Err("OOB 域名无效（需是 operator 自有的 DNSlog/collaborator 域名，非 URL、非内网/环回）".into());
    }
    let connection = db::open(&state.db_path)?;
    if !oob_scan_exists(&connection, &scan_id)? {
        return Err("任务不存在".into());
    }
    let attempt = scan_attempt_number(&connection, &scan_id);
    let domain = base_domain.trim().trim_end_matches('.').to_ascii_lowercase();
    let provider = provider_label.unwrap_or_default().chars().take(40).collect::<String>();
    connection
        .execute_batch(OOB_CANARY_SCHEMA)
        .map_err(|_| "oob_canary_storage_unavailable")?;
    connection
        .execute(
            "INSERT INTO oob_canary_domains(scan_id,attempt_number,base_domain,provider_label) VALUES(?1,?2,?3,?4) \
             ON CONFLICT(scan_id) DO UPDATE SET attempt_number=excluded.attempt_number,base_domain=excluded.base_domain,provider_label=excluded.provider_label,created_at=datetime('now','localtime')",
            params![scan_id, attempt, domain, provider],
        )
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({"scanId": scan_id, "baseDomain": domain, "attemptNumber": attempt, "provider": provider}))
}

/// Agent/operator: mint one canary subdomain (gated on `exfil`). `output_text` is a
/// TINY proof-of-exfil string (e.g. a `whoami`); omit for a pure blind-callback marker.
#[tauri::command]
pub fn mint_oob_canary(
    state: State<AppState>,
    scan_id: String,
    label: Option<String>,
    output_text: Option<String>,
) -> Result<JsonValue, String> {
    if scan_id.trim().is_empty() || scan_id.contains('/') || scan_id.contains("..") {
        return Err("任务 ID 非法".into());
    }
    let connection = db::open(&state.db_path)?;
    if !oob_scan_exists(&connection, &scan_id)? {
        return Err("任务不存在".into());
    }
    let attempt = scan_attempt_number(&connection, &scan_id);
    let label = label.unwrap_or_default().chars().take(60).collect::<String>();
    mint_oob_canary_in(&connection, &scan_id, attempt, &label, output_text.as_deref())
}

/// Operator: record that a minted subdomain got a DNS callback (seen in THEIR DNSlog).
/// Marks the probe and writes a confirmed `oob_canary` finding (blind OOB proven).
#[tauri::command]
pub fn record_oob_canary_callback(
    state: State<AppState>,
    scan_id: String,
    subdomain: String,
) -> Result<JsonValue, String> {
    if scan_id.trim().is_empty() || scan_id.contains('/') || scan_id.contains("..") {
        return Err("任务 ID 非法".into());
    }
    let connection = db::open(&state.db_path)?;
    connection
        .execute_batch(OOB_CANARY_SCHEMA)
        .map_err(|_| "oob_canary_storage_unavailable")?;
    let changed = connection
        .execute(
            "UPDATE oob_canary_probes SET callback_seen_at=datetime('now','localtime') \
             WHERE scan_id=?1 AND subdomain=?2 AND callback_seen_at=''",
            params![scan_id, subdomain],
        )
        .map_err(|e| e.to_string())?;
    if changed == 1 {
        let encoded: String = connection
            .query_row(
                "SELECT encoded_output FROM oob_canary_probes WHERE scan_id=?1 AND subdomain=?2",
                params![scan_id, subdomain],
                |r| r.get(0),
            )
            .unwrap_or_default();
        let record = serde_json::json!({
            "subdomain": subdomain,
            "encodedOutput": encoded,
            "cwe": "CWE-918/CWE-78",
            "detail": "带外(OOB)回连已在 operator DNSlog 侧确认——盲漏洞得证(canary 形态，仅极小输出)。"
        });
        let _ = connection.execute(
            "INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title,severity,record_json) \
             VALUES(?1,?2,'exploitation','oob_canary',?3,?4,'high',?5) \
             ON CONFLICT(scan_id,target_url,stage,kind,record_key) DO UPDATE SET \
             title=excluded.title,severity=excluded.severity,record_json=excluded.record_json,updated_at=datetime('now','localtime')",
            params![scan_id, subdomain, subdomain, "带外回连确认(OOB canary)", record.to_string()],
        );
    }
    Ok(serde_json::json!({"recorded": changed == 1, "subdomain": subdomain}))
}

#[cfg(test)]
mod oob_canary_tests {
    use super::*;
    use rusqlite::Connection;

    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE sentinel_scans(id TEXT PRIMARY KEY, attempt_count INTEGER NOT NULL DEFAULT 1);
             INSERT INTO sentinel_scans(id,attempt_count) VALUES('s',1);
             CREATE TABLE scan_capability_authorizations(scan_id TEXT NOT NULL, attempt_number INTEGER NOT NULL, legal_attestation INTEGER NOT NULL, authorized_classes TEXT NOT NULL, PRIMARY KEY(scan_id,attempt_number));",
        )
        .unwrap();
        c.execute_batch(OOB_CANARY_SCHEMA).unwrap();
        c
    }
    fn authorize_exfil(c: &Connection) {
        c.execute(
            "INSERT INTO scan_capability_authorizations(scan_id,attempt_number,legal_attestation,authorized_classes) VALUES('s',1,1,'[\"exfil\"]')",
            [],
        )
        .unwrap();
    }
    fn set_domain(c: &Connection) {
        c.execute(
            "INSERT INTO oob_canary_domains(scan_id,attempt_number,base_domain) VALUES('s',1,'abc.dnslog.example')",
            [],
        )
        .unwrap();
    }

    #[test]
    fn base_domain_validation_rejects_urls_ips_loopback() {
        assert!(valid_oob_base_domain("x.dnslog.example"));
        assert!(!valid_oob_base_domain("http://x.dnslog.example"));
        assert!(!valid_oob_base_domain("x.dnslog.example/path"));
        assert!(!valid_oob_base_domain("localhost"));
        assert!(!valid_oob_base_domain("127.0.0.1"));
        assert!(!valid_oob_base_domain("nodot"));
    }

    #[test]
    fn mint_fails_closed_without_authorization() {
        let c = db();
        set_domain(&c); // domain present, but exfil NOT authorized
        assert!(mint_oob_canary_in(&c, "s", 1, "probe", None).is_err());
    }

    #[test]
    fn mint_fails_closed_without_configured_domain() {
        let c = db();
        authorize_exfil(&c); // authorized, but no domain configured
        assert!(mint_oob_canary_in(&c, "s", 1, "probe", None).is_err());
    }

    #[test]
    fn mint_under_base_and_recorded_when_authorized_and_configured() {
        let c = db();
        authorize_exfil(&c);
        set_domain(&c);
        let out = mint_oob_canary_in(&c, "s", 1, "ssrf", None).unwrap();
        let sub = out["subdomain"].as_str().unwrap();
        assert!(sub.ends_with(".abc.dnslog.example"), "{sub}");
        let count: i64 = c
            .query_row("SELECT COUNT(*) FROM oob_canary_probes WHERE scan_id='s' AND subdomain=?1", [sub], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn tiny_output_is_encoded_but_oversize_is_refused() {
        let c = db();
        authorize_exfil(&c);
        set_domain(&c);
        let ok = mint_oob_canary_in(&c, "s", 1, "rce", Some("root")).unwrap();
        assert!(!ok["encodedOutput"].as_str().unwrap().is_empty());
        // Over the tiny cap → refused (never batch exfil).
        let big = "x".repeat(OOB_MAX_OUTPUT_BYTES + 1);
        assert!(mint_oob_canary_in(&c, "s", 1, "rce", Some(&big)).is_err());
    }
}
