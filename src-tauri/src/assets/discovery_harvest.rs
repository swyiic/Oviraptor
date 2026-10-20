//! Durable local-only extraction. No worker executable, network, API key or CAS access.
use super::*;
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Mutex, OnceLock},
};
const BATCH: i64 = 200;
const TABLES:&str="CREATE TABLE IF NOT EXISTS asset_discovery_harvests(id TEXT PRIMARY KEY,project_id INTEGER NOT NULL,owner_pid INTEGER NOT NULL DEFAULT 0,status TEXT NOT NULL,cursor INTEGER NOT NULL DEFAULT 0,total INTEGER NOT NULL DEFAULT 0,processed INTEGER NOT NULL DEFAULT 0,skipped INTEGER NOT NULL DEFAULT 0,added INTEGER NOT NULL DEFAULT 0,certificates INTEGER NOT NULL DEFAULT 0,reason TEXT NOT NULL DEFAULT '',created_at TEXT NOT NULL DEFAULT(datetime('now')),updated_at TEXT NOT NULL DEFAULT(datetime('now')));CREATE INDEX IF NOT EXISTS asset_discovery_harvest_project ON asset_discovery_harvests(project_id,created_at);CREATE TABLE IF NOT EXISTS asset_discovery_harvest_items(run_id TEXT NOT NULL,asset_id INTEGER NOT NULL,PRIMARY KEY(run_id,asset_id));";
static ACTIVE: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
static START: Mutex<()> = Mutex::new(());
pub(super) fn open_existing(path: &Path, write: bool) -> Result<Connection, String> {
    let mode = if write {
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
    } else {
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
    };
    let c = db::open_with_flags(path, mode | rusqlite::OpenFlags::SQLITE_OPEN_FULL_MUTEX)
        .map_err(err)?;
    c.busy_timeout(Duration::from_secs(10)).map_err(err)?;
    c.pragma_update(None, "foreign_keys", "ON").map_err(err)?;
    Ok(c)
}
fn active() -> &'static Mutex<HashSet<String>> {
    ACTIVE.get_or_init(|| Mutex::new(HashSet::new()))
}
fn table_exists(c: &Connection) -> Result<bool, String> {
    c.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='asset_discovery_harvests' AND type='table')",[],|r|r.get(0)).map_err(err)
}
fn pid_alive(pid: i64) -> bool {
    if pid <= 0 {
        return false;
    }
    #[cfg(unix)]
    {
        unsafe {
            libc::kill(pid as i32, 0) == 0
                || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
        }
    }
    #[cfg(not(unix))]
    {
        pid == std::process::id() as i64
    }
}
fn row_on(c: &Connection, id: &str) -> Result<Value, String> {
    c.query_row("SELECT id,project_id,owner_pid,status,cursor,total,processed,skipped,added,certificates,reason,updated_at FROM asset_discovery_harvests WHERE id=?1",[id],|r|Ok(json!({"id":r.get::<_,String>(0)?,"projectId":r.get::<_,i64>(1)?,"ownerPid":r.get::<_,i64>(2)?,"status":r.get::<_,String>(3)?,"cursor":r.get::<_,i64>(4)?,"total":r.get::<_,i64>(5)?,"processed":r.get::<_,i64>(6)?,"skipped":r.get::<_,i64>(7)?,"added":r.get::<_,i64>(8)?,"certificates":r.get::<_,i64>(9)?,"reason":r.get::<_,String>(10)?,"updatedAt":r.get::<_,String>(11)?}))).map_err(err)
}
pub(super) fn status_on(c: &Connection, p: i64) -> Result<Value, String> {
    if !table_exists(c)? {
        return Ok(Value::Null);
    }
    let id:Option<String>=c.query_row("SELECT id FROM asset_discovery_harvests WHERE project_id=?1 ORDER BY rowid DESC LIMIT 1",[p],|r|r.get(0)).optional().map_err(err)?;
    let Some(id) = id else { return Ok(Value::Null) };
    let mut v = row_on(c, &id)?;
    if matches!(v["status"].as_str(), Some("running" | "stopping"))
        && !pid_alive(v["ownerPid"].as_i64().unwrap_or(0))
    {
        v["status"] = json!("interrupted");
        v["reason"] = json!("原进程已退出，已保留提取进度，可继续");
    }
    if let Some(o) = v.as_object_mut() {
        o.remove("ownerPid");
    }
    Ok(v)
}
fn prepare(c: &mut Connection, p: i64) -> Result<(String, bool), String> {
    project(c, p)?;
    ensure(c)?;
    c.execute_batch(TABLES).map_err(err)?;
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    let old:Option<(String,String,i64)>=tx.query_row("SELECT id,status,owner_pid FROM asset_discovery_harvests WHERE project_id=?1 ORDER BY rowid DESC LIMIT 1",[p],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(err)?;
    let id = if let Some((id, status, pid)) = old.filter(|(_, s, _)| s != "completed") {
        let live = active()
            .lock()
            .map_err(|_| "本地提取锁不可用")?
            .contains(&id);
        if matches!(status.as_str(), "running" | "stopping")
            && (live || (pid != std::process::id() as i64 && pid_alive(pid)))
        {
            return Ok((id, false));
        }
        if live {
            return Err("正在完成暂停，请稍后继续".into());
        }
        tx.execute("UPDATE asset_discovery_harvests SET status='running',owner_pid=?2,reason='正在继续本地提取',updated_at=datetime('now') WHERE id=?1",params![id,std::process::id()]).map_err(err)?;
        id
    } else {
        let id = uuid::Uuid::new_v4().to_string();
        tx.execute("INSERT INTO asset_discovery_harvests(id,project_id,owner_pid,status,reason) VALUES(?1,?2,?3,'running','正在分批提取本地证书、主机、页面与 CNAME 线索')",params![id,p,std::process::id()]).map_err(err)?;
        // Freeze associations, not merely the highest global ID. Concurrent additions go into the next explicit run.
        tx.execute("INSERT INTO asset_discovery_harvest_items SELECT ?1,a.id FROM assets a JOIN project_assets pa ON pa.asset_id=a.id WHERE pa.project_id=?2 AND pa.is_deleted=0",params![id,p]).map_err(err)?;
        tx.execute("UPDATE asset_discovery_harvests SET total=(SELECT count(*) FROM asset_discovery_harvest_items WHERE run_id=?1) WHERE id=?1",[&id]).map_err(err)?;
        id
    };
    tx.commit().map_err(err)?;
    Ok((id, true))
}
fn batch(c: &mut Connection, p: i64, id: &str) -> Result<bool, String> {
    project(c, p)?;
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    let state = row_on(&tx, id)?;
    if state["projectId"] != p {
        return Err("本地提取任务不属于当前项目".into());
    }
    if state["status"] == "stopping" {
        tx.execute("UPDATE asset_discovery_harvests SET status='paused',owner_pid=0,reason='已暂停；下次从已保存进度继续',updated_at=datetime('now') WHERE id=?1",[id]).map_err(err)?;
        tx.commit().map_err(err)?;
        return Ok(false);
    }
    if state["status"] != "running" {
        return Ok(false);
    }
    let rows = {
        let mut s=tx.prepare("SELECT i.asset_id,a.host,a.link,a.extra_json,pa.is_deleted FROM asset_discovery_harvest_items i LEFT JOIN assets a ON a.id=i.asset_id LEFT JOIN project_assets pa ON pa.asset_id=i.asset_id AND pa.project_id=?2 WHERE i.run_id=?1 AND i.asset_id>?3 ORDER BY i.asset_id LIMIT ?4").map_err(err)?;
        let rows = s
            .query_map(
                params![id, p, state["cursor"].as_i64().unwrap_or(0), BATCH],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, Option<bool>>(4)?,
                    ))
                },
            )
            .map_err(err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err)?;
        rows
    };
    if rows.is_empty() {
        tx.execute("UPDATE asset_discovery_harvests SET status='completed',owner_pid=0,reason='本轮快照已全部提取；未联网，也未消耗查询积分',updated_at=datetime('now') WHERE id=?1",[id]).map_err(err)?;
        // The completed receipt survives; only the transient queue is reclaimed.
        tx.execute(
            "DELETE FROM asset_discovery_harvest_items WHERE run_id=?1",
            [id],
        )
        .map_err(err)?;
        tx.commit().map_err(err)?;
        return Ok(false);
    }
    let mut valid = vec![];
    let mut certs = 0;
    let mut skipped = 0;
    for (asset_id, h, l, e, deleted) in &rows {
        if *deleted != Some(false) || e.is_none() {
            skipped += 1;
            continue;
        }
        let e = e.as_ref().unwrap();
        if local::certificate_facts(&serde_json::from_str(e).unwrap_or(json!({}))).present() {
            certs += 1;
        }
        valid.push((
            *asset_id,
            h.clone().unwrap_or_default(),
            l.clone().unwrap_or_default(),
            e.clone(),
        ));
    }
    let added = ingest_observations(&tx, p, &valid)?;
    tx.execute("UPDATE asset_discovery_harvests SET cursor=?2,processed=processed+?3,skipped=skipped+?4,added=added+?5,certificates=certificates+?6,updated_at=datetime('now') WHERE id=?1",params![id,rows.last().unwrap().0,rows.len() as i64,skipped,added as i64,certs]).map_err(err)?;
    tx.commit().map_err(err)?;
    Ok(true)
}
struct Guard {
    path: PathBuf,
    id: String,
}
impl Drop for Guard {
    fn drop(&mut self) {
        if let Ok(c) = open_existing(&self.path, true) {
            let _=c.execute("UPDATE asset_discovery_harvests SET status=CASE WHEN status='stopping' THEN 'paused' ELSE 'interrupted' END,owner_pid=0,reason='进度已保存，可继续提取',updated_at=datetime('now') WHERE id=?1 AND status IN ('running','stopping')",[&self.id]);
        }
        if let Ok(mut a) = active().lock() {
            a.remove(&self.id);
        }
    }
}
fn launch(path: PathBuf, p: i64, id: String) -> Result<(), String> {
    if !active()
        .lock()
        .map_err(|_| "本地提取锁不可用")?
        .insert(id.clone())
    {
        return Ok(());
    }
    let guard = Guard {
        path: path.clone(),
        id: id.clone(),
    };
    std::thread::Builder::new().name("asset-local-extraction".into()).spawn(move||{
        let _guard=guard;
        let result=(||{let mut c=open_existing(&path,true)?;while batch(&mut c,p,&id)?{std::thread::sleep(Duration::from_millis(15));}Ok::<_,String>(())})();
        if let Err(e)=result{if let Ok(c)=open_existing(&path,true){let _=c.execute("UPDATE asset_discovery_harvests SET status='failed',owner_pid=0,reason=?2,updated_at=datetime('now') WHERE id=?1",params![id,e]);}}
    }).map_err(|_|"无法启动本地提取线程；进度仍保留")?;
    Ok(())
}
#[tauri::command]
pub async fn start_asset_discovery_harvest(
    app: tauri::AppHandle,
    project_id: i64,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _serial = START.lock().map_err(|_| "本地提取锁不可用")?;
        let path = app.state::<AppState>().db_path.clone();
        let mut c = open_existing(&path, true)?;
        let (id, start) = prepare(&mut c, project_id)?;
        if start {
            launch(path, project_id, id)?;
        }
        status_on(&c, project_id)
    })
    .await
    .map_err(err)?
}
#[tauri::command]
pub async fn pause_asset_discovery_harvest(
    app: tauri::AppHandle,
    project_id: i64,
    run_id: String,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move||{
        let c=open_existing(&app.state::<AppState>().db_path,true)?;project(&c,project_id)?;
        if !table_exists(&c)?{return Err("没有可暂停的本地提取任务".into());}
        let state=row_on(&c,&run_id)?;if state["projectId"]!=project_id{return Err("提取任务与项目不匹配".into());}
        c.execute("UPDATE asset_discovery_harvests SET status='stopping',reason='正在保存本批进度并暂停' WHERE id=?1 AND project_id=?2 AND status='running'",params![run_id,project_id]).map_err(err)?;
        status_on(&c,project_id)
    }).await.map_err(err)?
}
#[cfg(test)]
#[path = "discovery_harvest_tests.rs"]
mod tests;
