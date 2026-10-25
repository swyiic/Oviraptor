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
    c.execute_batch("CREATE TABLE IF NOT EXISTS asset_cleanup_dispatch(run_id TEXT PRIMARY KEY,cursor INTEGER NOT NULL DEFAULT 0);CREATE INDEX IF NOT EXISTS asset_cleanup_research_target ON asset_cleanup_research(run_id,kind,target,state,attempts);").map_err(err)?;
    c.execute_batch("CREATE TABLE IF NOT EXISTS asset_cleanup_group_dispatch(run_id TEXT NOT NULL,group_key TEXT NOT NULL,last_dispatch INTEGER NOT NULL,PRIMARY KEY(run_id,group_key));").map_err(err)?;
    c.execute_batch("CREATE TABLE IF NOT EXISTS asset_cleanup_source_failures(run_id TEXT NOT NULL,provider TEXT NOT NULL,failures INTEGER NOT NULL,error TEXT NOT NULL,checked_at INTEGER NOT NULL,PRIMARY KEY(run_id,provider));").map_err(err)?;
    Ok(())
}
pub(super) const MAX_PUBLIC_ATTEMPTS: i64 = 2;
const PUBLIC_BATCH: usize = 12;
const KINDS: [&str; 4] = [
    "public_ownership",
    "ct_certificate",
    "ct_search",
    "dns_cname",
];
type ResearchRow = (i64, String, String, String, i64);

/// A shared platform's bare root is not a tenant. Old snapshots may still have
/// these groups, but downloading every platform certificate cannot establish a
/// customer's ownership. Keep the original receipts and stop this expansion.
fn shared_root(name: &str) -> bool {
    [
        "cmecloud.cn",
        "myqcloud.com",
        "aliyuncs.com",
        "cloudfront.net",
        "akamaiedge.net",
        "akadns.net",
        "azureedge.net",
        "chinacloudsites.cn",
        "51job.com",
        "csdn.net",
        "163.com",
        "139site.com",
    ]
    .contains(&name)
}

