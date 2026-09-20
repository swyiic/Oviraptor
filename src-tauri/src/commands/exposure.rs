fn exposure_text(value: String, limit: usize) -> String {
    value.trim().chars().take(limit).collect()
}

#[tauri::command]
pub fn exposure_summary(state: State<AppState>, project_id: i64) -> Result<ExposureSummary, String> {
    let connection = db::open(&state.db_path)?;
    connection.query_row(
        "SELECT (SELECT COUNT(*) FROM asset_ownership_decisions WHERE project_id=?1 AND exposure_eligible=1),COUNT(*),COALESCE(SUM(status='new'),0),COALESCE(SUM(status='reviewing'),0),COALESCE(SUM(status='confirmed'),0),COALESCE(SUM(status='dismissed'),0),COALESCE(SUM(status<>'dismissed' AND severity IN ('critical','high')),0) FROM exposure_findings WHERE project_id=?1",
        [project_id],
        |row| Ok(ExposureSummary { eligible_assets: row.get(0)?, total: row.get(1)?, new_count: row.get(2)?, reviewing: row.get(3)?, confirmed: row.get(4)?, dismissed: row.get(5)?, high_risk: row.get(6)? }),
    ).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn list_exposure_findings(state: State<AppState>, project_id: i64, status: String, search: String) -> Result<Vec<ExposureFinding>, String> {
    let connection = db::open(&state.db_path)?;
    let status = if ["new","reviewing","confirmed","dismissed"].contains(&status.as_str()) { status } else { String::new() };
    let search = format!("%{}%", search.trim());
    let mut statement = connection.prepare(
        "SELECT f.id,f.project_id,f.asset_id,f.category,f.title,f.source_type,f.source_url,f.evidence_excerpt,f.severity,f.confidence,f.status,f.note,COALESCE(NULLIF(a.domain,''),NULLIF(a.host,''),NULLIF(a.link,''),a.ip,''),COALESCE(o.exposure_eligible,0),f.first_seen_at,f.last_seen_at FROM exposure_findings f LEFT JOIN assets a ON a.id=f.asset_id LEFT JOIN asset_ownership_decisions o ON o.project_id=f.project_id AND o.asset_id=f.asset_id WHERE f.project_id=?1 AND (?2='' OR f.status=?2) AND (?3='%%' OR f.title LIKE ?3 OR f.source_url LIKE ?3 OR f.evidence_excerpt LIKE ?3) ORDER BY CASE f.severity WHEN 'critical' THEN 4 WHEN 'high' THEN 3 WHEN 'medium' THEN 2 ELSE 1 END DESC,f.confidence DESC,f.last_seen_at DESC LIMIT 500"
    ).map_err(|error| error.to_string())?;
    let findings = statement.query_map(params![project_id,status,search], |row| Ok(ExposureFinding { id:row.get(0)?,project_id:row.get(1)?,asset_id:row.get(2)?,category:row.get(3)?,title:row.get(4)?,source_type:row.get(5)?,source_url:row.get(6)?,evidence_excerpt:row.get(7)?,severity:row.get(8)?,confidence:row.get(9)?,status:row.get(10)?,note:row.get(11)?,asset_label:row.get(12)?,exposure_eligible:row.get::<_,i64>(13)?!=0,first_seen_at:row.get(14)?,last_seen_at:row.get(15)? }))
        .map_err(|error| error.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|error| error.to_string())?;
    Ok(findings)
}

#[tauri::command]
pub fn save_exposure_finding(state: State<AppState>, mut input: ExposureFindingInput) -> Result<i64, String> {
    let categories = ["source_code","document","credential","storage","repository","metadata","other"];
    let severities = ["low","medium","high","critical"];
    if !categories.contains(&input.category.as_str()) || !severities.contains(&input.severity.as_str()) { return Err("暴露面类别或风险等级无效".into()); }
    input.title=exposure_text(input.title,240); input.source_type=exposure_text(input.source_type,40); input.source_url=exposure_text(input.source_url,2000); input.evidence_excerpt=exposure_text(input.evidence_excerpt,4000); input.confidence=input.confidence.clamp(0,100);
    if input.title.is_empty() || input.source_url.is_empty() { return Err("请填写标题和公开来源地址".into()); }
    let connection=db::open(&state.db_path)?;
    if let Some(asset_id)=input.asset_id {
        let eligible: i64=connection.query_row("SELECT COUNT(*) FROM asset_ownership_decisions WHERE project_id=?1 AND asset_id=?2 AND exposure_eligible=1",params![input.project_id,asset_id],|row|row.get(0)).map_err(|error|error.to_string())?;
        if eligible==0 { return Err("该资产尚未通过归属与授权门禁，不能进入暴露面".into()); }
    }
    let fingerprint=format!("{:x}",Sha256::digest(format!("{}|{}|{}",input.category,input.source_url.to_lowercase(),input.title.to_lowercase()).as_bytes()));
    connection.execute("INSERT INTO exposure_findings(project_id,asset_id,category,title,source_type,source_url,evidence_excerpt,severity,confidence,fingerprint) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(project_id,fingerprint) DO UPDATE SET asset_id=COALESCE(excluded.asset_id,exposure_findings.asset_id),evidence_excerpt=excluded.evidence_excerpt,severity=excluded.severity,confidence=MAX(exposure_findings.confidence,excluded.confidence),last_seen_at=datetime('now','localtime'),updated_at=datetime('now','localtime')",params![input.project_id,input.asset_id,input.category,input.title,input.source_type,input.source_url,input.evidence_excerpt,input.severity,input.confidence,fingerprint]).map_err(|error|error.to_string())?;
    connection.query_row("SELECT id FROM exposure_findings WHERE project_id=?1 AND fingerprint=?2",params![input.project_id,fingerprint],|row|row.get(0)).map_err(|error|error.to_string())
}

#[tauri::command]
pub fn review_exposure_finding(state: State<AppState>, mut input: ExposureFindingReviewInput) -> Result<(), String> {
    if !["new","reviewing","confirmed","dismissed"].contains(&input.status.as_str()) { return Err("复核状态无效".into()); }
    input.note=exposure_text(input.note,2000);
    db::open(&state.db_path)?.execute("UPDATE exposure_findings SET status=?1,note=?2,updated_at=datetime('now','localtime') WHERE id=?3",params![input.status,input.note,input.id]).map_err(|error|error.to_string())?;
    Ok(())
}

fn exposure_run_from_row(row: &Row<'_>) -> rusqlite::Result<ExposureRun> {
    Ok(ExposureRun { id:row.get(0)?,project_id:row.get(1)?,status:row.get(2)?,eligible_assets:row.get(3)?,scanned_assets:row.get(4)?,fetched_resources:row.get(5)?,findings:row.get(6)?,stage:row.get(7)?,current_source:row.get(8)?,error:row.get(9)?,created_at:row.get(10)?,started_at:row.get(11)?,completed_at:row.get(12)? })
}

#[tauri::command]
pub fn list_exposure_runs(state: State<AppState>, project_id: i64) -> Result<Vec<ExposureRun>, String> {
    let connection=db::open(&state.db_path)?;
    let mut statement=connection.prepare("SELECT id,project_id,status,eligible_assets,scanned_assets,fetched_resources,findings,stage,current_source,error,created_at,started_at,completed_at FROM exposure_runs WHERE project_id=?1 ORDER BY id DESC LIMIT 20").map_err(|error|error.to_string())?;
    let rows=statement.query_map([project_id],exposure_run_from_row).map_err(|error|error.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|error|error.to_string())?;
    Ok(rows)
}

#[tauri::command]
pub fn list_exposure_source_results(state:State<AppState>,run_id:i64)->Result<Vec<ExposureSourceResult>,String>{
    let connection=db::open(&state.db_path)?;let mut statement=connection.prepare("SELECT run_id,source_key,status,item_count,error,completed_at FROM exposure_source_results WHERE run_id=?1 ORDER BY id").map_err(|error|error.to_string())?;
    let rows=statement.query_map([run_id],|row|Ok(ExposureSourceResult{run_id:row.get(0)?,source_key:row.get(1)?,status:row.get(2)?,item_count:row.get(3)?,error:row.get(4)?,completed_at:row.get(5)?})).map_err(|error|error.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|error|error.to_string())?;Ok(rows)
}

#[tauri::command]
pub fn cancel_exposure_scan(state:State<AppState>,run_id:i64)->Result<(),String>{
    db::open(&state.db_path)?.execute("UPDATE exposure_runs SET cancel_requested=1,stage='cancelling' WHERE id=?1 AND status IN ('queued','running')",[run_id]).map_err(|error|error.to_string())?;Ok(())
}

#[tauri::command]
pub fn export_exposure_findings(state:State<AppState>,project_id:i64)->Result<ExportResult,String>{
    let connection=db::open(&state.db_path)?;let mut statement=connection.prepare("SELECT category,title,severity,confidence,status,source_type,source_url,evidence_excerpt,note,first_seen_at,last_seen_at FROM exposure_findings WHERE project_id=?1 ORDER BY id").map_err(|error|error.to_string())?;
    let rows=statement.query_map([project_id],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,i64>(3)?,row.get::<_,String>(4)?,row.get::<_,String>(5)?,row.get::<_,String>(6)?,row.get::<_,String>(7)?,row.get::<_,String>(8)?,row.get::<_,String>(9)?,row.get::<_,String>(10)?))).map_err(|error|error.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|error|error.to_string())?;
    fs::create_dir_all(&state.export_dir).map_err(|error|error.to_string())?;let path=state.export_dir.join(format!("exposure-{}.csv",Uuid::new_v4()));let file=File::create(&path).map_err(|error|error.to_string())?;let mut writer=csv::WriterBuilder::new().from_writer(file);
    writer.write_record(["类别","标题","风险","置信度","状态","来源类型","来源 URL","脱敏证据","备注","首次发现","最近发现"]).map_err(|error|error.to_string())?;for row in &rows{writer.write_record([&row.0,&row.1,&row.2,&row.3.to_string(),&row.4,&row.5,&row.6,&row.7,&row.8,&row.9,&row.10]).map_err(|error|error.to_string())?;}writer.flush().map_err(|error|error.to_string())?;Ok(ExportResult{path:path.to_string_lossy().into_owned(),rows:rows.len() as i64})
}

