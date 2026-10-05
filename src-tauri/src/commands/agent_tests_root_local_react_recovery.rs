#[test]
fn root_local_react_actual_paid_local_publication_fault_resumes_without_replaying_first_call() {
    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = count.clone();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |_| {
        let turn = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let body = if turn == 0 {
            root_local_wire(
                json!([root_local_call(
                    "call-paid-local",
                    "snapshot.read",
                    json!({})
                )]),
                "",
                true,
            )
        } else {
            proposal_model_response(&root_tick_valid_text(
                "continued original paid local result",
            ))
        };
        (200, "application/json", body)
    }));
    let f = root_tick_fixture(
        "root-local-paid-recovery",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let db = db::open(&f.context.db_path).unwrap();
    db.execute_batch("CREATE TRIGGER local_publication_fault BEFORE INSERT ON agent_events WHEN NEW.event_type='model_round_completed' BEGIN SELECT RAISE(ABORT,'paid_local_before_publication'); END;").unwrap();
    assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "decision"), 1);
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 0);
    let original:String=db.query_row("SELECT fact_json FROM agent_root_tick_receipts WHERE root_run_id=?1 AND round=1 AND phase='decision'",[&f.actor.root_run_id],|r|r.get(0)).unwrap();
    db.execute_batch("DROP TRIGGER local_publication_fault;")
        .unwrap();
    let receipt = native_coordinator_tick(&f.context, &f.actor).unwrap();
    assert_eq!(
        seen.lock().unwrap().len(),
        2,
        "only second original request may be sent"
    );
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 2);
    assert_eq!(db.query_row("SELECT fact_json FROM agent_root_tick_receipts WHERE root_run_id=?1 AND round=1 AND phase='decision'",[&f.actor.root_run_id],|r|r.get::<_,String>(0)).unwrap(),original);
    let before = web_mode_test_rows(&db);
    assert_eq!(
        native_coordinator_tick(&f.context, &f.actor)
            .unwrap()
            .event_sequence,
        receipt.event_sequence
    );
    assert_eq!(seen.lock().unwrap().len(), 2);
    web_mode_assert_rows(&db, &before);
}
#[test]
fn root_local_react_actual_prior_local_event_mutation_after_second_bill_blocks_final_publication() {
    let paths = std::sync::Arc::new(std::sync::Mutex::new(None::<PathBuf>));
    let writer = paths.clone();
    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = count.clone();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |_| {
        let turn = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let body = if turn == 0 {
            root_local_wire(
                json!([root_local_call(
                    "call-original-event",
                    "snapshot.read",
                    json!({})
                )]),
                "",
                true,
            )
        } else {
            let db = db::open(writer.lock().unwrap().as_ref().unwrap()).unwrap();
            db.execute("UPDATE agent_events SET created_at=created_at||' changed' WHERE event_type='model_round_completed'",[]).unwrap();
            proposal_model_response(&root_tick_valid_text(
                "second bill must survive stale first event",
            ))
        };
        (200, "application/json", body)
    }));
    let f = root_tick_fixture(
        "root-local-event-damage",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    *paths.lock().unwrap() = Some(f.context.db_path.clone());
    let db = db::open(&f.context.db_path).unwrap();
    let error = native_coordinator_tick(&f.context, &f.actor).err().unwrap();
    assert_eq!(error, "root_tick_publication_changed");
    assert_eq!(seen.lock().unwrap().len(), 2);
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "decision"), 2);
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 1);
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &f.actor.root_run_id,
            None,
            "model_requests"
        )
        .unwrap()
        .consumed,
        2
    );
    let before = web_mode_test_rows(&db);
    assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
    assert_eq!(seen.lock().unwrap().len(), 2);
    web_mode_assert_rows(&db, &before);
}
