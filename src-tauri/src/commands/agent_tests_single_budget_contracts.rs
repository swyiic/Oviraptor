// These contracts run after the red-first localhost tests and minimal patch.
#[test]
fn single_budget_original_owner_load_is_read_only_and_cannot_adopt_partial_history() {
    use crate::agent_runtime::multi_agent::budget::root::RootOwner;
    let harness = single_target_harness("single-readonly-owner", 30);
    let db = db::open(&harness.db_path).unwrap();
    let root = &harness.context.run.as_ref().unwrap().run_id;
    let before = super::tests::application_table_snapshot(&db);
    let owner = RootOwner::load_single(&db, root).unwrap();
    owner.require_live(&db).unwrap();
    assert!(super::tests::application_table_snapshot(&db) == before);
    db.execute("UPDATE agent_runs SET status='paused' WHERE id=?1", [root])
        .unwrap();
    let stopped = super::tests::application_table_snapshot(&db);
    RootOwner::load_single(&db, root).unwrap();
    assert!(owner.require_live(&db).is_err());
    assert!(super::tests::application_table_snapshot(&db) == stopped);
    let other = root_budget_harness("single-missing-original-owner");
    let other_db = db::open(&other.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&other_db);
    assert!(
        RootOwner::load_single(&other_db, &other.context.run.as_ref().unwrap().run_id).is_err()
    );
    assert!(super::tests::application_table_snapshot(&other_db) == before);
    drop(other_db);
    drop(db);
    single_target_cleanup(other);
    single_target_cleanup(harness);
}

#[test]
fn single_budget_preparation_rejects_native_and_http_history_without_backfill() {
    for kind in ["tool", "checkpoint", "usage", "partial_limits"] {
        let harness = root_budget_harness("single-history-no-owner");
        let db = db::open(&harness.db_path).unwrap();
        let root = &harness.context.run.as_ref().unwrap().run_id;
        match kind {
            "tool" => {
                single_target_start(&harness.context);
            }
            "checkpoint" => {
                write_agent_checkpoint(
                    &harness.db_path,
                    &harness.context.scan_id,
                    &harness.context.target_url,
                    NATIVE_AGENT_STATE_STAGE,
                    &json!({"schemaVersion":2,"attemptNumber":1,"targetRequests":3}),
                )
                .unwrap();
            }
            "usage" => {
                db.execute("UPDATE agent_runs SET used_requests=1 WHERE id=?1", [root])
                    .unwrap();
            }
            _ => {
                db.execute("INSERT INTO agent_budget_limits(root_run_id,dimension,hard_limit) VALUES(?1,'target_requests',99)", [root]).unwrap();
            }
        }
        let before = super::tests::application_table_snapshot(&db);
        assert!(
            native_prepare_single_budget(&harness.context).is_err(),
            "{kind}"
        );
        assert!(
            super::tests::application_table_snapshot(&db) == before,
            "{kind}"
        );
        drop(db);
        single_target_cleanup(harness);
    }
}

#[test]
fn single_budget_http_claim_faults_rollback_original_journal_and_all_ten_dimensions() {
    for table in [
        "agent_http_budget_origins",
        "agent_http_request_claims",
        "agent_budget_entries",
    ] {
        for behavior in ["IGNORE", "ABORT", "business"] {
            let harness = single_target_harness("single-http-claim-fault", 30);
            let (mut runtime, request) = single_target_start(&harness.context);
            let db = db::open(&harness.db_path).unwrap();
            let effect = match behavior {
                "IGNORE" => "SELECT RAISE(IGNORE)",
                "ABORT" => "SELECT RAISE(ABORT,'single-http-fault')",
                _ => "INSERT INTO projects(name) VALUES('single-http-unrelated-write')",
            };
            db.execute_batch(&format!(
                "CREATE TRIGGER single_http_fault BEFORE INSERT ON {table} BEGIN {effect}; END;"
            ))
            .unwrap();
            let before = super::tests::application_table_snapshot(&db);
            assert!(
                agent_http_exchange(&harness.context, &mut runtime, &request).is_err(),
                "{table}/{behavior}"
            );
            assert!(harness.site_seen.lock().unwrap().is_empty());
            assert!(
                super::tests::application_table_snapshot(&db) == before,
                "{table}/{behavior}"
            );
            drop(db);
            drop(runtime);
            single_target_cleanup(harness);
        }
    }
}

