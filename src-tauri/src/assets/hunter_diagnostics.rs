//! Hunter diagnostics only. Never log credential-bearing URLs, personal_info or asset bodies.
use super::*;
use std::{
    error::Error,
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::Mutex,
    time::Instant,
};

static WRITER: Mutex<()> = Mutex::new(());
const LIMIT: u64 = 8 * 1024 * 1024;

pub(super) struct Context {
    file: Option<PathBuf>,
    project: Option<i64>,
    query: Option<String>,
    mode: String,
}
impl Context {
    pub(super) fn new(
        database: &Path,
        mode: String,
        project: Option<i64>,
        query: Option<String>,
    ) -> Self {
        Self {
            file: database
                .parent()
                .map(|p| p.join("diagnostics/hunter.jsonl")),
            project,
            query,
            mode,
        }
    }
    pub(super) fn event(&self, key: &str, phase: &str, details: Value) -> Result<(), String> {
        let Some(file) = &self.file else {
            return Ok(());
        };
        let _guard = WRITER.lock().map_err(|_| "Hunter 诊断日志锁不可用")?;
        let parent = file.parent().ok_or("Hunter 诊断目录无效")?;
        fs::create_dir_all(parent).map_err(|_| "无法创建 Hunter 诊断目录；请求未自动重试")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
                .map_err(|_| "无法保护 Hunter 诊断目录")?;
        }
        if file.metadata().map(|m| m.len() >= LIMIT).unwrap_or(false) {
            let previous = file.with_extension("previous.jsonl");
            if previous.exists() {
                fs::remove_file(&previous).map_err(|_| "无法轮转 Hunter 诊断日志")?;
            }
            fs::rename(file, previous).map_err(|_| "无法轮转 Hunter 诊断日志")?;
        }
        let entry = json!({"time":chrono::Utc::now().to_rfc3339(),"phase":phase,"projectId":self.project,"queryId":self.query,"networkMode":self.mode,"automaticRetries":0,"details":details});
        // Sanitize the final serialization too: even a hostile error response cannot echo the key.
        let line = scrub(entry, key).to_string();
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut writer = options.open(file).map_err(|_| "无法写入 Hunter 诊断日志")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            writer
                .set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|_| "无法保护 Hunter 诊断日志")?;
        }
        writeln!(writer, "{line}")
            .and_then(|_| writer.sync_data())
            .map_err(|_| "Hunter 诊断日志持久化失败".to_string())
    }
}
fn scrub(value: Value, key: &str) -> Value {
    match value {
        Value::String(s) => json!(safe(&s, key, 8000)),
        Value::Array(a) => Value::Array(a.into_iter().map(|v| scrub(v, key)).collect()),
        Value::Object(o) => Value::Object(o.into_iter().map(|(k, v)| (k, scrub(v, key))).collect()),
        v => v,
    }
}

fn safe(raw: &str, key: &str, limit: usize) -> String {
    let mut s = raw.to_string();
    if !key.is_empty() {
        for secret in [
            key.to_string(),
            base64::engine::general_purpose::STANDARD.encode(key),
            base64::engine::general_purpose::URL_SAFE.encode(key),
            url_key(key),
        ] {
            s = s.replace(&secret, "[REDACTED]");
        }
    }
    for pattern in [
        r"(?i)[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}",
        r"\b1[3-9][0-9]{9}\b",
        r#"(?i)(?:api[-_]?key|access[-_]?token|authorization|password)\s*[=:]\s*[^\s&,;\"\\]+"#,
    ] {
        s = regex::Regex::new(pattern)
            .unwrap()
            .replace_all(&s, "[REDACTED]")
            .into_owned();
    }
    s.chars().filter(|c| !c.is_control()).take(limit).collect()
}
fn url_key(key: &str) -> String {
    reqwest::Url::parse_with_params("https://redacted.invalid", [("key", key)])
        .unwrap()
        .query()
        .unwrap_or("")
        .trim_start_matches("key=")
        .to_string()
}
pub(super) fn message(raw: &str, key: &str) -> String {
    safe(raw, key, 1000)
}

fn provider_summary(value: &Value, key: &str) -> Value {
    let d = &value["data"];
    json!({"code":value["code"].as_i64(),"message":value["message"].as_str().map(|s|safe(s,key,1000)),"syntaxPrompt":d["syntax_prompt"].as_str().map(|s|safe(s,key,1000)),"total":d["total"].as_u64(),"returned":d["arr"].as_array().map(Vec::len),"consumeQuota":d["consume_quota"].as_str().map(|s|safe(s,key,160)),"restQuota":d["rest_quota"].as_str().map(|s|safe(s,key,160)),"restFreePoint":d["rest_free_point"].as_i64(),"restEquityPoint":d["rest_equity_point"].as_i64(),"restExportQuota":d["rest_export_quota"].as_i64()})
}

