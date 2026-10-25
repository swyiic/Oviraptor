use super::*;
use std::io::Read as _;
fn fetch_json(
    client: &reqwest::blocking::Client,
    url: reqwest::Url,
    max: u64,
) -> Result<(Value, String), String> {
    let r = client
        .get(url)
        .send()
        .map_err(|e| format!("公开研究请求失败：{}；保留候选，不改变VPN", e.without_url()))?;
    if !r.status().is_success() {
        return Err(format!(
            "公开研究 HTTP {}；不能据此判断目标不存在或无关",
            r.status().as_u16()
        ));
    }
    let mut bytes = Vec::new();
    r.take(max + 1).read_to_end(&mut bytes).map_err(err)?;
    if bytes.len() as u64 > max {
        return Err("公开研究响应超出大小限制，未当作完整结果".into());
    }
    Ok((
        serde_json::from_slice(&bytes).map_err(|_| "公开研究响应不是有效JSON")?,
        format!("{:x}", Sha256::digest(&bytes)),
    ))
}
fn research(
    client: &reqwest::blocking::Client,
    c: &Connection,
    k: &work::Claim,
    key: &str,
    kind: &str,
    target: &str,
) -> Result<Value, String> {
    match kind {
        "ct_search" => {
            let item = passive_evidence::Item {
                id: 0,
                kind: kind.into(),
                target: target.into(),
                host: target.into(),
                token: String::new(),
                attempts: 1,
            };
            let mut v = passive_evidence::execute(client, &item)?;
            v["kind"] = json!(kind);
            v["sourceUrl"] = json!(format!(
                "https://crt.sh/?q={target}&output=json&exclude=expired"
            ));
            v["checkedAt"] = json!(now());
            v["receiptSha256"] = json!(hash(&v));
            Ok(v)
        }
        "ct_certificate" => {
            let domain: String = c
                .query_row(
                    "SELECT domain FROM asset_cleanup_groups WHERE run_id=?1 AND group_key=?2",
                    params![k.id, key],
                    |r| r.get(0),
                )
                .map_err(err)?;
            let item = passive_evidence::Item {
                id: 0,
                kind: kind.into(),
                target: target.into(),
                host: domain.clone(),
                token: String::new(),
                attempts: 1,
            };
            let v = passive_evidence::execute(client, &item)?;
            let current = v["notBefore"].as_i64().is_some_and(|t| t <= now())
                && v["notAfter"].as_i64().is_some_and(|t| t >= now())
                && v["precertificate"] != true;
            let names = v["names"].as_array().cloned().unwrap_or_default();
            let hosts:Vec<String>=c.prepare("SELECT DISTINCT host FROM asset_cleanup_seeds WHERE run_id=?1 AND group_key=?2 AND source!='certificate_sha256'").map_err(err)?.query_map(params![k.id,key],|r|r.get(0)).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;
            let mut facts = Vec::new();
            if current {
                for candidate in hosts.iter().filter_map(|h| host(h)) {
                    let covers = names
                        .iter()
                        .filter_map(Value::as_str)
                        .any(|n| enrichment::certificate_covers(&candidate, n));
                    if !covers {
                        continue;
                    }
                    for organization in v["organizations"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                    {
                        facts.push(json!({"host":candidate,"organization":organization,"source":"public_ct_subject","certificateSha256":v["sha256"],"sourceUrl":v["sourceUrl"],"checkedAt":now(),"notAfter":v["notAfter"]}));
                    }
                }
            }
            Ok(
                json!({"kind":kind,"certificateId":target,"current":current,"facts":facts,"names":names,"sourceUrl":v["sourceUrl"],"bodySha256":v["sha256"],"checkedAt":now(),"certificateType":v["certificateType"],"hunterRequests":0}),
            )
        }
        "dns_cname" => {
            let mut url = reqwest::Url::parse("https://dns.google/resolve").unwrap();
            url.query_pairs_mut()
                .append_pair("name", target)
                .append_pair("type", "5")
                .append_pair("edns_client_subnet", "0.0.0.0/0");
            let source = url.to_string();
            let (v, sha) = fetch_json(client, url, 2 * 1024 * 1024)?;
            let records = parse_dns(&v, target)?;
            Ok(
                json!({"kind":kind,"sourceUrl":source,"bodySha256":sha,"checkedAt":now(),"dnsStatus":v["Status"],"records":records,"facts":[],"hunterRequests":0}),
            )
        }
        _ => Err("cleanup_research_kind_invalid".into()),
    }
}
pub(super) fn parse_dns(v: &Value, target: &str) -> Result<Vec<Value>, String> {
    if v["TC"] == true || !matches!(v["Status"].as_i64(), Some(0 | 3)) {
        return Err("DNS返回错误或截断，未采纳为完整结果".into());
    }
    let questions = v["Question"].as_array().ok_or("DNS响应缺少请求绑定")?;
    if questions.len() != 1
        || questions[0]["type"] != 5
        || questions[0]["name"].as_str().and_then(host).as_deref() != Some(target)
    {
        return Err("DNS响应Question与本次候选不匹配".into());
    }
    let mut mapping = BTreeMap::<String, String>::new();
    for r in v["Answer"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["type"] == 5)
    {
        let name = r["name"]
            .as_str()
            .and_then(host)
            .ok_or("DNS CNAME名称无效")?;
        let next = r["data"]
            .as_str()
            .and_then(host)
            .ok_or("DNS CNAME目标无效")?;
        if mapping.insert(name, next).is_some() {
            return Err("DNS CNAME同名返回多个目标，保留冲突".into());
        }
    }
    let mut current = target.to_string();
    let mut seen = BTreeSet::new();
    let mut records = Vec::new();
    while let Some(next) = mapping.get(&current) {
        if records.len() >= 32 || !seen.insert(current.clone()) {
            return Err("DNS CNAME循环或链过长，未采纳".into());
        }
        records.push(json!({"name":current,"cname":next}));
        current = next.clone();
    }
    if records.len() != mapping.len() {
        return Err("DNS响应含与候选无关的CNAME答案，未采纳".into());
    }
    Ok(records)
}
pub(super) fn one(path: &Path, c: &mut Connection, k: &work::Claim) -> Result<bool, String> {
    if !work::claim_valid(c, k)? {
        return Ok(false);
    }
    let row:Option<(i64,String,String,String,i64)>=c.query_row("SELECT t.id,t.group_key,t.kind,t.target,t.attempts FROM asset_cleanup_research t JOIN asset_cleanup_groups g ON g.run_id=t.run_id AND g.group_key=t.group_key WHERE t.run_id=?1 AND t.state IN ('pending','retry') AND t.next_at<=?2 AND g.state='evidence' ORDER BY CASE g.advice WHEN 'exclude' THEN 0 ELSE 1 END,CASE t.kind WHEN 'ct_certificate' THEN 0 WHEN 'ct_search' THEN 1 ELSE 2 END,t.id LIMIT 1",params![k.id,now()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(err)?;
    let Some((id, key, kind, target, attempts)) = row else {
        return Ok(false);
    };
    let settings = crate::model_configuration::default_settings_for(c, &["network"]);
    if settings.get("_configurationError").is_some() {
        return Err("cleanup_network_configuration_unavailable".into());
    }
    let proxy = settings["proxyUrl"].as_str().unwrap_or("");
    let client = passive_evidence::client(proxy)?;
    c.execute("UPDATE asset_cleanup_research SET state='running',claim_token=?2,attempts=attempts+1 WHERE id=?1",params![id,k.token]).map_err(err)?;
    let outcome = research(&client, c, k, &key, &kind, &target);
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    if !work::claim_valid(&tx, k)? {
        return Ok(true);
    }
    match outcome {
        Ok(v) => {
            tx.execute("UPDATE asset_cleanup_research SET state='done',result_json=?3,error='',claim_token='' WHERE id=?1 AND claim_token=?2",params![id,k.token,v.to_string()]).map_err(err)?;
            if kind == "ct_search" {
                // All returned IDs are durable work; classification may stop
                // further GETs once the family has enough independent evidence.
                let ids = v["certificateIds"].as_array().cloned().unwrap_or_default();
                let mut ids = ids
                    .into_iter()
                    .filter_map(|v| v.as_i64())
                    .collect::<Vec<_>>();
                ids.sort_unstable_by(|a, b| b.cmp(a));
                for cert in ids {
                    tx.execute("INSERT OR IGNORE INTO asset_cleanup_research(run_id,group_key,kind,target) VALUES(?1,?2,'ct_certificate',?3)",params![k.id,key,cert.to_string()]).map_err(err)?;
                }
            }
        }
        Err(e) => {
            let delay = match attempts {
                0 => 60,
                1 => 300,
                2 => 1800,
                _ => 86400,
            };
            tx.execute("UPDATE asset_cleanup_research SET state='retry',error=?3,next_at=?4,claim_token='' WHERE id=?1 AND claim_token=?2",params![id,k.token,e.chars().take(2000).collect::<String>(),now()+delay]).map_err(err)?;
        }
    }
    tx.commit().map_err(err)?;
    work::reclassify(c, k, &key)?;
    c.execute(
        "UPDATE asset_cleanup_runs SET phase='researching' WHERE id=?1 AND claim_token=?2",
        params![k.id, k.token],
    )
    .map_err(err)?;
    let _ = path;
    Ok(true)
}

pub(super) fn organization(
    path: &Path,
    c: &mut Connection,
    k: &work::Claim,
) -> Result<bool, String> {
    let existing:Option<(String,i64)>=c.query_row("SELECT state,next_at FROM asset_cleanup_research WHERE run_id=?1 AND kind='organization'",[&k.id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(err)?;
    if existing
        .as_ref()
        .is_some_and(|(s, t)| s == "done" || *t > now())
    {
        return Ok(false);
    }
    let target: String = c
        .query_row(
            "SELECT organization FROM asset_cleanup_runs WHERE id=?1",
            [&k.id],
            |r| r.get(0),
        )
        .map_err(err)?;
    let settings = crate::model_configuration::default_settings_for(c, &["network"]);
    let proxy = settings["proxyUrl"].as_str().unwrap_or("");
    c.execute("INSERT OR IGNORE INTO asset_cleanup_research(run_id,group_key,kind,target) VALUES(?1,'__organization','organization',?2)",params![k.id,target]).map_err(err)?;
    let response = organization_evidence::refresh(path, k.p, &target, proxy)?;
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    if !work::claim_valid(&tx, k)? {
        return Ok(true);
    }
    let retry = response["retryAt"].as_i64().unwrap_or(0);
    tx.execute("UPDATE asset_cleanup_research SET state=?2,result_json=?3,error=?4,next_at=?5,attempts=attempts+1 WHERE run_id=?1 AND kind='organization'",params![k.id,if retry>now(){"retry"}else{"done"},response.to_string(),response["error"].as_str().unwrap_or(""),retry]).map_err(err)?;
    let target = review::scope::target_context(&tx, k.p, &target)?;
    tx.execute(
        "UPDATE asset_cleanup_runs SET target_names_json=?2 WHERE id=?1",
        params![k.id, json!(target.names).to_string()],
    )
    .map_err(err)?;
    tx.commit().map_err(err)?;
    Ok(true)
}