fn quoted_resource_urls(text: &str) -> Vec<String> {
    let mut values=Vec::new();
    for marker in ["href=\"","src=\"","href='","src='","sourceMappingURL="] {
        let mut rest=text;
        while let Some(index)=rest.find(marker) {
            rest=&rest[index+marker.len()..];
            let end=if marker.ends_with('=') { rest.find(|c:char|c.is_whitespace()||c=='\"'||c=='\'').unwrap_or(rest.len()) } else { rest.find(if marker.ends_with('\"') {'\"'} else {'\''}).unwrap_or(rest.len()) };
            let value=rest[..end].trim();
            if !value.is_empty()&&value.len()<=2000 { values.push(value.to_string()); }
            rest=&rest[end.min(rest.len())..];
            if values.len()>=200 { break; }
        }
    }
    values
}

fn masked_sensitive_evidence(text: &str) -> Option<String> {
    let keys=["api_key","apikey","access_key","secret_key","client_secret","private_token","authorization"];
    for line in text.lines().take(4000) {
        let lower=line.to_lowercase();
        if !keys.iter().any(|key|lower.contains(key))||(!line.contains('=')&&!line.contains(':')) {continue}
        if ["example","placeholder","your_","xxxx","test_key","process.env"].iter().any(|term|lower.contains(term)){continue}
        let separator=line.find('=').or_else(||line.find(':'))?;let key=line[..separator].trim().chars().take(60).collect::<String>();
        let value=line[separator+1..].trim().trim_matches(['\"','\'',';',',',' ']);
        if value.len()<8||value.contains(' ')||value.len()>500 {continue}
        let prefix=value.chars().take(3).collect::<String>();let suffix=value.chars().rev().take(2).collect::<String>().chars().rev().collect::<String>();
        return Some(format!("公开资源出现字段 {key}，值已脱敏：{prefix}***{suffix}"));
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn upsert_passive_exposure(connection:&rusqlite::Connection,project_id:i64,asset_id:i64,category:&str,title:&str,url:&str,evidence:&str,severity:&str,confidence:i64)->Result<bool,String>{
    let fingerprint=format!("{:x}",Sha256::digest(format!("{category}|{}|{}",url.to_lowercase(),title.to_lowercase()).as_bytes()));
    let existed:i64=connection.query_row("SELECT COUNT(*) FROM exposure_findings WHERE project_id=?1 AND fingerprint=?2",params![project_id,fingerprint],|row|row.get(0)).map_err(|error|error.to_string())?;
    connection.execute("INSERT INTO exposure_findings(project_id,asset_id,category,title,source_type,source_url,evidence_excerpt,severity,confidence,fingerprint) VALUES(?1,?2,?3,?4,'passive_web',?5,?6,?7,?8,?9) ON CONFLICT(project_id,fingerprint) DO UPDATE SET last_seen_at=datetime('now','localtime'),evidence_excerpt=excluded.evidence_excerpt,confidence=MAX(exposure_findings.confidence,excluded.confidence),updated_at=datetime('now','localtime')",params![project_id,asset_id,category,title,url,evidence,severity,confidence,fingerprint]).map_err(|error|error.to_string())?;
    Ok(existed==0)
}

#[allow(clippy::too_many_arguments)]
fn upsert_entity_exposure(connection:&rusqlite::Connection,project_id:i64,category:&str,title:&str,url:&str,evidence:&str,severity:&str,confidence:i64,source:&str)->Result<bool,String>{
    let fingerprint=format!("{:x}",Sha256::digest(format!("{category}|{}|{}",url.to_lowercase(),title.to_lowercase()).as_bytes()));let existed:i64=connection.query_row("SELECT COUNT(*) FROM exposure_findings WHERE project_id=?1 AND fingerprint=?2",params![project_id,fingerprint],|row|row.get(0)).map_err(|e|e.to_string())?;
    connection.execute("INSERT INTO exposure_findings(project_id,category,title,source_type,source_url,evidence_excerpt,severity,confidence,fingerprint) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(project_id,fingerprint) DO UPDATE SET last_seen_at=datetime('now','localtime'),evidence_excerpt=excluded.evidence_excerpt,confidence=MAX(exposure_findings.confidence,excluded.confidence),updated_at=datetime('now','localtime')",params![project_id,category,title,source,url,evidence,severity,confidence,fingerprint]).map_err(|e|e.to_string())?;Ok(existed==0)
}

fn collect_public_indices(connection:&rusqlite::Connection,client:&reqwest::blocking::Client,run_id:i64,project_id:i64)->Result<i64,String>{
    let domains:Vec<String>=connection.query_row("SELECT approved_domains_json FROM asset_ownership_profiles WHERE project_id=?1",[project_id],|row|row.get::<_,String>(0)).optional().map_err(|e|e.to_string())?.and_then(|raw|serde_json::from_str(&raw).ok()).unwrap_or_default();let mut new_findings=0;
    connection.execute("UPDATE exposure_runs SET stage='public_indexes',current_source='certificate_transparency' WHERE id=?1",[run_id]).map_err(|e|e.to_string())?;let mut ct_count=0;let mut ct_errors=Vec::new();
    for domain in domains.iter().take(20){if exposure_cancelled(connection,run_id){break}let url=format!("https://crt.sh/?q=%25.{domain}&output=json");match client.get(&url).send().and_then(|r|r.error_for_status()).and_then(|r|r.text()){Ok(body)=>{if let Ok(items)=serde_json::from_str::<Vec<JsonValue>>(&body){let mut names=HashSet::new();for item in items{if let Some(raw)=item.get("name_value").and_then(JsonValue::as_str){for name in raw.lines(){let name=name.trim_start_matches("*.").to_lowercase();if name==*domain||name.ends_with(&format!(".{domain}")){names.insert(name);}}}}ct_count+=names.len() as i64;}},Err(e)=>ct_errors.push(format!("{domain}: {e}"))}}
    save_exposure_source_result(connection,run_id,"certificate_transparency",if ct_errors.is_empty(){"completed"}else{"partial"},ct_count,&ct_errors.join("；"))?;
    connection.execute("UPDATE exposure_runs SET current_source='web_archive' WHERE id=?1",[run_id]).map_err(|e|e.to_string())?;let mut archive_count=0;let mut archive_errors=Vec::new();
    for domain in domains.iter().take(20){if exposure_cancelled(connection,run_id){break}let query=format!("https://web.archive.org/cdx/search/cdx?url=*.{domain}/*&output=json&fl=original,statuscode,mimetype&filter=statuscode:200&collapse=urlkey&limit=200");match client.get(&query).send().and_then(|r|r.error_for_status()).and_then(|r|r.text()){Ok(body)=>{if let Ok(rows)=serde_json::from_str::<Vec<Vec<JsonValue>>>(&body){for row in rows.into_iter().skip(1){let Some(url)=row.first().and_then(JsonValue::as_str)else{continue};archive_count+=1;let lower=url.to_lowercase();let doc=[".pdf",".docx",".xlsx",".pptx"].iter().any(|v|lower.contains(v));let sensitive=["internal","confidential","private","backup","内部","运维","架构"].iter().any(|v|lower.contains(v));if doc&&sensitive&&upsert_entity_exposure(connection,project_id,"document","历史归档中发现疑似内部文档",url,"Wayback 历史索引包含敏感语义文档路径；需确认当前可访问性和公开合理性。","medium",66,"web_archive")?{new_findings+=1;}
    if [".map","/.git/",".env","config.json","backup.zip"].iter().any(|v|lower.contains(v))&&upsert_entity_exposure(connection,project_id,"source_code","历史归档中发现高价值公开路径",url,"历史索引命中源码、配置或备份语义；仅作为复核线索，不代表当前仍可访问。","medium",62,"web_archive")?{new_findings+=1;}}}},Err(e)=>archive_errors.push(format!("{domain}: {e}"))}}
    save_exposure_source_result(connection,run_id,"web_archive",if archive_errors.is_empty(){"completed"}else{"partial"},archive_count,&archive_errors.join("；"))?;Ok(new_findings)
}

fn exposure_cancelled(connection:&rusqlite::Connection,run_id:i64)->bool{
    connection.query_row("SELECT cancel_requested FROM exposure_runs WHERE id=?1",[run_id],|row|row.get::<_,i64>(0)).unwrap_or(1)!=0
}

fn save_exposure_source_result(connection:&rusqlite::Connection,run_id:i64,source:&str,status:&str,count:i64,error:&str)->Result<(),String>{
    connection.execute("INSERT INTO exposure_source_results(run_id,source_key,status,item_count,error) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(run_id,source_key) DO UPDATE SET status=excluded.status,item_count=excluded.item_count,error=excluded.error,completed_at=datetime('now','localtime')",params![run_id,source,status,count,error]).map_err(|e|e.to_string())?;Ok(())
}

fn run_passive_exposure_scan(db_path:PathBuf,run_id:i64,project_id:i64){
    let result=(||->Result<(),String>{
        let connection=db::open(&db_path)?;
        let assets={
            let mut statement=connection.prepare("SELECT a.id,COALESCE(NULLIF(a.link,''),CASE WHEN a.host<>'' THEN 'https://'||a.host ELSE '' END) FROM assets a JOIN asset_ownership_decisions o ON o.asset_id=a.id AND o.project_id=?1 JOIN project_assets pa ON pa.asset_id=a.id AND pa.project_id=?1 WHERE o.exposure_eligible=1 AND pa.is_deleted=0 ORDER BY o.confidence DESC LIMIT 200").map_err(|error|error.to_string())?;
            let rows=statement.query_map([project_id],|row|Ok((row.get::<_,i64>(0)?,row.get::<_,String>(1)?))).map_err(|error|error.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|error|error.to_string())?;rows
        };
        connection.execute("UPDATE exposure_runs SET status='running',stage='collecting',current_source='passive_web',eligible_assets=?1,started_at=datetime('now','localtime') WHERE id=?2",params![assets.len() as i64,run_id]).map_err(|error|error.to_string())?;
        save_exposure_source_result(&connection,run_id,"passive_web","running",0,"")?;
        let client=reqwest::blocking::Client::builder().timeout(Duration::from_secs(12)).redirect(reqwest::redirect::Policy::limited(3)).user_agent("Oviraptor-Exposure/1.1 (+passive-public-evidence)").build().map_err(|error|error.to_string())?;
        let mut fetched=0i64;let mut found=0i64;let mut failures=0i64;
        for (asset_id,raw_url) in &assets {
            if exposure_cancelled(&connection,run_id){connection.execute("UPDATE exposure_runs SET status='cancelled',stage='cancelled',current_source='',error='用户已取消；已采集结果保留',completed_at=datetime('now','localtime') WHERE id=?1",[run_id]).map_err(|error|error.to_string())?;save_exposure_source_result(&connection,run_id,"passive_web","cancelled",found,"用户取消")?;return Ok(())}
            let Ok(base)=reqwest::Url::parse(raw_url) else {failures+=1;continue};
            if !matches!(base.scheme(),"http"|"https"){continue}
            let base_host=base.host_str().unwrap_or("").to_string();
            let mut queue=VecDeque::from([base.clone(),base.join("/robots.txt").unwrap_or(base.clone()),base.join("/sitemap.xml").unwrap_or(base.clone()),base.join("/.well-known/security.txt").unwrap_or(base.clone())]);
            let mut visited=HashSet::new();let mut resource_count=0;
            while let Some(url)=queue.pop_front(){
                if exposure_cancelled(&connection,run_id){break}
                if resource_count>=20||!visited.insert(url.as_str().to_string()){continue}
                let host=url.host_str().unwrap_or("");if !(host==base_host||host.ends_with(&format!(".{base_host}"))){continue}
                let response=match client.get(url.clone()).send(){Ok(value)=>value,Err(_)=>{failures+=1;continue}};
                if !response.status().is_success(){continue}
                let content_type=response.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v|v.to_str().ok()).unwrap_or("").to_lowercase();
                if !(content_type.contains("text")||content_type.contains("json")||content_type.contains("javascript")||url.path().ends_with(".map")){continue}
                let mut bytes=Vec::new();if response.take(1_000_001).read_to_end(&mut bytes).is_err()||bytes.len()>1_000_000{continue}
                let body=String::from_utf8_lossy(&bytes);fetched+=1;resource_count+=1;let path=url.path().to_lowercase();
                if path.ends_with(".map")&&upsert_passive_exposure(&connection,project_id,*asset_id,"source_code","公开可访问的 Source Map",url.as_str(),"页面或脚本自然引用的 Source Map 可公开读取；需复核是否包含业务源码和内部路径。","high",92)?{found+=1}
                if let Some(evidence)=masked_sensitive_evidence(&body){if upsert_passive_exposure(&connection,project_id,*asset_id,"credential","公开资源中出现疑似凭据赋值",url.as_str(),&evidence,"high",76)?{found+=1}}
                for raw in quoted_resource_urls(&body){
                    let Ok(link)=url.join(raw.trim()) else {continue};let lower=link.as_str().to_lowercase();
                    if ["github.com/","gitlab.com/","gitee.com/"].iter().any(|v|lower.contains(v))&&upsert_passive_exposure(&connection,project_id,*asset_id,"repository","页面公开引用代码仓库",link.as_str(),"从已准入资产的公开页面自然发现，需复核仓库归属与可见内容。","medium",72)?{found+=1}
                    if ["s3.amazonaws.com","blob.core.windows.net","aliyuncs.com","myqcloud.com"].iter().any(|v|lower.contains(v))&&upsert_passive_exposure(&connection,project_id,*asset_id,"storage","页面公开引用对象存储",link.as_str(),"公开页面自然引用的对象存储地址；仅记录来源，不进行目录枚举。","medium",68)?{found+=1}
                    let doc=[".pdf",".docx",".xlsx",".pptx"].iter().any(|v|lower.ends_with(v));let sensitive=["internal","confidential","private","内部","运维","架构"].iter().any(|v|lower.contains(v));
                    if doc&&sensitive&&upsert_passive_exposure(&connection,project_id,*asset_id,"document","公开链接包含疑似内部文档",link.as_str(),"文件名或路径含内部语义，需人工确认内容与公开合理性。","medium",74)?{found+=1}
                    let linked_host=link.host_str().unwrap_or("");let fetchable=(linked_host==base_host||linked_host.ends_with(&format!(".{base_host}")))&&[".js",".map",".json"].iter().any(|v|lower.ends_with(v));
                    if fetchable&&visited.len()+queue.len()<40{queue.push_back(link)}
                }
            }
            connection.execute("UPDATE exposure_runs SET scanned_assets=scanned_assets+1,fetched_resources=?1,findings=?2 WHERE id=?3",params![fetched,found,run_id]).map_err(|error|error.to_string())?;
        }
        save_exposure_source_result(&connection,run_id,"passive_web",if failures>0{"partial"}else{"completed"},found,&if failures>0{format!("{failures} 次请求失败或超时")}else{String::new()})?;
        if !exposure_cancelled(&connection,run_id){found+=collect_public_indices(&connection,&client,run_id,project_id)?;}
        if exposure_cancelled(&connection,run_id){connection.execute("UPDATE exposure_runs SET status='cancelled',stage='cancelled',current_source='',findings=?1,error='用户已取消；已采集结果保留',completed_at=datetime('now','localtime') WHERE id=?2",params![found,run_id]).map_err(|e|e.to_string())?;return Ok(())}
        let status=if !assets.is_empty()&&failures>=assets.len() as i64{"partial"}else{"completed"};let error=if failures>0{format!("{failures} 次公开资源请求失败或超时；其他结果已保留")}else{String::new()};
        connection.execute("UPDATE exposure_runs SET status=?1,stage='completed',current_source='',fetched_resources=?2,findings=?3,error=?4,completed_at=datetime('now','localtime') WHERE id=?5",params![status,fetched,found,error,run_id]).map_err(|error|error.to_string())?;Ok(())
    })();
    if let Err(error)=result{if let Ok(connection)=db::open(&db_path){let _=connection.execute("UPDATE exposure_runs SET status='failed',error=?1,completed_at=datetime('now','localtime') WHERE id=?2",params![error,run_id]);}}
}

#[tauri::command]
pub fn start_exposure_scan(state:State<AppState>,project_id:i64)->Result<i64,String>{
    let connection=db::open(&state.db_path)?;let running:i64=connection.query_row("SELECT COUNT(*) FROM exposure_runs WHERE project_id=?1 AND status IN ('queued','running')",[project_id],|row|row.get(0)).map_err(|error|error.to_string())?;
    if running>0{return Err("当前工作空间已有暴露面采集任务运行中".into())}
    connection.execute("INSERT INTO exposure_runs(project_id) VALUES(?1)",[project_id]).map_err(|error|error.to_string())?;let run_id=connection.last_insert_rowid();let db_path=state.db_path.clone();thread::spawn(move||run_passive_exposure_scan(db_path,run_id,project_id));Ok(run_id)
}

#[cfg(test)]
mod exposure_tests {
    use super::{masked_sensitive_evidence, quoted_resource_urls};

    #[test]
    fn passive_parser_keeps_natural_resources_and_source_maps() {
        let values=quoted_resource_urls("<script src=\"/app.js\"></script>\n//# sourceMappingURL=app.js.map");
        assert!(values.contains(&"/app.js".to_string()));
        assert!(values.contains(&"app.js.map".to_string()));
    }

    #[test]
    fn credential_evidence_is_masked_and_placeholders_are_ignored() {
        let evidence=masked_sensitive_evidence("client_secret = abcdefgh123456").unwrap();
        assert!(evidence.contains("abc***56"));
        assert!(!evidence.contains("abcdefgh123456"));
        assert!(masked_sensitive_evidence("api_key = YOUR_API_KEY").is_none());
    }
}
