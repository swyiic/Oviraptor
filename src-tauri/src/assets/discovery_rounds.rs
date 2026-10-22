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

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE projects(id INTEGER PRIMARY KEY,status TEXT);INSERT INTO projects VALUES(1,'active'),(2,'active');CREATE TABLE project_assets(project_id INTEGER);CREATE TABLE asset_discovery_pipelines(project_id INTEGER PRIMARY KEY,operation_id TEXT,round INTEGER,baseline_assets INTEGER);INSERT INTO asset_discovery_pipelines VALUES(1,'operation-a',27,0),(2,'operation-b',1,0);").unwrap();
        c
    }
    #[test]
    fn legacy_counter_is_not_history_and_pause_resume_never_increments() {
        let c = fixture();
        assert_eq!(start(&c, 1).unwrap(), 1);
        pause(&c, 1).unwrap();
        assert_eq!(start(&c, 1).unwrap(), 1);
        assert_eq!(history(&c, 1, 1, 20).unwrap()["total"], 1);
        assert_eq!(start(&c, 2).unwrap(), 1);
    }
    #[test]
    fn completed_round_only_advances_for_new_evidence_and_survives_new_operation() {
        let c = fixture();
        start(&c, 1).unwrap();
        complete(&c, 1, "settled").unwrap();
        assert_eq!(start(&c, 1).unwrap(), 1);
        assert_eq!(new_evidence(&c, 1, "new TLS").unwrap(), 2);
        assert_eq!(new_evidence(&c, 1, "same active pass").unwrap(), 2);
        c.execute("UPDATE asset_discovery_pipelines SET operation_id='operation-c',round=1 WHERE project_id=1",[]).unwrap();
        assert_eq!(start(&c, 1).unwrap(), 3);
        let h = history(&c, 1, 1, 20).unwrap();
        assert_eq!(h["total"], 3);
        assert_eq!(h["items"][1]["state"], "superseded");
        assert!(history(&c, 2, 1, 20).unwrap()["items"]
            .as_array()
            .unwrap()
            .is_empty());
    }
    #[test]
    fn wave_is_immutable_and_multiple_queries_are_not_multiple_rounds() {
        let c = fixture();
        start(&c, 1).unwrap();
        let q = |host: &str| hunter_expansion::ExpansionInput {
            project_id: 1,
            kind: "domain_tree".into(),
            target: host.into(),
        };
        save_wave(&c, 1, &[q("a.example"), q("b.example")]).unwrap();
        save_wave(&c, 1, &[q("changed.example")]).unwrap();
        assert_eq!(wave(&c, 1).unwrap().unwrap().len(), 2);
        assert_eq!(start(&c, 1).unwrap(), 1);
        assert_eq!(next_wave(&c, 1, "new seed").unwrap(), 2);
        assert!(wave(&c, 1).unwrap().is_none());
    }
    #[test]
    fn history_is_bounded_read_only_and_cross_project() {
        let c = fixture();
        assert!(history(&c, 1, 0, 10).is_err());
        assert!(history(&c, 1, 1, 31).is_err());
        assert!(!exists(&c).unwrap());
        assert_eq!(history(&c, 1, 1, 10).unwrap()["total"], 0);
        assert!(!exists(&c).unwrap());
    }
    #[test]
    fn archived_round_history_is_readable_but_missing_projects_are_rejected() {
        let c = fixture();
        start(&c, 1).unwrap();
        c.execute("UPDATE projects SET status='archived' WHERE id=1", []).unwrap();
        assert_eq!(history(&c, 1, 1, 10).unwrap()["total"], 1);
        assert!(project(&c, 1).is_err(), "execution admission must remain closed for archived projects");
        assert!(history(&c, 99, 1, 10).is_err());
        assert!(history(&c, 0, 1, 10).is_err());
    }
}
