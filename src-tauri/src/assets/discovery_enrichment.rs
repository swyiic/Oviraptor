//! A persistent, free discovery queue shared by UI and Agent. No model/paid API,
//! shell, HTTP page fetch, ownership approval or quarantine restoration lives here.
use super::*;
use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex, OnceLock,
    },
    thread,
};
#[path = "discovery_enrichment_network.rs"]
mod network;
const TABLES:&str="
CREATE TABLE IF NOT EXISTS asset_enrichment_runs(id TEXT PRIMARY KEY,project_id INTEGER NOT NULL REFERENCES projects(id),operation_id TEXT NOT NULL,roots_json TEXT NOT NULL,status TEXT NOT NULL,enabled INTEGER NOT NULL,continuous INTEGER NOT NULL,interval_seconds INTEGER NOT NULL,workers INTEGER NOT NULL,owner_pid INTEGER NOT NULL DEFAULT 0,asset_cursor INTEGER NOT NULL DEFAULT 0,seed_cursor INTEGER NOT NULL DEFAULT 0,last_sync INTEGER NOT NULL DEFAULT 0,reason TEXT NOT NULL DEFAULT '',created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,UNIQUE(project_id,operation_id));
CREATE UNIQUE INDEX IF NOT EXISTS asset_enrichment_active ON asset_enrichment_runs(project_id) WHERE enabled=1;
CREATE TABLE IF NOT EXISTS asset_enrichment_items(id INTEGER PRIMARY KEY,run_id TEXT NOT NULL REFERENCES asset_enrichment_runs(id),kind TEXT NOT NULL,host TEXT NOT NULL,target TEXT NOT NULL,status TEXT NOT NULL DEFAULT 'pending',attempts INTEGER NOT NULL DEFAULT 0,failures INTEGER NOT NULL DEFAULT 0,next_at INTEGER NOT NULL,result_json TEXT NOT NULL DEFAULT '{}',error TEXT NOT NULL DEFAULT '',updated_at INTEGER NOT NULL,UNIQUE(run_id,kind,target));
CREATE INDEX IF NOT EXISTS asset_enrichment_due ON asset_enrichment_items(run_id,next_at,status,id);
CREATE TABLE IF NOT EXISTS asset_enrichment_observations(id INTEGER PRIMARY KEY,project_id INTEGER NOT NULL,kind TEXT NOT NULL,target TEXT NOT NULL,content_hash TEXT NOT NULL,payload_json TEXT NOT NULL,first_seen INTEGER NOT NULL,last_seen INTEGER NOT NULL,UNIQUE(project_id,kind,target,content_hash));
CREATE TABLE IF NOT EXISTS asset_enrichment_events(id INTEGER PRIMARY KEY,run_id TEXT NOT NULL,item_id INTEGER,kind TEXT NOT NULL,target TEXT NOT NULL,outcome TEXT NOT NULL,detail TEXT NOT NULL,created_at INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS asset_enrichment_events_run ON asset_enrichment_events(run_id,id);
CREATE TABLE IF NOT EXISTS asset_enrichment_conflicts(project_id INTEGER NOT NULL,kind TEXT NOT NULL,identity TEXT NOT NULL,hosts_json TEXT NOT NULL,detail TEXT NOT NULL,decision TEXT NOT NULL DEFAULT 'pending',revision INTEGER NOT NULL DEFAULT 0,first_seen INTEGER NOT NULL,last_seen INTEGER NOT NULL,PRIMARY KEY(project_id,kind,identity));
";
pub(crate) fn certificate_covers(host: &str, pattern: &str) -> bool {
    network::covers(host, pattern)
}
fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
const MAX_FREE_FAILURES: i64 = 5;
pub(crate) fn storage_busy(error:&str)->bool{
    let value=error.trim().to_ascii_lowercase();
    value.starts_with("database is locked")||value.starts_with("database table is locked")||value.starts_with("database schema is locked")||value.contains("sqlite_busy")||value.contains("sqlite_locked")
}

fn finish_single_pass(c: &Connection, id: &str, continuous: bool) -> Result<bool, String> {
    if continuous { return Ok(false); }
    let pending:i64=c.query_row("SELECT COUNT(*) FROM asset_enrichment_items WHERE run_id=?1 AND status IN ('pending','running','retry')",[id],|r|r.get(0)).map_err(err)?;
    if pending != 0 { return Ok(false); }
    let failed:i64=c.query_row("SELECT COUNT(*) FROM asset_enrichment_items WHERE run_id=?1 AND status='failed'",[id],|r|r.get(0)).map_err(err)?;
    let reason = if failed == 0 { "本轮补证全部处理完成；不表示资产归属或存活已确认".to_string() }
        else { format!("本轮补证已结束；{failed} 项永久错误或已达到重试上限，详情保留，未将失败当作成功") };
    c.execute("UPDATE asset_enrichment_runs SET status='completed',enabled=0,reason=?2 WHERE id=?1",params![id,reason]).map_err(err)?;
    Ok(true)
}
#[derive(Default)]
struct Signal {
    stop: AtomicBool,
    storage_busy: AtomicBool,
    lock: Mutex<()>,
    wake: Condvar,
}
impl Signal {
    fn stop(&self) {
        // The predicate and notification share the waiter's mutex. Otherwise a
        // stop between its predicate check and entering wait loses the wakeup.
        let _guard = self.lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        self.stop.store(true, Ordering::SeqCst);
        self.wake.notify_all();
    }
    fn wait(&self, seconds: u64) {
        if let Ok(g) = self.lock.lock() {
            let _ = self
                .wake
                .wait_timeout_while(g, Duration::from_secs(seconds), |_| {
                    !self.stop.load(Ordering::SeqCst)
                });
        }
    }
}
#[derive(Clone)]
pub(crate) struct Manager {
    path: std::path::PathBuf,
    active: Arc<Mutex<HashMap<String, Arc<Signal>>>>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Start {
    #[serde(default = "yes")]
    continuous: bool,
    #[serde(default = "interval")]
    interval_seconds: i64,
    #[serde(default = "workers")]
    workers: usize,
}
fn yes() -> bool {
    true
}
fn interval() -> i64 {
    3600
}
fn workers() -> usize {
    4
}
#[derive(Clone, Debug)]
struct Item {
    id: i64,
    kind: String,
    host: String,
    target: String,
}
impl Manager {
    pub(crate) fn new(path: &Path) -> Result<Self, String> {
        let c = db::open(path)?;
        c.execute_batch(TABLES).map_err(err)?;
        // UI, conversation Host and native autonomous planner share one owner.
        static ACTIVE: OnceLock<Arc<Mutex<HashMap<String, Arc<Signal>>>>> = OnceLock::new();
        let m = Self {
            path: path.into(),
            active: ACTIVE
                .get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
                .clone(),
        };
        let saved: Vec<(String, i64, i64)> = c
            .prepare("SELECT id,project_id,owner_pid FROM asset_enrichment_runs WHERE enabled=1")
            .map_err(err)?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?;
        for (id, p, pid) in saved {
            if pid > 0 && unsafe { libc::kill(pid as i32, 0) } == 0 {
                continue;
            }
            c.execute("UPDATE asset_enrichment_items SET status='pending',next_at=?2,error='进程退出后恢复免费补证请求' WHERE run_id=?1 AND status='running'",params![id,now()]).map_err(err)?;
            if let Err(e) = m.resume(p, &id) {
                if storage_busy(&e){let _=c.execute("UPDATE asset_enrichment_runs SET status='waiting_storage',reason=?2,owner_pid=0 WHERE id=?1",params![id,e]);m.wait_for_storage(p,&id)?;}
                else{c.execute("UPDATE asset_enrichment_runs SET enabled=0,status='scope_changed',reason=?2 WHERE id=?1",params![id,e]).map_err(err)?;}
            }
        }
        Ok(m)
    }
    pub(crate) fn dispatch(
        &self,
        p: i64,
        action: &str,
        args: Value,
        operation: &str,
    ) -> Result<Value, String> {
        match action {
            "status" => {
                if !args.is_object() || args.as_object().unwrap().keys().any(|k| k != "taskId") {
                    return Err("discovery_arguments_invalid".into());
                }
                self.status(p, args["taskId"].as_str())
            }
            "start" => self.start(p, serde_json::from_value(args).map_err(err)?, operation),
            "pause" | "resume" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase", deny_unknown_fields)]
                struct Control {
                    task_id: String,
                }
                let input: Control = serde_json::from_value(args).map_err(err)?;
                if action == "pause" {
                    self.pause(p, &input.task_id)
                } else {
                    self.resume(p, &input.task_id)
                }
            }
            "review_conflict" => self.decide(p, args),
            _ => Err("discovery_action_invalid".into()),
        }
    }
    fn wait_for_storage(&self,p:i64,id:&str)->Result<(),String>{
        let mut active=self.active.lock().map_err(err)?;
        if active.contains_key(id){return Ok(())}
        let signal=Arc::new(Signal::default());signal.storage_busy.store(true,Ordering::SeqCst);
        active.insert(id.into(),signal.clone());drop(active);
        let owner=self.clone();let saved=id.to_string();let cleanup=saved.clone();
        if let Err(e)=thread::Builder::new().name("scoped-evidence-storage-retry".into()).spawn(move||owner.run(p,&saved,signal)){
            self.active.lock().map_err(err)?.remove(&cleanup);return Err(err(e))
        }
        Ok(())
    }
    fn start(&self, p: i64, s: Start, operation: &str) -> Result<Value, String> {
        if !(900..=86400).contains(&s.interval_seconds)
            || !(1..=8).contains(&s.workers)
            || operation.is_empty()
            || operation.len() > 128
        {
            return Err("补证参数无效：间隔15分钟至24小时，并发1–8".into());
        }
        let mut c = db::open(&self.path)?;
        project(&c, p)?;
        ensure(&c)?;
        let scope = roots(&c, p)?;
        if scope.is_empty() {
            return Err("请先保存明确确认的域名范围；目标单位或AI建议不是探测授权".into());
        }
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(err)?;
        let mut retired = None;
        if let Some((id, saved))=tx.query_row("SELECT id,roots_json FROM asset_enrichment_runs WHERE project_id=?1 AND (operation_id=?2 OR enabled=1) ORDER BY enabled DESC,created_at DESC,rowid DESC LIMIT 1",params![p,operation],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional().map_err(err)? {
            if saved == json!(scope).to_string() {drop(tx);return self.resume(p,&id)}
            tx.execute("UPDATE asset_enrichment_runs SET status='scope_changed',enabled=0,reason='确认范围已变化，旧进度保留，由新范围任务接续' WHERE id=?1",[&id]).map_err(err)?;
            retired = Some(id);
        }
        // An explicit continuation may carry a fresh operation ID. Reuse its
        // persisted free queue rather than throwing away cursors and completed
        // observations. Disabled scope_changed histories are never candidates.
        if s.continuous {
            let recoverable: Option<String> = tx.query_row(
                "SELECT id FROM asset_enrichment_runs r WHERE project_id=?1 AND roots_json=?2 AND enabled=0 AND continuous=1 AND (status IN ('paused','pausing','interrupted') OR (status='completed' AND EXISTS(SELECT 1 FROM asset_enrichment_items i WHERE i.run_id=r.id AND i.status='retry'))) ORDER BY updated_at DESC,created_at DESC,rowid DESC LIMIT 1",
                params![p,json!(scope).to_string()], |r|r.get(0),
            ).optional().map_err(err)?;
            if let Some(id) = recoverable {
                tx.commit().map_err(err)?;
                if let Some(retired) = retired {
                    if let Some(signal) = self.active.lock().map_err(err)?.get(&retired) { signal.stop(); }
                }
                return self.resume(p, &id);
            }
        }
        let id = uuid::Uuid::new_v4().to_string();
        tx.execute("INSERT INTO asset_enrichment_runs(id,project_id,operation_id,roots_json,status,enabled,continuous,interval_seconds,workers,created_at,updated_at) VALUES(?1,?2,?3,?4,'paused',1,?5,?6,?7,?8,?8)",params![id,p,operation,json!(scope).to_string(),s.continuous,s.interval_seconds,s.workers as i64,now()]).map_err(err)?;
        tx.commit().map_err(err)?;
        if let Some(retired) = retired {
            if let Some(signal) = self.active.lock().map_err(err)?.get(&retired) {
                signal.stop();
            }
        }
        self.resume(p, &id)
    }
    fn resume(&self, p: i64, id: &str) -> Result<Value, String> {
        let mut active = self.active.lock().map_err(err)?;
        if let Some(signal) = active.get(id) {
            if signal.stop.load(Ordering::SeqCst) {
                return Err("补证任务正在保存暂停进度，请稍后继续；原队列和游标保留".into());
            }
            drop(active);
            return self.status(p, Some(id));
        }
        let c = db::open(&self.path)?;
        if let Err(e) = validate(&c, p, id) {
            if !storage_busy(&e){c.execute("UPDATE asset_enrichment_runs SET status='scope_changed',enabled=0,owner_pid=0,reason=?3 WHERE id=?1 AND project_id=?2",params![id,p,e]).map_err(err)?;}
            return Err(e);
        }
        let pid: i64 = c
            .query_row(
                "SELECT owner_pid FROM asset_enrichment_runs WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(err)?;
        if pid > 0 && pid != std::process::id() as i64 && unsafe { libc::kill(pid as i32, 0) } == 0
        {
            return Err("补证任务仍由其他存活进程持有".into());
        }
        c.execute("UPDATE asset_enrichment_items SET status='pending' WHERE run_id=?1 AND status='running'",[id]).map_err(err)?;
        c.execute("UPDATE asset_enrichment_runs SET status='running',enabled=1,owner_pid=?2,reason='自动补证：本地证书 → CT → DNS/CNAME → 范围内TLS，逐项保存',updated_at=?3 WHERE id=?1",params![id,std::process::id(),now()]).map_err(err)?;
        let signal = Arc::new(Signal::default());
        active.insert(id.into(), signal.clone());
        drop(active);
        let m = self.clone();
        let id = id.to_string();
        let copy = id.clone();
        if let Err(e) = thread::Builder::new()
            .name("scoped-evidence-discovery".into())
            .spawn(move || m.run(p, &copy, signal))
        {
            self.active.lock().map_err(err)?.remove(&id);
            c.execute("UPDATE asset_enrichment_runs SET status='interrupted',enabled=0,owner_pid=0,reason=?2 WHERE id=?1",params![id,e.to_string()]).map_err(err)?;
            return Err(err(e));
        }
        self.status(p, Some(&id))
    }
    fn pause(&self, p: i64, id: &str) -> Result<Value, String> {
        let c = db::open(&self.path)?;
        owned(&c, p, id)?;
        let a = self.active.lock().map_err(err)?;
        let signal = a.get(id);
        let already_stopping = signal.is_some_and(|s| s.stop.load(Ordering::SeqCst));
        let state = if signal.is_some() { "pausing" } else { "paused" };
        // Commit before waking a fast owner. Its final paused receipt must not
        // subsequently be overwritten by this control or a repeated pause.
        c.execute("UPDATE asset_enrichment_runs SET status=CASE WHEN ?5 OR status IN ('paused','interrupted','completed','scope_changed') THEN status ELSE ?3 END,enabled=0,reason=CASE WHEN ?5 OR status IN ('paused','interrupted','completed','scope_changed') THEN reason ELSE '已停止领取新项，正在保存当前免费请求；进度可恢复' END,updated_at=?4 WHERE id=?1 AND project_id=?2",params![id,p,state,now(),already_stopping]).map_err(err)?;
        if let Some(signal) = signal.filter(|_| !already_stopping) { signal.stop(); }
        drop(a);
        self.status(p, Some(id))
    }
    pub(crate) fn shutdown(&self) {
        if let Ok(a) = self.active.lock() {
            for s in a.values() {
                s.stop()
            }
        }
    }
    fn run(&self, p: i64, id: &str, signal: Arc<Signal>) {
        let mut recovering_storage=signal.storage_busy.load(Ordering::SeqCst);
        let outcome = loop { let attempt=(|| -> Result<(), String> {
            let mut c = db::open(&self.path)?;
            if recovering_storage{
                validate(&c,p,id)?;
                // No network batch survives an iteration boundary. These are
                // claims that never reached execution, not received responses.
                c.execute("UPDATE asset_enrichment_items SET status='pending',attempts=MAX(0,attempts-1),error='本地存储竞争后恢复未执行领取项',next_at=?2 WHERE run_id=?1 AND status='running'",params![id,now()]).map_err(err)?;
                c.execute("UPDATE asset_enrichment_runs SET status='running',owner_pid=?3,reason='本地存储已可用，沿原队列继续补证',updated_at=?2 WHERE id=?1 AND enabled=1",params![id,now(),std::process::id()]).map_err(err)?;
                signal.storage_busy.store(false,Ordering::SeqCst);
                recovering_storage=false;
            }
            loop {
                if signal.stop.load(Ordering::SeqCst) {
                    break;
                }
                validate(&c, p, id)?;
                let more_local = sync(&mut c, p, id)?;
                // Finish the cheap project inventory scan before network work;
                // a slow CT/TLS request must not delay discovering later rows.
                if more_local {
                    continue;
                }
                let (n, continuous): (i64, bool) = c
                    .query_row(
                        "SELECT workers,continuous FROM asset_enrichment_runs WHERE id=?1",
                        [id],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .map_err(err)?;
                let mut batch = vec![];
                for _ in 0..n {
                    if let Some(item) = claim(&mut c, p, id)? {
                        batch.push(item)
                    } else {
                        break;
                    }
                }
                if batch.is_empty() {
                    if more_local {
                        continue;
                    }
                    if finish_single_pass(&c, id, continuous)? {
                        break;
                    }
                    c.execute("UPDATE asset_enrichment_runs SET status='waiting',reason=CASE WHEN EXISTS(SELECT 1 FROM asset_enrichment_items WHERE run_id=?1 AND status='retry') THEN '等待退避重试；当前任务未完成，重启后自动接续' ELSE '当前可执行项已处理；等待新资产、线索或下一次增量检查' END,updated_at=?2 WHERE id=?1",params![id,now()]).map_err(err)?;
                    signal.wait(15);
                    continue;
                }
                c.execute(
                    "UPDATE asset_enrichment_runs SET status='running',updated_at=?2 WHERE id=?1",
                    params![id, now()],
                )
                .map_err(err)?;
                let settings = crate::model_configuration::default_settings_for(&c, &["network"]);
                if settings.get("_configurationError").is_some() {
                    return Err("业务网络配置不可用".into());
                }
                let client = network::client(settings["proxyUrl"].as_str().unwrap_or(""))?;
                let (tx, rx) = std::sync::mpsc::channel();
                thread::scope(|scope| {
                    let count = batch.len();
                    for item in batch {
                        let sender = tx.clone();
                        let client = client.clone();
                        let path = self.path.clone();
                        let task = id.to_string();
                        scope.spawn(move || {
                            let result = network::execute(&path, &task, &item, &client);
                            let _ = sender.send((item, result));
                        });
                    }
                    drop(tx);
                    let mut failure = None;
                    for _ in 0..count {
                        if let Ok((item, result)) = rx.recv() {
                            if let Err(e) = finish_received(&mut c, p, id, &item, result, &signal) {
                                failure = Some(e)
                            }
                        }
                    }
                    failure.map_or(Ok(()), Err)
                })?;
            }
            Ok(())
        })();
            match attempt{
                Err(e) if storage_busy(&e)=>{
                    if signal.stop.load(Ordering::SeqCst){break Ok(())}
                    signal.storage_busy.store(true,Ordering::SeqCst);
                    if let Ok(c)=db::open(&self.path){let _=c.execute("UPDATE asset_enrichment_runs SET status='waiting_storage',reason=?2,updated_at=?3 WHERE id=?1 AND enabled=1",params![id,format!("本地数据库繁忙，保留原队列并自动重试：{e}"),now()]);}
                    recovering_storage=true;
                    signal.wait(1);
                }
                other=>break other,
            }
        };
        if let Ok(c) = db::open(&self.path) {
            let (state, reason) = match outcome {
                Ok(()) => ("paused", "已暂停，逐项进度与结果保留".to_string()),
                Err(e) => ("interrupted", e),
            };
            let _=c.execute("UPDATE asset_enrichment_runs SET status=CASE WHEN status IN ('completed','scope_changed') THEN status ELSE ?2 END,enabled=CASE WHEN ?2='interrupted' THEN 0 ELSE enabled END,owner_pid=0,reason=CASE WHEN status IN ('completed','scope_changed') THEN reason ELSE ?3 END,updated_at=?4 WHERE id=?1",params![id,state,reason,now()]);
        }
        if let Ok(mut a) = self.active.lock() {
            a.remove(id);
        }
    }
    fn status(&self, p: i64, id: Option<&str>) -> Result<Value, String> {
        let c = db::open(&self.path)?;
        let id=match id{Some(id)=>{owned(&c,p,id)?;Some(id.to_string())},None=>c.query_row("SELECT id FROM asset_enrichment_runs WHERE project_id=?1 ORDER BY rowid DESC LIMIT 1",[p],|r|r.get::<_,String>(0)).optional().map_err(err)?};
        let mut run = Value::Null;
        let mut events = vec![];
        let mut conflicts = vec![];
        if let Some(id) = id {
            run=c.query_row("SELECT id,status,continuous,interval_seconds,workers,enabled,reason,updated_at FROM asset_enrichment_runs WHERE id=?1",[&id],|r|Ok(json!({"id":r.get::<_,String>(0)?,"status":r.get::<_,String>(1)?,"continuous":r.get::<_,bool>(2)?,"intervalSeconds":r.get::<_,i64>(3)?,"workers":r.get::<_,i64>(4)?,"enabled":r.get::<_,bool>(5)?,"reason":r.get::<_,String>(6)?,"updatedAt":r.get::<_,i64>(7)?}))).map_err(err)?;
            let mut counts = BTreeMap::<String, i64>::new();
            for row in c.prepare("SELECT status,COUNT(*) FROM asset_enrichment_items WHERE run_id=?1 GROUP BY status").map_err(err)?.query_map([&id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?))).map_err(err)?{let(k,n)=row.map_err(err)?;counts.insert(k,n);}
            run["counts"] = json!(counts);
            if self.active.lock().map_err(err)?.get(&id).is_some_and(|s|s.storage_busy.load(Ordering::SeqCst)){
                run["status"]=json!("waiting_storage");run["reason"]=json!("本地数据库繁忙，正在退避重试；原请求结果、队列和游标保留");
            }
            run["operations"] = json!(c
                .query_row(
                    "SELECT COALESCE(SUM(attempts),0) FROM asset_enrichment_items WHERE run_id=?1",
                    [&id],
                    |r| r.get::<_, i64>(0)
                )
                .map_err(err)?);
            run["nextCheckAt"]=json!(c.query_row("SELECT MIN(next_at) FROM asset_enrichment_items WHERE run_id=?1 AND status IN ('pending','running','retry','done')",[&id],|r|r.get::<_,Option<i64>>(0)).map_err(err)?);
            events=c.prepare("SELECT kind,target,outcome,detail,created_at FROM asset_enrichment_events WHERE run_id=?1 ORDER BY id DESC LIMIT 20").map_err(err)?.query_map([&id],|r|Ok(json!({"kind":r.get::<_,String>(0)?,"target":r.get::<_,String>(1)?,"outcome":r.get::<_,String>(2)?,"detail":r.get::<_,String>(3)?,"at":r.get::<_,i64>(4)?}))).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;
        }
        for row in c.prepare("SELECT kind,identity,hosts_json,detail,decision,revision FROM asset_enrichment_conflicts WHERE project_id=?1 ORDER BY CASE decision WHEN 'pending' THEN 0 ELSE 1 END,last_seen DESC LIMIT 20").map_err(err)?.query_map([p],|r|Ok(json!({"kind":r.get::<_,String>(0)?,"identity":r.get::<_,String>(1)?,"hosts":serde_json::from_str::<Value>(&r.get::<_,String>(2)?).unwrap_or(json!([])),"detail":r.get::<_,String>(3)?,"decision":r.get::<_,String>(4)?,"revision":r.get::<_,i64>(5)?}))).map_err(err)?{let mut row=row.map_err(err)?;if let Some(hosts)=row["hosts"].as_array(){let count=hosts.len();let preview=hosts.iter().take(8).cloned().collect::<Vec<_>>();row["hostCount"]=json!(count);row["hosts"]=json!(preview);}conflicts.push(row)}
        Ok(
            json!({"projectId":p,"run":run,"roots":roots(&c,p)?,"events":events,"conflicts":conflicts,"hunterRequests":0,"modelRequests":0,"scopeExpanded":false}),
        )
    }
    fn decide(&self, p: i64, args: Value) -> Result<Value, String> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Decision {
            kind: String,
            identity: String,
            expected_revision: i64,
            decision: String,
        }
        let d: Decision = serde_json::from_value(args).map_err(err)?;
        if !["pending", "acknowledged"].contains(&d.decision.as_str())
            || d.identity.len() > 300
            || d.kind.len() > 60
        {
            return Err("冲突审核参数无效".into());
        }
        let c = db::open(&self.path)?;
        if c.execute("UPDATE asset_enrichment_conflicts SET decision=?5,revision=revision+1 WHERE project_id=?1 AND kind=?2 AND identity=?3 AND revision=?4",params![p,d.kind,d.identity,d.expected_revision,d.decision]).map_err(err)?!=1{return Err("冲突证据已变化，请刷新".into())}
        self.status(p, None)
    }
}
fn owned(c: &Connection, p: i64, id: &str) -> Result<(), String> {
    let yes: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM asset_enrichment_runs WHERE id=?1 AND project_id=?2)",
            params![id, p],
            |r| r.get(0),
        )
        .map_err(err)?;
    if yes {
        Ok(())
    } else {
        Err("补证任务不属于当前项目".into())
    }
}
fn validate(c: &Connection, p: i64, id: &str) -> Result<Vec<String>, String> {
    project(c, p)?;
    owned(c, p, id)?;
    let saved: String = c
        .query_row(
            "SELECT roots_json FROM asset_enrichment_runs WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .map_err(err)?;
    let current = roots(c, p)?;
    if saved != json!(current).to_string() {
        return Err("确认范围已变化；补证已停止，请按当前范围重新启动".into());
    }
    Ok(current)
}
fn admitted(c: &Connection, p: i64, name: &str) -> Result<bool, String> {
    Ok(approved_host(c, p, name).is_ok())
}
fn enqueue(c: &Connection, id: &str, kind: &str, h: &str, target: &str) -> Result<(), String> {
    c.execute("INSERT OR IGNORE INTO asset_enrichment_items(run_id,kind,host,target,next_at,updated_at) VALUES(?1,?2,?3,?4,?5,?5) ON CONFLICT(run_id,kind,target) DO UPDATE SET status=CASE WHEN status='skipped' THEN 'pending' ELSE status END,next_at=CASE WHEN status='skipped' THEN excluded.next_at ELSE next_at END",params![id,kind,h,target,now()]).map_err(err)?;
    Ok(())
}
fn enqueue_host(c: &Connection, p: i64, id: &str, h: &str) -> Result<(), String> {
    if !admitted(c, p, h)? {
        return Ok(());
    }
    for k in ["dns_a", "dns_aaaa", "dns_cname"] {
        enqueue(c, id, k, h, h)?
    }
    enqueue(c, id, "tls", h, &format!("https://{h}/"))
}
fn sync(c: &mut Connection, p: i64, id: &str) -> Result<bool, String> {
    let scope = validate(c, p, id)?;
    let(ac,sc,last,repeat):(i64,i64,i64,i64)=c.query_row("SELECT asset_cursor,seed_cursor,last_sync,interval_seconds FROM asset_enrichment_runs WHERE id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(err)?;
    for root in &scope {
        if admitted(c, p, root)? {
            insert_seed(c, p, root, "confirmed_scope", root, false)?;
            enqueue(c, id, "ct", root, root)?;
            enqueue_host(c, p, id, root)?
        }
    }
    let reset = last > 0 && now() - last >= repeat;
    let scan = if reset {
        c.execute(
            "UPDATE asset_enrichment_runs SET asset_cursor=0,seed_cursor=0,last_sync=0 WHERE id=?1",
            [id],
        )
        .map_err(err)?;
        0
    } else {
        ac
    };
    let rows:Vec<(i64,String,String,String)>=c.prepare("SELECT a.id,a.host,a.link,a.extra_json FROM assets a JOIN project_assets pa ON pa.asset_id=a.id WHERE pa.project_id=?1 AND a.id>?2 ORDER BY a.id LIMIT 250").map_err(err)?.query_map(params![p,scan],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    for (a, h, l, e) in &rows {
        // A certificate's SAN may belong to the confirmed project even when
        // its observed endpoint is an external/cloud IP. Read the full saved
        // certificate, not the bounded review-card preview.
        let value: Value = serde_json::from_str(e).unwrap_or(Value::Null);
        for name in review::observed_names(&value, if l.is_empty() { h } else { l }) {
            if admitted(&tx, p, &name)? {
                insert_seed(
                    &tx,
                    p,
                    &name,
                    "confirmed_review_name",
                    &format!("asset:{a}"),
                    false,
                )?;
                enqueue_host(&tx, p, id, &name)?;
            }
        }
        if let Some(name) =
            host(if l.is_empty() { h } else { l }).filter(|h| admitted(&tx, p, h).unwrap_or(false))
        {
            ingest_observations(&tx, p, &[(*a, h.clone(), l.clone(), e.clone())])?;
            enqueue_host(&tx, p, id, &name)?;
            if let Ok(mut url) = reqwest::Url::parse(if l.is_empty() { h } else { l }) {
                if url.scheme() == "https" && url.username().is_empty() && url.password().is_none()
                {
                    url.set_path("/");
                    url.set_query(None);
                    url.set_fragment(None);
                    enqueue(&tx, id, "tls", &name, url.as_str())?;
                }
            }
        }
    }
    let next = rows.last().map(|r| r.0).unwrap_or(scan);
    tx.execute("UPDATE asset_enrichment_runs SET asset_cursor=?2,last_sync=CASE WHEN ?3 AND (last_sync=0 OR ?5) THEN ?4 ELSE last_sync END,updated_at=?4 WHERE id=?1",params![id,next,rows.len()<250,now(),!rows.is_empty()]).map_err(err)?;
    let seeds:Vec<(i64,String)>=tx.prepare("SELECT id,host FROM asset_discovery_seeds WHERE project_id=?1 AND id>?2 AND wildcard=0 ORDER BY id LIMIT 1000").map_err(err)?.query_map(params![p,if reset{0}else{sc}],|r|Ok((r.get(0)?,r.get(1)?))).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;
    for (_, h) in &seeds {
        if let Some(h) = host(h) {
            enqueue_host(&tx, p, id, &h)?
        }
    }
    let next = seeds
        .last()
        .map(|r| r.0)
        .unwrap_or(if reset { 0 } else { sc });
    tx.execute(
        "UPDATE asset_enrichment_runs SET seed_cursor=?2 WHERE id=?1",
        params![id, next],
    )
    .map_err(err)?;
    tx.commit().map_err(err)?;
    let more = rows.len() == 250 || seeds.len() == 1000;
    review::refresh_enriched_assets(c, p, rows.iter().map(|r| r.0))?;
    Ok(more)
}
fn claim(c: &mut Connection, p: i64, id: &str) -> Result<Option<Item>, String> {
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    validate(&tx, p, id)?;
    if !tx
        .query_row(
            "SELECT enabled FROM asset_enrichment_runs WHERE id=?1",
            [id],
            |r| r.get::<_, bool>(0),
        )
        .map_err(err)?
    {
        return Ok(None);
    }
    let item:Option<Item>=tx.query_row("SELECT i.id,i.kind,i.host,i.target FROM asset_enrichment_items i JOIN asset_enrichment_runs run ON run.id=i.run_id WHERE i.run_id=?1 AND (i.status IN ('pending','retry') OR (run.continuous=1 AND i.status='done')) AND i.next_at<=?2 AND (i.kind!='tls' OR NOT EXISTS(SELECT 1 FROM asset_enrichment_items d WHERE d.run_id=i.run_id AND d.host=i.host AND d.kind IN ('dns_a','dns_aaaa') AND (d.attempts=0 OR d.status='running'))) ORDER BY i.next_at,i.id LIMIT 1",params![id,now()],|r|Ok(Item{id:r.get(0)?,kind:r.get(1)?,host:r.get(2)?,target:r.get(3)?})).optional().map_err(err)?;
    if let Some(item) = &item {
        if !admitted(&tx, p, &item.host)? {
            tx.execute("UPDATE asset_enrichment_items SET status='skipped',error='域名已被排除或不在范围' WHERE id=?1",[item.id]).map_err(err)?;
            tx.commit().map_err(err)?;
            return Ok(None);
        }
        tx.execute("UPDATE asset_enrichment_items SET status='running',attempts=attempts+1,updated_at=?2 WHERE id=?1",params![item.id,now()]).map_err(err)?;
    }
    tx.commit().map_err(err)?;
    Ok(item)
}
fn observe(
    c: &Connection,
    p: i64,
    kind: &str,
    target: &str,
    payload: &Value,
) -> Result<(), String> {
    let raw = payload.to_string();
    let hash = format!("{:x}", Sha256::digest(raw.as_bytes()));
    c.execute("INSERT INTO asset_enrichment_observations(project_id,kind,target,content_hash,payload_json,first_seen,last_seen) VALUES(?1,?2,?3,?4,?5,?6,?6) ON CONFLICT(project_id,kind,target,content_hash) DO UPDATE SET last_seen=excluded.last_seen",params![p,kind,target,hash,raw,now()]).map_err(err)?;
    Ok(())
}
fn finish_received(c:&mut Connection,p:i64,id:&str,item:&Item,result:Result<Value,String>,signal:&Signal)->Result<(),String>{
    if result.as_ref().err().is_some_and(|e|storage_busy(e)){return Err(result.unwrap_err())}
    loop{
        match finish(c,p,id,item,result.clone()){
            Err(e) if storage_busy(&e)=>{
                signal.storage_busy.store(true,Ordering::SeqCst);
                if signal.stop.load(Ordering::SeqCst){return Err(e)}
                // Keep this exact received result in memory. Retrying local
                // persistence must never execute CT/DNS/TLS again.
                signal.wait(1);
            }
            result=>{signal.storage_busy.store(false,Ordering::SeqCst);return result}
        }
    }
}
fn finish(
    c: &mut Connection,
    p: i64,
    id: &str,
    item: &Item,
    result: Result<Value, String>,
) -> Result<(), String> {
    let scope = validate(c, p, id)?;
    if !admitted(c, p, &item.host)? {
        c.execute("UPDATE asset_enrichment_items SET status='skipped',error='请求完成前范围或排除规则已变化' WHERE id=?1",[item.id]).map_err(err)?;
        return Ok(());
    }
    let period: i64 = c
        .query_row(
            "SELECT interval_seconds FROM asset_enrichment_runs WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .map_err(err)?;
    let result = match result {
        Ok(mut v)=>{
            persist(c, p, id, item, &v, &scope)?;
            if item.kind == "tls" {
                v["hunterEligibleAtCapture"] = json!(hunter_expansion::evidence_anchor(
                    c,
                    p,
                    v["sha256"].as_str().unwrap_or(""),
                    "fingerprint"
                )
                .is_ok());
            }
            Ok(v)
        }
        Err(e)=>Err(e),
    };
    // Receipt state and its event commit atomically. A busy event insert cannot
    // increment a network failure twice when this received response is retried.
    let tx=c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(err)?;
    validate(&tx,p,id)?;
    let outcome=match result{
        Ok(v)=>{
            let next = if item.kind == "tls" {
                period.max(86400)
            } else {
                period
            };
            tx.execute("UPDATE asset_enrichment_items SET status='done',failures=0,result_json=?2,error='',next_at=?3,updated_at=?4 WHERE id=?1",params![item.id,v.to_string(),now()+next,now()]).map_err(err)?;
            (
                "done",
                format!(
                    "{}",
                    v.get("summary")
                        .and_then(Value::as_str)
                        .unwrap_or("证据已保存；不代表归属或存活已确认")
                ),
            )
        }
        Err(e) => {
            let error = e
                .chars()
                .filter(|c| !c.is_control())
                .take(1000)
                .collect::<String>();
            let failures: i64 = tx
                .query_row(
                    "SELECT failures FROM asset_enrichment_items WHERE id=?1",
                    [item.id],
                    |r| r.get(0),
                )
                .map_err(err)?;
            let backoff = (60_i64 * 2_i64.pow((failures as u32).min(10))).min(86400);
            let permanent = matches!(error.as_str(), "non_public_address" | "invalid_endpoint" | "tls_config_failed" | "certificate_invalid" | "certificate_missing");
            let state = if permanent || failures + 1 >= MAX_FREE_FAILURES { "failed" } else { "retry" };
            tx.execute("UPDATE asset_enrichment_items SET status=?5,failures=failures+1,error=?2,next_at=?3,updated_at=?4 WHERE id=?1",params![item.id,error,now()+backoff,now(),state]).map_err(err)?;
            (if state == "failed" { "failed" } else { "error" }, error)
        }
    };
    tx.execute("INSERT INTO asset_enrichment_events(run_id,item_id,kind,target,outcome,detail,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![id,item.id,item.kind,item.target,outcome.0,outcome.1,now()]).map_err(err)?;
    tx.commit().map_err(err)
}
fn persist(
    c: &mut Connection,
    p: i64,
    id: &str,
    item: &Item,
    v: &Value,
    scope: &[String],
) -> Result<(), String> {
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    validate(&tx, p, id)?;
    observe(&tx, p, &item.kind, &item.target, v)?;
    if item.kind == "ct" {
        for row in v["names"].as_array().into_iter().flatten() {
            let Some(name) = row["name"].as_str() else {
                continue;
            };
            let wildcard = row["wildcard"].as_bool().unwrap_or(false);
            if admitted(&tx, p, name)? {
                insert_seed(
                    &tx,
                    p,
                    name,
                    "certificate_transparency",
                    &format!("ct:{}:{}", item.host, row["certificateId"]),
                    wildcard,
                )?;
                if !wildcard {
                    enqueue_host(&tx, p, id, name)?
                }
            }
        }
    } else if item.kind.starts_with("dns_") {
        for row in v["records"].as_array().into_iter().flatten() {
            if row["type"] == 5 {
                if let Some(target) = row["data"].as_str().and_then(host) {
                    insert_seed(&tx, p, &target, "dns_cname", &item.host, false)?;
                    if admitted(&tx, p, &target)? {
                        enqueue_host(&tx, p, id, &target)?
                    }
                }
            }
        }
    }
    tx.commit().map_err(err)?;
    if item.kind == "tls" {
        persist_tls(c, p, id, item, v, scope)?
    }
    Ok(())
}
fn persist_tls(
    c: &mut Connection,
    p: i64,
    id: &str,
    item: &Item,
    v: &Value,
    scope: &[String],
) -> Result<(), String> {
    let cert = v["der"]
        .as_str()
        .and_then(crate::certificates::from_text)
        .ok_or("返回的叶证书无效")?;
    let patterns = cert.server_names();
    if !patterns
        .iter()
        .any(|name| network::covers(&item.host, name))
    {
        let old:Option<String>=c.query_row("SELECT hosts_json FROM asset_enrichment_conflicts WHERE project_id=?1 AND kind='certificate_hostname_mismatch' AND identity=?2",params![p,cert.sha256],|r|r.get(0)).optional().map_err(err)?;
        let mut hosts = old
            .and_then(|s| serde_json::from_str::<BTreeSet<String>>(&s).ok())
            .unwrap_or_default();
        hosts.insert(item.host.clone());
        c.execute("INSERT INTO asset_enrichment_conflicts(project_id,kind,identity,hosts_json,detail,first_seen,last_seen) VALUES(?1,'certificate_hostname_mismatch',?2,?3,'叶证书不覆盖请求主机；不将此证书送入Hunter，不据此排除资产',?4,?4) ON CONFLICT(project_id,kind,identity) DO UPDATE SET hosts_json=excluded.hosts_json,last_seen=excluded.last_seen,revision=CASE WHEN hosts_json!=excluded.hosts_json THEN revision+1 ELSE revision END,decision=CASE WHEN hosts_json!=excluded.hosts_json THEN 'pending' ELSE decision END",params![p,cert.sha256,json!(hosts).to_string(),now()]).map_err(err)?;
        return Ok(());
    }
    let url = reqwest::Url::parse(&item.target).map_err(err)?;
    if url.port_or_known_default() == Some(443) {
        if let Err(e) = crate::jobs::autonomous::seed_candidates(c, p, scope, &[item.host.clone()])
        {
            if !e.contains("已在回收站") {
                return Err(e);
            }
        }
    }
    let rows:Vec<(i64,String,String,String)>=c.prepare("SELECT a.id,a.host,a.link,a.extra_json FROM assets a JOIN project_assets pa ON pa.asset_id=a.id WHERE pa.project_id=?1 AND (instr(lower(a.host),?2)>0 OR instr(lower(a.link),?2)>0)").map_err(err)?.query_map(params![p,item.host],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;
    let mut touched = vec![];
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    validate(&tx, p, id)?;
    for (a, h, l, raw) in rows {
        if network::endpoint(&l, &h).as_deref() != Some(item.target.as_str()) {
            continue;
        }
        let mut e: Value = serde_json::from_str(&raw).map_err(err)?;
        if !e.is_object() {
            continue;
        }
        if let Some(old) = e["probe_cert_der_base64"]
            .as_str()
            .and_then(crate::certificates::from_text)
        {
            observe(
                &tx,
                p,
                "tls_previous_leaf",
                &item.target,
                &json!({"sha256":old.sha256,"der":old.der_base64,"names":old.dns_names,"organizations":old.organizations}),
            )?;
        }
        e["probe_cert_sha256"] = json!(cert.sha256);
        e["probe_cert_der_base64"] = json!(cert.der_base64);
        e["probe_cert_dns_names_json"] = json!(json!(cert.dns_names).to_string());
        e["probe_cert_capture"] = json!("captured");
        e["probe_cert_url"] = json!(item.target);
        e["probe_cert_tls_verified"] = json!("false");
        e["probe_cert_captured_at"] = json!(chrono::Utc::now().to_rfc3339());
        e["probe_cert_enrichment_id"] = json!(id);
        let raw = e.to_string();
        tx.execute(
            "UPDATE assets SET extra_json=?2 WHERE id=?1",
            params![a, raw],
        )
        .map_err(err)?;
        ingest_observations(&tx, p, &[(a, h, l, raw)])?;
        touched.push(a);
    }
    for name in cert.server_names() {
        if let Some(h) = host(name) {
            if admitted(&tx, p, &h)? {
                let wildcard = name.starts_with("*.");
                insert_seed(
                    &tx,
                    p,
                    &h,
                    "certificate_name",
                    &format!("tls:{}:{}", item.host, cert.sha256),
                    wildcard,
                )?;
                if !wildcard {
                    enqueue_host(&tx, p, id, &h)?
                }
            }
        }
    }
    tx.commit().map_err(err)?;
    for a in touched {
        review::refresh_enriched_asset(c, p, a)?;
    }
    Ok(())
}
#[tauri::command]
pub(crate) async fn enrichment_action(
    state: tauri::State<'_, Manager>,
    project_id: i64,
    action: String,
    args: Value,
) -> Result<Value, String> {
    let m = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        m.dispatch(project_id, &action, args, &uuid::Uuid::new_v4().to_string())
    })
    .await
    .map_err(err)?
}
#[cfg(test)]
#[path = "discovery_enrichment_tests.rs"]
mod tests;
