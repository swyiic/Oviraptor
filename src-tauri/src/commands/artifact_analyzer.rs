// Static analysis of operator-provided DECOMPILED artifacts (WeChat mini-program
// JS, APK dex/jadx source, uni-app packages). The operator unpacks; this only
// SCANS the extracted source for the leak classes that show up in real reports:
// hardcoded secrets (HMAC/sign keys, JWT secrets, generic API keys, Aliyun OSS
// keys, Ezviz appKey/appSecret), hardcoded JWTs, and endpoints/URLs. It is
// read-only over the provided directory, deterministic, and never unpacks or
// fetches anything. Values are redacted in findings.

#[derive(Clone, Debug, PartialEq)]
struct ArtifactFinding {
    kind: String,
    severity: String,
    title: String,
    location: String,
    evidence: String,
}

// (identifier substring, kind, severity, title). Matched case-insensitively as a
// substring of an identifier on a line that also assigns a quoted value.
const SECRET_RULES: &[(&str, &str, &str, &str)] = &[
    ("accesskeysecret", "secret_leak", "high", "硬编码 Aliyun AccessKeySecret"),
    ("accesskeyid", "secret_leak", "medium", "硬编码 Aliyun AccessKeyId"),
    ("appsecret", "secret_leak", "high", "硬编码 appSecret"),
    ("app_secret", "secret_leak", "high", "硬编码 appSecret"),
    ("secretkey", "secret_leak", "high", "硬编码 secretKey"),
    ("secret_key", "secret_leak", "high", "硬编码 secret_key"),
    ("hmac", "secret_leak", "high", "硬编码 HMAC 签名密钥"),
    ("signkey", "secret_leak", "high", "硬编码签名密钥"),
    ("sign_key", "secret_leak", "high", "硬编码签名密钥"),
    ("jwt", "jwt_secret", "high", "疑似硬编码 JWT 密钥"),
    ("appkey", "secret_leak", "medium", "硬编码 appKey"),
    ("apikey", "secret_leak", "medium", "硬编码 API Key"),
    ("api_key", "secret_leak", "medium", "硬编码 API Key"),
    ("privatekey", "secret_leak", "high", "硬编码私钥"),
    ("private_key", "secret_leak", "high", "硬编码私钥"),
];

// Values that are obviously not real secrets.
fn is_placeholder_value(value: &str) -> bool {
    let v = value.trim().to_ascii_lowercase();
    v.is_empty()
        || v.len() < 6
        || v.len() > 512
        || v.starts_with("${")
        || v.starts_with("process.env")
        || v.starts_with("import.meta")
        || matches!(v.as_str(), "null" | "undefined" | "true" | "false" | "changeme" | "your_key" | "xxxxxx")
        || v.chars().all(|c| c == 'x' || c == '*' || c == '0')
}

// Redact a secret for evidence: keep only shape, never the full value.
fn redact_secret(value: &str) -> String {
    let n = value.chars().count();
    if n <= 4 {
        return format!("<redacted len={n}>");
    }
    let head: String = value.chars().take(2).collect();
    let tail: String = value.chars().rev().take(2).collect::<String>().chars().rev().collect();
    format!("{head}…{tail} <len={n}>")
}

// Extract the first quoted value on a line at/after `from`.
fn quoted_value_after(line: &str, from: usize) -> Option<String> {
    let bytes = line.as_bytes();
    let mut i = from;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'"' || c == b'\'' || c == b'`' {
            let quote = c;
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && bytes[j] != quote {
                j += 1;
            }
            if j <= bytes.len() && j > start {
                return line.get(start..j).map(str::to_string);
            }
            return None;
        }
        i += 1;
    }
    None
}

