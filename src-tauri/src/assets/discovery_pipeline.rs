//! Coordinates existing collection, review, free-enrichment and exact Hunter
//! owners. This journal grants neither ownership nor additional network scope.
use super::*;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS asset_discovery_pipelines(project_id INTEGER PRIMARY KEY REFERENCES projects(id),operation_id TEXT NOT NULL,organization TEXT NOT NULL,include_isolated INTEGER NOT NULL,allow_hunter INTEGER NOT NULL DEFAULT 0,collect_first INTEGER NOT NULL DEFAULT 0,targets_json TEXT NOT NULL DEFAULT '[]',collection_run_id INTEGER,enrichment_run_id TEXT NOT NULL DEFAULT '',phase TEXT NOT NULL DEFAULT 'local_analysis',round INTEGER NOT NULL DEFAULT 1,baseline_assets INTEGER NOT NULL DEFAULT 0,enabled INTEGER NOT NULL DEFAULT 1,reason TEXT NOT NULL DEFAULT '',last_signature TEXT NOT NULL DEFAULT '',updated_at INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS asset_discovery_pipeline_queries(operation_id TEXT NOT NULL,project_id INTEGER NOT NULL,kind TEXT NOT NULL,target TEXT NOT NULL,query_id TEXT NOT NULL,state TEXT NOT NULL DEFAULT 'prepared',baseline_spent INTEGER NOT NULL DEFAULT 0,baseline_pages INTEGER NOT NULL DEFAULT 0,baseline_unique INTEGER NOT NULL DEFAULT 0,reason TEXT NOT NULL DEFAULT '',PRIMARY KEY(operation_id,kind,target));
CREATE TABLE IF NOT EXISTS asset_discovery_pipeline_facts(operation_id TEXT NOT NULL,kind TEXT NOT NULL,payload_json TEXT NOT NULL,PRIMARY KEY(operation_id,kind));";

pub(super) fn ensure(c: &Connection) -> Result<(), String> {
    c.execute_batch(SCHEMA).map_err(err)?;
    let columns = c
        .prepare("PRAGMA table_info(asset_discovery_pipeline_queries)")
        .map_err(err)?
        .query_map([], |r| r.get::<_, String>(1))
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    if !columns.iter().any(|name| name == "round_number") {
        c.execute(
            "ALTER TABLE asset_discovery_pipeline_queries ADD COLUMN round_number INTEGER",
            [],
        )
        .map_err(err)?;
    }
    crate::asset_discovery::rounds::ensure(c)
}
fn track(c: &Connection, p: i64) -> Result<(), String> {
    let enabled: bool = c
        .query_row(
            "SELECT enabled FROM asset_discovery_pipelines WHERE project_id=?1",
            [p],
            |r| r.get(0),
        )
        .optional()
        .map_err(err)?
        .unwrap_or(false);
    if !enabled {
        return Ok(());
    }
    let round = crate::asset_discovery::rounds::start(c, p)?;
    let operation: String = c
        .query_row(
            "SELECT operation_id FROM asset_discovery_pipelines WHERE project_id=?1",
            [p],
            |r| r.get(0),
        )
        .map_err(err)?;
    crate::asset_discovery::seed_ledger::initialize(c, p)?;
    crate::asset_discovery::seed_ledger::sync(c, p, &operation, round)?;
    Ok(())
}
fn has(c: &Connection, table: &str) -> Result<bool, String> {
    c.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name=?1)",
        [table],
        |r| r.get(0),
    )
    .map_err(err)
}
fn count(c: &Connection, p: i64) -> Result<i64, String> {
    c.query_row(
        "SELECT COUNT(*) FROM project_assets WHERE project_id=?1",
        [p],
        |r| r.get(0),
    )
    .map_err(err)
}
pub(super) fn start_on(c: &Connection, input: &Input, org: &str) -> Result<(), String> {
    ensure(c)?;
    let targets = collection_targets(input, org)?;
    let old:Option<(String,bool,String,String)>=c.query_row("SELECT organization,include_isolated,phase,targets_json FROM asset_discovery_pipelines WHERE project_id=?1",[input.project_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(err)?;
    let same = old.as_ref().is_some_and(|(o, include, stage, saved)| {
        o == org
            && *include == input.include_isolated
            && (!input.collect_first
                || (stage == "collecting" && *saved == json!(targets).to_string()))
    });
    if same {
        // Continuing never turns on paid discovery merely because the UI omitted
        // an option. Unknown requests remain held in their original owner.
        c.execute("UPDATE asset_discovery_pipelines SET enabled=1,allow_hunter=COALESCE(?2,allow_hunter),last_signature=CASE WHEN ?2 IS NOT NULL AND ?2!=allow_hunter THEN '' ELSE last_signature END,phase=CASE WHEN phase='paused' THEN CASE WHEN collection_run_id IS NOT NULL AND NOT EXISTS(SELECT 1 FROM runs WHERE id=collection_run_id AND status='completed') THEN 'collecting' ELSE 'local_analysis' END WHEN phase='steady' AND ?2 IS NOT NULL AND ?2!=allow_hunter THEN 'local_analysis' ELSE phase END,updated_at=?3 WHERE project_id=?1",params![input.project_id,input.allow_hunter,chrono::Utc::now().timestamp()]).map_err(err)?;
        track(c, input.project_id)?;
        return Ok(());
    }
    c.execute("INSERT INTO asset_discovery_pipelines(project_id,operation_id,organization,include_isolated,allow_hunter,collect_first,targets_json,phase,baseline_assets,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(project_id) DO UPDATE SET operation_id=excluded.operation_id,organization=excluded.organization,include_isolated=excluded.include_isolated,allow_hunter=excluded.allow_hunter,collect_first=excluded.collect_first,targets_json=excluded.targets_json,collection_run_id=NULL,enrichment_run_id='',phase=excluded.phase,round=1,baseline_assets=excluded.baseline_assets,enabled=1,reason='',last_signature='',updated_at=excluded.updated_at",params![input.project_id,uuid::Uuid::new_v4().to_string(),org,input.include_isolated,input.allow_hunter.unwrap_or(false),input.collect_first,json!(targets).to_string(),if input.collect_first{"collecting"}else{"local_analysis"},count(c,input.project_id)?,chrono::Utc::now().timestamp()]).map_err(err)?;
    track(c, input.project_id)?;
    Ok(())
}
fn collection_targets(input: &Input, org: &str) -> Result<Vec<String>, String> {
    let mut targets = input
        .collection_targets
        .iter()
        .map(|s| s.trim().to_owned())
        .collect::<Vec<_>>();
    if input.collect_first && targets.is_empty() {
        let mut anchor = org.trim();
        for suffix in [
            "及其下属单位",
            "及下属单位",
            "及其子公司",
            "及子公司",
            "及其下属公司",
            "及下属公司",
        ] {
            if let Some(value) = anchor.strip_suffix(suffix) {
                anchor = value.trim();
                break;
            }
        }
        if anchor == "中国移动集团" {
            anchor = "中国移动";
        }
        targets.push(anchor.into());
    }
    if targets.len() > 100
        || targets
            .iter()
            .any(|s| s.is_empty() || s.chars().count() > 240 || s.chars().any(char::is_control))
    {
        return Err("首采种子最多100条，每条1–240字".into());
    }
    targets.sort_by_key(|s| s.to_lowercase());
    targets.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    Ok(targets)
}
pub(super) fn phase(c: &Connection, p: i64, next: &str, reason: &str) -> Result<(), String> {
    c.execute("UPDATE asset_discovery_pipelines SET phase=?2,reason=?3,updated_at=?4 WHERE project_id=?1 AND enabled=1",params![p,next,reason,chrono::Utc::now().timestamp()]).map_err(err)?;
    track(c, p)
}
pub(super) fn enrichment(c: &Connection, p: i64, result: &Value) -> Result<(), String> {
    if let Some(id) = result["run"]["id"].as_str() {
        // A manually started queue can be observed, but this coordinator must
        // not claim it merely because start() returned the active project run.
        c.execute("UPDATE asset_discovery_pipelines SET enrichment_run_id=?2 WHERE project_id=?1 AND enabled=1 AND EXISTS(SELECT 1 FROM asset_enrichment_runs e WHERE e.id=?2 AND e.project_id=?1 AND (e.operation_id='assist-'||asset_discovery_pipelines.operation_id OR e.id=asset_discovery_pipelines.enrichment_run_id))",params![p,id]).map_err(err)?;
    }
    Ok(())
}
pub(super) fn confirmed_enrichment(
    c: &Connection,
    p: i64,
    run: &str,
    token: &str,
) -> Result<(), String> {
    if has(c, "asset_enrichment_runs")? {
        c.execute("UPDATE asset_discovery_pipelines SET enrichment_run_id=COALESCE((SELECT id FROM asset_enrichment_runs WHERE project_id=?1 AND operation_id=?2),enrichment_run_id) WHERE project_id=?1 AND enabled=1 AND EXISTS(SELECT 1 FROM asset_seed_assists a WHERE a.project_id=?1 AND a.run_id=?3 AND a.enabled=1 AND a.enrichment_pending=?4 AND a.pending_roots_json=?5)",params![p,format!("scope-{token}"),run,token,json!(roots(c,p)?).to_string()]).map_err(err)?;
        c.execute("UPDATE asset_seed_assists SET owned_enrichment_run_id=COALESCE((SELECT id FROM asset_enrichment_runs WHERE project_id=?1 AND operation_id=?2),owned_enrichment_run_id) WHERE project_id=?1 AND run_id=?3 AND enabled=1 AND enrichment_pending=?4 AND pending_roots_json=?5",params![p,format!("scope-{token}"),run,token,json!(roots(c,p)?).to_string()]).map_err(err)?;
    }
    Ok(())
}
pub(super) fn status(c: &Connection, p: i64) -> Result<Value, String> {
    if !has(c, "asset_discovery_pipelines")? {
        return Ok(Value::Null);
    }
    let row:Option<(String,String,i64,Option<i64>,String,bool,String,i64)>=c.query_row("SELECT operation_id,phase,round,collection_run_id,enrichment_run_id,allow_hunter,reason,baseline_assets FROM asset_discovery_pipelines WHERE project_id=?1",[p],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).optional().map_err(err)?;
    let Some((op, stage, round, collection, free, allowed, reason, baseline)) = row else {
        return Ok(Value::Null);
    };
    let collection_pending:bool=c.query_row("SELECT collect_first AND (collection_run_id IS NULL OR NOT EXISTS(SELECT 1 FROM runs r WHERE r.id=collection_run_id AND r.project_id=?1 AND r.status='completed')) FROM asset_discovery_pipelines WHERE project_id=?1",[p],|r|r.get(0)).map_err(err)?;
    let current = count(c, p)?;
    let mut hunter = json!({"queries":0,"pages":0,"spentPoints":0,"costUnknown":false,"added":0});
    if has(c, "hunter_expansions")? {
        let (queries,pages,spent,unknown,added):(i64,i64,i64,bool,i64)=c.query_row("SELECT COUNT(*),COALESCE(SUM(MAX(0,(SELECT COUNT(*) FROM hunter_expansion_pages pg WHERE pg.query_id=q.query_id)-q.baseline_pages)),0),COALESCE(SUM(MAX(0,h.spent-q.baseline_spent)),0),COALESCE(MAX(h.cost_unknown OR EXISTS(SELECT 1 FROM hunter_expansion_pages pg WHERE pg.query_id=q.query_id AND (pg.state='sending' OR pg.accounted=0))),0),COALESCE(SUM(MAX(0,h.unique_rows-q.baseline_unique)),0) FROM asset_discovery_pipeline_queries q JOIN hunter_expansions h ON h.id=q.query_id AND h.project_id=q.project_id WHERE q.operation_id=?1 AND q.project_id=?2",params![op,p],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(err)?;
        hunter = json!({"queries":queries,"pages":pages,"spentPoints":spent,"costUnknown":unknown,"added":added});
        let active:Option<(String,String)>=c.query_row("SELECT query_id,reason FROM asset_discovery_pipeline_queries WHERE operation_id=?1 AND project_id=?2 AND state!='completed' ORDER BY rowid LIMIT 1",params![op,p],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(err)?;
        if let Some((id, reason)) = active {
            hunter["activeQueryId"] = json!(id);
            hunter["reason"] = json!(reason)
        }
    }
    let completed = if !free.is_empty() && has(c, "asset_enrichment_items")? {
        c.query_row(
            "SELECT COUNT(*) FROM asset_enrichment_items WHERE run_id=?1 AND status='done'",
            [&free],
            |r| r.get::<_, i64>(0),
        )
        .map_err(err)?
    } else {
        0
    };
    let fact:Option<String>=c.query_row("SELECT payload_json FROM asset_discovery_pipeline_facts WHERE operation_id=?1 AND kind='organization'",[&op],|r|r.get(0)).optional().map_err(err)?;
    let mut value = json!({"phase":stage,"round":round,"collectionRunId":collection,"collectionPending":collection_pending,"enrichmentRunId":if free.is_empty(){Value::Null}else{json!(free)},"allowHunter":allowed,"counts":{"collectedAssets":current,"uniqueAssets":current,"roundAdded":(current-baseline).max(0),"freeCompleted":completed,"hunterAdded":hunter["added"]},"hunter":hunter,"reason":reason,"organizationEvidence":fact.and_then(|raw|serde_json::from_str::<Value>(&raw).ok())});
    let proposal: Option<String> = c
        .query_row(
            "SELECT proposal_json FROM asset_seed_assists WHERE project_id=?1",
            [p],
            |r| r.get(0),
        )
        .optional()
        .map_err(err)?;
    if let Some(v) = proposal
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .filter(|v| v.is_object())
    {
        value["counts"]["verifiedGroups"] =
            json!(v["groups"].as_array().map(|g| g.len()).unwrap_or(0));
        value["counts"]["pendingEvidenceGroups"] = json!(
            v["heldForVerification"].as_i64().unwrap_or(0)
                + v["removedUnverified"].as_i64().unwrap_or(0)
        );
        value["counts"]["conflicts"] = json!(v["verificationConflicts"].as_i64().unwrap_or(0));
    }
    if let Some(run) = c
        .query_row(
            "SELECT run_id FROM asset_seed_assists WHERE project_id=?1",
            [p],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(err)?
    {
        if !run.is_empty() {
            value["passiveEvidence"] =
                crate::asset_discovery::passive_evidence::summary(c, p, &run)?;
        }
    }
    value["roundHistory"] = crate::asset_discovery::rounds::history(c, p, 1, 5)?;
    Ok(value)
}

/// A root-domain query covers its observed children; a fingerprint is queried
/// once. Every candidate still passes the same per-page paid-source gate.
fn candidates(
    c: &Connection,
    p: i64,
    op: &str,
) -> Result<Vec<hunter_expansion::ExpansionInput>, String> {
    let mut out = vec![];
    let confirmed = roots(c, p)?;
    for root in &confirmed {
        if c.query_row("SELECT EXISTS(SELECT 1 FROM asset_discovery_pipeline_queries WHERE operation_id=?1 AND kind='domain_tree' AND target=?2 AND state='completed')",params![op,root],|r|r.get::<_,bool>(0)).map_err(err)?{continue}
        let i = hunter_expansion::ExpansionInput {
            project_id: p,
            kind: "domain_tree".into(),
            target: root.clone(),
        };
        if hunter_expansion::evidence_anchor(c, p, root, "domain_tree").is_ok() {
            out.push(i)
        }
    }
    let mut q=c.prepare("SELECT DISTINCT host,source FROM asset_discovery_seeds s WHERE project_id=?1 AND wildcard=0 AND source IN ('certificate_sha256','cname','dns_cname') AND NOT EXISTS(SELECT 1 FROM asset_discovery_pipeline_queries q WHERE q.operation_id=?2 AND q.target=s.host AND q.kind=CASE s.source WHEN 'certificate_sha256' THEN 'fingerprint' ELSE 'cname' END AND q.state='completed') ORDER BY CASE source WHEN 'certificate_sha256' THEN 0 ELSE 1 END,host").map_err(err)?;
    for row in q
        .query_map(params![p, op], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(err)?
    {
        let (target, source) = row.map_err(err)?;
        let kind = if source == "certificate_sha256" {
            "fingerprint"
        } else {
            "cname"
        };
        if out.iter().any(|i| i.kind == kind && i.target == target) {
            continue;
        }
        if hunter_expansion::evidence_anchor(c, p, &target, kind).is_ok() {
            out.push(hunter_expansion::ExpansionInput {
                project_id: p,
                kind: kind.into(),
                target,
            })
        }
    }
    for target in crate::asset_discovery::passive_evidence::fingerprint_candidates(c, p)? {
        if c.query_row("SELECT EXISTS(SELECT 1 FROM asset_discovery_pipeline_queries WHERE operation_id=?1 AND kind='fingerprint' AND target=?2 AND state='completed')",params![op,target],|r|r.get::<_,bool>(0)).map_err(err)?{continue}
        if !out
            .iter()
            .any(|i| i.kind == "fingerprint" && i.target == target)
            && hunter_expansion::evidence_anchor(c, p, &target, "fingerprint").is_ok()
        {
            out.push(hunter_expansion::ExpansionInput {
                project_id: p,
                kind: "fingerprint".into(),
                target,
            })
        }
    }
    Ok(out)
}

impl Manager {
    pub(super) fn start_collection(
        &self,
        c: &Connection,
        p: i64,
        explicit_resume: bool,
    ) -> Result<bool, String> {
        let row:Option<(String,Option<i64>,String)>=c.query_row("SELECT operation_id,collection_run_id,targets_json FROM asset_discovery_pipelines WHERE project_id=?1 AND collect_first=1 AND enabled=1",[p],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(err)?;
        let Some((op, saved, raw)) = row else {
            return Ok(false);
        };
        let targets: Vec<String> = serde_json::from_str(&raw).map_err(err)?;
        let id = if let Some(id) = saved {
            let jobs = self
                .jobs
                .as_ref()
                .ok_or("当前环境无法核对原 FOFA 任务的持久归属；原任务保留，未采用其他任务")?;
            crate::jobs::validate_owned_discovery_collection(jobs, p, &op, &targets, id)?;
            id
        } else {
            let jobs = self
                .jobs
                .as_ref()
                .ok_or("当前运行环境未接通 FOFA 采集任务；未发起采集")?;
            let id = crate::jobs::start_job_for_discovery_in(jobs, p, &op, &targets)?;
            c.execute("UPDATE asset_discovery_pipelines SET collection_run_id=?2 WHERE project_id=?1 AND operation_id=?3",params![p,id,op]).map_err(err)?;
            id
        };
        let (state,message):(String,String)=c.query_row("SELECT status,error FROM runs WHERE id=?1 AND project_id=?2 AND pipeline='collect'",params![id,p],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|"首采任务不属于当前项目或类型已变化")?;
        if state == "completed" {
            return Ok(false);
        }
        let blocked = matches!(state.as_str(), "failed" | "cancelled" | "interrupted");
        if blocked && explicit_resume {
            let jobs = self
                .jobs
                .as_ref()
                .ok_or("当前环境无法恢复原 FOFA 任务；原进度保留")?;
            crate::jobs::resume_job_in(jobs, id)?;
            phase(
                c,
                p,
                "collecting",
                "按本次明确继续指令恢复原 FOFA 任务；原查询回执与断点保留",
            )?;
            return Ok(true);
        }
        phase(
            c,
            p,
            if blocked { "blocked" } else { "collecting" },
            &if blocked {
                format!("FOFA 首采已{state}：{message}；请在任务中心恢复原任务，不会自动重复采集")
            } else {
                "正在使用 FOFA 采集，完成后自动整理项目证据".into()
            },
        )?;
        Ok(true)
    }
    pub(super) fn continue_pipelines(&self, c: &mut Connection) -> Result<(), String> {
        let rows:Vec<(i64,String,String,bool,String,i64)>=c.prepare("SELECT a.project_id,a.run_id,p.organization,p.include_isolated,p.phase,a.owner_pid FROM asset_seed_assists a JOIN asset_discovery_pipelines p ON p.project_id=a.project_id WHERE a.enabled=1 AND p.enabled=1").map_err(err)?.query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;
        for (p, id, org, include, _stage, pid) in rows {
            if pid > 0
                && pid != std::process::id() as i64
                && unsafe { libc::kill(pid as i32, 0) } == 0
            {
                continue;
            }
            let result = (|| -> Result<(), String> {
                if active(&id) {
                    return Ok(());
                }
                if self.start_collection(c, p, false)? {
                    return Ok(());
                }
                if id.is_empty() {
                    let input = Input {
                        project_id: p,
                        action: "start".into(),
                        organization: Some(org),
                        include_isolated: include,
                        collect_first: false,
                        collection_targets: vec![],
                        allow_hunter: None,
                    };
                    let view = self.start_local(&input)?;
                    if view["hasLocalAssets"] == false {
                        phase(
                            c,
                            p,
                            "steady",
                            "首采未返回本地资产；原采集任务与错误明细保留，未空跑 AI 或 Hunter",
                        )?;
                    } else {
                        phase(
                            c,
                            p,
                            "local_analysis",
                            "FOFA 已完成；正在按证据分析本项目资产",
                        )?;
                    }
                    return Ok(());
                }
                let pending:bool=c.query_row("SELECT cursor<max_id OR status IN ('preparing','analyzing','running') FROM asset_seed_reviews WHERE id=?1",[&id],|r|r.get(0)).map_err(err)?;
                if pending {
                    phase(
                        c,
                        p,
                        "local_analysis",
                        "正在整理和分析项目证据；已完成模型请求不会重复发送",
                    )?;
                    return Ok(());
                }
                // Paid ambiguity is handled after free scoped work in tick.
                // It cannot freeze public evidence or force a paid replay.
                self.launch_pipeline(p, &id)?;
                Ok(())
            })();
            if let Err(e) = result {
                phase(c, p, "blocked", &e)?;
            }
        }
        Ok(())
    }
    fn launch_pipeline(&self, p: i64, id: &str) -> Result<(), String> {
        let mut guard = controls().lock().map_err(err)?;
        if guard.contains_key(id) {
            return Ok(());
        }
        let cancel = CancelToken::new();
        guard.insert(id.into(), cancel.clone());
        drop(guard);
        let manager = self.clone();
        let run = id.to_string();
        let c = db::open(&self.path)?;
        c.execute("UPDATE asset_seed_assists SET owner_pid=?3 WHERE project_id=?1 AND run_id=?2 AND enabled=1",params![p,id,std::process::id()]).map_err(err)?;
        spawn_owned(
            self.path.clone(),
            id.into(),
            "discovery-pipeline",
            move || {
                let mut c = db::open(&manager.path)?;
                let result = manager.pipeline_tick(&mut c, p, &run, &cancel);
                if let Err(e) = &result {
                    phase(&c, p, "blocked", e)?;
                }
                c.execute(
                    "UPDATE asset_seed_assists SET owner_pid=0 WHERE project_id=?1 AND run_id=?2",
                    params![p, run],
                )
                .map_err(err)?;
                // Execution failures belong to this stage; they must not rewrite
                // successful model receipts as an unknown model request.
                Ok(())
            },
        )
    }
    fn pipeline_tick(
        &self,
        c: &mut Connection,
        p: i64,
        id: &str,
        cancel: &CancelToken,
    ) -> Result<(), String> {
        if cancel.is_cancelled() || !enabled(c, p, id)? {
            return Ok(());
        }
        let(op,allowed):(String,bool)=c.query_row("SELECT operation_id,allow_hunter FROM asset_discovery_pipelines WHERE project_id=?1 AND enabled=1",[p],|r|Ok((r.get(0)?,r.get(1)?))).map_err(err)?;
        let organization: String = c
            .query_row(
                "SELECT organization FROM asset_discovery_pipelines WHERE project_id=?1",
                [p],
                |r| r.get(0),
            )
            .map_err(err)?;
        let settings = crate::model_configuration::default_settings_for(c, &["network"]);
        if settings.get("_configurationError").is_none() {
            // A fixed official source can establish controlled legal aliases;
            // its failure cannot authorize anything or stop free scoped work.
            match crate::asset_discovery::organization_evidence::refresh(
                &self.path,
                p,
                &organization,
                settings["proxyUrl"].as_str().unwrap_or(""),
            ) {
                Ok(value) => {
                    let previous:Option<String>=c.query_row("SELECT payload_json FROM asset_discovery_pipeline_facts WHERE operation_id=?1 AND kind='organization'",[&op],|r|r.get(0)).optional().map_err(err)?;
                    let names_changed = previous
                        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
                        .map(|old| old["names"] != value["names"])
                        .unwrap_or(true);
                    c.execute("INSERT INTO asset_discovery_pipeline_facts VALUES(?1,'organization',?2) ON CONFLICT(operation_id,kind) DO UPDATE SET payload_json=excluded.payload_json",params![op,value.to_string()]).map_err(err)?;
                    if names_changed {
                        triage::run(c, p, id)?;
                        c.execute("UPDATE asset_discovery_pipelines SET last_signature='' WHERE project_id=?1",[p]).map_err(err)?;
                    }
                }
                Err(e) => {
                    c.execute("INSERT INTO asset_discovery_pipeline_facts VALUES(?1,'organization',?2) ON CONFLICT(operation_id,kind) DO UPDATE SET payload_json=excluded.payload_json",params![op,json!({"error":e,"networkVerified":false,"scopeAuthorized":false}).to_string()]).map_err(err)?;
                }
            }
        }
        crate::asset_discovery::passive_evidence::enqueue_review_candidates(c, p, id)?;
        let passive = crate::asset_discovery::passive_evidence::summary(c, p, id)?;
        let passive_pending = passive["pending"].as_i64().unwrap_or(0) > 0
            || passive["running"].as_i64().unwrap_or(0) > 0
            || passive["retry"].as_i64().unwrap_or(0) > 0;
        if passive_pending {
            crate::asset_discovery::rounds::new_evidence(
                c,
                p,
                "自动补证发现新证据，接续本轮增量分析",
            )?;
            phase(
                c,
                p,
                "passive_evidence",
                "正在从公开 CT 记录核实候选证书；不连接未确认主机、不消耗 Hunter 积分",
            )?;
            let receipt =
                crate::asset_discovery::passive_evidence::tick(&self.path, p, id, || {
                    cancel.is_cancelled()
                })?;
            if receipt["changed"] == true {
                triage::run(c, p, id)?;
                refresh_proposal(c, p, id)?;
            }
        }
        let revisions:(i64,i64)=c.query_row("SELECT COUNT(*),COALESCE(SUM(revision),0) FROM asset_seed_review_items WHERE run_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(err)?;
        let seed_version:(i64,i64)=c.query_row("SELECT COUNT(*),COALESCE(MAX(id),0) FROM asset_discovery_seeds WHERE project_id=?1",[p],|r|Ok((r.get(0)?,r.get(1)?))).map_err(err)?;
        let signature = json!([
            id,
            revisions,
            seed_version,
            roots(c, p)?,
            exclusions::rules(c, p)?,
            allowed
        ])
        .to_string();
        let (old_signature, old_phase): (String, String) = c
            .query_row(
                "SELECT last_signature,phase FROM asset_discovery_pipelines WHERE project_id=?1",
                [p],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(err)?;
        if signature != old_signature {
            refresh_proposal(c, p, id)?;
            c.execute(
                "UPDATE asset_discovery_pipelines SET last_signature=?2 WHERE project_id=?1",
                params![p, signature],
            )
            .map_err(err)?;
        }
        let unchanged_steady=signature==old_signature&&old_phase=="steady"&&!passive_pending;
        if cancel.is_cancelled() || !enabled(c, p, id)? {
            return Ok(());
        }
        let scope = roots(c, p)?;
        if scope.is_empty() {
            phase(
                c,
                p,
                "awaiting_scope",
                "证据已整理；请一次确认有直接依据的新增范围，AI 不会把建议当作执行授权",
            )?;
            return Ok(());
        }
        let saved_free: String = c
            .query_row(
                "SELECT enrichment_run_id FROM asset_discovery_pipelines WHERE project_id=?1",
                [p],
                |r| r.get(0),
            )
            .map_err(err)?;
        let mut free = self.enrichment.dispatch(
            p,
            "status",
            if saved_free.is_empty() {
                json!({})
            } else {
                json!({"taskId":saved_free})
            },
            "",
        )?;
        let matches_scope = free["run"]["id"]
            .as_str()
            .map(|task| {
                c.query_row(
                    "SELECT roots_json FROM asset_enrichment_runs WHERE id=?1 AND project_id=?2",
                    params![task, p],
                    |r| r.get::<_, String>(0),
                )
                .map(|raw| raw == json!(scope).to_string())
                .unwrap_or(false)
            })
            .unwrap_or(false);
        if !matches_scope || matches!(free["run"]["status"].as_str(), Some("scope_changed")) {
            self.start_enrichment(c, p)?;
            free = self.enrichment.dispatch(p, "status", json!({}), "")?;
        }
        if let Some(task)=free["run"]["id"].as_str().map(str::to_owned){
            let owned=task==saved_free||c.query_row("SELECT EXISTS(SELECT 1 FROM asset_seed_assist_free_owners WHERE project_id=?1 AND task_id=?2)",params![p,task],|r|r.get::<_,bool>(0)).unwrap_or(false);
            let state=free["run"]["status"].as_str().unwrap_or("");
            if owned&&state=="interrupted"&&enrichment::storage_busy(free["run"]["reason"].as_str().unwrap_or("")){
                match self.enrichment.dispatch(p,"resume",json!({"taskId":task}),""){
                    Ok(value)=>free=value,
                    Err(e) if enrichment::storage_busy(&e)=>{phase(c,p,"waiting_storage",&format!("本地数据库仍繁忙；原补证队列保留并自动重试：{e}"))?;return Ok(())}
                    Err(e)=>return Err(e),
                }
            }
        }
        enrichment(c, p, &free)?;
        let free_state = free["run"]["status"].as_str().unwrap_or("");
        if free_state=="waiting_storage"{
            phase(c,p,"waiting_storage",free["run"]["reason"].as_str().unwrap_or("本地数据库繁忙，正在沿原补证队列退避重试"))?;return Ok(())
        }
        if free["run"]["enabled"]==false&&!matches!(free_state,"completed"){
            phase(c,p,"blocked",&format!("免费补证当前为 {free_state}，未继续执行：{}；原任务和逐项进度保留",free["run"]["reason"].as_str().unwrap_or("请查看任务明细")))?;return Ok(())
        }
        let counts = &free["run"]["counts"];
        if matches!(free_state, "running" | "starting")
            || counts["pending"].as_i64().unwrap_or(0) > 0
            || counts["running"].as_i64().unwrap_or(0) > 0
        {
            crate::asset_discovery::rounds::new_evidence(
                c,
                p,
                "范围内继续采集 CT、DNS 和 TLS 证据",
            )?;
            phase(
                c,
                p,
                "free_discovery",
                "先自动补采已确认范围内的证书、CT 与 DNS/CNAME；网络错误保留明细并退避重试",
            )?;
            return Ok(());
        }
        if unchanged_steady {
            return Ok(());
        }
        if !allowed {
            if !passive_pending {
                crate::asset_discovery::rounds::complete(
                    c,
                    p,
                    "免费发现已收敛，等待新增证据；Hunter未启用",
                )?;
            }
            phase(
                c,
                p,
                if passive_pending {
                    "passive_evidence"
                } else {
                    "steady"
                },
                if passive_pending {
                    "已确认范围内免费发现同步进行；范围外候选仅向公开 CT 补证，失败自动退避"
                } else {
                    "本轮免费发现已处理；新证据自动回流，Hunter 自动补查未开启"
                },
            )?;
            return Ok(());
        }
        let (_, key, _, _) = setting(c)?;
        if key.is_empty() {
            phase(
                c,
                p,
                "blocked",
                "Hunter 自动补查已允许，但尚未保存 Key；免费发现与已完成结果保留",
            )?;
            return Ok(());
        }
        let outstanding:Vec<(String,String)>=c.prepare("SELECT query_id,state FROM asset_discovery_pipeline_queries WHERE operation_id=?1 AND project_id=?2 AND state IN ('held','running')").map_err(err)?.query_map(params![op,p],|r|Ok((r.get(0)?,r.get(1)?))).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;
        for (query, _) in outstanding {
            let receipt = hunter_expansion::status_on(c, p, &query)?;
            if receipt["status"] == "completed" {
                c.execute("UPDATE asset_discovery_pipeline_queries SET state='completed' WHERE operation_id=?1 AND query_id=?2",params![op,query]).map_err(err)?;
            } else {
                phase(c,p,"blocked","原 Hunter 请求需要在查询记录中核对或恢复；未追加其他付费查询，免费证据继续保留和处理")?;
                return Ok(());
            }
        }
        // Freeze one paid wave. New seeds returned by its queries belong to the
        // next round, while all original seeds share this same round number.
        let wave = match crate::asset_discovery::rounds::wave(c, p)? {
            Some(wave) => wave,
            None => {
                let wave = candidates(c, p, &op)?;
                if !wave.is_empty() {
                    crate::asset_discovery::rounds::new_evidence(c, p, "开始已允许的精确 Hunter 查询波次")?;
                }
                crate::asset_discovery::rounds::save_wave(c, p, &wave)?;
                wave
            }
        };
        for input in wave {
            if input.project_id!=p{return Err("发现轮次计划不属于当前项目，未执行查询".into())}
            if cancel.is_cancelled() || !enabled(c, p, id)? {
                return Ok(());
            }
            // Scope, exclusions and evidence may change after the wave was
            // planned. A stale plan is never paid authorization.
            if hunter_expansion::evidence_anchor(c, p, &input.target, &input.kind).is_err() {
                continue;
            }
            let old:Option<(String,String)>=c.query_row("SELECT query_id,state FROM asset_discovery_pipeline_queries WHERE operation_id=?1 AND kind=?2 AND target=?3",params![op,input.kind,input.target],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(err)?;
            let (query, state) = if let Some(old) = old {
                old
            } else {
                let query = hunter_expansion::prepare_inline(&self.path, &input)?;
                let before = hunter_expansion::status_on(c, p, &query)?;
                c.execute("INSERT INTO asset_discovery_pipeline_queries(operation_id,project_id,kind,target,query_id,baseline_spent,baseline_pages,baseline_unique,round_number) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,(SELECT round FROM asset_discovery_pipelines WHERE project_id=?2))",params![op,p,input.kind,input.target,query,before["spentPoints"].as_i64().unwrap_or(0),c.query_row("SELECT COUNT(*) FROM hunter_expansion_pages WHERE query_id=?1",[&query],|r|r.get::<_,i64>(0)).map_err(err)?,before["uniqueRows"].as_i64().unwrap_or(0)]).map_err(err)?;
                (query, "prepared".into())
            };
            let before = hunter_expansion::status_on(c, p, &query)?;
            if before["status"] == "completed" {
                c.execute("UPDATE asset_discovery_pipeline_queries SET state='completed' WHERE operation_id=?1 AND query_id=?2",params![op,query]).map_err(err)?;
                continue;
            }
            if state == "held"
                || before["costUnknown"] == true
                || matches!(
                    before["status"].as_str(),
                    Some("unknown" | "interrupted" | "provider_limit" | "quota" | "partial")
                )
            {
                phase(
                    c,
                    p,
                    "blocked",
                    "Hunter 原请求需核对或恢复；可能已扣费，未自动重发。免费结果和下页游标已保留",
                )?;
                return Ok(());
            }
            phase(
                c,
                p,
                "hunter",
                "仅对有真实来源的证书 SHA-256、已确认域名树或具体 CNAME 自动翻页；费用逐页记录",
            )?;
            c.execute("UPDATE asset_discovery_pipeline_queries SET state='running' WHERE operation_id=?1 AND query_id=?2",params![op,query]).map_err(err)?;
            let data = self
                .data_dir
                .as_deref()
                .ok_or("当前运行环境缺少资产数据目录，未发起 Hunter 请求")?;
            let result = hunter_expansion::run_prepared_inline(
                &self.path,
                data,
                p,
                &query,
                || {
                    cancel.is_cancelled()||db::open(&self.path).and_then(|check|check.query_row("SELECT enabled AND allow_hunter AND operation_id=?2 FROM asset_discovery_pipelines WHERE project_id=?1",params![p,op],|r|r.get::<_,bool>(0)).map_err(err)).map(|on|!on).unwrap_or(true)
                },
            )?;
            let done = result["status"] == "completed";
            c.execute("UPDATE asset_discovery_pipeline_queries SET state=?3,reason=?4 WHERE operation_id=?1 AND query_id=?2",params![op,query,if done{"completed"}else if result["status"]=="paused"{"paused"}else{"held"},result["reason"].as_str().unwrap_or("")]).map_err(err)?;
            if !done {
                if !cancel.is_cancelled() {
                    phase(
                        c,
                        p,
                        "blocked",
                        result["reason"]
                            .as_str()
                            .unwrap_or("Hunter 请求未完成，进度保留；未自动重发"),
                    )?;
                }
                return Ok(());
            }
            // Yield after one query. Imported rows are reviewed before the next
            // paid seed, so later queries use current scope and evidence.
            c.execute("UPDATE asset_discovery_pipelines SET phase='local_analysis',reason='Hunter 结果已去重回流；先分析新增证据，再执行本轮其余种子' WHERE project_id=?1 AND operation_id=?2 AND enabled=1",params![p,op]).map_err(err)?;
            track(c, p)?;
            return Ok(());
        }
        if !candidates(c, p, &op)?.is_empty() {
            crate::asset_discovery::rounds::next_wave(
                c,
                p,
                "本轮精确查询已完成；新增可信种子进入下一轮",
            )?;
            phase(
                c,
                p,
                "local_analysis",
                "已合并并提炼新增种子，自动开始下一轮；已完成查询不重放",
            )?;
            return Ok(());
        }
        if !passive_pending {
            crate::asset_discovery::rounds::complete(c, p, "本轮精确查询已完成，等待新增可信种子")?;
        }
        phase(
            c,
            p,
            if passive_pending {
                "passive_evidence"
            } else {
                "steady"
            },
            if passive_pending {
                "已确认范围的精确查询已完成；公开 CT 继续核实其他候选，不阻塞现有范围"
            } else {
                "本轮可信精确查询已完成；相同种子和已完成分页不会重复收费，继续等待新增证据"
            },
        )?;
        Ok(())
    }
    pub(super) fn pause_pipeline(&self, c: &Connection, p: i64) -> Result<(), String> {
        let row:Option<(String,Option<i64>,String)>=c.query_row("SELECT operation_id,collection_run_id,enrichment_run_id FROM asset_discovery_pipelines WHERE project_id=?1",[p],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(err)?;
        let Some((op, collection, free)) = row else {
            return Ok(());
        };
        c.execute("UPDATE asset_discovery_pipelines SET enabled=0,phase='paused',reason='已暂停；原任务、证据和分页回执保留，未知请求不会重发' WHERE project_id=?1",[p]).map_err(err)?;
        crate::asset_discovery::rounds::pause(c, p)?;
        if let (Some(id), Some(jobs)) = (collection, self.jobs.as_ref()) {
            let running:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM runs WHERE id=?1 AND project_id=?2 AND status IN ('queued','pending','running'))",params![id,p],|r|r.get(0)).map_err(err)?;
            if running {
                let _ = crate::jobs::cancel_job_in(jobs, id);
            }
        }
        if !free.is_empty() {
            let _ = self
                .enrichment
                .dispatch(p, "pause", json!({"taskId":free}), "");
        }
        if has(c, "hunter_expansions")? {
            let ids:Vec<String>=c.prepare("SELECT query_id FROM asset_discovery_pipeline_queries WHERE project_id=?1 AND operation_id=?2 AND state='running'").map_err(err)?.query_map(params![p,op],|r|r.get(0)).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;
            for query in ids {
                hunter_expansion::pause_inline(&self.path, p, &query)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "discovery_pipeline_tests.rs"]
mod tests;
