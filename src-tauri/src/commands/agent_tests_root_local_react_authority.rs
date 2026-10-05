#[test]
fn root_local_react_original_finance_binds_local_contract_before_any_sdk_or_write() {
    for local in [false, true] {
        let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|_| {
            (
                200,
                "application/json",
                proposal_model_response(&root_tick_valid_text("must not send")),
            )
        }));
        let f = root_tick_fixture_protocol(
            "root-local-anti-upgrade",
            &format!("http://127.0.0.1:{port}/v1"),
            local,
        );
        let db = db::open(&f.context.db_path).unwrap();
        let root = &f.actor.root_run_id;
        let (text,original):(String,String)=db.query_row("SELECT m.definition_json,b.contract_json FROM agent_root_mode_definitions m JOIN agent_root_budget_attempts b ON b.root_run_id=m.root_run_id WHERE m.root_run_id=?1",[root],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        let mut declaration: JsonValue = serde_json::from_str(&text).unwrap();
        assert_eq!(declaration.get("localDeliberation").is_some(), local);
        if local {
            declaration
                .as_object_mut()
                .unwrap()
                .remove("localDeliberation");
        } else {
            declaration["localDeliberation"] = json!({"schemaVersion":1,"modelRoundLimit":3,"localCallLimit":4,"contextByteLimit":98304,"reviewerTokenFloor":15000,"reviewerRequestFloor":1,"tools":["snapshot.read","evidence.read","capability_budget.read","plan.propose"]});
        }
        let declaration: crate::agent_runtime::web_mode::root::NewRootModeDeclaration =
            serde_json::from_value(declaration).unwrap();
        // Simulate disk corruption in this temporary database. The canonical
        // financial Original is deliberately left intact, even with zero calls.
        db.execute_batch("DROP TRIGGER root_mode_immutable_update;")
            .unwrap();
        db.execute(
            "UPDATE agent_root_mode_definitions SET definition_json=?1 WHERE root_run_id=?2",
            params![serde_json::to_string(&declaration).unwrap(), root],
        )
        .unwrap();
        assert!(
            crate::agent_runtime::web_mode::root::read(&db, root)
                .unwrap()
                .is_some(),
            "a structurally valid mode change must reach the original financial binding"
        );
        let before = web_mode_test_rows(&db);
        assert_eq!(
            crate::agent_runtime::multi_agent::budget::root::RootOwner::load_original(&db, root)
                .err()
                .unwrap(),
            "budget_root_original_owner_conflict"
        );
        assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
        assert_eq!(seen.lock().unwrap().len(), 0);
        assert_eq!(root_tick_count(&db, root, "request"), 0);
        web_mode_assert_rows(&db, &before);
        db.execute(
            "UPDATE agent_root_mode_definitions SET definition_json=?1 WHERE root_run_id=?2",
            params![text, root],
        )
        .unwrap();
        assert_eq!(
            db.query_row(
                "SELECT contract_json FROM agent_root_budget_attempts WHERE root_run_id=?1",
                [root],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            original
        );
        crate::agent_runtime::multi_agent::budget::root::RootOwner::load_original(&db, root)
            .unwrap();
    }
}
#[test]
fn root_local_react_genuine_before_insert_v1_receipt_keeps_original_one_shot_bytes() {
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            proposal_model_response(&root_tick_valid_text("original v1 decision")),
        )
    }));
    let f = root_tick_fixture_protocol(
        "root-local-original-v1",
        &format!("http://127.0.0.1:{port}/v1"),
        false,
    );
    let db = db::open(&f.context.db_path).unwrap();
    let contract: String = db
        .query_row(
            "SELECT contract_json FROM agent_root_budget_attempts WHERE root_run_id=?1",
            [&f.actor.root_run_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!contract.contains("localDeliberation"));
    let result = native_coordinator_tick(&f.context, &f.actor).unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1);
    let request: JsonValue =
        serde_json::from_str(seen.lock().unwrap()[0].split_once("\r\n\r\n").unwrap().1).unwrap();
    assert!(request.get("tools").is_none_or(|v| v == &json!([])));
    let original:String=db.query_row("SELECT fact_json FROM agent_root_tick_receipts WHERE root_run_id=?1 AND phase='request'",[&f.actor.root_run_id],|r|r.get(0)).unwrap();
    assert!(!original.contains("localRound"));
    assert!(!original.contains("localStep"));
    assert!(!original.contains("localDeliberation"));
    let before = web_mode_test_rows(&db);
    assert_eq!(
        native_coordinator_tick(&f.context, &f.actor)
            .unwrap()
            .event_sequence,
        result.event_sequence
    );
    assert_eq!(seen.lock().unwrap().len(), 1);
    web_mode_assert_rows(&db, &before);
    assert_eq!(
        db.query_row(
            "SELECT contract_json FROM agent_root_budget_attempts WHERE root_run_id=?1",
            [&f.actor.root_run_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        contract
    );
}
