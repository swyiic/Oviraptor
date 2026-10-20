//! Exact, resumable Hunter pagination. Search receipts precede local import;
//! unknown remote outcomes are never retried automatically. No API credential
//! or full user-info record is persisted in the expansion journal.
use super::*;
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::Instant,
};
const TABLES:&str="CREATE TABLE IF NOT EXISTS hunter_expansions(
 id TEXT PRIMARY KEY,project_id INTEGER NOT NULL REFERENCES projects(id),account_hash TEXT NOT NULL,query_hash TEXT NOT NULL,
 kind TEXT NOT NULL,target TEXT NOT NULL,anchor TEXT NOT NULL,query TEXT NOT NULL,start_date TEXT NOT NULL,end_date TEXT NOT NULL,
 status TEXT NOT NULL,page_size INTEGER NOT NULL DEFAULT 0,next_page INTEGER NOT NULL DEFAULT 1,total INTEGER,
 received INTEGER NOT NULL DEFAULT 0,unique_rows INTEGER NOT NULL DEFAULT 0,rejected INTEGER NOT NULL DEFAULT 0,
 spent INTEGER NOT NULL DEFAULT 0,cost_unknown INTEGER NOT NULL DEFAULT 0,account_json TEXT NOT NULL DEFAULT '{}',
 owner_pid INTEGER NOT NULL DEFAULT 0,index_changed INTEGER NOT NULL DEFAULT 0,reason TEXT NOT NULL DEFAULT '',created_at TEXT NOT NULL DEFAULT(datetime('now','localtime')),updated_at TEXT NOT NULL DEFAULT(datetime('now','localtime')),
 UNIQUE(project_id,account_hash,query_hash));
 CREATE UNIQUE INDEX IF NOT EXISTS hunter_one_account_writer ON hunter_expansions(account_hash) WHERE status IN ('running','stopping');
 CREATE TABLE IF NOT EXISTS hunter_expansion_pages(query_id TEXT NOT NULL REFERENCES hunter_expansions(id),page INTEGER NOT NULL,state TEXT NOT NULL,
 request_id TEXT NOT NULL UNIQUE,payload_json TEXT NOT NULL DEFAULT '{}',page_hash TEXT NOT NULL DEFAULT '',cost INTEGER,accounted INTEGER NOT NULL DEFAULT 0,cost_text TEXT NOT NULL DEFAULT '',rest_text TEXT NOT NULL DEFAULT '',raw_count INTEGER NOT NULL DEFAULT 0,accepted INTEGER NOT NULL DEFAULT 0,rejected INTEGER NOT NULL DEFAULT 0,import_json TEXT NOT NULL DEFAULT '{}',PRIMARY KEY(query_id,page));
 CREATE TABLE IF NOT EXISTS hunter_expansion_links(project_id INTEGER NOT NULL,endpoint TEXT NOT NULL,query_id TEXT NOT NULL REFERENCES hunter_expansions(id),page INTEGER NOT NULL,evidence_json TEXT NOT NULL,PRIMARY KEY(project_id,endpoint,query_id));";
