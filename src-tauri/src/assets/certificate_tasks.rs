//! Durable certificate-only queue shared by the native UI and the authenticated Host bridge.
use super::*;
use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
};
#[path = "certificate_tls.rs"]
pub(crate) mod tls;
const SCHEMA:&str="
CREATE TABLE IF NOT EXISTS asset_certificate_tasks(id TEXT PRIMARY KEY,project_id INTEGER NOT NULL REFERENCES projects(id),operation_id TEXT NOT NULL,status TEXT NOT NULL,workers INTEGER NOT NULL,timeout_seconds INTEGER NOT NULL,owner_pid INTEGER NOT NULL DEFAULT 0,selection_json TEXT NOT NULL DEFAULT '{}',reason TEXT NOT NULL DEFAULT '',created_at TEXT NOT NULL DEFAULT(datetime('now')),updated_at TEXT NOT NULL DEFAULT(datetime('now')),UNIQUE(project_id,operation_id));
CREATE TABLE IF NOT EXISTS asset_certificate_task_items(task_id TEXT NOT NULL REFERENCES asset_certificate_tasks(id),asset_id INTEGER NOT NULL,asset_key TEXT NOT NULL,endpoint TEXT NOT NULL,status TEXT NOT NULL DEFAULT 'pending',attempts INTEGER NOT NULL DEFAULT 0,fingerprint TEXT NOT NULL DEFAULT '',new_fingerprint INTEGER NOT NULL DEFAULT 0,reason TEXT NOT NULL DEFAULT '',updated_at TEXT NOT NULL DEFAULT(datetime('now')),PRIMARY KEY(task_id,asset_id));
CREATE INDEX IF NOT EXISTS asset_certificate_items_queue ON asset_certificate_task_items(task_id,status,asset_id);
CREATE INDEX IF NOT EXISTS asset_certificate_project ON asset_certificate_tasks(project_id,created_at);
";
#[derive(Clone)]
pub(crate) struct Manager {
    path: std::path::PathBuf,
    active: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
}
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Start {
    pub asset_ids: Option<Vec<i64>>,
    pub workers: Option<usize>,
    pub timeout_seconds: Option<u64>,
    #[serde(default)]
    pub start_paused: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Control {
    pub task_id: String,
    #[serde(default)]
    pub retry_failed: bool,
}
fn endpoint(link: &str, host: &str) -> Option<reqwest::Url> {
    let mut u = reqwest::Url::parse(if link.is_empty() { host } else { link }).ok()?;
    if u.scheme() != "https" || !u.username().is_empty() || u.password().is_some() {
        return None;
    }
    u.set_path("/");
    u.set_query(None);
    u.set_fragment(None);
    Some(u)
}
fn approved(c: &Connection, p: i64, u: &reqwest::Url) -> Result<bool, String> {
    Ok(super::roots(c, p)?
        .iter()
        .any(|r| u.host_str().is_some_and(|h| super::in_scope(h, r))))
}
fn saved_hash(e: &Value) -> bool {
    [
        "probe_cert_sha256",
        "cert_sha256",
        "cert.sha256",
        "tls_sha256",
    ]
    .iter()
    .any(|k| {
        e[*k]
            .as_str()
            .and_then(super::hunter_expansion::fingerprint)
            .is_some()
    })
}
fn raw(e: &Value) -> Option<crate::certificates::Certificate> {
    for k in [
        "probe_cert_der_base64",
        "certificate_der_base64",
        "certificate_pem",
        "ssl_certificate",
    ] {
        if let Some(v) = e[k].as_str() {
            if let Some(c) = crate::certificates::from_text(v) {
                return Some(c);
            }
        }
    }
    None
}
impl Manager {
    pub(crate) fn new(path: &Path) -> Result<Self, String> {
        let c = db::open(path)?;
        c.execute_batch(SCHEMA).map_err(err)?;
        let old: Vec<(String, i32)> = {
            let mut s=c.prepare("SELECT id,owner_pid FROM asset_certificate_tasks WHERE status IN ('running','pausing')").map_err(err)?;
            let rows = s
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(err)?;
            rows.collect::<Result<_, _>>().map_err(err)?
        };
        for (id, pid) in old {
            let alive = pid > 0 && unsafe { libc::kill(pid, 0) } == 0;
            if !alive {
                c.execute("UPDATE asset_certificate_tasks SET status='interrupted',owner_pid=0,reason='应用退出，进度已保存；可继续未完成项',updated_at=datetime('now') WHERE id=?1",[&id]).map_err(err)?;
                c.execute("UPDATE asset_certificate_task_items SET status='pending',reason='上次握手未保存，等待恢复' WHERE task_id=?1 AND status='running'",[id]).map_err(err)?;
            }
        }
        Ok(Self {
            path: path.into(),
            active: Arc::new(Mutex::new(HashMap::new())),
        })
    }
    pub(crate) fn dispatch(
        &self,
        p: i64,
        action: &str,
        args: Value,
        operation: &str,
    ) -> Result<Value, String> {
        let c = db::open(&self.path)?;
        if action == "start" || action == "resume" {
            super::project(&c, p)?;
        }
        drop(c);
        match action {
            "start" => self.start(p, serde_json::from_value(args).map_err(err)?, operation),
            "status" => {
                if !args.is_object() || args.as_object().unwrap().keys().any(|k| k != "taskId") {
                    return Err("certificate_arguments_invalid".into());
                }
                self.status(p, args.get("taskId").and_then(Value::as_str))
            }
            "pause" => {
                let input: Control = serde_json::from_value(args).map_err(err)?;
                self.pause(p, &input.task_id)
            }
            "resume" => {
                let input: Control = serde_json::from_value(args).map_err(err)?;
                self.resume(p, &input.task_id, input.retry_failed)
            }
            _ => Err("certificate_action_invalid".into()),
        }
    }
    fn start(&self, p: i64, input: Start, operation: &str) -> Result<Value, String> {
        if operation.is_empty() || operation.len() > 128 {
            return Err("certificate_operation_invalid".into());
        }
        let workers = input.workers.unwrap_or(4);
        let timeout = input.timeout_seconds.unwrap_or(8);
        if !(1..=16).contains(&workers) || !(2..=30).contains(&timeout) {
            return Err("certificate_limits_invalid".into());
        }
        let selected = input
            .asset_ids
            .map(|ids| ids.into_iter().collect::<HashSet<_>>());
        if selected
            .as_ref()
            .is_some_and(|v| v.is_empty() || v.len() > 200_000 || v.iter().any(|i| *i < 1))
        {
            return Err("certificate_selection_invalid".into());
        }
        let mut c = db::open(&self.path)?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(err)?;
        if let Some(id)=tx.query_row("SELECT id FROM asset_certificate_tasks WHERE project_id=?1 AND (operation_id=?2 OR status IN ('running','pausing','paused','interrupted')) ORDER BY created_at DESC LIMIT 1",params![p,operation],|r|r.get::<_,String>(0)).optional().map_err(err)?{drop(tx);if !input.start_paused{return self.resume(p,&id,false)}return self.status(p,Some(&id))}
        let confirmed_roots = super::roots(&tx, p)?;
        if confirmed_roots.is_empty() {
            return Err("请先在域名线索与补查中保存确认的域名范围".into());
        }
        let id = uuid::Uuid::new_v4().to_string();
        tx.execute("INSERT INTO asset_certificate_tasks(id,project_id,operation_id,status,workers,timeout_seconds) VALUES(?1,?2,?3,'paused',?4,?5)",params![id,p,operation,workers as i64,timeout as i64]).map_err(err)?;
        let mut inserted = 0;
        let mut selected_found = 0;
        let mut https = 0;
        let mut outside = 0;
        let mut existing = 0;
        {
            let mut q=tx.prepare("SELECT a.id,a.asset_key,a.host,a.link,a.extra_json FROM assets a JOIN project_assets pa ON pa.asset_id=a.id WHERE pa.project_id=?1 AND pa.is_deleted=0 ORDER BY a.id").map_err(err)?;
            let mut rows = q.query([p]).map_err(err)?;
            while let Some(r) = rows.next().map_err(err)? {
                let aid: i64 = r.get(0).map_err(err)?;
                if selected.as_ref().is_some_and(|s| !s.contains(&aid)) {
                    continue;
                }
                selected_found += 1;
                let key: String = r.get(1).map_err(err)?;
                let host: String = r.get(2).map_err(err)?;
                let link: String = r.get(3).map_err(err)?;
                let extra: String = r.get(4).map_err(err)?;
                let Some(u) = endpoint(&link, &host) else {
                    continue;
                };
                https += 1;
                if !confirmed_roots
                    .iter()
                    .any(|root| u.host_str().is_some_and(|h| super::in_scope(h, root)))
                {
                    outside += 1;
                    continue;
                }
                let e: Value = serde_json::from_str(&extra).unwrap_or(json!({}));
                if saved_hash(&e) {
                    existing += 1;
                    continue;
                }
                tx.execute("INSERT INTO asset_certificate_task_items(task_id,asset_id,asset_key,endpoint) VALUES(?1,?2,?3,?4)",params![id,aid,key,u.as_str()]).map_err(err)?;
                inserted += 1;
            }
        }
        if selected.as_ref().is_some_and(|s| selected_found != s.len()) {
            return Err("certificate_selection_outside_project".into());
        }
        tx.execute(
            "UPDATE asset_certificate_tasks SET status=?2,reason=?3 WHERE id=?1",
            params![
                id,
                if inserted == 0 { "completed" } else { "paused" },
                if inserted == 0 {
                    "确认范围内没有缺少指纹的 HTTPS 资产"
                } else {
                    "资产快照已保存，等待执行"
                }
            ],
        )
        .map_err(err)?;
        tx.execute("UPDATE asset_certificate_tasks SET selection_json=?2 WHERE id=?1",params![id,json!({"selectedAssets":selected_found,"httpsAssets":https,"outsideConfirmedScope":outside,"alreadyHaveFingerprint":existing,"queued":inserted,"roots":confirmed_roots}).to_string()]).map_err(err)?;
        tx.commit().map_err(err)?;
        if inserted > 0 && !input.start_paused {
            self.resume(p, &id, false)
        } else {
            self.status(p, Some(&id))
        }
    }
    pub(crate) fn status(&self, p: i64, id: Option<&str>) -> Result<Value, String> {
        let c = db::open(&self.path)?;
        if !c
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1)",
                [p],
                |r| r.get::<_, bool>(0),
            )
            .map_err(err)?
        {
            return Err("certificate_project_unavailable".into());
        }
        let ids: Vec<String> = if let Some(id) = id {
            if id.len() > 128 {
                return Err("certificate_task_unavailable".into());
            }
            vec![id.into()]
        } else {
            let mut q=c.prepare("SELECT id FROM asset_certificate_tasks WHERE project_id=?1 ORDER BY rowid DESC LIMIT 10").map_err(err)?;
            let rows = q.query_map([p], |r| r.get(0)).map_err(err)?;
            rows.collect::<Result<_, _>>().map_err(err)?
        };
        let mut tasks = vec![];
        for id in ids {
            let mut job=c.query_row("SELECT id,status,workers,timeout_seconds,reason,created_at,updated_at,selection_json FROM asset_certificate_tasks WHERE id=?1 AND project_id=?2",params![id,p],|r|Ok(json!({"id":r.get::<_,String>(0)?,"status":r.get::<_,String>(1)?,"workers":r.get::<_,i64>(2)?,"timeoutSeconds":r.get::<_,i64>(3)?,"reason":r.get::<_,String>(4)?,"createdAt":r.get::<_,String>(5)?,"updatedAt":r.get::<_,String>(6)?,"selection":serde_json::from_str::<Value>(&r.get::<_,String>(7)?).unwrap_or(json!({}))}))).optional().map_err(err)?.ok_or("certificate_task_unavailable")?;
            let counts=c.query_row("SELECT count(*),COALESCE(sum(status='captured'),0),COALESCE(sum(status='failed'),0),COALESCE(sum(status='skipped'),0),COALESCE(sum(status='pending'),0),COALESCE(sum(status='running'),0),count(DISTINCT NULLIF(fingerprint,'')),COALESCE(sum(new_fingerprint),0),COALESCE(sum(attempts),0) FROM asset_certificate_task_items WHERE task_id=?1",[&id],|r|Ok(json!({"total":r.get::<_,i64>(0)?,"captured":r.get::<_,i64>(1)?,"failed":r.get::<_,i64>(2)?,"skipped":r.get::<_,i64>(3)?,"pending":r.get::<_,i64>(4)?,"running":r.get::<_,i64>(5)?,"uniqueFingerprints":r.get::<_,i64>(6)?,"newFingerprints":r.get::<_,i64>(7)?,"attempts":r.get::<_,i64>(8)?}))).map_err(err)?;
            job["progress"] = counts;
            let mut q=c.prepare("SELECT asset_id,endpoint,status,reason,attempts,fingerprint FROM asset_certificate_task_items WHERE task_id=?1 ORDER BY CASE status WHEN 'failed' THEN 0 WHEN 'running' THEN 1 ELSE 2 END,asset_id LIMIT 20").map_err(err)?;
            let items=q.query_map([&id],|r|Ok(json!({"assetId":r.get::<_,i64>(0)?,"endpoint":r.get::<_,String>(1)?,"status":r.get::<_,String>(2)?,"reason":r.get::<_,String>(3)?,"attempts":r.get::<_,i64>(4)?,"fingerprint":r.get::<_,String>(5)?}))).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)?;
            job["items"] = json!(items);
            tasks.push(job);
        }
        Ok(
            json!({"projectId":p,"tasks":tasks,"roots":super::roots(&c,p)?,"networkOperation":"TLS certificate handshake only","providerCreditsConsumed":0,"tlsTrustVerified":false}),
        )
    }
    fn pause(&self, p: i64, id: &str) -> Result<Value, String> {
        let c = db::open(&self.path)?;
        let active = self.active.lock().map_err(err)?;
        let signal = active.get(id);
        let status = if signal.is_some() {
            "pausing"
        } else {
            "paused"
        };
        c.execute("UPDATE asset_certificate_tasks SET status=?3,reason='正在保存当前握手结果；暂停后不领取新资产',updated_at=datetime('now') WHERE id=?1 AND project_id=?2 AND status IN ('running','pausing','paused','interrupted')",params![id,p,status]).map_err(err)?;
        if let Some(stop) = signal {
            stop.store(true, Ordering::SeqCst)
        }
        drop(active);
        self.status(p, Some(id))
    }
    fn resume(&self, p: i64, id: &str, retry: bool) -> Result<Value, String> {
        let mut active = self.active.lock().map_err(err)?;
        if active.contains_key(id) {
            drop(active);
            return self.status(p, Some(id));
        }
        let mut c = db::open(&self.path)?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(err)?;
        let (workers,timeout,status,pid):(i64,i64,String,i32)=tx.query_row("SELECT workers,timeout_seconds,status,owner_pid FROM asset_certificate_tasks WHERE id=?1 AND project_id=?2",params![id,p],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(err)?.ok_or("certificate_task_unavailable")?;
        if ["running", "pausing"].contains(&status.as_str())
            && pid != std::process::id() as i32
            && pid > 0
            && unsafe { libc::kill(pid, 0) } == 0
        {
            return Err("certificate_task_owned_by_live_process".into());
        }
        if retry {
            tx.execute("UPDATE asset_certificate_task_items SET status='pending',reason='' WHERE task_id=?1 AND status='failed'",[id]).map_err(err)?;
        }
        let pending:i64=tx.query_row("SELECT count(*) FROM asset_certificate_task_items WHERE task_id=?1 AND status IN ('pending','running')",[id],|r|r.get(0)).map_err(err)?;
        if pending == 0 {
            tx.execute("UPDATE asset_certificate_tasks SET status='completed',owner_pid=0,reason='本轮快照已无未完成项；失败项可显式重试',updated_at=datetime('now') WHERE id=?1 AND project_id=?2",params![id,p]).map_err(err)?;
            tx.commit().map_err(err)?;
            drop(active);
            return self.status(p, Some(id));
        }
        tx.execute("UPDATE asset_certificate_task_items SET status='pending' WHERE task_id=?1 AND status='running'",[id]).map_err(err)?;
        tx.execute("UPDATE asset_certificate_tasks SET status='running',owner_pid=?3,reason='仅补采缺失证书；逐项持久化',updated_at=datetime('now') WHERE id=?1 AND project_id=?2",params![id,p,std::process::id()]).map_err(err)?;
        tx.commit().map_err(err)?;
        let stop = Arc::new(AtomicBool::new(false));
        active.insert(id.into(), stop.clone());
        drop(active);
        let this = self.clone();
        let task = id.to_owned();
        thread::spawn(move || this.run(p, task, workers as usize, timeout as u64, stop));
        self.status(p, Some(id))
    }
    fn claim(&self, p: i64, id: &str) -> Result<Option<(i64, String, String, Value)>, String> {
        let mut c = db::open(&self.path)?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(err)?;
        let next=tx.query_row("SELECT i.asset_id,i.asset_key,i.endpoint FROM asset_certificate_task_items i JOIN asset_certificate_tasks t ON t.id=i.task_id WHERE i.task_id=?1 AND i.status='pending' AND t.status='running' AND t.project_id=?2 ORDER BY i.asset_id LIMIT 1",params![id,p],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional().map_err(err)?;
        let Some((aid, key, url)) = next else {
            return Ok(None);
        };
        let row=tx.query_row("SELECT a.asset_key,a.host,a.link,a.extra_json FROM assets a JOIN project_assets pa ON pa.asset_id=a.id WHERE a.id=?1 AND pa.project_id=?2 AND pa.is_deleted=0",params![aid,p],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?))).optional().map_err(err)?;
        let mut extra = json!({});
        let mut valid = false;
        if let Some((current, h, l, e)) = row {
            extra = serde_json::from_str(&e).unwrap_or(json!({}));
            valid = super::project(&tx, p).is_ok()
                && extra.is_object()
                && current == key
                && endpoint(&l, &h).is_some_and(|u| u.as_str() == url)
                && approved(&tx, p, &reqwest::Url::parse(&url).map_err(err)?)?
                && !saved_hash(&extra);
        }
        tx.execute("UPDATE asset_certificate_task_items SET status=?3,reason=?4,attempts=attempts+?5,updated_at=datetime('now') WHERE task_id=?1 AND asset_id=?2",params![id,aid,if valid{"running"}else{"skipped"},if valid{""}else{"资产已移除、范围变化或已有指纹"},if valid{1}else{0}]).map_err(err)?;
        tx.commit().map_err(err)?;
        if valid {
            Ok(Some((aid, key, url, extra)))
        } else {
            Ok(Some((aid, key, String::new(), extra)))
        }
    }
    async fn work(&self, p: i64, id: &str, timeout: u64, stop: &AtomicBool) -> Result<(), String> {
        while !stop.load(Ordering::SeqCst) {
            let Some((aid, key, url, extra)) = self.claim(p, id)? else {
                break;
            };
            if url.is_empty() {
                continue;
            }
            let result = match raw(&extra) {
                Some(cert) => Ok(cert),
                None => tls::capture(&url, timeout, false).await,
            };
            self.finish(p, id, aid, &key, &url, result)?;
        }
        Ok(())
    }
    fn finish(
        &self,
        p: i64,
        id: &str,
        aid: i64,
        key: &str,
        url: &str,
        result: Result<crate::certificates::Certificate, String>,
    ) -> Result<(), String> {
        let mut c = db::open(&self.path)?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(err)?;
        let row=tx.query_row("SELECT a.host,a.link,a.extra_json FROM assets a JOIN project_assets pa ON pa.asset_id=a.id WHERE a.id=?1 AND a.asset_key=?2 AND pa.project_id=?3 AND pa.is_deleted=0",params![aid,key,p],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional().map_err(err)?;
        let still_approved = super::project(&tx, p).is_ok()
            && approved(&tx, p, &reqwest::Url::parse(url).map_err(err)?)?;
        let mut status = "skipped";
        let mut reason = "资产或确认范围已变化".to_string();
        let mut hash = String::new();
        let mut added = false;
        if let Some((host, link, old)) = row.filter(|(_, l, _)| {
            endpoint(l, "").is_some_and(|u| u.as_str() == url) && still_approved
        }) {
            match result {
                Ok(cert) => {
                    let mut e: Value =
                        serde_json::from_str(&old).map_err(|_| "certificate_asset_json_invalid")?;
                    if !e.is_object() {
                        return Err("certificate_asset_json_invalid".into());
                    }
                    if saved_hash(&e) {
                        tx.execute("UPDATE asset_certificate_task_items SET status='skipped',reason='其他任务已保存指纹' WHERE task_id=?1 AND asset_id=?2",params![id,aid]).map_err(err)?;
                        tx.commit().map_err(err)?;
                        return Ok(());
                    }
                    // Only certificate facts are merged. Source snapshots, liveness and manual decisions stay untouched.
                    hash = cert.sha256;
                    e["probe_cert_capture"] = json!("captured");
                    e["probe_cert_sha256"] = json!(hash);
                    e["probe_cert_der_base64"] = json!(cert.der_base64);
                    e["probe_cert_dns_names_json"] =
                        json!(serde_json::to_string(&cert.dns_names).map_err(err)?);
                    e["probe_cert_url"] = json!(url);
                    e["probe_cert_tls_verified"] = json!("false");
                    e["probe_cert_task_id"] = json!(id);
                    e["probe_cert_captured_at"] = json!(chrono::Utc::now().to_rfc3339());
                    added=!tx.query_row("SELECT EXISTS(SELECT 1 FROM asset_discovery_seeds WHERE project_id=?1 AND host=?2 AND source='certificate_sha256')",params![p,hash],|r|r.get::<_,bool>(0)).unwrap_or(false);
                    let text = serde_json::to_string(&e).map_err(err)?;
                    tx.execute(
                        "UPDATE assets SET extra_json=?2 WHERE id=?1",
                        params![aid, text],
                    )
                    .map_err(err)?;
                    super::ingest_observations(&tx, p, &[(aid, host, link, text)])?;
                    status = "captured";
                    reason = "已保存叶证书；未确认 TLS 信任或资产归属".into();
                }
                Err(error) => {
                    status = "failed";
                    reason = error;
                }
            }
        }
        tx.execute("UPDATE asset_certificate_task_items SET status=?3,reason=?4,fingerprint=?5,new_fingerprint=?6,updated_at=datetime('now') WHERE task_id=?1 AND asset_id=?2 AND status='running'",params![id,aid,status,reason,hash,added]).map_err(err)?;
        tx.execute(
            "UPDATE asset_certificate_tasks SET updated_at=datetime('now') WHERE id=?1",
            [id],
        )
        .map_err(err)?;
        tx.commit().map_err(err)
    }
    fn run(&self, p: i64, id: String, workers: usize, timeout: u64, stop: Arc<AtomicBool>) {
        let result = (|| -> Result<(), String> {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(err)?;
            let result = rt.block_on(async {
                let mut set = tokio::task::JoinSet::new();
                for _ in 0..workers {
                    let m = self.clone();
                    let id = id.clone();
                    let stop = stop.clone();
                    set.spawn(async move { m.work(p, &id, timeout, &stop).await });
                }
                let mut error = None;
                while let Some(r) = set.join_next().await {
                    match r {
                        Ok(Ok(())) => {}
                        Ok(Err(e)) => {
                            stop.store(true, Ordering::SeqCst);
                            error = Some(e)
                        }
                        Err(e) => {
                            stop.store(true, Ordering::SeqCst);
                            error = Some(e.to_string())
                        }
                    }
                }
                error.map_or(Ok(()), Err)
            });
            rt.shutdown_timeout(Duration::from_millis(100));
            result
        })();
        let mut activity = self.active.lock().ok();
        if let Ok(c) = db::open(&self.path) {
            let remaining=c.query_row("SELECT count(*) FROM asset_certificate_task_items WHERE task_id=?1 AND status IN ('pending','running')",[&id],|r|r.get::<_,i64>(0)).unwrap_or(1);
            let status = if result.is_err() {
                "failed"
            } else if remaining == 0 {
                "completed"
            } else if stop.load(Ordering::SeqCst) {
                "paused"
            } else {
                "completed"
            };
            let _=c.execute("UPDATE asset_certificate_tasks SET status=?2,owner_pid=0,reason=?3,updated_at=datetime('now') WHERE id=?1",params![id,status,result.err().unwrap_or_else(||if status=="completed"{"本轮快照处理完成；查看成功、失败与跳过数量".into()}else{"已暂停，逐项结果已保存，可继续未完成项".into()})]);
        }
        if let Some(ref mut active) = activity {
            active.remove(&id);
        }
    }
    pub(crate) fn shutdown(&self) {
        if let Ok(active) = self.active.lock() {
            for stop in active.values() {
                stop.store(true, Ordering::SeqCst)
            }
        }
    }
}
#[tauri::command]
pub(crate) async fn certificate_task_action(
    state: tauri::State<'_, Manager>,
    project_id: i64,
    action: String,
    args: Value,
) -> Result<Value, String> {
    let manager = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        manager.dispatch(project_id, &action, args, &uuid::Uuid::new_v4().to_string())
    })
    .await
    .map_err(err)?
}

#[cfg(test)]
#[path = "certificate_tasks_tests.rs"]
mod tests;
