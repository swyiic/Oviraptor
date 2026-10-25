//! Whole-project, reversible seed relevance cleanup. No asset deletion, scope
//! approval, candidate HTTP fetch, shell, FOFA or Hunter lives in this owner.
//! Public research and paid model receipts have independent durable journals.
use super::*;
use std::collections::BTreeMap;
#[path = "discovery_cleanup_network.rs"]
mod network;
#[path = "discovery_cleanup_work.rs"]
mod work;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS asset_cleanup_runs(id TEXT PRIMARY KEY,project_id INTEGER NOT NULL,organization TEXT NOT NULL,recipient TEXT NOT NULL,model TEXT NOT NULL,target_names_json TEXT NOT NULL,roots_json TEXT NOT NULL,auto_exclude INTEGER NOT NULL,phase TEXT NOT NULL,enabled INTEGER NOT NULL,generation INTEGER NOT NULL DEFAULT 1,cursor INTEGER NOT NULL DEFAULT 0,raw_count INTEGER NOT NULL,ledger_count INTEGER NOT NULL,reason TEXT NOT NULL DEFAULT '',owner_pid INTEGER NOT NULL DEFAULT 0,claim_token TEXT NOT NULL DEFAULT '',lease_until INTEGER NOT NULL DEFAULT 0,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL);
CREATE UNIQUE INDEX IF NOT EXISTS asset_cleanup_active ON asset_cleanup_runs(project_id) WHERE enabled=1;
CREATE TABLE IF NOT EXISTS asset_cleanup_seeds(run_id TEXT NOT NULL,source_kind TEXT NOT NULL,source_id INTEGER NOT NULL,host TEXT NOT NULL,source TEXT NOT NULL,source_ref TEXT NOT NULL,wildcard INTEGER NOT NULL,group_key TEXT NOT NULL DEFAULT '',PRIMARY KEY(run_id,source_kind,source_id));
CREATE INDEX IF NOT EXISTS asset_cleanup_snapshot_page ON asset_cleanup_seeds(run_id,group_key);
CREATE TABLE IF NOT EXISTS asset_cleanup_groups(run_id TEXT NOT NULL,group_key TEXT NOT NULL,domain TEXT NOT NULL,shared INTEGER NOT NULL DEFAULT 0,seed_count INTEGER NOT NULL DEFAULT 0,state TEXT NOT NULL DEFAULT 'pending',advice TEXT NOT NULL DEFAULT '',reason TEXT NOT NULL DEFAULT '',basis_hash TEXT NOT NULL DEFAULT '',ai_basis_hash TEXT NOT NULL DEFAULT '',revision INTEGER NOT NULL DEFAULT 0,evidence_json TEXT NOT NULL DEFAULT '{}',PRIMARY KEY(run_id,group_key));
CREATE TABLE IF NOT EXISTS asset_cleanup_facts(run_id TEXT NOT NULL,asset_id INTEGER NOT NULL,endpoint TEXT NOT NULL,observations_json TEXT NOT NULL,signature TEXT NOT NULL,mixed INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(run_id,asset_id));
CREATE TABLE IF NOT EXISTS asset_cleanup_links(run_id TEXT NOT NULL,group_key TEXT NOT NULL,asset_id INTEGER NOT NULL,PRIMARY KEY(run_id,group_key,asset_id));
CREATE TABLE IF NOT EXISTS asset_cleanup_research(id INTEGER PRIMARY KEY,run_id TEXT NOT NULL,group_key TEXT NOT NULL,kind TEXT NOT NULL,target TEXT NOT NULL,state TEXT NOT NULL DEFAULT 'pending',attempts INTEGER NOT NULL DEFAULT 0,next_at INTEGER NOT NULL DEFAULT 0,claim_token TEXT NOT NULL DEFAULT '',result_json TEXT NOT NULL DEFAULT '{}',error TEXT NOT NULL DEFAULT '',UNIQUE(run_id,group_key,kind,target));
CREATE INDEX IF NOT EXISTS asset_cleanup_research_due ON asset_cleanup_research(run_id,state,next_at);
CREATE TABLE IF NOT EXISTS asset_cleanup_provider_backoffs(run_id TEXT NOT NULL,provider TEXT NOT NULL,next_at INTEGER NOT NULL,error TEXT NOT NULL DEFAULT '',updated_at INTEGER NOT NULL,PRIMARY KEY(run_id,provider));
CREATE TABLE IF NOT EXISTS asset_cleanup_models(id TEXT PRIMARY KEY,run_id TEXT NOT NULL,recipient TEXT NOT NULL,request_json TEXT NOT NULL,state TEXT NOT NULL,response_text TEXT NOT NULL DEFAULT '',usage_json TEXT NOT NULL DEFAULT '{}',error TEXT NOT NULL DEFAULT '',created_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS asset_cleanup_model_items(request_id TEXT NOT NULL,run_id TEXT NOT NULL,group_key TEXT NOT NULL,basis_hash TEXT NOT NULL,PRIMARY KEY(request_id,group_key),UNIQUE(run_id,group_key,basis_hash));
CREATE TABLE IF NOT EXISTS asset_cleanup_batches(id TEXT PRIMARY KEY,run_id TEXT NOT NULL,project_id INTEGER NOT NULL,domains_json TEXT NOT NULL,receipt_json TEXT NOT NULL,restored INTEGER NOT NULL DEFAULT 0,created_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS asset_cleanup_operations(project_id INTEGER NOT NULL,operation_id TEXT NOT NULL,action TEXT NOT NULL,args_hash TEXT NOT NULL,run_id TEXT NOT NULL,result_json TEXT NOT NULL,PRIMARY KEY(project_id,operation_id));
";
fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
fn hash(v: &Value) -> String {
    format!("{:x}", Sha256::digest(v.to_string().as_bytes()))
}
fn has(c: &Connection, table: &str) -> Result<bool, String> {
    c.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [table],
        |r| r.get(0),
    )
    .map_err(err)
}
fn ensure(c: &Connection) -> Result<(), String> {
    c.execute_batch(SCHEMA).map_err(err)
}
fn owned(c: &Connection, p: i64, id: &str) -> Result<(), String> {
    let valid: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM asset_cleanup_runs WHERE id=?1 AND project_id=?2)",
            params![id, p],
            |r| r.get(0),
        )
        .map_err(err)?;
    if valid {
        Ok(())
    } else {
        Err("cleanup_run_project_mismatch".into())
    }
}
fn run_id(c: &Connection, p: i64, id: Option<&str>) -> Result<Option<String>, String> {
    if let Some(id) = id {
        owned(c, p, id)?;
        return Ok(Some(id.into()));
    }
    c.query_row("SELECT id FROM asset_cleanup_runs WHERE project_id=?1 ORDER BY created_at DESC,rowid DESC LIMIT 1",[p],|r|r.get(0)).optional().map_err(err)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Start {
    organization: String,
    #[serde(default)]
    auto_exclude_unrelated: bool,
}
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Read {
    run_id: Option<String>,
    page: Option<i64>,
    page_size: Option<i64>,
    filter: Option<String>,
    operation_id: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Control {
    run_id: String,
    batch_id: Option<String>,
}

fn profile(
    c: &Connection,
) -> Result<(crate::agent_runtime::model::GatewayProfile, String, String), String> {
    let settings = crate::model_configuration::default_settings_for(c, &["network"]);
    if settings.get("_configurationError").is_some() {
        return Err("cleanup_network_configuration_unavailable".into());
    }
    let proxy = settings["proxyUrl"].as_str().unwrap_or("").to_owned();
    let mut profile = crate::model_configuration::gateway_profile_on(
        c,
        if proxy.is_empty() { None } else { Some(&proxy) },
    )?;
    let identity = format!(
        "{:x}",
        Sha256::digest(
            format!("{}|{}", profile.endpoint, profile.bearer().unwrap_or("")).as_bytes()
        )
    );
    profile.max_output_tokens = Some(2048);
    Ok((profile, identity, proxy))
}

/// `operation` comes from the trusted human-intent Host envelope, never model
/// arguments. Same operation plus a changed action/body is rejected atomically.
pub(crate) fn dispatch(
    path: &Path,
    p: i64,
    action: &str,
    args: Value,
    operation: &str,
) -> Result<Value, String> {
    let mut c = db::open(path)?;
    project(&c, p)?;
    if !args.is_object() {
        return Err("cleanup_arguments_invalid".into());
    }
    let action = action.strip_prefix("cleanup_").unwrap_or(action);
    if action == "status" {
        let read: Read = serde_json::from_value(args).map_err(|_| "cleanup_arguments_invalid")?;
        return status_on(&c, p, &read);
    }
    if operation.is_empty() || operation.len() > 240 || operation.chars().any(char::is_control) {
        return Err("cleanup_trusted_operation_required".into());
    }
    ensure(&c)?;
    let digest = hash(&args);
    if let Some((a,h,result))=c.query_row("SELECT action,args_hash,result_json FROM asset_cleanup_operations WHERE project_id=?1 AND operation_id=?2",params![p,operation],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional().map_err(err)? {
        if a!=action||h!=digest{return Err("cleanup_operation_conflict".into())}
        let mut result:Value=serde_json::from_str(&result).map_err(err)?;result["replayed"]=json!(true);return Ok(result)
    }
    let resolved = if action == "start" {
        Some(profile(&c)?)
    } else {
        None
    };
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    // Recheck after obtaining the writer lock: concurrent duplicate commands
    // must not create two snapshots, batches, or restore a different rule.
    if let Some((a,h,result))=tx.query_row("SELECT action,args_hash,result_json FROM asset_cleanup_operations WHERE project_id=?1 AND operation_id=?2",params![p,operation],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional().map_err(err)? {
        if a!=action||h!=digest{return Err("cleanup_operation_conflict".into())}
        let mut result:Value=serde_json::from_str(&result).map_err(err)?;result["replayed"]=json!(true);return Ok(result)
    }
    let id = match action {
        "start" => {
            let input: Start =
                serde_json::from_value(args.clone()).map_err(|_| "cleanup_arguments_invalid")?;
            let organization = input.organization.trim();
            if organization.chars().count() < 2 || organization.chars().count() > 240 {
                return Err("cleanup_organization_invalid".into());
            }
            let busy:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM asset_cleanup_runs WHERE project_id=?1 AND enabled=1)",[p],|r|r.get(0)).map_err(err)?;
            if busy {
                return Err("cleanup_project_already_running".into());
            }
            let (_, recipient, _) = resolved.as_ref().unwrap();
            let model = &resolved.as_ref().unwrap().0.model;
            let id = uuid::Uuid::new_v4().to_string();
            let target = review::scope::target_context(&tx, p, organization)?;
            let raw = if has(&tx, "asset_discovery_seeds")? {
                tx.query_row(
                    "SELECT count(*) FROM asset_discovery_seeds WHERE project_id=?1",
                    [p],
                    |r| r.get::<_, i64>(0),
                )
                .map_err(err)?
            } else {
                0
            };
            let ledger = if has(&tx, "asset_project_seeds")? {
                tx.query_row(
                    "SELECT count(*) FROM asset_project_seeds WHERE project_id=?1",
                    [p],
                    |r| r.get::<_, i64>(0),
                )
                .map_err(err)?
            } else {
                0
            };
            tx.execute("INSERT INTO asset_cleanup_runs(id,project_id,organization,recipient,model,target_names_json,roots_json,auto_exclude,phase,enabled,raw_count,ledger_count,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'preparing',1,?9,?10,?11,?11)",params![id,p,organization,recipient,model,json!(target.names).to_string(),json!(roots(&tx,p)?).to_string(),input.auto_exclude_unrelated,raw,ledger,now()]).map_err(err)?;
            if raw > 0 {
                tx.execute("INSERT INTO asset_cleanup_seeds(run_id,source_kind,source_id,host,source,source_ref,wildcard) SELECT ?1,'raw',id,host,source,source_ref,wildcard FROM asset_discovery_seeds WHERE project_id=?2",params![id,p]).map_err(err)?;
            }
            if ledger > 0 {
                tx.execute("INSERT INTO asset_cleanup_seeds(run_id,source_kind,source_id,host,source,source_ref,wildcard) SELECT ?1,'ledger',id,value,kind,'',wildcard FROM asset_project_seeds WHERE project_id=?2",params![id,p]).map_err(err)?;
            }
            if has(&tx, "asset_discovery_seed_evidence")? {
                tx.execute("INSERT INTO asset_cleanup_seeds(run_id,source_kind,source_id,host,source,source_ref,wildcard) SELECT ?1,'evidence',rowid,host,source,source_ref,wildcard FROM asset_discovery_seed_evidence WHERE project_id=?2",params![id,p]).map_err(err)?;
            }
            if has(&tx, "asset_project_seed_observations")? {
                tx.execute("INSERT INTO asset_cleanup_seeds(run_id,source_kind,source_id,host,source,source_ref,wildcard) SELECT ?1,'observation',o.id,s.value,o.source_type,CASE WHEN o.asset_id IS NOT NULL THEN 'asset:'||o.asset_id ELSE o.source_ref END,s.wildcard FROM asset_project_seed_observations o JOIN asset_project_seeds s ON s.id=o.seed_id AND s.project_id=o.project_id WHERE o.project_id=?2",params![id,p]).map_err(err)?;
            }
            id
        }
        "pause" | "resume" | "restore" => {
            let input: Control =
                serde_json::from_value(args).map_err(|_| "cleanup_arguments_invalid")?;
            owned(&tx, p, &input.run_id)?;
            if action != "restore" && input.batch_id.is_some() {
                return Err("cleanup_arguments_invalid".into());
            }
            if action == "pause" {
                tx.execute("UPDATE asset_cleanup_runs SET enabled=0,phase='paused',generation=generation+1,reason='已暂停，进度和排除批次保留',updated_at=?2 WHERE id=?1",params![input.run_id,now()]).map_err(err)?;
            } else if action == "resume" {
                let busy:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM asset_cleanup_runs WHERE project_id=?1 AND enabled=1 AND id!=?2)",params![p,input.run_id],|r|r.get(0)).map_err(err)?;
                if busy {
                    return Err("cleanup_project_already_running".into());
                }
                tx.execute("UPDATE asset_cleanup_runs SET enabled=1,phase=CASE WHEN EXISTS(SELECT 1 FROM asset_cleanup_seeds WHERE run_id=?1 AND group_key='') THEN 'preparing' ELSE 'researching' END,generation=generation+1,reason='已继续；未知模型请求不会重放',updated_at=?2 WHERE id=?1",params![input.run_id,now()]).map_err(err)?;
            } else {
                work::restore_on(&tx, p, &input.run_id, input.batch_id.as_deref())?;
            }
            input.run_id
        }
        _ => return Err("cleanup_action_invalid".into()),
    };
    let mut result = status_on(
        &tx,
        p,
        &Read {
            run_id: Some(id.clone()),
            page_size: Some(1),
            ..Default::default()
        },
    )?;
    result["action"] = json!(action);
    result["operationId"] = json!(operation);
    tx.execute(
        "INSERT INTO asset_cleanup_operations VALUES(?1,?2,?3,?4,?5,?6)",
        params![p, operation, action, digest, id, result.to_string()],
    )
    .map_err(err)?;
    tx.commit().map_err(err)?;
    Ok(result)
}

fn status_on(c: &Connection, p: i64, input: &Read) -> Result<Value, String> {
    if !has(c, "asset_cleanup_runs")? {
        return idle_status(c, p);
    }
    if let Some(op) = &input.operation_id {
        let receipt:Option<String>=c.query_row("SELECT result_json FROM asset_cleanup_operations WHERE project_id=?1 AND operation_id=?2",params![p,op],|r|r.get(0)).optional().map_err(err)?;
        let result = receipt
            .as_ref()
            .and_then(|s| serde_json::from_str::<Value>(s).ok());
        let receipt_run = result
            .as_ref()
            .and_then(|v| v["runId"].as_str())
            .map(str::to_owned);
        if input
            .run_id
            .as_ref()
            .zip(receipt_run.as_ref())
            .is_some_and(|(requested, saved)| requested != saved)
        {
            return Err("cleanup_operation_run_mismatch".into());
        }
        let mut current = status_on(
            c,
            p,
            &Read {
                run_id: receipt_run.or_else(|| input.run_id.clone()),
                page: input.page,
                page_size: Some(1),
                filter: input.filter.clone(),
                operation_id: None,
            },
        )?;
        let projection=result.as_ref().map(|r|json!({"projectId":r["projectId"],"runId":r["runId"],"action":r["action"],"operationId":r["operationId"],"receiptProjection":true}));
        current["operation"] = json!({"id":op,"state":if receipt.is_some(){"completed"}else{"unknown"},"result":projection});
        current["operationFound"] = json!(receipt.is_some());
        return Ok(current);
    }
    let Some(id) = run_id(c, p, input.run_id.as_deref())? else {
        return idle_status(c, p);
    };
    let (organization,phase,enabled,reason,raw,ledger,cursor,model): (String,String,bool,String,i64,i64,i64,String)=c.query_row("SELECT organization,phase,enabled,reason,raw_count,ledger_count,cursor,model FROM asset_cleanup_runs WHERE id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).map_err(err)?;
    let mut counts = BTreeMap::<String, i64>::new();
    let mut q = c
        .prepare("SELECT state,count(*) FROM asset_cleanup_groups WHERE run_id=?1 GROUP BY state")
        .map_err(err)?;
    for row in q
        .query_map([&id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
        .map_err(err)?
    {
        let (k, n) = row.map_err(err)?;
        counts.insert(k, n);
    }
    let groups: i64 = counts.values().sum();
    let page = input.page.unwrap_or(1).max(1);
    let size = input.page_size.unwrap_or(5).clamp(1, 100);
    let filter = input.filter.as_deref().unwrap_or("all");
    let filter = if filter == "needs_evidence" {
        "evidence"
    } else {
        filter
    };
    if ![
        "all", "pending", "retained", "excluded", "evidence", "conflict", "restored", "held",
    ]
    .contains(&filter)
    {
        return Err("cleanup_filter_invalid".into());
    }
    let total: i64 = c
        .query_row(
            "SELECT count(*) FROM asset_cleanup_groups WHERE run_id=?1 AND (?2='all' OR state=?2)",
            params![id, filter],
            |r| r.get(0),
        )
        .map_err(err)?;
    let mut q=c.prepare("SELECT group_key,domain,seed_count,state,advice,reason,evidence_json,revision FROM asset_cleanup_groups WHERE run_id=?1 AND (?2='all' OR state=?2) ORDER BY CASE state WHEN 'conflict' THEN 0 WHEN 'excluded' THEN 1 WHEN 'evidence' THEN 2 ELSE 3 END,domain LIMIT ?3 OFFSET ?4").map_err(err)?;
    let items=q.query_map(params![id,filter,size,(page-1).saturating_mul(size)],|r|Ok(json!({"key":r.get::<_,String>(0)?,"domain":r.get::<_,String>(1)?,"seedCount":r.get::<_,i64>(2)?,"state":r.get::<_,String>(3)?,"status":r.get::<_,String>(3)?,"advice":r.get::<_,String>(4)?,"reason":r.get::<_,String>(5)?,"evidence":display_evidence(serde_json::from_str::<Value>(&r.get::<_,String>(6)?).unwrap_or(Value::Null)),"revision":r.get::<_,i64>(7)?}))).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)?;
    let mut q=c.prepare("SELECT id,domains_json,restored,created_at FROM asset_cleanup_batches WHERE run_id=?1 ORDER BY created_at DESC,rowid DESC LIMIT 50").map_err(err)?;
    let batches=q.query_map([&id],|r|Ok(json!({"batchId":r.get::<_,String>(0)?,"id":r.get::<_,String>(0)?,"domains":serde_json::from_str::<Value>(&r.get::<_,String>(1)?).unwrap_or(Value::Null),"restored":r.get::<_,bool>(2)?,"createdAt":r.get::<_,i64>(3)?}))).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)?;
    let (requests,unknown,input_tokens,output_tokens):(i64,i64,i64,i64)=c.query_row("SELECT count(*),COALESCE(sum(state='unknown'),0),COALESCE(sum(json_extract(usage_json,'$.inputTokens')),0),COALESCE(sum(json_extract(usage_json,'$.outputTokens')),0) FROM asset_cleanup_models WHERE run_id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(err)?;
    let errors:Vec<Value>=c.prepare("SELECT kind,target,error,attempts,next_at FROM asset_cleanup_research WHERE run_id=?1 AND error!='' ORDER BY id DESC LIMIT 5").map_err(err)?.query_map([&id],|r|Ok(json!({"kind":r.get::<_,String>(0)?,"target":r.get::<_,String>(1)?,"error":r.get::<_,String>(2)?,"attempts":r.get::<_,i64>(3)?,"nextAt":r.get::<_,i64>(4)?}))).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;
    let research: Value = {
        let (done,retry):(i64,i64)=c.query_row("SELECT COALESCE(sum(state='done'),0),COALESCE(sum(state IN ('retry','pending','running')),0) FROM asset_cleanup_research WHERE run_id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(err)?;
        json!({"completed":done,"pending":retry,"recentErrors":errors})
    };
    counts.insert(
        "needs_evidence".into(),
        *counts.get("evidence").unwrap_or(&0),
    );
    let snapshot_total: i64 = c
        .query_row(
            "SELECT count(*) FROM asset_cleanup_seeds WHERE run_id=?1",
            [&id],
            |r| r.get(0),
        )
        .map_err(err)?;
    Ok(
        json!({"projectId":p,"runId":id,"organization":organization,"phase":phase,"enabled":enabled,"reason":reason,"snapshot":{"rawIndexSeeds":raw,"projectLedgerSeeds":ledger,"groups":groups,"processed":cursor,"total":snapshot_total,"provenanceRows":snapshot_total-raw-ledger},"counts":counts,"items":items,"batches":batches,"page":page,"pageSize":size,"total":total,"hasMore":page.saturating_mul(size)<total,"model":{"name":model,"requests":requests,"unknownRequests":unknown,"inputTokens":input_tokens,"outputTokens":output_tokens},"research":research,"hunterRequests":0,"assetsDeleted":0,"scopeChanged":false,"note":"相关性清洗不授予资产归属或探测权限；原始种子、资产与历史保留。缺证自动补查，真正冲突集中审核。"}),
    )
}
fn display_evidence(mut value: Value) -> Value {
    let mut truncated = false;
    for (field, limit) in [("domainEvidence", 5), ("names", 10), ("publicSources", 3)] {
        if let Some(rows) = value[field].as_array_mut() {
            let count = rows.len();
            if count > limit {
                rows.truncate(limit);
                truncated = true;
            }
            if field == "domainEvidence" {
                value["fullEvidenceCount"] = json!(count);
            }
        }
    }
    if let Some(sources) = value["publicSources"].as_array().cloned() {
        value["publicSources"]=json!(sources.into_iter().map(|source|json!({"kind":source["kind"],"sourceUrl":source["sourceUrl"],"bodySha256":source["bodySha256"],"checkedAt":source["checkedAt"],"current":source["current"],"certificateId":source["certificateId"],"indexedCertificateCount":source["certificateIds"].as_array().map(Vec::len),"certificateNameCount":source["names"].as_array().map(Vec::len),"dnsStatus":source["dnsStatus"],"recordCount":source["records"].as_array().map(Vec::len),"presentationSummary":true})).collect::<Vec<_>>());
        truncated |= !value["publicSources"].as_array().is_none_or(Vec::is_empty);
    }
    value["presentationTruncated"] = json!(truncated);
    value
}

fn idle_status(c: &Connection, p: i64) -> Result<Value, String> {
    let raw = if has(c, "asset_discovery_seeds")? {
        c.query_row(
            "SELECT count(*) FROM asset_discovery_seeds WHERE project_id=?1",
            [p],
            |r| r.get::<_, i64>(0),
        )
        .map_err(err)?
    } else {
        0
    };
    let ledger = if has(c, "asset_project_seeds")? {
        c.query_row(
            "SELECT count(*) FROM asset_project_seeds WHERE project_id=?1",
            [p],
            |r| r.get::<_, i64>(0),
        )
        .map_err(err)?
    } else {
        0
    };
    let mut organization = String::new();
    for table in ["asset_seed_assists", "asset_seed_reviews"] {
        if has(c, table)? {
            if let Some(v)=c.query_row(&format!("SELECT organization FROM {table} WHERE project_id=?1 ORDER BY rowid DESC LIMIT 1"),[p],|r|r.get::<_,String>(0)).optional().map_err(err)?{organization=v;break;}
        }
    }
    Ok(
        json!({"projectId":p,"runId":Value::Null,"organization":organization,"organizationHint":organization,"phase":"idle","enabled":false,"snapshot":{"rawIndexSeeds":raw,"projectLedgerSeeds":ledger,"groups":0,"processed":0,"total":raw+ledger},"counts":{},"items":[],"batches":[],"hunterRequests":0,"assetsDeleted":0}),
    )
}

/// Called by the application-owned service. One small local page or one
/// external request per tick; no database write transaction spans the network.
pub(crate) fn tick(path: &Path) -> Result<bool, String> {
    work::tick(path)
}
/// Stop only claims owned by this process. Other app instances retain theirs.
pub(crate) fn interrupt(path: &Path) -> Result<(), String> {
    work::interrupt(path)
}

#[cfg(test)]
#[path = "discovery_cleanup_tests.rs"]
mod tests;
