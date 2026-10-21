// Contract: user-provided Hunter official help-center text (2026-10-20).
// Live connectivity is separate from this documented HTTP/JSON contract.
const HUNTER_CONTRACT_VERIFIED: bool = true;

fn hunter_query(
    c: &Connection,
    project_id: i64,
    scope: &str,
    kind: &str,
) -> Result<String, String> {
    let quoted = serde_json::to_string(scope).map_err(err)?;
    match kind {
        "domain" => {
            let root = crate::commands::ownership::ownership_domain_parts(scope)
                .and_then(|p| p.registrable);
            Ok(if root.as_deref() == Some(scope) {
                format!("domain.suffix={quoted}")
            } else {
                // A subdomain scope must not expand to its whole parent domain.
                format!(
                    "(domain=={quoted}||domain={})",
                    serde_json::to_string(&format!(".{scope}")).map_err(err)?
                )
            })
        }
        "certificate" | "cname" => {
            let sources = if kind == "certificate" {
                [
                    "certificate_domain",
                    "certificate_cn",
                    "certificate_transparency",
                ]
            } else {
                ["cname", "dns_cname", "dns_cname"]
            };
            let observed: bool = c.query_row(
                "SELECT EXISTS(SELECT 1 FROM asset_discovery_seeds WHERE project_id=?1 AND host=?2 AND source IN (?3,?4,?5))",
                params![project_id, scope, sources[0], sources[1], sources[2]], |r| r.get(0),
            ).map_err(err)?;
            if !observed {
                return Err(
                    "此补查方式需要当前项目实际提取到的证书或 CNAME 线索，不能只凭企业名称生成"
                        .into(),
                );
            }
            Ok(if kind == "certificate" {
                format!("cert.subject.suffix={quoted}")
            } else {
                format!("cname_target=={quoted}")
            })
        }
        _ => Err("不支持的 Hunter 补查方式".into()),
    }
}

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

fn hunter_page_size(account: &Value, requested: i64) -> Result<usize, String> {
    if account["rest_free_point"].as_i64() == Some(0) {
        return Err("Hunter 今日免费积分为 0；未发出资产查询，不自动改用权益积分".into());
    }
    let mut limit = requested.clamp(10, 100);
    for field in ["once_export_quota", "rest_export_quota", "rest_free_point"] {
        if let Some(value) = account[field].as_i64().filter(|n| *n >= 0) {
            limit = limit.min(value);
        }
    }
    // Official examples use 10; never probe alternate page sizes by spending queries.
    [100, 50, 20, 10]
        .into_iter()
        .find(|size| *size <= limit)
        .map(|n| n as usize)
        .ok_or_else(|| "Hunter 剩余额度不足以查询最小 10 条；未发出资产查询".into())
}

fn hunter_search_url(
    base: &str,
    key: &str,
    query: &str,
    size: usize,
    today: chrono::NaiveDate,
) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(base).map_err(err)?;
    // Inclusive 30 calendar days; never silently request historical paid data.
    let start = today - chrono::Duration::days(29);
    url.query_pairs_mut().append_pair("api-key", key)
        .append_pair("search", &base64::engine::general_purpose::URL_SAFE.encode(query.as_bytes()))
        .append_pair("page", "1").append_pair("page_size", &size.to_string()).append_pair("is_web", "1")
        .append_pair("start_time", &start.to_string()).append_pair("end_time", &today.to_string())
        .append_pair("fields", "ip,port,domain,url,web_title,protocol,status_code,company,number,cert_sha256,ssl_certificate,updated_at");
    Ok(url)
}

fn hunter_rows(payload: &Value, scope: &str, max: usize, kind: &str) -> Result<Vec<Value>, String> {
    if payload["code"].as_i64() != Some(200) {
        return Err("Hunter 查询未成功；可能是语法、权限或额度限制，未自动重试".into());
    }
    let d = payload
        .get("data")
        .and_then(Value::as_object)
        .ok_or("Hunter 返回结构不完整")?;
    let empty = vec![];
    let a = match d.get("arr").and_then(Value::as_array) {
        Some(a) => a,
        None if d.get("total").and_then(Value::as_u64) == Some(0) => &empty,
        None => return Err("Hunter 返回结构不完整".into()),
    };
    let mut rows = vec![];
    for raw in a.iter().take(max) {
        let remote_url = raw["url"].as_str().unwrap_or("");
        let remote_domain = raw["domain"].as_str().unwrap_or("");
        let ip = raw["ip"].as_str().unwrap_or("");
        if !ip.is_empty() && ip.parse::<std::net::IpAddr>().is_err() {
            continue;
        }
        let port = raw["port"]
            .as_u64()
            .or_else(|| raw["port"].as_str().and_then(|p| p.parse().ok()));
        if !port.is_some_and(|p| (1..=65535).contains(&p)) {
            continue;
        }
        let protocol = raw["protocol"].as_str().unwrap_or("https");
        if !["http", "https"].contains(&protocol) {
            continue;
        }
        let link = if remote_url.is_empty() {
            let Some(h) = host(remote_domain) else {
                continue;
            };
            format!("{protocol}://{h}:{}/", port.unwrap())
        } else {
            remote_url.to_string()
        };
        let Ok(u) = reqwest::Url::parse(&link) else {
            continue;
        };
        if !["http", "https"].contains(&u.scheme())
            || !u.username().is_empty()
            || u.password().is_some()
            || u.query().is_some()
            || u.fragment().is_some()
        {
            continue;
        }
        let Some(endpoint) = u.host_str() else {
            continue;
        };
        if u.port_or_known_default().map(u64::from) != port {
            continue;
        }
        // For domain lookup use the actual endpoint, never a remote domain label.
        // Certificate/CNAME results may be IPs or external names: retain as leads.
        if kind == "domain" && !host(&link).is_some_and(|h| in_scope(&h, scope)) {
            continue;
        }
        if endpoint.parse::<std::net::IpAddr>().is_err() && host(&link).is_none() {
            continue;
        }
        let evidence = match kind {
            "domain" => format!("注册域:{scope}"),
            "certificate" => format!("证书域:{scope} | Hunter证书查询命中待核验"),
            "cname" => format!("CNAME线索:{scope} | Hunter解析关系命中待核验"),
            _ => return Err("不支持的 Hunter 补查方式".into()),
        };
        rows.push(json!({"host":endpoint,"link":u.to_string(),"ip":ip,"port":port.unwrap().to_string(),
            "title":raw["web_title"].as_str().unwrap_or(""),"protocol":u.scheme(),
            "status_code":raw["status_code"].as_i64().map(|s|s.to_string()).unwrap_or_default(),
            "lastupdatetime":raw["updated_at"].as_str().unwrap_or(""),"domain":remote_domain,
            "cert_sha256":raw["cert_sha256"].as_str().unwrap_or(""),"ssl_certificate":raw["ssl_certificate"].as_str().unwrap_or(""),
            "evidence":evidence,"hunter_query_kind":kind,"matched_queries":format!("Hunter {kind}: {scope}"),
            "score":if kind=="domain"{"70"}else{"40"}}));
    }
    Ok(rows)
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
