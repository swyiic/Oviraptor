//! Source-bound legal-name research, separate from network authorization.
//! A known official source is re-read; a model cannot invent directory entries.
use super::*;

const SOURCE: &str = "https://www.chinamobileltd.com/sc/about/overview.php";
const RELATIONSHIP_SOURCES: &[&str] = &[
    SOURCE,
    "https://hdh.10086.cn/mobile/introduce.jsp",
    "https://www.cmccsi.cn/aboutus/",
    "https://job.10086.cn/personal/job/detail.html?id=0df9d17f-0979-4044-a7b7-3a1156da7ed1&typess=1",
];
const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS asset_discovery_organization_evidence(project_id INTEGER NOT NULL,organization_key TEXT NOT NULL,source_url TEXT NOT NULL,body_sha256 TEXT NOT NULL DEFAULT '',names_json TEXT NOT NULL DEFAULT '[]',checked_at INTEGER NOT NULL DEFAULT 0,retry_at INTEGER NOT NULL DEFAULT 0,error TEXT NOT NULL DEFAULT '',PRIMARY KEY(project_id,organization_key,source_url));";

fn target_key(s: &str) -> String {
    let mut s = s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    for suffix in [
        "及其下属单位",
        "及其下属公司",
        "及其子公司",
        "及下属单位",
        "及子公司",
    ] {
        if let Some(prefix) = s.strip_suffix(suffix) {
            s = prefix.into();
            break;
        }
    }
    s
}
fn supported(s: &str) -> bool {
    matches!(
        target_key(s).as_str(),
        "中国移动" | "中国移动集团" | "中国移动通信集团" | "中国移动通信集团有限公司"
    )
}
fn text(html: &str) -> String {
    let scripts = regex::Regex::new(r"(?is)<(?:script|style)\b[^>]*>.*?</(?:script|style)>")
        .expect("static regex");
    let tags = regex::Regex::new(r"(?s)<[^>]*>").expect("static regex");
    tags.replace_all(&scripts.replace_all(html, " "), " ")
        .replace("&nbsp;", " ")
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}
fn extract(html: &str) -> Result<Vec<String>, String> {
    let t = text(html);
    let identity = t.contains("中国移动有限公司（「本公司」")
        || t.contains("中国移动有限公司（“本公司”")
        || t.contains("中国移动有限公司(「本公司」");
    let control = t.contains("本公司的最终控股股东是中国移动通信集团有限公司")
        || t.contains("公司的最终控股股东是中国移动通信集团有限公司");
    if !identity || !control {
        return Err(
            "官方页面未返回所需的法定名称与控股关系正文；未采纳网页标题、挑战页或模型推断".into(),
        );
    }
    // This verifies precisely two named legal entities. Other companies,
    // associates, banks and service customers do not inherit this relation.
    Ok(vec![
        "中国移动通信集团有限公司".into(),
        "中国移动有限公司".into(),
    ])
}

