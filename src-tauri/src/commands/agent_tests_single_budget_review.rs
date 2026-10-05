// Independent tests-only draft. Include from commands/agent_tests.rs.
// Existing production APIs only; no repository/Cargo/DB changes were made to
// prepare this file. The parent must run red before modifying those APIs.

fn single_budget_review_resume_fixture(tag: &str, mode: &str, prior: i64) -> AgentHarness {
    let mut harness = agent_harness(tag, mock_site, vec![AgentIdentity::anonymous()]);
    freeze_harness_plan(&harness);
    // A genuine current Native JSON checkpoint, not hand-built fake balances.
    accounting_native(&harness.context, prior);
    harness.context.attempt_number = 2;
    harness.context.execution_plan = harness.context.execution_plan.clone().with_attempt(2);
    let db = db::open(&harness.db_path).unwrap();
    db.execute(
        "UPDATE sentinel_scans SET attempt_count=2 WHERE id=?1",
        [&harness.context.scan_id],
    )
    .unwrap();
    drop(db);
    freeze_harness_plan(&harness);
    seed_attempt_row(&harness.db_path, 2, mode);
    harness.context.run = runtime_open_run(
        &harness.db_path,
        &harness.context.scan_id,
        &harness.context.route,
    );
    assert!(harness.context.run.is_some());
    harness
}

#[test]
fn single_budget_direct_sdk_cannot_backfill_root_from_native_resume_ancestor() {
    // Zero prior traffic is still a historical Native checkpoint requiring an
    // original owner; nonzero traffic must not disappear from Root admission.
    for prior in [0, 3] {
        let mut harness =
            single_budget_review_resume_fixture("single-review-resume-owner", "resume", prior);
        let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(|_| {
            (200, "application/json", model_round(&[], 20))
        }));
        harness.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
        let client = AgentModelClient::new(
            agent_model_profile(&harness.context.environment, None).unwrap(),
            &[],
        );
        let db = db::open(&harness.db_path).unwrap();
        let root = &harness.context.run.as_ref().unwrap().run_id;
        let usage = agent_request_accounting(
            &db,
            &harness.context.scan_id,
            2,
            &harness.context.target_url,
        )
        .unwrap();
        assert_eq!(usage.attempts, vec![2, 1]);
        assert_eq!(usage.budget_committed, prior);
        let original_checkpoint: String = db.query_row(
            "SELECT raw_json FROM sentinel_checkpoints WHERE scan_id=?1 AND url=?2 AND stage=?3",
            params![harness.context.scan_id, harness.context.target_url, NATIVE_AGENT_STATE_STAGE],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(
            serde_json::from_str::<JsonValue>(&original_checkpoint).unwrap()["attemptNumber"],
            1,
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM agent_root_budget_attempts WHERE root_run_id=?1",
                [root],
                |r| r.get::<_, i64>(0),
            )
            .unwrap(),
            0
        );
        let before = super::tests::application_table_snapshot(&db);
        // Preparation already rejects this exact original production lineage.
        assert!(native_prepare_single_budget(&harness.context).is_err());
        assert!(super::tests::application_table_snapshot(&db) == before);
        // Exercise the real SDK transport boundary independently of executor
        // preparation. It must enforce the same no-backfill contract itself.
        let model = native_model_transport(
            &harness.context,
            &client,
            vec![json!({"role":"user","content":"do not adopt ancestor Native execution history"})],
            &[],
            1,
        );
        let calls = seen.lock().unwrap().len();
        let unchanged = super::tests::application_table_snapshot(&db) == before;
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        drop(client);
        drop(seen);
        drop(stop);
        drop(db);
        single_target_cleanup(harness);
        assert!(model.is_err(), "resume ancestor prior={prior}");
        assert_eq!(
            calls, 0,
            "resume ancestor must not create new SDK I/O, prior={prior}"
        );
        assert!(
            unchanged,
            "cannot backfill Root owner, limits, clock, or claims, prior={prior}"
        );
    }
}

