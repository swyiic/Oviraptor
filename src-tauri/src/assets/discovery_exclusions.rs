//! Reversible project exclusions. Assets, observations and billing receipts are never deleted.
use super::*;
use std::collections::{BTreeMap, HashMap};
const SCHEMA:&str="CREATE TABLE IF NOT EXISTS asset_discovery_domain_exclusions(project_id INTEGER NOT NULL,domain TEXT NOT NULL,enabled INTEGER NOT NULL DEFAULT 1,updated_at TEXT NOT NULL DEFAULT(datetime('now')),PRIMARY KEY(project_id,domain));CREATE TABLE IF NOT EXISTS asset_discovery_excluded_evidence(project_id INTEGER NOT NULL,host TEXT NOT NULL,source TEXT NOT NULL,source_ref TEXT NOT NULL,domain TEXT NOT NULL,PRIMARY KEY(project_id,host,source,source_ref,domain));CREATE TABLE IF NOT EXISTS asset_discovery_exclusion_events(id INTEGER PRIMARY KEY,project_id INTEGER NOT NULL,domain TEXT NOT NULL,action TEXT NOT NULL,receipt_json TEXT NOT NULL,created_at TEXT NOT NULL DEFAULT(datetime('now')));";
pub(super) fn exists(c: &Connection) -> Result<bool, String> {
    c.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='asset_discovery_domain_exclusions' AND type='table')",[],|r|r.get(0)).map_err(err)
}
pub(super) fn rules(c: &Connection, p: i64) -> Result<Vec<String>, String> {
    if !exists(c)? {
        return Ok(vec![]);
    }
    c.prepare("SELECT domain FROM asset_discovery_domain_exclusions WHERE project_id=?1 AND enabled=1 ORDER BY domain").map_err(err)?.query_map([p],|r|r.get(0)).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)
}
pub(super) fn matches(h: &str, domains: &[String]) -> bool {
    domains.iter().any(|d| in_scope(h, d))
}
pub(super) fn blocked_name(c: &Connection, p: i64, h: &str) -> Result<bool, String> {
    Ok(matches(h, &rules(c, p)?))
}
pub(super) fn hidden_sql() -> &'static str {
    r#"(
 EXISTS(SELECT 1 FROM asset_discovery_domain_exclusions dx WHERE dx.project_id=asset_discovery_seeds.project_id AND dx.enabled=1 AND (asset_discovery_seeds.host=dx.domain OR asset_discovery_seeds.host LIKE '%.'||dx.domain))
 OR (EXISTS(SELECT 1 FROM asset_discovery_excluded_evidence x JOIN asset_discovery_domain_exclusions dx ON dx.project_id=x.project_id AND dx.domain=x.domain AND dx.enabled=1 WHERE x.project_id=asset_discovery_seeds.project_id AND x.host=asset_discovery_seeds.host AND x.source=asset_discovery_seeds.source AND x.source_ref=asset_discovery_seeds.source_ref)
 AND NOT EXISTS(SELECT 1 FROM asset_discovery_seed_evidence e WHERE e.project_id=asset_discovery_seeds.project_id AND e.host=asset_discovery_seeds.host AND e.source=asset_discovery_seeds.source AND NOT EXISTS(SELECT 1 FROM asset_discovery_excluded_evidence x JOIN asset_discovery_domain_exclusions dx ON dx.project_id=x.project_id AND dx.domain=x.domain AND dx.enabled=1 WHERE x.project_id=e.project_id AND x.host=e.host AND x.source=e.source AND x.source_ref=e.source_ref))))"#
}
pub(super) fn reference_blocked(
    c: &Connection,
    p: i64,
    h: &str,
    source: &str,
    reference: &str,
) -> Result<bool, String> {
    if !exists(c)? {
        return Ok(false);
    }
    c.query_row("SELECT EXISTS(SELECT 1 FROM asset_discovery_excluded_evidence x JOIN asset_discovery_domain_exclusions d ON d.project_id=x.project_id AND d.domain=x.domain AND d.enabled=1 WHERE x.project_id=?1 AND x.host=?2 AND x.source=?3 AND x.source_ref=?4)",params![p,h,source,reference],|r|r.get(0)).map_err(err)
}
fn reference_matches(
    c: &Connection,
    p: i64,
    h: &str,
    source: &str,
    reference: &str,
    domain: &str,
) -> Result<bool, String> {
    if let Some(id) = reference
        .strip_prefix("asset:")
        .and_then(|id| id.parse::<i64>().ok())
    {
        let row:Option<(String,String,String)>=c.query_row("SELECT a.host,a.link,a.extra_json FROM assets a JOIN project_assets pa ON pa.asset_id=a.id WHERE pa.project_id=?1 AND a.id=?2",params![p,id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(err)?;
        if let Some((endpoint, link, extra)) = row {
            if host(if link.is_empty() { &endpoint } else { &link })
                .is_some_and(|h| in_scope(&h, domain))
            {
                return Ok(true);
            }
            if source == "certificate_sha256" {
                let v: Value = serde_json::from_str(&extra).unwrap_or(Value::Null);
                return Ok(evidence::identities(&v).into_iter().any(|(hash, names)| {
                    hash == h && !names.is_empty() && names.iter().all(|n| in_scope(n, domain))
                }));
            }
        }
        return Ok(false);
    }
    Ok(host(reference).is_some_and(|h| in_scope(&h, domain)))
}
/// Called for just the inserted provenance edge; a shared hash is never globally deleted.
pub(super) fn observe(
    c: &Connection,
    p: i64,
    h: &str,
    source: &str,
    reference: &str,
) -> Result<(), String> {
    for domain in rules(c, p)? {
        if reference_matches(c, p, h, source, reference, &domain)? {
            c.execute(
                "INSERT OR IGNORE INTO asset_discovery_excluded_evidence VALUES(?1,?2,?3,?4,?5)",
                params![p, h, source, reference, domain],
            )
            .map_err(err)?;
        } else {
            c.execute("DELETE FROM asset_discovery_excluded_evidence WHERE project_id=?1 AND host=?2 AND source=?3 AND source_ref=?4 AND domain=?5",params![p,h,source,reference,domain]).map_err(err)?;
        }
    }
    Ok(())
}
fn domain(raw: &str) -> Result<String, String> {
    let h = host(raw)
        .filter(|h| raw.trim().trim_end_matches('.').eq_ignore_ascii_case(h))
        .ok_or("请输入单个完整域名，不接受搜索通配符、URL、IP 或证书指纹")?;
    if hunter_expansion::fingerprint(&h).is_some()
        || crate::commands::ownership::ownership_domain_parts(&h)
            .and_then(|p| p.registrable)
            .is_none()
    {
        return Err("不能将公共后缀作为排除域名；请填写具体域名".into());
    }
    Ok(h)
}
#[derive(Clone)]
struct Seed {
    id: i64,
    h: String,
    source: String,
    reference: String,
}
struct Plan {
    receipt: Value,
    edges: Vec<(String, String, String)>,
}
fn plan(c: &Connection, p: i64, raw: &str) -> Result<Plan, String> {
    project(c, p)?;
    let d = domain(raw)?;
    let mut seeds = vec![];
    let mut refs: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    if super::exists(c)? {
        let mut q=c.prepare("SELECT id,host,source,source_ref FROM asset_discovery_seeds WHERE project_id=?1 ORDER BY id").map_err(err)?;
        for row in q
            .query_map([p], |r| {
                Ok(Seed {
                    id: r.get(0)?,
                    h: r.get(1)?,
                    source: r.get(2)?,
                    reference: r.get(3)?,
                })
            })
            .map_err(err)?
        {
            let s = row.map_err(err)?;
            refs.entry((s.h.clone(), s.source.clone()))
                .or_default()
                .insert(s.reference.clone());
            seeds.push(s);
        }
        if c.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='asset_discovery_seed_evidence')",
            [],
            |r| r.get::<_, bool>(0),
        )
        .map_err(err)?
        {
            let mut q=c.prepare("SELECT host,source,source_ref FROM asset_discovery_seed_evidence WHERE project_id=?1 ORDER BY host,source,source_ref").map_err(err)?;
            for row in q
                .query_map([p], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })
                .map_err(err)?
            {
                let (h, s, r) = row.map_err(err)?;
                refs.entry((h, s)).or_default().insert(r);
            }
        }
    }
    // Preserve all provenance edges so independent references protect shared fingerprints.
    let mut matched = HashMap::new();
    let mut edges = vec![];
    let mut hasher = Sha256::new();
    hasher.update(format!("{p}\0{d}\0"));
    for ((h, s), references) in &refs {
        for reference in references {
            let hit = reference_matches(c, p, h, s, reference, &d)?;
            hasher.update(serde_json::to_vec(&(h, s, reference, hit)).map_err(err)?);
            if hit {
                edges.push((h.clone(), s.clone(), reference.clone()));
            }
            matched.insert((h.clone(), s.clone(), reference.clone()), hit);
        }
    }
    let mut names = 0;
    let mut fingerprints = 0;
    let mut protected = 0;
    let mut samples = vec![];
    for s in &seeds {
        let direct = in_scope(&s.h, &d);
        let references = &refs[&(s.h.clone(), s.source.clone())];
        let any = references
            .iter()
            .any(|r| matched[&(s.h.clone(), s.source.clone(), r.clone())]);
        let all = any
            && references
                .iter()
                .all(|r| matched[&(s.h.clone(), s.source.clone(), r.clone())]);
        hasher.update(s.id.to_le_bytes());
        if direct || all {
            if s.source == "certificate_sha256" {
                fingerprints += 1
            } else {
                names += 1
            }
            if samples.len() < 12 {
                samples.push(json!({"host":s.h,"source":s.source,"reference":s.reference}));
            }
        } else if any && s.source == "certificate_sha256" {
            protected += 1;
        }
    }
    let active = rules(c, p)?;
    hasher.update(serde_json::to_vec(&active).map_err(err)?);
    let scopes = roots(c, p)?;
    hasher.update(serde_json::to_vec(&scopes).map_err(err)?);
    Ok(Plan {
        receipt: json!({"domain":d,"domainLeads":names,"relatedFingerprints":fingerprints,"protectedSharedFingerprints":protected,"total":names+fingerprints,"samples":samples,"scopeConflict":scopes.iter().any(|r|in_scope(r,&d)||in_scope(&d,r)),"digest":format!("{:x}",hasher.finalize()),"assetsDeleted":0,"networkRequested":false}),
        edges,
    })
}
pub(super) fn list_on(c: &Connection, p: i64) -> Result<Value, String> {
    project(c, p)?;
    let domains = rules(c, p)?;
    Ok(json!({"domains":domains,"networkRequested":false}))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExclusionInput {
    pub project_id: i64,
    pub domain: String,
    pub action: String,
    pub expected_digest: Option<String>,
}
pub(super) fn manage(c: &mut Connection, input: ExclusionInput) -> Result<Value, String> {
    let p = input.project_id;
    let d = domain(&input.domain)?;
    project(c, p)?;
    if input.action == "preview" {
        return Ok(plan(c, p, &d)?.receipt);
    }
    if !["apply", "restore"].contains(&input.action.as_str()) {
        return Err("排除操作无效".into());
    }
    ensure(c)?;
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    tx.execute_batch(SCHEMA).map_err(err)?;
    let current = plan(&tx, p, &d)?;
    if input.expected_digest.as_deref() != current.receipt["digest"].as_str() {
        return Err("线索或排除规则已变化，请重新预览后再操作".into());
    }
    tx.execute("INSERT INTO asset_discovery_domain_exclusions(project_id,domain,enabled) VALUES(?1,?2,?3) ON CONFLICT(project_id,domain) DO UPDATE SET enabled=excluded.enabled,updated_at=datetime('now')",params![p,d,input.action=="apply"]).map_err(err)?;
    if input.action == "apply" {
        tx.execute(
            "DELETE FROM asset_discovery_excluded_evidence WHERE project_id=?1 AND domain=?2",
            params![p, d],
        )
        .map_err(err)?;
        for (h, s, r) in current.edges {
            tx.execute(
                "INSERT INTO asset_discovery_excluded_evidence VALUES(?1,?2,?3,?4,?5)",
                params![p, h, s, r, d],
            )
            .map_err(err)?;
        }
    }
    tx.execute("INSERT INTO asset_discovery_exclusion_events(project_id,domain,action,receipt_json) VALUES(?1,?2,?3,?4)",params![p,d,input.action,current.receipt.to_string()]).map_err(err)?;
    tx.commit().map_err(err)?;
    Ok(
        json!({"receipt":current.receipt,"domains":rules(c,p)?,"action":input.action,"assetsDeleted":0,"networkRequested":false}),
    )
}
#[tauri::command]
pub async fn get_discovery_domain_exclusions(
    state: tauri::State<'_, AppState>,
    project_id: i64,
) -> Result<Value, String> {
    let path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        list_on(&harvest::open_existing(&path, false)?, project_id)
    })
    .await
    .map_err(err)?
}
#[tauri::command]
pub async fn manage_discovery_domain_exclusion(
    state: tauri::State<'_, AppState>,
    input: ExclusionInput,
) -> Result<Value, String> {
    let path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        manage(
            &mut harvest::open_existing(&path, input.action != "preview")?,
            input,
        )
    })
    .await
    .map_err(err)?
}

#[cfg(test)]
#[path = "discovery_exclusions_tests.rs"]
mod tests;