/// Scan one line for hardcoded secrets and hardcoded JWTs. Pure.
fn scan_line(line: &str) -> Vec<(String, String, String, String)> {
    // (kind, severity, title, redacted_evidence)
    let mut out = Vec::new();
    let lower = line.to_ascii_lowercase();
    for (needle, kind, severity, title) in SECRET_RULES {
        if let Some(pos) = lower.find(needle) {
            if let Some(value) = quoted_value_after(line, pos + needle.len()) {
                if !is_placeholder_value(&value) {
                    out.push((
                        (*kind).to_string(),
                        (*severity).to_string(),
                        (*title).to_string(),
                        redact_secret(&value),
                    ));
                }
            }
        }
    }
    // Cloud access-key ID literals (AWS AKIA…, Aliyun LTAI…, Tencent AKID…).
    for (prefix, provider) in [("AKIA", "AWS"), ("LTAI", "Aliyun"), ("AKID", "Tencent")] {
        if let Some(pos) = line.find(prefix) {
            let token: String = line[pos..].chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
            if token.len() >= prefix.len() + 12 {
                out.push((
                    "cloud_key_leak".into(),
                    "high".into(),
                    format!("硬编码 {provider} AccessKey"),
                    redact_secret(&token),
                ));
            }
        }
    }
    // Embedded private key (PEM).
    if line.contains("-----BEGIN") && line.contains("PRIVATE KEY") {
        out.push(("private_key_leak".into(), "high".into(), "源码中内嵌私钥（PEM）".into(), "<PEM private key>".into()));
    }
    // Credentialed connection string (scheme://user:pass@host or jdbc:...:pass@).
    let lower_line = line.to_ascii_lowercase();
    for scheme in ["mysql://", "postgresql://", "postgres://", "mongodb://", "mongodb+srv://", "redis://", "amqp://", "jdbc:"] {
        if let Some(pos) = lower_line.find(scheme) {
            let rest: String = line[pos..].chars().take(256).collect();
            if rest.contains('@') && rest[scheme.len().min(rest.len())..].contains(':') {
                out.push(("conn_string_leak".into(), "high".into(), "硬编码带凭证的连接串".into(), redact_secret(&rest)));
                break;
            }
        }
    }
    // Hardcoded JWT literal (header.payload.signature, base64url starting eyJ).
    if let Some(pos) = line.find("eyJ") {
        let token: String = line[pos..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
            .collect();
        if token.matches('.').count() == 2 && token.len() > 30 {
            out.push((
                "hardcoded_jwt".into(),
                "medium".into(),
                "源码中硬编码 JWT".into(),
                redact_secret(&token),
            ));
        }
    }
    // JWT alg=none (forgeable tokens).
    let compact = lower.replace([' ', '\'', '"'], "");
    if compact.contains("alg:none") || compact.contains("algorithm:none") {
        out.push(("weak_jwt_alg".into(), "high".into(), "JWT 使用 alg=none（可伪造令牌）".into(), "alg=none".into()));
    }
    // Client-side inline SQL — raw SQL built/sent from the client is a likely
    // injection surface (the "client puts raw SQL in request params" class).
    let sqlish = (lower.contains("select") && lower.contains(" from "))
        || lower.contains("insert into")
        || (lower.contains("update ") && lower.contains(" set "))
        || lower.contains("delete from");
    if sqlish && (line.contains('"') || line.contains('\'')) {
        out.push((
            "client_side_sql".into(),
            "medium".into(),
            "客户端内联 SQL（潜在注入面）".into(),
            line.trim().chars().take(120).collect::<String>(),
        ));
    }
    out
}

/// Extract http(s) endpoints from a line.
fn scan_endpoints(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    for scheme in ["https://", "http://"] {
        let mut search_from = 0;
        while let Some(rel) = line[search_from..].find(scheme) {
            let start = search_from + rel;
            let url: String = line[start..]
                .chars()
                .take_while(|c| !c.is_whitespace() && !matches!(c, '"' | '\'' | '`' | '<' | '>' | ')' | '(' | '\\'))
                .collect();
            if url.len() > scheme.len() + 3 {
                out.push(url.clone());
            }
            search_from = start + scheme.len();
        }
    }
    out
}

const MAX_ARTIFACT_FILES: usize = 20_000;
const MAX_ARTIFACT_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_ARTIFACT_DEPTH: usize = 24;

fn looks_textual(sample: &[u8]) -> bool {
    !sample.iter().take(4096).any(|b| *b == 0)
}

fn analyze_artifact_dir(root: &Path) -> Result<(Vec<ArtifactFinding>, Vec<String>), String> {
    if !root.is_dir() {
        return Err("提供的路径不是目录".into());
    }
    let mut findings: Vec<ArtifactFinding> = Vec::new();
    let mut endpoints: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut files_seen = 0usize;
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > MAX_ARTIFACT_DEPTH || files_seen >= MAX_ARTIFACT_FILES {
            continue;
        }
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            if files_seen >= MAX_ARTIFACT_FILES {
                break;
            }
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push((path, depth + 1));
                continue;
            }
            if !meta.is_file() || meta.len() > MAX_ARTIFACT_FILE_BYTES {
                continue;
            }
            let Ok(bytes) = fs::read(&path) else { continue };
            if !looks_textual(&bytes) {
                continue;
            }
            files_seen += 1;
            let text = String::from_utf8_lossy(&bytes);
            let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().into_owned();
            for (lineno, line) in text.lines().enumerate() {
                if line.len() > 8192 {
                    continue;
                }
                for (kind, severity, title, evidence) in scan_line(line) {
                    findings.push(ArtifactFinding {
                        kind,
                        severity,
                        title,
                        location: format!("{rel}:{}", lineno + 1),
                        evidence,
                    });
                }
                for endpoint in scan_endpoints(line) {
                    endpoints.insert(endpoint);
                }
            }
        }
    }
    Ok((findings, endpoints.into_iter().take(200).collect()))
}

