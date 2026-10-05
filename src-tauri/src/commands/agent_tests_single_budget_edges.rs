// Additional boundary contracts. Existing 14 cases stay unchanged.
#[test]
fn single_budget_shared_observation_writer_keeps_multi_postwrite_active_guard() {
    let (root, context, lease) = public_surface_fixture("http://127.0.0.1:9/");
    let db = db::open(&context.db_path).unwrap();
    db.execute_batch("CREATE TRIGGER observation_pauses_original AFTER INSERT ON sentinel_findings BEGIN UPDATE sentinel_scans SET status='paused'; END;").unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let outcome = stage_agent_finding_with_active_guard(
        &context,
        AGENT_EVIDENCE_STAGE,
        "evidence",
        "observed-original",
        "observed original evidence",
        "info",
        &json!({"original":true}),
        true,
    );
    let unchanged = super::tests::application_table_snapshot(&db) == before;
    drop(db);
    drop(context);
    drop(lease);
    fs::remove_dir_all(root).unwrap();
    assert!(
        outcome.is_err(),
        "observation commit must recheck its active contract"
    );
    assert!(
        unchanged,
        "pause trigger and observation must roll back together"
    );
}

#[test]
fn single_budget_zero_write_and_upload_grants_deny_real_transport_before_io() {
    for operation in ["write", "upload"] {
        let harness = single_target_harness("single-zero-operation-grants", 30);
        let (mut runtime, mut request) = single_target_start(&harness.context);
        if operation == "write" {
            request.method = "POST".into();
            request.body = Some("original-write-body".into());
            request.content_type = Some("application/json".into());
        } else {
            // Keep a read method so this independently proves upload_bytes=0.
            request.body = Some("--boundary\r\noriginal-upload\r\n--boundary--".into());
            request.content_type = Some("multipart/form-data; boundary=boundary".into());
        }
        let scope_allowed = agent_scope_check(
            &harness.context,
            &request.url,
            &request.method,
            request.source,
        )
        .is_ok();
        let db = db::open(&harness.db_path).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        let outcome = agent_http_exchange(&harness.context, &mut runtime, &request);
        let unchanged = super::tests::application_table_snapshot(&db) == before;
        let seen = harness.site_seen.lock().unwrap().len();
        let spent = runtime.target_requests;
        drop(db);
        drop(runtime);
        single_target_cleanup(harness);
        assert!(
            scope_allowed,
            "same original target must reach Root budget gate: {operation}"
        );
        let failure = outcome.unwrap_err();
        assert_eq!(
            failure["error"], "budget_operation_not_granted",
            "{operation}"
        );
        assert_eq!((seen, spent), (0, 0), "{operation}");
        assert!(
            unchanged,
            "Root contract, journal, all ten dimensions and business tables: {operation}"
        );
    }
}