/// Terminal means this finite evidence pass is exhausted, never unrelated.
/// A new explicit pass or genuinely new local evidence can reopen it; status
/// polling and a cold restart must not reset attempts or paid model receipts.
pub(super) fn settle_exhausted(c: &mut Connection, k: &work::Claim) -> Result<bool, String> {
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    if !work::claim_valid(&tx, k)? {
        return Ok(false);
    }
    ensure_provider_backoffs(&tx)?;
    // Older versions charged an unadopted, interrupted public GET as a failed
    // attempt and sometimes inherited a provider's day-long retry deadline.
    // Repair only those two exact receipts, once, under this owner generation.
    // Ordinary HTTP/DNS failures and source 403/429 backoffs remain untouched.
    let legacy_interruptions=tx.execute(
        "UPDATE asset_cleanup_research SET attempts=MAX(attempts-1,0),next_at=MIN(next_at,?2),claim_token='',error='历史公开只读请求中断，原回执已保留；已退还本次未采纳的补证次数，继续本轮查证',result_json=json_set(CASE WHEN json_valid(result_json) AND substr(ltrim(result_json),1,1)='{' THEN result_json ELSE '{}' END,'$.legacyInterruptionReceipt',json_object('error',error,'nextAt',next_at,'attempts',attempts,'normalizedAt',?2,'refundedAttempts',CASE WHEN attempts>0 THEN 1 ELSE 0 END,'previousResultJson',result_json),'$.cancelledRequests',MAX(COALESCE(CAST(json_extract(CASE WHEN json_valid(result_json) THEN result_json ELSE '{}' END,'$.cancelledRequests') AS INTEGER),0),0)+1,'$.lastCancellationAt',?2) WHERE run_id=?1 AND kind IN ('public_ownership','ct_certificate','ct_search','dns_cname') AND state='retry' AND error IN ('公开只读请求中断，保存后退避重试','公开只读请求在暂停或中断后返回，结果未采纳；保留断点') AND json_type(CASE WHEN json_valid(result_json) THEN result_json ELSE '{}' END,'$.legacyInterruptionReceipt') IS NULL",
        params![k.id,now()],
    ).map_err(err)?;
    let organization_settled=tx.execute("UPDATE asset_cleanup_research SET state='exhausted',claim_token='',error=CASE WHEN error='' THEN 'organization_source_unavailable：本轮单位关系来源补证次数已用尽；已接受的单位与原回执保留，不代表法定名单完整' ELSE error||'；organization_source_unavailable：本轮单位关系补证次数已用尽，已接受单位保留' END WHERE run_id=?1 AND kind='organization' AND state IN ('pending','retry') AND attempts>=?2",params![k.id,MAX_PUBLIC_ATTEMPTS]).map_err(err)?;
    let candidates:Vec<(i64,String,String,String,i64)>=tx.prepare("SELECT t.id,t.group_key,t.kind,g.domain,t.attempts FROM asset_cleanup_research t JOIN asset_cleanup_groups g ON g.run_id=t.run_id AND g.group_key=t.group_key WHERE t.run_id=?1 AND t.state IN ('pending','retry') AND g.state='evidence' AND (t.attempts>=?2 OR t.kind IN ('ct_search','ct_certificate'))").map_err(err)?.query_map(params![k.id,MAX_PUBLIC_ATTEMPTS],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;
    let mut changed = organization_settled + legacy_interruptions;
    let refused:Vec<(String,String)>=tx.prepare("SELECT provider,error FROM asset_cleanup_provider_backoffs WHERE run_id=?1 AND (lower(error) LIKE '%http 403%' OR lower(error) LIKE '%http 429%') AND next_at>?2").map_err(err)?.query_map(params![k.id,now()],|r|Ok((r.get(0)?,r.get(1)?))).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;
    for (provider, error) in refused {
        changed += exhaust_provider(&tx, &k.id, &provider, &error)?;
    }
    for (id, _, kind, domain, attempts) in candidates {
        let platform =
            shared_root(&domain) && matches!(kind.as_str(), "ct_search" | "ct_certificate");
        if platform || attempts >= MAX_PUBLIC_ATTEMPTS {
            let reason = if platform {
                "共享平台根域不是客户归属证据，本轮停止全平台 CT 扩展；历史结果保留"
            } else {
                "本轮公开来源补证次数已用尽；保留失败原文和候选，待新证据再处理，不代表无关"
            };
            changed+=tx.execute("UPDATE asset_cleanup_research SET state='exhausted',claim_token='',error=CASE WHEN error='' THEN ?2 ELSE error||'；'||?2 END WHERE id=?1 AND state IN ('pending','retry')",params![id,reason]).map_err(err)?;
        }
    }
    // Historic builds queried every saved hostname independently. An ownership
    // pass needs one bounded family index, not a rediscovery crawl of each SAN.
    let families:Vec<(String,String)>=tx.prepare("SELECT DISTINCT g.group_key,g.domain FROM asset_cleanup_groups g JOIN asset_cleanup_research t ON t.run_id=g.run_id AND t.group_key=g.group_key WHERE g.run_id=?1 AND g.state='evidence' AND g.domain!='' AND t.kind='ct_search' AND t.state IN ('pending','retry') AND t.target!=g.domain").map_err(err)?.query_map([&k.id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;
    for (key, domain) in families {
        if !shared_root(&domain) {
            tx.execute("INSERT OR IGNORE INTO asset_cleanup_research(run_id,group_key,kind,target) VALUES(?1,?2,'ct_search',?3)",params![k.id,key,domain]).map_err(err)?;
        }
        changed+=tx.execute("UPDATE asset_cleanup_research SET state='exhausted',claim_token='',error=CASE WHEN error='' THEN 'duplicate_family_index：已合并为一个域名族 CT 索引，原记录保留' ELSE error||'；duplicate_family_index：已合并为一个域名族 CT 索引' END WHERE run_id=?1 AND group_key=?2 AND kind='ct_search' AND target!=?3 AND state IN ('pending','retry')",params![k.id,key,domain]).map_err(err)?;
    }
    // A CT index can contain thousands of certificates. Keep its full ID list,
    // read at most five recent leaves for this relevance pass, and report the
    // rest as a bounded evidence gap rather than a negative ownership result.
    let overflow:Vec<i64>=tx.prepare("SELECT id FROM (SELECT t.id,ROW_NUMBER() OVER(PARTITION BY t.group_key ORDER BY CAST(t.target AS INTEGER) DESC,t.id DESC) n,(SELECT count(*) FROM asset_cleanup_research d WHERE d.run_id=t.run_id AND d.group_key=t.group_key AND d.kind='ct_certificate' AND d.state='done') done FROM asset_cleanup_research t JOIN asset_cleanup_groups g ON g.run_id=t.run_id AND g.group_key=t.group_key WHERE t.run_id=?1 AND t.kind='ct_certificate' AND t.state IN ('pending','retry') AND g.state='evidence') WHERE n+done>5").map_err(err)?.query_map([&k.id],|r|r.get(0)).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;
    for id in overflow {
        changed+=tx.execute("UPDATE asset_cleanup_research SET state='exhausted',claim_token='',error=CASE WHEN error='' THEN 'certificate_sample_limit：本轮仅读取近期最多 5 张证书，完整索引与未读取编号保留；不作为无关证据' ELSE error||'；certificate_sample_limit：本轮证书样本上限，未读取记录保留' END WHERE id=?1 AND state IN ('pending','retry')",[id]).map_err(err)?;
    }
    tx.commit().map_err(err)?;
    Ok(changed > 0)
}
fn exhaust_provider(
    c: &Connection,
    run: &str,
    provider: &str,
    error: &str,
) -> Result<usize, String> {
    let reason = format!(
        "source_blocked：本轮公开来源 {provider} 不可用；{}；其他来源继续，原候选保留，未推断无关",
        error.chars().take(1400).collect::<String>()
    );
    c.execute("UPDATE asset_cleanup_research SET state='exhausted',claim_token='',error=CASE WHEN error='' THEN ?3 ELSE error||'；'||?3 END WHERE run_id=?1 AND state IN ('pending','retry') AND ((?2='ct' AND kind IN ('ct_search','ct_certificate')) OR (?2='dns' AND kind='dns_cname'))",params![run,provider,reason]).map_err(err)
}
fn record_source_failure(
    c: &Connection,
    k: &work::Claim,
    kind: &str,
    error: &str,
) -> Result<(), String> {
    if !matches!(kind, "ct_search" | "ct_certificate" | "dns_cname") {
        return Ok(());
    }
    let message = error.to_ascii_lowercase();
    let refusal = message.contains("http 403") || message.contains("http 429");
    if kind == "dns_cname"
        && (message.contains("dns status=") || message.contains("tc=true, status="))
        && !refusal
    {
        return Ok(());
    }
    if !refusal
        && !transient_gateway_failure(error)
        && !["请求失败", "timeout", "timed out", "不是有效json"]
            .iter()
            .any(|x| message.contains(x))
    {
        return Ok(());
    }
    let provider = if kind.starts_with("ct_") { "ct" } else { "dns" };
    c.execute("INSERT INTO asset_cleanup_source_failures(run_id,provider,failures,error,checked_at) VALUES(?1,?2,1,?3,?4) ON CONFLICT(run_id,provider) DO UPDATE SET failures=failures+1,error=excluded.error,checked_at=excluded.checked_at",params![k.id,provider,error.chars().take(2000).collect::<String>(),now()]).map_err(err)?;
    let count: i64 = c
        .query_row(
            "SELECT failures FROM asset_cleanup_source_failures WHERE run_id=?1 AND provider=?2",
            params![k.id, provider],
            |r| r.get(0),
        )
        .map_err(err)?;
    if refusal || count >= 2 {
        exhaust_provider(c, &k.id, provider, error)?;
    }
    Ok(())
}
fn record_source_success(c: &Connection, k: &work::Claim, kind: &str) -> Result<(), String> {
    if !matches!(kind, "ct_search" | "ct_certificate" | "dns_cname") {
        return Ok(());
    }
    let provider = if kind.starts_with("ct_") { "ct" } else { "dns" };
    let previous: Option<String> = c
        .query_row(
            "SELECT error FROM asset_cleanup_source_failures WHERE run_id=?1 AND provider=?2",
            params![k.id, provider],
            |r| r.get(0),
        )
        .optional()
        .map_err(err)?;
    c.execute("UPDATE asset_cleanup_source_failures SET failures=0,checked_at=?3 WHERE run_id=?1 AND provider=?2",params![k.id,provider,now()]).map_err(err)?;
    // A successful response in the same batch proves a transient outage has
    // recovered. Reopen work with remaining budget, retaining cooldowns and receipts;
    // never override an explicit refusal/rate limit or a spent target budget.
    if previous.is_some_and(|e| {
        let e = e.to_ascii_lowercase();
        !e.contains("http 403") && !e.contains("http 429")
    }) {
        c.execute("UPDATE asset_cleanup_research SET state=CASE WHEN attempts=0 THEN 'pending' ELSE 'retry' END,error=error||'；该来源已返回有效结果，尚有次数的请求恢复本轮排队' WHERE run_id=?1 AND state='exhausted' AND attempts<?3 AND error LIKE '%source_blocked：%' AND lower(error) NOT LIKE '%http 403%' AND lower(error) NOT LIKE '%http 429%' AND error NOT LIKE '%certificate_sample_limit%' AND error NOT LIKE '%duplicate_family_index%' AND ((?2='ct' AND kind IN ('ct_search','ct_certificate')) OR (?2='dns' AND kind='dns_cname'))",params![k.id,provider,MAX_PUBLIC_ATTEMPTS]).map_err(err)?;
    }
    Ok(())
}

/// Remaining potentially useful requests, including a due provider cooldown.
/// Call settle_exhausted first; only truly exhausted work leaves the count.
pub(super) fn runnable_count(c: &Connection, k: &work::Claim) -> Result<i64, String> {
    c.query_row("SELECT count(*) FROM asset_cleanup_research t JOIN asset_cleanup_groups g ON g.run_id=t.run_id AND g.group_key=t.group_key WHERE t.run_id=?1 AND t.state IN ('pending','retry','running') AND g.state='evidence' AND (t.state='running' OR t.attempts<?2)",params![k.id,MAX_PUBLIC_ATTEMPTS],|r|r.get(0)).map_err(err)
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
    let message = error.to_ascii_lowercase();
    let limited = message.contains("http 403") || message.contains("http 429");
    let transport = ["请求失败", "timeout", "timed out", "不是有效json"]
        .iter()
        .any(|token| message.contains(token));
    if !limited && (transient_gateway_failure(error) || transport) {
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
    // A candidate website is its own source; its refusal must never park all
    // other sites. DNS SERVFAIL/TC with a responding resolver is target-level
    // failure even when the fallback resolver's route fails separately.
    if kind == "public_ownership"
        || (kind == "dns_cname"
            && (message.contains("dns status=") || message.contains("tc=true, status="))
            && !message.contains("http 403")
            && !message.contains("http 429"))
    {
        return Ok(());
    }
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
    Ok(batch_rows(c, k)?.into_iter().next())
}
fn batch_rows(c: &Connection, k: &work::Claim) -> Result<Vec<ResearchRow>, String> {
    if !work::claim_valid(c, k)? {
        return Ok(Vec::new());
    }
    let cursor: i64 = c
        .query_row(
            "SELECT cursor FROM asset_cleanup_dispatch WHERE run_id=?1",
            [&k.id],
            |r| r.get(0),
        )
        .optional()
        .map_err(err)?
        .unwrap_or(0);
    let mut sources = Vec::new();
    for offset in 0..KINDS.len() {
        let kind = KINDS[(cursor.rem_euclid(KINDS.len() as i64) as usize + offset) % KINDS.len()];
        // The row-number partition prevents a single root's thousands of CT
        // IDs from hiding every other family behind a small SELECT limit.
        let rows:Vec<ResearchRow>=c.prepare("SELECT id,group_key,kind,target,attempts FROM (SELECT t.id,t.group_key,t.kind,t.target,t.attempts,COALESCE(d.last_dispatch,-1) last_dispatch,ROW_NUMBER() OVER(PARTITION BY t.group_key ORDER BY t.attempts,t.id) family_row FROM asset_cleanup_research t JOIN asset_cleanup_groups g ON g.run_id=t.run_id AND g.group_key=t.group_key LEFT JOIN asset_cleanup_group_dispatch d ON d.run_id=t.run_id AND d.group_key=t.group_key WHERE t.run_id=?1 AND t.kind=?2 AND t.state IN ('pending','retry') AND t.attempts<?3 AND t.next_at<=?4 AND g.state='evidence' AND NOT EXISTS(SELECT 1 FROM asset_cleanup_provider_backoffs b WHERE b.run_id=t.run_id AND b.provider=CASE WHEN t.kind IN ('ct_search','ct_certificate') THEN 'ct' WHEN t.kind='dns_cname' THEN 'dns' ELSE 'other' END AND b.next_at>?4) AND NOT EXISTS(SELECT 1 FROM asset_cleanup_research active WHERE active.run_id=t.run_id AND active.kind=t.kind AND active.target=t.target AND active.state='running')) WHERE family_row=1 ORDER BY attempts,last_dispatch,id LIMIT 48").map_err(err)?.query_map(params![k.id,kind,MAX_PUBLIC_ATTEMPTS,now()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;
        sources.push(rows);
    }
    let first = sources.iter().flatten().any(|r| r.4 == 0);
    // Keep each bounded batch homogeneous. A fast DNS response must not wait
    // for an unrelated 28s website fallback before the owner can publish it and
    // claim the next DNS batch. Rotate kinds between batches, not within one.
    // First attempts still precede retries across every available source.
    for rows in sources {
        let mut selected = Vec::new();
        let mut groups = BTreeSet::new();
        let mut targets = BTreeSet::new();
        for row in rows {
            if (first && row.4 != 0)
                || groups.contains(&row.1)
                || targets.contains(&(row.2.clone(), row.3.clone()))
            {
                continue;
            }
            groups.insert(row.1.clone());
            targets.insert((row.2.clone(), row.3.clone()));
            selected.push(row);
            if selected.len() == PUBLIC_BATCH {
                break;
            }
        }
        if !selected.is_empty() {
            return Ok(selected);
        }
    }
    Ok(Vec::new())
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
    let mut query = c.prepare("SELECT t.id,t.group_key,t.target,t.attempts,s.result_json FROM asset_cleanup_research t JOIN asset_cleanup_groups g ON g.run_id=t.run_id AND g.group_key=t.group_key JOIN asset_cleanup_research s ON s.run_id=t.run_id AND s.kind=t.kind AND s.target=t.target AND s.state='done' WHERE t.run_id=?1 AND t.kind='ct_certificate' AND t.state IN ('pending','retry','exhausted') AND g.state='evidence' AND CASE WHEN json_valid(s.result_json) THEN json_type(s.result_json,'$.certificateFetchedAt')='integer' ELSE 0 END ORDER BY t.id,s.id LIMIT 64").map_err(err)?;
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
/// Public sites have independent health and at most four bounded requests run
/// together. Claim every row durably before any GET; no SQLite handle crosses
/// a thread boundary or transaction remains open while waiting on the network.
fn website_batch_rows(
    c: &Connection,
    k: &work::Claim,
) -> Result<Vec<(i64, String, String, i64)>, String> {
    if !work::claim_valid(c, k)? {
        return Ok(Vec::new());
    }
    c.prepare("SELECT t.id,t.group_key,t.target,t.attempts FROM asset_cleanup_research t JOIN asset_cleanup_groups g ON g.run_id=t.run_id AND g.group_key=t.group_key WHERE t.run_id=?1 AND t.kind='public_ownership' AND t.state IN ('pending','retry') AND t.next_at<=?2 AND g.state='evidence' ORDER BY CASE g.advice WHEN 'exclude' THEN 0 ELSE 1 END,t.id LIMIT 4").map_err(err)?.query_map(params![k.id,now()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(err)?.collect::<Result<_,_>>().map_err(err)
}
fn claim_website_batch(
    c: &mut Connection,
    k: &work::Claim,
) -> Result<Vec<(i64, String, String, i64)>, String> {
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    let rows = website_batch_rows(&tx, k)?;
    for (id, _, _, _) in &rows {
        tx.execute("UPDATE asset_cleanup_research SET state='running',claim_token=?2,attempts=attempts+1 WHERE id=?1 AND state IN ('pending','retry')",params![id,k.token]).map_err(err)?;
    }
    tx.commit().map_err(err)?;
    Ok(rows)
}
fn finish_website_result(
    c: &mut Connection,
    k: &work::Claim,
    id: i64,
    key: &str,
    attempts: i64,
    outcome: Result<Value, String>,
) -> Result<bool, String> {
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    if !work::claim_valid(&tx, k)? {
        tx.rollback().map_err(err)?;
        settle_public_rows(c, k, &[id])?;
        return Ok(false);
    }
    let changed=match outcome {
        Ok(v)=>tx.execute("UPDATE asset_cleanup_research SET state='done',result_json=?3,error='',claim_token='' WHERE id=?1 AND claim_token=?2 AND state='running'",params![id,k.token,v.to_string()]),
        Err(error)=>{
            let delay=match attempts {0=>60,1=>300,2=>1800,_=>86400};
            tx.execute("UPDATE asset_cleanup_research SET state='retry',error=?3,next_at=?4,claim_token='' WHERE id=?1 AND claim_token=?2 AND state='running'",params![id,k.token,error.chars().take(2000).collect::<String>(),now()+delay])
        }
    }.map_err(err)?;
    tx.commit().map_err(err)?;
    if changed > 0 {
        work::reclassify(c, k, key)?;
    }
    Ok(changed > 0)
}
fn settle_public_rows(c: &mut Connection, k: &work::Claim, ids: &[i64]) -> Result<(), String> {
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    for id in ids {
        tx.execute("UPDATE asset_cleanup_research SET state='retry',claim_token='',attempts=MAX(attempts-1,0),next_at=?4,error='公开只读请求在暂停或中断后返回，结果未采纳；保留断点并退还本次补证次数',result_json=json_set(CASE WHEN json_valid(result_json) AND substr(ltrim(result_json),1,1)='{' THEN result_json ELSE '{}' END,'$.cancelledRequests',MAX(COALESCE(CAST(json_extract(CASE WHEN json_valid(result_json) THEN result_json ELSE '{}' END,'$.cancelledRequests') AS INTEGER),0),0)+1,'$.lastCancellationAt',?6) WHERE id=?1 AND run_id=?2 AND claim_token=?3 AND state='running' AND EXISTS(SELECT 1 FROM asset_cleanup_runs r WHERE r.id=?2 AND r.project_id=?5)",params![id,k.id,k.token,now()+30,k.p,now()]).map_err(err)?;
    }
    tx.commit().map_err(err)
}
pub(super) fn one(path: &Path, c: &mut Connection, k: &work::Claim) -> Result<bool, String> {
    let _ = path;
    if drain_reusable(c, k)? {
        return Ok(true);
    }
    settle_exhausted(c, k)?;
    ensure_provider_backoffs(c)?;
    let rows = claim_public_batch(c, k)?;
    if rows.is_empty() {
        return Ok(false);
    }
    let settings = crate::model_configuration::default_settings_for(c, &["network"]);
    if settings.get("_configurationError").is_some() {
        settle_public_rows(c, k, &rows.iter().map(|r| r.0).collect::<Vec<_>>())?;
        return Err("cleanup_network_configuration_unavailable".into());
    }
    let proxy = settings["proxyUrl"].as_str().unwrap_or("").to_owned();
    let client = passive_evidence::client(&proxy)?;
    let jobs = rows
        .iter()
        .map(|r| {
            let pattern = if r.2 == "ct_search" {
                pattern_only(c, k, &r.1, &r.3)?
            } else {
                false
            };
            Ok((r.clone(), pattern))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let outcomes = std::thread::scope(|scope| {
        let handles = jobs
            .into_iter()
            .map(|(row, pattern)| {
                let client = client.clone();
                let proxy = proxy.clone();
                scope.spawn(move || {
                    let result = execute_public(&client, &proxy, &row.2, &row.3, pattern);
                    (row, result)
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .map_err(|_| "公开补证线程异常；保留断点，不推断归属".to_owned())
            })
            .collect::<Vec<_>>()
    });
    let ids = rows.iter().map(|r| r.0).collect::<Vec<_>>();
    for outcome in outcomes {
        let (row, result) = match outcome {
            Ok(v) => v,
            Err(e) => {
                settle_public_rows(c, k, &ids)?;
                return Err(e);
            }
        };
        if !finish_public(c, k, &row, result)? {
            settle_public_rows(c, k, &ids)?;
            break;
        }
    }
    c.execute("UPDATE asset_cleanup_runs SET phase='researching' WHERE id=?1 AND claim_token=?2 AND generation=?3 AND enabled=1",params![k.id,k.token,k.generation]).map_err(err)?;
    Ok(true)
}
fn claim_public_batch(c: &mut Connection, k: &work::Claim) -> Result<Vec<ResearchRow>, String> {
    ensure_provider_backoffs(c)?;
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    let rows = batch_rows(&tx, k)?;
    if !rows.is_empty() {
        for (id, _, _, _, _) in &rows {
            tx.execute("UPDATE asset_cleanup_research SET state='running',claim_token=?2,attempts=attempts+1 WHERE id=?1 AND state IN ('pending','retry') AND attempts<?3",params![id,k.token,MAX_PUBLIC_ATTEMPTS]).map_err(err)?;
        }
        // Advance after the actually selected kind, including skipped empty
        // sources. Keep the cursor monotonic because group fairness records use
        // it as their dispatch age; modulo-only writes would starve old groups.
        let cursor: i64 = tx
            .query_row(
                "SELECT cursor FROM asset_cleanup_dispatch WHERE run_id=?1",
                [&k.id],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?
            .unwrap_or(0);
        let selected_kind = KINDS
            .iter()
            .position(|kind| *kind == rows[0].2)
            .ok_or("cleanup_research_kind_invalid")? as i64;
        let next_cursor = cursor
            + (selected_kind - cursor.rem_euclid(KINDS.len() as i64))
                .rem_euclid(KINDS.len() as i64)
            + 1;
        tx.execute("INSERT INTO asset_cleanup_dispatch(run_id,cursor) VALUES(?1,?2) ON CONFLICT(run_id) DO UPDATE SET cursor=excluded.cursor",params![k.id,next_cursor]).map_err(err)?;
        for (_, key, _, _, _) in &rows {
            tx.execute("INSERT INTO asset_cleanup_group_dispatch(run_id,group_key,last_dispatch) SELECT ?1,?2,cursor FROM asset_cleanup_dispatch WHERE run_id=?1 ON CONFLICT(run_id,group_key) DO UPDATE SET last_dispatch=excluded.last_dispatch",params![k.id,key]).map_err(err)?;
        }
    }
    tx.commit().map_err(err)?;
    Ok(rows)
}
fn pattern_only(c: &Connection, k: &work::Claim, key: &str, target: &str) -> Result<bool, String> {
    c.query_row("SELECT (EXISTS(SELECT 1 FROM asset_cleanup_groups WHERE run_id=?1 AND group_key=?2 AND domain=?3) OR (count(*)>0 AND min(wildcard)=1)) FROM asset_cleanup_seeds WHERE run_id=?1 AND group_key=?2 AND host=?3",params![k.id,key,target],|r|r.get(0)).map_err(err)
}
fn execute_public(
    client: &reqwest::blocking::Client,
    proxy: &str,
    kind: &str,
    target: &str,
    pattern: bool,
) -> Result<Value, String> {
    match kind {
        "public_ownership" => ownership::execute(client, target, proxy),
        "dns_cname" => dns_with(target, |url| fetch_json(client, url, 2 * 1024 * 1024, true)),
        "ct_search" => {
            let mut url = reqwest::Url::parse("https://crt.sh/").unwrap();
            let query = if pattern {
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
                json!({"kind":kind,"certificateIds":ids,"sourceUrl":source,"bodySha256":sha,"checkedAt":now(),"requestPattern":query,"complete":false,"coverage":"public_index_response","hunterRequests":0}),
            )
        }
        "ct_certificate" => {
            let item = passive_evidence::Item {
                id: 0,
                kind: kind.into(),
                target: target.into(),
                host: String::new(),
                token: String::new(),
                attempts: 1,
            };
            passive_evidence::execute(client, &item)
        }
        _ => Err("cleanup_research_kind_invalid".into()),
    }
}
fn finish_public(
    c: &mut Connection,
    k: &work::Claim,
    row: &ResearchRow,
    outcome: Result<Value, String>,
) -> Result<bool, String> {
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    if !work::claim_valid(&tx, k)? {
        tx.rollback().map_err(err)?;
        settle_public_rows(c, k, &[row.0])?;
        return Ok(false);
    }
    let current:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM asset_cleanup_research WHERE id=?1 AND run_id=?2 AND state='running' AND claim_token=?3)",params![row.0,k.id,k.token],|r|r.get(0)).map_err(err)?;
    if !current {
        return Ok(false);
    }
    let pattern = if row.2 == "ct_search" {
        pattern_only(&tx, k, &row.1, &row.3)?
    } else {
        false
    };
    let outcome = outcome.and_then(|value| validate_public(value, &row.2, &row.3, pattern));
    let mut keys = Vec::new();
    match outcome {
        Ok(value) => {
            record_source_success(&tx, k, &row.2)?;
            let recipients:Vec<(i64,String)>=tx.prepare("SELECT t.id,t.group_key FROM asset_cleanup_research t JOIN asset_cleanup_groups g ON g.run_id=t.run_id AND g.group_key=t.group_key WHERE t.run_id=?1 AND t.kind=?2 AND t.target=?3 AND g.state='evidence' AND (t.id=?4 OR t.state IN ('pending','retry','exhausted'))").map_err(err)?.query_map(params![k.id,row.2,row.3,row.0],|r|Ok((r.get(0)?,r.get(1)?))).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;
            for (id, key) in recipients {
                let mut result = if row.2 == "ct_certificate" {
                    certificate_result(&tx, k, &key, &row.3, &value, id != row.0)?
                } else {
                    value.clone()
                };
                preserve_cancellations(&tx, id, &mut result)?;
                if row.2 == "ct_search" {
                    let query = if pattern_only(&tx, k, &key, &row.3)? {
                        format!("%.{}", row.3)
                    } else {
                        row.3.clone()
                    };
                    if result["requestPattern"].as_str() != Some(query.as_str()) {
                        continue;
                    }
                    enqueue_certificates(&tx, k, &key, &result)?;
                }
                tx.execute("UPDATE asset_cleanup_research SET state='done',result_json=?2,error='',claim_token='' WHERE id=?1",params![id,result.to_string()]).map_err(err)?;
                keys.push(key);
            }
        }
        Err(error) => {
            let attempts = row.4 + 1;
            let delay = if attempts < MAX_PUBLIC_ATTEMPTS {
                60
            } else {
                0
            };
            let state = if attempts >= MAX_PUBLIC_ATTEMPTS {
                "exhausted"
            } else {
                "retry"
            };
            // One failed public target is one request, even if several groups
            // depend on it. Share its failure receipt and retry deadline too.
            tx.execute("UPDATE asset_cleanup_research SET state=?4,error=?5,next_at=?6,claim_token='',attempts=MAX(attempts,?7),result_json=json_set(CASE WHEN json_valid(result_json) THEN result_json ELSE '{}' END,'$.sourceFailureAt',?9,'$.sourceFailureKind',?2,'$.sourceFailureTarget',?3) WHERE run_id=?1 AND kind=?2 AND target=?3 AND (id=?8 OR state IN ('pending','retry'))",params![k.id,row.2,row.3,state,error.chars().take(2000).collect::<String>(),now()+delay,attempts,row.0,now()]).map_err(err)?;
            provider_backoff(
                &tx,
                &k.id,
                &row.2,
                &error,
                now() + provider_retry_delay(&error, delay.max(60)),
            )?;
            record_source_failure(&tx, k, &row.2, &error)?;
            keys.push(row.1.clone());
        }
    }
    tx.commit().map_err(err)?;
    keys.sort();
    keys.dedup();
    for key in keys {
        work::reclassify(c, k, &key)?;
    }
    Ok(true)
}
fn validate_public(value: Value, kind: &str, target: &str, pattern: bool) -> Result<Value, String> {
    if !value.is_object() {
        return Err("公开补证响应不是有效结果对象；有限重试，未采纳为证据".into());
    }
    if kind == "ct_certificate" {
        let id = target
            .parse::<i64>()
            .ok()
            .filter(|id| *id > 0)
            .ok_or("公开证书编号无效，未采纳")?;
        let expected = format!("https://crt.sh/?d={id}");
        let sha = value["sha256"].as_str().unwrap_or("");
        let bound = value["certificateId"].as_i64() == Some(id)
            && value["sourceUrl"].as_str() == Some(expected.as_str())
            && sha.len() == 64
            && sha.bytes().all(|b| b.is_ascii_hexdigit());
        let leaf = value["names"]
            .as_array()
            .is_some_and(|a| a.iter().all(Value::is_string))
            && value["organizations"]
                .as_array()
                .is_some_and(|a| a.iter().all(Value::is_string))
            && value["notBefore"]
                .as_i64()
                .zip(value["notAfter"].as_i64())
                .is_some_and(|(a, b)| a < b);
        if !bound || (value["precertificate"] != true && !leaf) {
            return Err(
                "公开证书响应的编号、来源或解析字段不匹配；该请求有限重试，不采纳为归属证据".into(),
            );
        }
    } else if matches!(kind, "ct_search" | "dns_cname")
        && !reusable_public(&value, kind, target, pattern)
    {
        return Err(
            "公开索引或 DNS 响应与请求不匹配或字段无效；该请求有限重试，不采纳为归属证据".into(),
        );
    }
    Ok(value)
}
/// Cancellation accounting belongs to this dependency row, not the source
/// receipt reused by another group. A successful response must retain it.
fn preserve_cancellations(c: &Connection, id: i64, value: &mut Value) -> Result<(), String> {
    let raw: String = c
        .query_row(
            "SELECT result_json FROM asset_cleanup_research WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .map_err(err)?;
    let old: Value = serde_json::from_str(&raw).unwrap_or_else(|_| json!({}));
    let Some(object) = value.as_object_mut() else {
        return Ok(());
    };
    object.remove("cancelledRequests");
    object.remove("lastCancellationAt");
    if let Some(count) = old["cancelledRequests"].as_u64().filter(|n| *n > 0) {
        object.insert("cancelledRequests".into(), json!(count));
        if let Some(at) = old["lastCancellationAt"].as_i64() {
            object.insert("lastCancellationAt".into(), json!(at));
        }
    }
    Ok(())
}
fn enqueue_certificates(
    c: &Connection,
    k: &work::Claim,
    key: &str,
    result: &Value,
) -> Result<(), String> {
    let domain: String = c
        .query_row(
            "SELECT domain FROM asset_cleanup_groups WHERE run_id=?1 AND group_key=?2",
            params![k.id, key],
            |r| r.get(0),
        )
        .map_err(err)?;
    if shared_root(&domain) {
        return Ok(());
    }
    let mut ids = result["certificateIds"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_i64)
        .filter(|id| *id > 0)
        .collect::<Vec<_>>();
    ids.sort_unstable_by(|a, b| b.cmp(a));
    ids.dedup();
    let done:i64=c.query_row("SELECT count(*) FROM asset_cleanup_research WHERE run_id=?1 AND group_key=?2 AND kind='ct_certificate' AND state='done'",params![k.id,key],|r|r.get(0)).map_err(err)?;
    for (i, id) in ids.into_iter().enumerate() {
        let skipped = i as i64 + done >= 5;
        c.execute("INSERT OR IGNORE INTO asset_cleanup_research(run_id,group_key,kind,target,state,error) VALUES(?1,?2,'ct_certificate',?3,?4,?5)",params![k.id,key,id.to_string(),if skipped{"exhausted"}else{"pending"},if skipped{"certificate_sample_limit：本轮仅读取近期最多 5 张证书，完整索引与未读取编号保留；不作为无关证据"}else{""}]).map_err(err)?;
    }
    Ok(())
}
pub(super) fn drain_reusable(c: &mut Connection, k: &work::Claim) -> Result<bool, String> {
    // Certificate bytes are immutable. Existing validated cache reuse remains
    // independent of provider cooldowns and never counts a network attempt.
    if let Some(((id, key, _, target, _), value)) = cached_research(c, k)? {
        let mut result = certificate_result(c, k, &key, &target, &value, true)?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(err)?;
        if !work::claim_valid(&tx, k)? {
            return Ok(false);
        }
        preserve_cancellations(&tx, id, &mut result)?;
        let changed=tx.execute("UPDATE asset_cleanup_research SET state='done',result_json=?2,error='',claim_token='' WHERE id=?1 AND state IN ('pending','retry','exhausted')",params![id,result.to_string()]).map_err(err)?;
        tx.commit().map_err(err)?;
        if changed > 0 {
            work::reclassify(c, k, &key)?;
            return Ok(true);
        }
    }
    // DNS negatives and empty CT indexes are successful, bounded responses too.
    // Reuse an exact target only after checking its source binding and TTL;
    // never copy a previous group's derived ownership facts.
    let rows:Vec<(i64,String,String,String,String)>=c.prepare("SELECT t.id,t.group_key,t.kind,t.target,s.result_json FROM asset_cleanup_research t JOIN asset_cleanup_groups g ON g.run_id=t.run_id AND g.group_key=t.group_key JOIN asset_cleanup_research s ON s.run_id=t.run_id AND s.kind=t.kind AND s.target=t.target AND s.state='done' AND s.id!=t.id WHERE t.run_id=?1 AND t.state IN ('pending','retry','exhausted') AND t.kind IN ('dns_cname','ct_search') AND g.state='evidence' ORDER BY t.id,s.id DESC LIMIT 64").map_err(err)?.query_map([&k.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;
    for (id, key, kind, target, raw) in rows {
        let Ok(mut value) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        if !reusable_public(
            &value,
            &kind,
            &target,
            if kind == "ct_search" {
                pattern_only(c, k, &key, &target)?
            } else {
                false
            },
        ) {
            continue;
        }
        value["cacheHit"] = json!(true);
        value["networkRequested"] = json!(false);
        if kind == "ct_search" {
            value["requestPattern"] = json!(if pattern_only(c, k, &key, &target)? {
                format!("%.{target}")
            } else {
                target.clone()
            });
        }
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(err)?;
        if !work::claim_valid(&tx, k)? {
            return Ok(false);
        }
        if kind == "ct_search" {
            enqueue_certificates(&tx, k, &key, &value)?;
        }
        preserve_cancellations(&tx, id, &mut value)?;
        let changed=tx.execute("UPDATE asset_cleanup_research SET state='done',result_json=?2,error='',claim_token='' WHERE id=?1 AND state IN ('pending','retry','exhausted')",params![id,value.to_string()]).map_err(err)?;
        tx.commit().map_err(err)?;
        if changed > 0 {
            work::reclassify(c, k, &key)?;
            return Ok(true);
        }
    }
    let failure:Option<(i64,String,i64,i64,String,String)>=c.query_row("SELECT t.id,s.state,s.attempts,s.next_at,s.error,s.result_json FROM asset_cleanup_research t JOIN asset_cleanup_groups g ON g.run_id=t.run_id AND g.group_key=t.group_key JOIN asset_cleanup_research s ON s.run_id=t.run_id AND s.kind=t.kind AND s.target=t.target AND s.id!=t.id AND s.state IN ('retry','exhausted') WHERE t.run_id=?1 AND g.state='evidence' AND t.state='pending' AND t.attempts=0 AND json_valid(s.result_json) AND json_extract(s.result_json,'$.sourceFailureAt')>=?2 AND json_extract(s.result_json,'$.sourceFailureAt')<=?3 AND json_extract(s.result_json,'$.sourceFailureKind')=t.kind AND json_extract(s.result_json,'$.sourceFailureTarget')=t.target ORDER BY t.id,s.id DESC LIMIT 1",params![k.id,now()-86400,now()+60],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional().map_err(err)?;
    if let Some((id, state, attempts, next, error, result)) = failure {
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(err)?;
        if !work::claim_valid(&tx, k)? {
            return Ok(false);
        }
        let mut result: Value = serde_json::from_str(&result).map_err(err)?;
        preserve_cancellations(&tx, id, &mut result)?;
        let changed=tx.execute("UPDATE asset_cleanup_research SET state=?2,attempts=?3,next_at=?4,error=?5,result_json=?6 WHERE id=?1 AND state='pending' AND attempts=0",params![id,state,attempts,next,error,result.to_string()]).map_err(err)?;
        tx.commit().map_err(err)?;
        return Ok(changed > 0);
    }
    Ok(false)
}
fn reusable_public(value: &Value, kind: &str, target: &str, pattern: bool) -> bool {
    let Some(at) = value["checkedAt"].as_i64() else {
        return false;
    };
    let sha = value["bodySha256"].as_str().unwrap_or("");
    if value["kind"] != kind
        || at < now() - 86400
        || at > now() + 60
        || sha.len() != 64
        || !sha.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return false;
    }
    let Some(url) = value["sourceUrl"]
        .as_str()
        .and_then(|s| reqwest::Url::parse(s).ok())
    else {
        return false;
    };
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
    {
        return false;
    }
    let pairs = url.query_pairs().collect::<BTreeMap<_, _>>();
    if kind == "ct_search" {
        let query = if pattern {
            format!("%.{target}")
        } else {
            target.into()
        };
        return url.host_str() == Some("crt.sh")
            && url.path() == "/"
            && pairs.get("q").is_some_and(|q| q.as_ref() == query)
            && pairs.get("output").is_some_and(|v| v == "json")
            && value["certificateIds"].as_array().is_some_and(|ids| {
                ids.len() < 10000 && ids.iter().all(|v| v.as_i64().is_some_and(|i| i > 0))
            });
    }
    if kind != "dns_cname"
        || !matches!(
            (url.host_str(), url.path()),
            (Some("dns.google"), "/resolve") | (Some("cloudflare-dns.com"), "/dns-query")
        )
        || !pairs.get("name").is_some_and(|n| n.as_ref() == target)
        || !pairs.get("type").is_some_and(|n| n == "5")
        || value["complete"] != true
        || !value["facts"].as_array().is_some_and(Vec::is_empty)
    {
        return false;
    }
    let Some(records) = value["records"].as_array() else {
        return false;
    };
    let response = json!({"Status":value["dnsStatus"],"TC":false,"Question":[{"name":target,"type":5}],"Answer":records.iter().map(|r|json!({"type":5,"name":r["name"],"data":r["cname"]})).collect::<Vec<_>>()});
    parse_dns(&response, target).is_ok()
}

pub(super) fn organization(
    path: &Path,
    c: &mut Connection,
    k: &work::Claim,
) -> Result<bool, String> {
    let target: String = c
        .query_row(
            "SELECT organization FROM asset_cleanup_runs WHERE id=?1",
            [&k.id],
            |r| r.get(0),
        )
        .map_err(err)?;
    let existing:Option<(String,i64,i64,String)>=c.query_row("SELECT state,next_at,attempts,result_json FROM asset_cleanup_research WHERE run_id=?1 AND kind='organization'",[&k.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(err)?;
    // New independently verified official sources deserve a first attempt even
    // when the previous catalog was exhausted. Never reset old source budgets:
    // the catalog upgrade collector only requests URLs with no stored receipt.
    let catalog_upgrade =
        organization_evidence::source_catalog_revision_for(&target).is_some_and(|revision| {
            existing.as_ref().is_some_and(|(_, _, _, raw)| {
                serde_json::from_str::<Value>(raw).unwrap_or_default()["sourceCatalogRevision"]
                    != revision
            })
        });
    if !catalog_upgrade
        && existing.as_ref().is_some_and(|(s, _, attempts, _)| {
            !matches!(s.as_str(), "done" | "exhausted") && *attempts >= MAX_PUBLIC_ATTEMPTS
        })
    {
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(err)?;
        if !work::claim_valid(&tx, k)? {
            return Ok(false);
        }
        let changed=tx.execute("UPDATE asset_cleanup_research SET state='exhausted',claim_token='',error=CASE WHEN error='' THEN 'organization_source_unavailable：本轮单位关系来源补证次数已用尽；已接受单位与原回执保留，法定名单可能不完整' ELSE error||'；organization_source_unavailable：本轮单位关系补证次数已用尽，已接受单位保留' END WHERE run_id=?1 AND kind='organization' AND state IN ('pending','retry') AND attempts>=?2",params![k.id,MAX_PUBLIC_ATTEMPTS]).map_err(err)?;
        tx.commit().map_err(err)?;
        return Ok(changed > 0);
    }
    if existing.as_ref().is_some_and(|(s, t, _, _)| {
        !catalog_upgrade && (matches!(s.as_str(), "done" | "exhausted") || *t > now())
    }) {
        return Ok(false);
    }
    let attempts = existing.as_ref().map(|r| r.2).unwrap_or(0) + 1;
    let settings = crate::model_configuration::default_settings_for(c, &["network"]);
    let proxy = settings["proxyUrl"].as_str().unwrap_or("");
    c.execute("INSERT OR IGNORE INTO asset_cleanup_research(run_id,group_key,kind,target) VALUES(?1,'__organization','organization',?2)",params![k.id,target]).map_err(err)?;
    let mut response = if catalog_upgrade {
        organization_evidence::refresh_new_sources(path, k.p, &target, proxy)?
    } else {
        organization_evidence::refresh(path, k.p, &target, proxy)?
    };
    if catalog_upgrade {
        if let Some((_, _, prior_attempts, raw)) = &existing {
            response["catalogUpgrade"] = json!({
                "previousAttempts": prior_attempts,
                "previousReceipt": serde_json::from_str::<Value>(raw).unwrap_or_else(|_| json!({"unparsedReceipt":raw})),
                "oldSourcesReplayed": false
            });
        }
    }
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    if !work::claim_valid(&tx, k)? {
        return Ok(true);
    }
    let retry = response["retryAt"].as_i64().unwrap_or(0);
    let exhausted = retry > now() && attempts >= MAX_PUBLIC_ATTEMPTS;
    let source_error = response["error"].as_str().unwrap_or("");
    let message = if exhausted {
        format!("{source_error}；organization_source_unavailable：本轮单位关系来源补证次数已用尽；已接受单位保留，法定名单可能不完整")
    } else {
        source_error.to_owned()
    };
    tx.execute("UPDATE asset_cleanup_research SET state=?2,result_json=?3,error=?4,next_at=?5,attempts=attempts+1 WHERE run_id=?1 AND kind='organization'",params![k.id,if exhausted{"exhausted"}else if retry>now(){"retry"}else{"done"},response.to_string(),message,retry]).map_err(err)?;
    let target = review::scope::target_context(&tx, k.p, &target)?;
    apply_target_names(&tx, k, &target.names)?;
    tx.commit().map_err(err)?;
    Ok(true)
}
/// A new accepted legal relationship changes local relevance, not probe scope.
/// Revisit unresolved groups without changing prior paid-request bases or any
/// manual decision, exclusion, restore, or authorization. Classification can
/// retain a now-known subsidiary directly from existing bound local evidence.
fn apply_target_names(
    c: &Connection,
    k: &work::Claim,
    names: &BTreeSet<String>,
) -> Result<bool, String> {
    if !work::claim_valid(c, k)? {
        return Ok(false);
    }
    let old: String = c
        .query_row(
            "SELECT target_names_json FROM asset_cleanup_runs WHERE id=?1",
            [&k.id],
            |r| r.get(0),
        )
        .map_err(err)?;
    let old: BTreeSet<String> = serde_json::from_str(&old).map_err(err)?;
    if &old == names {
        return Ok(false);
    }
    c.execute(
        "UPDATE asset_cleanup_runs SET target_names_json=?2 WHERE id=?1",
        params![k.id, json!(names).to_string()],
    )
    .map_err(err)?;
    let manual = if has(c, "asset_seed_review_items")? && has(c, "asset_seed_reviews")? {
        "AND NOT EXISTS(SELECT 1 FROM asset_seed_review_items i JOIN asset_seed_reviews r ON r.id=i.run_id JOIN asset_cleanup_runs current ON current.id=?1 AND current.project_id=r.project_id AND current.organization=r.organization JOIN json_each(i.evidence_json,'$.domains') n WHERE i.decision IN ('accepted','rejected') AND (n.value=asset_cleanup_groups.domain OR n.value LIKE '%.'||asset_cleanup_groups.domain))"
    } else {
        ""
    };
    c.execute(&format!("UPDATE asset_cleanup_groups SET state='pending',reason='已接受的目标单位关系有新增或变化，重新应用本地证据；保留原模型回执、人工结论和授权范围' WHERE run_id=?1 AND state IN ('evidence','deferred') {manual}"),[&k.id]).map_err(err)?;
    Ok(true)
}

#[cfg(test)]
pub(super) fn claim_website_batch_for_test(
    c: &mut Connection,
    k: &work::Claim,
) -> Result<Vec<(i64, String, String, i64)>, String> {
    claim_website_batch(c, k)
}
#[cfg(test)]
pub(super) fn finish_website_for_test(
    c: &mut Connection,
    k: &work::Claim,
    id: i64,
    key: &str,
    attempts: i64,
    result: Result<Value, String>,
) -> Result<bool, String> {
    finish_website_result(c, k, id, key, attempts, result)
}

#[cfg(test)]
pub(super) fn settle_public_for_test(
    c: &mut Connection,
    k: &work::Claim,
    ids: &[i64],
) -> Result<(), String> {
    settle_public_rows(c, k, ids)
}
