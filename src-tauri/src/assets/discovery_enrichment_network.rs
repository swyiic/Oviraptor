use super::super::certificate_tasks::tls;
use super::*;
use std::net::IpAddr;
pub(super) fn endpoint(link: &str, fallback: &str) -> Option<String> {
    let mut u = reqwest::Url::parse(if link.is_empty() { fallback } else { link }).ok()?;
    if u.scheme() != "https"
        || !u.username().is_empty()
        || u.password().is_some()
        || super::super::host(u.host_str()?).is_none()
    {
        return None;
    }
    u.set_path("/");
    u.set_query(None);
    u.set_fragment(None);
    Some(u.to_string())
}
pub(super) fn client(proxy: &str) -> Result<reqwest::blocking::Client, String> {
    let mut b = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("Oviraptor/ScopedEvidenceDiscovery");
    if !proxy.is_empty() {
        b = b.proxy(reqwest::Proxy::all(proxy).map_err(|_| "业务代理格式无效")?)
    }
    b.build().map_err(|e| format!("发现客户端初始化失败：{e}"))
}
fn fetch(
    client: &reqwest::blocking::Client,
    url: reqwest::Url,
    max: usize,
) -> Result<Value, String> {
    let source = url.host_str().unwrap_or("external").to_string();
    let response = client.get(url).send().map_err(|e| {
        format!(
            "{source} 请求失败：{}；不改变VPN或系统代理",
            e.without_url()
        )
    })?;
    let code = response.status();
    if !code.is_success() {
        return Err(format!(
            "{source} HTTP {}；{}；稍后退避重试，不消耗Hunter积分",
            code.as_u16(),
            if code.as_u16() == 403 {
                "服务拒绝访问，不能据此判断资产不存在"
            } else if code.as_u16() == 429 {
                "触发平台限流"
            } else {
                "外部服务未成功"
            }
        ));
    }
    let mut bytes = vec![];
    response
        .take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{source} 响应读取失败：{e}"))?;
    if bytes.len() > max {
        return Err(format!(
            "{source} 响应超过{}MiB，未截取前1000条冒充完整结果",
            max / 1024 / 1024
        ));
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| format!("{source} 非有效JSON（{}字节），未采纳", bytes.len()))
}
pub(super) fn execute(
    path: &Path,
    id: &str,
    item: &Item,
    client: &reqwest::blocking::Client,
) -> Result<Value, String> {
    let c = db::open(path)?;
    let p: i64 = c
        .query_row(
            "SELECT project_id FROM asset_enrichment_runs WHERE id=?1 AND enabled=1",
            [id],
            |r| r.get(0),
        )
        .map_err(|_| "补证已暂停，未启动新请求")?;
    validate(&c, p, id)?;
    if !admitted(&c,p,&item.host)? {
        return Err("域名或其全部线索来源已被当前项目排除，未发出 CT / DNS / TLS 请求".into());
    }
    match item.kind.as_str() {
        "ct" => {
            let mut url = reqwest::Url::parse("https://crt.sh/").unwrap();
            url.query_pairs_mut()
                .append_pair("q", &format!("%.{}", item.host))
                .append_pair("output", "json");
            let response = fetch(client, url, 16 * 1024 * 1024)?;
            ct(&response, &item.host)
        }
        "dns_a" | "dns_aaaa" | "dns_cname" => {
            let kind = match item.kind.as_str() {
                "dns_a" => 1,
                "dns_aaaa" => 28,
                _ => 5,
            };
            let mut url = reqwest::Url::parse("https://dns.google/resolve").unwrap();
            url.query_pairs_mut()
                .append_pair("name", &item.host)
                .append_pair("type", &kind.to_string())
                .append_pair("edns_client_subnet", "0.0.0.0/0");
            let response = fetch(client, url, 2 * 1024 * 1024)?;
            dns(&response, &item.host, kind)
        }
        "tls" => {
            let stored:Vec<String>=c.prepare("SELECT result_json FROM asset_enrichment_items WHERE run_id=?1 AND host=?2 AND kind IN ('dns_a','dns_aaaa') AND status='done'").map_err(err)?.query_map(params![id,item.host],|r|r.get(0)).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;
            let ips = stored
                .into_iter()
                .filter_map(|s| serde_json::from_str::<Value>(&s).ok())
                .flat_map(|v| v["records"].as_array().cloned().unwrap_or_default())
                .filter(|r| r["type"] == 1 || r["type"] == 28)
                .filter_map(|r| r["data"].as_str().and_then(|s| s.parse::<IpAddr>().ok()))
                .filter(|ip| tls::public(*ip))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            // Reuse saved raw DER first only when its actual fingerprint was missing.
            let local:Option<String>=c.query_row("SELECT a.extra_json FROM assets a JOIN project_assets pa ON pa.asset_id=a.id WHERE pa.project_id=?1 AND (a.link=?2 OR a.host=?2) AND COALESCE(json_extract(a.extra_json,'$.probe_cert_sha256'),'')='' LIMIT 1",params![p,item.target],|r|r.get(0)).optional().map_err(err)?;
            let raw = local
                .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                .and_then(|v| {
                    [
                        "probe_cert_der_base64",
                        "certificate_der_base64",
                        "certificate_pem",
                        "ssl_certificate",
                    ]
                    .iter()
                    .find_map(|key| v[*key].as_str().and_then(crate::certificates::from_text))
                });
            let is_local = raw.is_some();
            let cert = match raw {
                Some(c) => c,
                None => {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(err)?;
                    rt.block_on(tls::capture_at(
                        &item.target,
                        8,
                        false,
                        if ips.is_empty() { None } else { Some(ips) },
                    ))?
                }
            };
            let patterns = cert.server_names();
            let matches = patterns.iter().any(|n| covers(&item.host, n));
            Ok(
                json!({"sha256":cert.sha256,"der":cert.der_base64,"names":cert.dns_names,"commonNames":cert.common_names,"organizations":cert.organizations,"hostnameMatches":matches,"tlsTrustVerified":false,"networkRequest":!is_local,"summary":if matches{"叶证书已读取并覆盖请求主机；未确认CA信任、页面存活或业务归属"}else{"叶证书不覆盖请求主机；按指纹合并为冲突，不自动送入Hunter"}}),
            )
        }
        _ => Err("发现动作无效".into()),
    }
}
pub(super) fn covers(h: &str, pattern: &str) -> bool {
    let p = pattern.trim().trim_end_matches('.').to_ascii_lowercase();
    if let Some(root) = p.strip_prefix("*.") {
        h.strip_suffix(&format!(".{root}"))
            .is_some_and(|prefix| !prefix.is_empty() && !prefix.contains('.'))
    } else {
        h == p
    }
}
pub(super) fn ct(v: &Value, root: &str) -> Result<Value, String> {
    let entries = v.as_array().ok_or("crt.sh未返回证书列表")?;
    let mut names = BTreeMap::<(String, bool), i64>::new();
    for cert in entries {
        for (name, wildcard) in field_hosts(&cert["name_value"]) {
            if in_scope(&name, root) {
                names
                    .entry((name, wildcard))
                    .and_modify(|id| *id = (*id).max(cert["id"].as_i64().unwrap_or(0)))
                    .or_insert(cert["id"].as_i64().unwrap_or(0));
            }
        }
    }
    Ok(
        json!({"source":"crt.sh","entries":entries.len(),"names":names.into_iter().map(|((name,wildcard),id)|json!({"name":name,"wildcard":wildcard,"certificateId":id})).collect::<Vec<_>>(),"responseTruncated":false,"summary":format!("已处理CT返回的全部{}条证书记录；证书出现不代表主机存在",entries.len())}),
    )
}
pub(super) fn dns(v: &Value, name: &str, kind: i64) -> Result<Value, String> {
    if v["TC"] == true {
        return Err("DNS响应标记截断，未作为完整结果采纳".into());
    }
    let code = v["Status"].as_i64().ok_or("DNS响应缺少状态码")?;
    if ![0, 3].contains(&code) {
        return Err(format!(
            "DNS RCODE {code}：{}",
            v["Comment"].as_str().unwrap_or("解析服务未成功")
        )
        .chars()
        .take(600)
        .collect());
    }
    let question = v["Question"]
        .as_array()
        .and_then(|q| q.first())
        .ok_or("DNS响应缺少原始问题")?;
    if question["name"]
        .as_str()
        .map(|s| s.trim_end_matches('.').to_ascii_lowercase())
        != Some(name.to_string())
        || question["type"] != kind
    {
        return Err("DNS返回的问题与请求不匹配，未采纳".into());
    }
    let answers = v["Answer"].as_array().cloned().unwrap_or_default();
    let mut owners = BTreeSet::from([name.to_string()]);
    let mut records = BTreeSet::<(String, i64, String)>::new();
    for _ in 0..32 {
        let before = owners.len();
        for row in &answers {
            let Some(owner) = row["name"].as_str().and_then(host) else {
                continue;
            };
            if !owners.contains(&owner) {
                continue;
            }
            let t = row["type"].as_i64().unwrap_or(0);
            if t == 5 {
                if let Some(target) = row["data"].as_str().and_then(host) {
                    owners.insert(target.clone());
                    records.insert((owner, t, target));
                }
            } else if [1, 28].contains(&t) {
                if let Some(ip) = row["data"].as_str().and_then(|s| s.parse::<IpAddr>().ok()) {
                    records.insert((owner, t, ip.to_string()));
                }
            }
        }
        if owners.len() == before {
            break;
        }
    }
    Ok(
        json!({"name":name,"queryType":kind,"status":code,"records":records.into_iter().map(|(name,kind,data)|json!({"name":name,"type":kind,"data":data})).collect::<Vec<_>>(),"summary":if code==3{"DNS返回NXDOMAIN，保存结果并按间隔复查；不据此排除历史资产"}else{"DNS地址/CNAME关系已保存；外部CNAME不扩域或跟随探测"}}),
    )
}
