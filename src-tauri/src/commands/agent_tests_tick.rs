#[test]
fn coordinator_tick_actual_sdk_root_receipt_and_strict_semantics() {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            proposal_model_response(&root_tick_valid_text("paid semantic result")),
        )
    }));
    let f = root_tick_fixture("tick-real-sdk", &format!("http://127.0.0.1:{port}/v1"));
    let db = db::open(&f.context.db_path).unwrap();
    let raw_plan: String = db
        .query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [&f.actor.root_run_id],
            |r| r.get(0),
        )
        .unwrap();
    let result = native_coordinator_tick(&f.context, &f.actor).unwrap();
    assert!(!result.replayed);
    assert!(result.summary.as_json()["observed"]
        .to_string()
        .contains("paid semantic result"));
    let requests = seen.lock().unwrap();
    assert_eq!(requests.len(), 1, "actual Root SDK, no role double");
    let wire: JsonValue =
        serde_json::from_str(requests[0].split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(wire["tools"].as_array().unwrap().iter().map(|t|
        t["function"]["name"].as_str().unwrap()).collect::<std::collections::BTreeSet<_>>(),
        ["snapshot.read", "evidence.read", "capability_budget.read", "plan.propose"].into_iter().collect());
    assert_eq!(
        db.query_row("SELECT count(*) FROM agent_assignments", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='received'",[&f.actor.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &f.actor.root_run_id,
            None,
            "model_requests"
        )
        .unwrap()
        .consumed,
        1
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM agent_http_request_claims", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [&f.actor.root_run_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        raw_plan
    );
}

#[test]
fn coordinator_tick_paid_publication_fault_same_api_recovers_without_second_sdk() {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            proposal_model_response(&root_tick_valid_text("saved original decision")),
        )
    }));
    let f = root_tick_fixture("tick-paid-recovery", &format!("http://127.0.0.1:{port}/v1"));
    let db = db::open(&f.context.db_path).unwrap();
    db.execute_batch("CREATE TRIGGER tick_publication_fault BEFORE INSERT ON agent_events
        WHEN NEW.event_type='model_round_completed' AND json_extract(NEW.payload_json,'$.rootTickVersion')=1
        BEGIN SELECT RAISE(ABORT,'paid_before_publication'); END;").unwrap();
    assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_root_model_journal WHERE phase='received'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &f.actor.root_run_id,
            None,
            "model_requests"
        )
        .unwrap()
        .consumed,
        1
    );
    db.execute_batch("DROP TRIGGER tick_publication_fault;")
        .unwrap();
    // The red is original same-API recovery after actual SDK and paid receipt,
    // not absence of a table or a test helper manufacturing a response.
    let restored = native_coordinator_tick(&f.context, &f.actor)
        .unwrap_or_else(|e| panic!("paid semantic recovery must not invoke SDK: {e}"));
    assert!(restored.replayed);
    assert!(restored.summary.as_json()["observed"]
        .to_string()
        .contains("saved original decision"));
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "request"), 1);
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "decision"), 1);
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 1);
    let before = web_mode_test_rows(&db);
    let again = native_coordinator_tick(&f.context, &f.actor).unwrap();
    assert_eq!(again.event_sequence, restored.event_sequence);
    assert_eq!(again.summary, restored.summary);
    assert_eq!(seen.lock().unwrap().len(), 1);
    web_mode_assert_rows(&db, &before);
    let mut changed = f.context.clone();
    changed.evidence["newUnpaidInput"] = json!(true);
    assert!(native_coordinator_tick(&changed, &f.actor).is_err());
    assert_eq!(seen.lock().unwrap().len(), 1);
    web_mode_assert_rows(&db, &before);
}

