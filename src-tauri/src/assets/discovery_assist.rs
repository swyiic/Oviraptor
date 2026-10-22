//! One authorized, durable owner for local evidence -> AI -> project proposal.
//! Scope remains explicit; only the existing free enrichment owner executes it.
use super::*;
use crate::agent_runtime::model::GatewayProfile;
use std::{
    path::PathBuf,
    sync::{Arc, Condvar},
};

const ASSIST_TABLE: &str = "CREATE TABLE IF NOT EXISTS asset_seed_assists(project_id INTEGER PRIMARY KEY REFERENCES projects(id),run_id TEXT NOT NULL DEFAULT '',organization TEXT NOT NULL,include_isolated INTEGER NOT NULL,recipient TEXT NOT NULL DEFAULT '',phase TEXT NOT NULL,enabled INTEGER NOT NULL,reason TEXT NOT NULL DEFAULT '',proposal_json TEXT NOT NULL DEFAULT 'null',enrichment_pending TEXT NOT NULL DEFAULT '',pending_roots_json TEXT NOT NULL DEFAULT '[]',enrichment_error TEXT NOT NULL DEFAULT '',enrichment_retry_at INTEGER NOT NULL DEFAULT 0,owner_pid INTEGER NOT NULL DEFAULT 0,updated_at TEXT NOT NULL DEFAULT(datetime('now')));CREATE TABLE IF NOT EXISTS asset_seed_assist_refresh(project_id INTEGER NOT NULL,asset_id INTEGER NOT NULL,PRIMARY KEY(project_id,asset_id));";

