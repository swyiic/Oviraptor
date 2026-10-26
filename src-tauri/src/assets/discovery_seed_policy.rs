//! Exact seed decisions, shared by presentation, discovery and paid admission.
//! No asset/source deletion, suffix expansion, network request or worker resume.
use super::*;
use serde::Serialize;
const SCHEMA:&str="CREATE TABLE IF NOT EXISTS asset_seed_exact_rules(project_id INTEGER NOT NULL,kind TEXT NOT NULL,value TEXT NOT NULL,wildcard INTEGER NOT NULL,enabled INTEGER NOT NULL,batch_id TEXT NOT NULL,PRIMARY KEY(project_id,kind,value,wildcard));
CREATE TABLE IF NOT EXISTS asset_seed_exact_batches(id TEXT PRIMARY KEY,project_id INTEGER NOT NULL,origin TEXT NOT NULL,receipt_json TEXT NOT NULL,restored INTEGER NOT NULL DEFAULT 0,created_at TEXT NOT NULL DEFAULT(datetime('now')));
CREATE TABLE IF NOT EXISTS asset_seed_exact_operations(project_id INTEGER NOT NULL,id TEXT NOT NULL,args_hash TEXT NOT NULL,result_json TEXT NOT NULL,PRIMARY KEY(project_id,id));";
pub(super) fn has(c: &Connection, table: &str) -> Result<bool, String> {
    c.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [table],
        |r| r.get(0),
    )
    .map_err(err)
}
pub(super) fn exact_sql(c: &Connection, alias: &str) -> Result<String, String> {
    if !has(c, "asset_seed_exact_rules")? {
        return Ok("0".into());
    }
    Ok(format!("EXISTS(SELECT 1 FROM asset_seed_exact_rules sx WHERE sx.project_id={alias}.project_id AND sx.enabled=1 AND sx.value={alias}.value AND sx.kind={alias}.kind AND sx.wildcard={alias}.wildcard)"))
}
fn manual_ready(c: &Connection) -> Result<bool, String> {
    if !has(c, "asset_seed_review_items")? || !has(c, "asset_seed_reviews")? {
        return Ok(false);
    }
    let fields = c
        .prepare("PRAGMA table_info(asset_seed_review_items)")
        .map_err(err)?
        .query_map([], |r| r.get::<_, String>(1))
        .map_err(err)?
        .collect::<Result<BTreeSet<_>, _>>()
        .map_err(err)?;
    Ok(["kind", "target", "decision", "evidence_json", "run_id"]
        .iter()
        .all(|s| fields.contains(*s)))
}
pub(super) fn manual_sql(c: &Connection) -> Result<String, String> {
    if !manual_ready(c)? {
        return Ok("SELECT NULL value,0 accepted,0 rejected WHERE 0".into());
    }
    Ok("SELECT lower(n.value) value,max(i.decision='accepted') accepted,max(i.decision='rejected') rejected FROM asset_seed_review_items i JOIN asset_seed_reviews r ON r.id=i.run_id JOIN json_each(i.evidence_json,'$.domains') n WHERE r.project_id=?1 AND i.decision IN ('accepted','rejected') AND i.kind IN ('domain','host') AND lower(i.target)=lower(n.value) AND n.type='text' GROUP BY lower(n.value)".into())
}
pub(super) fn raw_sql(c: &Connection) -> Result<String, String> {
    let exact = if has(c, "asset_seed_exact_rules")? {
        "EXISTS(SELECT 1 FROM asset_seed_exact_rules sx WHERE sx.project_id=asset_discovery_seeds.project_id AND sx.enabled=1 AND sx.value=asset_discovery_seeds.host AND sx.wildcard=asset_discovery_seeds.wildcard AND sx.kind=CASE WHEN asset_discovery_seeds.source='certificate_sha256' THEN 'fingerprint' WHEN asset_discovery_seeds.source IN ('cname','dns_cname') THEN 'cname' ELSE 'domain' END)"
    } else {
        "0"
    };
    // Only explicit name decisions apply here. Rejecting a mixed certificate is not a rejection of every SAN.
    let rejected = if manual_ready(c)? {
        "asset_discovery_seeds.source NOT IN ('certificate_sha256','cname','dns_cname') AND EXISTS(SELECT 1 FROM asset_seed_review_items i JOIN asset_seed_reviews r ON r.id=i.run_id JOIN json_each(i.evidence_json,'$.domains') n WHERE r.project_id=asset_discovery_seeds.project_id AND i.decision='rejected' AND i.kind IN ('domain','host') AND i.target=n.value AND n.value=asset_discovery_seeds.host) AND NOT EXISTS(SELECT 1 FROM asset_seed_review_items i JOIN asset_seed_reviews r ON r.id=i.run_id JOIN json_each(i.evidence_json,'$.domains') n WHERE r.project_id=asset_discovery_seeds.project_id AND i.decision='accepted' AND i.kind IN ('domain','host') AND i.target=n.value AND n.value=asset_discovery_seeds.host)"
    } else {
        "0"
    };
    if exact == "0" && rejected == "0" {
        Ok("0".into())
    } else {
        Ok(format!("({exact} OR ({rejected}))"))
    }
}
pub(super) fn blocked(c: &Connection, p: i64, kind: &str, value: &str) -> Result<bool, String> {
    let kind = if kind == "domain_tree" {
        "domain"
    } else {
        kind
    };
    if has(c,"asset_seed_exact_rules")?&&c.query_row("SELECT EXISTS(SELECT 1 FROM asset_seed_exact_rules WHERE project_id=?1 AND kind=?2 AND value=?3 AND enabled=1)",params![p,kind,value],|r|r.get::<_,bool>(0)).map_err(err)?{return Ok(true)}
    if kind == "domain" {
        return c.query_row(&format!("SELECT EXISTS(SELECT 1 FROM ({}) WHERE value=?2 AND rejected=1 AND accepted=0)",manual_sql(c)?),params![p,value],|r|r.get(0)).map_err(err);
    }
    Ok(false)
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Input {
    pub project_id: i64,
    pub action: String,
    #[serde(default)]
    pub seed_ids: Vec<i64>,
    pub query: Option<seed_ledger::Query>,
    pub batch_page: Option<i64>,
    pub expected_digest: Option<String>,
    pub operation_id: Option<String>,
    pub batch_id: Option<String>,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub(super) struct Identity {
    pub kind: String,
    pub value: String,
    pub wildcard: bool,
}
pub(super) fn apply_on(
    c: &Connection,
    p: i64,
    ids: &[Identity],
    batch: &str,
    origin: &str,
    basis: &Value,
) -> Result<Value, String> {
    if c.is_autocommit() {
        return Err("seed_decision_transaction_required".into());
    }
    c.execute_batch(SCHEMA).map_err(err)?;
    let mut changed = vec![];
    for id in ids {
        let count=c.execute("INSERT INTO asset_seed_exact_rules VALUES(?1,?2,?3,?4,1,?5) ON CONFLICT(project_id,kind,value,wildcard) DO UPDATE SET enabled=1,batch_id=excluded.batch_id WHERE asset_seed_exact_rules.enabled=0",params![p,id.kind,id.value,id.wildcard,batch]).map_err(err)?;
        if count > 0 {
            changed.push(id.clone());
        }
    }
    let result = json!({"batchId":batch,"count":changed.len(),"identities":changed,"assetsDeleted":0,"networkRequested":false,"basis":basis});
    c.execute("INSERT INTO asset_seed_exact_batches(id,project_id,origin,receipt_json) VALUES(?1,?2,?3,?4)",params![batch,p,origin,result.to_string()]).map_err(err)?;
    Ok(result)
}
fn plan(c: &Connection, i: &Input) -> Result<Value, String> {
    if i.seed_ids.len() > 100000 || (i.query.is_some() && !i.seed_ids.is_empty()) {
        return Err("seed_selection_invalid".into());
    }
    let ids = seed_ledger::selected_ids(c, i.project_id, &i.seed_ids, i.query.as_ref())?;
    let identities: Vec<Identity> = ids
        .iter()
        .map(|id| {
            c.query_row(
                "SELECT kind,value,wildcard FROM asset_project_seeds WHERE project_id=?1 AND id=?2",
                params![i.project_id, id],
                |r| {
                    Ok(Identity {
                        kind: r.get(0)?,
                        value: r.get(1)?,
                        wildcard: r.get(2)?,
                    })
                },
            )
            .map_err(err)
        })
        .collect::<Result<_, _>>()?;
    let digest = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&json!([i.project_id, ids, identities])).map_err(err)?)
    );
    Ok(
        json!({"count":identities.len(),"digest":digest,"identities":identities,"assetsDeleted":0,"networkRequested":false,"scope":"exact_seed_identity"}),
    )
}
pub(crate) fn manage(c: &mut Connection, i: Input) -> Result<Value, String> {
    project(c, i.project_id)?;
    if i.action == "history" {
        return batch_history(c, i.project_id, i.batch_page.unwrap_or(1));
    }
    if i.action == "preview" {
        let mut preview = plan(c, &i)?;
        preview["samples"] = json!(preview["identities"]
            .as_array()
            .into_iter()
            .flatten()
            .take(8)
            .cloned()
            .collect::<Vec<_>>());
        preview
            .as_object_mut()
            .ok_or("seed_preview_invalid")?
            .remove("identities");
        return Ok(preview);
    }
    if !matches!(i.action.as_str(), "apply" | "restore") {
        return Err("seed_decision_action_invalid".into());
    }
    let op = i
        .operation_id
        .as_deref()
        .filter(|v| uuid::Uuid::parse_str(v).is_ok())
        .ok_or("seed_decision_operation_required")?;
    let hash = format!("{:x}", Sha256::digest(serde_json::to_vec(&i).map_err(err)?));
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    tx.execute_batch(SCHEMA).map_err(err)?;
    if let Some((prior,result))=tx.query_row("SELECT args_hash,result_json FROM asset_seed_exact_operations WHERE project_id=?1 AND id=?2",params![i.project_id,op],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional().map_err(err)?{
  if prior!=hash{return Err("seed_decision_operation_conflict".into())}return serde_json::from_str(&result).map_err(err)
 }
    let mut result = if i.action == "apply" {
        let preview = plan(&tx, &i)?;
        if preview["digest"].as_str() != i.expected_digest.as_deref() {
            return Err("种子或排除状态已变化，请重新预览".into());
        }
        let identities: Vec<Identity> =
            serde_json::from_value(preview["identities"].clone()).map_err(err)?;
        if identities.is_empty() {
            return Err("没有可排除的种子".into());
        }
        apply_on(
            &tx,
            i.project_id,
            &identities,
            op,
            "manual",
            &json!({"explicitSelection":true}),
        )?
    } else {
        let batch = i
            .batch_id
            .as_deref()
            .ok_or("seed_decision_batch_required")?;
        let restored: bool = tx
            .query_row(
                "SELECT restored FROM asset_seed_exact_batches WHERE project_id=?1 AND id=?2",
                params![i.project_id, batch],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?
            .ok_or("seed_decision_batch_unavailable")?;
        let count = if restored {
            0
        } else {
            tx.execute("UPDATE asset_seed_exact_rules SET enabled=0 WHERE project_id=?1 AND batch_id=?2 AND enabled=1",params![i.project_id,batch]).map_err(err)?
        };
        tx.execute(
            "UPDATE asset_seed_exact_batches SET restored=1 WHERE project_id=?1 AND id=?2",
            params![i.project_id, batch],
        )
        .map_err(err)?;
        json!({"batchId":batch,"restored":count,"assetsDeleted":0,"networkRequested":false})
    };
    // Full selected identities and proof stay in the durable batch receipt,
    // never force a many-megabyte response into the native UI for select-all.
    if let Some(object) = result.as_object_mut() {
        object.remove("identities");
        object.remove("basis");
    }
    tx.execute(
        "INSERT INTO asset_seed_exact_operations VALUES(?1,?2,?3,?4)",
        params![i.project_id, op, hash, result.to_string()],
    )
    .map_err(err)?;
    tx.commit().map_err(err)?;
    Ok(result)
}
pub(super) fn batches(c: &Connection, p: i64) -> Result<Value, String> {
    Ok(batch_history(c, p, 1)?["items"].clone())
}
fn batch_history(c: &Connection, p: i64, page: i64) -> Result<Value, String> {
    if !(1..=100000).contains(&page) {
        return Err("seed_batch_page_invalid".into());
    }
    if !has(c, "asset_seed_exact_batches")? {
        return Ok(json!({"items":[],"total":0,"page":page}));
    }
    let total: i64 = c
        .query_row(
            "SELECT count(*) FROM asset_seed_exact_batches WHERE project_id=?1",
            [p],
            |r| r.get(0),
        )
        .map_err(err)?;
    let rows=c.prepare("SELECT id,origin,restored,created_at,json_extract(receipt_json,'$.count') FROM asset_seed_exact_batches WHERE project_id=?1 ORDER BY rowid DESC LIMIT 10 OFFSET ?2").map_err(err)?.query_map(params![p,(page-1)*10],|r|Ok(json!({"id":r.get::<_,String>(0)?,"origin":r.get::<_,String>(1)?,"restored":r.get::<_,bool>(2)?,"createdAt":r.get::<_,String>(3)?,"count":r.get::<_,i64>(4)?}))).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)?;
    Ok(json!({"items":rows,"total":total,"page":page}))
}
/// Local-only reconciliation is independent of the network worker. It may run
/// while paused; it neither acquires a worker claim nor changes its generation.
pub(super) fn reconcile_on(c: &mut Connection, p: i64) -> Result<Value, String> {
    if project(c, p).is_err() {
        return Ok(json!({"skipped":0,"excluded":0,"reason":"project_not_active"}));
    }
    if !has(c, "asset_cleanup_runs")? {
        return Ok(json!({"skipped":0,"excluded":0}));
    }
    let run:Option<(String,String,bool)>=c.query_row("SELECT id,target_names_json,auto_exclude FROM asset_cleanup_runs WHERE project_id=?1 AND phase='paused' AND enabled=0 AND owner_pid=0 AND claim_token='' AND lease_until=0 ORDER BY created_at DESC LIMIT 1",[p],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(err)?;
    let Some((run, raw, auto)) = run else {
        return Ok(json!({"skipped":0,"excluded":0,"reason":"no_quiescent_paused_run"}));
    };
    let targets: BTreeSet<String> = serde_json::from_str(&raw).map_err(err)?;
    let groups:Vec<(String,String)>=c.prepare("SELECT group_key,evidence_json FROM asset_cleanup_groups WHERE run_id=?1 AND state IN ('evidence','deferred') AND shared=0 AND json_array_length(evidence_json,'$.domainEvidence')>0").map_err(err)?.query_map([&run],|r|Ok((r.get(0)?,r.get(1)?))).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    let skipped=tx.execute("UPDATE asset_cleanup_research SET state='skipped',claim_token='',next_at=0,error='本轮跳过，不自动重试；'||error WHERE run_id=?1 AND state IN ('retry','pending') AND error!='' AND (kind='public_ownership' OR lower(error) LIKE '%http 502%' OR lower(error) LIKE '%http 503%' OR lower(error) LIKE '%http 504%' OR error LIKE '%error sending request%' OR error LIKE '%network_error_kind=%' OR error LIKE '%请求失败%')",[&run]).map_err(err)?;
    let mut count = 0;
    for (key, raw) in groups {
        if !auto {
            break;
        }
        let evidence: Value = serde_json::from_str(&raw).map_err(err)?;
        let candidates = cleanup::work::exact_candidates(&tx, p, &evidence, &targets)?;
        if candidates.is_empty() {
            continue;
        }
        let batch = uuid::Uuid::new_v4().to_string();
        let result = apply_on(
            &tx,
            p,
            &candidates,
            &batch,
            "automatic",
            &json!({"runId":run,"group":key,"evidence":evidence,"localOnly":true}),
        )?;
        count += result["count"].as_u64().unwrap_or(0);
    }
    tx.commit().map_err(err)?;
    Ok(
        json!({"skipped":skipped,"excluded":count,"networkRequested":false,"modelRequested":false,"workerResumed":false}),
    )
}

pub(super) fn decided(c: &Connection, p: i64, kind: &str, value: &str) -> Result<bool, String> {
    // Human review may change while the network worker is paused. Consult the
    // current decisions as well as the frozen group state before auto-excluding.
    if kind=="domain" && manual_ready(c)? && c.query_row("SELECT EXISTS(SELECT 1 FROM asset_seed_review_items i JOIN asset_seed_reviews r ON r.id=i.run_id JOIN json_each(i.evidence_json,'$.domains') n WHERE r.project_id=?1 AND i.decision IN ('accepted','rejected') AND n.value=?2)",params![p,value],|r|r.get::<_,bool>(0)).map_err(err)? {return Ok(true)}
    if !has(c, "asset_seed_exact_rules")? {
        return Ok(false);
    }
    c.query_row("SELECT EXISTS(SELECT 1 FROM asset_seed_exact_rules WHERE project_id=?1 AND kind=?2 AND value=?3)",params![p,kind,value],|r|r.get(0)).map_err(err)
}

/// Pending independent conflicts are live policy, not a cached AI judgment.
pub(super) fn conflict_sql(c: &Connection) -> Result<String, String> {
    if !has(c, "asset_enrichment_conflicts")? {
        return Ok("SELECT NULL value WHERE 0".into());
    }
    Ok("SELECT identity value FROM asset_enrichment_conflicts WHERE project_id=?1 AND decision='pending' UNION SELECT n.value FROM asset_enrichment_conflicts e,json_each(e.hosts_json) n WHERE e.project_id=?1 AND e.decision='pending' AND n.type='text'".into())
}
