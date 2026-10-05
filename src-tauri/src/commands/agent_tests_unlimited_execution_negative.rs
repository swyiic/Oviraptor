#[test]
fn unlimited_execution_actual_unknown_root_or_web_sdk_keeps_fee_debt_and_stops_target() {
    let _real = RealSpecialistTransport::enter();
    for reply in [1, 2] {
        let f = unlimited_execution_fixture((0, 0), reply);
        let db = db::open(&f.h.db_path).unwrap();
        let s = f.session.as_ref().unwrap();
        let outcome = run_native_agent(&f.h.context);
        assert_eq!(
            outcome.terminal_code(),
            terminal_code::REQUEST_RECONCILIATION_REQUIRED,
            "{}",
            outcome.detail()
        );
        assert_eq!(f.h.site_seen.lock().unwrap().len(), 0);
        assert_eq!(
            f.h.model_seen.lock().unwrap().len(),
            if reply == 1 { 4 } else { 5 }
        );
        let requests = crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &s.lease.root_run_id,
            None,
            "model_requests",
        )
        .unwrap();
        assert_eq!(requests.indeterminate, 1);
        assert!(requests.consumed > 0);
        assert!(
            crate::agent_runtime::multi_agent::budget::balance(
                &db,
                &s.lease.root_run_id,
                None,
                "model_input_tokens"
            )
            .unwrap()
            .indeterminate
                > 0
        );
        assert_eq!(
            crate::agent_runtime::multi_agent::budget::model::original_web_grant(
                &db,
                &s.lease,
                &s.executor.assignment_id
            )
            .unwrap(),
            (0, 0)
        );
        let before = web_mode_test_rows(&db);
        let calls = f.h.model_seen.lock().unwrap().len();
        assert!(native_coordinator_budget_before_executor(&f.h.context).is_err());
        web_mode_assert_rows(&db, &before);
        assert_eq!(f.h.model_seen.lock().unwrap().len(), calls);
    }
}
#[test]
fn unlimited_execution_missing_estimate_stale_call_and_policy_tamper_refuse_zero_write() {
    let _real = RealSpecialistTransport::enter();
    let f = unlimited_execution_fixture((0, 0), 0);
    let db = db::open(&f.h.db_path).unwrap();
    let before = web_mode_test_rows(&db);
    assert!(
        matches!(native_model_budget_admission(&f.h.context,1,"not-an-estimated-request".into()),Err(code) if code=="budget_unbounded_model_estimate_missing")
    );
    web_mode_assert_rows(&db, &before);
    assert!(native_model_budget_admission_with_estimate(
        &f.h.context,
        2,
        "stale-round".into(),
        Some(8192)
    )
    .is_err());
    web_mode_assert_rows(&db, &before);
    let id = &f.session.as_ref().unwrap().executor.assignment_id;
    db.execute("UPDATE agent_assignments SET task_slice_json=json_set(task_slice_json,'$.rootDecision.rustPolicy.modelCostRule','free_model_calls') WHERE id=?1",[id]).unwrap();
    let corrupted = web_mode_test_rows(&db);
    assert!(native_coordinator_budget_before_executor(&f.h.context).is_err());
    web_mode_assert_rows(&db, &corrupted);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), 3);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 0);
}
#[test]
fn unlimited_execution_unbounded_reservation_refuses_business_escape_before_any_sdk() {
    let _real = RealSpecialistTransport::enter();
    for trigger in [
        "CREATE TRIGGER escape_unbounded_reserve BEFORE INSERT ON agent_budget_entries WHEN NEW.kind='reserve' AND NEW.source_id LIKE 'web-model:%' BEGIN SELECT RAISE(ABORT,'reserve unavailable'); END;",
        "CREATE TRIGGER escape_unbounded_reserve BEFORE INSERT ON agent_budget_entries WHEN NEW.kind='reserve' AND NEW.source_id LIKE 'web-model:%' BEGIN SELECT RAISE(FAIL,'reserve unavailable'); END;",
        "CREATE TRIGGER escape_unbounded_reserve BEFORE INSERT ON agent_budget_entries WHEN NEW.kind='reserve' AND NEW.source_id LIKE 'web-model:%' BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER escape_unbounded_reserve AFTER INSERT ON agent_budget_entries WHEN NEW.kind='reserve' AND NEW.source_id LIKE 'web-model:%' BEGIN UPDATE projects SET name='escaped-financial-write'; END;",
    ] {
        let f = unlimited_execution_fixture((0, 0), 0);
        let db = db::open(&f.h.db_path).unwrap();
        db.execute_batch(trigger).unwrap();
        let before = web_mode_test_rows(&db);
        let result = native_model_budget_admission_with_estimate(&f.h.context,1,"protected-estimated-original-call".into(),Some(8192));
        assert!(result.is_err(), "reserve persistence must fail atomically: {trigger}");
        web_mode_assert_rows(&db, &before);
        assert_eq!(f.h.model_seen.lock().unwrap().len(), 3);
        assert_eq!(f.h.site_seen.lock().unwrap().len(), 0);
    }
}
#[test]
fn unlimited_execution_budget_advisory_cannot_dispatch_another_target_worker() {
    let _real = RealSpecialistTransport::enter();
    let f = unlimited_execution_fixture((0, 0), 4);
    let outcome = run_native_agent(&f.h.context);
    assert!(
        outcome.detail().contains("root_budget_step_not_bounded"),
        "{}",
        outcome.detail()
    );
    assert_eq!(f.h.model_seen.lock().unwrap().len(), 4);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 0);
    let db = db::open(&f.h.db_path).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_assignments WHERE role='web_executor'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn unlimited_execution_native_current_request_estimate_keeps_low_original_ceiling_without_sdk() {
    let _real = RealSpecialistTransport::enter();
    let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(|request| {
        (
            200,
            "application/json",
            changed_fact_response(&request, true),
        )
    }));
    let f = root_tick_fixture_protocol_limits(
        "native5-estimate-floor",
        &format!("http://127.0.0.1:{port}/v1"),
        true,
        (22_000, 20),
    );
    let db = db::open(&f.context.db_path).unwrap();
    let before = web_mode_test_rows(&db);
    assert!(
        matches!(native_coordinator_tick(&f.context,&f.actor),Err(code) if code=="child_budget_reservation_exceeded_or_stale")
    );
    web_mode_assert_rows(&db, &before);
    assert!(seen.lock().unwrap().is_empty());
    drop(stop);
}