fn ensure_assist(c: &Connection) -> Result<(), String> {
    c.execute_batch(TABLES).map_err(err)?;
    c.execute_batch(ASSIST_TABLE).map_err(err)?;
    c.execute_batch("CREATE TABLE IF NOT EXISTS asset_seed_assist_free_owners(project_id INTEGER PRIMARY KEY REFERENCES projects(id),task_id TEXT NOT NULL)").map_err(err)?;
    pipeline::ensure(c)?;
    let mut fields = c
        .prepare("PRAGMA table_info(asset_seed_assists)")
        .map_err(err)?;
    let columns = fields
        .query_map([], |r| r.get::<_, String>(1))
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    drop(fields);
    if !columns.iter().any(|c| c == "enrichment_retry_at") {
        c.execute("ALTER TABLE asset_seed_assists ADD COLUMN enrichment_retry_at INTEGER NOT NULL DEFAULT 0",[]).map_err(err)?;
    }
    if !columns.iter().any(|c| c == "owned_enrichment_run_id") {
        c.execute("ALTER TABLE asset_seed_assists ADD COLUMN owned_enrichment_run_id TEXT NOT NULL DEFAULT ''",[]).map_err(err)?;
    }
    Ok(())
}
fn exists_assist(c: &Connection) -> Result<bool, String> {
    c.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='asset_seed_assists')",
        [],
        |r| r.get(0),
    )
    .map_err(err)
}
pub(super) fn refresh_run_on(c: &Connection, p: i64) -> Result<Option<String>, String> {
    if !exists_assist(c)? {
        return Ok(None);
    }
    c.query_row("SELECT a.run_id FROM asset_seed_assists a JOIN asset_seed_reviews r ON r.id=a.run_id AND r.project_id=a.project_id WHERE a.project_id=?1 AND a.enabled=1",[p],|r|r.get(0)).optional().map_err(err)
}
pub(super) fn defer_refresh(c: &Connection, p: i64, asset: i64) -> Result<(), String> {
    if exists_assist(c)? {
        c.execute("INSERT OR IGNORE INTO asset_seed_assist_refresh SELECT ?1,?2 WHERE EXISTS(SELECT 1 FROM asset_seed_assists WHERE project_id=?1 AND enabled=1)",params![p,asset]).map_err(err)?;
    }
    Ok(())
}
fn drain_refresh(c: &mut Connection, p: i64, id: &str) -> Result<(), String> {
    if active(id) {
        return Ok(());
    }
    let ids: Vec<i64> = c
        .prepare("SELECT asset_id FROM asset_seed_assist_refresh WHERE project_id=?1 LIMIT 250")
        .map_err(err)?
        .query_map([p], |r| r.get(0))
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?;
    if ids.is_empty() {
        return Ok(());
    }
    refresh_enriched_assets(c, p, ids.iter().copied())?;
    if !active(id) {
        for asset in ids {
            c.execute(
                "DELETE FROM asset_seed_assist_refresh WHERE project_id=?1 AND asset_id=?2",
                params![p, asset],
            )
            .map_err(err)?;
        }
    }
    Ok(())
}
fn recipient(profile: &GatewayProfile) -> String {
    format!(
        "{:x}",
        Sha256::digest(
            format!("{}|{}", profile.endpoint, profile.bearer().unwrap_or("")).as_bytes()
        )
    )
}
fn profile_on(c: &Connection) -> Result<GatewayProfile, String> {
    let network = crate::model_configuration::default_settings_for(c, &["network"]);
    if network.get("_configurationError").is_some() {
        return Err("业务网络配置不可用".into());
    }
    let mut profile = crate::model_configuration::gateway_profile_on(
        c,
        network["proxyUrl"].as_str().filter(|s| !s.is_empty()),
    )?;
    profile.max_output_tokens = Some(2048);
    Ok(profile)
}
fn checked_profile(c: &Connection, p: i64, id: &str) -> Result<GatewayProfile, String> {
    let profile = profile_on(c)?;
    check_recipient(c, p, id, &recipient(&profile))?;
    Ok(profile)
}
fn check_recipient(c: &Connection, p: i64, id: &str, actual: &str) -> Result<(), String> {
    let (run, organization, target, expected, saved): (String,String,String,String,String) = c.query_row("SELECT a.run_id,a.organization,r.organization,a.recipient,r.recipient FROM asset_seed_assists a JOIN asset_seed_reviews r ON r.id=a.run_id AND r.project_id=a.project_id WHERE a.project_id=?1 AND a.run_id=?2", params![p,id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(err)?;
    if run != id
        || organization != target
        || expected.is_empty()
        || expected != actual
        || !saved.is_empty() && saved != actual
    {
        return Err(
            "目标单位或模型接收方已变化，自动分析已停止；已有证据和回执保留，未向新接收方发送"
                .into(),
        );
    }
    Ok(())
}
fn enabled(c: &Connection, p: i64, id: &str) -> Result<bool, String> {
    c.query_row(
        "SELECT enabled FROM asset_seed_assists WHERE project_id=?1 AND run_id=?2",
        params![p, id],
        |r| r.get(0),
    )
    .map_err(err)
}
fn phase(c: &Connection, p: i64, id: &str, next: &str, reason: &str) -> Result<(), String> {
    c.execute("UPDATE asset_seed_assists SET phase=?3,reason=?4,updated_at=datetime('now') WHERE project_id=?1 AND run_id=?2 AND enabled=1",params![p,id,next,reason]).map_err(err)?;
    Ok(())
}
fn block(c: &Connection, p: i64, id: &str, reason: &str) -> Result<(), String> {
    c.execute("UPDATE asset_seed_assists SET phase='blocked',enabled=0,owner_pid=0,reason=?3,updated_at=datetime('now') WHERE project_id=?1 AND run_id=?2 AND enabled=1",params![p,id,reason]).map_err(err)?;
    Ok(())
}

// Cached once per completed pass or explicit confirmation. Polling never scans
// all evidence. The confirmation owner still validates the current digest.
pub(super) fn refresh_proposal(c: &Connection, p: i64, id: &str) -> Result<(), String> {
    if !exists_assist(c)? {
        return Ok(());
    }
    let selected: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM asset_seed_assists WHERE project_id=?1 AND run_id=?2)",
            params![p, id],
            |r| r.get(0),
        )
        .map_err(err)?;
    if !selected {
        return Ok(());
    }
    let input: scope::ProposalInput =
        serde_json::from_value(json!({"projectId":p,"runId":id})).map_err(err)?;
    let proposal = scope::proposal_on(c, &input)?;
    let has = proposal["groups"].as_array().is_some_and(|v| !v.is_empty());
    c.execute("UPDATE asset_seed_assists SET proposal_json=?3,phase=CASE WHEN enabled=0 THEN phase WHEN ?4 THEN 'awaiting_scope' ELSE 'ready' END,reason=CASE WHEN enabled=0 THEN reason WHEN ?4 THEN 'AI 已完成项目提炼；只需一次确认新增范围，已有范围继续自动补证' ELSE '已完成提炼；已确认范围自动补证，缺证线索自动暂存，仅证据冲突需审核' END,updated_at=datetime('now') WHERE project_id=?1 AND run_id=?2",params![p,id,proposal.to_string(),has]).map_err(err)?;
    Ok(())
}
pub(super) fn record_enrichment_intent(
    c: &Connection,
    p: i64,
    id: &str,
    token: &str,
) -> Result<(), String> {
    if exists_assist(c)? {
        c.execute("UPDATE asset_seed_assists SET enrichment_pending=?3,pending_roots_json=?4,enrichment_error='',enrichment_retry_at=0,updated_at=datetime('now') WHERE project_id=?1 AND run_id=?2",params![p,id,token,json!(roots(c,p)?).to_string()]).map_err(err)?;
    }
    Ok(())
}
pub(super) fn enrichment_receipt(
    c: &Connection,
    p: i64,
    id: &str,
    token: &str,
    failure: Option<&str>,
) -> Result<(), String> {
    if exists_assist(c)? {
        if failure.is_none(){pipeline::confirmed_enrichment(c,p,id,token)?;}
        c.execute("UPDATE asset_seed_assists SET enrichment_pending=CASE WHEN ?5='' THEN '' ELSE enrichment_pending END,enrichment_error=?5,enrichment_retry_at=CASE WHEN ?5='' THEN 0 ELSE CAST(strftime('%s','now') AS INTEGER)+60 END,updated_at=datetime('now') WHERE project_id=?1 AND run_id=?2 AND enrichment_pending=?3 AND pending_roots_json=?4",params![p,id,token,json!(roots(c,p)?).to_string(),failure.unwrap_or("")]).map_err(err)?;
    }
    Ok(())
}

