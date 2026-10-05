#[test]
fn web_mode_single_real_user_entry_uses_native_sdk_without_any_coordinator_or_team() {
    let (site_port, site_seen, _site_stop) = spawn_endpoint(std::sync::Arc::new(mock_site));
    let fixture = web_mode_fixture("single", &format!("http://127.0.0.1:{site_port}"));
    let (model_port, _model_stop, model_seen) = spawn_model(vec![model_round(
        &[(
            "finish_target",
            serde_json::json!({
        "coverage":closing_ledger(&[]),"stopReason":"single original frozen budget"}),
        )],
        100,
    )]);
    let mut context = test_context(
        &fixture.path,
        &fixture.target,
        vec![AgentIdentity::anonymous()],
    );
    context.scan_id = fixture.scan.clone();
    context.environment.api_base = format!("http://127.0.0.1:{model_port}/v1");
    context.target_dir = fixture.work.join("url-pipeline/target-00001");
    fs::create_dir_all(&context.target_dir).unwrap();
    fs::write(context.target_dir.join(".oviraptor-scan-id"), &fixture.scan).unwrap();
    fs::write(
        context.target_dir.join("frontend-evidence.json"),
        context.evidence.to_string(),
    )
    .unwrap();
    context.log_path = fixture.work.join("oviraptor-runner.log");
    let prepared = PreparedFrontendTarget {
        position: 1,
        route: context.route.clone(),
        target_dir: context.target_dir.clone(),
        proxy: None,
        browser: None,
    };
    let settings = serde_json::json!({"agentBackendPolicy":"native"});
    let adaptive = AgentBudgetSettings::from_json(&settings);
    let _real = RealSpecialistTransport::enter();
    let outcome = run_agent_target(
        &prepared,
        AgentTargetExecution {
            db_path: &fixture.path,
            scan_id: &fixture.scan,
            attempt_number: 1,
            settings: &settings,
            environment: &context.environment,
            adaptive: &adaptive,
            log_path: &context.log_path,
        },
    )
    .unwrap();
    assert!(
        !matches!(
            outcome.outcome,
            AgentTargetOutcome::Failed(_) | AgentTargetOutcome::ResumeIncompatible(_)
        ),
        "{:?}",
        outcome.outcome
    );
    let db = db::open(&fixture.path).unwrap();
    let actual_calls = model_seen.lock().unwrap().len();
    let frozen_text: String = db
        .query_row(
            "SELECT plan_json FROM agent_runs WHERE scan_id=?1 AND role='coordinator'",
            [&fixture.scan],
            |row| row.get(0),
        )
        .unwrap();
    let frozen =
        AgentExecutionPlan::from_json(&serde_json::from_str(&frozen_text).unwrap()).unwrap();
    // An early finish with uncovered families is rejected by the original loop.
    // Count every physical call and invoice; do not relax that coverage policy.
    assert!(actual_calls > 0 && i64::try_from(actual_calls).unwrap() <= frozen.hard_model_requests);
    assert_eq!(site_seen.lock().unwrap().len(), 0);
    for table in [
        "agent_coordinator_leases",
        "agent_assignments",
        "agent_assignment_attempts",
        "agent_user_directives",
        "agent_budget_ledger",
        "agent_specialist_calls",
        "agent_messages",
    ] {
        assert_eq!(
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0,
            "{table}"
        );
    }
    assert_eq!(db.query_row("SELECT count(*) FROM agent_runs WHERE role='coordinator' AND orchestration_policy='single'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(
        db.query_row("SELECT count(*) FROM agent_budget_limits", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        10,
        "original Single ten dimensions"
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_root_model_journal WHERE phase='received'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        i64::try_from(actual_calls).unwrap()
    );
}

#[test]
fn web_mode_single_rejects_multi_prepare_before_any_write_or_sdk() {
    let fixture = web_mode_fixture("single", "https://mode.example.test/app");
    let mut context = test_context(&fixture.path, &fixture.target, vec![]);
    context.scan_id = fixture.scan.clone();
    persist_frozen_web_execution_plan(
        &fixture.path,
        &fixture.scan,
        1,
        &fixture.target,
        &context.execution_plan,
    )
    .unwrap();
    context.run = runtime_open_run(&fixture.path, &fixture.scan, &context.route);
    let db = db::open(&fixture.path).unwrap();
    let before = web_mode_test_rows(&db);
    assert!(multi_agent_prepare(&mut context).is_err());
    web_mode_assert_rows(&db, &before);
    assert!(take_human_directives(&context).unwrap().lease.is_none());
    web_mode_assert_rows(&db, &before);
}

#[test]
fn web_mode_duplicate_multi_prepare_during_real_sdk_changes_no_application_row() {
    let fixture = web_mode_fixture("multi", "https://mode.example.test/app");
    let mut observer = test_context(&fixture.path, &fixture.target, vec![]);
    observer.scan_id = fixture.scan.clone();
    observer.target_dir = fixture.work.join("url-pipeline/target-00001");
    fs::create_dir_all(&observer.target_dir).unwrap();
    fs::write(
        observer.target_dir.join(".oviraptor-scan-id"),
        &fixture.scan,
    )
    .unwrap();
    fs::write(observer.target_dir.join("frontend-evidence.json"), observer.evidence.to_string()).unwrap();
    observer.execution_plan.hard_total_tokens = 60_000;
    observer.execution_plan.soft_uncached_tokens = 30_000;
    observer.execution_plan.hard_model_requests = 20;
    observer.execution_plan.soft_model_requests = 10;
    let (arrived_tx, arrived_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let release_rx = std::sync::Mutex::new(release_rx);
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(move |request| {
        if !request.contains("mapper-output") && request.contains("You are the Root Coordinator") {
        arrived_tx.send(()).unwrap();
        release_rx
            .lock()
            .unwrap()
            .recv_timeout(std::time::Duration::from_secs(30))
            .unwrap();
        }
        (
            200,
            "application/json",
            changed_fact_response(&request, true),
        )
    }));
    observer.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    persist_frozen_web_execution_plan(
        &fixture.path,
        &fixture.scan,
        1,
        &fixture.target,
        &observer.execution_plan,
    )
    .unwrap();
    observer.run = runtime_open_run(&fixture.path, &fixture.scan, &observer.route);
    // Independent observer input captured before the owner can mutate its context.
    let mut owner = observer.clone();
    let worker = std::thread::spawn(move || {
        let _real = RealSpecialistTransport::enter();
        multi_agent_prepare(&mut owner)
    });
    let arrived = arrived_rx.recv_timeout(std::time::Duration::from_secs(30));
    if let Err(error) = arrived {
        let _ = release_tx.send(());
        let owner_result = worker.join().unwrap();
        panic!("owner did not reach real SDK: {error}; {:?}", owner_result.err());
    }
    let db = db::open(&fixture.path).unwrap();
    let before = web_mode_test_rows(&db);
    let _real = RealSpecialistTransport::enter();
    let duplicate = multi_agent_prepare(&mut observer);
    let after = web_mode_test_rows(&db);
    // Always release and join before assertions; an assertion cannot strand the paid SDK owner.
    release_tx.send(()).unwrap();
    let result = worker.join().unwrap();
    assert!(duplicate
        .err()
        .unwrap()
        .starts_with("native_invocation_not_owned:"));
    assert!(before==after,"duplicate prepare changed the complete application snapshot while SDK response/fee was still pending");
    assert_eq!(seen.lock().unwrap().len(), 3, "actual Root/Mapper/changed-fact Root SDK");
    assert!(result.is_ok(), "{:?}", result.err());
    drop(result);
}

#[test]
fn web_mode_parent_owner_scope_cannot_be_passed_to_another_root() {
    use crate::agent_runtime::multi_agent::parent_invocation_owner::ParentInvocationOwner;
    let fixture = web_mode_fixture("multi", "https://mode.example.test/app");
    let owner =
        ParentInvocationOwner::claim(&fixture.path, &fixture.scan, 1, "original-root").unwrap();
    let db = db::open(&fixture.path).unwrap();
    let before = web_mode_test_rows(&db);
    assert!(owner
        .validate_scope(&fixture.path, &fixture.scan, 1, "other-root")
        .is_err());
    assert!(owner
        .validate_scope(&fixture.path, &fixture.scan, 2, "original-root")
        .is_err());
    web_mode_assert_rows(&db, &before);
}
