#[test]
fn root_budget_estimate_must_fit_before_actual_provider_dispatch() {
    let mut harness = root_budget_harness("root-estimate-over-budget");
    let db = db::open(&harness.db_path).unwrap();
    db.execute(
        "UPDATE agent_runs SET hard_token_budget=1 WHERE id=?1",
        [&harness.context.run.as_ref().unwrap().run_id],
    )
    .unwrap();
    root_budget_fixture_owner_for_test(&harness);
    let before = super::tests::application_table_snapshot(&db);
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = calls.clone();
    let (port, _seen, _stop) = spawn_endpoint(std::sync::Arc::new(move |_| {
        observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        (200, "application/json", model_round(&[], 20))
    }));
    harness.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let client = AgentModelClient::new(
        agent_model_profile(&harness.context.environment, None).unwrap(),
        &[],
    );
    assert!(native_model_transport(&harness.context, &client, vec![json!({"role":"user","content":"Root cannot dispatch an estimate larger than the original remaining allowance"})], &[], 1).is_err());
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "shrinking a reservation does not shrink the provider request"
    );
    assert!(super::tests::application_table_snapshot(&db) == before);
    drop(db);
    fs::remove_dir_all(harness.root).unwrap();
}

#[test]
fn root_budget_unknown_execution_policy_cannot_bypass_claim_or_send() {
    let mut harness = root_budget_owned_fixture_for_test("root-invalid-policy");
    let db = db::open(&harness.db_path).unwrap();
    db.execute(
        "UPDATE agent_runs SET orchestration_policy='invalid' WHERE id=?1",
        [&harness.context.run.as_ref().unwrap().run_id],
    )
    .unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = calls.clone();
    let (port, _seen, _stop) = spawn_endpoint(std::sync::Arc::new(move |_| {
        observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        (
            500,
            "application/json",
            r#"{"error":{"message":"must not send"}}"#.into(),
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
        vec![json!({"role":"user","content":"invalid policy"})],
        &[],
        1
    )
    .is_err());
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "unknown Native policy bypassed durable Root/worker admission"
    );
    assert!(super::tests::application_table_snapshot(&db) == before);
    drop(db);
    fs::remove_dir_all(harness.root).unwrap();
}

#[test]
fn root_budget_single_invoice_precedes_output_and_releases_only_call_estimate() {
    use crate::agent_runtime::multi_agent::budget;
    let mut harness = root_budget_owned_fixture_for_test("root-known-invoice");
    let path = harness.db_path.clone();
    let root = harness.context.run.as_ref().unwrap().run_id.clone();
    let observed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let claimed = observed.clone();
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(move |request| {
        let db = db::open(&path).unwrap();
        let exists:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_root_model_journal j JOIN agent_root_budget_attempts a
            ON a.id=j.lease_attempt_id AND a.root_run_id=j.root_run_id WHERE j.root_run_id=?1 AND phase='dispatch' AND round=1)",[&root],|r|r.get(0)).unwrap();
        let body: JsonValue =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        claimed.store(
            exists && body["max_tokens"] == 8192,
            std::sync::atomic::Ordering::SeqCst,
        );
        (200, "application/json", model_round(&[], 20))
    }));
    harness.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let client = AgentModelClient::new(
        agent_model_profile(&harness.context.environment, None).unwrap(),
        &[],
    );
    let result = native_model_transport(
        &harness.context,
        &client,
        vec![json!({"role":"user","content":"known Root invoice"})],
        &[],
        1,
    );
    let Ok(response) = result else {
        panic!("known Root invoice must return executable output")
    };
    assert_eq!(response.usage.total_tokens, 60);
    assert!(observed.load(std::sync::atomic::Ordering::SeqCst));
    let db = db::open(&harness.db_path).unwrap();
    let root = &harness.context.run.as_ref().unwrap().run_id;
    for (dimension, amount) in [
        ("model_input_tokens", 20),
        ("model_cached_tokens", 0),
        ("model_output_tokens", 40),
        ("model_requests", 1),
    ] {
        let b = budget::balance(&db, root, Some(""), dimension).unwrap();
        assert_eq!(
            (b.reserved, b.consumed, b.indeterminate),
            (0, amount, 0),
            "{dimension}"
        );
    }
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_budget_limits WHERE root_run_id=?1",
            [root],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        10
    );
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='received'",[root],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(seen.lock().unwrap().len(), 1);
    let before = super::tests::application_table_snapshot(&db);
    assert!(native_model_transport(
        &harness.context,
        &client,
        vec![json!({"role":"user","content":"same round"})],
        &[],
        1
    )
    .is_err());
    assert!(super::tests::application_table_snapshot(&db) == before);
    drop(db);
    fs::remove_dir_all(harness.root).unwrap();
}

#[test]
fn root_budget_single_missing_usage_keeps_tokens_unknown_and_original_invoice() {
    use crate::agent_runtime::multi_agent::budget;
    let mut harness = root_budget_owned_fixture_for_test("root-missing-usage");
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            r#"{"choices":[{"message":{"content":"missing usage"},"finish_reason":"stop"}]}"#
                .into(),
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
        vec![json!({"role":"user","content":"missing usage"})],
        &[],
        1
    )
    .is_err());
    let db = db::open(&harness.db_path).unwrap();
    let root = &harness.context.run.as_ref().unwrap().run_id;
    for dimension in budget::DIMENSIONS.iter().take(3) {
        let b = budget::balance(&db, root, Some(""), dimension).unwrap();
        assert_eq!((b.reserved, b.consumed), (0, 0));
        assert!(b.indeterminate > 0);
    }
    let requests = budget::balance(&db, root, Some(""), "model_requests").unwrap();
    assert_eq!(
        (requests.reserved, requests.consumed, requests.indeterminate),
        (0, 1, 0)
    );
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert!(!db.query_row("SELECT json_extract(receipt_json,'$.usageReported') FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='received'",[root],|r|r.get::<_,bool>(0)).unwrap());
    assert!(budget::admission::require_determinate(&db, root).is_err());
    drop(db);
    fs::remove_dir_all(harness.root).unwrap();
}