#[test]
fn single_budget_late_original_header_fee_and_duplicate_receipt_keep_exact_scope() {
    let harness = single_target_harness("single-late-header-owner", 30);
    let (runtime, request) = single_target_start(&harness.context);
    let claim = claim_agent_http_request(&harness.context, &runtime, &request, &request.url)
        .unwrap()
        .unwrap();
    let db = db::open(&harness.db_path).unwrap();
    let root = &harness.context.run.as_ref().unwrap().run_id;
    db.execute("UPDATE agent_runs SET status='paused' WHERE id=?1", [root])
        .unwrap();
    let before = super::tests::application_table_snapshot(&db);
    receive_agent_http_headers(&harness.context, &claim, 429).unwrap();
    let after = super::tests::application_table_snapshot(&db);
    for (table, rows) in &before {
        if !matches!(
            table.as_str(),
            "agent_http_request_claims" | "agent_budget_entries"
        ) {
            assert_eq!(
                after
                    .iter()
                    .find(|(name, _)| name == table)
                    .map(|(_, values)| values),
                Some(rows),
                "{table}"
            );
        }
    }
    let b =
        crate::agent_runtime::multi_agent::budget::balance(&db, root, Some(""), "target_requests")
            .unwrap();
    assert_eq!((b.reserved, b.consumed, b.indeterminate), (0, 1, 0));
    assert!(receive_agent_http_headers(&harness.context, &claim, 200).is_err());
    assert!(super::tests::application_table_snapshot(&db) == after);
    drop(db);
    drop(runtime);
    single_target_cleanup(harness);
}

#[test]
fn single_budget_http_receipt_fault_keeps_original_unknown_debt_and_no_business_effect() {
    for effect in [
        "SELECT RAISE(IGNORE)",
        "SELECT RAISE(ABORT,'receipt-fault')",
        "INSERT INTO projects(name) VALUES('single-http-receipt-collateral')",
    ] {
        let harness = single_target_harness("single-http-receipt-fault", 30);
        let (runtime, request) = single_target_start(&harness.context);
        let claim = claim_agent_http_request(&harness.context, &runtime, &request, &request.url)
            .unwrap()
            .unwrap();
        let db = db::open(&harness.db_path).unwrap();
        db.execute_batch(&format!("CREATE TRIGGER single_http_receipt_fault BEFORE UPDATE ON agent_http_request_claims BEGIN {effect}; END;")).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        assert!(receive_agent_http_headers(&harness.context, &claim, 200).is_err());
        assert!(super::tests::application_table_snapshot(&db) == before);
        let root = &harness.context.run.as_ref().unwrap().run_id;
        let b = crate::agent_runtime::multi_agent::budget::balance(
            &db,
            root,
            Some(""),
            "target_requests",
        )
        .unwrap();
        assert_eq!((b.reserved, b.consumed, b.indeterminate), (0, 0, 1));
        drop(db);
        drop(runtime);
        single_target_cleanup(harness);
    }
}

