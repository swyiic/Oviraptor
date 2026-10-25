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
pub(super) fn indexed_ids(v: &Value, target: &str) -> Result<Vec<i64>, String> {
    let rows = v.as_array().ok_or("CT索引不是数组，未采纳")?;
    if rows.len() >= 10_000 {
        return Err("CT索引已到平台上限，保留未完成状态并继续具体主机补证".into());
    }
    let mut ids = BTreeSet::new();
    for row in rows {
        let relevant = row["name_value"]
            .as_str()
            .into_iter()
            .flat_map(str::lines)
            .any(|name| {
                host(name).is_some_and(|n| n == target || in_scope(&n, target))
                    || enrichment::certificate_covers(target, name)
            });
        if !relevant {
            continue;
        }
        let id = row["id"]
            .as_i64()
            .or_else(|| row["min_cert_id"].as_i64())
            .filter(|id| *id > 0)
            .ok_or("CT匹配记录缺少有效证书编号，未采纳为完整响应")?;
        ids.insert(id);
    }
    Ok(ids.into_iter().collect())
}
pub(super) fn covers_saved_name(names: &[Value], candidate: &str, pattern_only: bool) -> bool {
    names.iter().filter_map(Value::as_str).any(|name| {
        enrichment::certificate_covers(candidate, name)
            || (pattern_only && name.starts_with("*.") && host(name).as_deref() == Some(candidate))
    })
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
            let pattern_only:bool=c.query_row("SELECT count(*)>0 AND min(wildcard)=1 FROM asset_cleanup_seeds WHERE run_id=?1 AND group_key=?2 AND host=?3",params![k.id,key,target],|r|r.get(0)).map_err(err)?;
            let mut url = reqwest::Url::parse("https://crt.sh/").unwrap();
            let query = if pattern_only {
                format!("%.{target}")
            } else {
                target.to_owned()
            };
            url.query_pairs_mut()
                .append_pair("q", &query)
                .append_pair("output", "json")
                .append_pair("exclude", "expired");
            let source = url.to_string();
            let (v, sha) = fetch_json(client, url, 16 * 1024 * 1024)?;
            let ids = indexed_ids(&v, target)?;
            Ok(
                json!({"kind":kind,"certificateIds":ids,"sourceUrl":source,"checkedAt":now(),"bodySha256":sha,"complete":false,"coverage":"public_index_response","hunterRequests":0}),
            )
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
                    let pattern_only:bool=c.query_row("SELECT count(*)>0 AND min(wildcard)=1 FROM asset_cleanup_seeds WHERE run_id=?1 AND group_key=?2 AND host=?3",params![k.id,key,candidate],|r|r.get(0)).map_err(err)?;
                    let covers = covers_saved_name(&names, &candidate, pattern_only);
                    if !covers {
                        continue;
                    }
                    for organization in v["organizations"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                    {
                        facts.push(json!({"host":candidate,"organization":organization,"source":"public_ct_subject","wildcard":pattern_only,"certificateSha256":v["sha256"],"sourceUrl":v["sourceUrl"],"checkedAt":now(),"notAfter":v["notAfter"]}));
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
fn ensure_provider_backoffs(c: &Connection) -> Result<(), String> {
    // Additive recovery for an already-running cleanup created by an older
    // build. Status reads stay read-only; the execution owner migrates before
    // selecting any due network work, including newly enqueued rows.
    if !has(c, "asset_cleanup_provider_backoffs")? {
        c.execute_batch("CREATE TABLE IF NOT EXISTS asset_cleanup_provider_backoffs(run_id TEXT NOT NULL,provider TEXT NOT NULL,next_at INTEGER NOT NULL,error TEXT NOT NULL DEFAULT '',updated_at INTEGER NOT NULL,PRIMARY KEY(run_id,provider));").map_err(err)?;
    }
    Ok(())
}
pub(super) fn provider_backoff(
    c: &Connection,
    run: &str,
    kind: &str,
    error: &str,
    until: i64,
) -> Result<(), String> {
    let message = error.to_ascii_lowercase();
    if ![
        "http 403",
        "http 429",
        "请求失败",
        "timeout",
        "timed out",
        "不是有效json",
    ]
    .iter()
    .any(|code| message.contains(code))
    {
        return Ok(());
    }
    // Provider refusal or an unavailable current route is not 30,000 separate
    // failed hosts. Back off that provider without touching VPN or DNS results.
    let provider = if kind.starts_with("ct_") { "ct" } else { "dns" };
    ensure_provider_backoffs(c)?;
    // The provider deadline is independent of the current queue contents. A
    // later local page or CT index response may append work with next_at=0;
    // that work must still honor this same run/provider-wide circuit breaker.
    // Shorter subsequent failures cannot shorten a previously longer delay.
    c.execute("INSERT INTO asset_cleanup_provider_backoffs(run_id,provider,next_at,error,updated_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(run_id,provider) DO UPDATE SET next_at=MAX(asset_cleanup_provider_backoffs.next_at,excluded.next_at),error=CASE WHEN excluded.next_at>=asset_cleanup_provider_backoffs.next_at THEN excluded.error ELSE asset_cleanup_provider_backoffs.error END,updated_at=excluded.updated_at",params![run,provider,until,error.chars().take(2000).collect::<String>(),now()]).map_err(err)?;
    c.execute("UPDATE asset_cleanup_research SET next_at=MAX(next_at,?3) WHERE run_id=?1 AND state IN ('pending','retry') AND ((?2='ct' AND kind IN ('ct_search','ct_certificate')) OR (?2='dns' AND kind='dns_cname'))",params![run,provider,until]).map_err(err)?;
    Ok(())
}
/// Select exactly the next production request without creating a client or
/// contacting a provider. Also owns the additive deadline-table migration.
#[allow(clippy::type_complexity)]
pub(super) fn next_research(
    c: &Connection,
    k: &work::Claim,
) -> Result<Option<(i64, String, String, String, i64)>, String> {
    if !work::claim_valid(c, k)? {
        return Ok(None);
    }
    ensure_provider_backoffs(c)?;
    c.query_row("SELECT t.id,t.group_key,t.kind,t.target,t.attempts FROM asset_cleanup_research t JOIN asset_cleanup_groups g ON g.run_id=t.run_id AND g.group_key=t.group_key WHERE t.run_id=?1 AND t.state IN ('pending','retry') AND t.next_at<=?2 AND g.state='evidence' AND NOT EXISTS(SELECT 1 FROM asset_cleanup_provider_backoffs b WHERE b.run_id=t.run_id AND b.provider=CASE WHEN t.kind IN ('ct_search','ct_certificate') THEN 'ct' WHEN t.kind='dns_cname' THEN 'dns' ELSE 'other' END AND b.next_at>?2) ORDER BY CASE g.advice WHEN 'exclude' THEN 0 ELSE 1 END,CASE t.kind WHEN 'ct_certificate' THEN 0 WHEN 'ct_search' THEN 1 ELSE 2 END,t.id LIMIT 1",params![k.id,now()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(err)
}
pub(super) fn one(path: &Path, c: &mut Connection, k: &work::Claim) -> Result<bool, String> {
    let Some((id, key, kind, target, attempts)) = next_research(c, k)? else {
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
            provider_backoff(&tx, &k.id, &kind, &e, now() + delay.max(300))?;
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
