use super::super::super::fixture::{response, routes, Fixture};
use super::super::super::scope::ExposureGrant;
use super::super::http::PassiveHttpLimits;
use super::walk_checkpoint::WalkCheckpoint;
use super::*;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

fn fixture_scope(fixture: &Fixture) -> ExposureScope {
    ExposureScope {
        project_id: 41,
        scope_version: 3,
        issued_at: utc_now(),
        valid_until: None,
        coarse_legacy: false,
        grants: vec![
            ExposureGrant::from_url(&reqwest::Url::parse(&fixture.origin).unwrap()).unwrap(),
        ],
        sources: vec!["passive_web".to_string()],
        max_requests: 200,
        max_response_bytes: 40_000,
        max_total_bytes: 4_000_000,
        allow_private_destinations: true,
        allow_same_host_http_redirects: false,
        excluded: Vec::new(),
    }
}

fn fixture_item(fixture: &Fixture) -> WorkItem {
    WorkItem {
        id: 71,
        run_id: 61,
        project_id: 41,
        work_key: format!("passive_web|{}", fixture.origin),
        kind: "passive_web".to_string(),
        source_key: "passive_web".to_string(),
        asset_id: Some(81),
        origin: fixture.origin.clone(),
        entity: fixture.origin.clone(),
        input_hash: "fixture-input".to_string(),
        status: WorkItemStatus::Claimed,
        owner: "fixture-owner".to_string(),
        claim_version: 1,
        attempts: 1,
        observations: 0,
        requests: 0,
        bytes: 0,
    }
}

fn fixture_client() -> PassiveClient {
    PassiveClient::new(PassiveHttpLimits {
        timeout: std::time::Duration::from_secs(2),
        ..PassiveHttpLimits::default()
    })
    .unwrap()
}

fn step(
    fixture: &Fixture,
    item: &WorkItem,
    meter: &mut RequestMeter,
    raw: &str,
    saved: &[String],
    persisted: &RefCell<String>,
    cancelled: &dyn Fn() -> bool,
    charged: &AtomicUsize,
) -> ItemOutcome {
    execute_checkpointed(
        &fixture_client(),
        &fixture_scope(fixture),
        item,
        meter,
        &WalkBudget::default(),
        &|| Ok(()),
        cancelled,
        raw,
        saved,
        &|cursor| {
            *persisted.borrow_mut() = cursor.to_string();
            Ok(())
        },
        &|| {
            charged.fetch_add(1, Ordering::SeqCst);
            Ok(())
        },
    )
    .unwrap()
}

#[test]
fn checkpointed_walk_drains_more_than_twenty_urls_without_repeating_saved_requests() {
    let html = (0..27)
        .map(|index| format!("<script src='/bundle-{index}.js'></script>"))
        .collect::<String>();
    let fixture = Fixture::start(routes(vec![(
        "/",
        vec![response(200, &[("Content-Type", "text/html")], &html)],
    )]));
    let mut item = fixture_item(&fixture);
    let scope = fixture_scope(&fixture);
    let mut meter = RequestMeter::for_scope(&scope);
    let persisted = RefCell::new(String::new());
    let charged = AtomicUsize::new(0);
    let mut cursor = String::new();
    let mut saved = Vec::new();
    for _ in 0..50 {
        let outcome = step(
            &fixture,
            &item,
            &mut meter,
            &cursor,
            &saved,
            &persisted,
            &|| false,
            &charged,
        );
        saved.extend(outcome.observations.iter().map(|row| row.url.clone()));
        cursor = outcome.cursor_json.clone().unwrap();
        item.attempts += 1;
        if !outcome.continuation {
            assert_eq!(outcome.status, Some(WorkItemStatus::Completed));
            assert_eq!(outcome.source_status, Some(SourceStatus::Completed));
            break;
        }
    }
    assert_eq!(cursor_pending_count(&cursor, &item.origin).unwrap(), 0);
    let seen = fixture.seen.lock().unwrap();
    assert_eq!(seen.len(), 31, "四个自然入口加27个脚本必须全部读到");
    let unique = seen.iter().collect::<HashSet<_>>();
    assert_eq!(unique.len(), seen.len(), "任何已保存URL不得重新发送");
    assert_eq!(charged.load(Ordering::SeqCst), 31);
    assert_eq!(meter.requests_used, 31);
}

