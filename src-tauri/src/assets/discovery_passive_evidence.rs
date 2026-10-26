//! Public CT evidence for unconfirmed candidates. Only crt.sh is contacted;
//! this module owns no thread, target connection, paid request or scope grant.
//! Contract: crtsh/certwatch_db fnc/web_apis.fnc and download_cert.fnc.
use super::*;
use std::collections::{BTreeMap, HashMap};

const TABLES: &str = "
CREATE TABLE IF NOT EXISTS asset_passive_seeded_items(project_id INTEGER NOT NULL,run_id TEXT NOT NULL,item_key TEXT NOT NULL,revision INTEGER NOT NULL,state TEXT NOT NULL,advice TEXT NOT NULL,decision TEXT NOT NULL,PRIMARY KEY(project_id,run_id,item_key));
CREATE TABLE IF NOT EXISTS asset_passive_candidates(project_id INTEGER NOT NULL,run_id TEXT NOT NULL,item_key TEXT NOT NULL,host TEXT NOT NULL,PRIMARY KEY(project_id,run_id,item_key,host));
CREATE INDEX IF NOT EXISTS asset_passive_candidate_host ON asset_passive_candidates(project_id,run_id,host,item_key);
CREATE TABLE IF NOT EXISTS asset_passive_tasks(id INTEGER PRIMARY KEY,project_id INTEGER NOT NULL,run_id TEXT NOT NULL,kind TEXT NOT NULL,target TEXT NOT NULL,host TEXT NOT NULL,state TEXT NOT NULL DEFAULT 'pending',attempts INTEGER NOT NULL DEFAULT 0,next_at INTEGER NOT NULL DEFAULT 0,lease_until INTEGER NOT NULL DEFAULT 0,claim_token TEXT NOT NULL DEFAULT '',result_json TEXT NOT NULL DEFAULT '{}',error TEXT NOT NULL DEFAULT '',updated_at TEXT NOT NULL DEFAULT(datetime('now')),UNIQUE(project_id,run_id,kind,target));
CREATE INDEX IF NOT EXISTS asset_passive_queue ON asset_passive_tasks(project_id,run_id,state,next_at);
CREATE TABLE IF NOT EXISTS asset_passive_certificate_refs(project_id INTEGER NOT NULL,run_id TEXT NOT NULL,host TEXT NOT NULL,certificate_id INTEGER NOT NULL,PRIMARY KEY(project_id,run_id,host,certificate_id));
CREATE INDEX IF NOT EXISTS asset_passive_ref_certificate ON asset_passive_certificate_refs(project_id,run_id,certificate_id,host);
CREATE TABLE IF NOT EXISTS asset_passive_certificates(certificate_id INTEGER PRIMARY KEY,sha256 TEXT NOT NULL,der_base64 TEXT NOT NULL,names_json TEXT NOT NULL,organizations_json TEXT NOT NULL,not_before INTEGER NOT NULL,not_after INTEGER NOT NULL,source_url TEXT NOT NULL,fetched_at TEXT NOT NULL DEFAULT(datetime('now')));
";
fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
pub(crate) fn ensure(c: &Connection) -> Result<(), String> {
    c.execute_batch(TABLES).map_err(err)
}
fn exists(c: &Connection) -> Result<bool, String> {
    c.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='asset_passive_tasks')",
        [],
        |r| r.get(0),
    )
    .map_err(err)
}
fn owned(c: &Connection, p: i64, run: &str) -> Result<(), String> {
    project(c, p)?;
    let valid:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM asset_seed_reviews WHERE id=?1 AND project_id=?2 AND cursor>=max_id)",params![run,p],|r|r.get(0)).map_err(err)?;
    if valid {
        Ok(())
    } else {
        Err("被动补证需要已完成本地整理的当前项目审阅任务".into())
    }
}
/// Idempotent per review+host; polling never creates a new public request.
/// Model relevance only selects public research, never active probing/ownership.
pub(crate) fn enqueue_review_candidates(
    c: &Connection,
    p: i64,
    run: &str,
) -> Result<usize, String> {
    ensure(c)?;
    // Publish one coherent batch. Thousands of individual autocommits used to
    // starve the free-discovery writer and mark a healthy task interrupted.
    let tx = rusqlite::Transaction::new_unchecked(c, rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    let count = enqueue_candidates_on(&tx, p, run)?;
    tx.commit().map_err(err)?;
    Ok(count)
}
fn enqueue_candidates_on(c: &Connection, p: i64, run: &str) -> Result<usize, String> {
    owned(c, p, run)?;
    let blocked = exclusions::rules(c, p)?;
    let mut q=c.prepare("SELECT i.item_key,i.evidence_json,i.revision,i.state,COALESCE(json_extract(i.evidence_json,'$.currentAdvice'),i.advice),i.decision,i.confidence FROM asset_seed_review_items i LEFT JOIN asset_passive_seeded_items seeded ON seeded.project_id=?2 AND seeded.run_id=i.run_id AND seeded.item_key=i.item_key WHERE i.run_id=?1 AND (seeded.item_key IS NULL OR seeded.revision!=i.revision OR seeded.state!=i.state OR seeded.advice!=COALESCE(json_extract(i.evidence_json,'$.currentAdvice'),i.advice) OR seeded.decision!=i.decision)").map_err(err)?;
    let rows = q
        .query_map(params![run, p], |r| {
            Ok((
                r.get::<_, String>(0)?,
                serde_json::from_str::<Value>(&r.get::<_, String>(1)?).unwrap_or(Value::Null),
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, f64>(6)?,
            ))
        })
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    if rows.is_empty() {
        return Ok(0);
    }
    let items = rows
        .iter()
        .filter(|r| r.5 != "rejected")
        .map(|r| (r.0.clone(), r.1.clone()))
        .collect::<Vec<_>>();
    let projected = review::provenance::current_projections(c, p, run, &items)?;
    let mut candidates = review::scope::public_research_candidates(c, p, run, &projected)?;
    // A manual/model relevance label may request inexpensive public research,
    // but only for original endpoints, never all provider cert.domain names.
    let selected = rows
        .iter()
        .filter(|r| {
            r.5 != "rejected"
                && (r.5 == "accepted" || r.3 == "done" && r.4 == "retain" && r.6 >= 0.8)
        })
        .map(|r| r.0.clone())
        .collect::<BTreeSet<_>>();
    if !selected.is_empty() {
        let mut endpoints=c.prepare(&format!("SELECT refs.item_key,a.host,a.link FROM json_each(?3) keys CROSS JOIN asset_seed_review_refs refs JOIN asset_seed_reviews review ON review.id=refs.run_id JOIN assets a ON a.id=refs.asset_id JOIN project_assets pa ON pa.asset_id=a.id AND pa.project_id=?2 WHERE refs.run_id=?1 AND refs.item_key=keys.value AND (review.include_isolated=1 OR NOT {})",review::ISOLATED)).map_err(err)?;
        for row in endpoints
            .query_map(params![run, p, json!(selected).to_string()], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(err)?
        {
            let (key, h, l) = row.map_err(err)?;
            if let Some(h) = host(if l.is_empty() { &h } else { &l }) {
                candidates.push((key, h));
            }
        }
    }
    let mut count = 0;
    for (key, name) in candidates.into_iter().collect::<BTreeSet<_>>() {
        if exclusions::matches(&name, &blocked) {
            continue;
        }
        c.execute(
            "INSERT OR IGNORE INTO asset_passive_candidates VALUES(?1,?2,?3,?4)",
            params![p, run, key, name],
        )
        .map_err(err)?;
        count+=c.execute("INSERT OR IGNORE INTO asset_passive_tasks(project_id,run_id,kind,target,host) VALUES(?1,?2,'ct_search',?3,?3)",params![p,run,name]).map_err(err)?;
    }
    for (key, _, revision, state, advice, decision, _) in rows {
        c.execute("INSERT INTO asset_passive_seeded_items VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(project_id,run_id,item_key) DO UPDATE SET revision=excluded.revision,state=excluded.state,advice=excluded.advice,decision=excluded.decision",params![p,run,key,revision,state,advice,decision]).map_err(err)?;
    }
    Ok(count)
}
#[derive(Clone, Debug)]
pub(crate) struct Item {
    pub(crate) id: i64,
    pub(crate) kind: String,
    pub(crate) target: String,
    pub(crate) host: String,
    pub(crate) token: String,
    pub(crate) attempts: i64,
}
/// Free idempotent GETs may be recovered after an expired lease. A still-live
/// request keeps its lease; its late receipt cannot overwrite a reclaimed task.
pub(crate) fn recover(c: &Connection, p: i64, run: &str) -> Result<usize, String> {
    if !exists(c)? {
        return Ok(0);
    }
    owned(c, p, run)?;
    c.execute("UPDATE asset_passive_tasks SET state='retry',next_at=?3,error='应用中断；公开只读请求将在退避后恢复',claim_token='',lease_until=0 WHERE project_id=?1 AND run_id=?2 AND state='running' AND lease_until<=?3",params![p,run,now()]).map_err(err)
}
pub(crate) fn claim(c: &mut Connection, p: i64, run: &str) -> Result<Option<Item>, String> {
    ensure(c)?;
    owned(c, p, run)?;
    recover(c, p, run)?;
    let blocked = exclusions::rules(c, p)?;
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    // One public request per project at a time, and at most one per two seconds.
    let busy:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM asset_passive_tasks WHERE project_id=?1 AND (state='running' AND lease_until>strftime('%s','now') OR (state IN ('done','retry','failed') AND updated_at>datetime('now','-2 seconds'))))",[p],|r|r.get(0)).map_err(err)?;
    if busy {
        return Ok(None);
    }
    tx.execute("UPDATE asset_passive_tasks AS t SET state='skipped',error='候选已被人工排除或不再存在' WHERE t.project_id=?1 AND t.run_id=?2 AND t.state IN ('pending','retry') AND NOT ((t.kind='ct_search' AND EXISTS(SELECT 1 FROM asset_passive_candidates pc JOIN asset_seed_review_items i ON i.run_id=pc.run_id AND i.item_key=pc.item_key WHERE pc.project_id=t.project_id AND pc.run_id=t.run_id AND pc.host=t.host AND i.decision!='rejected')) OR (t.kind='ct_certificate' AND EXISTS(SELECT 1 FROM asset_passive_certificate_refs cr JOIN asset_passive_candidates pc ON pc.project_id=cr.project_id AND pc.run_id=cr.run_id AND pc.host=cr.host JOIN asset_seed_review_items i ON i.run_id=pc.run_id AND i.item_key=pc.item_key WHERE cr.project_id=t.project_id AND cr.run_id=t.run_id AND cr.certificate_id=CAST(t.target AS INTEGER) AND i.decision!='rejected')))",params![p,run]).map_err(err)?;
    let row:Option<(i64,String,String,String,i64)>=tx.query_row("SELECT t.id,t.kind,t.target,t.host,t.attempts FROM asset_passive_tasks t WHERE t.project_id=?1 AND t.run_id=?2 AND t.state IN ('pending','retry') AND t.next_at<=?3 AND ((t.kind='ct_search' AND EXISTS(SELECT 1 FROM asset_passive_candidates pc JOIN asset_seed_review_items i ON i.run_id=pc.run_id AND i.item_key=pc.item_key WHERE pc.project_id=t.project_id AND pc.run_id=t.run_id AND pc.host=t.host AND i.decision!='rejected')) OR (t.kind='ct_certificate' AND EXISTS(SELECT 1 FROM asset_passive_certificate_refs cr JOIN asset_passive_candidates pc ON pc.project_id=cr.project_id AND pc.run_id=cr.run_id AND pc.host=cr.host JOIN asset_seed_review_items i ON i.run_id=pc.run_id AND i.item_key=pc.item_key WHERE cr.project_id=t.project_id AND cr.run_id=t.run_id AND cr.certificate_id=CAST(t.target AS INTEGER) AND i.decision!='rejected'))) ORDER BY CASE t.kind WHEN 'ct_certificate' THEN 0 ELSE 1 END,t.id LIMIT 1",params![p,run,now()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(err)?;
    let Some((id, kind, target, h, attempts)) = row else {
        tx.commit().map_err(err)?;
        return Ok(None);
    };
    if exclusions::matches(&h, &blocked) {
        tx.execute("UPDATE asset_passive_tasks SET state='skipped',error='候选已被项目排除规则阻止' WHERE id=?1",[id]).map_err(err)?;
        tx.commit().map_err(err)?;
        return Ok(None);
    }
    let token = uuid::Uuid::new_v4().to_string();
    tx.execute("UPDATE asset_passive_tasks SET state='running',attempts=attempts+1,claim_token=?2,lease_until=?3,updated_at=datetime('now') WHERE id=?1",params![id,token,now()+120]).map_err(err)?;
    tx.commit().map_err(err)?;
    Ok(Some(Item {
        id,
        kind,
        target,
        host: h,
        token,
        attempts: attempts + 1,
    }))
}

pub(crate) fn client(proxy: &str) -> Result<reqwest::blocking::Client, String> {
    let mut b = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("Oviraptor/PublicCertificateEvidence");
    if !proxy.is_empty() {
        b = b.proxy(reqwest::Proxy::all(proxy).map_err(|_| "业务代理格式无效")?);
    }
    b.build()
        .map_err(|e| format!("公开证书客户端初始化失败：{}", e.without_url()))
}
fn request_url(item: &Item) -> Result<reqwest::Url, String> {
    let mut u = reqwest::Url::parse("https://crt.sh/").expect("fixed source");
    match item.kind.as_str() {
        "ct_search" => {
            if host(&item.target).as_deref() != Some(item.target.as_str()) {
                return Err("被动证书候选不是规范域名".into());
            }
            u.query_pairs_mut()
                .append_pair("q", &item.target)
                .append_pair("output", "json")
                .append_pair("exclude", "expired");
        }
        "ct_certificate" => {
            let id = item
                .target
                .parse::<i64>()
                .ok()
                .filter(|id| *id > 0)
                .ok_or("公开证书编号无效")?;
            u.query_pairs_mut().append_pair("d", &id.to_string());
        }
        _ => return Err("未知公开证书动作".into()),
    }
    Ok(u)
}
fn fetch(client: &reqwest::blocking::Client, item: &Item) -> Result<Vec<u8>, String> {
    let u = request_url(item)?;
    let max = if item.kind == "ct_search" {
        16 * 1024 * 1024
    } else {
        256 * 1024
    };
    let response = client.get(u).send().map_err(|e| {
        format!(
            "crt.sh {} 请求失败：{}；保留候选，不能推断资产不存在；VPN保持原状",
            item.kind,
            e.without_url()
        )
    })?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "crt.sh {} HTTP {}；{}；公开证书请求，不消耗Hunter积分",
            item.kind,
            status.as_u16(),
            match status.as_u16() {
                403 => "服务拒绝访问",
                429 => "服务限流",
                _ => "服务未成功",
            }
        ));
    }
    let mut bytes = Vec::new();
    response
        .take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("crt.sh {} 读取失败：{e}", item.kind))?;
    if bytes.len() > max {
        return Err(format!(
            "crt.sh {} 响应超过 {max} 字节；未将截断结果当作完整结果",
            item.kind
        ));
    }
    Ok(bytes)
}
fn name_covers(raw: &str, candidate: &str) -> bool {
    let Some(h) = host(raw) else {
        return false;
    };
    h == candidate
        || raw.starts_with("*.")
            && candidate
                .strip_suffix(&format!(".{h}"))
                .is_some_and(|prefix| !prefix.is_empty() && !prefix.contains('.'))
}
fn parse_search(bytes: &[u8], candidate: &str) -> Result<Value, String> {
    let v: Value = serde_json::from_slice(bytes)
        .map_err(|_| format!("crt.sh ct_search 非有效JSON（{}字节）；未采纳", bytes.len()))?;
    let rows = v
        .as_array()
        .ok_or("crt.sh ct_search 返回类型不是记录数组")?;
    if rows.len() >= 10_000 {
        return Err("crt.sh 返回已达到平台结果上限；候选保留，不能声称完整查询".into());
    }
    let mut ids = BTreeSet::new();
    for row in rows {
        // JSON identity is only an index hint; the downloaded leaf is rechecked.
        if !row["name_value"]
            .as_str()
            .into_iter()
            .flat_map(str::lines)
            .any(|n| name_covers(n, candidate))
        {
            continue;
        }
        let id = row["id"]
            .as_i64()
            .or_else(|| row["min_cert_id"].as_i64())
            .filter(|id| *id > 0)
            .ok_or("crt.sh 匹配记录缺少有效证书编号")?;
        ids.insert(id);
    }
    Ok(
        json!({"certificateIds":ids,"indexedRecords":rows.len(),"source":"crt.sh","candidate":candidate,"networkRequested":true,"complete":false,"coverage":"public_index_response","hunterRequests":0}),
    )
}
fn der_field<'a>(b: &mut &'a [u8]) -> Option<(u8, &'a [u8])> {
    let tag = *b.first()?;
    let first = *b.get(1)?;
    let mut offset = 2;
    let len = if first < 128 {
        first as usize
    } else {
        let n = (first & 127) as usize;
        if n == 0 || n > 4 {
            return None;
        }
        let mut len = 0usize;
        for _ in 0..n {
            len = len
                .checked_mul(256)?
                .checked_add(*b.get(offset)? as usize)?;
            offset += 1;
        }
        len
    };
    let end = offset.checked_add(len)?;
    let out = b.get(offset..end)?;
    *b = b.get(end..)?;
    Some((tag, out))
}
// Parse validity from the same DER, never trust provider/index date fields.
pub(super) fn validity(der: &[u8]) -> Option<(i64, i64)> {
    fn date(tag: u8, b: &[u8]) -> Option<i64> {
        if !b.is_ascii() {
            return None;
        }
        let s = std::str::from_utf8(b).ok()?;
        let expanded = if tag == 0x17 && s.len() == 13 {
            let year = s[..2].parse::<u32>().ok()?;
            format!("{}{}", if year >= 50 { "19" } else { "20" }, s)
        } else if tag == 0x18 && s.len() == 15 {
            s.to_string()
        } else {
            return None;
        };
        chrono::NaiveDateTime::parse_from_str(&expanded, "%Y%m%d%H%M%SZ")
            .ok()
            .map(|d| d.and_utc().timestamp())
    }
    let mut outer = der;
    let (0x30, mut cert) = der_field(&mut outer)? else {
        return None;
    };
    let (0x30, mut tbs) = der_field(&mut cert)? else {
        return None;
    };
    if tbs.first() == Some(&0xa0) {
        der_field(&mut tbs)?;
    }
    for _ in 0..3 {
        der_field(&mut tbs)?;
    }
    let (0x30, mut times) = der_field(&mut tbs)? else {
        return None;
    };
    let (a, x) = der_field(&mut times)?;
    let (b, y) = der_field(&mut times)?;
    Some((date(a, x)?, date(b, y)?))
}
// A precertificate has a different DER fingerprint from the deployed final
// leaf. It remains public evidence but is never a paid fingerprint seed.
fn ct_poison(der: &[u8]) -> Option<bool> {
    fn read(der: &[u8]) -> Option<bool> {
        let mut outer = der;
        let (0x30, mut cert) = der_field(&mut outer)? else {
            return None;
        };
        let (0x30, mut tbs) = der_field(&mut cert)? else {
            return None;
        };
        if tbs.first() == Some(&0xa0) {
            der_field(&mut tbs)?;
        }
        for _ in 0..6 {
            der_field(&mut tbs)?;
        }
        while !tbs.is_empty() {
            let (tag, mut explicit) = der_field(&mut tbs)?;
            if tag != 0xa3 {
                continue;
            }
            let (0x30, mut extensions) = der_field(&mut explicit)? else {
                return None;
            };
            while !extensions.is_empty() {
                let (0x30, mut extension) = der_field(&mut extensions)? else {
                    return None;
                };
                let (0x06, oid) = der_field(&mut extension)? else {
                    return None;
                };
                if oid == [0x2b, 0x06, 0x01, 0x04, 0x01, 0xd6, 0x79, 0x02, 0x04, 0x03] {
                    return Some(true);
                }
            }
        }
        Some(false)
    }
    read(der)
}
pub(super) fn is_precertificate(der: &[u8]) -> bool {
    ct_poison(der).unwrap_or(true)
}