pub(super) fn get_json(
    client: &reqwest::blocking::Client,
    url: reqwest::Url,
    key: &str,
    context: &Context,
) -> Result<Value, String> {
    let request = uuid::Uuid::new_v4().to_string();
    let mut meta = json!({"requestId":request,"method":"GET","endpoint":format!("{}://{}{}",url.scheme(),url.host_str().unwrap_or(""),url.path())});
    for (name, value) in url.query_pairs() {
        if ["page", "page_size", "start_time", "end_time", "is_web"].contains(&name.as_ref()) {
            meta[&*name] = json!(safe(&value, key, 100));
        }
        if name == "search" {
            meta["query"] = base64::engine::general_purpose::URL_SAFE
                .decode(value.as_bytes())
                .ok()
                .and_then(|b| String::from_utf8(b).ok())
                .map(|s| json!(safe(&s, key, 2000)))
                .unwrap_or(json!("[invalid base64url]"));
        }
    }
    context.event(key, "request_started", meta.clone())?; // Do not send a paid request without a durable start record.
    let started = Instant::now();
    let result = (|| {
        let mut response = client.get(url).send().map_err(|e| {
            let category = if e.is_timeout() {
                "timeout"
            } else if e.is_connect() {
                "connect_dns_tls_or_proxy"
            } else if e.is_request() {
                "request"
            } else {
                "transport"
            };
            let e = e.without_url();
            let mut chain = vec![safe(&e.to_string(), key, 1000)];
            let mut source = e.source();
            for _ in 0..8 {
                let Some(s) = source else { break };
                chain.push(safe(&s.to_string(), key, 1000));
                source = s.source();
            }
            meta["transportCategory"] = json!(category);
            meta["errorChain"] = json!(chain);
            format!("Hunter 连接失败（{category}）：{}", chain.join(" → "))
        })?;
        let status = response.status();
        meta["httpStatus"] = json!(status.as_u16());
        let mut headers = serde_json::Map::new();
        for name in [
            "content-type",
            "server",
            "x-request-id",
            "x-correlation-id",
            "cf-ray",
            "retry-after",
        ] {
            if let Some(v) = response.headers().get(name).and_then(|v| v.to_str().ok()) {
                headers.insert(name.into(), json!(safe(v, key, 200)));
            }
        }
        meta["responseHeaders"] = json!(headers);
        let mut bytes = Vec::new();
        (&mut response)
            .take(2 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| {
                meta["bodyReadError"] = json!(safe(&e.to_string(), key, 400));
                "Hunter 响应读取失败，远端结果与扣费未知".to_string()
            })?;
        meta["bodyBytesRead"] = json!(bytes.len());
        meta["bodySha256"] = json!(format!("{:x}", Sha256::digest(&bytes)));
        if bytes.len() > 2 * 1024 * 1024 {
            return Err("Hunter 响应超过 2 MiB；未采纳结果".into());
        }
        let value = serde_json::from_slice::<Value>(&bytes);
        if let Ok(v) = &value {
            meta["provider"] = provider_summary(v, key);
        } else {
            meta["jsonError"] = json!(value.as_ref().unwrap_err().to_string());
            // Never keep a raw HTML page, which may contain cookies, tokens or personal details.
            meta["nonJsonBody"] = json!(true);
        }
        if !status.is_success() {
            return Err(format!(
                "Hunter 返回 HTTP {}{}{}",
                status.as_u16(),
                meta["provider"]["message"]
                    .as_str()
                    .map(|s| format!("：{s}"))
                    .unwrap_or_default(),
                if status.as_u16() == 403 {
                    "；访问被拒绝，不能据此判断 Key 错误"
                } else {
                    ""
                }
            ));
        }
        let mut v = value.map_err(|_| "Hunter 未返回有效 JSON".to_string())?;
        // Do not let a provider-echoed credential reach persisted page payloads either.
        if !key.is_empty() {
            v = serde_json::from_str(&v.to_string().replace(key, "[REDACTED]")).map_err(err)?;
        }
        Ok(v)
    })();
    meta["elapsedMs"] = json!(started.elapsed().as_millis() as u64);
    meta["outcome"] = json!(match &result {
        Ok(v) if v["code"].as_i64().is_some_and(|code| code != 200) => "provider_rejected",
        Ok(_) => "response_received",
        Err(_) => "request_failed",
    });
    if let Err(e) = &result {
        meta["error"] = json!(safe(e, key, 4000));
    }
    let written = context.event(key, "request_finished", meta);
    match result {
        Err(e) => Err(format!(
            "{e}；诊断编号 {request}；未自动重试{}",
            written.err().map(|e| format!("；{e}")).unwrap_or_default()
        )),
        Ok(v) => {
            written
                .map_err(|e| format!("远端响应已到达，但{e}；诊断编号 {request}；禁止自动重发"))?;
            Ok(v)
        }
    }
}

#[cfg(test)]
#[path = "hunter_diagnostics_tests.rs"]
mod tests;
