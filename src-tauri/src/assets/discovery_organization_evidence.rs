//! Source-bound legal-name research, separate from network authorization.
//! A known official source is re-read; a model cannot invent directory entries.
use super::*;

const SOURCE: &str = "https://www.chinamobileltd.com/sc/about/overview.php";
const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS asset_discovery_organization_evidence(project_id INTEGER NOT NULL,organization_key TEXT NOT NULL,source_url TEXT NOT NULL,body_sha256 TEXT NOT NULL DEFAULT '',names_json TEXT NOT NULL DEFAULT '[]',checked_at INTEGER NOT NULL DEFAULT 0,retry_at INTEGER NOT NULL DEFAULT 0,error TEXT NOT NULL DEFAULT '',PRIMARY KEY(project_id,organization_key,source_url));";

fn target_key(s: &str) -> String {
    let mut s = s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    for suffix in ["及其下属单位", "及其下属公司", "及其子公司", "及下属单位", "及子公司"] {
        if let Some(prefix) = s.strip_suffix(suffix) { s = prefix.into(); break; }
    }
    s
}
fn supported(s: &str) -> bool {
    matches!(target_key(s).as_str(), "中国移动" | "中国移动集团" | "中国移动通信集团" | "中国移动通信集团有限公司")
}
fn text(html: &str) -> String {
    let scripts = regex::Regex::new(r"(?is)<(?:script|style)\b[^>]*>.*?</(?:script|style)>").expect("static regex");
    let tags = regex::Regex::new(r"(?s)<[^>]*>").expect("static regex");
    tags.replace_all(&scripts.replace_all(html, " "), " ")
        .replace("&nbsp;", " ").chars().filter(|c| !c.is_whitespace()).collect()
}
fn extract(html: &str) -> Result<Vec<String>, String> {
    let t = text(html);
    let identity = t.contains("中国移动有限公司（「本公司」")
        || t.contains("中国移动有限公司（“本公司”")
        || t.contains("中国移动有限公司(「本公司」");
    let control = t.contains("本公司的最终控股股东是中国移动通信集团有限公司")
        || t.contains("公司的最终控股股东是中国移动通信集团有限公司");
    if !identity || !control {
        return Err("官方页面未返回所需的法定名称与控股关系正文；未采纳网页标题、挑战页或模型推断".into());
    }
    // This verifies precisely two named legal entities. Other companies,
    // associates, banks and service customers do not inherit this relation.
    Ok(vec!["中国移动通信集团有限公司".into(), "中国移动有限公司".into()])
}