#[test]
fn coordinator_tick_invalid_or_private_response_keeps_invoice_without_raw_body() {
    for text in [
        "UNIQUE_PRIVATE_DRAFT_DO_NOT_STORE",
        r#"{"schemaVersion":1,"observed":[],"missing":[],"suggestions":[],"costNotes":[],"risks":[],"reasoning":"UNIQUE_PRIVATE_DRAFT_DO_NOT_STORE"}"#,
    ] {
        let text = text.to_string();
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(move |_| {
            (200, "application/json", proposal_model_response(&text))
        }));
        let f = root_tick_fixture(
            "tick-private-invalid",
            &format!("http://127.0.0.1:{port}/v1"),
        );
        assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
        let db = db::open(&f.context.db_path).unwrap();
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM agent_root_model_journal WHERE phase='received'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "decision"), 0);
        assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 0);
        assert!(
            !format!("{:?}", web_mode_test_rows(&db)).contains("UNIQUE_PRIVATE_DRAFT_DO_NOT_STORE")
        );
        let before = web_mode_test_rows(&db);
        assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
        assert_eq!(seen.lock().unwrap().len(), 1);
        web_mode_assert_rows(&db, &before);
    }
}

#[test]
fn coordinator_tick_final_publication_ignore_or_collateral_is_atomic_after_bill() {
    for fault in [
        "ignore_event",
        "ignore_publication",
        "after_fence",
        "after_root_close",
    ] {
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
            (
                200,
                "application/json",
                proposal_model_response(&root_tick_valid_text("paid once")),
            )
        }));
        let f = root_tick_fixture("tick-final-proof", &format!("http://127.0.0.1:{port}/v1"));
        let db = db::open(&f.context.db_path).unwrap();
        let sql=match fault {
            "ignore_event"=>"CREATE TRIGGER tick_test_fault BEFORE INSERT ON agent_events WHEN NEW.event_type='model_round_completed' AND json_extract(NEW.payload_json,'$.rootTickVersion')=1 BEGIN SELECT RAISE(IGNORE); END;",
            "ignore_publication"=>"CREATE TRIGGER tick_test_fault BEFORE INSERT ON agent_root_tick_receipts WHEN NEW.phase='publication' BEGIN SELECT RAISE(IGNORE); END;",
            "after_fence"=>"CREATE TRIGGER tick_test_fault AFTER INSERT ON agent_root_tick_receipts WHEN NEW.phase='publication' BEGIN UPDATE agent_coordinator_leases SET fencing_token='replaced' WHERE root_run_id=NEW.root_run_id; END;",
            _=>"CREATE TRIGGER tick_test_fault AFTER INSERT ON agent_root_tick_receipts WHEN NEW.phase='publication' BEGIN UPDATE agent_runs SET status='terminal' WHERE id=NEW.root_run_id; END;",
        };
        // SQLite authorizers inspect every trigger program during statement
        // preparation, even a publication-only WHEN. Reach the actual paid
        // commit first, then test only the local publication with its same API.
        db.execute_batch("CREATE TRIGGER tick_capture_paid BEFORE INSERT ON agent_events WHEN NEW.event_type='model_round_completed' AND json_extract(NEW.payload_json,'$.rootTickVersion')=1 BEGIN SELECT RAISE(ABORT,'capture_actual_paid'); END;").unwrap();
        assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
        assert_eq!(
            seen.lock().unwrap().len(),
            1,
            "{fault}: actual SDK required"
        );
        assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "decision"), 1);
        assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 0);
        db.execute_batch("DROP TRIGGER tick_capture_paid;").unwrap();
        db.execute_batch(sql).unwrap();
        assert!(
            native_coordinator_tick(&f.context, &f.actor).is_err(),
            "{fault}"
        );
        assert_eq!(
            seen.lock().unwrap().len(),
            1,
            "{fault}: expected paid transport before fault"
        );
        assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "decision"), 1);
        assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 0);
        assert_eq!(db.query_row("SELECT count(*) FROM agent_events WHERE run_id=?1 AND event_type='model_round_completed'",[&f.actor.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),0);
        crate::agent_runtime::multi_agent::lease::validate_coordinator_lease(&db, &f.actor)
            .unwrap();
        db.execute_batch("DROP TRIGGER tick_test_fault;").unwrap();
        assert!(native_coordinator_tick(&f.context, &f.actor).is_ok());
        assert_eq!(seen.lock().unwrap().len(), 1);
    }
}
