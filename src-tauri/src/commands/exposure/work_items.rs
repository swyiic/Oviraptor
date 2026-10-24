// §Q3 — durable work items for one exposure run. Every planned object gets a row before
// any network call, so "what is left" is a query rather than a guess, and a claim carries
// an owner plus a version so a worker that lost its lease cannot write a terminal state.

use super::state::{shift_seconds_back, utc_now, WorkItemCounts, WorkItemStatus};
use super::transaction::ImmediateTx;
use rusqlite::{params, Connection, OptionalExtension};

/// Every passive send refreshes the claim immediately before opening its socket.
/// The lease must exceed the largest valid total HTTP deadline, leaving time to
/// classify and commit the bounded response without a second executor taking over.
pub const CLAIM_COMMIT_MARGIN_SECONDS: i64 = 60;
pub const DEFAULT_LEASE_SECONDS: i64 = super::settings::TIMEOUT_SECONDS_LIMIT + CLAIM_COMMIT_MARGIN_SECONDS;

#[derive(Clone, Debug)]
pub struct WorkItemSeed {
    pub work_key: String,
    pub kind: String,
    pub source_key: String,
    pub asset_id: Option<i64>,
    pub origin: String,
    pub entity: String,
    pub input_hash: String,
}

#[derive(Clone, Debug)]
pub struct WorkItem {
    pub id: i64,
    pub run_id: i64,
    pub project_id: i64,
    pub work_key: String,
    pub kind: String,
    pub source_key: String,
    pub asset_id: Option<i64>,
    pub origin: String,
    pub entity: String,
    pub input_hash: String,
    pub status: WorkItemStatus,
    pub owner: String,
    pub claim_version: i64,
    pub attempts: i64,
    pub observations: i64,
    pub requests: i64,
    pub bytes: i64,
}

const ITEM_COLUMNS: &str = "id,run_id,project_id,work_key,kind,source_key,asset_id,origin,entity,input_hash,status,owner,claim_version,attempts,observations,requests,bytes";

fn item_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkItem> {
    let status: String = row.get(10)?;
    Ok(WorkItem {
        id: row.get(0)?,
        run_id: row.get(1)?,
        project_id: row.get(2)?,
        work_key: row.get(3)?,
        kind: row.get(4)?,
        source_key: row.get(5)?,
        asset_id: row.get(6)?,
        origin: row.get(7)?,
        entity: row.get(8)?,
        input_hash: row.get(9)?,
        status: WorkItemStatus::parse(&status).unwrap_or(WorkItemStatus::Pending),
        owner: row.get(11)?,
        claim_version: row.get(12)?,
        attempts: row.get(13)?,
        observations: row.get(14)?,
        requests: row.get(15)?,
        bytes: row.get(16)?,
    })
}

/// Writes the frozen plan. Re-running with the same keys is idempotent: a resumed run does
/// not invent a second work item for the same object.
pub fn plan(
    connection: &Connection,
    run_id: i64,
    project_id: i64,
    attempt_number: i64,
    seeds: &[WorkItemSeed],
) -> Result<usize, String> {
    for seed in seeds {
        if seed.work_key.trim().is_empty() || seed.kind.trim().is_empty() {
            return Err("工作项缺少稳定 key 或类型，拒绝写入计划".to_string());
        }
        connection
            .execute(
                "INSERT INTO exposure_work_items(run_id,project_id,attempt_number,work_key,kind,source_key,asset_id,origin,entity,input_hash,status)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'pending')
                 ON CONFLICT(run_id,work_key) DO NOTHING",
                params![
                    run_id,
                    project_id,
                    attempt_number,
                    seed.work_key,
                    seed.kind,
                    seed.source_key,
                    seed.asset_id,
                    seed.origin,
                    seed.entity,
                    seed.input_hash
                ],
            )
            .map_err(|error| format!("工作项计划写入失败：{error}"))?;
    }
    Ok(seeds.len())
}