#[test]
fn single_budget_original_http_proof_rejects_every_extra_or_unreceived_target_entry() {
    use crate::agent_runtime::multi_agent::budget::root::RootOwner;
    let mut accepted_invalid = Vec::new();
    for corruption in [
        "orphan",
        "extra_source_row",
        "unreceived_receipt",
        "wrong_owner",
        "wrong_amount",
        "wrong_kind",
    ] {
        let harness = single_target_harness("single-exact-original-http-proof", 30);
        let (runtime, request) = single_target_start(&harness.context);
        let claim = claim_agent_http_request(&harness.context, &runtime, &request, &request.url)
            .unwrap()
            .unwrap();
        let db = db::open(&harness.db_path).unwrap();
        let root = &harness.context.run.as_ref().unwrap().run_id;
        let original_id: String = db
            .query_row(
                "SELECT id FROM agent_root_budget_attempts WHERE root_run_id=?1",
                [root],
                |r| r.get(0),
            )
            .unwrap();
        let source = if corruption == "orphan" {
            format!("http:{root}:missing-original-http:1:{}", "a".repeat(64))
        } else {
            agent_http_budget_source(&claim)
        };
        let owner = if corruption == "wrong_owner" {
            Uuid::new_v4().to_string()
        } else {
            original_id
        };
        let (kind, suffix) = match corruption {
            "unreceived_receipt" => ("reconcile", "receipt"),
            "wrong_kind" => ("consume", "forged-consume"),
            "orphan" => ("reserve", "reserve"),
            _ => ("reserve", "extra-reserve"),
        };
        let amount = if corruption == "wrong_amount" { 2 } else { 1 };
        db.execute(
            "INSERT INTO agent_budget_entries(entry_id,root_run_id,assignment_id,
            lease_attempt_id,dimension,kind,amount,idempotency_key,source_id)
            VALUES(?1,?2,'',?3,'target_requests',?4,?5,?6,?7)",
            params![
                Uuid::new_v4().to_string(),
                root,
                owner,
                kind,
                amount,
                format!("root:{owner}:target:{source}:{suffix}"),
                source
            ],
        )
        .unwrap();
        let before = super::tests::application_table_snapshot(&db);
        let loaded = RootOwner::load_single(&db, root);
        let accepted_live = loaded
            .as_ref()
            .is_ok_and(|owner| owner.require_live(&db).is_ok());
        let unchanged = super::tests::application_table_snapshot(&db) == before;
        let seen = harness.site_seen.lock().unwrap().len();
        drop(db);
        drop(runtime);
        single_target_cleanup(harness);
        if loaded.is_ok() {
            accepted_invalid.push(format!("{corruption} (live={accepted_live})"));
        }
        assert!(
            unchanged,
            "invalid history must not be backfilled: {corruption}"
        );
        assert_eq!(seen, 0);
    }
    assert!(
        accepted_invalid.is_empty(),
        "invalid original HTTP financial proof accepted: {}",
        accepted_invalid.join(", ")
    );
}

#[test]
fn single_budget_real_headers_reconcile_insert_fault_rolls_back_headers_and_retains_debt() {
    for effect in [
        "SELECT RAISE(IGNORE)",
        "SELECT RAISE(ABORT,'target-reconcile-fault')",
    ] {
        let harness = single_target_harness("single-real-header-reconcile-fault", 30);
        let (mut runtime, request) = single_target_start(&harness.context);
        let db = db::open(&harness.db_path).unwrap();
        db.execute_batch(&format!(
            "CREATE TRIGGER single_target_receipt_fault BEFORE INSERT
            ON agent_budget_entries WHEN NEW.dimension='target_requests' AND NEW.kind='reconcile'
            BEGIN {effect}; END;"
        ))
        .unwrap();
        let before = super::tests::application_table_snapshot(&db);
        let outcome = agent_http_exchange(&harness.context, &mut runtime, &request);
        let root = &harness.context.run.as_ref().unwrap().run_id;
        let debt = crate::agent_runtime::multi_agent::budget::balance(
            &db,
            root,
            Some(""),
            "target_requests",
        )
        .unwrap();
        let header_rows: i64 = db
            .query_row(
                "SELECT count(*) FROM agent_http_request_claims
            WHERE response_status=0 AND received_at=''",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let receipt_rows: i64 = db
            .query_row(
                "SELECT count(*) FROM agent_budget_entries
            WHERE dimension='target_requests' AND kind='reconcile'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let after = super::tests::application_table_snapshot(&db);
        let other_rows_unchanged = before.iter().all(|(table, rows)| {
            matches!(
                table.as_str(),
                "agent_http_budget_origins" | "agent_http_request_claims" | "agent_budget_entries"
            ) || after
                .iter()
                .find(|(name, _)| name == table)
                .map(|(_, values)| values)
                == Some(rows)
        });
        let seen = harness.site_seen.lock().unwrap().len();
        let output_empty = runtime.requests.is_empty() && runtime.coverage.is_empty();
        drop(db);
        drop(runtime);
        single_target_cleanup(harness);
        assert!(outcome.is_err(), "{effect}");
        assert_eq!((seen, header_rows, receipt_rows), (1, 1, 0), "{effect}");
        assert_eq!(
            (debt.reserved, debt.consumed, debt.indeterminate),
            (0, 0, 1)
        );
        assert!(output_empty && other_rows_unchanged, "{effect}");
    }
}