// Reuse the same immutable target and model receipts. Only newly attached high
// water rows extend this pass; completed/unknown model rows are never reset.
fn prepare_pass(
    c: &mut Connection,
    p: i64,
    organization: &str,
    include: bool,
    identity: &str,
    live: &HashSet<String>,
) -> Result<String, String> {
    let organization = organization.trim();
    if organization.is_empty() || organization.chars().count() > 240 {
        return Err("请填写明确的目标单位，最多240字".into());
    }
    let existing: Option<String> = c.query_row("SELECT id FROM asset_seed_reviews WHERE project_id=?1 AND organization=?2 AND include_isolated=?3 ORDER BY rowid DESC LIMIT 1",params![p,organization,include],|r|r.get(0)).optional().map_err(err)?;
    let id = match existing {
        Some(id) => id,
        None => prepare_on(c, p, organization, include, live)?,
    };
    let old_recipient: String = c
        .query_row(
            "SELECT recipient FROM asset_seed_reviews WHERE id=?1",
            [&id],
            |r| r.get(0),
        )
        .map_err(err)?;
    if !old_recipient.is_empty() && old_recipient != identity {
        return Err("模型接收方已变化，未重新发送旧证据；请先确认接收方配置".into());
    }
    let (total,max): (i64,i64) = c.query_row(&format!("SELECT COUNT(*),COALESCE(MAX(a.id),0) FROM assets a JOIN project_assets pa ON pa.asset_id=a.id WHERE pa.project_id=?1 AND (?2 OR NOT {ISOLATED})"),params![p,include],|r|Ok((r.get(0)?,r.get(1)?))).map_err(err)?;
    c.execute(
        "UPDATE asset_seed_reviews SET cursor=CASE WHEN total!=?3 AND ?2<=max_id THEN 0 ELSE cursor END,processed=CASE WHEN total!=?3 AND ?2<=max_id THEN 0 ELSE processed END,max_id=MAX(max_id,?2),total=?3,recipient=?4 WHERE id=?1",
        params![id, max, total, identity],
    )
    .map_err(err)?;
    Ok(id)
}