fn running() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    static R: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
}
fn tables(c: &Connection) -> Result<bool, String> {
    c.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='hunter_expansions')",[],|r|r.get(0)).map_err(err)
}
pub(crate) fn fingerprint(raw: &str) -> Option<String> {
    let h = raw.trim().replace(':', "").to_ascii_lowercase();
    (h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit())).then_some(h)
}
/// An exact seed must have an actual project observation and a confirmed domain
/// anchor. An arbitrary pasted fingerprint or shared CNAME cannot create scope.
pub(crate) fn evidence_anchor(
    c: &Connection,
    p: i64,
    target: &str,
    kind: &str,
) -> Result<String, String> {
    let confirmed = roots(c, p)?;
    if confirmed.is_empty() {
        return Err("请先保存当前项目明确确认的域名范围".into());
    }
    if kind == "domain" || kind == "domain_tree" {
        let h = approved_host(c, p, target)?;
        if confirmed.contains(&h) {
            return Ok(h);
        }
        let observed:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM asset_discovery_seeds WHERE project_id=?1 AND host=?2 AND wildcard=0 AND source IN ('observed_host','certificate_domain','certificate_cn','certificate_transparency','page_or_inline_script'))",params![p,h],|r|r.get(0)).map_err(err)?;
        return if observed {
            Ok(h)
        } else {
            Err("该域名尚未在当前项目被实际发现，也不是明确确认的根域；禁止猜测域名消耗积分".into())
        };
    }
    let sources = if kind == "fingerprint" {
        vec!["certificate_sha256"]
    } else if kind == "cname" {
        vec!["cname", "dns_cname"]
    } else {
        return Err("仅支持证书 SHA-256 指纹、精确域名、域名树或精确 CNAME".into());
    };
    let has_evidence:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='asset_discovery_seed_evidence')",[],|r|r.get(0)).map_err(err)?;
    let refs_sql = if has_evidence {
        "SELECT source_ref FROM asset_discovery_seeds WHERE project_id=?1 AND host=?2 AND source IN (SELECT value FROM json_each(?3)) AND wildcard=0 UNION ALL SELECT source_ref FROM asset_discovery_seed_evidence WHERE project_id=?1 AND host=?2 AND source IN (SELECT value FROM json_each(?3)) AND wildcard=0"
    } else {
        "SELECT source_ref FROM asset_discovery_seeds WHERE project_id=?1 AND host=?2 AND source IN (SELECT value FROM json_each(?3)) AND wildcard=0"
    };
    let mut s = c.prepare(refs_sql).map_err(err)?;
    let refs = s
        .query_map(params![p, target, json!(sources).to_string()], |r| {
            r.get::<_, String>(0)
        })
        .map_err(err)?;
    for reference in refs {
        let reference = reference.map_err(err)?;
        if let Some(id) = reference
            .strip_prefix("asset:")
            .and_then(|s| s.parse::<i64>().ok())
        {
            let present:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM project_assets WHERE project_id=?1 AND asset_id=?2 AND is_deleted=0)",params![p,id],|r|r.get(0)).map_err(err)?;
            if !present {
                continue;
            }
            let names_sql = if has_evidence {
                "SELECT host FROM asset_discovery_seeds WHERE project_id=?1 AND source_ref=?2 AND source IN ('observed_host','certificate_domain','certificate_cn') UNION SELECT host FROM asset_discovery_seed_evidence WHERE project_id=?1 AND source_ref=?2 AND source IN ('observed_host','certificate_domain','certificate_cn')"
            } else {
                "SELECT host FROM asset_discovery_seeds WHERE project_id=?1 AND source_ref=?2 AND source IN ('observed_host','certificate_domain','certificate_cn')"
            };
            let mut names = c.prepare(names_sql).map_err(err)?;
            for h in names
                .query_map(params![p, reference], |r| r.get::<_, String>(0))
                .map_err(err)?
            {
                let h = h.map_err(err)?;
                if confirmed.iter().any(|root| in_scope(&h, root)) {
                    return Ok(h);
                }
            }
        } else if kind == "cname" {
            if let Ok(anchor) = approved_host(c, p, &reference) {
                return Ok(anchor);
            }
        }
    }
    Err("缺少当前项目范围内的实际证书指纹 / CNAME 来源；请先提取真实资产线索，不能用公司名或猜测值替代".into())
}
fn exact_query(kind: &str, target: &str) -> Result<String, String> {
    let q = serde_json::to_string(target).map_err(err)?;
    Ok(match kind {
        "fingerprint" => format!("cert.sha-256={q}"),
        "domain" => format!("domain=={q}"),
        "domain_tree" => format!("(domain=={q}||domain.suffix={q})"),
        "cname" => format!("cname_target=={q}"),
        _ => return Err("不支持的 Hunter 精确检索方式".into()),
    })
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExpansionInput {
    pub project_id: i64,
    pub kind: String,
    pub target: String,
}
fn normalized(i: &ExpansionInput) -> Result<String, String> {
    if i.kind == "fingerprint" {
        fingerprint(&i.target).ok_or("证书 SHA-256 指纹必须是64位十六进制，可包含冒号分隔".into())
    } else {
        host(&i.target)
            .filter(|h| {
                i.target
                    .trim()
                    .trim_end_matches('.')
                    .eq_ignore_ascii_case(h)
            })
            .ok_or("请输入具体 DNS 主机名，不接受查询语句、URL 或通配符".into())
    }
}
fn prepare(c: &mut Connection, input: &ExpansionInput, key: &str) -> Result<String, String> {
    project(c, input.project_id)?;
    let target = normalized(input)?;
    let anchor = evidence_anchor(c, input.project_id, &target, &input.kind)?;
    let query = exact_query(&input.kind, &target)?;
    c.execute_batch(TABLES).map_err(err)?;
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    let day = chrono::Utc::now().date_naive();
    let start = day - chrono::Duration::days(29);
    let account_hash = format!("{:x}", Sha256::digest(key.as_bytes()));
    let hash = format!(
        "{:x}",
        Sha256::digest(format!("{query}\0{start}\0{day}\0all-assets-v1").as_bytes())
    );
    let old:Option<String>=tx.query_row("SELECT id FROM hunter_expansions WHERE project_id=?1 AND account_hash=?2 AND query_hash=?3",params![input.project_id,account_hash,hash],|r|r.get(0)).optional().map_err(err)?;
    let id = if let Some(id) = old {
        id
    } else {
        let id = uuid::Uuid::new_v4().to_string();
        tx.execute("INSERT INTO hunter_expansions(id,project_id,account_hash,query_hash,kind,target,anchor,query,start_date,end_date,status) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'paused')",params![id,input.project_id,account_hash,hash,input.kind,target,anchor,query,start.to_string(),day.to_string()]).map_err(err)?;
        id
    };
    tx.commit().map_err(err)?;
    Ok(id)
}
pub(crate) fn list_on(c: &Connection, p: i64) -> Result<Vec<Value>, String> {
    if !tables(c)? {
        return Ok(vec![]);
    }
    let mut s = c
        .prepare(
            "SELECT id FROM hunter_expansions WHERE project_id=?1 ORDER BY rowid DESC LIMIT 30",
        )
        .map_err(err)?;
    let ids = s
        .query_map([p], |r| r.get::<_, String>(0))
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    ids.iter().map(|id| status(c, p, id)).collect()
}
fn status(c: &Connection, p: i64, id: &str) -> Result<Value, String> {
    let mut v=c.query_row("SELECT id,kind,target,anchor,query,status,page_size,next_page,total,received,unique_rows,rejected,spent,cost_unknown,account_json,reason,start_date,end_date FROM hunter_expansions WHERE id=?1 AND project_id=?2",params![id,p],|r|Ok(json!({"id":r.get::<_,String>(0)?,"kind":r.get::<_,String>(1)?,"target":r.get::<_,String>(2)?,"anchor":r.get::<_,String>(3)?,"query":r.get::<_,String>(4)?,"status":r.get::<_,String>(5)?,"pageSize":r.get::<_,i64>(6)?,"nextPage":r.get::<_,i64>(7)?,"total":r.get::<_,Option<i64>>(8)?,"received":r.get::<_,i64>(9)?,"uniqueRows":r.get::<_,i64>(10)?,"rejected":r.get::<_,i64>(11)?,"spentPoints":r.get::<_,i64>(12)?,"costUnknown":r.get::<_,bool>(13)?,"account":serde_json::from_str::<Value>(&r.get::<_,String>(14)?).unwrap_or(Value::Null),"reason":r.get::<_,String>(15)?,"startDate":r.get::<_,String>(16)?,"endDate":r.get::<_,String>(17)?}))).map_err(|_|"Hunter 查询不在当前项目")?;
    if matches!(v["status"].as_str(), Some("running" | "stopping"))
        && !process_alive(
            c.query_row(
                "SELECT owner_pid FROM hunter_expansions WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(err)?,
        )
        && !running()
            .lock()
            .map_err(|_| "Hunter 状态锁异常")?
            .contains_key(id)
    {
        v["status"] = json!("interrupted");
        v["reason"] = json!("运行进程已退出；已收到的页可恢复入库，结果未知的远端请求不会自动重发");
    }
    let mut s=c.prepare("SELECT page,state,cost,cost_text,rest_text,raw_count,accepted,rejected FROM hunter_expansion_pages WHERE query_id=?1 ORDER BY page DESC LIMIT 20").map_err(err)?;
    v["pages"]=json!(s.query_map([id],|r|Ok(json!({"page":r.get::<_,i64>(0)?,"state":r.get::<_,String>(1)?,"points":r.get::<_,Option<i64>>(2)?,"consumeQuota":r.get::<_,String>(3)?,"restQuota":r.get::<_,String>(4)?,"returned":r.get::<_,i64>(5)?,"accepted":r.get::<_,i64>(6)?,"rejected":r.get::<_,i64>(7)?}))).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)?);
    let pending:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM hunter_expansion_pages WHERE query_id=?1 AND (state='sending' OR accounted=0))",[id],|r|r.get(0)).map_err(err)?;
    v["costUnknown"] = json!(v["costUnknown"] == true || pending);
    v["complete"] = json!(v["status"] == "completed");
    Ok(v)
}
fn finish(c: &Connection, id: &str, state: &str, reason: &str) -> Result<(), String> {
    c.execute("UPDATE hunter_expansions SET status=?2,reason=?3,updated_at=datetime('now','localtime') WHERE id=?1",params![id,state,reason]).map_err(err)?;
    Ok(())
}
trait Provider {
    fn account(&mut self) -> Result<Value, String>;
    fn search(
        &mut self,
        query: &str,
        page: i64,
        size: i64,
        start: &str,
        end: &str,
        kind: &str,
    ) -> Result<Value, String>;
}
struct Live<'a> {
    client: reqwest::blocking::Client,
    key: &'a str,
}
impl Provider for Live<'_> {
    fn account(&mut self) -> Result<Value, String> {
        hunter_user_info(&self.client, self.key)
    }
    fn search(
        &mut self,
        q: &str,
        page: i64,
        size: i64,
        start: &str,
        end: &str,
        kind: &str,
    ) -> Result<Value, String> {
        let u = page_url(
            "https://hunter.qianxin.com/openApi/search",
            self.key,
            q,
            page,
            size,
            start,
            end,
            kind,
        )?;
        get_json(&self.client, u, self.key)
    }
}
fn page_url(
    base: &str,
    key: &str,
    q: &str,
    page: i64,
    size: i64,
    start: &str,
    end: &str,
    kind: &str,
) -> Result<reqwest::Url, String> {
    let mut u = reqwest::Url::parse(base).map_err(err)?;
    u.query_pairs_mut().append_pair("api-key",key).append_pair("search",&base64::engine::general_purpose::URL_SAFE.encode(q.as_bytes())).append_pair("page",&page.to_string()).append_pair("page_size",&size.to_string()).append_pair("is_web","3").append_pair("start_time",start).append_pair("end_time",end).append_pair("fields",if kind=="cname"{"ip,port,domain,url,web_title,protocol,base_protocol,status_code,company,number,cert_sha256,ssl_certificate,cname_target,updated_at"}else{"ip,port,domain,url,web_title,protocol,base_protocol,status_code,company,number,cert_sha256,ssl_certificate,updated_at"});
    Ok(u)
}
fn balance(account: &Value) -> Option<i64> {
    let f = account["rest_free_point"].as_i64()?;
    let e = account["rest_equity_point"].as_i64()?;
    if f < 0 || e < 0 {
        None
    } else {
        f.checked_add(e)
    }
}
fn page_size(account: &Value, requested: i64) -> Result<i64, String> {
    let mut cap = requested.clamp(10, 100);
    for k in ["once_export_quota", "rest_export_quota"] {
        if let Some(n) = account[k].as_i64().filter(|n| *n >= 0) {
            cap = cap.min(n);
        }
    }
    if balance(account) == Some(0) {
        return Err("免费积分和权益积分均已用尽".into());
    }
    [100, 50, 20, 10]
        .into_iter()
        .find(|n| *n <= cap)
        .ok_or("账户单次或剩余导出额度不足最小分页，未发送搜索".into())
}
fn points(v: &Value) -> Option<i64> {
    if let Some(n) = v.as_i64() {
        return (n >= 0).then_some(n);
    }
    let s = v.as_str()?.trim();
    if s.bytes().all(|b| b.is_ascii_digit()) && !s.is_empty() {
        return s.parse().ok();
    }
    let n = s
        .strip_prefix("消耗积分：")
        .or_else(|| s.strip_prefix("消耗积分:"))?
        .trim();
    if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
        None
    } else {
        n.parse().ok()
    }
}
fn cost(payload: &Value, _before: &Value, _after: &Value) -> Option<i64> {
    // Account deltas can contain other clients' usage, top-ups or delayed debit.
    // Only the official page receipt proves this query's cost. Keep unknown
    // distinct from zero and pause before another paid search if it is absent.
    points(&payload["data"]["consume_quota"])
}

