//! Prospective project-wide discovery rounds. Legacy query counters are never
//! relabelled as discovery history, and pause/resume keeps the original round.
use super::*;

pub(crate) fn ensure(c: &Connection) -> Result<(), String> {
    c.execute_batch("CREATE TABLE IF NOT EXISTS asset_discovery_pipeline_rounds(project_id INTEGER NOT NULL,number INTEGER NOT NULL,operation_id TEXT NOT NULL,state TEXT NOT NULL DEFAULT 'active',reason TEXT NOT NULL DEFAULT '',started_at TEXT NOT NULL DEFAULT(datetime('now')),completed_at TEXT,wave_json TEXT NOT NULL DEFAULT 'null',PRIMARY KEY(project_id,number));CREATE INDEX IF NOT EXISTS asset_discovery_round_operation ON asset_discovery_pipeline_rounds(project_id,operation_id,number);").map_err(err)
}
fn exists(c: &Connection) -> Result<bool, String> {
    c.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='asset_discovery_pipeline_rounds')",
        [],
        |r| r.get(0),
    )
    .map_err(err)
}
/// Bind a prospective round before executing work. A known completed round is
/// retained until genuinely new evidence or a fresh collection starts a wave.
pub(crate) fn start(c: &Connection, p: i64) -> Result<i64, String> {
    ensure(c)?;
    let tx = rusqlite::Transaction::new_unchecked(c, rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    let (op, current): (String, i64) = tx
        .query_row(
            "SELECT operation_id,round FROM asset_discovery_pipelines WHERE project_id=?1",
            [p],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(err)?;
    let saved:Option<String>=tx.query_row("SELECT state FROM asset_discovery_pipeline_rounds WHERE project_id=?1 AND number=?2 AND operation_id=?3",params![p,current,op],|r|r.get(0)).optional().map_err(err)?;
    let number = if let Some(state) = saved {
        if state == "paused" || state == "blocked" {
            tx.execute("UPDATE asset_discovery_pipeline_rounds SET state='active' WHERE project_id=?1 AND number=?2",params![p,current]).map_err(err)?;
        }
        current
    } else {
        tx.execute("UPDATE asset_discovery_pipeline_rounds SET state='superseded',completed_at=COALESCE(completed_at,datetime('now')),reason='目标或采集意图已切换；历史与回执保留' WHERE project_id=?1 AND state IN ('active','paused','blocked')",[p]).map_err(err)?;
        allocate(&tx, p, &op, "开始增量发现")?
    };
    tx.commit().map_err(err)?;
    Ok(number)
}
fn allocate(c: &Connection, p: i64, op: &str, reason: &str) -> Result<i64, String> {
    let number:i64=c.query_row("SELECT COALESCE(MAX(number),0)+1 FROM asset_discovery_pipeline_rounds WHERE project_id=?1",[p],|r|r.get(0)).map_err(err)?;
    c.execute("INSERT INTO asset_discovery_pipeline_rounds(project_id,number,operation_id,reason) VALUES(?1,?2,?3,?4)",params![p,number,op,reason]).map_err(err)?;
    c.execute("UPDATE asset_discovery_pipelines SET round=?2,baseline_assets=(SELECT COUNT(*) FROM project_assets WHERE project_id=?1) WHERE project_id=?1",params![p,number]).map_err(err)?;
    Ok(number)
}
pub(crate) fn new_evidence(c: &Connection, p: i64, reason: &str) -> Result<i64, String> {
    let owned:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM asset_discovery_pipelines WHERE project_id=?1)",[p],|r|r.get(0)).map_err(err)?;
    // Older Assist-only tasks have no pipeline operation. Keep their original
    // continuation intact; prospective round tracking starts with a real owner.
    if !owned{return Ok(0)}
    let current = start(c, p)?;
    let tx = rusqlite::Transaction::new_unchecked(c, rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    let(op,state):(String,String)=tx.query_row("SELECT operation_id,state FROM asset_discovery_pipeline_rounds WHERE project_id=?1 AND number=?2",params![p,current],|r|Ok((r.get(0)?,r.get(1)?))).map_err(err)?;
    let number = if state == "completed" {
        allocate(&tx, p, &op, reason)?
    } else {
        current
    };
    tx.commit().map_err(err)?;
    Ok(number)
}
pub(crate) fn next_wave(c: &Connection, p: i64, reason: &str) -> Result<i64, String> {
    complete(c, p, reason)?;
    new_evidence(c, p, reason)
}
pub(crate) fn complete(c: &Connection, p: i64, reason: &str) -> Result<(), String> {
    c.execute("UPDATE asset_discovery_pipeline_rounds SET state='completed',completed_at=COALESCE(completed_at,datetime('now')),reason=?2 WHERE project_id=?1 AND number=(SELECT round FROM asset_discovery_pipelines WHERE project_id=?1) AND operation_id=(SELECT operation_id FROM asset_discovery_pipelines WHERE project_id=?1) AND state IN ('active','blocked')",params![p,reason]).map_err(err)?;
    Ok(())
}
pub(crate) fn pause(c: &Connection, p: i64) -> Result<(), String> {
    c.execute("UPDATE asset_discovery_pipeline_rounds SET state='paused' WHERE project_id=?1 AND number=(SELECT round FROM asset_discovery_pipelines WHERE project_id=?1) AND state IN ('active','blocked')",[p]).map_err(err)?;
    Ok(())
}
pub(crate) fn wave(
    c: &Connection,
    p: i64,
) -> Result<Option<Vec<hunter_expansion::ExpansionInput>>, String> {
    let raw:String=c.query_row("SELECT wave_json FROM asset_discovery_pipeline_rounds WHERE project_id=?1 AND number=(SELECT round FROM asset_discovery_pipelines WHERE project_id=?1)",[p],|r|r.get(0)).map_err(err)?;
    if raw == "null" {
        Ok(None)
    } else {
        serde_json::from_str(&raw).map(Some).map_err(err)
    }
}
pub(crate) fn save_wave(
    c: &Connection,
    p: i64,
    items: &[hunter_expansion::ExpansionInput],
) -> Result<(), String> {
    c.execute("UPDATE asset_discovery_pipeline_rounds SET wave_json=?2 WHERE project_id=?1 AND number=(SELECT round FROM asset_discovery_pipelines WHERE project_id=?1) AND wave_json='null'",params![p,serde_json::to_string(items).map_err(err)?]).map_err(err)?;
    Ok(())
}
/// Historical reads and local derived indexes remain available after archive.
/// This checks existence only and is never used to admit discovery execution.
pub(crate) fn read_project(c: &Connection, p: i64) -> Result<(), String> {
    if p < 1 || !c.query_row("SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1)", [p], |r| r.get::<_, bool>(0)).map_err(err)? {
        return Err("种子历史项目不存在".into());
    }
    Ok(())
}
pub(crate) fn history(c: &Connection, p: i64, page: i64, page_size: i64) -> Result<Value, String> {
    read_project(c, p)?;
    if page < 1 || !(1..=30).contains(&page_size) || page > 1_000_000 {
        return Err("种子轮次分页参数无效".into());
    }
    if !exists(c)? {
        return Ok(
            json!({"items":[],"total":0,"page":page,"pageSize":page_size,"historyBeforeTracking":"unknown"}),
        );
    }
    let total: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM asset_discovery_pipeline_rounds WHERE project_id=?1",
            [p],
            |r| r.get(0),
        )
        .map_err(err)?;
    let mut q=c.prepare("SELECT number,state,reason,started_at,completed_at,operation_id,json_array_length(wave_json) FROM asset_discovery_pipeline_rounds WHERE project_id=?1 ORDER BY number DESC LIMIT ?2 OFFSET ?3").map_err(err)?;
    let items=q.query_map(params![p,page_size,(page-1)*page_size],|r|Ok(json!({"number":r.get::<_,i64>(0)?,"state":r.get::<_,String>(1)?,"reason":r.get::<_,String>(2)?,"startedAt":r.get::<_,String>(3)?,"completedAt":r.get::<_,Option<String>>(4)?,"operationId":r.get::<_,String>(5)?,"plannedQueries":r.get::<_,Option<i64>>(6)?}))).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)?;
    Ok(
        json!({"items":items,"total":total,"page":page,"pageSize":page_size,"historyBeforeTracking":"unknown"}),
    )
}