/// Claims the next unfinished item for `owner`, taking over claims whose lease expired.
/// The candidate row and the claim update are in one transaction so two workers cannot both
/// believe they own the same object.
pub fn claim_next(
    connection: &rusqlite::Connection,
    run_id: i64,
    owner: &str,
    lease_seconds: i64,
) -> Result<Option<WorkItem>, String> {
    reclaim_expired(connection, run_id, lease_seconds)?;
    // Immediate so two concurrent claimants serialize instead of both believing they won.
    let transaction = ImmediateTx::begin(connection)?;
    loop {
        let candidate: Option<i64> = transaction
            .connection()
            .query_row(
                "SELECT id FROM exposure_work_items
                 WHERE run_id=?1 AND status IN ('pending','orphaned')
                 ORDER BY CASE WHEN cursor='' THEN 0 ELSE attempts END,work_key LIMIT 1",
                [run_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let Some(id) = candidate else {
            transaction.commit()?;
            return Ok(None);
        };
        let now = utc_now();
        let updated = transaction
            .connection()
            .execute(
                "UPDATE exposure_work_items
                 SET status='claimed', owner=?2, claim_version=claim_version+1,
                     claimed_at=?3, heartbeat_at=?3, attempts=attempts+1, updated_at=?3
                 WHERE id=?1 AND status IN ('pending','orphaned')",
                params![id, owner, now],
            )
            .map_err(|error| error.to_string())?;
        if updated == 1 {
            let item: Option<WorkItem> = transaction
                .connection()
                .query_row(
                    &format!("SELECT {ITEM_COLUMNS} FROM exposure_work_items WHERE id=?1"),
                    [id],
                    item_from_row,
                )
                .optional()
                .map_err(|error| error.to_string())?;
            transaction.commit()?;
            return Ok(item);
        }
    }
}

pub fn heartbeat(
    connection: &rusqlite::Connection,
    id: i64,
    owner: &str,
    claim_version: i64,
) -> Result<bool, String> {
    let now = utc_now();
    let updated = connection
        .execute(
            "UPDATE exposure_work_items SET heartbeat_at=?4, status='in_progress'
             WHERE id=?1 AND owner=?2 AND claim_version=?3 AND status IN ('claimed','in_progress')",
            params![id, owner, claim_version, now],
        )
        .map_err(|error| error.to_string())?;
    Ok(updated == 1)
}

/// Terminal transition, guarded by owner and claim version. Returns false when the caller
/// no longer holds the claim, which is how a late worker is refused after a takeover.
#[allow(clippy::too_many_arguments)]
pub fn finish(
    connection: &rusqlite::Connection,
    id: i64,
    owner: &str,
    claim_version: i64,
    status: WorkItemStatus,
    reason: &str,
    observations: i64,
    requests: i64,
    bytes: i64,
) -> Result<bool, String> {
    if !status.is_terminal() {
        return Err("非终态不能作为工作项收尾写入".to_string());
    }
    let now = utc_now();
    let updated = connection
        .execute(
            "UPDATE exposure_work_items
             SET status=?4, reason=?5, observations=observations+?6, requests=requests+?7,
                 bytes=bytes+?8, finished_at=?3, heartbeat_at=?3, updated_at=?3
             WHERE id=?1 AND owner=?2 AND claim_version=?9 AND status IN ('claimed','in_progress')",
            params![
                id,
                owner,
                now,
                status.as_str(),
                reason,
                observations,
                requests,
                bytes,
                claim_version
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(updated == 1)
}

/// Drops claims whose owner stopped beating, without touching fresh ones.
pub fn reclaim_expired(
    connection: &rusqlite::Connection,
    run_id: i64,
    lease_seconds: i64,
) -> Result<i64, String> {
    let cutoff = shift_seconds_back(lease_seconds);
    let updated = connection
        .execute(
            "UPDATE exposure_work_items SET status='orphaned', reason='lease_expired', updated_at=?3
             WHERE run_id=?1 AND status IN ('claimed','in_progress')
               AND claimed_at<>'' AND (heartbeat_at='' OR heartbeat_at<?2)",
            params![run_id, cutoff, utc_now()],
        )
        .map_err(|error| error.to_string())?;
    Ok(updated as i64)
}

pub fn counts(connection: &rusqlite::Connection, run_id: i64) -> Result<WorkItemCounts, String> {
    let mut statement = connection
        .prepare("SELECT status,COUNT(*) FROM exposure_work_items WHERE run_id=?1 GROUP BY status")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([run_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|error| error.to_string())?;
    let mut counts = WorkItemCounts::default();
    for row in rows.flatten() {
        let Some(status) = WorkItemStatus::parse(&row.0) else {
            // Corrupt vocabulary is counted as open: it must never disappear from coverage.
            counts.open += row.1;
            counts.total += row.1;
            continue;
        };
        counts.total += row.1;
        if status.is_terminal() {
            counts.terminal += row.1;
        } else {
            counts.open += row.1;
        }
        match status {
            WorkItemStatus::Completed => counts.completed += row.1,
            WorkItemStatus::Failed => counts.failed += row.1,
            WorkItemStatus::Cancelled => counts.cancelled += row.1,
            WorkItemStatus::SkippedUnconfigured => counts.unconfigured += row.1,
            WorkItemStatus::BlockedQuota => counts.quota += row.1,
            WorkItemStatus::BlockedAuthorization => counts.authorization += row.1,
            WorkItemStatus::Orphaned => counts.orphaned += row.1,
            _ => {}
        }
    }
    Ok(counts)
}

/// The named leftovers, so `partial` can point at objects instead of an error string.
pub fn enumerate_gaps(
    connection: &rusqlite::Connection,
    run_id: i64,
    limit: i64,
) -> Result<Vec<(String, String, String)>, String> {
    let mut statement = connection
        .prepare(
            "SELECT work_key,status,reason FROM exposure_work_items
             WHERE run_id=?1 AND status<>'completed' ORDER BY work_key LIMIT ?2",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![run_id, limit], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .map_err(|error| error.to_string())?;
    Ok(rows.flatten().collect())
}

/// The per-item counters that a resumed round must keep charging against.
#[derive(Clone, Debug, Default)]
pub struct RunUsage {
    pub planned: i64,
    pub requests_used: i64,
    pub bytes_used: i64,
    pub max_requests: i64,
    pub max_bytes: i64,
}

pub fn record_usage(
    connection: &rusqlite::Connection,
    run_id: i64,
    requests: i64,
    bytes: i64,
) -> Result<(), String> {
    connection
        .execute(
            "UPDATE exposure_run_plans SET requests_used=requests_used+?2, bytes_used=bytes_used+?3 WHERE run_id=?1",
            params![run_id, requests, bytes],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub fn usage(connection: &rusqlite::Connection, run_id: i64) -> Result<Option<RunUsage>, String> {
    let mut statement = connection
        .prepare(
            "SELECT planned_items,requests_used,bytes_used,budget_requests,budget_bytes
             FROM exposure_run_plans WHERE run_id=?1",
        )
        .map_err(|error| error.to_string())?;
    let row = statement
        .query_map([run_id], |row| {
            Ok(RunUsage {
                planned: row.get(0)?,
                requests_used: row.get(1)?,
                bytes_used: row.get(2)?,
                max_requests: row.get(3)?,
                max_bytes: row.get(4)?,
            })
        })
        .map_err(|error| error.to_string())?
        .flatten()
        .next();
    Ok(row)
}

pub fn item_status(connection: &rusqlite::Connection, id: i64) -> Result<(String, String), String> {
    connection
        .query_row(
            "SELECT status,reason FROM exposure_work_items WHERE id=?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map(|value| value.unwrap_or_default())
        .map_err(|error| error.to_string())
}

/// Cursor reads are tied to the current claim. A worker that lost its lease cannot load
/// a replacement worker's frontier and then append observations against that newer state.
pub fn load_claim_cursor(connection: &Connection, item: &WorkItem) -> Result<String, String> {
    connection.query_row(
        "SELECT cursor FROM exposure_work_items WHERE id=?1 AND run_id=?2 AND project_id=?3 AND owner=?4 AND claim_version=?5 AND status IN ('claimed','in_progress')",
        params![item.id,item.run_id,item.project_id,item.owner,item.claim_version],
        |row| row.get(0),
    ).optional().map_err(|error| error.to_string())?
        .ok_or_else(|| "[exposure_claim_lost] 工作项租约已更换，拒绝读取断点".to_string())
}

/// Save the exact URL about to leave the process before opening any socket. The caller
/// must not send if this returns false. A later crash keeps this URL marked unknown instead
/// of assuming that the absence of a saved response means no request was sent.
pub fn save_cursor(connection: &Connection, item: &WorkItem, cursor: &str) -> Result<bool, String> {
    let now = utc_now();
    connection.execute(
        "UPDATE exposure_work_items SET cursor=?6,status='in_progress',heartbeat_at=?7,updated_at=?7 WHERE id=?1 AND run_id=?2 AND project_id=?3 AND owner=?4 AND claim_version=?5 AND status IN ('claimed','in_progress')",
        params![item.id,item.run_id,item.project_id,item.owner,item.claim_version,cursor,now],
    ).map(|updated| updated==1).map_err(|error| error.to_string())
}

pub fn owns_claim(connection: &Connection, item: &WorkItem) -> Result<bool, String> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM exposure_work_items WHERE id=?1 AND run_id=?2 AND project_id=?3 AND owner=?4 AND claim_version=?5 AND status IN ('claimed','in_progress'))",
        params![item.id,item.run_id,item.project_id,item.owner,item.claim_version],
        |row| row.get(0),
    ).map_err(|error| error.to_string())
}

/// Existing immutable observations are authoritative when an older crash happened after
/// writing a response. Only this run/project/work item is consulted; history from another
/// project, origin, or round cannot silently skip a planned request.
pub fn saved_urls(connection: &Connection, item: &WorkItem) -> Result<Vec<String>, String> {
    let mut statement = connection.prepare(
        "SELECT DISTINCT url FROM exposure_observations WHERE run_id=?1 AND project_id=?2 AND work_item_id=?3 ORDER BY url",
    ).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![item.run_id, item.project_id, item.id], |row| {
            row.get(0)
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<String>, _>>()
        .map_err(|error| error.to_string())
}

/// Called inside the orchestrator's evidence transaction: observations and cursor either
/// commit together or neither does. A batch yield becomes Pending and releases its owner;
/// the stable work row, immutable evidence, and all usage survive the next claim.
#[allow(clippy::too_many_arguments)]
pub fn commit_progress(
    connection: &Connection,
    item: &WorkItem,
    status: WorkItemStatus,
    reason: &str,
    cursor: &str,
    observations: i64,
    requests: i64,
    bytes: i64,
) -> Result<bool, String> {
    if status != WorkItemStatus::Pending && !status.is_terminal() {
        return Err("[exposure_progress_status_invalid] 续采仅能保存待续或明确终态".to_string());
    }
    if observations < 0 || requests < 0 || bytes < 0 {
        return Err("[exposure_progress_usage_invalid] 续采增量不能为负".to_string());
    }
    let now = utc_now();
    connection.execute(
        "UPDATE exposure_work_items SET status=?6,reason=?7,cursor=?8,observations=observations+?9,requests=requests+?10,bytes=bytes+?11,heartbeat_at=?12,updated_at=?12,finished_at=CASE WHEN ?6='pending' THEN '' ELSE ?12 END,owner=CASE WHEN ?6='pending' THEN '' ELSE owner END WHERE id=?1 AND run_id=?2 AND project_id=?3 AND owner=?4 AND claim_version=?5 AND status IN ('claimed','in_progress')",
        params![item.id,item.run_id,item.project_id,item.owner,item.claim_version,status.as_str(),reason,cursor,observations,requests,bytes,now],
    ).map(|updated| updated==1).map_err(|error| error.to_string())
}

pub fn load_cursor(connection: &Connection, item_id: i64) -> Result<String, String> {
    connection
        .query_row(
            "SELECT cursor FROM exposure_work_items WHERE id=?1",
            [item_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "[exposure_work_item_missing] 工作项不存在，无法读取断点".to_string())
}

pub fn read_run_items(
    connection: &Connection,
    run_id: i64,
    project_id: i64,
) -> Result<Vec<WorkItem>, String> {
    let mut statement=connection.prepare(&format!("SELECT {ITEM_COLUMNS} FROM exposure_work_items WHERE run_id=?1 AND project_id=?2 ORDER BY work_key"))
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![run_id, project_id], item_from_row)
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod checkpoint_progress_tests {
    use super::*;

    fn database() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE exposure_work_items (
            id INTEGER PRIMARY KEY,run_id INTEGER NOT NULL,project_id INTEGER NOT NULL,
            attempt_number INTEGER NOT NULL DEFAULT 1,work_key TEXT NOT NULL,kind TEXT NOT NULL,
            source_key TEXT NOT NULL DEFAULT '',asset_id INTEGER,origin TEXT NOT NULL DEFAULT '',entity TEXT NOT NULL DEFAULT '',input_hash TEXT NOT NULL DEFAULT '',
            status TEXT NOT NULL DEFAULT 'pending',reason TEXT NOT NULL DEFAULT '',owner TEXT NOT NULL DEFAULT '',claim_version INTEGER NOT NULL DEFAULT 0,
            claimed_at TEXT NOT NULL DEFAULT '',heartbeat_at TEXT NOT NULL DEFAULT '',cursor TEXT NOT NULL DEFAULT '',attempts INTEGER NOT NULL DEFAULT 0,
            observations INTEGER NOT NULL DEFAULT 0,requests INTEGER NOT NULL DEFAULT 0,bytes INTEGER NOT NULL DEFAULT 0,
            finished_at TEXT NOT NULL DEFAULT '',created_at TEXT NOT NULL DEFAULT '',updated_at TEXT NOT NULL DEFAULT '',UNIQUE(run_id,work_key));
            CREATE TABLE exposure_observations(id INTEGER PRIMARY KEY,run_id INTEGER,project_id INTEGER,work_item_id INTEGER,url TEXT);").unwrap();
        for key in ["a", "b"] {
            plan(
                &connection,
                1,
                2,
                1,
                &[WorkItemSeed {
                    work_key: key.into(),
                    kind: "passive_web".into(),
                    source_key: "passive_web".into(),
                    asset_id: None,
                    origin: format!("https://{key}.example"),
                    entity: key.into(),
                    input_hash: key.into(),
                }],
            )
            .unwrap();
        }
        connection
    }

    #[test]
    fn checkpoint_progress_requeues_fairly_and_retains_usage_cursor_and_claim_version() {
        let connection = database();
        let first = claim_next(&connection, 1, "one", 90).unwrap().unwrap();
        assert_eq!(first.work_key, "a");
        assert!(save_cursor(&connection, &first, "durable-inflight").unwrap());
        let tx = ImmediateTx::begin(&connection).unwrap();
        assert!(owns_claim(tx.connection(), &first).unwrap());
        assert!(commit_progress(
            tx.connection(),
            &first,
            WorkItemStatus::Pending,
            "",
            "durable-next",
            1,
            2,
            13
        )
        .unwrap());
        tx.commit().unwrap();
        assert_eq!(load_cursor(&connection, first.id).unwrap(), "durable-next");
        assert!(!owns_claim(&connection, &first).unwrap());
        let second = claim_next(&connection, 1, "two", 90).unwrap().unwrap();
        assert_eq!(
            second.work_key, "b",
            "未开始的origin必须先于已经处理一片的origin"
        );
        finish(
            &connection,
            second.id,
            "two",
            second.claim_version,
            WorkItemStatus::Completed,
            "",
            0,
            0,
            0,
        )
        .unwrap();
        let resumed = claim_next(&connection, 1, "three", 90).unwrap().unwrap();
        assert_eq!(resumed.work_key, "a");
        assert!(resumed.claim_version > first.claim_version);
        assert_eq!(
            (resumed.observations, resumed.requests, resumed.bytes),
            (1, 2, 13)
        );
        assert_eq!(
            load_claim_cursor(&connection, &resumed).unwrap(),
            "durable-next"
        );
        assert!(!save_cursor(&connection, &first, "stale-overwrite").unwrap());
        assert!(!commit_progress(
            &connection,
            &first,
            WorkItemStatus::Completed,
            "",
            "stale",
            0,
            0,
            0
        )
        .unwrap());
        assert_eq!(load_cursor(&connection, first.id).unwrap(), "durable-next");
    }

    #[test]
    fn checkpoint_progress_and_evidence_roll_back_together() {
        let connection = database();
        let item = claim_next(&connection, 1, "one", 90).unwrap().unwrap();
        assert!(save_cursor(&connection, &item, "before-send").unwrap());
        {
            let tx = ImmediateTx::begin(&connection).unwrap();
            tx.connection().execute("INSERT INTO exposure_observations(run_id,project_id,work_item_id,url) VALUES(1,2,?1,'https://a.example/')",[item.id]).unwrap();
            assert!(commit_progress(
                tx.connection(),
                &item,
                WorkItemStatus::Pending,
                "",
                "after-response",
                1,
                1,
                42
            )
            .unwrap());
            // Deliberately no commit: a later classifier failure cannot leave a new cursor
            // while the matching observation was rolled back.
        }
        assert_eq!(
            load_claim_cursor(&connection, &item).unwrap(),
            "before-send"
        );
        assert!(saved_urls(&connection, &item).unwrap().is_empty());
        let current = read_run_items(&connection, 1, 2).unwrap();
        assert_eq!(
            (
                current[0].observations,
                current[0].requests,
                current[0].bytes
            ),
            (0, 0, 0)
        );
    }

    #[test]
    fn checkpoint_saved_urls_are_bound_to_run_project_and_item() {
        let connection = database();
        let item = claim_next(&connection, 1, "one", 90).unwrap().unwrap();
        connection.execute("INSERT INTO exposure_observations(run_id,project_id,work_item_id,url) VALUES(1,2,?1,'https://a.example/saved')",[item.id]).unwrap();
        connection.execute("INSERT INTO exposure_observations(run_id,project_id,work_item_id,url) VALUES(2,2,?1,'https://a.example/foreign-run')",[item.id]).unwrap();
        connection.execute("INSERT INTO exposure_observations(run_id,project_id,work_item_id,url) VALUES(1,3,?1,'https://a.example/foreign-project')",[item.id]).unwrap();
        connection.execute("INSERT INTO exposure_observations(run_id,project_id,work_item_id,url) VALUES(1,2,999,'https://a.example/foreign-item')",[]).unwrap();
        assert_eq!(
            saved_urls(&connection, &item).unwrap(),
            vec!["https://a.example/saved"]
        );
        assert!(read_run_items(&connection, 1, 3).unwrap().is_empty());
    }

    #[test]
    fn default_lease_covers_maximum_http_deadline_for_collection_and_recovery() {
        assert_eq!(DEFAULT_LEASE_SECONDS, super::super::settings::TIMEOUT_SECONDS_LIMIT + CLAIM_COMMIT_MARGIN_SECONDS);
        assert_eq!(super::super::recovery::default_lease(), DEFAULT_LEASE_SECONDS);
        let connection = database();
        let item = claim_next(&connection, 1, "bounded-slow-request", DEFAULT_LEASE_SECONDS).unwrap().unwrap();
        for age in [91, super::super::settings::TIMEOUT_SECONDS_LIMIT + 1, DEFAULT_LEASE_SECONDS - 5] {
            let heartbeat = shift_seconds_back(age);
            connection.execute("UPDATE exposure_work_items SET heartbeat_at=?2 WHERE id=?1", params![item.id, heartbeat]).unwrap();
            assert_eq!(super::super::recovery::live_claims(&connection, 1, DEFAULT_LEASE_SECONDS).unwrap(), 1, "age {age}: slow but valid request must remain live");
            assert_eq!(reclaim_expired(&connection, 1, DEFAULT_LEASE_SECONDS).unwrap(), 0, "age {age}: collector cannot steal the request");
            assert!(owns_claim(&connection, &item).unwrap());
        }
        connection.execute("UPDATE exposure_work_items SET heartbeat_at=?2 WHERE id=?1", params![item.id, shift_seconds_back(DEFAULT_LEASE_SECONDS + 5)]).unwrap();
        assert_eq!(super::super::recovery::live_claims(&connection, 1, DEFAULT_LEASE_SECONDS).unwrap(), 0);
        assert_eq!(reclaim_expired(&connection, 1, DEFAULT_LEASE_SECONDS).unwrap(), 1, "a genuinely dead owner still expires");
        assert!(!owns_claim(&connection, &item).unwrap());
    }

}
