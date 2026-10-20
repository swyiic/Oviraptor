//! Asset-source FOFA account validation and connection test.
use crate::asset_models::{FofaApiTestInput, FofaApiTestResult};
use serde_json::Value as JsonValue;
use std::{io::Read, time::Duration};

// Email is optional for current key-only accounts. Compare account identities
// only when FOFA returns an unmasked address, not its privacy placeholder.
pub(crate) fn normalize_fofa_email(email: &str) -> Result<String, String> {
    let email = email.trim();
    if email.is_empty() {
        return Ok(String::new());
    }
    let valid = email.len() <= 320
        && !email
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
        && email.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty() && !domain.is_empty() && !domain.contains('@')
        });
    if !valid {
        return Err("FOFA 账户邮箱格式无效；也可留空，仅使用 Key".into());
    }
    Ok(email.to_string())
}

fn fofa_transport_error(error: &reqwest::Error, proxy_url: &str) -> String {
    if error.is_timeout() {
        return if proxy_url.is_empty() {
            "FOFA 连接超时，请检查当前网络后重试".into()
        } else {
            "FOFA 连接超时；当前配置的代理没有正常转发请求".into()
        };
    }
    if !proxy_url.is_empty() {
        return "无法通过已配置的代理连接 FOFA；请确认代理程序已启动且端口正确".into();
    }
    "无法连接 FOFA，请检查 DNS、网络或防火墙后重试".into()
}

fn fofa_account_endpoint(api_url: &str, key: &str, email: &str) -> Result<reqwest::Url, String> {
    if key.is_empty()
        || !key
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err("FOFA Key 格式无效".into());
    }
    let email = normalize_fofa_email(email)?;
    let mut endpoint = reqwest::Url::parse(api_url).map_err(|_| "FOFA API 地址无效")?;
    endpoint.query_pairs_mut().append_pair("key", key);
    if !email.is_empty() {
        endpoint.query_pairs_mut().append_pair("email", &email);
    }
    Ok(endpoint)
}

fn request_fofa_account_with_client(
    client: &reqwest::blocking::Client,
    api_url: &str,
    key: &str,
    email: &str,
    proxy_url: &str,
) -> Result<JsonValue, String> {
    let email = normalize_fofa_email(email)?;
    let endpoint = fofa_account_endpoint(api_url, key, &email)?;
    let response = client
        .get(endpoint)
        .send()
        .map_err(|error| fofa_transport_error(&error, proxy_url))?;
    let status = response.status();
    // A remote response and its diagnostic strings are untrusted. Do not echo
    // an authentication URL, key or proxy userinfo from them into UI/job logs.
    let mut body = Vec::new();
    response
        .take(512 * 1024 + 1)
        .read_to_end(&mut body)
        .map_err(|_| "读取 FOFA 响应失败".to_string())?;
    if body.len() > 512 * 1024 {
        return Err("FOFA 账户响应过大".into());
    }
    let payload: JsonValue = serde_json::from_slice(&body)
        .map_err(|_| format!("FOFA 返回了无法解析的响应（HTTP {}）", status.as_u16()))?;
    if !status.is_success()
        || payload.get("error").and_then(JsonValue::as_bool) != Some(false)
    {
        return Err(format!(
            "FOFA 鉴权失败（HTTP {}）；请检查 Key、账户或账户权限",
            status.as_u16()
        ));
    }
    if !email.is_empty() {
        let actual = payload
            .get("email")
            .and_then(JsonValue::as_str)
            .ok_or("FOFA 未返回账户邮箱，无法确认所填账户与 Key 是否匹配")?;
        // FOFA can return "********" even for a valid authenticated account.
        // That value cannot establish an identity mismatch. Authentication
        // above must still explicitly succeed; full addresses remain checked.
        if !actual.contains('*') && !actual.trim().eq_ignore_ascii_case(&email) {
            return Err("FOFA 账户与 Key 不匹配；请检查账户邮箱或更新 Key".into());
        }
    }
    Ok(payload)
}

pub(crate) fn request_fofa_account(
    key: &str,
    proxy_url: &str,
    email: &str,
) -> Result<JsonValue, String> {
    // Validate before any network activity, including proxy initialization.
    fofa_account_endpoint("https://fofa.info/api/v1/info/my", key, email)?;
    let mut builder = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none());
    if !proxy_url.is_empty() {
        let proxy = reqwest::Proxy::all(proxy_url)
            .map_err(|_| "代理地址格式无效，应类似 http://127.0.0.1:7890".to_string())?;
        builder = builder.proxy(proxy);
    }
    let client = builder
        .build()
        .map_err(|_| "无法初始化 FOFA 网络客户端".to_string())?;
    request_fofa_account_with_client(
        &client,
        "https://fofa.info/api/v1/info/my",
        key,
        email,
        proxy_url,
    )
}

#[tauri::command]
pub async fn test_fofa_api(input: FofaApiTestInput) -> Result<FofaApiTestResult, String> {
    let key = input.key.trim().to_string();
    let proxy_url = input.proxy_url.trim().to_string();
    let email = normalize_fofa_email(&input.email)?;
    if key.is_empty() {
        return Err("请先填写 FOFA key".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let payload = request_fofa_account(&key, &proxy_url, &email)?;
        let account = payload
            .get("email")
            .or_else(|| payload.get("username"))
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .to_string();
        let vip = payload.get("vip_level").and_then(|v| v.as_i64().or_else(||v.as_str()?.parse().ok()));
        let plan = match vip {
            Some(2) => "旧版高级会员 (vip_level=2)；不等同新版商业版".to_string(),
            Some(1) => "旧版普通会员 (vip_level=1)".to_string(),
            Some(3 | 5) => format!("旧版企业会员 (vip_level={})", vip.unwrap()),
            Some(11) => "订阅个人版 (11)".into(), Some(12) => "订阅专业版 (12)".into(),
            Some(13) => "订阅商业版 (13)".into(), Some(level) => format!("vip_level={level}；权限以接口实际返回为准"),
            None => "账户有效，未返回会员等级".into(),
        };
        let quota = ["fofa_point", "remain_api_query", "remain_api_data"].into_iter()
            .filter_map(|key| payload.get(key).filter(|v|v.is_number()).map(|v|format!("{key}={v}")))
            .collect::<Vec<_>>().join("，");
        Ok(FofaApiTestResult {
            ok: true,
            status: "available".into(),
            message: format!("FOFA 鉴权可用。{}。F点、API查询次数和可导出条数是独立额度；遇额度不足停止，不自动切换付费。", quota),
            account,
            plan,
        })
    })
    .await
    .map_err(|error| format!("FOFA 测试任务异常：{error}"))?
}

#[cfg(test)]
#[path = "fofa_tests.rs"]
mod tests;
