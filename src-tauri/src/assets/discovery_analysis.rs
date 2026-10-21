//! Full local observation scan for operator-reviewed seed suggestions. No network,
//! credentials, raw pages or automatic ownership/paid-query admission.
use super::*;
use std::collections::BTreeMap;
#[derive(Default)]
struct Candidate {
    assets: BTreeSet<i64>, sources: BTreeSet<String>, companies: BTreeSet<String>,
    examples: BTreeSet<i64>, fingerprints: BTreeSet<String>, quarantined: usize,
}
fn subject_organizations(v: &Value) -> Vec<String> {
    [&v["cert.subject.org"], &v["cert.subject.o"], &v["subject"]["O"], &v["subject"]["organizationName"], &v["cert"]["subject"]["O"], &v["certificate"]["subject"]["O"]]
        .into_iter().filter_map(Value::as_str).map(|s|s.chars().take(160).collect()).collect()
}
pub(crate) fn report_on(c: &Connection, p: i64, organization: &str) -> Result<Value,String> {
    let name:String=c.query_row("SELECT name FROM projects WHERE id=?1",[p],|r|r.get(0)).map_err(|_|"工作空间不存在")?;
    let organization=organization.trim();
    if organization.chars().count()>240 { return Err("单位或域名关键词最多240字".into()); }
    let terms=organization.split(|ch:char|ch.is_whitespace()||matches!(ch,','|';'|'，'|'；')).filter(|s|!s.is_empty()).map(str::to_lowercase).collect::<Vec<_>>();
    let confirmed=roots(c,p)?; let exclusions=exclusions::rules(c,p)?;
    let mut candidates=BTreeMap::<String,Candidate>::new(); let mut scanned=0; let mut matched=0; let mut certificates=0;
    let mut company_counts=BTreeMap::<String,usize>::new();
    let mut statement=c.prepare("SELECT a.id,a.host,a.link,a.company,a.extra_json,a.content_category FROM assets a JOIN project_assets pa ON pa.asset_id=a.id WHERE pa.project_id=?1 AND pa.is_deleted=0 ORDER BY a.id").map_err(err)?;
    let mut rows=statement.query([p]).map_err(err)?;
    while let Some(row)=rows.next().map_err(err)? {
        scanned+=1; let id:i64=row.get(0).map_err(err)?; let h:String=row.get(1).map_err(err)?; let link:String=row.get(2).map_err(err)?; let company:String=row.get(3).map_err(err)?;
        let extra:String=row.get(4).map_err(err)?; let category:String=row.get(5).map_err(err)?;
        let v=serde_json::from_str::<Value>(&extra).unwrap_or(Value::Null); let facts=local::certificate_facts(&v);
        if facts.present() {certificates+=1;}
        let organizations=subject_organizations(&v);
        if !company.is_empty(){*company_counts.entry(company.chars().take(160).collect()).or_default()+=1;}
        let mut observations=BTreeSet::<(String,String)>::new();
        if let Some(h)=host(if link.is_empty(){&h}else{&link}) {observations.insert((h,"observed_host".into()));}
        for (h,_) in &facts.names {observations.insert((h.clone(),"certificate_name".into()));}
        for key in ["cert.domain","cert.subject.cn","cname","cname_target","discovery_hosts_json"] {
            let source=if key.contains("cname"){"cname"}else if key=="discovery_hosts_json"{"page"}else{"certificate_name"};
            for (h,_) in field_hosts(&v[key]) {observations.insert((h,source.into()));}
        }
        // Company/subject matches are only relevance hints. ISP, ASN, issuer and page body
        // do not establish a company relationship; no endpoint grants ownership.
        let company_match=terms.iter().any(|t|company.to_lowercase().contains(t)||organizations.iter().any(|o|o.to_lowercase().contains(t)));
        let relevant=terms.is_empty()||company_match||observations.iter().any(|(h,_)|terms.iter().any(|t|h.contains(t)));
        if !relevant {continue;} matched+=1;
        for (h,source) in observations {
            if exclusions::matches(&h,&exclusions) {continue;}
            if !terms.is_empty()&&!company_match&&!terms.iter().any(|t|h.contains(t)) {continue;}
            let entry=candidates.entry(h).or_default(); entry.assets.insert(id);entry.sources.insert(source);
            if entry.examples.len()<3 {entry.examples.insert(id);}
            if !company.is_empty()&&entry.companies.len()<3 {entry.companies.insert(company.chars().take(160).collect());}
            for fingerprint in facts.hashes.iter().take(3) {if entry.fingerprints.len()<3{entry.fingerprints.insert(fingerprint.clone());}}
            if category.starts_with("blocked")||category=="isolated" {entry.quarantined+=1;}
        }
    }
    let mut companies=company_counts.into_iter().map(|(name,count)|json!({"name":name,"assetCount":count})).collect::<Vec<_>>();
    companies.sort_by(|a,b|b["assetCount"].as_u64().cmp(&a["assetCount"].as_u64()).then_with(||a["name"].as_str().cmp(&b["name"].as_str())));companies.truncate(20);
    let total=candidates.len();
    let mut entries=candidates.into_iter().map(|(h,v)|{
        let within=confirmed.iter().any(|r|in_scope(&h,r));
        json!({"host":h,"assetCount":v.assets.len(),"assetIds":v.examples,"sources":v.sources,"companies":v.companies,"fingerprints":v.fingerprints,"quarantinedCount":v.quarantined,"inConfirmedScope":within,"ownershipConfirmed":false,"hunterAuthorized":false})
    }).collect::<Vec<_>>();
    entries.sort_by(|a,b|b["inConfirmedScope"].as_bool().cmp(&a["inConfirmedScope"].as_bool()).then_with(||b["assetCount"].as_u64().cmp(&a["assetCount"].as_u64())).then_with(||a["host"].as_str().cmp(&b["host"].as_str())));
    entries.truncate(80);
    Ok(json!({"projectId":p,"projectName":name,"organization":organization,"roots":confirmed,"scannedAssets":scanned,"matchedAssets":matched,"certificateAssets":certificates,"companyObservations":companies,"candidateTotal":total,"candidates":entries,"previewTruncated":total>80,"networkRequested":false,"dataModified":false,"note":"全量遍历非回收资产；仅返回本地观察摘要，前80个候选供审阅。单位、同机证书与CNAME共现不是归属证明；外部域名和混合证书须人工核验，确认前不进入Hunter。可缩小单位或域名关键词重新分析。"}))
}
#[tauri::command]
pub async fn analyze_local_discovery(state:tauri::State<'_,AppState>,project_id:i64,organization:String)->Result<Value,String>{
    let path=state.db_path.clone();tauri::async_runtime::spawn_blocking(move||{
        let c=Connection::open_with_flags(path,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(err)?;
        c.execute_batch("BEGIN").map_err(err)?;report_on(&c,project_id,&organization)
    }).await.map_err(err)?
}
#[derive(Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct ConfirmInput { project_id:i64, organization:String, hosts:Vec<String>, expected_roots:Vec<String>, user_confirmed:bool }
pub(crate) fn confirm_on(c:&mut Connection,input:ConfirmInput)->Result<Value,String>{
    if !input.user_confirmed||input.hosts.is_empty()||input.hosts.len()>20 {return Err("请明确确认所选域名及其子域与目标单位相关，最多20项".into());}
    ensure(c)?;
    let tx=c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(err)?;
    project(&tx,input.project_id)?;
    let mut confirmed=roots(&tx,input.project_id)?.into_iter().collect::<BTreeSet<_>>();
    if confirmed!=input.expected_roots.into_iter().collect::<BTreeSet<_>>(){return Err("确认范围已被修改，请重新读取候选后核对".into());}
    let report=report_on(&tx,input.project_id,&input.organization)?;
    for value in input.hosts {
        let h=host(&value).filter(|h|h==&value).ok_or("候选域名格式无效")?;
        if !report["candidates"].as_array().is_some_and(|rows|rows.iter().any(|r|r["host"]==h)) {return Err("候选依据已变化，请重新分析；不能确认未展示的域名".into());}
        if crate::commands::ownership::ownership_domain_parts(&h).and_then(|p|p.registrable).is_none(){return Err("公共后缀或共享平台需要缩小至具体主机名".into());}
        confirmed.insert(h);
    }
    if confirmed.len()>20{return Err("确认范围最多20项，请先整理原有范围".into());}
    tx.execute("INSERT INTO asset_discovery_scopes VALUES(?1,?2) ON CONFLICT(project_id) DO UPDATE SET roots_json=excluded.roots_json",params![input.project_id,json!(confirmed).to_string()]).map_err(err)?;
    tx.commit().map_err(err)?;
    Ok(json!({"roots":confirmed,"networkRequested":false,"ownershipConfirmed":false,"hunterStarted":false}))
}
#[tauri::command]
pub async fn confirm_local_discovery(state:tauri::State<'_,AppState>,input:ConfirmInput)->Result<Value,String>{
    let path=state.db_path.clone();tauri::async_runtime::spawn_blocking(move||confirm_on(&mut db::open(&path)?,input)).await.map_err(err)?
}

#[cfg(test)]
#[path="discovery_analysis_tests.rs"]
mod tests;