/// Only explicit legal-entity control statements on known official sources.
/// Customers, procurement counterparties and service providers do not inherit
/// a subsidiary relationship just because their names occur on the page.
fn extract_source(source: &str, html: &str) -> Result<Vec<String>, String> {
    if source == SOURCE {
        return extract(html);
    }
    let t = text(html).replace('（', "(").replace('）', ")");
    let valid = if source == RELATIONSHIP_SOURCES[1] {
        t.contains("中移互联网有限公司(以下简称“中移互联网公司”)是中国移动负责全网互联网业务集中运营的专业子公司")
            || t.contains("中移互联网有限公司(以下简称\"中移互联网公司\")是中国移动负责全网互联网业务集中运营的专业子公司")
    } else if source == RELATIONSHIP_SOURCES[2] {
        t.contains("中移系统集成有限公司成立于1999年，是中国移动集团下属")
            && t.contains("全资子公司")
    } else if source == RELATIONSHIP_SOURCES[3] {
        t.contains("所属公司：中移(苏州)软件技术有限公司")
            && t.contains("中国移动云公司是中国移动通信集团的全资子公司")
    } else {
        false
    };
    if !valid {
        return Err(
            "官方页面未返回完整法定主体与子公司关系正文，未采用标题、脚本、招聘供应商或客户名称"
                .into(),
        );
    }
    Ok(vec![if source == RELATIONSHIP_SOURCES[1] {
        "中移互联网有限公司"
    } else if source == RELATIONSHIP_SOURCES[2] {
        "中移系统集成有限公司"
    } else {
        "中移（苏州）软件技术有限公司"
    }
    .into()])
}
/// Bounded fixed official sources, independently cached by target and URL.
/// One source being a JS shell, unavailable or rate limited never invalidates
/// other successful relationships. No candidate endpoint or paid API is used.
pub(crate) fn refresh(
    path: &Path,
    p: i64,
    organization: &str,
    proxy: &str,
) -> Result<Value, String> {
    if !supported(organization) {
        return Ok(
            json!({"supported":false,"reason":"按输入法定名称及已确认单位目录核验；尚无适用的自动官方关系来源"}),
        );
    }
    let c = db::open(path)?;
    project(&c, p)?;
    c.execute_batch(SCHEMA).map_err(err)?;
    let key = target_key(organization);
    let at = chrono::Utc::now().timestamp();
    let client = passive_evidence::client(proxy)?;
    let mut sources = Vec::new();
    let mut next = None::<i64>;
    let mut requested = false;
    for source in RELATIONSHIP_SOURCES {
        let prior:Option<(i64,i64,String)>=c.query_row("SELECT checked_at,retry_at,error FROM asset_discovery_organization_evidence WHERE project_id=?1 AND organization_key=?2 AND source_url=?3",params![p,key,source],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(err)?;
        if let Some((checked, retry, error)) =
            prior.filter(|(checked, retry, _)| *retry > at || *checked > at - 86400)
        {
            if retry > at {
                next = Some(next.map_or(retry, |n| n.min(retry)));
            }
            sources.push(json!({"sourceUrl":source,"cached":true,"checkedAt":checked,"error":error,"retryAt":retry}));
            continue;
        }
        requested = true;
        let response = (|| -> Result<(String, Vec<String>), String> {
            let r = client
                .get(*source)
                .timeout(std::time::Duration::from_secs(12))
                .send()
                .map_err(|e| format!("官方单位关系来源请求失败：{}", e.without_url()))?;
            if !r.status().is_success() {
                return Err(format!(
                    "官方单位关系来源 HTTP {}：{source}",
                    r.status().as_u16()
                ));
            }
            if r.content_length().is_some_and(|n| n > 2 * 1024 * 1024) {
                return Err("官方单位关系页面超出2MiB，未解析".into());
            }
            let mut bytes = Vec::new();
            r.take(2 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| format!("官方关系正文读取失败：{e}"))?;
            if bytes.len() > 2 * 1024 * 1024 {
                return Err("官方单位关系页面超出2MiB，未解析".into());
            }
            let html = std::str::from_utf8(&bytes).map_err(|_| "官方关系正文编码无效，未采纳")?;
            let ns = extract_source(source, html)?;
            Ok((format!("{:x}", Sha256::digest(&bytes)), ns))
        })();
        match response {
            Ok((hash, ns)) => {
                c.execute("INSERT INTO asset_discovery_organization_evidence(project_id,organization_key,source_url,body_sha256,names_json,checked_at) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(project_id,organization_key,source_url) DO UPDATE SET body_sha256=excluded.body_sha256,names_json=excluded.names_json,checked_at=excluded.checked_at,retry_at=0,error=''",params![p,key,source,hash,json!(ns).to_string(),at]).map_err(err)?;
                sources.push(json!({"sourceUrl":source,"networkVerified":true,"bodySha256":hash,"checkedAt":at,"names":ns}));
            }
            Err(e) => {
                let retry = at + 3600;
                next = Some(next.map_or(retry, |n| n.min(retry)));
                c.execute("INSERT INTO asset_discovery_organization_evidence(project_id,organization_key,source_url,retry_at,error) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(project_id,organization_key,source_url) DO UPDATE SET retry_at=excluded.retry_at,error=excluded.error",params![p,key,source,retry,e]).map_err(err)?;
                sources.push(
                    json!({"sourceUrl":source,"networkVerified":false,"error":e,"retryAt":retry}),
                );
            }
        }
    }
    let ns = names(&c, p, organization)?;
    Ok(
        json!({"supported":true,"networkRequested":requested,"networkVerified":!ns.is_empty(),"names":ns,"sources":sources,"retryAt":next.unwrap_or(0),"scopeAuthorized":false}),
    )
}

pub(crate) fn names(c: &Connection, p: i64, organization: &str) -> Result<Vec<String>, String> {
    if !supported(organization) {
        return Ok(vec![]);
    }
    let exists:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='asset_discovery_organization_evidence')",[],|r|r.get(0)).map_err(err)?;
    if !exists {
        return Ok(vec![]);
    }
    let allowed = json!(RELATIONSHIP_SOURCES).to_string();
    let rows:Vec<String>=c.prepare("SELECT names_json FROM asset_discovery_organization_evidence WHERE project_id=?1 AND organization_key=?2 AND source_url IN (SELECT value FROM json_each(?3)) AND checked_at>=?4 AND length(body_sha256)=64").map_err(err)?.query_map(params![p,target_key(organization),allowed,chrono::Utc::now().timestamp()-30*86400],|r|r.get(0)).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;
    let mut ns = BTreeSet::new();
    for row in rows {
        ns.extend(serde_json::from_str::<Vec<String>>(&row).map_err(err)?);
    }
    Ok(ns.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_official_statement_does_not_adopt_other_names_or_a_title() {
        let page="<title>中国移动</title><p>中国移动有限公司（「本公司」，包括子公司合称为「本集团」）</p><p>本公司的最终控股股东是中国移动通信集团有限公司（简称「中国移动集团公司」）。</p><p>参股：邮储银行；客户：grcbank.com；中移不相关公司</p>";
        assert_eq!(
            extract(page).unwrap(),
            vec!["中国移动通信集团有限公司", "中国移动有限公司"]
        );
        assert!(extract("<title>中国移动有限公司（「本公司」</title><script>本公司的最终控股股东是中国移动通信集团有限公司</script>").is_err());
        assert!(extract("<title>中国移动</title><p>需要验证后访问</p>").is_err());
        assert!(!supported("中国移动支付服务有限公司"));
        assert!(!supported("其他集团"));
    }
    #[test]
    fn captured_relationship_is_project_target_and_freshness_bound() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(SCHEMA).unwrap();
        let now = chrono::Utc::now().timestamp();
        c.execute("INSERT INTO asset_discovery_organization_evidence VALUES(1,'中国移动集团',?1,?2,'[\"中国移动通信集团有限公司\"]',?3,0,'')",params![SOURCE,"a".repeat(64),now]).unwrap();
        assert_eq!(names(&c, 1, "中国移动集团及其下属单位").unwrap().len(), 1);
        assert!(names(&c, 2, "中国移动集团").unwrap().is_empty());
        assert!(names(&c, 1, "其他集团").unwrap().is_empty());
        c.execute(
            "UPDATE asset_discovery_organization_evidence SET checked_at=?1",
            [now - 31 * 86400],
        )
        .unwrap();
        assert!(names(&c, 1, "中国移动集团").unwrap().is_empty());
    }
    #[test]
    fn official_subsidiary_relation_does_not_promote_operating_partner_or_shell() {
        let source = RELATIONSHIP_SOURCES[1];
        let page="<p>中移互联网有限公司(以下简称“中移互联网公司”)是中国移动负责全网互联网业务集中运营的专业子公司</p><p>杭州东信北邮信息技术有限公司是授权的网络运营单位</p>";
        assert_eq!(
            extract_source(source, page).unwrap(),
            vec!["中移互联网有限公司"]
        );
        assert!(extract_source(
            source,
            "<title>中移互联网有限公司</title><script>子公司关系</script>"
        )
        .is_err());
        assert!(extract_source("https://untrusted.example/", page).is_err());
    }
    #[test]
    #[ignore = "explicit public-source network validation, never paid or a candidate target"]
    fn live_official_company_relationship() {
        let dir =
            std::env::temp_dir().join(format!("ovi-official-evidence-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let path = crate::db::initialize(&dir).unwrap();
        let c = db::open(&path).unwrap();
        c.execute(
            "INSERT INTO projects(id,name) VALUES(91001,'public research validation')",
            [],
        )
        .unwrap();
        let v = refresh(&path, 91001, "中国移动集团及其下属单位", "").unwrap();
        assert_eq!(v["networkVerified"], true, "{v}");
        assert!(names(&c, 91001, "中国移动集团及其下属单位").unwrap().len() >= 2);
        drop(c);
        let _ = std::fs::remove_dir_all(dir);
    }
}
