//! Explicit, resumable local evidence refresh. No network/model request lives here.
//! The original seed snapshot and paid/manual receipts remain immutable. Only
//! derived per-asset observations are refreshed, with the old value journaled.
use super::*;

const GROUPS_PER_TICK: usize = 25;
const ASSETS_PER_TICK: i64 = 100;
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS asset_cleanup_local_refresh(run_id TEXT PRIMARY KEY,project_id INTEGER NOT NULL,generation INTEGER NOT NULL,cursor TEXT NOT NULL DEFAULT '',current_key TEXT NOT NULL DEFAULT '',asset_cursor INTEGER NOT NULL DEFAULT -1,include_evidence INTEGER NOT NULL DEFAULT 0,state TEXT NOT NULL DEFAULT 'pending',checked_groups INTEGER NOT NULL DEFAULT 0,awakened_groups INTEGER NOT NULL DEFAULT 0,updated_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS asset_cleanup_fact_refresh_history(id INTEGER PRIMARY KEY,run_id TEXT NOT NULL,asset_id INTEGER NOT NULL,endpoint TEXT NOT NULL,observations_json TEXT NOT NULL,signature TEXT NOT NULL,mixed INTEGER NOT NULL,refreshed_at INTEGER NOT NULL);
";

fn ensure(c: &Connection) -> Result<(), String> {
    c.execute_batch(SCHEMA).map_err(err)
}
fn generation(c: &Connection, p: i64, run: &str) -> Result<i64, String> {
    c.query_row("SELECT r.generation FROM asset_cleanup_runs r JOIN projects p ON p.id=r.project_id AND p.status='active' WHERE r.id=?1 AND r.project_id=?2 AND r.enabled=1",params![run,p],|r|r.get(0)).optional().map_err(err)?.ok_or_else(||"cleanup_refresh_run_project_or_state_mismatch".into())
}

/// Call inside the normal resume owner transaction, after its generation update.
/// It requests one bounded scan; merely polling status never requests a refresh.
pub(super) fn request_on_resume(c: &Connection, p: i64, run: &str) -> Result<Value, String> {
    let gen = generation(c, p, run)?;
    ensure(c)?;
    c.execute("INSERT INTO asset_cleanup_local_refresh(run_id,project_id,generation,updated_at) VALUES(?1,?2,?3,?4) ON CONFLICT(run_id) DO UPDATE SET project_id=excluded.project_id,generation=excluded.generation,cursor='',current_key='',asset_cursor=-1,include_evidence=0,state='pending',checked_groups=0,awakened_groups=0,updated_at=excluded.updated_at",params![run,p,gen,now()]).map_err(err)?;
    Ok(
        json!({"requested":true,"runId":run,"generation":gen,"networkRequested":false,"modelRequested":false,"snapshotChanged":false}),
    )
}

/// Call after accepting changed target_names_json in the organization owner.
/// Unit-directory changes affect existing evidence even when its own basis did
/// not change. Same normalized names, reordered names, and whitespace do not.
pub(super) fn request_targets_changed(
    c: &Connection,
    p: i64,
    run: &str,
    old: &str,
    new: &str,
) -> Result<bool, String> {
    fn names(raw: &str) -> Result<BTreeSet<String>, String> {
        let values: Vec<String> = serde_json::from_str(raw).map_err(err)?;
        Ok(values
            .into_iter()
            .map(|s| review::scope::organization_key(&s))
            .filter(|s| !s.is_empty())
            .collect())
    }
    if names(old)? == names(new)? {
        return Ok(false);
    }
    let gen = generation(c, p, run)?;
    ensure(c)?;
    c.execute("INSERT INTO asset_cleanup_local_refresh(run_id,project_id,generation,include_evidence,updated_at) VALUES(?1,?2,?3,1,?4) ON CONFLICT(run_id) DO UPDATE SET project_id=excluded.project_id,generation=excluded.generation,cursor='',current_key='',asset_cursor=-1,include_evidence=1,state='pending',checked_groups=0,awakened_groups=0,updated_at=excluded.updated_at",params![run,p,gen,now()]).map_err(err)?;
    Ok(true)
}

/// Identity keys deliberately omit timestamps, asset samples/counts, certificate
/// hashes and duplicate source receipts. Renewing a certificate with the same
/// identity is not a reason to replay a paid model request. Genuine new current
/// local leaf evidence may strengthen an existing identity witness.
fn identities(e: &Value) -> BTreeMap<String, bool> {
    let mut result = BTreeMap::<String, bool>::new();
    for f in e["domainEvidence"].as_array().into_iter().flatten() {
        let h = f["host"].as_str().unwrap_or("");
        let org = review::scope::organization_key(f["organization"].as_str().unwrap_or(""));
        let source = f["source"].as_str().unwrap_or("");
        if h.is_empty() || org.is_empty() || source.is_empty() {
            continue;
        }
        let mut key = json!([h, org, source, f["wildcard"] == true]);
        if source == "official_unit_relation" {
            key.as_array_mut()
                .unwrap()
                .push(json!(review::scope::organization_key(
                    f["parentOrganization"].as_str().unwrap_or("")
                )));
        }
        if source == "website_icp_declared" {
            key.as_array_mut()
                .unwrap()
                .push(f["registrationNumber"].clone());
        }
        if matches!(
            source,
            "official_site_operator" | "website_icp_declared" | "official_unit_relation"
        ) {
            key.as_array_mut().unwrap().push(json!(f["sourceUrl"]
                .as_str()
                .and_then(|u| reqwest::Url::parse(u).ok())
                .map(|u| u.scheme().to_owned())));
        }
        let current = f["currentLocalLeaf"] == true;
        result
            .entry(key.to_string())
            .and_modify(|s| *s |= current)
            .or_insert(current);
    }
    result
}
fn new_material(old: &Value, new: &Value) -> bool {
    let old = identities(old);
    identities(new)
        .into_iter()
        .any(|(key, current)| old.get(&key).is_none_or(|before| current && !*before))
}

fn refresh_asset(c: &Connection, k: &work::Claim, asset: i64) -> Result<(), String> {
    let prior:Option<(String,String,String,bool)>=c.query_row("SELECT endpoint,observations_json,signature,mixed FROM asset_cleanup_facts WHERE run_id=?1 AND asset_id=?2",params![k.id,asset],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(err)?;
    let live:Option<(String,String)>=c.query_row("SELECT COALESCE(NULLIF(a.link,''),a.host),a.extra_json FROM assets a JOIN project_assets p ON p.asset_id=a.id WHERE p.project_id=?1 AND a.id=?2",params![k.p,asset],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(err)?;
    let (endpoint, observations, mixed) = if let Some((endpoint, raw)) = live {
        let value: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
        let observations = serde_json::to_value(review::provenance::ownership_observations(
            &value, &endpoint,
        ))
        .map_err(err)?;
        let mixed = evidence::identities(&value).iter().any(|(_, names)| {
            names
                .iter()
                .map(|n| {
                    crate::commands::ownership::ownership_domain_parts(n)
                        .and_then(|p| p.registrable)
                        .unwrap_or_else(|| n.clone())
                })
                .collect::<BTreeSet<_>>()
                .len()
                > 1
        });
        (endpoint, observations, mixed)
    } else {
        // Removed/unlinked assets cannot keep attesting old identities. The old
        // derived receipt is retained below; original seed rows are untouched.
        (String::new(), json!([]), false)
    };
    let signature = hash(&observations);
    if prior
        .as_ref()
        .is_some_and(|(e, _, s, m)| *e == endpoint && *s == signature && *m == mixed)
    {
        return Ok(());
    }
    if let Some((endpoint, observations, signature, mixed)) = prior {
        c.execute("INSERT INTO asset_cleanup_fact_refresh_history(run_id,asset_id,endpoint,observations_json,signature,mixed,refreshed_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![k.id,asset,endpoint,observations,signature,mixed,now()]).map_err(err)?;
    }
    c.execute("INSERT INTO asset_cleanup_facts(run_id,asset_id,endpoint,observations_json,signature,mixed) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(run_id,asset_id) DO UPDATE SET endpoint=excluded.endpoint,observations_json=excluded.observations_json,signature=excluded.signature,mixed=excluded.mixed",params![k.id,asset,endpoint,observations.to_string(),signature,mixed]).map_err(err)?;
    Ok(())
}

/// Call before pending-group classification in the worker. Each tick handles at
/// most 25 groups and 100 derived asset observations. A large group resumes from
/// its asset cursor. No source attempts, exhaustion, model items, manual reviews,
/// exclusions, authorized roots or original seed snapshot rows are reset.
pub(super) fn tick(c: &mut Connection, k: &work::Claim) -> Result<bool, String> {
    if !work::claim_valid(c, k)? || !has(c, "asset_cleanup_local_refresh")? {
        return Ok(false);
    }
    let job:Option<(String,String,i64,bool)>=c.query_row("SELECT cursor,current_key,asset_cursor,include_evidence FROM asset_cleanup_local_refresh WHERE run_id=?1 AND project_id=?2 AND generation=?3 AND state='pending'",params![k.id,k.p,k.generation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(err)?;
    let Some((mut cursor, mut current, mut asset_cursor, include_evidence)) = job else {
        return Ok(false);
    };
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    if !work::claim_valid(&tx, k)? {
        return Ok(false);
    }
    let mut groups = 0usize;
    let mut budget = ASSETS_PER_TICK;
    while groups < GROUPS_PER_TICK && budget > 0 {
        if current.is_empty() {
            let key:Option<String>=tx.query_row("SELECT group_key FROM asset_cleanup_groups WHERE run_id=?1 AND group_key>?2 AND (state='deferred' OR (?3 AND state='evidence')) ORDER BY group_key LIMIT 1",params![k.id,cursor,include_evidence],|r|r.get(0)).optional().map_err(err)?;
            let Some(key) = key else {
                tx.execute("UPDATE asset_cleanup_local_refresh SET state='completed',cursor=?4,current_key='',asset_cursor=-1,updated_at=?5 WHERE run_id=?1 AND project_id=?2 AND generation=?3",params![k.id,k.p,k.generation,cursor,now()]).map_err(err)?;
                tx.commit().map_err(err)?;
                return Ok(true);
            };
            current = key;
            asset_cursor = -1;
        }
        let eligible:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM asset_cleanup_groups WHERE run_id=?1 AND group_key=?2 AND (state='deferred' OR (?3 AND state='evidence')))",params![k.id,current,include_evidence],|r|r.get(0)).map_err(err)?;
        if !eligible {
            cursor = current.clone();
            current.clear();
            asset_cursor = -1;
            groups += 1;
            continue;
        }
        let ids:Vec<i64>=tx.prepare("SELECT asset_id FROM asset_cleanup_links WHERE run_id=?1 AND group_key=?2 AND asset_id>?3 ORDER BY asset_id LIMIT ?4").map_err(err)?.query_map(params![k.id,current,asset_cursor,budget],|r|r.get(0)).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;
        for asset in &ids {
            refresh_asset(&tx, k, *asset)?;
            asset_cursor = *asset;
            budget -= 1;
        }
        let remaining:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM asset_cleanup_links WHERE run_id=?1 AND group_key=?2 AND asset_id>?3)",params![k.id,current,asset_cursor],|r|r.get(0)).map_err(err)?;
        if remaining {
            break;
        }
        let (raw,before):(String,String)=tx.query_row("SELECT evidence_json,basis_hash FROM asset_cleanup_groups WHERE run_id=?1 AND group_key=?2",params![k.id,current],|r|Ok((r.get(0)?,r.get(1)?))).map_err(err)?;
        let previous: Value = serde_json::from_str(&raw).unwrap_or_else(|_| json!({}));
        let mut fresh = work::evidence_for(&tx, k, &current)?;
        let changed = work::basis(&fresh) != before && new_material(&previous, &fresh);
        let awaken = changed || include_evidence;
        // Keep policy/AI history visible without using its stale basis as a new
        // paid request. classify will enforce the existing exact-basis fence.
        for field in ["aiAdvice", "completionReason", "researchOutcome"] {
            if previous.get(field).is_some() {
                fresh[field] = previous[field].clone();
            }
        }
        fresh["localRefresh"] = json!({"checkedAt":now(),"generation":k.generation,"newIdentityEvidence":changed,"targetDirectoryChanged":include_evidence,"snapshotChanged":false,"networkRequested":false,"modelRequested":false});
        // Do not update basis_hash here: a genuine new identity must remain
        // distinguishable from the original accepted/paid decision basis.
        tx.execute("UPDATE asset_cleanup_groups SET state=CASE WHEN ?3 THEN 'pending' ELSE state END,reason=CASE WHEN ?3 THEN '本地发现新增有效身份或单位关系证据，复用原回执并重新核验；未恢复耗尽来源' ELSE reason END,evidence_json=?4,revision=revision+1 WHERE run_id=?1 AND group_key=?2 AND (state='deferred' OR (?5 AND state='evidence'))",params![k.id,current,awaken,fresh.to_string(),include_evidence]).map_err(err)?;
        tx.execute("UPDATE asset_cleanup_local_refresh SET checked_groups=checked_groups+1,awakened_groups=awakened_groups+?4 WHERE run_id=?1 AND project_id=?2 AND generation=?3",params![k.id,k.p,k.generation,awaken]).map_err(err)?;
        cursor = current.clone();
        current.clear();
        asset_cursor = -1;
        groups += 1;
    }
    tx.execute("UPDATE asset_cleanup_local_refresh SET cursor=?4,current_key=?5,asset_cursor=?6,updated_at=?7 WHERE run_id=?1 AND project_id=?2 AND generation=?3 AND state='pending'",params![k.id,k.p,k.generation,cursor,current,asset_cursor,now()]).map_err(err)?;
    tx.commit().map_err(err)?;
    Ok(true)
}