fn row_identity(row: &Value) -> String {
    let link = row["link"].as_str().unwrap_or("");
    if let Ok(u) = reqwest::Url::parse(link) {
        format!("{}|{}", row["protocol"].as_str().unwrap_or(""), u)
    } else {
        format!(
            "{}|{}|{}",
            row["protocol"].as_str().unwrap_or(""),
            row["host"].as_str().unwrap_or(""),
            row["port"].as_str().unwrap_or("")
        )
    }
}
/// Return relation-verified candidates, including IP-only TLS and non-Web services.
fn rows(payload: &Value, kind: &str, target: &str) -> Result<(Vec<Value>, usize), String> {
    if payload["code"].as_i64() != Some(200) {
        return Err(format!(
            "Hunter 拒绝本页（code={}）：{}；不会自动重发",
            payload["code"],
            payload["message"]
                .as_str()
                .unwrap_or("语法、权限、额度或接口限制")
                .chars()
                .take(300)
                .collect::<String>()
        ));
    }
    let raw = payload["data"]["arr"]
        .as_array()
        .ok_or("Hunter 本页缺少资产列表，已保留响应")?;
    let mut accepted = vec![];
    for item in raw {
        let relation = match kind {
            "fingerprint" => {
                item["cert_sha256"]
                    .as_str()
                    .and_then(fingerprint)
                    .as_deref()
                    == Some(target)
            }
            "cname" => field_hosts(&item["cname_target"])
                .iter()
                .any(|(h, w)| !w && h == target),
            "domain" => {
                host(item["domain"].as_str().unwrap_or("")).as_deref() == Some(target)
                    || host(item["url"].as_str().unwrap_or("")).as_deref() == Some(target)
            }
            "domain_tree" => ["domain", "url"].iter().any(|k| {
                host(item[*k].as_str().unwrap_or("")).is_some_and(|h| in_scope(&h, target))
            }),
            _ => false,
        };
        if !relation {
            continue;
        }
        let ip = item["ip"].as_str().unwrap_or("");
        if !ip.is_empty() && ip.parse::<std::net::IpAddr>().is_err() {
            continue;
        }
        let Some(port) = item["port"]
            .as_u64()
            .or_else(|| item["port"].as_str().and_then(|p| p.parse().ok()))
            .filter(|p| (1..=65535).contains(p))
        else {
            continue;
        };
        let protocol = item["protocol"]
            .as_str()
            .unwrap_or("tcp")
            .to_ascii_lowercase();
        if protocol.is_empty()
            || protocol.len() > 40
            || !protocol
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.+".contains(&b))
        {
            continue;
        }
        let domain = host(item["domain"].as_str().unwrap_or(""));
        let fallback = domain.clone().unwrap_or(ip.into());
        if fallback.is_empty() {
            continue;
        }
        let (endpoint, link) = if matches!(protocol.as_str(), "http" | "https") {
            let authority = if fallback.parse::<std::net::Ipv6Addr>().is_ok() {
                format!("[{fallback}]")
            } else {
                fallback.clone()
            };
            let fallback_url = format!("{protocol}://{authority}:{port}/");
            let url = item["url"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or(&fallback_url);
            let Ok(u) = reqwest::Url::parse(url) else {
                continue;
            };
            if u.scheme() != protocol
                || !u.username().is_empty()
                || u.password().is_some()
                || u.port_or_known_default().map(u64::from) != Some(port)
            {
                continue;
            }
            let Some(h) = u.host_str() else { continue };
            if kind == "domain"
                && h.parse::<std::net::IpAddr>().is_err()
                && !h.eq_ignore_ascii_case(target)
            {
                continue;
            }
            if kind == "domain_tree"
                && h.parse::<std::net::IpAddr>().is_err()
                && !in_scope(h, target)
            {
                continue;
            }
            (h.to_string(), u.to_string())
        } else {
            (fallback, String::new())
        };
        accepted.push(json!({"host":endpoint,"link":link,"ip":ip,"port":port.to_string(),"protocol":protocol,"domain":domain.unwrap_or_default(),"title":item["web_title"].as_str().unwrap_or(""),"status_code":item["status_code"].as_i64().map(|n|n.to_string()).unwrap_or_default(),"lastupdatetime":item["updated_at"].as_str().unwrap_or(""),"cert_sha256":item["cert_sha256"].as_str().unwrap_or(""),"ssl_certificate":item["ssl_certificate"].as_str().unwrap_or(""),"cname_target":item["cname_target"].as_str().unwrap_or(""),"hunter_query_kind":kind,"evidence":format!("Hunter精确关联 {kind}: {target}；待核验业务归属"),"matched_queries":format!("Hunter {kind}: {target}"),"score":"40"}));
    }
    let rejected = raw.len() - accepted.len();
    Ok((accepted, rejected))
}
fn scan<P: Provider, F: Fn() -> bool, I: FnMut(&str, &[Value]) -> Result<Value, String>>(
    c: &mut Connection,
    p: i64,
    id: &str,
    provider: &mut P,
    stopped: F,
    mut import: I,
) -> Result<Value, String> {
    let (kind,target,query,start,end,account_hash):(String,String,String,String,String,String)=c.query_row("SELECT kind,target,query,start_date,end_date,account_hash FROM hunter_expansions WHERE id=?1 AND project_id=?2",params![id,p],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).map_err(err)?;
    loop {
        if stopped()
            || c.query_row(
                "SELECT status='stopping' FROM hunter_expansions WHERE id=?1",
                [id],
                |r| r.get::<_, bool>(0),
            )
            .map_err(err)?
        {
            finish(
                c,
                id,
                "paused",
                "已停止，保留下页游标；已发出的请求不会自动重放",
            )?;
            break;
        }
        project(c, p)?;
        evidence_anchor(c, p, &target, &kind)?;
        let (page, mut size): (i64, i64) = c
            .query_row(
                "SELECT next_page,page_size FROM hunter_expansions WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(err)?;
        let old_total: Option<i64> = c
            .query_row(
                "SELECT total FROM hunter_expansions WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(err)?;
        if size > 0 && page > 1 && old_total.is_some_and(|t| (page - 1) * size >= t) {
            finish(
                c,
                id,
                "partial",
                "已到已报告分页末尾，仍有未验证证据；不会额外请求越界页，请先核对原始回执",
            )?;
            break;
        }
        let saved:Option<(String,String,String)>=c.query_row("SELECT state,payload_json,request_id FROM hunter_expansion_pages WHERE query_id=?1 AND page=?2",params![id,page],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(err)?;
        let (request_id, payload) = if let Some((state, payload, request_id)) = saved {
            if state == "sending" {
                finish(
                    c,
                    id,
                    "unknown",
                    "本页请求结果未知，可能已扣费；禁止自动重发，需先核对账户和回执",
                )?;
                break;
            }
            (
                request_id,
                serde_json::from_str::<Value>(&payload).map_err(err)?,
            )
        } else {
            let before = provider.account()?;
            if balance(&before) == Some(0) {
                finish(c, id, "quota", "免费积分和权益积分均已用尽，已保留下页")?;
                break;
            }
            if size == 0 {
                size = page_size(&before, setting(c)?.3)?;
                c.execute(
                    "UPDATE hunter_expansions SET page_size=?2 WHERE id=?1",
                    params![id, size],
                )
                .map_err(err)?;
            }
            let _ = &account_hash;
            let request_id = uuid::Uuid::new_v4().to_string();
            c.execute("INSERT INTO hunter_expansion_pages(query_id,page,state,request_id) VALUES(?1,?2,'sending',?3)",params![id,page,request_id]).map_err(err)?;
            if stopped() {
                c.execute("DELETE FROM hunter_expansion_pages WHERE query_id=?1 AND page=?2 AND state='sending'",params![id,page]).map_err(err)?;
                finish(c, id, "paused", "请求尚未发出，已停止")?;
                break;
            }
            let payload = match provider.search(&query, page, size, &start, &end, &kind) {
                Ok(v) => v,
                Err(e) => {
                    finish(
                        c,
                        id,
                        "unknown",
                        &format!("第{page}页远端结果未确认：{e}；保留游标，不自动重试"),
                    )?;
                    break;
                }
            };
            // Save the complete page before any further network call or local import.
            c.execute("UPDATE hunter_expansion_pages SET state='received',payload_json=?3 WHERE query_id=?1 AND page=?2",params![id,page,payload.to_string()]).map_err(err)?;
            let after = provider.account().unwrap_or(Value::Null);
            let charge = cost(&payload, &before, &after);
            let quota = payload["data"]["consume_quota"].as_str().unwrap_or("");
            let rest = payload["data"]["rest_quota"].as_str().unwrap_or("");
            let tx = c
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(err)?;
            tx.execute("UPDATE hunter_expansion_pages SET cost=?3,accounted=1,cost_text=?4,rest_text=?5 WHERE query_id=?1 AND page=?2",params![id,page,charge,quota,rest]).map_err(err)?;
            tx.execute("UPDATE hunter_expansions SET spent=spent+?2,cost_unknown=MAX(cost_unknown,?3),account_json=?4 WHERE id=?1",params![id,charge.unwrap_or(0),charge.is_none(),after.to_string()]).map_err(err)?;
            tx.commit().map_err(err)?;
            (request_id, payload)
        };
        let accounted: bool = c
            .query_row(
                "SELECT accounted FROM hunter_expansion_pages WHERE query_id=?1 AND page=?2",
                params![id, page],
                |r| r.get(0),
            )
            .map_err(err)?;
        if !accounted {
            let charge = points(&payload["data"]["consume_quota"]);
            let tx = c
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(err)?;
            tx.execute("UPDATE hunter_expansion_pages SET accounted=1,cost=?3,cost_text=?4,rest_text=?5 WHERE query_id=?1 AND page=?2",params![id,page,charge,payload["data"]["consume_quota"].as_str().unwrap_or(""),payload["data"]["rest_quota"].as_str().unwrap_or("")]).map_err(err)?;
            tx.execute("UPDATE hunter_expansions SET spent=spent+?2,cost_unknown=MAX(cost_unknown,?3) WHERE id=?1",params![id,charge.unwrap_or(0),charge.is_none()]).map_err(err)?;
            tx.commit().map_err(err)?;
        }
        if payload["code"].as_i64() != Some(200) {
            finish(
                c,
                id,
                "provider_limit",
                &format!(
                    "第{page}页未成功：{}；请核对语法/权限/积分或平台分页限制，未假装完成",
                    payload["message"].as_str().unwrap_or("未知接口错误")
                ),
            )?;
            break;
        }
        let Some(raw) = payload["data"]["arr"].as_array() else {
            finish(c, id, "partial", "本页结构不完整，响应已保存；不会跳过该页")?;
            break;
        };
        let Some(total) = payload["data"]["total"].as_i64().filter(|t| *t >= 0) else {
            finish(
                c,
                id,
                "partial",
                "本页缺少有效总量，无法判定完整覆盖；响应已保存",
            )?;
            break;
        };
        if raw.len() > size as usize {
            finish(
                c,
                id,
                "partial",
                "本页条数超过分页契约；响应已保存，未截断后冒充完整",
            )?;
            break;
        }
        let hash = format!("{:x}", Sha256::digest(json!(raw).to_string().as_bytes()));
        let repeated:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM hunter_expansion_pages WHERE query_id=?1 AND page<?2 AND page_hash=?3 AND state='imported')",params![id,page,hash],|r|r.get(0)).map_err(err)?;
        if repeated && !raw.is_empty() {
            finish(
                c,
                id,
                "partial",
                "平台重复返回已读页，游标未假装前进；本页扣费保留在账本",
            )?;
            break;
        }
        if raw.is_empty() && total > (page - 1) * size {
            finish(
                c,
                id,
                "partial",
                "平台提前返回空页但仍报告后续资产，已暂停并保留缺口",
            )?;
            break;
        }
        let (parsed, rejected) = rows(&payload, &kind, &target)?;
        let mut unique = HashSet::new();
        let fresh=parsed.iter().filter(|r|unique.insert(row_identity(r))).filter_map(|r|{let key=row_identity(r);match c.query_row("SELECT EXISTS(SELECT 1 FROM hunter_expansion_links WHERE project_id=?1 AND query_id=?2 AND endpoint=?3)",params![p,id,key],|r|r.get::<_,bool>(0)){Ok(false)=>Some(Ok(r.clone())),Ok(true)=>None,Err(e)=>Some(Err(err(e)))}}).collect::<Result<Vec<_>,_>>()?;
        let receipt = import(&request_id, &fresh)?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(err)?;
        for r in &fresh {
            tx.execute("INSERT OR IGNORE INTO hunter_expansion_links(project_id,endpoint,query_id,page,evidence_json) VALUES(?1,?2,?3,?4,?5)",params![p,row_identity(r),id,page,r.to_string()]).map_err(err)?;
        }
        tx.execute("UPDATE hunter_expansion_pages SET state='imported',page_hash=?3,raw_count=?4,accepted=?5,rejected=?6,import_json=?7 WHERE query_id=?1 AND page=?2",params![id,page,hash,raw.len() as i64,fresh.len() as i64,rejected as i64,receipt.to_string()]).map_err(err)?;
        tx.execute("UPDATE hunter_expansions SET next_page=next_page+1,index_changed=MAX(index_changed,CASE WHEN total IS NOT NULL AND total<>?2 THEN 1 ELSE 0 END),total=MAX(COALESCE(total,0),?2),received=received+?3,unique_rows=unique_rows+?4,rejected=rejected+?5,updated_at=datetime('now','localtime') WHERE id=?1",params![id,total,raw.len() as i64,fresh.len() as i64,rejected as i64]).map_err(err)?;
        tx.commit().map_err(err)?;
        let (frozen_total, rejected_all, unknown): (i64, i64, bool) = c
            .query_row(
                "SELECT total,rejected,cost_unknown FROM hunter_expansions WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(err)?;
        if rejected > 0 {
            finish(c,id,"partial","本页含无法核对精确关联或身份的记录，原始页已保存；为避免错误消耗已暂停，未跳过后续缺口")?;
            break;
        }
        if unknown {
            finish(
                c,
                id,
                "partial",
                "本页已保存并入库，但积分消耗无法确认；已暂停继续扣费",
            )?;
            break;
        }
        let changed: bool = c
            .query_row(
                "SELECT index_changed FROM hunter_expansions WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(err)?;
        if page * size >= frozen_total && changed {
            finish(
                c,
                id,
                "partial",
                "已读取报告的分页范围，但平台总量发生变化；实时索引不提供快照保证，不能证明无遗漏",
            )?;
            break;
        }
        if page * size >= frozen_total {
            finish(
                c,
                id,
                if rejected_all == 0 && !unknown {
                    "completed"
                } else {
                    "partial"
                },
                if rejected_all == 0 && !unknown {
                    "已遍历该精确查询当前报告的全部分页；关联证据不代表业务归属确认，实时索引变化仍可能影响覆盖"
                } else {
                    "分页已读到末尾，但存在无法核对的关联记录或积分消耗，不能宣称完整验收"
                },
            )?;
            break;
        }
        if raw.len() < size as usize {
            finish(
                c,
                id,
                "partial",
                "平台返回短页但总量仍有后续，未按末页处理；下页游标已保留",
            )?;
            break;
        }
    }
    status(c, p, id)
}
pub(crate) fn run_inline(
    path: &Path,
    data: &Path,
    input: ExpansionInput,
    stopped: impl Fn() -> bool,
) -> Result<Value, String> {
    let mut c = db::open(path)?;
    ensure(&c)?;
    let (_, key, _, _) = setting(&c)?;
    if key.is_empty() {
        return Err("请先保存 Hunter Key".into());
    }
    let id = prepare(&mut c, &input, &key)?;
    run_existing(path, data, input.project_id, &id, &key, stopped)
}
fn process_alive(pid: i64) -> bool {
    if pid <= 0 {
        return false;
    }
    #[cfg(unix)]
    {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        true
    }
}
struct WorkerGuard {
    path: std::path::PathBuf,
    id: String,
}
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        if let Ok(c) = db::open(&self.path) {
            let _=c.execute("UPDATE hunter_expansions SET status='paused',reason='分页进程已退出；保留回执和游标，未知请求不会自动重放' WHERE id=?1 AND status IN ('running','stopping')",[&self.id]);
        }
        if let Ok(mut g) = running().lock() {
            g.remove(&self.id);
        }
    }
}
fn run_existing(
    path: &Path,
    data: &Path,
    p: i64,
    id: &str,
    key: &str,
    stopped: impl Fn() -> bool,
) -> Result<Value, String> {
    let mut c = db::open(path)?;
    let token = Arc::new(AtomicBool::new(false));
    {
        let mut g = running().lock().map_err(|_| "Hunter 状态锁异常")?;
        if g.contains_key(id) {
            return Err("此查询正在分页，请勿重复启动".into());
        }
        let state: String = c
            .query_row(
                "SELECT status FROM hunter_expansions WHERE id=?1 AND project_id=?2",
                params![id, p],
                |r| r.get(0),
            )
            .map_err(err)?;
        if state == "completed" {
            drop(g);
            return status(&c, p, id);
        }
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(err)?;
        let account: String = tx
            .query_row(
                "SELECT account_hash FROM hunter_expansions WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(err)?;
        let active = {
            let mut s=tx.prepare("SELECT id,owner_pid FROM hunter_expansions WHERE account_hash=?1 AND status IN ('running','stopping')").map_err(err)?;
            let rows = s
                .query_map([account], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
                })
                .map_err(err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(err)?;
            rows
        };
        for (old, owner) in active {
            if process_alive(owner) && (owner != std::process::id() as i64 || g.contains_key(&old))
            {
                return Err("此账户已有分页进程，等待或暂停后才能继续".into());
            }
            tx.execute("UPDATE hunter_expansions SET status='paused',reason='旧进程已退出，保留本页游标' WHERE id=?1",[old]).map_err(err)?;
        }
        tx.execute("UPDATE hunter_expansions SET status='running',owner_pid=?2,reason='正在按精确条件自动翻页' WHERE id=?1",params![id,std::process::id()]).map_err(err)?;
        tx.commit().map_err(err)?;
        g.insert(id.into(), token.clone());
    }
    let _guard = WorkerGuard {
        path: path.to_path_buf(),
        id: id.to_string(),
    };
    let result = (|| {
        let settings = crate::model_configuration::default_settings_for(&c, &["network"]);
        if settings.get("_configurationError").is_some() {
            return Err("业务网络配置不可用".into());
        }
        let client = super::hunter_network::client(&c, &settings, 60)?;
        let mut provider = Live { client, key };
        scan(
            &mut c,
            p,
            id,
            &mut provider,
            || token.load(Ordering::SeqCst) || stopped(),
            |request, rows| {
                if rows.is_empty() {
                    return Ok(json!({"newRows":0}));
                }
                crate::jobs::import_hunter_discovery(
                    path,
                    data,
                    p,
                    &c_anchor(path, id)?,
                    request,
                    rows,
                )
            },
        )
    })();
    if let Err(e) = &result {
        let _ = finish(&c, id, "paused", e);
    }
    if let Ok(mut g) = running().lock() {
        g.remove(id);
    }
    result.or_else(|_| status(&c, p, id))
}
fn c_anchor(path: &Path, id: &str) -> Result<String, String> {
    db::open(path)?
        .query_row(
            "SELECT anchor FROM hunter_expansions WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .map_err(err)
}
fn record_start_failure(path: &Path, id: &str, reason: &str) {
    if let Ok(g) = running().lock() {
        if g.contains_key(id) {
            return;
        }
    }
    if let Ok(c) = db::open(path) {
        let _=c.execute("UPDATE hunter_expansions SET reason=?2 WHERE id=?1 AND status NOT IN ('running','stopping')",params![id,reason]);
    }
}
#[tauri::command]
pub async fn start_hunter_expansion(
    app: tauri::AppHandle,
    input: ExpansionInput,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let s = app.state::<AppState>();
        let mut c = db::open(&s.db_path)?;
        ensure(&c)?;
        let (_, key, _, _) = setting(&c)?;
        if key.is_empty() {
            return Err("请先保存 Hunter Key".into());
        }
        let id = prepare(&mut c, &input, &key)?;
        let path = s.db_path.clone();
        let data = s.app_data_dir.clone();
        let worker = id.clone();
        std::thread::Builder::new()
            .name("hunter-exact-pages".into())
            .spawn(move || {
                if let Err(e) =
                    run_existing(&path, &data, input.project_id, &worker, &key, || false)
                {
                    record_start_failure(&path, &worker, &e);
                }
            })
            .map_err(err)?;
        Ok(id)
    })
    .await
    .map_err(err)?
}
#[tauri::command]
pub async fn stop_hunter_expansion(
    app: tauri::AppHandle,
    project_id: i64,
    query_id: String,
) -> Result<(), String> {
    let path = app.state::<AppState>().db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let c = db::open(&path)?;
        let _ = status(&c, project_id, &query_id)?;
        if let Some(token) = running()
            .lock()
            .map_err(|_| "Hunter 状态锁异常")?
            .get(&query_id)
        {
            token.store(true, Ordering::SeqCst);
        }
        let (state, owner): (String, i64) = c
            .query_row(
                "SELECT status,owner_pid FROM hunter_expansions WHERE id=?1 AND project_id=?2",
                params![query_id, project_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(err)?;
        if matches!(state.as_str(), "running" | "stopping") {
            finish(
                &c,
                &query_id,
                if process_alive(owner) {
                    "stopping"
                } else {
                    "paused"
                },
                "正在等待当前页返回；未知请求不会自动重发",
            )?;
        }
        Ok(())
    })
    .await
    .map_err(err)?
}
#[tauri::command]
pub async fn resume_hunter_expansion(
    app: tauri::AppHandle,
    project_id: i64,
    query_id: String,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let s = app.state::<AppState>();
        let c = db::open(&s.db_path)?;
        let _ = status(&c, project_id, &query_id)?;
        let (_, key, _, _) = setting(&c)?;
        let expected: String = c
            .query_row(
                "SELECT account_hash FROM hunter_expansions WHERE id=?1",
                [&query_id],
                |r| r.get(0),
            )
            .map_err(err)?;
        if expected != format!("{:x}", Sha256::digest(key.as_bytes())) {
            return Err("Hunter 账户 Key 已变化，不能把原游标接到另一个账户".into());
        }
        let path = s.db_path.clone();
        let data = s.app_data_dir.clone();
        let worker = query_id.clone();
        std::thread::spawn(move || {
            if let Err(e) = run_existing(&path, &data, project_id, &worker, &key, || false) {
                record_start_failure(&path, &worker, &e);
            }
        });
        Ok(query_id)
    })
    .await
    .map_err(err)?
}

#[cfg(test)]
#[path = "hunter_expansion_tests.rs"]
mod tests;