fn write_artifact_findings(
    connection: &rusqlite::Connection,
    scan_id: &str,
    findings: &[ArtifactFinding],
    endpoints: &[String],
) -> Result<usize, String> {
    let mut written = 0usize;
    let insert = |kind: &str, record_key: &str, title: &str, severity: &str, record: JsonValue| -> Result<(), String> {
        connection
            .execute(
                "INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title,severity,record_json) \
                 VALUES(?1,'artifact','artifact',?2,?3,?4,?5,?6) \
                 ON CONFLICT(scan_id,target_url,stage,kind,record_key) DO UPDATE SET \
                 title=excluded.title,severity=excluded.severity,record_json=excluded.record_json,updated_at=datetime('now','localtime')",
                params![scan_id, kind, record_key, title, severity, record.to_string()],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    };
    for f in findings {
        insert(
            &f.kind,
            &format!("{}:{}", f.kind, f.location),
            &f.title,
            &f.severity,
            serde_json::json!({"location": f.location, "evidence": f.evidence, "source": "decompiled_artifact"}),
        )?;
        written += 1;
    }
    if !endpoints.is_empty() {
        insert(
            "endpoints_discovered",
            "artifact:endpoints",
            &format!("反编译产物中发现 {} 个接口/URL", endpoints.len()),
            "info",
            serde_json::json!({"endpoints": endpoints, "source": "decompiled_artifact"}),
        )?;
        written += 1;
    }
    Ok(written)
}

/// Operator command: scan a provided decompiled artifact directory for hardcoded
/// secrets / JWTs / endpoints and record findings against the scan. Read-only on
/// the artifact; writes only findings. Returns the number of findings written.
#[tauri::command]
pub fn analyze_decompiled_artifact(
    state: State<AppState>,
    scan_id: String,
    artifact_dir: String,
) -> Result<usize, String> {
    if scan_id.trim().is_empty() || scan_id.contains('/') || scan_id.contains("..") {
        return Err("任务 ID 非法".into());
    }
    let connection = db::open(&state.db_path)?;
    let exists: bool = connection
        .query_row("SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1)", [&scan_id], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if !exists {
        return Err("任务不存在".into());
    }
    let (findings, endpoints) = analyze_artifact_dir(Path::new(&artifact_dir))?;
    write_artifact_findings(&connection, &scan_id, &findings, &endpoints)
}

#[cfg(test)]
mod artifact_analyzer_tests {
    use super::{scan_endpoints, scan_line};

    #[test]
    fn detects_aliyun_oss_keys() {
        let hits = scan_line(r#"const accessKeySecret = "wJalrXUtnFEMIK7MDENGbPxRfiCY";"#);
        assert!(hits.iter().any(|(k, s, _, _)| k == "secret_leak" && s == "high"), "{hits:?}");
    }

    #[test]
    fn detects_hmac_and_jwt_secret() {
        assert!(scan_line(r#"var hmacKey = 'a1b2c3d4e5f6g7';"#).iter().any(|(_, _, t, _)| t.contains("HMAC")));
        assert!(scan_line(r#"jwtSecret: "supersecretvalue123""#).iter().any(|(k, _, _, _)| k == "jwt_secret"));
    }

    #[test]
    fn redacts_the_value_never_leaks_it() {
        let hits = scan_line(r#"appSecret="abcdefghijklmnop""#);
        let ev = &hits[0].3;
        assert!(!ev.contains("abcdefghijklmnop"), "evidence must be redacted: {ev}");
        assert!(ev.contains("len="));
    }

    #[test]
    fn placeholders_are_ignored() {
        assert!(scan_line(r#"const appSecret = process.env.APP_SECRET;"#).is_empty());
        assert!(scan_line(r#"apiKey: """#).is_empty());
        assert!(scan_line(r#"secretKey = "xxxxxx""#).is_empty());
    }

    #[test]
    fn detects_hardcoded_jwt_literal() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJ1aWQiOjF9.3n8xQwQ2Zk9abcDEFghijkLMNopqrstuv";
        assert!(scan_line(&format!("token = '{jwt}'")).iter().any(|(k, _, _, _)| k == "hardcoded_jwt"));
    }

    #[test]
    fn detects_cloud_keys_private_key_and_conn_strings() {
        assert!(scan_line(r#"accessKeyId = "AKIAIOSFODNN7EXAMPLE""#).iter().any(|(k, _, _, _)| k == "cloud_key_leak" || k == "secret_leak"));
        assert!(scan_line(r#"ak="LTAI5tABCDEFGHIJKLMNopqr""#).iter().any(|(k, _, t, _)| k == "cloud_key_leak" && t.contains("Aliyun")));
        assert!(scan_line("-----BEGIN RSA PRIVATE KEY-----").iter().any(|(k, _, _, _)| k == "private_key_leak"));
        assert!(scan_line(r#"DB="mysql://root:p@ssw0rd@10.0.0.5:3306/app""#).iter().any(|(k, _, _, _)| k == "conn_string_leak"));
    }

    #[test]
    fn detects_weak_jwt_alg_and_client_side_sql() {
        assert!(scan_line(r#"const header = {"alg":"none","typ":"JWT"}"#).iter().any(|(k, _, _, _)| k == "weak_jwt_alg"));
        assert!(scan_line(r#"var q = "select * from users where id="+uid; post('/api', q)"#).iter().any(|(k, _, _, _)| k == "client_side_sql"));
        assert!(scan_line(r#"body = 'delete from t where id=1'"#).iter().any(|(k, _, _, _)| k == "client_side_sql"));
        // A plain SELECT with no string literal is not flagged (reduces ORM noise).
        assert!(scan_line("db.select().from(users)").iter().all(|(k, _, _, _)| k != "client_side_sql"));
    }

    #[test]
    fn extracts_endpoints() {
        let got = scan_endpoints(r#"fetch("https://wxapi.jsxysx.com/x/login"); u='http://58.213.48.106/arcgis'"#);
        assert!(got.iter().any(|u| u.contains("wxapi.jsxysx.com")));
        assert!(got.iter().any(|u| u.contains("58.213.48.106")));
    }
}