#[test]
fn single_budget_direct_sdk_fresh_attempt_excludes_other_native_lineage() {
    let mut harness = single_budget_review_resume_fixture("single-review-fresh-owner", "fresh", 3);
    let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (200, "application/json", model_round(&[], 20))
    }));
    harness.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let client = AgentModelClient::new(
        agent_model_profile(&harness.context.environment, None).unwrap(),
        &[],
    );
    let db = db::open(&harness.db_path).unwrap();
    // This lower-level accounting fixture has an explicitly issued original
    // owner before SDK work; a fresh lineage flag alone grants no authority.
    let tx = db.unchecked_transaction().unwrap();
    crate::agent_runtime::multi_agent::budget::root::RootOwner::initialize_financial_fixture_for_test(
        &tx,
        &harness.context.run.as_ref().unwrap().run_id,
    )
    .unwrap();
    tx.commit().unwrap();
    let before_checkpoint: String = db
        .query_row(
            "SELECT raw_json FROM sentinel_checkpoints WHERE scan_id=?1 AND url=?2 AND stage=?3",
            params![
                harness.context.scan_id,
                harness.context.target_url,
                NATIVE_AGENT_STATE_STAGE
            ],
            |r| r.get(0),
        )
        .unwrap();
    let usage = agent_request_accounting(
        &db,
        &harness.context.scan_id,
        2,
        &harness.context.target_url,
    )
    .unwrap();
    assert_eq!((usage.attempts, usage.budget_committed), (vec![2], 0));
    let model = native_model_transport(
        &harness.context,
        &client,
        vec![json!({"role":"user","content":"fresh original frozen attempt may execute"})],
        &[],
        1,
    );
    let calls = seen.lock().unwrap().len();
    let after_checkpoint: String = db
        .query_row(
            "SELECT raw_json FROM sentinel_checkpoints WHERE scan_id=?1 AND url=?2 AND stage=?3",
            params![
                harness.context.scan_id,
                harness.context.target_url,
                NATIVE_AGENT_STATE_STAGE
            ],
            |r| r.get(0),
        )
        .unwrap();
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    drop(client);
    drop(seen);
    drop(stop);
    drop(db);
    single_target_cleanup(harness);
    assert!(model.is_ok());
    assert_eq!(calls, 1);
    assert_eq!(
        before_checkpoint, after_checkpoint,
        "current Native JSON is preserved"
    );
}

#[test]
fn single_budget_generic_timeout_cannot_authorize_interrupted_original_http() {
    use crate::agent_runtime::multi_agent::budget;
    let harness = single_target_harness("single-review-send-proof", 30);
    let (runtime, request) = single_target_start(&harness.context);
    let claim = claim_agent_http_request(&harness.context, &runtime, &request, &request.url)
        .unwrap()
        .unwrap();
    let db = db::open(&harness.db_path).unwrap();
    let root = &harness.context.run.as_ref().unwrap().run_id;
    let original = claim.root_target.as_ref().unwrap();
    original.require_executable(&db).unwrap();
    let timeout = original
        .transport_timeout(&db, Duration::from_secs(15))
        .unwrap();
    assert!(!timeout.is_zero() && timeout <= Duration::from_secs(15));
    // Genuine production crash-recovery transition preserves allow and the
    // original request debt, while removing permission to send this tool.
    assert_eq!(
        crate::agent_runtime::store::mark_interrupted_tool_invocations(&db, root).unwrap(),
        1
    );
    assert!(original.require_executable(&db).is_err());
    assert!(original
        .transport_timeout(&db, Duration::from_secs(15))
        .is_err());
    let before = super::tests::application_table_snapshot(&db);
    // This is exactly the generic function currently used immediately before
    // actual call.send(). It has no original call argument and must deny Single.
    let send_admission = budget::target::transport_timeout(&db, root, Duration::from_secs(15));
    let unchanged = super::tests::application_table_snapshot(&db) == before;
    let b = budget::balance(&db, root, Some(""), "target_requests").unwrap();
    let calls = harness.site_seen.lock().unwrap().len();
    drop(db);
    drop(runtime);
    single_target_cleanup(harness);
    assert!(
        send_admission.is_err(),
        "live Root alone cannot replace original pending tool proof"
    );
    assert_eq!((b.reserved, b.consumed, b.indeterminate), (0, 0, 1));
    assert_eq!(calls, 0);
    assert!(
        unchanged,
        "denied send never refunds an original uncertain claim"
    );
}

#[test]
fn single_budget_multipart_upload_in_actual_headers_cannot_evade_zero_upload_grant() {
    for (name, value, explicit) in [
        (
            "content-type",
            "multipart/form-data; boundary=single-upload",
            None,
        ),
        (
            "CoNtEnT-TyPe",
            "multipart/form-data; boundary=single-upload",
            None,
        ),
        (
            "content-type",
            " multipart/form-data; boundary=single-upload ",
            Some("application/json"),
        ),
    ] {
        let harness = single_target_harness("single-review-header-upload", 30);
        let (mut runtime, mut request) = single_target_start(&harness.context);
        // replay_http publicly exposes headers and body. GET is in the read-method
        // list, but Content-Type on the actual wire still identifies an upload.
        request.extra_headers = vec![(name.into(), value.into())];
        request.content_type = explicit.map(str::to_string);
        request.body = Some("--single-upload\r\nContent-Disposition: form-data; name=\"file\"; filename=\"proof.txt\"\r\n\r\noriginal-upload-bytes\r\n--single-upload--\r\n".into());
        let db = db::open(&harness.db_path).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        let exchange = agent_http_exchange(&harness.context, &mut runtime, &request);
        let calls = harness.site_seen.lock().unwrap().len();
        let unchanged = super::tests::application_table_snapshot(&db) == before;
        drop(db);
        drop(runtime);
        single_target_cleanup(harness);
        assert!(
            exchange.is_err(),
            "actual multipart bytes need an original upload grant"
        );
        assert_eq!(calls, 0);
        assert!(
            unchanged,
            "reject before all HTTP/Root claims, effects, or evidence writes"
        );
    }
}
