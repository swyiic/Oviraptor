fn bootstrap_paid_fixture(
    tag: &str,
) -> (
    RootTickFixture,
    NativeCoordinatorTickReceipt,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) {
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|request| {
        (
            200,
            "application/json",
            changed_fact_response(&request, true),
        )
    }));
    let f = root_tick_fixture(tag, &format!("http://127.0.0.1:{port}/v1"));
    let decision = native_coordinator_tick(&f.context, &f.actor).unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1);
    (f, decision, seen)
}
fn bootstrap_mapper_task(f: &RootTickFixture) -> JsonValue {
    json!({"target":f.context.target_url,"objective":"independent original frontend inventory"})
}

#[test]
fn coordinator_bootstrap_paid_mapper_grant_binds_decision_and_replay_changes_no_row() {
    let (f, decision, seen) = bootstrap_paid_fixture("bootstrap-bound-grant");
    let task = bootstrap_mapper_task(&f);
    let mapper = native_coordinator_prepare_mapper(&f.context, &f.actor, &decision, &task).unwrap();
    let db = db::open(&f.context.db_path).unwrap();
    let slice: String = db
        .query_row(
            "SELECT task_slice_json FROM agent_assignments WHERE id=?1",
            [&mapper.assignment_id],
            |r| r.get(0),
        )
        .unwrap();
    let slice: JsonValue = serde_json::from_str(&slice).unwrap();
    assert_eq!(slice["rootDecision"]["step"], "dispatch:spa_api_mapper");
    assert_eq!(
        slice["rootDecision"]["eventSequence"],
        decision.event_sequence
    );
    assert_eq!(
        slice["rootDecision"]["rustPolicy"]["targetRequestsGranted"],
        0
    );
    let caps: Vec<String> = {
        let mut q=db.prepare("SELECT capability FROM agent_capability_leases WHERE child_run_id=?1 ORDER BY capability").unwrap();
        q.query_map([&mapper.run_id], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    assert_eq!(caps, vec!["evidence.read", "mailbox.write"]);
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &f.actor.root_run_id,
            None,
            "model_requests"
        )
        .unwrap()
        .reserved,
        1
    );
    let before = web_mode_test_rows(&db);
    assert_eq!(
        native_coordinator_prepare_mapper(&f.context, &f.actor, &decision, &task).unwrap(),
        mapper
    );
    web_mode_assert_rows(&db, &before);
    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "the Mapper has a grant but no SDK execution is claimed by this admission test"
    );
}

#[test]
fn coordinator_bootstrap_paid_grant_transaction_rejects_business_and_original_scope_triggers() {
    for sql in [
        "CREATE TRIGGER bootstrap_escape AFTER INSERT ON agent_assignments WHEN NEW.role='spa_api_mapper' BEGIN UPDATE projects SET name='escaped'; END;",
        "CREATE TRIGGER bootstrap_escape AFTER UPDATE OF state ON agent_assignments WHEN NEW.role='spa_api_mapper' AND NEW.state='running' BEGIN UPDATE agent_runs SET status='paused' WHERE id=NEW.coordinator_run_id; END;",
        "CREATE TRIGGER bootstrap_escape BEFORE UPDATE OF state ON agent_assignments WHEN NEW.role='spa_api_mapper' AND NEW.state='running' BEGIN SELECT RAISE(IGNORE); END;",
    ] {
        let (f,decision,seen)=bootstrap_paid_fixture("bootstrap-atomic-proof");
        let db=db::open(&f.context.db_path).unwrap();db.execute_batch(sql).unwrap();
        let before=web_mode_test_rows(&db);
        assert!(native_coordinator_prepare_mapper(&f.context,&f.actor,&decision,&bootstrap_mapper_task(&f)).is_err());
        web_mode_assert_rows(&db,&before);
        assert_eq!(root_tick_count(&db,&f.actor.root_run_id,"publication"),1,"original paid publication remains");
        assert_eq!(seen.lock().unwrap().len(),1);
    }
}

#[test]
fn coordinator_bootstrap_paid_event_or_evidence_changed_before_grant_preserves_all_rows() {
    for kind in ["event", "evidence"] {
        let (f, decision, seen) = bootstrap_paid_fixture("bootstrap-last-boundary");
        let db = db::open(&f.context.db_path).unwrap();
        if kind == "event" {
            db.execute("UPDATE agent_events SET created_at='2000-01-01 00:00:00' WHERE run_id=?1 AND sequence=?2",params![f.actor.root_run_id,decision.event_sequence]).unwrap();
        } else {
            fs::write(
                f.context.target_dir.join("frontend-evidence.json"),
                json!({"url":f.context.target_url,"changed":"outside original frozen fact"})
                    .to_string(),
            )
            .unwrap();
        }
        let before = web_mode_test_rows(&db);
        assert!(
            native_coordinator_prepare_mapper(
                &f.context,
                &f.actor,
                &decision,
                &bootstrap_mapper_task(&f)
            )
            .is_err(),
            "{kind}"
        );
        web_mode_assert_rows(&db, &before);
        assert_eq!(seen.lock().unwrap().len(), 1);
    }
}

#[test]
fn coordinator_bootstrap_creation_contract_cannot_be_removed_or_added_to_original_finance() {
    for fresh in [false, true] {
        let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|_| {
            (
                200,
                "application/json",
                proposal_model_response(&root_tick_valid_text("must not send")),
            )
        }));
        let f = root_tick_fixture_protocol(
            "bootstrap-no-upgrade",
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
        assert_eq!(mode.get("bootstrapDispatch").is_some(), fresh);
        if fresh {
            mode.as_object_mut().unwrap().remove("bootstrapDispatch");
        } else {
            mode["bootstrapDispatch"] = json!({"schemaVersion":1,"permittedStep":"dispatch:spa_api_mapper","reservedModelTokens":8000,"reservedModelRequests":1});
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