#[test]
fn checkpointed_walk_pause_before_send_keeps_full_frontier_and_resumes_once() {
    let fixture = Fixture::start(routes(vec![(
        "/",
        vec![response(200, &[("Content-Type", "text/plain")], "public")],
    )]));
    let mut item = fixture_item(&fixture);
    let mut meter = RequestMeter::for_scope(&fixture_scope(&fixture));
    let persisted = RefCell::new(String::new());
    let charged = AtomicUsize::new(0);
    let paused = step(
        &fixture,
        &item,
        &mut meter,
        "",
        &[],
        &persisted,
        &|| true,
        &charged,
    );
    assert_eq!(paused.status, Some(WorkItemStatus::Cancelled));
    assert_eq!(
        cursor_pending_count(paused.cursor_json.as_ref().unwrap(), &item.origin).unwrap(),
        4
    );
    assert!(fixture.seen.lock().unwrap().is_empty());
    item.attempts += 1;
    let resumed = step(
        &fixture,
        &item,
        &mut meter,
        paused.cursor_json.as_ref().unwrap(),
        &[],
        &persisted,
        &|| false,
        &charged,
    );
    assert_eq!(resumed.observations.len(), 1);
    assert_eq!(fixture.seen.lock().unwrap().len(), 1);
}

#[test]
fn checkpointed_walk_crash_does_not_replay_unknown_inflight_and_continues_other_urls() {
    let fixture = Fixture::start(routes(vec![(
        "/",
        vec![response(200, &[("Content-Type", "text/plain")], "public")],
    )]));
    let mut item = fixture_item(&fixture);
    let mut meter = RequestMeter::for_scope(&fixture_scope(&fixture));
    let persisted = RefCell::new(String::new());
    let charged = AtomicUsize::new(0);
    let _lost_outcome = step(
        &fixture,
        &item,
        &mut meter,
        "",
        &[],
        &persisted,
        &|| false,
        &charged,
    );
    // Simulate a process dying after send but before evidence/cursor commit. Only the
    // pre-send inFlight checkpoint and the journaled charge are durable.
    let before_crash = persisted.borrow().clone();
    item.attempts += 1;
    let resumed = step(
        &fixture,
        &item,
        &mut meter,
        &before_crash,
        &[],
        &persisted,
        &|| false,
        &charged,
    );
    let state =
        WalkCheckpoint::decode(resumed.cursor_json.as_ref().unwrap(), &item.origin).unwrap();
    assert_eq!(state.unknown, vec![format!("{}/", fixture.origin)]);
    assert_eq!(
        state.visited,
        vec![format!("{}/robots.txt", fixture.origin)]
    );
    assert_eq!(
        fixture.requests(),
        vec!["/", "/robots.txt"],
        "夹具记录的是路径；恢复必须只读取下一个 URL，不能重放首页"
    );
    assert_eq!(charged.load(Ordering::SeqCst), 2);
}

#[test]
fn checkpointed_walk_reconciles_saved_response_without_fetching_it_again() {
    let fixture = Fixture::start(routes(vec![(
        "/",
        vec![response(200, &[("Content-Type", "text/plain")], "public")],
    )]));
    let mut item = fixture_item(&fixture);
    let mut meter = RequestMeter::for_scope(&fixture_scope(&fixture));
    let persisted = RefCell::new(String::new());
    let charged = AtomicUsize::new(0);
    let first = step(
        &fixture,
        &item,
        &mut meter,
        "",
        &[],
        &persisted,
        &|| false,
        &charged,
    );
    let precommit = persisted.borrow().clone();
    let saved = first
        .observations
        .iter()
        .map(|row| row.url.clone())
        .collect::<Vec<_>>();
    item.attempts += 1;
    let resumed = step(
        &fixture,
        &item,
        &mut meter,
        &precommit,
        &saved,
        &persisted,
        &|| false,
        &charged,
    );
    let state =
        WalkCheckpoint::decode(resumed.cursor_json.as_ref().unwrap(), &item.origin).unwrap();
    assert!(state.unknown.is_empty());
    assert!(state.visited.contains(&format!("{}/", fixture.origin)));
    assert_eq!(
        fixture.requests(),
        vec!["/", "/robots.txt"],
        "夹具记录的是路径；恢复必须只读取下一个 URL，不能重放首页"
    );
}

