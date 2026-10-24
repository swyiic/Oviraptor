//! Resume admission is driven by the saved frontier and original cumulative budget.
//! These tests use temporary databases only; they neither contact targets nor alter App data.

use super::fixture::seed_scope;
use super::orchestrator::{scope_hash_of, start_or_reuse, KIND_PASSIVE_WEB};
use super::recovery::{load_plan, prepare_resume, resume_view};
use super::scope::ExposureScope;
use super::state::ensure_exposure_schema;
use super::work_items::{claim_next, read_run_items, WorkItem};
use rusqlite::{params, Connection};
use serde_json::{json, Value};

const PROJECT: i64 = 11803;
struct Fixture {
    root: std::path::PathBuf,
    c: Connection,
    run_id: i64,
    scope: ExposureScope,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("ovi-exposure-resume-{}", uuid::Uuid::new_v4()));
        let path = crate::db::initialize(&root).unwrap();
        let c = crate::db::open(&path).unwrap();
        ensure_exposure_schema(&c).unwrap();
        seed_scope(&c, PROJECT, "https://a.example.test/", true);
        seed_scope(&c, PROJECT, "https://b.example.test/", true);
        let settings = json!({"exposureSources":["passive_web"],"exposureMaxRequests":200,"exposureMaxTotalBytes":10_000_000});
        let started = start_or_reuse(&c, PROJECT, &settings).unwrap();
        let frozen = super::orchestrator::frozen_settings_strict(&c, started.run_id, PROJECT)
            .unwrap()
            .to_settings_json();
        let scope = ExposureScope::load(&c, PROJECT, &frozen).unwrap();
        Self {
            root,
            c,
            run_id: started.run_id,
            scope,
        }
    }
    fn items(&self) -> Vec<WorkItem> {
        read_run_items(&self.c, self.run_id, PROJECT).unwrap()
    }
    fn checkpoint(&self, item: &WorkItem) -> String {
        json!({"version":1,"runId":item.run_id,"projectId":item.project_id,"workKey":item.work_key,
            "inputHash":item.input_hash,"origin":item.origin,"scopeHash":scope_hash_of(&self.scope),
            "queue":[format!("{}/next.js",item.origin)],"visited":[format!("{}/",item.origin)],
            "inFlight":null,"unknown":[],"successes":1,"sourceReason":"","coverageReason":"","queueTruncated":false,"deferredDiscoveryCount":0}).to_string()
    }
    fn stopped(&self, status: &str, item_status: &str) -> (WorkItem, WorkItem) {
        let items = self.items();
        assert_eq!(items.len(), 2);
        let first = items[0].clone();
        let second = items[1].clone();
        assert_eq!(first.kind, KIND_PASSIVE_WEB);
        self.c.execute("UPDATE exposure_work_items SET status='completed',owner='',heartbeat_at='',requests=1,bytes=10 WHERE id=?1",[first.id]).unwrap();
        self.c.execute("UPDATE exposure_work_items SET status=?2,owner='',heartbeat_at='',cursor=?3,attempts=1,requests=2,bytes=20 WHERE id=?1",params![second.id,item_status,self.checkpoint(&second)]).unwrap();
        self.c
            .execute(
                "UPDATE exposure_run_plans SET requests_used=3,bytes_used=30 WHERE run_id=?1",
                [self.run_id],
            )
            .unwrap();
        self.c
            .execute(
                "UPDATE exposure_runs SET status=?2,stage=?2,cancel_requested=0 WHERE id=?1",
                params![self.run_id, status],
            )
            .unwrap();
        (first, second)
    }
    fn ledger(&self) -> Value {
        let items:String=self.c.query_row("SELECT json_group_array(json_array(id,status,owner,heartbeat_at,cursor,attempts,requests,bytes)) FROM exposure_work_items WHERE run_id=?1",[self.run_id],|row|row.get(0)).unwrap();
        let run: Value = self
            .c
            .query_row(
                "SELECT status,stage,cancel_requested FROM exposure_runs WHERE id=?1",
                [self.run_id],
                |row| {
                    Ok(json!([
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?
                    ]))
                },
            )
            .unwrap();
        let plan = load_plan(&self.c, self.run_id, PROJECT).unwrap();
        json!({"items":items,"run":run,"requests":plan.requests_used,"bytes":plan.bytes_used})
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn partial_cursor_reopens_only_remaining_origin_and_keeps_original_usage() {
    let f = Fixture::new();
    let (done, pending) = f.stopped("partial", "blocked_quota");
    let before = f.ledger();
    let view = resume_view(&f.c, f.run_id, PROJECT).unwrap();
    assert!(view.resumable, "{view:?}");
    assert_eq!((view.pending_items, view.pending_urls), (1, 1));
    assert_eq!(view.requests_remaining, 197);
    assert_eq!(
        before,
        f.ledger(),
        "reading recovery must not mutate claims or usage"
    );
    assert!(prepare_resume(&f.c, f.run_id, PROJECT).unwrap());
    let items = f.items();
    assert_eq!(
        items
            .iter()
            .find(|i| i.id == done.id)
            .unwrap()
            .status
            .as_str(),
        "completed"
    );
    assert_eq!(
        items
            .iter()
            .find(|i| i.id == pending.id)
            .unwrap()
            .status
            .as_str(),
        "pending"
    );
    let plan = load_plan(&f.c, f.run_id, PROJECT).unwrap();
    assert_eq!((plan.requests_used, plan.bytes_used), (3, 30));
    let claimed = claim_next(&f.c, f.run_id, "new-worker", 90)
        .unwrap()
        .unwrap();
    assert_eq!(claimed.id, pending.id);
}

#[test]
fn hard_request_and_byte_limits_refuse_resume_without_resetting_or_reopening() {
    for (column, reason) in [
        ("requests_used=budget_requests", "request_budget_exhausted"),
        ("bytes_used=budget_bytes", "byte_budget_exhausted"),
    ] {
        let f = Fixture::new();
        f.stopped("partial", "blocked_quota");
        f.c.execute(
            &format!("UPDATE exposure_run_plans SET {column} WHERE run_id=?1"),
            [f.run_id],
        )
        .unwrap();
        let before = f.ledger();
        let view = resume_view(&f.c, f.run_id, PROJECT).unwrap();
        assert!(!view.resumable);
        assert_eq!(view.block_reason, reason);
        assert_eq!(view.pending_urls, 1);
        assert!(prepare_resume(&f.c, f.run_id, PROJECT)
            .unwrap_err()
            .contains(reason));
        assert_eq!(
            before,
            f.ledger(),
            "a denied resume cannot erase spend or change terminal work"
        );
    }
}

#[test]
fn fresh_claim_prevents_resume_even_after_a_pause_request() {
    let f = Fixture::new();
    let (_, pending) = f.stopped("cancelled", "cancelled");
    f.c.execute("UPDATE exposure_work_items SET status='in_progress',owner='still-exiting',heartbeat_at=?2 WHERE id=?1",params![pending.id,super::state::utc_now()]).unwrap();
    let before = f.ledger();
    let view = resume_view(&f.c, f.run_id, PROJECT).unwrap();
    assert_eq!(view.block_reason, "live_claim");
    assert!(!view.resumable);
    assert!(prepare_resume(&f.c, f.run_id, PROJECT)
        .unwrap_err()
        .contains("live_claim"));
    assert_eq!(before, f.ledger());
}

#[test]
fn damaged_or_foreign_cursor_is_never_an_empty_fresh_frontier() {
    for key in [
        "malformed",
        "runId",
        "projectId",
        "workKey",
        "inputHash",
        "scopeHash",
    ] {
        let f = Fixture::new();
        let (_, pending) = f.stopped("partial", "blocked_quota");
        let mut value: Value = serde_json::from_str(&f.checkpoint(&pending)).unwrap();
        let raw = if key == "malformed" {
            "{}".into()
        } else {
            value[key] = if matches!(key, "runId" | "projectId") {
                json!(999_999_999)
            } else {
                json!("foreign")
            };
            value.to_string()
        };
        f.c.execute(
            "UPDATE exposure_work_items SET cursor=?2 WHERE id=?1",
            params![pending.id, raw],
        )
        .unwrap();
        let before = f.ledger();
        let view = resume_view(&f.c, f.run_id, PROJECT).unwrap();
        assert_eq!(view.block_reason, "cursor_invalid", "{key}: {view:?}");
        assert!(!view.resumable);
        assert!(prepare_resume(&f.c, f.run_id, PROJECT).is_err());
        assert_eq!(before, f.ledger());
    }
}

#[test]
fn legacy_partial_without_frontier_preserves_evidence_and_refuses_blind_recollection() {
    let f = Fixture::new();
    let (_, pending) = f.stopped("partial", "blocked_quota");
    f.c.execute(
        "UPDATE exposure_work_items SET cursor='' WHERE id=?1",
        [pending.id],
    )
    .unwrap();
    let before = f.ledger();
    let view = resume_view(&f.c, f.run_id, PROJECT).unwrap();
    assert_eq!(view.block_reason, "legacy_checkpoint_missing");
    assert_eq!(view.cursor_missing_items, 1);
    assert!(prepare_resume(&f.c, f.run_id, PROJECT)
        .unwrap_err()
        .contains("legacy_checkpoint_missing"));
    assert_eq!(before, f.ledger());
}

#[test]
fn pause_only_sets_stop_flag_and_retains_checkpoint_and_ledger() {
    let f = Fixture::new();
    let (_, pending) = f.stopped("running", "pending");
    let cursor: String =
        f.c.query_row(
            "SELECT cursor FROM exposure_work_items WHERE id=?1",
            [pending.id],
            |row| row.get(0),
        )
        .unwrap();
    super::cancel_exposure_scan_in(&f.c, PROJECT, f.run_id).unwrap();
    assert_eq!(
        f.c.query_row(
            "SELECT cancel_requested FROM exposure_runs WHERE id=?1",
            [f.run_id],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        f.c.query_row(
            "SELECT cursor FROM exposure_work_items WHERE id=?1",
            [pending.id],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        cursor
    );
    assert_eq!(load_plan(&f.c, f.run_id, PROJECT).unwrap().requests_used, 3);
    let view = resume_view(&f.c, f.run_id, PROJECT).unwrap();
    assert!(!view.paused_reason.is_empty());
    assert!(!view.resumable, "fresh executor must exit before recovery");
}

#[test]
fn cold_restart_reclaims_only_expired_claim_and_continues_saved_queue() {
    let f = Fixture::new();
    let (_, pending) = f.stopped("running", "pending");
    let claimed = claim_next(&f.c, f.run_id, "dead-worker", 90)
        .unwrap()
        .unwrap();
    assert_eq!(claimed.id, pending.id);
    f.c.execute("UPDATE exposure_work_items SET heartbeat_at='2020-01-01 00:00:00',updated_at='2020-01-01 00:00:00' WHERE id=?1",[pending.id]).unwrap();
    f.c.execute("UPDATE exposure_runs SET created_at='2020-01-01 00:00:00',started_at='2020-01-01 00:00:00' WHERE id=?1",[f.run_id]).unwrap();
    assert!(super::recovery::mark_interrupted_if_dead(&f.c, f.run_id, 90).unwrap());
    assert!(resume_view(&f.c, f.run_id, PROJECT).unwrap().resumable);
    assert!(prepare_resume(&f.c, f.run_id, PROJECT).unwrap());
    let next = claim_next(&f.c, f.run_id, "new-worker", 90)
        .unwrap()
        .unwrap();
    assert_eq!(next.id, pending.id);
    assert_ne!(next.claim_version, claimed.claim_version);
    assert_eq!(load_plan(&f.c, f.run_id, PROJECT).unwrap().requests_used, 3);
}

#[test]
fn frozen_scope_change_or_cross_project_resume_is_refused_without_writes() {
    let f = Fixture::new();
    f.stopped("partial", "blocked_quota");
    let before = f.ledger();
    assert!(resume_view(&f.c, f.run_id, PROJECT + 1).is_err());
    assert!(prepare_resume(&f.c, f.run_id, PROJECT + 1).is_err());
    assert_eq!(before, f.ledger());
    f.c.execute(
        "UPDATE asset_ownership_decisions SET exposure_eligible=0 WHERE project_id=?1",
        [PROJECT],
    )
    .unwrap();
    let before = f.ledger();
    let view = resume_view(&f.c, f.run_id, PROJECT).unwrap();
    assert!(!view.resumable);
    assert!(matches!(
        view.block_reason.as_str(),
        "frozen_scope_changed" | "authorization_scope_unavailable"
    ));
    assert!(prepare_resume(&f.c, f.run_id, PROJECT).is_err());
    assert_eq!(before, f.ledger());
}

fn native_pages() -> std::collections::HashMap<String, Vec<String>> {
    use super::fixture::response;
    let links = (0..25)
        .map(|n| format!("<a href='/page-{n}'>next</a>"))
        .collect::<String>();
    let mut routes = std::collections::HashMap::new();
    routes.insert(
        "/".to_string(),
        vec![response(200, &[("Content-Type", "text/html")], &links)],
    );
    for n in 0..25 {
        routes.insert(
            format!("/page-{n}"),
            vec![response(
                200,
                &[("Content-Type", "text/plain")],
                "public fixture response",
            )],
        );
    }
    routes
}
fn native_settings() -> Value {
    json!({"exposureAllowPrivateDestinations":true,"exposureSources":["passive_web"],"exposureMaxRequests":200,"exposureMaxTotalBytes":10_000_000})
}
fn native_status(c: &Connection, run_id: i64) -> String {
    c.query_row(
        "SELECT status FROM exposure_runs WHERE id=?1",
        [run_id],
        |row| row.get(0),
    )
    .unwrap()
}

#[test]
fn native_worker_drains_more_than_twenty_urls_and_charges_each_socket_once() {
    let root = std::env::temp_dir().join(format!(
        "ovi-exposure-native-frontier-{}",
        uuid::Uuid::new_v4()
    ));
    let path = crate::db::initialize(&root).unwrap();
    let c = crate::db::open(&path).unwrap();
    ensure_exposure_schema(&c).unwrap();
    let server = super::fixture::Fixture::start(native_pages());
    seed_scope(&c, PROJECT, &format!("{}/", server.origin), true);
    let config = native_settings();
    let started = start_or_reuse(&c, PROJECT, &config).unwrap();
    super::execute_exposure_run(
        path.clone(),
        started.run_id,
        PROJECT,
        "native-drain".into(),
        config,
    );
    assert_eq!(native_status(&c, started.run_id), "completed");
    let requests = server.requests();
    assert_eq!(
        requests.len(),
        29,
        "four starting resources plus twenty-five natural links"
    );
    assert_eq!(
        requests
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        29,
        "each completed URL is fetched once"
    );
    let plan = load_plan(&c, started.run_id, PROJECT).unwrap();
    assert_eq!(
        plan.requests_used, 29,
        "before-send reservation and evidence commit cannot charge twice"
    );
    let items = read_run_items(&c, started.run_id, PROJECT).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].requests, 29);
    let raw = super::work_items::load_cursor(&c, items[0].id).unwrap();
    assert_eq!(
        super::adapters::passive_web::cursor_pending_count(&raw, &items[0].origin).unwrap(),
        0
    );
    drop(c);
    drop(server);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_pause_then_cold_connection_resume_keeps_saved_homepage_and_drains_frontier() {
    use std::sync::{
        atomic::{AtomicBool, AtomicI64, Ordering},
        Arc,
    };
    let root = std::env::temp_dir().join(format!(
        "ovi-exposure-native-pause-{}",
        uuid::Uuid::new_v4()
    ));
    let path = crate::db::initialize(&root).unwrap();
    let c = crate::db::open(&path).unwrap();
    ensure_exposure_schema(&c).unwrap();
    let run_id = Arc::new(AtomicI64::new(0));
    let paused = Arc::new(AtomicBool::new(false));
    let hook_path = path.clone();
    let hook_run = run_id.clone();
    let hook_paused = paused.clone();
    let server = super::fixture::Fixture::start_hooked(
        native_pages(),
        Box::new(move |url| {
            if url == "/" && !hook_paused.swap(true, Ordering::SeqCst) {
                let live = crate::db::open(&hook_path).unwrap();
                super::cancel_exposure_scan_in(&live, PROJECT, hook_run.load(Ordering::SeqCst))
                    .unwrap();
            }
        }),
    );
    seed_scope(&c, PROJECT, &format!("{}/", server.origin), true);
    let config = native_settings();
    let started = start_or_reuse(&c, PROJECT, &config).unwrap();
    run_id.store(started.run_id, Ordering::SeqCst);
    super::execute_exposure_run(
        path.clone(),
        started.run_id,
        PROJECT,
        "native-before-pause".into(),
        config.clone(),
    );
    assert_eq!(native_status(&c, started.run_id), "cancelled");
    assert_eq!(server.requests(), vec!["/".to_string()]);
    let before = load_plan(&c, started.run_id, PROJECT).unwrap();
    assert_eq!(before.requests_used, 1);
    let first_saved: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM exposure_observations WHERE run_id=?1 AND status_code=200",
            [started.run_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(first_saved, 1);
    drop(c);
    let restarted = crate::db::open(&path).unwrap();
    let view = resume_view(&restarted, started.run_id, PROJECT).unwrap();
    assert!(view.resumable, "{view:?}");
    assert_eq!(view.pending_urls, 28);
    assert!(prepare_resume(&restarted, started.run_id, PROJECT).unwrap());
    super::execute_exposure_run(
        path.clone(),
        started.run_id,
        PROJECT,
        "native-after-pause".into(),
        config,
    );
    assert_eq!(native_status(&restarted, started.run_id), "completed");
    let requests = server.requests();
    assert_eq!(requests.len(), 29);
    assert_eq!(
        requests.iter().filter(|url| url.as_str() == "/").count(),
        1,
        "the saved homepage is never re-sent on resume"
    );
    assert_eq!(
        load_plan(&restarted, started.run_id, PROJECT)
            .unwrap()
            .requests_used,
        29
    );
    let duplicate_observations:i64=restarted.query_row("SELECT COUNT(*) FROM (SELECT url,COUNT(*) AS n FROM exposure_observations WHERE run_id=?1 GROUP BY url HAVING n>1)",[started.run_id],|row|row.get(0)).unwrap();
    assert_eq!(duplicate_observations, 0);
    drop(restarted);
    drop(server);
    std::fs::remove_dir_all(root).unwrap();
}
