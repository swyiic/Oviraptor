#[test]
fn root_local_live_budget_creation_contract_cannot_upgrade_or_strip_original_finance() {
    for fresh in [false, true] {
        let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|_| {
            (
                200,
                "application/json",
                proposal_model_response(&root_tick_valid_text("must not send")),
            )
        }));
        let f = root_tick_fixture_protocol(
            "live-budget-original-birth",
            &format!("http://127.0.0.1:{port}/v1"),
            fresh,
        );
        let db = db::open(&f.context.db_path).unwrap();
        let text: String = db
            .query_row(
                "SELECT definition_json FROM agent_root_mode_definitions WHERE root_run_id=?1",
                [&f.actor.root_run_id],
                |r| r.get(0),
            )
            .unwrap();
        let mut mode: JsonValue = serde_json::from_str(&text).unwrap();
        assert_eq!(mode.get("liveBudgetObservation").is_some(), fresh);
        if fresh {
            mode.as_object_mut()
                .unwrap()
                .remove("liveBudgetObservation");
        } else {
            mode["liveBudgetObservation"] = json!({"schemaVersion":1,"snapshotTiming":"before_each_original_model_call","executionGrant":false});
        }
        let mode: crate::agent_runtime::web_mode::root::NewRootModeDeclaration =
            serde_json::from_value(mode).unwrap();
        db.execute_batch("DROP TRIGGER root_mode_immutable_update;")
            .unwrap();
        db.execute(
            "UPDATE agent_root_mode_definitions SET definition_json=?1 WHERE root_run_id=?2",
            params![serde_json::to_string(&mode).unwrap(), f.actor.root_run_id],
        )
        .unwrap();
        assert!(
            crate::agent_runtime::web_mode::root::read(&db, &f.actor.root_run_id)
                .unwrap()
                .is_some()
        );
        let before = web_mode_test_rows(&db);
        assert_eq!(
            crate::agent_runtime::multi_agent::budget::root::RootOwner::load_original(
                &db,
                &f.actor.root_run_id
            )
            .err()
            .unwrap(),
            "budget_root_original_owner_conflict"
        );
        assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
        assert_eq!(seen.lock().unwrap().len(), 0);
        web_mode_assert_rows(&db, &before);
    }
}

#[test]
fn root_local_live_budget_mutated_paid_snapshot_cannot_be_adopted_or_resent() {
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|request| {
        (
            200,
            "application/json",
            changed_fact_response(&request, true),
        )
    }));
    let f = root_tick_fixture(
        "live-budget-paid-tamper",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    native_coordinator_tick(&f.context, &f.actor).unwrap();
    let db = db::open(&f.context.db_path).unwrap();
    let page = root_tick_sdk_page(&f.context.db_path, &f.context.scan_id);
    assert_eq!(
        root_tick_sdk_stages(&root_tick_sdk_rows(&page, 1)),
        [
            "prepared",
            "sent",
            "response_received",
            "cost_saved",
            "validated",
            "terminal"
        ]
    );
    let text:String=db.query_row("SELECT fact_json FROM agent_root_tick_receipts WHERE root_run_id=?1 AND phase='request'",[&f.actor.root_run_id],|r|r.get(0)).unwrap();
    let mut fact: JsonValue = serde_json::from_str(&text).unwrap();
    assert_eq!(
        fact["request"]["basis"]["budgetSnapshot"]["remainingModelRequests"],
        20
    );
    fact["request"]["basis"]["budgetSnapshot"]["remainingModelRequests"] = json!(999999);
    db.execute_batch("DROP TRIGGER root_tick_no_update;")
        .unwrap();
    db.execute(
        "UPDATE agent_root_tick_receipts SET fact_json=?1 WHERE root_run_id=?2 AND phase='request'",
        params![fact.to_string(), f.actor.root_run_id],
    )
    .unwrap();
    let before = web_mode_test_rows(&db);
    assert_eq!(
        crate::agent_runtime::model::diagnostics::replay::read(
            &f.context.db_path,
            &f.context.scan_id,
            1,
            None,
            None,
            0,
            300
        )
        .unwrap_err(),
        "native_sdk_log_original_binding_changed"
    );
    web_mode_assert_rows(&db, &before);
    assert_eq!(
        native_coordinator_tick(&f.context, &f.actor).err().unwrap(),
        "root_decision_lifetime_original_binding_conflict"
    );
    assert_eq!(seen.lock().unwrap().len(), 1);
    web_mode_assert_rows(&db, &before);
}

#[test]
fn root_local_live_budget_changed_mapper_frame_observes_paid_child_cost_before_new_root_call() {
    let _real = RealSpecialistTransport::enter();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|request| {
        (
            200,
            "application/json",
            changed_fact_response(&request, true),
        )
    }));
    let mut f = root_tick_fixture(
        "live-budget-paid-mapper",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    drop(f.parent.take());
    let session = multi_agent_prepare(&mut f.context).unwrap();
    let db = db::open(&f.context.db_path).unwrap();
    let requests = seen.lock().unwrap();
    assert_eq!(requests.len(), 3);
    let wire: JsonValue =
        serde_json::from_str(requests[2].split_once("\r\n\r\n").unwrap().1).unwrap();
    let input: JsonValue =
        serde_json::from_str(wire["messages"][1]["content"].as_str().unwrap()).unwrap();
    assert_eq!(input["basis"]["phase"], "mapper-output");
    assert_eq!(
        input["basis"]["budgetSnapshot"]["remainingModelRequests"],
        18
    );
    assert_eq!(
        input["basis"]["budgetSnapshot"]["remainingModelTokens"],
        59960
    );
    assert_eq!(
        input["basis"]["budgetSnapshot"]["dimensions"]["model_requests"]["consumed"],
        2
    );
    assert_eq!(input["basis"]["budgetSnapshot"]["executionGrant"], false);
    drop(requests);
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 2);
    assert_eq!(
        db.query_row("SELECT count(*) FROM agent_http_request_claims", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap(),
        0
    );
    drop(session);
}
