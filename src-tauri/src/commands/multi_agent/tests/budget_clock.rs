#[test]
fn budget_clock_concurrent_children_share_one_root_elapsed_charge() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{budget, scheduler},
    };
    let (root, path, id, lease) = multi_agent_test_root("clock-shared", 100, 3);
    let db = db::open(&path).unwrap();
    db.execute(
        "UPDATE agent_runs SET created_at=datetime('now','-3 seconds','localtime') WHERE id=?1",
        [&id],
    )
    .unwrap();
    scheduler::schedule_child(
        &db,
        &lease,
        AgentRole::SpaApiMapper,
        AgentLane::ReadOnlyAnalysis,
        "clock-one",
        &serde_json::json!({}),
        1,
        &["evidence.read".into()],
        10,
        1,
    )
    .unwrap();
    scheduler::schedule_child(
        &db,
        &lease,
        AgentRole::EvidenceReviewer,
        AgentLane::Review,
        "clock-two",
        &serde_json::json!({}),
        1,
        &["evidence.read".into(), "review.write".into()],
        10,
        1,
    )
    .unwrap();
    let charge = budget::balance(&db, &id, None, "wall_time_ms").unwrap();
    assert_eq!((charge.reserved, charge.indeterminate), (0, 0));
    assert!(
        (2800..5000).contains(&charge.consumed),
        "elapsed was charged once: {charge:?}"
    );
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn budget_clock_expired_root_cannot_schedule_with_a_fresh_child_clock() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };
    let (root, path, id, lease) = multi_agent_test_root("clock-expired", 100, 3);
    let db = db::open(&path).unwrap();
    db.execute(
        "UPDATE agent_runs SET created_at=datetime('now','-1 day','localtime') WHERE id=?1",
        [&id],
    )
    .unwrap();
    let before = receipt_database_snapshot(&db);
    assert_eq!(
        scheduler::schedule_child(
            &db,
            &lease,
            AgentRole::SpaApiMapper,
            AgentLane::ReadOnlyAnalysis,
            "clock-expired",
            &serde_json::json!({}),
            1,
            &["evidence.read".into()],
            10,
            1
        )
        .unwrap_err(),
        "budget_wall_time_exhausted"
    );
    assert_eq!(receipt_database_snapshot(&db), before);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn budget_clock_origin_change_cannot_reset_model_deadline() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{scheduler, specialist},
    };
    let (root, path, id, lease) = multi_agent_test_root("clock-rebase", 100, 3);
    let db = db::open(&path).unwrap();
    let child = scheduler::schedule_child(
        &db,
        &lease,
        AgentRole::SpaApiMapper,
        AgentLane::ReadOnlyAnalysis,
        "clock-rebase",
        &serde_json::json!({}),
        1,
        &["evidence.read".into()],
        10,
        1,
    )
    .unwrap();
    scheduler::mark_child_running(&db, &lease, &child).unwrap();
    db.execute(
        "UPDATE agent_runs SET created_at=datetime('now','+1 day','localtime') WHERE id=?1",
        [&id],
    )
    .unwrap();
    let before = receipt_database_snapshot(&db);
    assert_eq!(
        specialist::start(&db, &lease, &child, &serde_json::json!({"messages":[]})).unwrap_err(),
        "budget_clock_origin_conflict"
    );
    assert_eq!(receipt_database_snapshot(&db), before);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn budget_clock_failed_sample_rolls_back_all_new_authority() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };
    for action in ["ABORT,'clock failed'", "FAIL,'clock failed'", "IGNORE"] {
        let (root, path, id, lease) = multi_agent_test_root("clock-rollback", 100, 3);
        let db = db::open(&path).unwrap();
        db.execute(
            "UPDATE agent_runs SET created_at=datetime('now','-3 seconds','localtime') WHERE id=?1",
            [&id],
        )
        .unwrap();
        db.execute_batch(&format!("CREATE TRIGGER reject_clock BEFORE INSERT ON agent_budget_entries WHEN NEW.dimension='wall_time_ms' BEGIN SELECT RAISE({action}); END;")).unwrap();
        let before = receipt_database_snapshot(&db);
        assert!(
            scheduler::schedule_child(
                &db,
                &lease,
                AgentRole::SpaApiMapper,
                AgentLane::ReadOnlyAnalysis,
                "clock-rollback",
                &serde_json::json!({}),
                1,
                &["evidence.read".into()],
                10,
                1
            )
            .is_err(),
            "{action}"
        );
        assert_eq!(receipt_database_snapshot(&db), before, "{action}");
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn budget_clock_limits_real_specialist_transport_to_root_time_remaining() {
    let (root, mut context, lease, child) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    db.execute_batch("DROP TRIGGER budget_limit_no_update")
        .unwrap();
    db.execute("UPDATE agent_budget_limits SET hard_limit=2000 WHERE root_run_id=?1 AND dimension='wall_time_ms'",[&lease.root_run_id]).unwrap();
    let entered = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let calls = entered.clone();
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(move |_| {
        calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        std::thread::sleep(Duration::from_secs(4));
        (200, "application/json", proposal_model_response("too late"))
    }));
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let started = std::time::Instant::now();
    let result = multi_agent_child_round_transport(
        &context,
        &lease,
        &child,
        "readonly",
        serde_json::json!({}),
    );
    assert!(
        result.is_err(),
        "root time expired before the response: {result:?}"
    );
    assert!(
        started.elapsed() < Duration::from_millis(3300),
        "transport did not close at root deadline: {:?}",
        started.elapsed()
    );
    assert_eq!(
        entered.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "one actual accepted request, no retry"
    );
    assert!(seen.lock().unwrap().len() <= 1);
    let cost = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &lease.root_run_id,
        None,
        "model_requests",
    )
    .unwrap();
    assert_eq!(
        (cost.reserved, cost.consumed, cost.indeterminate),
        (0, 0, 1)
    );
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn budget_clock_expiry_after_child_settlement_cannot_be_reported_as_complete() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };
    let (root, path, id, lease) = multi_agent_test_root("clock-final-close", 100, 3);
    let db = db::open(&path).unwrap();
    let child = scheduler::schedule_child(
        &db,
        &lease,
        AgentRole::SpaApiMapper,
        AgentLane::ReadOnlyAnalysis,
        "clock-final",
        &serde_json::json!({}),
        1,
        &["evidence.read".into()],
        10,
        1,
    )
    .unwrap();
    scheduler::finish_child(&db, &lease, &child, false, "never dispatched").unwrap();
    db.execute_batch("DROP TRIGGER budget_limit_no_update")
        .unwrap();
    db.execute("UPDATE agent_budget_limits SET hard_limit=1 WHERE root_run_id=?1 AND dimension='wall_time_ms'",[&id]).unwrap();
    let before = receipt_database_snapshot(&db);
    let outcome = AgentTargetOutcome::Completed(AgentCompletion::without_ledger(
        "complete",
        terminal_code::LEDGER_COMPLETE,
    ));
    assert_eq!(
        finish_coordinator_run(&db, &lease, &outcome).unwrap_err(),
        "budget_wall_time_exhausted"
    );
    assert_eq!(receipt_database_snapshot(&db), before);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn budget_clock_records_elapsed_after_worker_closes_without_reopening_admission() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{budget, scheduler},
    };
    for closed in [false, true] {
        let (root, path, id, lease) = multi_agent_test_root("clock-closed-worker", 100, 3);
        let db = db::open(&path).unwrap();
        let child = scheduler::schedule_child(
            &db,
            &lease,
            AgentRole::SpaApiMapper,
            AgentLane::ReadOnlyAnalysis,
            "clock-worker",
            &serde_json::json!({}),
            1,
            &["evidence.read".into()],
            10,
            1,
        )
        .unwrap();
        if closed {
            scheduler::finish_child(&db, &lease, &child, false, "never dispatched").unwrap();
        } else {
            db.execute("UPDATE agent_assignment_attempts SET expires_at=datetime('now','-1 day','localtime') WHERE child_run_id=?1",[&child.run_id]).unwrap();
        }
        let worker =
            crate::agent_runtime::multi_agent::attempts::current(&db, &lease, &child.assignment_id)
                .unwrap();
        let before = budget::balance(&db, &id, None, "wall_time_ms").unwrap();
        std::thread::sleep(Duration::from_millis(20));
        let tx = db.unchecked_transaction().unwrap();
        budget::clock::finish_sample(&tx, &lease).unwrap();
        tx.commit().unwrap();
        let after = budget::balance(&db, &id, None, "wall_time_ms").unwrap();
        assert!(after.consumed > before.consumed);
        assert_eq!((after.reserved, after.indeterminate), (0, 0));
        assert_eq!(
            crate::agent_runtime::multi_agent::attempts::current(&db, &lease, &child.assignment_id)
                .unwrap(),
            worker
        );
        let snapshot = receipt_database_snapshot(&db);
        let tx = db.unchecked_transaction().unwrap();
        assert!(budget::append(
            &tx,
            &lease,
            &child.assignment_id,
            "model_requests",
            budget::Kind::Reserve,
            1,
            "late-work",
            "late-work"
        )
        .is_err());
        tx.rollback().unwrap();
        assert_eq!(receipt_database_snapshot(&db), snapshot);
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}
