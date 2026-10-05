// Actual original Root SDK errors retain their financial facts at the consumer.
fn coordinator_unknown_bill_original_boundary(transport: bool) {
    let _real = RealSpecialistTransport::enter();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |_| {
        if transport {
            (
                503,
                "application/json",
                json!({"error":{"message":"temporary provider failure"}}).to_string(),
            )
        } else {
            let mut response: JsonValue = serde_json::from_str(&proposal_model_response(
                &root_tick_valid_text("paid but unknown usage"),
            ))
            .unwrap();
            response.as_object_mut().unwrap().remove("usage");
            (200, "application/json", response.to_string())
        }
    }));
    let mut f = root_tick_fixture(
        "root-unknown-outcome",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    drop(f.parent.take());
    let error = multi_agent_prepare(&mut f.context)
        .err()
        .expect("unknown cost cannot dispatch Mapper");
    if transport {
        assert!(error.starts_with("root_tick_uncertain:"), "{error}");
    } else {
        assert_eq!(error, "budget_indeterminate_requires_reconciliation");
    }
    let db = db::open(&f.context.db_path).unwrap();
    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "one original provider call, no implicit retry"
    );
    let cost = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &f.actor.root_run_id,
        None,
        "model_requests",
    )
    .unwrap();
    assert_eq!(cost.consumed, if transport { 0 } else { 1 });
    assert_eq!(cost.indeterminate, if transport { 1 } else { 0 });
    assert!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &f.actor.root_run_id,
            None,
            "model_input_tokens"
        )
        .unwrap()
        .indeterminate
            > 0
    );
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "request"), 1);
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 0);
    for table in [
        "agent_assignments",
        "agent_capability_leases",
        "agent_http_request_claims",
        "agent_specialist_calls",
    ] {
        assert_eq!(
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0,
            "{table}"
        );
    }
    let before = web_mode_test_rows(&db);
    for _ in 0..2 {
        let outcome = multi_agent_bootstrap_outcome(&error);
        assert!(
            matches!(outcome, AgentTargetOutcome::Incomplete(_)),
            "{error}: {outcome:?}"
        );
        let stop = outcome.stop().unwrap();
        assert_eq!(stop.code, terminal_code::REQUEST_RECONCILIATION_REQUIRED);
        assert!(!stop.requires_fuse());
        assert!(stop.reason.contains(&error));
    }
    let page = root_tick_sdk_page(&f.context.db_path, &f.context.scan_id);
    assert_eq!(
        root_tick_sdk_rows(&page, 1).last().unwrap()["terminalState"],
        if transport { "uncertain" } else { "withheld" }
    );
    web_mode_assert_rows(&db, &before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}
#[test]
fn coordinator_unknown_bill_missing_actual_usage_remains_incomplete_without_refund() {
    coordinator_unknown_bill_original_boundary(false);
}
#[test]
fn coordinator_unknown_bill_actual_provider_failure_remains_incomplete_without_retry() {
    coordinator_unknown_bill_original_boundary(true);
}
#[test]
fn coordinator_unknown_bill_narrative_or_corruption_does_not_gain_reconciliation_status() {
    for code in [
        "model said budget_indeterminate_requires_reconciliation",
        "budget_indeterminate_requires_reconciliation_fake",
        "root_tick_uncertain",
        "root_tick_uncertain:",
        "budget_projection_balance_conflict",
        "root_tick_original_input_conflict",
    ] {
        assert!(
            matches!(
                multi_agent_bootstrap_outcome(code),
                AgentTargetOutcome::Failed(_)
            ),
            "{code}"
        );
    }
}