// Encoding is decoded before the TLS-validity parser: RFC6962 poison is a
// deliberate critical extension and webpki correctly rejects it for TLS use.
fn decoded_certificate(bytes: &[u8]) -> Result<Vec<u8>, String> {
    use rustls::pki_types::{pem::PemObject, CertificateDer};
    let der = if bytes.first() == Some(&0x30) {
        bytes.to_vec()
    } else {
        let text = std::str::from_utf8(bytes)
            .map_err(|_| "公开证书既不是DER也不是UTF-8 PEM/base64")?
            .trim();
        if text.contains("-----BEGIN CERTIFICATE-----") {
            CertificateDer::from_pem_slice(text.as_bytes())
                .map_err(|_| "公开证书PEM编码无效")?
                .as_ref()
                .to_vec()
        } else {
            let compact = text
                .chars()
                .filter(|c| !c.is_ascii_whitespace())
                .collect::<String>();
            base64::engine::general_purpose::STANDARD
                .decode(compact)
                .map_err(|_| "公开证书PEM/base64编码无效")?
        }
    };
    if der.is_empty() || der.len() > 64 * 1024 {
        return Err("公开证书DER为空或超过64KiB".into());
    }
    Ok(der)
}
fn parse_certificate(bytes: &[u8], id: i64) -> Result<Value, String> {
    let der = decoded_certificate(bytes)
        .map_err(|e| format!("crt.sh 证书 {id} 编码错误（{}字节）：{e}", bytes.len()))?;
    if ct_poison(&der) == Some(true) {
        return Ok(
            json!({"certificateId":id,"certificateType":"precertificate","precertificate":true,"skipped":true,
            "reason":"CT 预证书包含 critical poison 扩展，不能用于 TLS；其 SHA-256 与最终部署证书不同，已完成分类，不再重试且不提交 Hunter",
            "sha256":format!("{:x}",Sha256::digest(&der)),"derBase64":base64::engine::general_purpose::STANDARD.encode(&der),
            "sourceUrl":format!("https://crt.sh/?d={id}"),"networkRequested":true,"hunterRequests":0}),
        );
    }
    let cert = crate::certificates::from_der(&der).ok_or_else(|| {
        format!(
            "crt.sh 证书 {id} 不是有效X.509叶证书（{}字节）；未采纳",
            bytes.len()
        )
    })?;
    let (before, after) = validity(&der).ok_or("公开证书有效期解析失败；未作为当前归属证据")?;
    Ok(
        json!({"certificateId":id,"sha256":cert.sha256,"derBase64":cert.der_base64,"names":cert.server_names(),"organizations":cert.organizations,"notBefore":before,"notAfter":after,"sourceUrl":format!("https://crt.sh/?d={id}"),"networkRequested":true,"hunterRequests":0}),
    )
}
/// Network runs outside any database transaction. Responses are bounded and no
/// returned URL is followed; certificates can only come from fixed crt.sh.
fn execute_with(
    item: &Item,
    transport: impl FnOnce(reqwest::Url, usize) -> Result<Vec<u8>, String>,
) -> Result<Value, String> {
    let url = request_url(item)?;
    let bytes = transport(
        url,
        if item.kind == "ct_search" {
            16 * 1024 * 1024
        } else {
            256 * 1024
        },
    )?;
    if bytes.len()
        > if item.kind == "ct_search" {
            16 * 1024 * 1024
        } else {
            256 * 1024
        }
    {
        return Err("crt.sh 响应超过允许大小；未采纳".into());
    }
    if item.kind == "ct_search" {
        parse_search(&bytes, &item.host)
    } else {
        parse_certificate(&bytes, item.target.parse::<i64>().map_err(err)?)
    }
}
pub(crate) fn execute(client: &reqwest::blocking::Client, item: &Item) -> Result<Value, String> {
    execute_with(item, |_, _| fetch(client, item))
}
/// Commit the receipt and children atomically. A late/replayed receipt is inert.
pub(crate) fn finish(
    c: &mut Connection,
    p: i64,
    run: &str,
    item: &Item,
    result: Result<Value, String>,
) -> Result<bool, String> {
    ensure(c)?;
    owned(c, p, run)?;
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    let live:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM asset_passive_tasks WHERE id=?1 AND project_id=?2 AND run_id=?3 AND state='running' AND claim_token=?4)",params![item.id,p,run,item.token],|r|r.get(0)).map_err(err)?;
    if !live {
        return Ok(false);
    }
    let result = result.and_then(|v| {
        if item.kind == "ct_certificate" {
            let id = item.target.parse::<i64>().map_err(err)?;
            let raw = v["derBase64"].as_str().ok_or("公开证书回执缺少DER")?;
            parse_certificate(raw.as_bytes(), id)
        } else {
            Ok(v)
        }
    });
    match result {
        Ok(v) => {
            if item.kind == "ct_search" {
                for id in v["certificateIds"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_i64)
                {
                    tx.execute(
                        "INSERT OR IGNORE INTO asset_passive_certificate_refs VALUES(?1,?2,?3,?4)",
                        params![p, run, item.host, id],
                    )
                    .map_err(err)?;
                    let cached:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM asset_passive_certificates WHERE certificate_id=?1)",[id],|r|r.get(0)).map_err(err)?;
                    if !cached {
                        tx.execute("INSERT OR IGNORE INTO asset_passive_tasks(project_id,run_id,kind,target,host) VALUES(?1,?2,'ct_certificate',?3,?4)",params![p,run,id.to_string(),item.host]).map_err(err)?;
                    }
                }
            } else if v["precertificate"] != true {
                let id = item.target.parse::<i64>().map_err(err)?;
                // Parse again at the persistence boundary. Mock transports and
                // future callers cannot manufacture SANs/O via result metadata.
                let bound = &v;
                tx.execute("INSERT OR IGNORE INTO asset_passive_certificates(certificate_id,sha256,der_base64,names_json,organizations_json,not_before,not_after,source_url) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![id,bound["sha256"].as_str(),bound["derBase64"].as_str(),bound["names"].to_string(),bound["organizations"].to_string(),bound["notBefore"].as_i64(),bound["notAfter"].as_i64(),bound["sourceUrl"].as_str()]).map_err(err)?;
            }
            let precertificate = item.kind == "ct_certificate" && v["precertificate"] == true;
            tx.execute("UPDATE asset_passive_tasks SET state=?4,result_json=?3,error=?5,claim_token='',lease_until=0,next_at=0,updated_at=datetime('now') WHERE id=?1 AND claim_token=?2",params![item.id,item.token,v.to_string(),if precertificate{"skipped"}else{"done"},if precertificate{v["reason"].as_str().unwrap_or("CT预证书已分类，不再重试")}else{""}]).map_err(err)?;
        }
        Err(error) => {
            let delay = match item.attempts {
                1 => 60,
                2 => 300,
                3 => 1800,
                _ => 7200,
            };
            let state = if item.attempts >= 5 {
                "failed"
            } else {
                "retry"
            };
            tx.execute("UPDATE asset_passive_tasks SET state=?3,error=?4,next_at=?5,claim_token='',lease_until=0,updated_at=datetime('now') WHERE id=?1 AND claim_token=?2",params![item.id,item.token,state,error.chars().filter(|c|!c.is_control()).take(2048).collect::<String>(),now()+delay]).map_err(err)?;
        }
    }
    tx.commit().map_err(err)?;
    Ok(true)
}
/// Only current public DER bound to this review's exact candidate. Historical
/// CT records remain in the cache, never become current ownership/scope proof.
pub(crate) fn observations(
    c: &Connection,
    p: i64,
    run: &str,
    names: &BTreeSet<String>,
) -> Result<Vec<Value>, String> {
    if !exists(c)? || names.is_empty() {
        return Ok(vec![]);
    }
    let mut q=c.prepare("SELECT DISTINCT refs.host,cert.certificate_id,cert.sha256,cert.der_base64,cert.source_url,cert.fetched_at FROM json_each(?4) ns JOIN asset_passive_certificate_refs refs ON refs.host=ns.value JOIN asset_passive_certificates cert ON cert.certificate_id=refs.certificate_id WHERE refs.project_id=?1 AND refs.run_id=?2 AND cert.not_before<=?3 AND cert.not_after>?3 ORDER BY cert.certificate_id DESC").map_err(err)?;
    let mut out = BTreeMap::new();
    let mut parsed = HashMap::<i64, Option<(String, Vec<String>, Vec<String>, i64, i64)>>::new();
    for row in q
        .query_map(params![p, run, now(), json!(names).to_string()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
            ))
        })
        .map_err(err)?
    {
        let (candidate, id, hash, raw, url, at) = row.map_err(err)?;
        // Derived cache columns are display/index aids, never authority for a
        // scope proposal. Reparse this exact DER once per certificate.
        let identity = parsed.entry(id).or_insert_with(|| {
            let cert = crate::certificates::from_text(&raw)?;
            if cert.sha256 != hash {
                return None;
            }
            let der = base64::engine::general_purpose::STANDARD
                .decode(&cert.der_base64)
                .ok()?;
            let (before, after) = validity(&der)?;
            let names = cert.server_names().to_vec();
            Some((cert.sha256, names, cert.organizations, before, after))
        });
        let Some((sha, ns, orgs, before, after)) = identity else {
            continue;
        };
        if *before > now() || *after <= now() {
            continue;
        }
        for name in ns.iter().filter(|name| name_covers(name, &candidate)) {
            let Some(h) = host(name) else {
                continue;
            };
            for org in orgs.iter().filter(|s| !s.trim().is_empty()) {
                let value = json!({"host":h,"organization":org,"source":"public_ct_subject","certificateSha256":sha,"wildcard":name.starts_with("*."),"certificateId":id,"sourceUrl":url,"observedAt":at,"validFrom":before,"validUntil":after,"candidate":candidate,"networkVerified":false,"currentCertificate":true});
                out.insert(value.to_string(), value);
            }
        }
    }
    Ok(out.into_values().collect())
}
pub(crate) fn summary(c: &Connection, p: i64, run: &str) -> Result<Value, String> {
    if !exists(c)? {
        return Ok(
            json!({"pending":0,"running":0,"retry":0,"failed":0,"done":0,"hunterRequests":0,"targetConnections":0}),
        );
    }
    let mut counts = HashMap::<String, i64>::new();
    let mut q=c.prepare("SELECT state,count(*) FROM asset_passive_tasks WHERE project_id=?1 AND run_id=?2 GROUP BY state").map_err(err)?;
    for row in q
        .query_map(params![p, run], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })
        .map_err(err)?
    {
        let (s, n) = row.map_err(err)?;
        counts.insert(s, n);
    }
    let errors:Vec<Value>=c.prepare("SELECT kind,target,state,attempts,error,next_at FROM asset_passive_tasks WHERE project_id=?1 AND run_id=?2 AND error!='' ORDER BY id DESC LIMIT 20").map_err(err)?.query_map(params![p,run],|r|Ok(json!({"kind":r.get::<_,String>(0)?,"target":r.get::<_,String>(1)?,"state":r.get::<_,String>(2)?,"attempts":r.get::<_,i64>(3)?,"error":r.get::<_,String>(4)?,"nextRetryAt":r.get::<_,i64>(5)?}))).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;
    Ok(
        json!({"pending":counts.get("pending").unwrap_or(&0),"running":counts.get("running").unwrap_or(&0),"retry":counts.get("retry").unwrap_or(&0),"failed":counts.get("failed").unwrap_or(&0),"done":counts.get("done").unwrap_or(&0),"skipped":counts.get("skipped").unwrap_or(&0),"errors":errors,"source":"crt.sh","hunterRequests":0,"targetConnections":0,"scopeExpanded":false}),
    )
}
/// Public certificate candidates are independent from legacy provider hashes.
pub(crate) fn fingerprint_candidates(c: &Connection, p: i64) -> Result<Vec<String>, String> {
    if !exists(c)? {
        return Ok(vec![]);
    }
    let mut q=c.prepare("SELECT DISTINCT cert.sha256,cert.der_base64 FROM asset_passive_certificates cert JOIN asset_passive_certificate_refs refs ON refs.certificate_id=cert.certificate_id JOIN asset_seed_reviews review ON review.id=refs.run_id AND review.project_id=refs.project_id WHERE refs.project_id=?1 AND cert.not_before<=?2 AND cert.not_after>?2 AND EXISTS(SELECT 1 FROM asset_passive_candidates pc JOIN asset_seed_review_items i ON i.run_id=pc.run_id AND i.item_key=pc.item_key WHERE pc.project_id=refs.project_id AND pc.run_id=refs.run_id AND pc.host=refs.host AND i.decision!='rejected') ORDER BY cert.sha256").map_err(err)?;
    let mut out = BTreeSet::new();
    for row in q
        .query_map(params![p, now()], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(err)?
    {
        let (sha, raw) = row.map_err(err)?;
        if let Some(cert) = crate::certificates::from_text(&raw) {
            let der = base64::engine::general_purpose::STANDARD
                .decode(&cert.der_base64)
                .map_err(err)?;
            if cert.sha256 == sha && !is_precertificate(&der) {
                out.insert(sha);
            }
        }
    }
    Ok(out.into_iter().collect())
}
/// Reparse the exact cached DER for every paid-query admission. Every SAN/CN
/// identity must be inside current confirmed scope and outside exclusions.
/// No provider association field or old asset hash participates.
pub(crate) fn fingerprint_anchor(
    c: &Connection,
    p: i64,
    sha: &str,
    confirmed: &[String],
) -> Result<Option<String>, String> {
    if !exists(c)? || confirmed.is_empty() {
        return Ok(None);
    }
    let blocked = exclusions::rules(c, p)?;
    let mut q=c.prepare("SELECT DISTINCT cert.der_base64,refs.host FROM asset_passive_certificates cert JOIN asset_passive_certificate_refs refs ON refs.certificate_id=cert.certificate_id JOIN asset_seed_reviews review ON review.id=refs.run_id AND review.project_id=refs.project_id WHERE refs.project_id=?1 AND cert.sha256=?2 AND EXISTS(SELECT 1 FROM asset_passive_candidates pc JOIN asset_seed_review_items i ON i.run_id=pc.run_id AND i.item_key=pc.item_key WHERE pc.project_id=refs.project_id AND pc.run_id=refs.run_id AND pc.host=refs.host AND i.decision!='rejected')").map_err(err)?;
    for row in q
        .query_map(params![p, sha], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(err)?
    {
        let (raw, candidate) = row.map_err(err)?;
        let Some(cert) = crate::certificates::from_text(&raw) else {
            continue;
        };
        if cert.sha256 != sha {
            continue;
        }
        let der = base64::engine::general_purpose::STANDARD
            .decode(&cert.der_base64)
            .map_err(err)?;
        let Some((before, after)) = validity(&der) else {
            continue;
        };
        if is_precertificate(&der) {
            continue;
        }
        if before > now() || after <= now() {
            continue;
        }
        let names = cert.server_names();
        if names.is_empty() || !names.iter().any(|name| name_covers(name, &candidate)) {
            continue;
        }
        let parsed = names.iter().map(|n| host(n)).collect::<Option<Vec<_>>>();
        let Some(parsed) = parsed else {
            continue;
        };
        let mut exact_blocked=false;
        for name in parsed.iter().chain(std::iter::once(&candidate)){if seed_policy::blocked(c,p,"domain",name)?{exact_blocked=true;break}}
        if exact_blocked{continue}
        if !parsed.iter().all(|name| {
            confirmed.iter().any(|root| in_scope(name, root))
                && !exclusions::matches(name, &blocked)
        }) {
            continue;
        }
        if !confirmed.iter().any(|root| in_scope(&candidate, root))
            || exclusions::matches(&candidate, &blocked)
        {
            continue;
        }
        return Ok(Some(candidate));
    }
    Ok(None)
}
/// The existing Assist owner calls this step; no second background owner is born.
pub(crate) fn tick(
    path: &Path,
    p: i64,
    run: &str,
    stopped: impl Fn() -> bool,
) -> Result<Value, String> {
    let mut c = db::open(path)?;
    ensure(&c)?;
    owned(&c, p, run)?;
    if stopped() {
        return Ok(json!({"changed":false,"processed":false,"paused":true}));
    }
    enqueue_review_candidates(&c, p, run)?;
    let Some(item) = claim(&mut c, p, run)? else {
        let mut v = summary(&c, p, run)?;
        v["changed"] = json!(false);
        v["processed"] = json!(false);
        return Ok(v);
    };
    if stopped() {
        c.execute("UPDATE asset_passive_tasks SET state='pending',attempts=attempts-1,claim_token='',lease_until=0 WHERE id=?1 AND claim_token=?2",params![item.id,item.token]).map_err(err)?;
        return Ok(json!({"changed":false,"processed":false,"paused":true}));
    }
    let settings = crate::model_configuration::default_settings_for(&c, &["network"]);
    let result = if settings.get("_configurationError").is_some() {
        Err("业务网络配置不可用；未发出公开证书请求".into())
    } else {
        client(settings["proxyUrl"].as_str().unwrap_or(""))
            .and_then(|client| execute(&client, &item))
    };
    let success = result
        .as_ref()
        .is_ok_and(|value| value["precertificate"] != true);
    let committed = finish(&mut c, p, run, &item, result)?;
    let mut v = summary(&c, p, run)?;
    // Search can attach an immutable cached DER to this run, so successful
    // searches also trigger one derived reprojection, without another AI call.
    v["changed"] = json!(committed && success);
    v["scopeProposalReprojectionNeeded"] = v["changed"].clone();
    v["processed"] = json!(committed);
    Ok(v)
}
#[cfg(test)]
#[path = "discovery_passive_evidence_tests.rs"]
mod tests;
