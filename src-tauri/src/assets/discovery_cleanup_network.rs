use super::*;
use std::io::Read as _;
fn fetch_json(
    client: &reqwest::blocking::Client,
    url: reqwest::Url,
    max: u64,
    dns: bool,
) -> Result<(Value, String), String> {
    let mut request = client.get(url);
    if dns {
        // Both fixed public DNS endpoints use the existing configured client
        // and route. Bound the entire fallback pair below the owner's lease.
        request = request
            .header(reqwest::header::ACCEPT, "application/dns-json")
            .timeout(std::time::Duration::from_secs(12));
    }
    let r = request
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
            let (v, sha) = fetch_json(client, url, 16 * 1024 * 1024, false)?;
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
            let cached = cached_certificate(c, &k.id, target)?;
            let cache_hit = cached.is_some();
            let v = match cached {
                Some(v) => v,
                None => passive_evidence::execute(client, &item)?,
            };
            certificate_result(c, k, key, target, &v, cache_hit)
        }
        "dns_cname" => dns_with(target, |url| fetch_json(client, url, 2 * 1024 * 1024, true)),
        _ => Err("cleanup_research_kind_invalid".into()),
    }
}
// A CT ID identifies immutable certificate bytes. Only a successful, bound
// parse from this run is reusable; a previous group's host/ownership facts are
// deliberately not read. Older incomplete receipts simply fetch as before.
fn cached_certificate(c: &Connection, run: &str, target: &str) -> Result<Option<Value>, String> {
    let mut query = c.prepare("SELECT result_json FROM asset_cleanup_research WHERE run_id=?1 AND kind='ct_certificate' AND target=?2 AND state='done' ORDER BY id").map_err(err)?;
    for row in query
        .query_map(params![run, target], |r| r.get::<_, String>(0))
        .map_err(err)?
    {
        let Ok(saved) = serde_json::from_str::<Value>(&row.map_err(err)?) else {
            continue;
        };
        if let Some(value) = reusable_certificate(&saved, target) {
            return Ok(Some(value));
        }
    }
    Ok(None)
}
fn reusable_certificate(saved: &Value, target: &str) -> Option<Value> {
    let id = target.parse::<i64>().ok().filter(|id| *id > 0)?;
    let expected_source = format!("https://crt.sh/?d={id}");
    let sha = saved["bodySha256"].as_str()?;
    let precertificate = saved["precertificate"].as_bool()?;
    if saved["certificateId"].as_str() != Some(target)
        || saved["sourceUrl"].as_str() != Some(expected_source.as_str())
        || sha.len() != 64
        || !sha.bytes().all(|b| b.is_ascii_hexdigit())
        || !saved["certificateFetchedAt"]
            .as_i64()
            .is_some_and(|t| t > 0)
    {
        return None;
    }
    if !precertificate {
        let before = saved["notBefore"].as_i64()?;
        let after = saved["notAfter"].as_i64()?;
        if after <= before
            || !saved["names"].as_array()?.iter().all(Value::is_string)
            || !saved["organizations"]
                .as_array()?
                .iter()
                .all(Value::is_string)
        {
            return None;
        }
    }
    Some(
        json!({"certificateId":id,"sourceUrl":expected_source,"sha256":sha,"names":saved["names"],"organizations":saved["organizations"],"notBefore":saved["notBefore"],"notAfter":saved["notAfter"],"precertificate":precertificate,"certificateType":saved["certificateType"],"certificateFetchedAt":saved["certificateFetchedAt"]}),
    )
}
fn certificate_result(
    c: &Connection,
    k: &work::Claim,
    key: &str,
    target: &str,
    v: &Value,
    cache_hit: bool,
) -> Result<Value, String> {
    let checked = now();
    let fetched = v["certificateFetchedAt"].as_i64().unwrap_or(checked);
    let current = v["notBefore"].as_i64().is_some_and(|t| t <= checked)
        && v["notAfter"].as_i64().is_some_and(|t| t >= checked)
        && v["precertificate"] != true;
    let names = v["names"].as_array().cloned().unwrap_or_default();
    let hosts:Vec<String>=c.prepare("SELECT DISTINCT host FROM asset_cleanup_seeds WHERE run_id=?1 AND group_key=?2 AND source!='certificate_sha256'").map_err(err)?.query_map(params![k.id,key],|r|r.get(0)).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;
    let mut facts = Vec::new();
    if current {
        for candidate in hosts.iter().filter_map(|h| host(h)) {
            let pattern_only:bool=c.query_row("SELECT count(*)>0 AND min(wildcard)=1 FROM asset_cleanup_seeds WHERE run_id=?1 AND group_key=?2 AND host=?3",params![k.id,key,candidate],|r|r.get(0)).map_err(err)?;
            if !covers_saved_name(&names, &candidate, pattern_only) {
                continue;
            }
            for organization in v["organizations"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                facts.push(json!({"host":candidate,"organization":organization,"source":"public_ct_subject","wildcard":pattern_only,"certificateSha256":v["sha256"],"sourceUrl":v["sourceUrl"],"checkedAt":fetched,"reboundAt":checked,"cacheHit":cache_hit,"notAfter":v["notAfter"]}));
            }
        }
    }
    Ok(
        json!({"kind":"ct_certificate","certificateId":target,"current":current,"facts":facts,"names":names,"organizations":v["organizations"],"notBefore":v["notBefore"],"notAfter":v["notAfter"],"precertificate":v["precertificate"]==true,"sourceUrl":v["sourceUrl"],"bodySha256":v["sha256"],"checkedAt":checked,"certificateFetchedAt":fetched,"certificateType":v["certificateType"],"cacheHit":cache_hit,"networkRequested":!cache_hit,"hunterRequests":0}),
    )
}
// Public DNS is supplementary relationship evidence, never an ownership
// decision. A provider error is retried through one independent fixed resolver,
// without disabling DNSSEC, following candidate URLs or changing VPN/proxy.
fn dns_with(
    target: &str,
    mut transport: impl FnMut(reqwest::Url) -> Result<(Value, String), String>,
) -> Result<Value, String> {
    if host(target).as_deref() != Some(target) {
        return Err("DNS候选不是规范域名，未发出请求".into());
    }
    let mut attempts = Vec::new();
    let mut errors = Vec::new();
    for endpoint in [
        "https://dns.google/resolve",
        "https://cloudflare-dns.com/dns-query",
    ] {
        let mut url = reqwest::Url::parse(endpoint).expect("fixed public DNS source");
        url.query_pairs_mut()
            .append_pair("name", target)
            .append_pair("type", "5");
        if url.host_str() == Some("dns.google") {
            url.query_pairs_mut()
                .append_pair("edns_client_subnet", "0.0.0.0/0");
        }
        let source = url.to_string();
        let provider = url.host_str().unwrap_or_default().to_owned();
        let checked = now();
        let response = transport(url);
        let error = match response {
            Ok((v, sha)) => match parse_dns(&v, target) {
                Ok(records) => {
                    attempts.push(json!({"provider":provider,"sourceUrl":source,"checkedAt":checked,"bodySha256":sha,"dnsStatus":v["Status"],"truncated":v["TC"]==true,"state":"done"}));
                    return Ok(
                        json!({"kind":"dns_cname","sourceUrl":source,"bodySha256":sha,"checkedAt":checked,"dnsStatus":v["Status"],"records":records,"facts":[],"sourceAttempts":attempts,"complete":true,"ownershipConclusion":false,"hunterRequests":0}),
                    );
                }
                Err(error) => {
                    attempts.push(json!({"provider":provider,"sourceUrl":source,"checkedAt":checked,"bodySha256":sha,"dnsStatus":v["Status"],"truncated":v["TC"]==true,"state":"unavailable","error":error}));
                    error
                }
            },
            Err(error) => {
                attempts.push(json!({"provider":provider,"sourceUrl":source,"checkedAt":checked,"state":"unavailable","error":error}));
                error
            }
        };
        errors.push(format!("{provider}: {error}"));
    }
    Err(format!(
        "DNS补证未完成（已按现有网络依次尝试两个解析服务）：{}；保留候选，解析失败不代表资产不存在或无关",
        errors.join("；")
    ))
}
pub(super) fn parse_dns(v: &Value, target: &str) -> Result<Vec<Value>, String> {
    let status = v["Status"]
        .as_i64()
        .ok_or("DNS响应缺少有效Status，未采纳")?;
    if v.get("TC").is_some_and(|value| !value.is_boolean()) {
        return Err("DNS响应TC字段无效，未采纳为完整结果".into());
    }
    if v["TC"] == true {
        return Err(format!(
            "DNS响应被截断（TC=true，Status={status}），未采纳为完整结果"
        ));
    }
    if !matches!(status, 0 | 3) {
        let reason = match status {
            1 => "FORMERR（解析服务无法理解请求）",
            2 => "SERVFAIL（上游解析或DNSSEC校验失败）",
            4 => "NOTIMP（解析服务不支持请求）",
            5 => "REFUSED（解析服务拒绝查询）",
            _ => "未知DNS错误",
        };
        return Err(format!("DNS Status={status} {reason}；未采纳为完整结果"));
    }
    let questions = v["Question"].as_array().ok_or("DNS响应缺少请求绑定")?;
    if questions.len() != 1
        || questions[0]["type"] != 5
        || questions[0]["name"].as_str().and_then(host).as_deref() != Some(target)
    {
        return Err("DNS响应Question与本次候选不匹配".into());
    }
    if v.get("Answer").is_some_and(|value| !value.is_array()) {
        return Err("DNS响应Answer不是记录数组，未采纳为完整结果".into());
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
        if let Some(previous) = mapping.insert(name, next.clone()) {
            if previous != next {
                return Err("DNS CNAME同名返回多个目标，保留冲突".into());
            }
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

#[cfg(test)]
#[path = "discovery_cleanup_network_tests.rs"]
mod tests;
fn ensure_provider_backoffs(c: &Connection) -> Result<(), String> {
    // Additive recovery for an already-running cleanup created by an older
    // build. Status reads stay read-only; the execution owner migrates before
    // selecting any due network work, including newly enqueued rows.
    if !has(c, "asset_cleanup_provider_backoffs")? {
        c.execute_batch("CREATE TABLE IF NOT EXISTS asset_cleanup_provider_backoffs(run_id TEXT NOT NULL,provider TEXT NOT NULL,next_at INTEGER NOT NULL,error TEXT NOT NULL DEFAULT '',updated_at INTEGER NOT NULL,PRIMARY KEY(run_id,provider));").map_err(err)?;
    }
    c.execute_batch("CREATE INDEX IF NOT EXISTS asset_cleanup_certificate_reuse ON asset_cleanup_research(run_id,kind,target,state);").map_err(err)?;
    Ok(())
}
pub(super) fn transient_gateway_failure(error: &str) -> bool {
    let message = error.to_ascii_lowercase();
    !["http 403", "http 429"]
        .iter()
        .any(|code| message.contains(code))
        && ["http 502", "http 503", "http 504"]
            .iter()
            .any(|code| message.contains(code))
}
fn provider_retry_delay(error: &str, target_delay: i64) -> i64 {
    // One unavailable certificate may keep its own 24h retry after repeated
    // failures. It must not park every other certificate/provider query for a
    // full day. Refusal/rate-limit policy remains separate and unchanged.
    if transient_gateway_failure(error) {
        target_delay.clamp(300, 900)
    } else {
        target_delay.max(300)
    }
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
        "http 502",
        "http 503",
        "http 504",
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
#[allow(clippy::type_complexity)]
fn cached_research(
    c: &Connection,
    k: &work::Claim,
) -> Result<Option<((i64, String, String, String, i64), Value)>, String> {
    if !work::claim_valid(c, k)? {
        return Ok(None);
    }
    ensure_provider_backoffs(c)?;
    // Local reuse is independent of a provider's current availability and does
    // not consume a request attempt. Keep the circuit breaker for all GETs.
    let mut query = c.prepare("SELECT t.id,t.group_key,t.target,t.attempts,s.result_json FROM asset_cleanup_research t JOIN asset_cleanup_groups g ON g.run_id=t.run_id AND g.group_key=t.group_key JOIN asset_cleanup_research s ON s.run_id=t.run_id AND s.kind=t.kind AND s.target=t.target AND s.state='done' WHERE t.run_id=?1 AND t.kind='ct_certificate' AND t.state IN ('pending','retry') AND g.state='evidence' AND CASE WHEN json_valid(s.result_json) THEN json_type(s.result_json,'$.certificateFetchedAt')='integer' ELSE 0 END ORDER BY t.id,s.id LIMIT 64").map_err(err)?;
    for row in query
        .query_map([&k.id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
            ))
        })
        .map_err(err)?
    {
        let (id, key, target, attempts, text) = row.map_err(err)?;
        let Ok(saved) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        if let Some(v) = reusable_certificate(&saved, &target) {
            return Ok(Some((
                (id, key, "ct_certificate".into(), target, attempts),
                v,
            )));
        }
    }
    Ok(None)
}
pub(super) fn one(path: &Path, c: &mut Connection, k: &work::Claim) -> Result<bool, String> {
    let (row, cached) = if let Some((row, v)) = cached_research(c, k)? {
        (row, Some(v))
    } else {
        let Some(row) = next_research(c, k)? else {
            return Ok(false);
        };
        (row, None)
    };
    let (id, key, kind, target, attempts) = row;
    let client = if cached.is_none() {
        let settings = crate::model_configuration::default_settings_for(c, &["network"]);
        if settings.get("_configurationError").is_some() {
            return Err("cleanup_network_configuration_unavailable".into());
        }
        let proxy = settings["proxyUrl"].as_str().unwrap_or("");
        Some(passive_evidence::client(proxy)?)
    } else {
        None
    };
    c.execute("UPDATE asset_cleanup_research SET state='running',claim_token=?2,attempts=attempts+?3 WHERE id=?1",params![id,k.token,i64::from(cached.is_none())]).map_err(err)?;
    let outcome = match cached {
        Some(v) => certificate_result(c, k, &key, &target, &v, true),
        None => research(
            client
                .as_ref()
                .expect("network client created for public GET"),
            c,
            k,
            &key,
            &kind,
            &target,
        ),
    };
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
            provider_backoff(
                &tx,
                &k.id,
                &kind,
                &e,
                now() + provider_retry_delay(&e, delay),
            )?;
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