#[test]
fn checkpointed_walk_hard_run_budget_keeps_unsent_url_for_manual_budget_decision() {
    let fixture = Fixture::start(routes(vec![(
        "/",
        vec![response(200, &[("Content-Type", "text/plain")], "public")],
    )]));
    let item = fixture_item(&fixture);
    let scope = fixture_scope(&fixture);
    let state = WalkCheckpoint::new(
        &scope,
        &item,
        seed_urls(&reqwest::Url::parse(&format!("{}/", fixture.origin)).unwrap())
            .into_iter()
            .map(|url| url.to_string())
            .collect(),
    );
    let mut meter = RequestMeter::for_scope(&scope);
    meter.requests_used = meter.max_requests;
    let persisted = RefCell::new(String::new());
    let charged = AtomicUsize::new(0);
    let outcome = step(
        &fixture,
        &item,
        &mut meter,
        &state.encode().unwrap(),
        &[],
        &persisted,
        &|| false,
        &charged,
    );
    assert_eq!(outcome.status, Some(WorkItemStatus::BlockedQuota));
    assert_eq!(
        cursor_pending_count(outcome.cursor_json.as_ref().unwrap(), &item.origin).unwrap(),
        4
    );
    assert!(!outcome.continuation);
    assert!(fixture.seen.lock().unwrap().is_empty());
}

#[test]
fn checkpointed_walk_rejects_foreign_corrupt_and_legacy_cursor_without_a_send() {
    let fixture = Fixture::start(routes(Vec::new()));
    let mut item = fixture_item(&fixture);
    let scope = fixture_scope(&fixture);
    let state = WalkCheckpoint::new(&scope, &item, vec![format!("{}/", fixture.origin)]);
    let raw = state.encode().unwrap();
    assert!(validate_cursor_for(&raw, &item, &scope).is_ok());
    let mut changed = item.clone();
    changed.run_id += 1;
    assert!(validate_cursor_for(&raw, &changed, &scope)
        .unwrap_err()
        .contains("scope_changed"));
    assert!(cursor_pending_count(&raw, "http://foreign.example").is_err());
    assert!(cursor_pending_count("{broken", &item.origin).is_err());
    let mut tampered = serde_json::to_value(&state).unwrap();
    tampered["queue"] = json!(["https://user:password@example.test/"]);
    assert!(cursor_pending_count(&tampered.to_string(), &item.origin).is_err());
    item.attempts = 2;
    let mut meter = RequestMeter::for_scope(&scope);
    let result = execute_checkpointed(
        &fixture_client(),
        &scope,
        &item,
        &mut meter,
        &WalkBudget::default(),
        &|| Ok(()),
        &|| false,
        "",
        &[],
        &|_| Ok(()),
        &|| Ok(()),
    );
    assert!(result.unwrap_err().contains("legacy_checkpoint_missing"));
    assert!(fixture.seen.lock().unwrap().is_empty());
}