fn run_pass(
    c: &mut Connection,
    p: i64,
    id: &str,
    cancel: &CancelToken,
    resolve: impl FnOnce(&Connection) -> Result<GatewayProfile, String>,
) -> Result<(), String> {
    let (cursor, max): (i64, i64) = c
        .query_row(
            "SELECT cursor,max_id FROM asset_seed_reviews WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(err)?;
    if cursor < max {
        phase(
            c,
            p,
            id,
            "preparing",
            "正在自动整理整个项目的本地证据，无需逐批点击",
        )?;
        status(c, id, "preparing", "自动整理整个项目证据")?;
        loop {
            if cancel.is_cancelled() || !enabled(c, p, id)? {
                return Ok(());
            }
            if harvest_page_mode(c, p, id, true)? {
                break;
            }
        }
    } else {
        triage::run(c, p, id)?;
    }
    if cancel.is_cancelled() || !enabled(c, p, id)? {
        return Ok(());
    }
    // An interrupted paid request is quarantined before continuation. Its
    // outcome can be unknown, so only remaining genuinely pending rows run.
    let lost: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM asset_seed_review_items WHERE run_id=?1 AND state='analyzing')",[id],|r|r.get(0)).map_err(err)?;
    if lost {
        unknown_on(
            c,
            id,
            "上次模型请求中断，是否计费未知；自动继续其他证据，不重放这批请求",
        )?;
        c.execute(
            "UPDATE asset_seed_reviews SET unknown_requests=unknown_requests+1 WHERE id=?1",
            [id],
        )
        .map_err(err)?;
    }
    let pending: i64 = c.query_row("SELECT COUNT(*) FROM asset_seed_review_items WHERE run_id=?1 AND state='pending' AND decision='pending'",[id],|r|r.get(0)).map_err(err)?;
    if pending > 0 {
        let profile = resolve(c)?;
        check_recipient(c, p, id, &recipient(&profile))?;
        if cancel.is_cancelled() || !enabled(c, p, id)? {
            return Ok(());
        }
        c.execute(
            "UPDATE asset_seed_reviews SET model=?2 WHERE id=?1",
            params![id, profile.model],
        )
        .map_err(err)?;
        phase(
            c,
            p,
            id,
            "analyzing",
            "AI 正在自动提炼相关种子；缺证暂存，无关项自动分流",
        )?;
        status(c, id, "analyzing", "自动 AI 审阅")?;
        analyze(
            c,
            p,
            id,
            OpenAiCompatibleGateway::new(profile, &[]),
            cancel,
            0,
        )?;
        if cancel.is_cancelled() || !enabled(c, p, id)? {
            return Ok(());
        }
        let (state, reason): (String, String) = c
            .query_row(
                "SELECT status,reason FROM asset_seed_reviews WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(err)?;
        if state != "completed" {
            return Err(reason);
        }
    } else {
        status(c, id, "completed", "自动分流完成；无需发送模型或重复审阅")?;
    }
    phase(
        c,
        p,
        id,
        "analyzing",
        "AI 已完成，正在汇总全项目范围建议与当前证书依据",
    )?;
    refresh_proposal(c, p, id)
}

#[derive(Clone)]
pub(crate) struct Manager {
    path: PathBuf,
    enrichment: enrichment::Manager,
    jobs: Option<crate::jobs::AssetJobContext>,
    data_dir: Option<PathBuf>,
    watcher: Arc<(Mutex<bool>, Condvar)>,
}
impl Manager {
    pub(crate) fn new(path: &Path, enrichment: enrichment::Manager) -> Result<Self, String> {
        Self::build(path,enrichment,None)
    }
    pub(crate) fn new_with_jobs(path:&Path,enrichment:enrichment::Manager,jobs:crate::jobs::AssetJobContext)->Result<Self,String>{
        Self::build(path,enrichment,Some(jobs))
    }
    fn build(path:&Path,enrichment:enrichment::Manager,jobs:Option<crate::jobs::AssetJobContext>)->Result<Self,String>{
        let c = db::open(path)?;
        ensure_assist(&c)?;
        let manager = Self {
            path: path.into(),
            enrichment,
            data_dir: jobs.as_ref().map(|j|j.app_data_dir().to_path_buf()),
            jobs,
            watcher: Arc::new((Mutex::new(false), Condvar::new())),
        };
        let rows: Vec<(i64, String, String, i64)> = c
            .prepare(
                "SELECT project_id,run_id,phase,owner_pid FROM asset_seed_assists WHERE enabled=1 OR enrichment_pending!=''",
            )
            .map_err(err)?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?;
        for (p, id, phase, pid) in rows {
            if pid > 0
                && pid != std::process::id() as i64
                && unsafe { libc::kill(pid as i32, 0) } == 0
            {
                continue;
            }
            let _ = manager.resume_enrichment(&c, p, &id);
            if matches!(phase.as_str(), "preparing" | "analyzing") {
                if let Err(e) = checked_profile(&c, p, &id).and_then(|_| manager.launch(p, &id)) {
                    block(&c, p, &id, &e)?;
                }
            }
        }
        let owner = manager.clone();
        thread::Builder::new()
            .name("project-seed-assist-watch".into())
            .spawn(move || loop {
                let (lock, wake) = &*owner.watcher;
                let Ok(stopped) = lock.lock() else { break };
                let Ok((stopped, _)) =
                    wake.wait_timeout_while(stopped, Duration::from_secs(15), |stopped| !*stopped)
                else {
                    break;
                };
                if *stopped {
                    break;
                }
                drop(stopped);
                let _ = owner.continue_pending();
            })
            .map_err(err)?;
        Ok(manager)
    }
    // A durable enabled workflow keeps learning from new local evidence. An
    // idle watcher does not hold the review control token, so TLS enrichment
    // can safely refresh pending evidence without resetting paid/model rows.
    fn continue_pending(&self) -> Result<(), String> {
        let mut c = db::open(&self.path)?;
        let saved:Vec<(i64,String,i64)>=c.prepare("SELECT project_id,run_id,owner_pid FROM asset_seed_assists WHERE run_id!='' AND ((enabled=1 AND phase IN ('ready','awaiting_scope','preparing','analyzing')) OR enrichment_pending!='')").map_err(err)?.query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;
        for (p, id, pid) in saved {
            let _ = self.resume_enrichment(&c, p, &id);
            if !enabled(&c, p, &id)? {
                continue;
            }
            if active(&id)
                || pid > 0
                    && pid != std::process::id() as i64
                    && unsafe { libc::kill(pid as i32, 0) } == 0
            {
                continue;
            }
            drain_refresh(&mut c, p, &id)?;
            let (include,old_max,cursor,old_total):(bool,i64,i64,i64)=c.query_row("SELECT include_isolated,max_id,cursor,total FROM asset_seed_reviews WHERE id=?1 AND project_id=?2",params![id,p],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(err)?;
            let(total,max):(i64,i64)=c.query_row(&format!("SELECT COUNT(*),COALESCE(MAX(a.id),0) FROM assets a JOIN project_assets pa ON pa.asset_id=a.id WHERE pa.project_id=?1 AND (?2 OR NOT {ISOLATED})"),params![p,include],|r|Ok((r.get(0)?,r.get(1)?))).map_err(err)?;
            let pending:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM asset_seed_review_items WHERE run_id=?1 AND decision='pending' AND state IN ('pending','analyzing'))",[&id],|r|r.get(0)).map_err(err)?;
            if max <= old_max && cursor >= old_max && total == old_total && !pending {
                continue;
            }
            if let Err(e) = checked_profile(&c, p, &id) {
                block(&c, p, &id, &e)?;
                continue;
            }
            // Deduplication may attach an older global asset ID. A membership
            // count change re-reads local facts, preserving all done/unknown rows.
            let rescan = total != old_total && max <= old_max;
            c.execute(
                "UPDATE asset_seed_reviews SET max_id=MAX(max_id,?2),total=?3,cursor=CASE WHEN ?4 THEN 0 ELSE cursor END,processed=CASE WHEN ?4 THEN 0 ELSE processed END WHERE id=?1",
                params![id, max, total,rescan],
            )
            .map_err(err)?;
            phase(
                &c,
                p,
                &id,
                if rescan || cursor < max.max(old_max) {
                    "preparing"
                } else {
                    "analyzing"
                },
                "发现新的项目证据，自动接续分析；已完成和结果未知的模型请求不重发",
            )?;
            if let Err(e) = self.launch(p, &id) {
                block(&c, p, &id, &e)?;
            }
        }
        // New membership and refreshed TLS facts take precedence over another
        // paid query. The pipeline shares this same review control token.
        self.continue_pipelines(&mut c)?;
        Ok(())
    }
    fn launch(&self, p: i64, id: &str) -> Result<(), String> {
        let mut guard = controls().lock().map_err(|_| "审阅状态锁不可用")?;
        if guard.contains_key(id) {
            return Ok(());
        }
        let cancel = CancelToken::new();
        guard.insert(id.into(), cancel.clone());
        let c = db::open(&self.path)?;
        c.execute("UPDATE asset_seed_assists SET owner_pid=?3 WHERE project_id=?1 AND run_id=?2 AND enabled=1",params![p,id,std::process::id()]).map_err(err)?;
        drop(guard);
        let path = self.path.clone();
        let run = id.to_string();
        spawn_owned(
            self.path.clone(),
            id.into(),
            "automatic-project-seeds",
            move || {
                let mut c = db::open(&path)?;
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_pass(&mut c, p, &run, &cancel, profile_on)
                }))
                .unwrap_or_else(|_| Err("自动分析线程异常，进度保留，未知请求不会重放".into()));
                if let Err(e) = &result {
                    block(&c, p, &run, e)?;
                }
                c.execute(
                    "UPDATE asset_seed_assists SET owner_pid=0 WHERE project_id=?1 AND run_id=?2",
                    params![p, run],
                )
                .map_err(err)?;
                result
            },
        )
    }
    fn resume_enrichment(&self, c: &Connection, p: i64, id: &str) -> Result<(), String> {
        let saved: Option<(String,String)> = c.query_row("SELECT enrichment_pending,pending_roots_json FROM asset_seed_assists WHERE project_id=?1 AND run_id=?2 AND enrichment_pending!='' AND enrichment_retry_at<=?3",params![p,id,chrono::Utc::now().timestamp()],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(err)?;
        if let Some((token, expected)) = saved {
            if expected != json!(roots(c, p)?).to_string() {
                c.execute("UPDATE asset_seed_assists SET enrichment_pending='',enrichment_retry_at=0,enrichment_error='待启动补证的确认范围已变化，未自动沿用旧请求' WHERE project_id=?1 AND run_id=?2 AND enrichment_pending=?3",params![p,id,token]).map_err(err)?;
                return Ok(());
            }
            let result = self
                .enrichment
                .dispatch(p, "start", json!({}), &format!("scope-{token}"));
            enrichment_receipt(c, p, id, &token, result.as_ref().err().map(String::as_str))?;
            pipeline::enrichment(c,p,&result?)?;
        }
        Ok(())
    }
    fn start_enrichment(&self, c: &Connection, p: i64) -> Result<(), String> {
        let scope = roots(c, p)?;
        if scope.is_empty() {
            return Ok(());
        }
        let operation:Option<String>=c.query_row("SELECT operation_id FROM asset_discovery_pipelines WHERE project_id=?1 AND enabled=1",[p],|r|r.get(0)).optional().map_err(err)?;
        let op = format!("assist-{}", operation.unwrap_or_else(||uuid::Uuid::new_v4().to_string()));
        let active_before:Vec<String>=if c.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='asset_enrichment_runs')",[],|r|r.get::<_,bool>(0)).map_err(err)?{
            c.prepare("SELECT id FROM asset_enrichment_runs WHERE project_id=?1 AND enabled=1").map_err(err)?.query_map([p],|r|r.get(0)).map_err(err)?.collect::<Result<_,_>>().map_err(err)?
        }else{vec![]};
        match self.enrichment.dispatch(p, "start", json!({}), &op) {
            Ok(value) => {
                if let Some(task)=value["run"]["id"].as_str(){
                    if !active_before.iter().any(|id|id==task){
                        c.execute("INSERT INTO asset_seed_assist_free_owners VALUES(?1,?2) ON CONFLICT(project_id) DO UPDATE SET task_id=excluded.task_id",params![p,task]).map_err(err)?;
                        c.execute("UPDATE asset_seed_assists SET owned_enrichment_run_id=?2 WHERE project_id=?1",params![p,task]).map_err(err)?;
                        c.execute("UPDATE asset_discovery_pipelines SET enrichment_run_id=?2 WHERE project_id=?1 AND enabled=1",params![p,task]).map_err(err)?;
                    }
                }
                pipeline::enrichment(c,p,&value)?;
                c.execute(
                    "UPDATE asset_seed_assists SET enrichment_error='',enrichment_pending=CASE WHEN pending_roots_json=?2 THEN '' ELSE enrichment_pending END,enrichment_retry_at=0 WHERE project_id=?1",
                    params![p,json!(scope).to_string()],
                )
                .map_err(err)?;
                Ok(())
            }
            Err(e) => {
                c.execute(
                    "UPDATE asset_seed_assists SET enrichment_error=?2 WHERE project_id=?1",
                    params![p, e],
                )
                .map_err(err)?;
                Err(e)
            }
        }
    }
    fn start(&self, input: &Input) -> Result<Value, String> {
        let c=db::open(&self.path)?;
        project(&c,input.project_id)?;ensure_assist(&c)?;
        let org=input.organization.as_deref().unwrap_or("").trim();
        if org.is_empty()||org.chars().count()>240{return Err("请填写明确的目标单位，最多240字".into())}
        if input.collect_first && self.jobs.is_none(){return Err("当前运行环境未接通 FOFA 采集任务；未发起采集".into())}
        let previous:Option<String>=c.query_row("SELECT run_id FROM asset_seed_assists WHERE project_id=?1",[input.project_id],|r|r.get(0)).optional().map_err(err)?;
        if input.collect_first && previous.as_deref().is_some_and(active){return Err("项目仍有分析或精确查询正在执行，请先暂停并等待保存，再重新首采".into())}
        if previous.as_deref().is_some_and(active){
            let same:bool=c.query_row("SELECT organization=?2 AND include_isolated=?3 FROM asset_seed_assists WHERE project_id=?1",params![input.project_id,org,input.include_isolated],|r|r.get(0)).map_err(err)?;
            if !same{return Err("项目仍有已绑定目标的执行正在退出或运行；请先暂停保存，再更换目标单位".into())}
        }
        let has_assets:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM project_assets WHERE project_id=?1)",[input.project_id],|r|r.get(0)).map_err(err)?;
        if has_assets||input.collect_first{let _=profile_on(&c)?;}
        pipeline::start_on(&c,input,org)?;
        let pending_collection:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM asset_discovery_pipelines d WHERE d.project_id=?1 AND d.collect_first=1 AND (d.collection_run_id IS NULL OR NOT EXISTS(SELECT 1 FROM runs r WHERE r.id=d.collection_run_id AND r.project_id=d.project_id AND r.status='completed')))",[input.project_id],|r|r.get(0)).map_err(err)?;
        if input.collect_first || pending_collection {
            // Save the receiving model and collection handoff before starting
            // the original job owner. A restart reuses the same operation ID.
            let profile=profile_on(&c)?;
            let identity=recipient(&profile);
            c.execute("INSERT INTO asset_seed_assists(project_id,organization,include_isolated,recipient,phase,enabled,reason) VALUES(?1,?2,?3,?4,'collecting',1,'FOFA 首采完成后自动分析') ON CONFLICT(project_id) DO UPDATE SET run_id='',organization=excluded.organization,include_isolated=excluded.include_isolated,recipient=excluded.recipient,enabled=1,phase='collecting',reason=excluded.reason",params![input.project_id,org,input.include_isolated,identity]).map_err(err)?;
            if self.start_collection(&c,input.project_id,true)?{return assist_status(&c,input.project_id)}
        }
        self.start_local(input)
    }
    fn start_local(&self, input: &Input) -> Result<Value, String> {
        let mut c = db::open(&self.path)?;
        project(&c, input.project_id)?;
        ensure_assist(&c)?;
        let org = input.organization.as_deref().unwrap_or("").trim();
        if org.is_empty() || org.chars().count() > 240 {
            return Err("请填写明确的目标单位，最多240字".into());
        }
        let count: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM project_assets WHERE project_id=?1",
                [input.project_id],
                |r| r.get(0),
            )
            .map_err(err)?;
        if count == 0 {
            c.execute("INSERT INTO asset_seed_assists(project_id,organization,include_isolated,phase,enabled,reason) VALUES(?1,?2,?3,'empty',0,'此项目还没有本地资产，请先采集或导入；无需空跑 AI') ON CONFLICT(project_id) DO UPDATE SET run_id='',organization=excluded.organization,include_isolated=excluded.include_isolated,phase='empty',enabled=0,proposal_json='null',reason=excluded.reason,updated_at=datetime('now')",params![input.project_id,org,input.include_isolated]).map_err(err)?;
            return assist_status(&c, input.project_id);
        }
        let profile = profile_on(&c)?;
        let identity = recipient(&profile);
        {
            let guard = controls().lock().map_err(|_| "审阅状态锁不可用")?;
            let owned_live: Vec<(String,String,bool)> = c.prepare("SELECT id,organization,include_isolated FROM asset_seed_reviews WHERE project_id=?1").map_err(err)?.query_map([input.project_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;
            for (id, target, include) in owned_live {
                if guard.contains_key(&id) {
                    let same: bool=c.query_row("SELECT EXISTS(SELECT 1 FROM asset_seed_assists WHERE project_id=?1 AND run_id=?2 AND enabled=1)",params![input.project_id,id],|r|r.get(0)).map_err(err)?;
                    if same && target == org && include == input.include_isolated {
                        drop(guard);
                        return assist_status(&c, input.project_id);
                    }
                    return Err(
                        "项目已有分析正在退出或执行，请先暂停并等待保存；不会重复发起 AI 请求"
                            .into(),
                    );
                }
            }
            let owner: Option<i64> = c
                .query_row(
                    "SELECT owner_pid FROM asset_seed_assists WHERE project_id=?1",
                    [input.project_id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(err)?;
            if owner.is_some_and(|pid| {
                pid > 0
                    && pid != std::process::id() as i64
                    && unsafe { libc::kill(pid as i32, 0) } == 0
            }) {
                return Err("项目分析仍由其他存活进程持有".into());
            }
            let id = prepare_pass(
                &mut c,
                input.project_id,
                org,
                input.include_isolated,
                &identity,
                &guard.keys().cloned().collect(),
            )?;
            let preparing: bool = c
                .query_row(
                    "SELECT cursor<max_id FROM asset_seed_reviews WHERE id=?1",
                    [&id],
                    |r| r.get(0),
                )
                .map_err(err)?;
            c.execute("INSERT INTO asset_seed_assists(project_id,run_id,organization,include_isolated,recipient,phase,enabled,reason) VALUES(?1,?2,?3,?4,?5,?6,1,'已开启自动分析；本地整理、AI提炼与范围建议自动接续') ON CONFLICT(project_id) DO UPDATE SET run_id=excluded.run_id,organization=excluded.organization,include_isolated=excluded.include_isolated,recipient=excluded.recipient,phase=excluded.phase,enabled=1,reason=excluded.reason,proposal_json='null',enrichment_error='',updated_at=datetime('now')",params![input.project_id,id,org,input.include_isolated,identity,if preparing {"preparing"}else{"analyzing"}]).map_err(err)?;
        }
        let id: String = c
            .query_row(
                "SELECT run_id FROM asset_seed_assists WHERE project_id=?1",
                [input.project_id],
                |r| r.get(0),
            )
            .map_err(err)?;
        let _ = self.start_enrichment(&c, input.project_id);
        if let Err(e) = self.launch(input.project_id, &id) {
            block(&c, input.project_id, &id, &e)?;
        }
        assist_status(&c, input.project_id)
    }
    fn pause(&self, p: i64) -> Result<Value, String> {
        let c = db::open(&self.path)?;
        project(&c, p)?;
        ensure_assist(&c)?;
        let id: Option<String> = c
            .query_row(
                "SELECT run_id FROM asset_seed_assists WHERE project_id=?1",
                [p],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        c.execute("UPDATE asset_seed_assists SET enabled=0,phase='paused',enrichment_pending='',enrichment_retry_at=0,reason='已暂停自动分析和免费补证；已完成结果保留，继续时不会重放未知模型请求',updated_at=datetime('now') WHERE project_id=?1",[p]).map_err(err)?;
        self.pause_pipeline(&c,p)?;
        if let Some(id) = id {
            if let Some(cancel) = controls().lock().map_err(err)?.get(&id) {
                cancel.cancel();
            }
        }
        // Pause a legacy queue only when its ID was recorded by this Assist's
        // successful start/confirmation, never by guessing the latest task.
        let owned_free:Option<String>=c.query_row("SELECT task_id FROM asset_seed_assist_free_owners WHERE project_id=?1 UNION SELECT owned_enrichment_run_id FROM asset_seed_assists WHERE project_id=?1 AND owned_enrichment_run_id!='' LIMIT 1",[p],|r|r.get(0)).optional().map_err(err)?;
        if let Some(task)=owned_free.filter(|s|!s.is_empty()){let _=self.enrichment.dispatch(p,"pause",json!({"taskId":task}),"");}
        assist_status(&c, p)
    }
    pub(crate) fn shutdown(&self) {
        let (lock, wake) = &*self.watcher;
        if let Ok(mut stopped) = lock.lock() {
            *stopped = true;
            wake.notify_all();
        }
        if let Ok(c) = db::open(&self.path) {
            if let Ok(mut q) =
                c.prepare("SELECT run_id FROM asset_seed_assists WHERE enabled=1 AND owner_pid=?1")
            {
                if let Ok(ids) = q.query_map([std::process::id()], |r| r.get::<_, String>(0)) {
                    if let Ok(guard) = controls().lock() {
                        for id in ids.flatten() {
                            if let Some(cancel) = guard.get(&id) {
                                cancel.cancel();
                            }
                        }
                    }
                }
            }
        }
    }
}

fn assist_status(c: &Connection, p: i64) -> Result<Value, String> {
    let local: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM project_assets WHERE project_id=?1)",
            [p],
            |r| r.get(0),
        )
        .map_err(err)?;
    let row: Option<(String, String, bool, String, String, String, String, bool)> = if exists_assist(c)? {
        c.query_row("SELECT run_id,phase,enabled,reason,proposal_json,organization,enrichment_error,include_isolated FROM asset_seed_assists WHERE project_id=?1",[p],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).optional().map_err(err)?
    } else {
        None
    };
    let (id, phase, enabled, reason, proposal, organization, enrichment_error, include_isolated) = row
        .unwrap_or_else(|| {
            (
                String::new(),
                if local { "idle" } else { "empty" }.into(),
                false,
                if local {
                    "填写目标单位后，点击一次开始；后续整理与分析会自动接续"
                } else {
                    "先采集或导入资产，再让 AI 提炼相关种子"
                }
                .into(),
                "null".into(),
                String::new(),
                String::new(),
                true,
            )
        });
    // Old cached advice may have promoted an unbound provider name list into
    // an entire family. Re-project the derived cache once on policy upgrade;
    // this performs no network/model work and preserves original decisions,
    // receipts, confirmed scopes and the paused execution state.
    let cached: Value = serde_json::from_str(&proposal).unwrap_or(Value::Null);
    if !id.is_empty()
        && cached["verificationPolicyVersion"].as_i64() != Some(scope::VERIFICATION_POLICY_VERSION)
        && !matches!(phase.as_str(), "preparing" | "analyzing")
    {
        let complete: bool = c.query_row(
            "SELECT cursor>=max_id FROM asset_seed_reviews WHERE id=?1 AND project_id=?2",
            params![id,p], |r| r.get(0),
        ).optional().map_err(err)?.unwrap_or(false);
        if complete {
            refresh_proposal(c, p, &id)?;
            return assist_status(c, p);
        }
    }
    let review = if id.is_empty() {
        json!({"run":null,"items":[],"total":0})
    } else {
        view_run_on(c, p, Some(&id), "conflict", 1, "")?
    };
    Ok(
        json!({"phase":phase,"enabled":enabled,"reason":reason,"runId":if id.is_empty(){Value::Null}else{json!(id)},"hasLocalAssets":local,"organization":organization,"includeIsolated":include_isolated,"review":review,"proposal":serde_json::from_str::<Value>(&proposal).unwrap_or(Value::Null),"enrichmentError":enrichment_error,"pipeline":pipeline::status(c,p)?}),
    )
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Input {
    project_id: i64,
    action: String,
    organization: Option<String>,
    #[serde(default = "yes")]
    include_isolated: bool,
    #[serde(default)]
    collect_first: bool,
    #[serde(default)]
    collection_targets: Vec<String>,
    #[serde(default)]
    allow_hunter: Option<bool>,
}
#[tauri::command]
pub async fn control_seed_assist(
    state: tauri::State<'_, Manager>,
    input: Input,
) -> Result<Value, String> {
    let manager = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || match input.action.as_str() {
        "start" => manager.start(&input),
        "pause" => manager.pause(input.project_id),
        _ => Err("自动分析动作无效".into()),
    })
    .await
    .map_err(err)?
}
#[tauri::command]
pub async fn seed_review_assist_status(
    state: tauri::State<'_, Manager>,
    project_id: i64,
) -> Result<Value, String> {
    let path = state.path.clone();
    tauri::async_runtime::spawn_blocking(move || assist_status(&db::open(&path)?, project_id))
        .await
        .map_err(err)?
}

#[cfg(test)]
#[path = "discovery_assist_tests.rs"]
mod tests;

#[path = "discovery_pipeline.rs"]
mod pipeline;

#[path = "discovery_assist_bridge.rs"]
mod chat_bridge;
pub(crate) use chat_bridge::dispatch as dispatch_chat;
