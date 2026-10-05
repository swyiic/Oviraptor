// Actual paid Root and Mapper transport; only temporary creator data/localhost SDK.
fn mapper_allocation_fixture(
    limits: (i64, i64),
) -> (
    RootTickFixture,
    NativeCoordinatorTickReceipt,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) {
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|request| {
        (
            200,
            "application/json",
            changed_fact_response(&request, true),
        )
    }));
    let f = root_tick_fixture_protocol_limits(
        "mapper-allocation",
        &format!("http://127.0.0.1:{port}/v1"),
        true,
        limits,
    );
    let decision = native_coordinator_tick(&f.context, &f.actor).unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1);
    (f, decision, seen)
}

#[test]
fn coordinator_mapper_allocation_actual_sdk_preserves_reviewer_and_releases_unused_grant() {
    let _real = RealSpecialistTransport::enter();
    let (f, decision, seen) = mapper_allocation_fixture((23_000, 20));
    let task = bootstrap_mapper_task(&f);
    let mapper = native_coordinator_prepare_mapper(&f.context, &f.actor, &decision, &task).unwrap();
    let db = db::open(&f.context.db_path).unwrap();
    let remaining = crate::agent_runtime::multi_agent::budget::root::remaining_child_capacity(
        &db,
        &f.actor.root_run_id,
    )
    .unwrap();
    assert_eq!(
        remaining.0,
        Some(15_000),
        "paid Root leaves 22980; Mapper must preserve original Reviewer floor"
    );
    let tokens: i64 = db
        .query_row(
            "SELECT reserved_tokens FROM agent_assignments WHERE id=?1",
            [&mapper.assignment_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tokens, 7_980);
    let before = web_mode_test_rows(&db);
    assert_eq!(
        native_coordinator_prepare_mapper(&f.context, &f.actor, &decision, &task).unwrap(),
        mapper
    );
    web_mode_assert_rows(&db, &before);
    let (text, usage) = multi_agent_child_round(
        &f.context,
        &f.actor,
        &mapper,
        "Independent Mapper reads original frontend evidence",
        json!({"frozenEvidence":f.context.evidence}),
    )
    .unwrap();
    assert_eq!(usage.total_tokens, 20);
    assert_eq!(seen.lock().unwrap().len(), 2);
    deliver_readonly_assessment(
        &db,
        &f.actor,
        &mapper,
        &usage,
        &json!({"summary":text}),
        Some(&f.context.target_dir),
    )
    .unwrap();
    let remaining = crate::agent_runtime::multi_agent::budget::root::remaining_child_capacity(
        &db,
        &f.actor.root_run_id,
    )
    .unwrap();
    assert_eq!(
        remaining,
        (Some(22_960), Some(18)),
        "only actual Root/Mapper invoices remain occupied"
    );
    let before = web_mode_test_rows(&db);
    assert_eq!(
        native_coordinator_prepare_mapper(&f.context, &f.actor, &decision, &task).unwrap(),
        mapper
    );
    web_mode_assert_rows(&db, &before);
    assert_eq!(
        seen.lock().unwrap().len(),
        2,
        "replay after release never grows original grant or calls SDK"
    );
}

#[test]
fn coordinator_mapper_allocation_concurrent_paid_admission_issues_one_original_worker() {
    let (f, decision, seen) = mapper_allocation_fixture((23_000, 20));
    let task = bootstrap_mapper_task(&f);
    let barrier = std::sync::Barrier::new(2);
    let (first, second) = std::thread::scope(|scope| {
        let one = scope.spawn(|| {
            barrier.wait();
            native_coordinator_prepare_mapper(&f.context, &f.actor, &decision, &task)
        });
        let two = scope.spawn(|| {
            barrier.wait();
            native_coordinator_prepare_mapper(&f.context, &f.actor, &decision, &task)
        });
        (one.join().unwrap().unwrap(), two.join().unwrap().unwrap())
    });
    assert_eq!(first, second);
    let db = db::open(&f.context.db_path).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM agent_assignment_attempts WHERE assignment_id=?1",
            [&first.assignment_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(db.query_row("SELECT COUNT(*) FROM agent_budget_entries WHERE assignment_id=?1 AND kind='reserve' AND dimension IN ('model_input_tokens','model_cached_tokens','model_output_tokens','model_requests')",[&first.assignment_id],|r|r.get::<_,i64>(0)).unwrap(),4);
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::root::remaining_child_capacity(
            &db,
            &f.actor.root_run_id
        )
        .unwrap(),
        (Some(15_000), Some(18))
    );
    let before = web_mode_test_rows(&db);
    assert_eq!(
        native_coordinator_prepare_mapper(&f.context, &f.actor, &decision, &task).unwrap(),
        first
    );
    web_mode_assert_rows(&db, &before);
    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "two concurrent grants do not mean specialist SDK parallel execution"
    );
}
