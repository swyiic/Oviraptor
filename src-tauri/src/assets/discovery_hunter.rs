// Contract: user-provided Hunter official help-center text (2026-10-20).
// Live connectivity is separate from this documented HTTP/JSON contract.
const HUNTER_CONTRACT_VERIFIED: bool = true;


fn hunter_account(payload: &Value) -> Result<Value, String> {
    if payload["code"].as_i64() != Some(200) {
        return Err(format!("Hunter 账户预检未通过（code={}）：{}；未发出资产查询，请检查账户状态或网络",payload["code"],payload["message"].as_str().unwrap_or("接口未提供错误说明").chars().take(500).collect::<String>()));
    }
    let d = payload
        .get("data")
        .and_then(Value::as_object)
        .ok_or("Hunter 用户信息返回结构不完整")?;
    // Never return personal_info, phone, username, or the credential.
    let mut account = serde_json::Map::new();
    for field in [
        "type",
        "rest_equity_point",
        "rest_free_point",
        "rest_export_quota",
        "day_free_point",
        "day_export_quota",
        "once_export_quota",
    ] {
        if let Some(v) = d.get(field) {
            account.insert(field.into(), v.clone());
        }
    }
    Ok(Value::Object(account))
}




fn hunter_user_info(client: &reqwest::blocking::Client, key: &str, diagnostic: &hunter_diagnostics::Context) -> Result<Value, String> {
    let mut url = reqwest::Url::parse("https://hunter.qianxin.com/openApi/userInfo").unwrap();
    url.query_pairs_mut().append_pair("api-key", key);
    let payload=hunter_diagnostics::get_json(client, url, key, diagnostic)?;
    // Error messages are filtered before they reach the UI or durable task reason.
    let mut safe_payload=payload.clone();
    if let Some(message)=payload["message"].as_str() { safe_payload["message"]=json!(hunter_diagnostics::message(message,key)); }
    hunter_account(&safe_payload)
}