#[test]
fn checkpointed_walk_persistence_failure_refuses_request_before_send() {
    let fixture = Fixture::start(routes(Vec::new()));
    let item = fixture_item(&fixture);
    let scope = fixture_scope(&fixture);
    let mut meter = RequestMeter::for_scope(&scope);
    let result = execute_checkpointed(
        &fixture_client(),
        &scope,
        &item,
        &mut meter,
        &WalkBudget::default(),
        &|| Ok(()),
        &|| false,
        "",
        &[],
        &|_| Err("disk unavailable".to_string()),
        &|| Ok(()),
    );
    assert_eq!(result.unwrap_err(), "disk unavailable");
    assert_eq!(meter.requests_used, 0);
    assert!(fixture.seen.lock().unwrap().is_empty());
}

#[test]
fn checkpointed_walk_cancel_between_checkpoint_and_send_does_not_drop_url() {
    let fixture = Fixture::start(routes(Vec::new()));
    let item = fixture_item(&fixture);
    let scope = fixture_scope(&fixture);
    let mut meter = RequestMeter::for_scope(&scope);
    let stop = AtomicBool::new(false);
    let result = execute_checkpointed(
        &fixture_client(),
        &scope,
        &item,
        &mut meter,
        &WalkBudget::default(),
        &|| Ok(()),
        &|| stop.load(Ordering::SeqCst),
        "",
        &[],
        &|_| {
            stop.store(true, Ordering::SeqCst);
            Ok(())
        },
        &|| Ok(()),
    )
    .unwrap();
    assert_eq!(result.status, Some(WorkItemStatus::Cancelled));
    assert_eq!(
        cursor_pending_count(result.cursor_json.as_ref().unwrap(), &item.origin).unwrap(),
        4
    );
    assert_eq!(meter.requests_used, 0);
    assert!(fixture.seen.lock().unwrap().is_empty());
}

#[test]
fn checkpoint_unknown_byte_reservation_is_retained_once_and_exposed_as_bounded_metadata() {
    let fixture = Fixture::start(routes(Vec::new()));
    let item = fixture_item(&fixture);
    let scope = fixture_scope(&fixture);
    let mut state = WalkCheckpoint::new(&scope, &item, vec![]);
    state.in_flight = Some(format!("{}/", fixture.origin));
    state.reserved_bytes = scope.max_response_bytes;
    state.reconcile_saved(&[]);
    assert_eq!(state.reserved_bytes, 0);
    assert_eq!(state.unknown_reserved_bytes, scope.max_response_bytes);
    state.reconcile_saved(&[]);
    assert_eq!(
        state.unknown_reserved_bytes, scope.max_response_bytes,
        "重复恢复不能重复保守计费"
    );
    assert_eq!(
        cursor_progress_counts(&state.encode().unwrap(), &item.origin).unwrap(),
        (1, 0, scope.max_response_bytes)
    );
    let mut invalid = serde_json::to_value(&state).unwrap();
    invalid["reservedBytes"] = json!(1);
    assert!(
        cursor_pending_count(&invalid.to_string(), &item.origin).is_err(),
        "没有在途 URL 不能存在当前请求预留"
    );
    invalid["reservedBytes"] = json!(0);
    invalid["unknownReservedBytes"] = json!(-1);
    assert!(cursor_pending_count(&invalid.to_string(), &item.origin).is_err());
}

#[test]
fn checkpoint_pause_race_from_send_journal_is_resumable_and_not_authorization_failure() {
    let fixture = Fixture::start(routes(Vec::new()));
    let item = fixture_item(&fixture);
    let scope = fixture_scope(&fixture);
    let mut meter = RequestMeter::for_scope(&scope);
    let result = execute_checkpointed(
        &fixture_client(),
        &scope,
        &item,
        &mut meter,
        &WalkBudget::default(),
        &|| Ok(()),
        &|| false,
        "",
        &[],
        &|_| Ok(()),
        &|| Err(ExposureDenial::new("exposure_paused", "已暂停")),
    )
    .unwrap();
    assert_eq!(result.status, Some(WorkItemStatus::Cancelled));
    assert_eq!(result.source_status, Some(SourceStatus::Cancelled));
    assert!(result.continuation);
    assert_eq!(
        cursor_pending_count(result.cursor_json.as_ref().unwrap(), &item.origin).unwrap(),
        4
    );
    assert_eq!(meter.requests_used, 0);
    assert!(fixture.seen.lock().unwrap().is_empty());
}

