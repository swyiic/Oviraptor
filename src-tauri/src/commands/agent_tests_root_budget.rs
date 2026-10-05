fn root_budget_harness(tag: &str) -> AgentHarness {
    let mut harness = agent_harness(tag, mock_site, vec![AgentIdentity::anonymous()]);
    freeze_harness_plan(&harness);
    harness.context.run = runtime_open_run(
        &harness.db_path,
        &harness.context.scan_id,
        &harness.context.route,
    );
    assert!(harness.context.run.is_some());
    harness
}

// Explicit lower-level accounting fixtures. Not a production creation or UI
// acceptance path: the live callee and all missing-owner fixtures remain pure.
fn root_budget_fixture_owner_for_test(harness:&AgentHarness) {
    let db=db::open(&harness.db_path).unwrap();let tx=db.unchecked_transaction().unwrap();
    crate::agent_runtime::multi_agent::budget::root::RootOwner::initialize_financial_fixture_for_test(
        &tx,&harness.context.run.as_ref().unwrap().run_id).unwrap();
    tx.commit().unwrap();
}
fn root_budget_owned_fixture_for_test(tag:&str)->AgentHarness {
    let harness=root_budget_harness(tag);root_budget_fixture_owner_for_test(&harness);harness
}

#[test]
fn root_budget_single_model_unknown_transport_has_one_dispatch_and_original_debt() {
    let mut harness = root_budget_owned_fixture_for_test("root-model-debt");
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = calls.clone();
    let (port, _seen, _stop) = spawn_endpoint(std::sync::Arc::new(move |_| {
        observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        (
            500,
            "application/json",
            r#"{"error":{"message":"original-root-model-failure"}}"#.into(),
        )
    }));
    harness.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let client = AgentModelClient::new(
        agent_model_profile(&harness.context.environment, None).unwrap(),
        &[],
    );
    let result = native_model_transport(
        &harness.context,
        &client,
        vec![json!({"role":"user","content":"one original Root call"})],
        &[],
        1,
    );
    assert!(result.is_err());
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "Single Root model work must never silently retry an unknown provider outcome"
    );
    let db = db::open(&harness.db_path).unwrap();
    let root_run = &harness.context.run.as_ref().unwrap().run_id;
    let known: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='agent_root_model_journal')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(known, "Root work must have its own durable model journal");
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='uncertain'", [root_run], |r|r.get::<_,i64>(0)).unwrap(), 1);
    let debt =
        crate::agent_runtime::multi_agent::budget::balance(&db, root_run, None, "model_requests")
            .unwrap();
    assert_eq!(
        (debt.reserved, debt.consumed, debt.indeterminate),
        (0, 0, 1)
    );
    for table in [
        "agent_assignments",
        "agent_assignment_attempts",
        "agent_coordinator_leases",
    ] {
        assert_eq!(
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    let before = super::tests::application_table_snapshot(&db);
    assert!(native_model_transport(
        &harness.context,
        &client,
        vec![json!({"role":"user","content":"must not resend"})],
        &[],
        1
    )
    .is_err());
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(super::tests::application_table_snapshot(&db) == before);
    drop(db);
    fs::remove_dir_all(harness.root).unwrap();
}

#[test]
fn root_budget_single_claim_rejects_trigger_business_effects_before_provider_io() {
    let mut harness = root_budget_owned_fixture_for_test("root-claim-collateral");
    let db = db::open(&harness.db_path).unwrap();
    db.execute_batch("CREATE TRIGGER root_claim_collateral AFTER INSERT ON agent_budget_entries WHEN NEW.dimension='model_requests' AND NEW.kind='reserve' BEGIN INSERT INTO projects(name) VALUES ('unapproved-root-claim-effect'); END;").unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = calls.clone();
    let (port, _seen, _stop) = spawn_endpoint(std::sync::Arc::new(move |_| {
        observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        (
            500,
            "application/json",
            r#"{"error":{"message":"must not be called"}}"#.into(),
        )
    }));
    harness.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let client = AgentModelClient::new(
        agent_model_profile(&harness.context.environment, None).unwrap(),
        &[],
    );
    assert!(native_model_transport(
        &harness.context,
        &client,
        vec![json!({"role":"user","content":"one Root call"})],
        &[],
        1
    )
    .is_err());
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "budget admission must not commit unrelated business effects and send"
    );
    assert!(super::tests::application_table_snapshot(&db) == before);
    drop(db);
    fs::remove_dir_all(harness.root).unwrap();
}

#[test]
fn root_budget_stopped_scan_cannot_claim_root_work_or_create_accounting_authority() {
    use crate::agent_runtime::multi_agent::budget::root::model::RootModelCall;
    let harness = root_budget_owned_fixture_for_test("root-stopped-scan");
    let db = db::open(&harness.db_path).unwrap();
    db.execute(
        "UPDATE sentinel_scans SET status='paused' WHERE id=?1",
        [&harness.context.scan_id],
    )
    .unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    let rejected = RootModelCall::claim(
        &tx,
        &harness.context.run.as_ref().unwrap().run_id,
        1,
        &"a".repeat(64),
        1000,
    );
    assert!(
        rejected.is_err(),
        "a live Root row cannot grant work after its scan stopped"
    );
    assert!(super::tests::application_table_snapshot(&tx) == before);
    tx.rollback().unwrap();
    drop(db);
    fs::remove_dir_all(harness.root).unwrap();
}
