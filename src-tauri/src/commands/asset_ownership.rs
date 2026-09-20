fn clean_ownership_list(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty() && value.chars().count() <= 240)
        .filter(|value| seen.insert(value.clone()))
        .take(500)
        .collect()
}

fn normalize_ownership_domain(value: &str) -> String {
    let mut value = value.trim().to_lowercase();
    if let Some((_, rest)) = value.split_once("://") {
        value = rest.to_string();
    }
    value = value
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .trim_end_matches('.')
        .to_string();
    if value.matches(':').count() == 1 {
        value = value.split(':').next().unwrap_or("").to_string();
    }
    value.trim_start_matches("www.").to_string()
}

fn domain_matches(host: &str, rule: &str) -> bool {
    let host = normalize_ownership_domain(host);
    let rule = normalize_ownership_domain(rule);
    !host.is_empty() && !rule.is_empty() && (host == rule || host.ends_with(&format!(".{rule}")))
}

fn text_matches_term(text: &str, term: &str) -> bool {
    let term = term.trim().to_lowercase();
    term.chars().count() >= 2 && text.to_lowercase().contains(&term)
}

fn network_only_ownership_candidate(company:&str,domain:&str,ip:&str,candidate_domain:&str)->bool{
    company.contains('/')||company.trim().to_uppercase().starts_with("AS")||(domain.is_empty()&&!ip.is_empty()&&candidate_domain.parse::<std::net::IpAddr>().is_ok())
}

const OWNERSHIP_SCOPE_SQL:&str="pa.is_deleted=0 AND COALESCE(pa.decision,'pending') NOT IN ('rejected','not_applicable') AND COALESCE(a.probe_outcome,'') IN ('','alive_clean','web_alive','web_restricted','browser_render_required','virtual_host_required') AND COALESCE(a.content_category,'') NOT IN ('gambling','porn','custom_rule')";

