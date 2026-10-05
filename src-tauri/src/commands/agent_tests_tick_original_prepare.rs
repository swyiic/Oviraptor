#[test]
fn coordinator_tick_expired_original_control_full_prepare_changes_no_application_rows() {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            proposal_model_response(&root_tick_valid_text("old original cost")),
        )
    }));
    let mut f = root_tick_fixture(
        "tick-original-control-preflight",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    native_coordinator_tick(&f.context, &f.actor).unwrap();
    let db = db::open(&f.context.db_path).unwrap();
    db.execute("UPDATE agent_coordinator_leases SET lease_expires_at=datetime('now','-1 seconds') WHERE root_run_id=?1",[&f.actor.root_run_id]).unwrap();
    drop(f.parent.take());
    let before = web_mode_test_rows(&db);
    let result = multi_agent_prepare(&mut f.context);
    assert!(result.is_err());
    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "no new model after original control lost its C"
    );
    web_mode_assert_rows(&db, &before);
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
}