#[test]
fn checkpoint_walk_preserves_natural_links_beyond_legacy_two_hundred() {
    let html = (0..250)
        .map(|index| format!("<script src='/chunk-{index}.js'></script>"))
        .collect::<String>();
    let fixture = Fixture::start(routes(vec![(
        "/",
        vec![response(200, &[("Content-Type", "text/html")], &html)],
    )]));
    let item = fixture_item(&fixture);
    let mut meter = RequestMeter::for_scope(&fixture_scope(&fixture));
    let outcome = step(
        &fixture,
        &item,
        &mut meter,
        "",
        &[],
        &RefCell::new(String::new()),
        &|| false,
        &AtomicUsize::new(0),
    );
    let cursor = outcome.cursor_json.as_ref().unwrap();
    let state = WalkCheckpoint::decode(cursor, &item.origin).unwrap();
    assert_eq!(
        state.queue.len(),
        253,
        "三个标准入口和250个自然资源都必须保留"
    );
    assert!(!state.queue_truncated);
    assert!(state
        .queue
        .contains(&format!("{}/chunk-249.js", fixture.origin)));
    let bounded = natural_links_bounded(
        &html,
        &reqwest::Url::parse(&format!("{}/", fixture.origin)).unwrap(),
        200,
    );
    assert_eq!(bounded.links.len(), 200);
    assert!(bounded.truncated);
    assert_eq!(bounded.deferred_count, 50);
}

#[test]
fn checkpoint_pause_between_known_redirect_steps_resumes_url_without_unknown_gap() {
    use std::sync::Arc;
    let paused = Arc::new(AtomicBool::new(false));
    let first = Arc::new(AtomicBool::new(true));
    let (pause_hook, first_hook) = (paused.clone(), first.clone());
    let fixture = Fixture::start_hooked(
        routes(vec![
            (
                "/",
                vec![response(302, &[("Location", "/destination")], "")],
            ),
            (
                "/destination",
                vec![response(
                    200,
                    &[("Content-Type", "text/plain")],
                    "public response",
                )],
            ),
        ]),
        Box::new(move |_| {
            if first_hook.swap(false, Ordering::SeqCst) {
                pause_hook.store(true, Ordering::SeqCst);
            }
        }),
    );
    let mut item = fixture_item(&fixture);
    let mut meter = RequestMeter::for_scope(&fixture_scope(&fixture));
    let persisted = RefCell::new(String::new());
    let charged = AtomicUsize::new(0);
    let stopped = step(
        &fixture,
        &item,
        &mut meter,
        "",
        &[],
        &persisted,
        &|| paused.load(Ordering::SeqCst),
        &charged,
    );
    assert_eq!(stopped.status, Some(WorkItemStatus::Cancelled));
    assert_eq!(meter.requests_used, 1, "已完成的重定向请求仍计入总用量");
    let checkpoint = stopped.cursor_json.as_ref().unwrap();
    let state = WalkCheckpoint::decode(checkpoint, &item.origin).unwrap();
    assert_eq!(state.queue.first(), Some(&format!("{}/", fixture.origin)));
    assert!(
        state.unknown.is_empty(),
        "明确响应后的正常暂停不能被标成崩溃未知请求"
    );
    paused.store(false, Ordering::SeqCst);
    item.attempts += 1;
    let resumed = step(
        &fixture,
        &item,
        &mut meter,
        checkpoint,
        &[],
        &persisted,
        &|| paused.load(Ordering::SeqCst),
        &charged,
    );
    assert_eq!(resumed.observations.len(), 1);
    assert_eq!(resumed.observations[0].status_code, 200);
    assert_eq!(meter.requests_used, 3);
    assert_eq!(
        cursor_progress_counts(resumed.cursor_json.as_ref().unwrap(), &item.origin)
            .unwrap()
            .0,
        0
    );
}
