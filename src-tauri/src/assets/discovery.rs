//! Operator-controlled discovery leads. Search hits never grant ownership or scan permission.
use crate::{db, AppState};
use base64::Engine;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, io::Read, path::Path, time::Duration};
use tauri::Manager;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS asset_discovery_settings(id INTEGER PRIMARY KEY CHECK(id=1),revision INTEGER NOT NULL,hunter_key TEXT NOT NULL,max_requests INTEGER NOT NULL,max_results INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS asset_discovery_scopes(project_id INTEGER PRIMARY KEY REFERENCES projects(id),roots_json TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS asset_discovery_seeds(id INTEGER PRIMARY KEY,project_id INTEGER NOT NULL REFERENCES projects(id),host TEXT NOT NULL,source TEXT NOT NULL,source_ref TEXT NOT NULL,wildcard INTEGER NOT NULL,UNIQUE(project_id,host,source));
CREATE INDEX IF NOT EXISTS asset_discovery_seeds_project ON asset_discovery_seeds(project_id,id);
CREATE TABLE IF NOT EXISTS asset_discovery_seed_evidence(project_id INTEGER NOT NULL,host TEXT NOT NULL,source TEXT NOT NULL,source_ref TEXT NOT NULL,wildcard INTEGER NOT NULL,PRIMARY KEY(project_id,host,source,source_ref));
CREATE INDEX IF NOT EXISTS asset_discovery_evidence_reference ON asset_discovery_seed_evidence(project_id,source_ref,source);
CREATE TABLE IF NOT EXISTS asset_discovery_requests(request_id TEXT PRIMARY KEY,project_id INTEGER NOT NULL,query_hash TEXT NOT NULL,day TEXT NOT NULL,state TEXT NOT NULL,result_json TEXT NOT NULL DEFAULT '{}');
CREATE INDEX IF NOT EXISTS asset_discovery_requests_budget ON asset_discovery_requests(project_id,day);
";
#[path = "discovery_analysis.rs"]
pub(crate) mod analysis;
#[path = "certificate_tasks.rs"]
pub(crate) mod certificate_tasks;
#[path = "discovery_enrichment.rs"]
pub(crate) mod enrichment;
#[path = "discovery_evidence.rs"]
pub(crate) mod evidence;
#[path = "discovery_exclusions.rs"]
pub(crate) mod exclusions;
#[path = "discovery_harvest.rs"]
pub(crate) mod harvest;
#[path = "hunter_diagnostics.rs"]
mod hunter_diagnostics;
#[path = "hunter_expansion.rs"]
pub(crate) mod hunter_expansion;
#[path = "hunter_network.rs"]
mod hunter_network;
#[path = "discovery_local.rs"]
pub(crate) mod local;
#[path = "discovery_organization_evidence.rs"]
pub(crate) mod organization_evidence;
#[path = "discovery_passive_evidence.rs"]
pub(crate) mod passive_evidence;
#[path = "discovery_review.rs"]
pub(crate) mod review;
#[path = "discovery_rounds.rs"]
pub(crate) mod rounds;
#[path = "discovery_seed_ledger.rs"]
pub(crate) mod seed_ledger;
#[path = "discovery_seed_page.rs"]
pub(crate) mod seed_page;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn exists(c: &Connection) -> Result<bool, String> {
    c.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='asset_discovery_settings')",[],|r|r.get(0)).map_err(err)
}
fn ensure(c: &Connection) -> Result<(), String> {
    c.execute_batch(SCHEMA).map_err(err)
}
fn project(c: &Connection, id: i64) -> Result<(), String> {
    let live: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1 AND status='active')",
            [id],
            |r| r.get(0),
        )
        .map_err(err)?;
    if live {
        Ok(())
    } else {
        Err("请选择有效的当前工作空间".into())
    }
}
fn setting(c: &Connection) -> Result<(i64, String, i64, i64), String> {
    if !exists(c)? {
        return Ok((0, String::new(), 3, 10));
    }
    Ok(c.query_row("SELECT revision,hunter_key,max_requests,max_results FROM asset_discovery_settings WHERE id=1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(err)?.unwrap_or((0,String::new(),3,10)))
}
pub(crate) fn settings_on(c: &Connection) -> Result<Value, String> {
    let (rev, key, requests, results) = setting(c)?;
    Ok(
        json!({"revision":rev,"keyConfigured":!key.is_empty(),"contractVerified":HUNTER_CONTRACT_VERIFIED,"maxRequests":requests,"maxResults":results,"networkMode":hunter_network::mode(c)?}),
    )
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HunterSettingsInput {
    pub expected_revision: i64,
    pub key: Option<String>,
    pub clear_key: bool,
    pub max_requests: i64,
    pub max_results: i64,
    #[serde(default)]
    pub network_mode: Option<String>,
}
pub(crate) fn save_settings_on(
    c: &mut Connection,
    input: HunterSettingsInput,
) -> Result<Value, String> {
    if input
        .network_mode
        .as_deref()
        .is_some_and(|m| !["direct", "business"].contains(&m))
    {
        return Err("Hunter 连接方式无效".into());
    }
    if !(0..=1000).contains(&input.max_requests) || !(10..=100).contains(&input.max_results) {
        return Err("Hunter 每项目每日查询上限为 0–1000（0 不限制），每页 10–100 条".into());
    }
    ensure(c)?;
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    let (revision, current, _, _) = setting(&tx)?;
    if revision != input.expected_revision {
        return Err("Hunter 设置已变化，请重新读取后保存".into());
    }
    let key = if input.clear_key {
        String::new()
    } else {
        input
            .key
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(current)
            .trim()
            .to_string()
    };
    if key.len() > 512 || key.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("Hunter Key 格式无效".into());
    }
    tx.execute("INSERT INTO asset_discovery_settings VALUES(1,?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,hunter_key=excluded.hunter_key,max_requests=excluded.max_requests,max_results=excluded.max_results",params![revision+1,key,input.max_requests,input.max_results]).map_err(err)?;
    if let Some(mode) = input.network_mode {
        tx.execute_batch("CREATE TABLE IF NOT EXISTS asset_discovery_hunter_network(id INTEGER PRIMARY KEY CHECK(id=1),mode TEXT NOT NULL CHECK(mode IN ('direct','business')))").map_err(err)?;
        tx.execute("INSERT INTO asset_discovery_hunter_network VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET mode=excluded.mode", [mode]).map_err(err)?;
    }
    tx.commit().map_err(err)?;
    settings_on(c)
}
/// Parse DNS identifiers only. Never take a URL's path/query or an email as a host.
pub(crate) fn host(raw: &str) -> Option<String> {
    let raw = raw.trim().trim_end_matches('.').trim_start_matches("*.");
    if raw.is_empty()
        || raw.len() > 253
        || raw.contains('@')
        || raw.chars().any(char::is_whitespace)
    {
        return None;
    }
    let url = if raw.contains("://") {
        reqwest::Url::parse(raw)
    } else {
        reqwest::Url::parse(&format!("https://{raw}"))
    }
    .ok()?;
    if !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    let h = url.host_str()?.trim_end_matches('.').to_ascii_lowercase();
    if h.parse::<std::net::IpAddr>().is_ok()
        || !h.contains('.')
        || h.split('.').any(|s| {
            s.is_empty()
                || s.len() > 63
                || s.starts_with('-')
                || s.ends_with('-')
                || !s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
    {
        return None;
    }
    Some(h)
}
pub(crate) fn in_scope(h: &str, root: &str) -> bool {
    h == root || h.ends_with(&format!(".{root}"))
}
/// Bounded extraction from actual collected HTML/JS; no external script fetch or execution.
pub(crate) fn body_hosts(body: &str) -> Vec<String> {
    let re = regex::Regex::new(
        r#"(?i)(?:https?:)?//([a-z0-9][a-z0-9._-]*\.[a-z0-9.-]+)(?::\d+)?(?:[/\s"'<>]|$)"#,
    )
    .expect("constant regex");
    let mut end = body.len().min(524288);
    while !body.is_char_boundary(end) {
        end -= 1;
    }
    let bounded = body[..end].replace("\\/", "/");
    re.captures_iter(&bounded)
        .filter_map(|c| host(&c[1]))
        .take(128)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
pub(crate) fn field_hosts(value: &Value) -> Vec<(String, bool)> {
    if let Some(a) = value.as_array() {
        return a.iter().take(128).flat_map(field_hosts).collect();
    }
    let Some(s) = value.as_str() else {
        return vec![];
    };
    let parsed = serde_json::from_str::<Value>(s).ok();
    if let Some(v) = parsed.filter(|v| v.is_array()) {
        return field_hosts(&v);
    }
    s.split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '|'))
        .take(128)
        .filter_map(|s| host(s).map(|h| (h, s.starts_with("*."))))
        .collect()
}
pub(crate) fn roots(c: &Connection, p: i64) -> Result<Vec<String>, String> {
    let mut all = BTreeSet::new();
    if exists(c)? {
        if let Some(s) = c
            .query_row(
                "SELECT roots_json FROM asset_discovery_scopes WHERE project_id=?1",
                [p],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(err)?
        {
            all.extend(serde_json::from_str::<Vec<String>>(&s).map_err(err)?);
        }
    }
    if let Some(s) = c
        .query_row(
            "SELECT approved_domains_json FROM asset_ownership_profiles WHERE project_id=?1",
            [p],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(err)?
    {
        all.extend(
            serde_json::from_str::<Vec<String>>(&s)
                .unwrap_or_default()
                .iter()
                .filter_map(|s| host(s)),
        );
    }
    Ok(all.into_iter().collect())
}
fn insert_seed(
    c: &Connection,
    p: i64,
    h: &str,
    source: &str,
    reference: &str,
    wildcard: bool,
) -> Result<usize, String> {
    let added = c.execute("INSERT OR IGNORE INTO asset_discovery_seeds(project_id,host,source,source_ref,wildcard) VALUES(?1,?2,?3,?4,?5)",params![p,h,source,reference,wildcard]).map_err(err)?;
    c.execute("INSERT OR IGNORE INTO asset_discovery_seed_evidence(project_id,host,source,source_ref,wildcard) VALUES(?1,?2,?3,?4,?5)",params![p,h,source,reference,wildcard]).map_err(err)?;
    exclusions::observe(c,p,h,source,reference)?;
    Ok(added)
}
pub(crate) fn ingest_observations(
    c: &Connection,
    p: i64,
    rows: &[(i64, String, String, String)],
) -> Result<usize, String> {
    ensure(c)?;
    let mut count = 0;
    for (id, host_value, link, extra) in rows {
        let reference = format!("asset:{id}");
        let v = serde_json::from_str::<Value>(extra).unwrap_or(json!({}));
        if let Some(h) = host(if link.is_empty() { host_value } else { link }) {
            count += insert_seed(c, p, &h, "observed_host", &reference, false)?;
        }
        for key in ["cert_sha256", "cert.sha256", "cert.sha-256", "tls_sha256"] {
            if let Some(hash) = v[key].as_str().and_then(hunter_expansion::fingerprint) {
                count += insert_seed(c, p, &hash, "certificate_sha256", &reference, false)?;
            }
        }
        count += local::ingest_certificate_metadata(c, p, &reference, &v)?;
        for (key, source) in [
            ("cert.domain", "certificate_domain"),
            ("cert.subject.cn", "certificate_cn"),
            ("cname", "cname"),
            ("cname_target", "cname"),
            ("discovery_hosts_json", "page_or_inline_script"),
        ] {
            for (h, w) in field_hosts(&v[key]) {
                count += insert_seed(c, p, &h, source, &reference, w)?;
            }
        }
    }
    Ok(count)
}
pub(crate) fn view(c: &Connection, p: i64) -> Result<Value, String> {
    view_filtered(c, p, "", "all", 1, 200)
}
pub(crate) fn view_filtered(
    c: &Connection,
    p: i64,
    search: &str,
    source: &str,
    page: i64,
    size: i64,
) -> Result<Value, String> {
    project(c, p)?;
    let source_sql = match source {
        "all" | "excluded" => "1=1",
        "certificate" => "source IN ('certificate_domain','certificate_cn','certificate_sha256','certificate_transparency')",
        "cname" => "source IN ('cname','dns_cname')",
        "host" => "source='observed_host'",
        "page" => "source='page_or_inline_script'",
        _ => return Err("线索分类无效".into()),
    };
    if search.chars().count() > 253 || !(1..=1000000).contains(&page) || !(1..=200).contains(&size)
    {
        return Err("线索查询参数无效".into());
    }
    let pattern = format!(
        "%{}%",
        search
            .trim()
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    let hidden = if exclusions::exists(c)? {exclusions::hidden_sql()}else{"0"};
    let show_excluded = source=="excluded";
    let visibility = if source=="excluded" {hidden.to_string()}else{format!("NOT {hidden}")};
    let filter = format!("project_id=?1 AND {source_sql} AND host LIKE ?2 ESCAPE '\\' AND {visibility}");
    let scope = roots(c, p)?;
    let mut seeds = vec![];
    let mut seed_total = 0i64;
    let mut all_seeds = 0i64;
    let mut effective_page = page;
    if exists(c)? {
        all_seeds = c
            .query_row(
                &format!("SELECT count(*) FROM asset_discovery_seeds WHERE project_id=?1 AND NOT {hidden}"),
                [p],
                |r| r.get(0),
            )
            .map_err(err)?;
        seed_total = c
            .query_row(
                &format!("SELECT count(*) FROM asset_discovery_seeds WHERE {filter}"),
                params![p, pattern],
                |r| r.get(0),
            )
            .map_err(err)?;
        effective_page = page.min(((seed_total + size - 1) / size).max(1));
        let mut s=c.prepare(&format!("SELECT id,host,source,source_ref,wildcard FROM asset_discovery_seeds WHERE {filter} ORDER BY id DESC LIMIT ?3 OFFSET ?4")).map_err(err)?;
        let rows = s
            .query_map(
                params![p, pattern, size, (effective_page - 1) * size],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, bool>(4)?,
                    ))
                },
            )
            .map_err(err)?;
        for row in rows {
            let (id, h, source, reference, wildcard) = row.map_err(err)?;
            let eligible = !show_excluded && scope.iter().any(|r| in_scope(&h, r)) && !wildcard;
            seeds.push(json!({"id":id,"host":h,"source":source,"reference":reference,"wildcard":wildcard,"excluded":show_excluded,"eligible":eligible,"queryEligible":!show_excluded && if source=="certificate_sha256" { hunter_expansion::evidence_anchor(c,p,&h,"fingerprint").is_ok() } else if source=="cname"||source=="dns_cname" { hunter_expansion::evidence_anchor(c,p,&h,"cname").is_ok() } else { scope.iter().any(|r|in_scope(&h,r)) && (!wildcard || source.starts_with("certificate")) }}));
        }
    }
    let mut requests = vec![];
    if exists(c)? {
        let mut s=c.prepare("SELECT request_id,day,state,result_json FROM asset_discovery_requests WHERE project_id=?1 ORDER BY rowid DESC LIMIT 10").map_err(err)?;
        for r in s
            .query_map([p], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(err)?
        {
            let (id, day, state, result) = r.map_err(err)?;
            requests.push(json!({"id":id,"day":day,"state":state,"receipt":serde_json::from_str::<Value>(&result).unwrap_or(json!({}))}));
        }
    }
    Ok(
        json!({"roots":scope,"seeds":seeds,"seedTotal":seed_total,"allSeeds":all_seeds,"seedPage":effective_page,"seedPageSize":size,"requests":requests,"settings":settings_on(c)?,"expansions":hunter_expansion::list_on(c,p)?,"harvest":harvest::status_on(c,p)?}),
    )
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiscoveryInput {
    pub project_id: i64,
    pub action: String,
    #[serde(default)]
    pub roots: Vec<String>,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub cursor: i64,
    #[serde(default)]
    pub request_id: String,
    #[serde(default = "hunter_default_kind")]
    pub hunter_kind: String,
}
fn hunter_default_kind() -> String {
    "domain".into()
}
fn network_client(proxy: &str) -> Result<reqwest::blocking::Client, String> {
    let mut b = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none());
    if !proxy.is_empty() {
        b = b.proxy(reqwest::Proxy::all(proxy).map_err(|_| "业务代理无效")?)
    }
    b.build().map_err(|_| "无法初始化发现客户端".into())
}
fn get_json(
    client: &reqwest::blocking::Client,
    url: reqwest::Url,
    key: &str,
) -> Result<Value, String> {
    let response = client
        .get(url)
        .send()
        .map_err(|_| "外部请求未完成；没有自动重试")?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "外部接口返回 HTTP {}；403 表示访问被拒绝，不能据此判断 Key 错误。未自动重试",
            status.as_u16()
        ));
    }
    let mut b = vec![];
    response
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut b)
        .map_err(|_| "读取外部响应失败")?;
    if b.len() > 2 * 1024 * 1024 {
        return Err("外部响应超过 2 MiB；结果未采纳".into());
    }
    let mut text = String::from_utf8(b).map_err(|_| "外部响应编码无效")?;
    if !key.is_empty() {
        text = text.replace(key, "[REDACTED]")
    }
    serde_json::from_str(&text).map_err(|_| "外部接口未返回有效 JSON".into())
}
pub(crate) fn approved_host(c: &Connection, p: i64, h: &str) -> Result<String, String> {
    let h = host(h).ok_or("域名格式无效")?;
    if exclusions::blocked_name(c,p,&h)? { return Err("该域名已被当前项目排除；可在线索库恢复排除规则后再补查".into()); }
    if !roots(c, p)?.iter().any(|r| in_scope(&h, r)) {
        return Err("只允许补查明确确认的域名范围；证书、CNAME、名称命中不能自动扩大范围".into());
    }
    Ok(h)
}
include!("discovery_hunter.rs");
fn hunter_reserve(
    c: &mut Connection,
    p: i64,
    id: &str,
    hash: &str,
    limit: i64,
) -> Result<Option<Value>, String> {
    uuid::Uuid::parse_str(id).map_err(|_| "Hunter 请求编号无效")?;
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    let old:Option<(i64,String,String,String)>=tx.query_row("SELECT project_id,query_hash,state,result_json FROM asset_discovery_requests WHERE request_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(err)?;
    if let Some((op, oh, state, result)) = old {
        if op != p || oh != hash {
            return Err("Hunter 请求编号与原参数不一致".into());
        }
        if state == "completed" {
            return Ok(Some(serde_json::from_str(&result).map_err(err)?));
        }
        return Err("原 Hunter 请求结果未确认，不自动重放，以免重复计费".into());
    }
    let day = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let uncertain:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM asset_discovery_requests WHERE project_id=?1 AND query_hash=?2 AND state<>'completed')",params![p,hash],|r|r.get(0)).map_err(err)?;
    if uncertain {
        return Err("同一 Hunter 查询有未确认请求；不自动再次扣费".into());
    }
    let cached:Option<String>=tx.query_row("SELECT result_json FROM asset_discovery_requests WHERE project_id=?1 AND query_hash=?2 AND day=?3 AND state='completed' LIMIT 1",params![p,hash,day],|r|r.get(0)).optional().map_err(err)?;
    if let Some(s) = cached {
        return Ok(Some(serde_json::from_str(&s).map_err(err)?));
    }
    let used: i64 = tx
        .query_row(
            "SELECT count(*) FROM asset_discovery_requests WHERE project_id=?1 AND day=?2",
            params![p, day],
            |r| r.get(0),
        )
        .map_err(err)?;
    if used >= limit {
        return Err("此项目今日 Hunter 请求预算已用尽（含失败及结果未知的请求）".into());
    }
    tx.execute("INSERT INTO asset_discovery_requests(request_id,project_id,query_hash,day,state) VALUES(?1,?2,?3,?4,'pending')",params![id,p,hash,day]).map_err(err)?;
    tx.commit().map_err(err)?;
    Ok(None)
}
pub(crate) fn action_on(path: &Path, data: &Path, input: DiscoveryInput) -> Result<Value, String> {
    action_on_limited(path, data, input, None)
}
/// Autonomous calls use a hard result cap and require known available free quota.
pub(crate) fn action_on_limited(
    path: &Path,
    data: &Path,
    input: DiscoveryInput,
    _result_cap: Option<i64>,
) -> Result<Value, String> {
    let mut c = db::open(path)?;
    project(&c, input.project_id)?;
    ensure(&c)?;
    match input.action.as_str() {
        "scopes" => {
            if input.roots.len() > 500 {
                return Err("最多确认 500 个域名范围；请优先整理已核实的根域或具体租户".into());
            }
            let mut roots = BTreeSet::new();
            for r in input.roots {
                let h = host(&r).ok_or("域名范围格式无效")?;
                if crate::commands::ownership::ownership_domain_parts(&h)
                    .and_then(|p| p.registrable)
                    .is_none()
                {
                    return Err("不能将公共后缀或共享平台本身作为目标范围".into());
                }
                roots.insert(h);
            }
            c.execute("INSERT INTO asset_discovery_scopes VALUES(?1,?2) ON CONFLICT(project_id) DO UPDATE SET roots_json=excluded.roots_json",params![input.project_id,serde_json::to_string(&roots).map_err(err)?]).map_err(err)?;
        }
        "promote" => {
            let h = approved_host(&c, input.project_id, &input.host)?;
            c.execute("INSERT OR IGNORE INTO targets(project_id,target_type,value,normalized_value,enabled) VALUES(?1,'domain',?2,?2,1)",params![input.project_id,h]).map_err(err)?;
        }
        "harvest" => {
            let rows = {
                let mut s=c.prepare("SELECT a.id,a.host,a.link,a.extra_json FROM assets a JOIN project_assets pa ON pa.asset_id=a.id WHERE pa.project_id=?1 AND pa.is_deleted=0 AND a.id>?2 ORDER BY a.id LIMIT 1000").map_err(err)?;
                let x = s
                    .query_map(params![input.project_id, input.cursor], |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, String>(3)?,
                        ))
                    })
                    .map_err(err)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(err)?;
                x
            };
            let next = rows.last().map(|r| r.0).unwrap_or(input.cursor);
            let tx = c.transaction().map_err(err)?;
            ingest_observations(&tx, input.project_id, &rows)?;
            tx.commit().map_err(err)?;
            let mut v = view(&c, input.project_id)?;
            v["nextCursor"] = json!(next);
            v["batchRows"] = json!(rows.len());
            return Ok(v);
        }
        "ct" | "dns" => {
            let h = approved_host(&c, input.project_id, &input.host)?;
            let settings = crate::model_configuration::default_settings_for(&c, &["network"]);
            if settings.get("_configurationError").is_some() {
                return Err("业务网络配置不可用".into());
            }
            let client = network_client(settings["proxyUrl"].as_str().unwrap_or(""))?;
            let mut url = reqwest::Url::parse(if input.action == "ct" {
                "https://crt.sh/"
            } else {
                "https://dns.google/resolve"
            })
            .unwrap();
            if input.action == "ct" {
                url.query_pairs_mut()
                    .append_pair("q", &format!("%.{h}"))
                    .append_pair("output", "json");
            } else {
                url.query_pairs_mut()
                    .append_pair("name", &h)
                    .append_pair("type", "CNAME")
                    .append_pair("edns_client_subnet", "0.0.0.0/0");
            }
            let payload = get_json(&client, url, "")?;
            let tx = c.transaction().map_err(err)?;
            if input.action == "ct" {
                let entries = payload.as_array().ok_or("CT 未返回列表")?;
                for raw in entries.iter().take(1000) {
                    for (name, wildcard) in field_hosts(&raw["name_value"]) {
                        if in_scope(&name, &h) {
                            insert_seed(
                                &tx,
                                input.project_id,
                                &name,
                                "certificate_transparency",
                                &h,
                                wildcard,
                            )?;
                        }
                    }
                }
            } else {
                if payload["Status"] != 0 || payload["TC"] == true {
                    return Err("DNS 未返回完整有效结果".into());
                }
                if let Some(a) = payload["Answer"].as_array() {
                    for raw in a.iter().take(20) {
                        if raw["type"] == 5 {
                            for (name, wildcard) in field_hosts(&raw["data"]) {
                                insert_seed(
                                    &tx,
                                    input.project_id,
                                    &name,
                                    "dns_cname",
                                    &h,
                                    wildcard,
                                )?;
                            }
                        }
                    }
                }
            }
            tx.commit().map_err(err)?;
        }
        "hunter" => {
            let receipt = hunter_expansion::run_inline(
                path,
                data,
                hunter_expansion::ExpansionInput {
                    project_id: input.project_id,
                    kind: input.hunter_kind,
                    target: input.host,
                },
                || false,
            )?;
            return Ok(json!({"receipt":receipt,"view":view(&c,input.project_id)?,"cached":false}));
        }
        _ => return Err("未知发现操作".into()),
    }
    view(&c, input.project_id)
}
#[tauri::command]
pub async fn get_hunter_discovery_settings(app: tauri::AppHandle) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        settings_on(&db::open(&app.state::<AppState>().db_path)?)
    })
    .await
    .map_err(err)?
}
#[tauri::command]
pub async fn save_hunter_discovery_settings(
    app: tauri::AppHandle,
    input: HunterSettingsInput,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        save_settings_on(&mut db::open(&app.state::<AppState>().db_path)?, input)
    })
    .await
    .map_err(err)?
}
#[tauri::command]
pub async fn test_hunter_discovery_api(app: tauri::AppHandle) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let c = db::open(&app.state::<AppState>().db_path)?;
        let (_, key, _, _) = setting(&c)?;
        if key.is_empty() {
            return Err("请先保存 Hunter Key，再测试账户连接".into());
        }
        let settings = crate::model_configuration::default_settings_for(&c, &["network"]);
        if settings.get("_configurationError").is_some() {
            return Err("业务网络配置不可用".into());
        }
        let diagnostic = hunter_diagnostics::Context::new(&app.state::<AppState>().db_path, hunter_network::mode(&c)?, None, None);
        hunter_user_info(&hunter_network::client(&c, &settings, 20)?, &key, &diagnostic)
    })
    .await
    .map_err(err)?
}
#[tauri::command]
pub async fn get_asset_discovery(app: tauri::AppHandle, project_id: i64) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        view(&db::open(&app.state::<AppState>().db_path)?, project_id)
    })
    .await
    .map_err(err)?
}
#[tauri::command]
pub async fn act_asset_discovery(
    app: tauri::AppHandle,
    input: DiscoveryInput,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        use tauri::Manager;
        let s = app.state::<AppState>();
        action_on(&s.db_path, &s.app_data_dir, input)
    })
    .await
    .map_err(err)?
}

#[cfg(test)]
#[path = "discovery_tests.rs"]
mod tests;
