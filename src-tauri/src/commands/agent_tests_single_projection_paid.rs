// Real localhost provider calls with original financial receipts, no target I/O.
#[test]
fn single_finally_actual_unknown_paid_usage_cannot_publish_completed() {
    let mut h = root_budget_owned_fixture_for_test("single-projection-unknown-sdk");
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            r#"{"choices":[{"message":{"content":"unknown usage"},"finish_reason":"stop"}]}"#
                .into(),
        )
    }));
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let client = AgentModelClient::new(
        agent_model_profile(&h.context.environment, None).unwrap(),
        &[],
    );
    assert!(native_model_transport(
        &h.context,
        &client,
        vec![json!({"role":"user","content":"unknown invoice"})],
        &[],
        1
    )
    .is_err());
    assert_eq!(seen.lock().unwrap().len(), 1);
    let db = db::open(&h.db_path).unwrap();
    let root = &h.context.run.as_ref().unwrap().run_id;
    assert!(
        crate::agent_runtime::multi_agent::budget::balance(&db, root, None, "model_input_tokens")
            .unwrap()
            .indeterminate
            > 0
    );
    let old = single_finally_original_sources(&db, root);
    let reduced = record_runtime_terminal_facts_checked(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        &single_projection_outcome(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        reduced.state,
        crate::agent_runtime::contract::TerminalState::Incomplete,
        "unresolved real SDK spend cannot count as completion"
    );
    assert_eq!(reduced.code, terminal_code::REQUEST_RECONCILIATION_REQUIRED);
    assert_eq!(single_finally_original_sources(&db, root), old);
    assert_eq!(seen.lock().unwrap().len(), 1);
    drop(db);
    single_target_cleanup(h);
}
#[test]
fn single_finally_actual_known_invoice_snapshot_reads_original_token_classes() {
    let mut h = root_budget_owned_fixture_for_test("single-projection-known-sdk");
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (200, "application/json", model_round(&[], 20))
    }));
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let client = AgentModelClient::new(
        agent_model_profile(&h.context.environment, None).unwrap(),
        &[],
    );
    let reply = native_model_transport(
        &h.context,
        &client,
        vec![json!({"role":"user","content":"known invoice"})],
        &[],
        1,
    )
    .unwrap_or_else(|_| panic!("original known SDK invoice must return"));
    assert_eq!(
        (reply.usage.input_tokens, reply.usage.output_tokens),
        (20, 40)
    );
    record_runtime_terminal_facts_checked(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        &single_projection_outcome(),
    )
    .unwrap();
    let db = db::open(&h.db_path).unwrap();
    let root = &h.context.run.as_ref().unwrap().run_id;
    let text: String = db
        .query_row(
            "SELECT snapshot_json FROM agent_snapshots WHERE run_id=?1",
            [root],
            |r| r.get(0),
        )
        .unwrap();
    let snapshot: JsonValue = serde_json::from_str(&text).unwrap();
    assert_eq!(
        snapshot["modelRequests"], 1,
        "do not replace a paid invoice with completion's zero summary"
    );
    assert_eq!(snapshot["inputTokens"], 20);
    assert_eq!(snapshot["outputTokens"], 40);
    assert_eq!(snapshot["usedTokens"], 60);
    assert_eq!(seen.lock().unwrap().len(), 1);
    drop(db);
    single_target_cleanup(h);
}