fn ownership_profile_from_row(row: &Row<'_>) -> rusqlite::Result<AssetOwnershipProfile> {
    let strings = |raw: String| {
        serde_json::from_str::<Vec<String>>(&raw).unwrap_or_default()
    };
    Ok(AssetOwnershipProfile {
        project_id: row.get(0)?,
        legal_name: row.get(1)?,
        jurisdiction: row.get(2)?,
        jurisdictions: strings(row.get(3)?),
        excluded_jurisdictions: strings(row.get(4)?),
        aliases: strings(row.get(5)?),
        approved_domains: strings(row.get(6)?),
        shared_domains: strings(row.get(7)?),
        excluded_names: strings(row.get(8)?),
        excluded_domains: strings(row.get(9)?),
        notes: row.get(10)?,
        policy_version: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

#[tauri::command]
pub fn get_asset_ownership_profile(
    state: State<AppState>,
    project_id: i64,
) -> Result<AssetOwnershipProfile, String> {
    let connection = db::open(&state.db_path)?;
    if let Some(profile) = connection
        .query_row(
            "SELECT project_id,legal_name,jurisdiction,jurisdictions_json,excluded_jurisdictions_json,aliases_json,approved_domains_json,shared_domains_json,excluded_names_json,excluded_domains_json,notes,policy_version,updated_at FROM asset_ownership_profiles WHERE project_id=?1",
            [project_id],
            ownership_profile_from_row,
        )
        .optional()
        .map_err(|error| error.to_string())?
    {
        return Ok(profile);
    }
    let name = connection
        .query_row("SELECT name FROM projects WHERE id=?1", [project_id], |row| row.get(0))
        .map_err(|_| "工作空间不存在".to_string())?;
    Ok(AssetOwnershipProfile {
        project_id,
        legal_name: name,
        jurisdictions: Vec::new(),
        excluded_jurisdictions: Vec::new(),
        aliases: Vec::new(),
        approved_domains: Vec::new(),
        shared_domains: Vec::new(),
        excluded_names: Vec::new(),
        excluded_domains: Vec::new(),
        jurisdiction: String::new(),
        notes: String::new(),
        policy_version: 1,
        updated_at: String::new(),
    })
}

#[tauri::command]
pub fn save_asset_ownership_profile(
    state: State<AppState>,
    mut input: AssetOwnershipProfile,
) -> Result<AssetOwnershipProfile, String> {
    input.legal_name = input.legal_name.trim().chars().take(240).collect();
    input.jurisdiction = input.jurisdiction.trim().chars().take(120).collect();
    input.jurisdictions = clean_ownership_list(input.jurisdictions);
    input.excluded_jurisdictions = clean_ownership_list(input.excluded_jurisdictions);
    input.aliases = clean_ownership_list(input.aliases);
    input.approved_domains = clean_ownership_list(input.approved_domains)
        .into_iter()
        .map(|value| normalize_ownership_domain(&value))
        .filter(|value| !value.is_empty())
        .collect();
    input.shared_domains = clean_ownership_list(input.shared_domains).into_iter().map(|value| normalize_ownership_domain(&value)).filter(|value|!value.is_empty()).collect();
    input.excluded_names = clean_ownership_list(input.excluded_names);
    input.excluded_domains = clean_ownership_list(input.excluded_domains)
        .into_iter()
        .map(|value| normalize_ownership_domain(&value))
        .filter(|value| !value.is_empty())
        .collect();
    input.notes = input.notes.trim().chars().take(2000).collect();
    if input.legal_name.is_empty() {
        return Err("请填写目标主体名称".into());
    }
    let connection = db::open(&state.db_path)?;
    connection
        .execute(
            "INSERT INTO asset_ownership_profiles(project_id,legal_name,jurisdiction,jurisdictions_json,excluded_jurisdictions_json,aliases_json,approved_domains_json,shared_domains_json,excluded_names_json,excluded_domains_json,notes,policy_version) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,1) ON CONFLICT(project_id) DO UPDATE SET legal_name=excluded.legal_name,jurisdiction=excluded.jurisdiction,jurisdictions_json=excluded.jurisdictions_json,excluded_jurisdictions_json=excluded.excluded_jurisdictions_json,aliases_json=excluded.aliases_json,approved_domains_json=excluded.approved_domains_json,shared_domains_json=excluded.shared_domains_json,excluded_names_json=excluded.excluded_names_json,excluded_domains_json=excluded.excluded_domains_json,notes=excluded.notes,policy_version=asset_ownership_profiles.policy_version+1,updated_at=datetime('now','localtime')",
            params![input.project_id,input.legal_name,input.jurisdiction,serde_json::to_string(&input.jurisdictions).unwrap_or_else(|_| "[]".into()),serde_json::to_string(&input.excluded_jurisdictions).unwrap_or_else(|_| "[]".into()),serde_json::to_string(&input.aliases).unwrap_or_else(|_| "[]".into()),serde_json::to_string(&input.approved_domains).unwrap_or_else(|_| "[]".into()),serde_json::to_string(&input.shared_domains).unwrap_or_else(|_| "[]".into()),serde_json::to_string(&input.excluded_names).unwrap_or_else(|_| "[]".into()),serde_json::to_string(&input.excluded_domains).unwrap_or_else(|_| "[]".into()),input.notes],
        )
        .map_err(|error| error.to_string())?;
    get_asset_ownership_profile(state, input.project_id)
}

fn asset_ownership_summary(
    connection: &rusqlite::Connection,
    project_id: i64,
) -> Result<AssetOwnershipSummary, String> {
    connection
        .query_row(
            &format!("SELECT COUNT(*),COALESCE(SUM(CASE WHEN COALESCE(o.status,'unreviewed')='unreviewed' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN o.status='attributed' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN o.status='confirmed' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN o.status='related' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN o.status='third_party' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN o.status='excluded' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN o.exposure_eligible=1 THEN 1 ELSE 0 END),0) FROM project_assets pa JOIN assets a ON a.id=pa.asset_id LEFT JOIN asset_ownership_decisions o ON o.project_id=pa.project_id AND o.asset_id=pa.asset_id WHERE pa.project_id=?1 AND {OWNERSHIP_SCOPE_SQL}"),
            [project_id],
            |row| Ok(AssetOwnershipSummary { all: row.get(0)?, unreviewed: row.get(1)?, attributed: row.get(2)?, confirmed: row.get(3)?, related: row.get(4)?, third_party: row.get(5)?, excluded: row.get(6)?, exposure_eligible: row.get(7)? }),
        )
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_asset_ownership_summary(
    state: State<AppState>,
    project_id: i64,
) -> Result<AssetOwnershipSummary, String> {
    asset_ownership_summary(&db::open(&state.db_path)?, project_id)
}

#[tauri::command]
pub fn assess_asset_ownership(
    state: State<AppState>,
    project_id: i64,
) -> Result<AssetOwnershipAssessmentResult, String> {
    let mut connection = db::open(&state.db_path)?;
    let profile = get_asset_ownership_profile(state.clone(), project_id)?;
    let rules = {
        let mut statement = connection
            .prepare("SELECT rule_type,pattern,action FROM asset_ownership_rules WHERE project_id=?1 AND enabled=1")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([project_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        rows
    };
    let assets = {
        let mut statement = connection
            .prepare(&format!("SELECT a.id,a.company,a.host,a.link,a.domain,a.title,a.ip,a.extra_json FROM assets a JOIN project_assets pa ON pa.asset_id=a.id LEFT JOIN asset_ownership_decisions o ON o.project_id=pa.project_id AND o.asset_id=pa.asset_id WHERE pa.project_id=?1 AND {OWNERSHIP_SCOPE_SQL} AND COALESCE(o.source,'auto')<>'manual'"))
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([project_id], |row| Ok((row.get::<_,i64>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,String>(4)?,row.get::<_,String>(5)?,row.get::<_,String>(6)?,row.get::<_,String>(7)?)))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        rows
    };
    let transaction = connection.transaction().map_err(|error| error.to_string())?;
    let mut assessed = 0i64;
    for (asset_id, company, host, link, domain, title, ip, extra_json) in assets {
        let candidate_domain = [domain.as_str(), host.as_str(), link.as_str()]
            .into_iter()
            .map(normalize_ownership_domain)
            .find(|value| !value.is_empty())
            .unwrap_or_default();
        let extra=json(extra_json);let org=extra.get("org").and_then(JsonValue::as_str).unwrap_or("");let region=["country","country_name","region","province"].iter().filter_map(|key|extra.get(*key).and_then(JsonValue::as_str)).collect::<Vec<_>>().join(" ");
        let combined = format!("{company} {title} {org}");
        let mut approved_domains = profile.approved_domains.clone();
        let mut excluded_domains = profile.excluded_domains.clone();
        let mut excluded_names = profile.excluded_names.clone();
        for (rule_type, pattern, action) in &rules {
            match (rule_type.as_str(), action.as_str()) {
                ("domain", "approve") => approved_domains.push(pattern.clone()),
                ("domain", "exclude") => excluded_domains.push(pattern.clone()),
                ("name", "exclude") => excluded_names.push(pattern.clone()),
                _ => {}
            }
        }
        let excluded_domain = excluded_domains.iter().find(|rule| domain_matches(&candidate_domain, rule));
        let approved_domain = approved_domains.iter().find(|rule| domain_matches(&candidate_domain, rule));
        let shared_domain = profile.shared_domains.iter().find(|rule| domain_matches(&candidate_domain,rule));
        let excluded_name = excluded_names.iter().find(|rule| text_matches_term(&combined, rule));
        let alias_match = std::iter::once(&profile.legal_name)
            .chain(profile.aliases.iter())
            .find(|term| text_matches_term(&company, term) || text_matches_term(&title, term)||text_matches_term(org,term));
        let cidr_or_asn=network_only_ownership_candidate(&company,&domain,&ip,&candidate_domain);
        let excluded_region=profile.excluded_jurisdictions.iter().find(|item|text_matches_term(&region,item));let allowed_region=profile.jurisdictions.iter().find(|item|text_matches_term(&region,item));
        let (status, confidence, reason, evidence, negative) = if let Some(rule) = excluded_domain {
            ("excluded", 100, format!("命中排除域名 {rule}"), Vec::<String>::new(), vec![format!("排除域名：{rule}")])
        } else if let Some(rule) = excluded_name {
            ("excluded", 94, format!("命中排除主体 {rule}"), Vec::<String>::new(), vec![format!("排除主体：{rule}")])
        } else if let Some(rule) = shared_domain {
            ("attributed", 52, format!("命中集团共享域名 {rule}；不能据此认定属于当前跨境主体"), vec![format!("集团域名线索：{rule}")], vec!["共享域名可能同时承载其他地区或子公司".into()])
        } else if let Some(rule) = approved_domain {
            ("attributed", 92, format!("属于确认根域名 {rule}；仍需确认授权范围"), vec![format!("确认域名：{rule}")], Vec::<String>::new())
        } else if cidr_or_asn&&alias_match.is_some() {
            let region_note=if let Some(value)=excluded_region{format!("；地域 {value} 位于排除范围")}else if let Some(value)=allowed_region{format!("；地域线索命中 {value}")}else{"；未取得可靠地域证据".into()};
            ("attributed",if allowed_region.is_some(){55}else{45},format!("ASN/IP 组织字段命中 {org}，仅证明网络登记或运营关系，不等于业务资产{region_note}"),vec![format!("网络组织：{org}"),format!("IP：{ip}")],vec!["ASN、CIDR、WHOIS 组织均属于弱归属证据".into()])
        } else if let Some(term) = alias_match {
            ("attributed", 58, format!("名称命中 {term}，缺少技术控制证据"), vec![format!("名称线索：{term}")], Vec::<String>::new())
        } else {
            ("unreviewed", 0, "没有足够的实体归属证据".into(), Vec::<String>::new(), Vec::<String>::new())
        };
        transaction.execute(
            "INSERT INTO asset_ownership_decisions(project_id,asset_id,status,confidence,authorization_status,exposure_eligible,source,reason,evidence_json,negative_evidence_json) VALUES(?1,?2,?3,?4,'unknown',0,'auto',?5,?6,?7) ON CONFLICT(project_id,asset_id) DO UPDATE SET status=excluded.status,confidence=excluded.confidence,authorization_status='unknown',exposure_eligible=0,source='auto',reason=excluded.reason,evidence_json=excluded.evidence_json,negative_evidence_json=excluded.negative_evidence_json,updated_at=datetime('now','localtime') WHERE asset_ownership_decisions.source<>'manual'",
            params![project_id,asset_id,status,confidence,reason,serde_json::to_string(&evidence).unwrap(),serde_json::to_string(&negative).unwrap()],
        ).map_err(|error| error.to_string())?;
        assessed += 1;
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(AssetOwnershipAssessmentResult { assessed, summary: asset_ownership_summary(&connection, project_id)? })
}

#[tauri::command]
pub fn update_asset_ownership(
    state: State<AppState>,
    input: AssetOwnershipDecisionInput,
) -> Result<i64, String> {
    if !["unreviewed", "confirmed", "related", "third_party", "excluded"].contains(&input.status.as_str()) {
        return Err("归属结论无效".into());
    }
    if input.selections.is_empty() || input.selections.len() > 20_000 {
        return Err("请选择 1–20000 条资产".into());
    }
    let mut connection = db::open(&state.db_path)?;
    let transaction = connection.transaction().map_err(|error| error.to_string())?;
    let mut unique = HashSet::new();
    let mut changed = 0i64;
    for item in input.selections.into_iter().filter(|item| unique.insert((item.project_id,item.asset_id))) {
        if input.status == "unreviewed" {
            changed += transaction.execute(
                "DELETE FROM asset_ownership_decisions WHERE project_id=?1 AND asset_id=?2",
                params![item.project_id, item.asset_id],
            ).map_err(|error| error.to_string())? as i64;
            transaction.execute(
                "INSERT INTO asset_events(project_id,asset_id,event_type,summary) VALUES(?1,?2,'ownership','撤销人工归属结论，恢复自动判断')",
                params![item.project_id, item.asset_id],
            ).map_err(|error| error.to_string())?;
            continue;
        }
        let (authorization, eligible, confidence) = match input.status.as_str() {
            "confirmed" => ("allowed", 1, 100),
            "excluded" | "third_party" => ("denied", 0, 100),
            _ => ("unknown", 0, 70),
        };
        changed += transaction.execute(
            "INSERT INTO asset_ownership_decisions(project_id,asset_id,status,confidence,authorization_status,exposure_eligible,source,reason,evidence_json,negative_evidence_json) VALUES(?1,?2,?3,?4,?5,?6,'manual',?7,'[]','[]') ON CONFLICT(project_id,asset_id) DO UPDATE SET status=excluded.status,confidence=excluded.confidence,authorization_status=excluded.authorization_status,exposure_eligible=excluded.exposure_eligible,source='manual',reason=excluded.reason,updated_at=datetime('now','localtime')",
            params![item.project_id,item.asset_id,input.status,confidence,authorization,eligible,input.note],
        ).map_err(|error| error.to_string())? as i64;
        transaction.execute("INSERT INTO asset_events(project_id,asset_id,event_type,summary) VALUES(?1,?2,'ownership',?3)", params![item.project_id,item.asset_id,format!("{}: {}",input.status,input.note)]).map_err(|error| error.to_string())?;
        if input.learn_domain_rule && matches!(input.status.as_str(), "confirmed" | "excluded") {
            let raw: String = transaction.query_row("SELECT COALESCE(NULLIF(domain,''),NULLIF(host,''),link) FROM assets WHERE id=?1", [item.asset_id], |row| row.get(0)).unwrap_or_default();
            let pattern = normalize_ownership_domain(&raw);
            if !pattern.is_empty() && pattern.parse::<std::net::IpAddr>().is_err() {
                let action = if input.status == "confirmed" { "approve" } else { "exclude" };
                transaction.execute("INSERT INTO asset_ownership_rules(project_id,rule_type,pattern,action,source) VALUES(?1,'domain',?2,?3,'manual_decision') ON CONFLICT(project_id,rule_type,pattern,action) DO UPDATE SET enabled=1,updated_at=datetime('now','localtime')", params![item.project_id,pattern,action]).map_err(|error| error.to_string())?;
            }
        }
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(changed)
}

#[cfg(test)]
mod asset_ownership_tests {
    use super::{domain_matches, network_only_ownership_candidate, normalize_ownership_domain, text_matches_term};

    #[test]
    fn ownership_domain_matching_respects_dns_label_boundaries() {
        assert!(domain_matches("api.cmhk.com", "cmhk.com"));
        assert!(domain_matches("https://www.cmhk.com:443/a", "cmhk.com"));
        assert!(!domain_matches("evilcmhk.com", "cmhk.com"));
        assert!(!domain_matches("cmhk.com.example.org", "cmhk.com"));
    }

    #[test]
    fn ownership_normalization_strips_transport_details_only() {
        assert_eq!(
            normalize_ownership_domain("HTTPS://WWW.CMHK.COM:443/path?q=1"),
            "cmhk.com"
        );
        assert_eq!(normalize_ownership_domain("mobile.com.hk."), "mobile.com.hk");
    }

    #[test]
    fn ownership_name_terms_reject_blank_and_single_character_noise() {
        assert!(text_matches_term("香港移动客户服务", "香港移动"));
        assert!(!text_matches_term("中国移动", "移"));
        assert!(!text_matches_term("中国移动", ""));
    }

    #[test]
    fn asn_cidr_and_ip_only_assets_are_weak_network_evidence() {
        assert!(network_only_ownership_candidate("182.239.124.0/23","","182.239.125.17","182.239.125.17"));
        assert!(network_only_ownership_candidate("AS58453","","1.1.1.1","1.1.1.1"));
        assert!(!network_only_ownership_candidate("CMHK","cmhk.com","1.1.1.1","cmhk.com"));
    }
}