#[test]
fn single_budget_actual_http_unknown_claim_blocks_different_url_and_local_work() {
    // Invalid status is a real request with no usable headers, on frozen origin.
    let harness = single_target_harness_with("single-http-unknown-no-new-work", 30, |_| {
        (0, "text/plain", "invalid original response".into())
    });
    let (mut runtime, request) = single_target_start(&harness.context);
    assert!(agent_http_exchange(&harness.context, &mut runtime, &request).is_err());
    assert_eq!(harness.site_seen.lock().unwrap().len(), 1);
    // A fresh runtime has no sticky in-memory stop; durable Root debt must win.
    let mut new_runtime = AgentToolRuntime {
        target_requests: 1,
        current_request_index: 1,
        ..Default::default()
    };
    new_runtime.set_invocation("independent-unknown-observer");
    harness
        .context
        .run
        .as_ref()
        .unwrap()
        .begin_tool("replay_http", &json!({}), "independent-unknown-observer")
        .unwrap();
    let mut new_request = request;
    new_request.url = format!("{}/different", harness.context.target_url);
    let db = db::open(&harness.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert!(agent_http_exchange(&harness.context, &mut new_runtime, &new_request).is_err());
    let result = agent_execute_tool_at(
        &harness.context,
        &mut new_runtime,
        "inspect_evidence",
        &json!({"kind":"api"}),
        "unknown-cost-local",
    );
    assert!(!value_first(&result.model_view, &["code"]).is_empty());
    assert_eq!(harness.site_seen.lock().unwrap().len(), 1);
    assert!(super::tests::application_table_snapshot(&db) == before);
    drop(db);
    drop(runtime);
    drop(new_runtime);
    single_target_cleanup(harness);
}

#[test]
fn single_budget_registered_browser_never_starts_unbrokered_helper_and_keeps_http_path() {
    let mut harness = single_target_harness("single-browser-network-boundary", 30);
    let marker = harness.root.join("browser-helper-started");
    let helper = harness.root.join("must-not-run.cjs");
    fs::write(&helper, format!("require('fs').writeFileSync({},'started'); process.stdout.write(JSON.stringify({{available:true,actions:[],requests:[]}}));",
        serde_json::to_string(&marker.to_string_lossy()).unwrap())).unwrap();
    harness.context.browser = Some(AgentBrowserRuntime {
        helper,
        runtime_path: std::env::var_os("PATH").unwrap_or_default(),
        no_proxy: "127.0.0.1,localhost".into(),
    });
    let mut runtime = AgentToolRuntime::default();
    let invocation = runtime.begin_invocation(1);
    let args = json!({"identity":"anonymous","actionKey":"open-orders","family":"business_flow"});
    harness
        .context
        .run
        .as_ref()
        .unwrap()
        .begin_tool("browser_action", &args, &invocation)
        .unwrap();
    let result = agent_execute_tool_at(
        &harness.context,
        &mut runtime,
        "browser_action",
        &args,
        &invocation,
    );
    assert!(!marker.exists());
    assert_eq!(result.model_view["browserDriven"], false);
    assert_eq!(
        result.model_view["browserAttempt"],
        "unsupported_sandbox:browser_network_broker_unavailable"
    );
    assert_eq!(harness.site_seen.lock().unwrap().len(), 1);
    let db = db::open(&harness.db_path).unwrap();
    let root = &harness.context.run.as_ref().unwrap().run_id;
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(&db, root, Some(""), "target_requests")
            .unwrap()
            .consumed,
        1
    );
    drop(db);
    drop(runtime);
    single_target_cleanup(harness);
}

#[test]
fn single_budget_preparation_loads_original_creation_control_and_repeat_is_readonly() {
    let fixture = web_mode_fixture("single", "https://authorized.example.test");
    let plan = web_mode_test_plan(&fixture);
    persist_frozen_web_execution_plan(&fixture.path, &fixture.scan, 1, &fixture.target, &plan)
        .unwrap();
    let db = db::open(&fixture.path).unwrap();
    let original_finance = fresh_multi_model_financial_rows_for_web_test(&db);
    let mut context = test_context(
        &fixture.path,
        &fixture.target,
        vec![AgentIdentity::anonymous()],
    );
    context.scan_id = fixture.scan.clone();
    context.execution_plan = plan;
    context.run = runtime_open_run(&fixture.path, &fixture.scan, &context.route);
    let root = &context.run.as_ref().unwrap().run_id;
    assert_eq!(
        fresh_multi_model_financial_rows_for_web_test(&db),
        original_finance
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_budget_limits WHERE root_run_id=?1",
            [root],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        10
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM tool_invocations WHERE run_id=?1",
            [root],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let before = super::tests::application_table_snapshot(&db);
    native_prepare_single_budget(&context).unwrap();
    assert!(super::tests::application_table_snapshot(&db) == before);
    native_prepare_single_budget(&context).unwrap();
    assert!(super::tests::application_table_snapshot(&db) == before);
}

#[test]
fn single_budget_observation_trigger_cannot_write_other_business_rows() {
    let harness = single_target_harness("single-observation-no-collateral", 30);
    let db = db::open(&harness.db_path).unwrap();
    db.execute_batch("CREATE TRIGGER single_observation_business AFTER INSERT ON sentinel_findings BEGIN INSERT INTO projects(name) VALUES('single-observation-collateral'); END;").unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let outcome = stage_agent_finding(
        &harness.context,
        AGENT_EVIDENCE_STAGE,
        "evidence",
        "minimal-observation",
        "original evidence",
        "info",
        &json!({"source":"original"}),
    );
    assert!(outcome.is_err());
    assert!(super::tests::application_table_snapshot(&db) == before);
    drop(db);
    single_target_cleanup(harness);
}