/// One bounded public GET, cached by project/target/source. No target connection,
/// model request, paid API call, scope write or company-directory mutation.
pub(crate) fn refresh(path: &Path, p: i64, organization: &str, proxy: &str) -> Result<Value, String> {
    if !supported(organization) {
        return Ok(json!({"supported":false,"reason":"按输入法定名称及已确认单位目录核验；尚无适用的自动官方关系来源"}));
    }
    let c=db::open(path)?; project(&c,p)?; c.execute_batch(SCHEMA).map_err(err)?;
    let key=target_key(organization); let now=chrono::Utc::now().timestamp();
    let prior:Option<(i64,i64,String)>=c.query_row("SELECT checked_at,retry_at,error FROM asset_discovery_organization_evidence WHERE project_id=?1 AND organization_key=?2 AND source_url=?3",params![p,key,SOURCE],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(err)?;
    if prior.as_ref().is_some_and(|(checked,retry,_)|*retry>now||*checked>now-86400) {
        let ns=names(&c,p,organization)?;
        let (checked,_,error)=prior.expect("checked prior");
        return Ok(json!({"supported":true,"cached":true,"networkRequested":false,"networkVerified":!ns.is_empty(),"sourceUrl":SOURCE,"checkedAt":checked,"names":ns,"error":error,"scopeAuthorized":false}));
    }
    let response=(||->Result<(String,Vec<String>),String>{
        let client=passive_evidence::client(proxy)?;
        let r=client.get(SOURCE).send().map_err(|e|format!("官方单位关系来源请求失败：{}",e.without_url()))?;
        if !r.status().is_success(){return Err(format!("官方单位关系来源 HTTP {}：{SOURCE}",r.status().as_u16()))}
        if r.content_length().is_some_and(|n|n>2*1024*1024){return Err("官方单位关系页面超出2MiB，未解析".into())}
        let mut bytes=Vec::new();r.take(2*1024*1024+1).read_to_end(&mut bytes).map_err(|e|format!("官方关系正文读取失败：{e}"))?;
        if bytes.len()>2*1024*1024{return Err("官方单位关系页面超出2MiB，未解析".into())}
        let html=std::str::from_utf8(&bytes).map_err(|_|"官方关系正文编码无效，未采纳")?;
        let names=extract(html)?;Ok((format!("{:x}",Sha256::digest(&bytes)),names))
    })();
    match response {
        Ok((hash,ns))=>{
            c.execute("INSERT INTO asset_discovery_organization_evidence(project_id,organization_key,source_url,body_sha256,names_json,checked_at) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(project_id,organization_key,source_url) DO UPDATE SET body_sha256=excluded.body_sha256,names_json=excluded.names_json,checked_at=excluded.checked_at,retry_at=0,error=''",params![p,key,SOURCE,hash,json!(ns).to_string(),now]).map_err(err)?;
            Ok(json!({"supported":true,"networkVerified":true,"sourceUrl":SOURCE,"bodySha256":hash,"checkedAt":now,"names":ns,"scopeAuthorized":false}))
        }
        Err(e)=>{
            c.execute("INSERT INTO asset_discovery_organization_evidence(project_id,organization_key,source_url,retry_at,error) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(project_id,organization_key,source_url) DO UPDATE SET retry_at=excluded.retry_at,error=excluded.error",params![p,key,SOURCE,now+3600,e]).map_err(err)?;
            Ok(json!({"supported":true,"networkVerified":false,"sourceUrl":SOURCE,"names":names(&c,p,organization)?,"error":e,"retryAt":now+3600}))
        }
    }
}

pub(crate) fn names(c:&Connection,p:i64,organization:&str)->Result<Vec<String>,String>{
    if !supported(organization){return Ok(vec![])}
    let exists:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='asset_discovery_organization_evidence')",[],|r|r.get(0)).map_err(err)?;
    if !exists{return Ok(vec![])}
    let raw:Option<String>=c.query_row("SELECT names_json FROM asset_discovery_organization_evidence WHERE project_id=?1 AND organization_key=?2 AND source_url=?3 AND checked_at>=?4 AND length(body_sha256)=64",params![p,target_key(organization),SOURCE,chrono::Utc::now().timestamp()-30*86400],|r|r.get(0)).optional().map_err(err)?;
    raw.map(|s|serde_json::from_str(&s).map_err(err)).transpose().map(Option::unwrap_or_default)
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn exact_official_statement_does_not_adopt_other_names_or_a_title(){
        let page="<title>中国移动</title><p>中国移动有限公司（「本公司」，包括子公司合称为「本集团」）</p><p>本公司的最终控股股东是中国移动通信集团有限公司（简称「中国移动集团公司」）。</p><p>参股：邮储银行；客户：grcbank.com；中移不相关公司</p>";
        assert_eq!(extract(page).unwrap(),vec!["中国移动通信集团有限公司","中国移动有限公司"]);
        assert!(extract("<title>中国移动有限公司（「本公司」</title><script>本公司的最终控股股东是中国移动通信集团有限公司</script>").is_err());
        assert!(extract("<title>中国移动</title><p>需要验证后访问</p>").is_err());
        assert!(!supported("中国移动支付服务有限公司"));assert!(!supported("其他集团"));
    }
    #[test]
    fn captured_relationship_is_project_target_and_freshness_bound(){
        let c=Connection::open_in_memory().unwrap();c.execute_batch(SCHEMA).unwrap();let now=chrono::Utc::now().timestamp();
        c.execute("INSERT INTO asset_discovery_organization_evidence VALUES(1,'中国移动集团',?1,?2,'[\"中国移动通信集团有限公司\"]',?3,0,'')",params![SOURCE,"a".repeat(64),now]).unwrap();
        assert_eq!(names(&c,1,"中国移动集团及其下属单位").unwrap().len(),1);
        assert!(names(&c,2,"中国移动集团").unwrap().is_empty());assert!(names(&c,1,"其他集团").unwrap().is_empty());
        c.execute("UPDATE asset_discovery_organization_evidence SET checked_at=?1",[now-31*86400]).unwrap();assert!(names(&c,1,"中国移动集团").unwrap().is_empty());
    }
    #[test]
    #[ignore = "explicit public-source network validation, never paid or a candidate target"]
    fn live_official_company_relationship(){
        let dir=std::env::temp_dir().join(format!("ovi-official-evidence-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&dir).unwrap();
        let path=crate::db::initialize(&dir).unwrap();let c=db::open(&path).unwrap();c.execute("INSERT INTO projects(id,name) VALUES(91001,'public research validation')",[]).unwrap();
        let v=refresh(&path,91001,"中国移动集团及其下属单位","").unwrap();assert_eq!(v["networkVerified"],true,"{v}");assert_eq!(names(&c,91001,"中国移动集团及其下属单位").unwrap().len(),2);drop(c);let _=std::fs::remove_dir_all(dir);
    }
}
