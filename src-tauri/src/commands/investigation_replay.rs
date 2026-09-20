// Single-request replay issued from the investigation desk. Included from
// investigation.rs.

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationReplayInput {
    pub url: String,
    pub method: String,
    #[serde(default)]
    pub headers: JsonValue,
    #[serde(default)]
    pub body: String,
    #[serde(default = "default_replay_timeout")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub allow_mutation: bool,
    #[serde(default)]
    pub identity_id: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationReplayResult {
    pub status: u16,
    pub status_text: String,
    pub headers: JsonValue,
    pub body: String,
    pub decoded_body: String,
    pub content_type: String,
    pub content_encoding: String,
    pub body_is_json: bool,
    pub elapsed_ms: u128,
    pub identity_id: String,
}

fn default_replay_timeout() -> u64 { 120000 }

fn decode_replay_body(bytes: &[u8], encoding: &str) -> Result<Vec<u8>, String> {
    use std::io::Read;
    match encoding.split(',').next().unwrap_or("").trim() {
        "gzip" => { let mut decoder = flate2::read::GzDecoder::new(bytes); let mut output = Vec::new(); decoder.read_to_end(&mut output).map_err(|e| e.to_string())?; Ok(output) }
        "deflate" => { let mut decoder = flate2::read::ZlibDecoder::new(bytes); let mut output = Vec::new(); decoder.read_to_end(&mut output).map_err(|e| e.to_string())?; Ok(output) }
        "br" => { let mut decoder = brotli::Decompressor::new(bytes, 4096); let mut output = Vec::new(); decoder.read_to_end(&mut output).map_err(|e| e.to_string())?; Ok(output) }
        _ => Ok(bytes.to_vec()),
    }
}

#[tauri::command]
pub async fn replay_investigation_request(input: InvestigationReplayInput) -> Result<InvestigationReplayResult, String> {
    let identity_id = input.identity_id.clone();
    let parsed = reqwest::Url::parse(input.url.trim()).map_err(|error| format!("请求 URL 无效：{error}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("只允许发送 HTTP/HTTPS 请求".into());
    }
    let method = input.method.trim().to_ascii_uppercase();
    if method.is_empty() || method.len() > 12 || !method.bytes().all(|value| value.is_ascii_alphabetic()) {
        return Err("请求方法无效".into());
    }
    if !input.allow_mutation && !matches!(method.as_str(), "GET" | "HEAD" | "OPTIONS") {
        return Err("当前为只读重放模式；POST/PUT/PATCH/DELETE 需要显式授权后再发送".into());
    }
    if input.body.len() > 512 * 1024 {
        return Err("请求体超过 512 KB 限制".into());
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(input.timeout_ms.clamp(1000, 120000)))
        .redirect(reqwest::redirect::Policy::limited(3))
        .user_agent("Oviraptor-Repeater/1.0")
        .build()
        .map_err(|error| format!("创建重放客户端失败：{error}"))?;
    let mut request = client.request(
        reqwest::Method::from_bytes(method.as_bytes()).map_err(|error| error.to_string())?,
        parsed,
    );
    if let Some(headers) = input.headers.as_object() {
        for (key, value) in headers {
            if key.starts_with(':') || matches!(key.to_ascii_lowercase().as_str(), "host" | "content-length" | "connection" | "transfer-encoding" | "accept-encoding") { continue; }
            let Some(value) = value.as_str() else { continue; };
            let name = reqwest::header::HeaderName::from_bytes(key.as_bytes()).map_err(|error| format!("请求头无效：{error}"))?;
            let value = reqwest::header::HeaderValue::from_str(value).map_err(|error| format!("请求头值无效：{error}"))?;
            request = request.header(name, value);
        }
    }
    if !input.body.is_empty() { request = request.body(input.body); }
    let started = std::time::Instant::now();
    let response = request.send().await.map_err(|error| format!("发送请求失败：{error}"))?;
    let status = response.status();
    let status_text = status.canonical_reason().unwrap_or("").to_string();
    let headers = serde_json::Value::Object(response.headers().iter().map(|(key, value)| (key.to_string(), serde_json::Value::String(value.to_str().unwrap_or("").to_string()))).collect());
    let content_type = response.headers().get(reqwest::header::CONTENT_TYPE).and_then(|value| value.to_str().ok()).unwrap_or("").to_string();
    let content_encoding = response.headers().get(reqwest::header::CONTENT_ENCODING).and_then(|value| value.to_str().ok()).unwrap_or("").to_ascii_lowercase();
    let raw_bytes = response.bytes().await.map_err(|error| format!("读取响应失败：{error}"))?.to_vec();
    let decoded_bytes = decode_replay_body(&raw_bytes, &content_encoding).unwrap_or_else(|_| raw_bytes.clone());
    let body = String::from_utf8_lossy(&raw_bytes).chars().take(2_000_000).collect::<String>();
    let decoded_body = String::from_utf8_lossy(&decoded_bytes).chars().take(2_000_000).collect::<String>();
    let body_is_json = serde_json::from_str::<JsonValue>(&decoded_body).is_ok() || content_type.to_ascii_lowercase().contains("json");
    Ok(InvestigationReplayResult { status: status.as_u16(), status_text, headers, body, decoded_body, content_type, content_encoding, body_is_json, elapsed_ms: started.elapsed().as_millis(), identity_id })
}
