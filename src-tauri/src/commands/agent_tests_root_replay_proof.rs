// Original new Root + real provider; deleting only temporary exit proof after I/O.
fn root_replay_proof_paid_original() -> (RootTickFixture, Seen) {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            sdk_exact_model_body(&root_tick_valid_text("original paid Root decision")),
        )
    }));
    let f = root_tick_fixture(
        "root-replay-original-proof",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let paid = native_coordinator_tick(&f.context, &f.actor).unwrap();
    assert!(!paid.replayed);
    assert_eq!(seen.lock().unwrap().len(), 1);
    let db = db::open(&f.context.db_path).unwrap();
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
    assert!(root_inflight_closure_original_path(&f).is_file());
    (f, seen)
}
#[test]
fn root_replay_proof_actual_paid_decision_cannot_recreate_missing_original_sdk_inode() {
    let (f, seen) = root_replay_proof_paid_original();
    let path = root_inflight_closure_original_path(&f);
    fs::remove_file(&path).unwrap();
    let db = db::open(&f.context.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let replay = native_coordinator_tick(&f.context, &f.actor);
    assert!(
        !path.exists(),
        "paid Root replay recreated historical original SDK proof"
    );
    assert!(
        matches!(replay, Err(ref code) if code.contains("root_decision_transport_original_exit_proof_missing"))
    );
    assert!(super::tests::application_table_snapshot(&db) == before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}
#[test]
fn root_replay_proof_actual_unknown_decision_cannot_recreate_missing_original_sdk_inode() {
    let (f, seen) = root_inflight_closure_finished_original();
    let path = root_inflight_closure_original_path(&f);
    fs::remove_file(&path).unwrap();
    let db = db::open(&f.context.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let replay = native_coordinator_tick(&f.context, &f.actor);
    assert!(
        !path.exists(),
        "unknown Root replay recreated historical original SDK proof"
    );
    assert!(
        matches!(replay, Err(ref code) if code.contains("root_decision_transport_original_exit_proof_missing"))
    );
    assert!(super::tests::application_table_snapshot(&db) == before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}
#[test]
fn root_replay_proof_rejected_original_request_never_creates_missing_sdk_inode() {
    let (f, seen) = root_replay_proof_paid_original();
    let path = root_inflight_closure_original_path(&f);
    fs::remove_file(&path).unwrap();
    let db = db::open(&f.context.db_path).unwrap();
    db.execute_batch("DROP TRIGGER root_tick_no_update; UPDATE agent_root_tick_receipts SET request_hash='ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff' WHERE phase='request';").unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let replay = native_coordinator_tick(&f.context, &f.actor);
    assert!(
        !path.exists(),
        "rejected original Root request created historical SDK proof"
    );
    assert!(
        matches!(replay, Err(ref code) if code.contains("root_decision_lifetime_original_request_missing"))
    );
    assert!(super::tests::application_table_snapshot(&db) == before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn root_replay_proof_fresh_hard_budget_refusal_creates_no_sdk_inode_or_rows() {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (200, "application/json", sdk_exact_model_body(&root_tick_valid_text("must never be sent")))
    }));
    let f = root_tick_fixture_protocol_limits("root-proof-budget-refusal", &format!("http://127.0.0.1:{port}/v1"), false, (1000, 1));
    let path = root_inflight_closure_original_path(&f);
    assert!(!path.exists());
    let db = db::open(&f.context.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let error = native_coordinator_tick(&f.context, &f.actor).err().unwrap();
    assert!(!path.exists(), "fresh rejected admission created SDK proof");
    assert!(error.contains("budget_hard_limit_exceeded"), "{error}");
    assert!(super::tests::application_table_snapshot(&db) == before);
    assert_eq!(seen.lock().unwrap().len(), 0);
}
#[test]
fn root_replay_proof_actual_paid_replay_keeps_original_inode_rows_and_one_bill() {
    let (f, seen) = root_replay_proof_paid_original();
    let path = root_inflight_closure_original_path(&f);
    let original = fs::metadata(&path).unwrap().created().unwrap();
    let db = db::open(&f.context.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let replay = native_coordinator_tick(&f.context, &f.actor).unwrap();
    assert!(replay.replayed);
    assert_eq!(fs::metadata(&path).unwrap().created().unwrap(), original);
    assert!(super::tests::application_table_snapshot(&db) == before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}
