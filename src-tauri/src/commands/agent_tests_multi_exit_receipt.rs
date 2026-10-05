// Original new producer and actual paid SDK, not a relabelled financial seed.
#[test]
fn multi_exit_receipt_actual_paid_replay_rejects_changed_original_reason() {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            sdk_exact_model_body(&root_tick_valid_text("original Multi exit")),
        )
    }));
    let f = root_tick_fixture(
        "multi-exit-original-reason",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    native_coordinator_tick(&f.context, &f.actor).unwrap();
    let db = db::open(&f.context.db_path).unwrap();
    finish_coordinator_run(
        &db,
        &f.actor,
        &AgentTargetOutcome::incomplete("original recorded reason"),
    )
    .unwrap();
    let altered = AgentTargetOutcome::incomplete("altered AFTER original paid exit");
    db.execute(
        "UPDATE agent_runs SET terminal_reason=?2 WHERE id=?1",
        params![f.actor.root_run_id, altered.detail()],
    )
    .unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert!(
        finish_coordinator_run(&db, &f.actor, &altered).is_err(),
        "original paid financial exit was silently reinterpreted from mutable terminal fields"
    );
    assert!(super::tests::application_table_snapshot(&db) == before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn multi_exit_receipt_actual_paid_old_open_label_cannot_regain_live_admission() {
    let (f, seen) = multi_exit_original_paid_fixture("multi-exit-no-reopen");
    let db = db::open(&f.context.db_path).unwrap();
    finish_coordinator_run(&db, &f.actor, &AgentTargetOutcome::incomplete("original closed owner")).unwrap();
    // Temporary corruption of the old mutable labels, never an original producer.
    db.execute("UPDATE agent_runs SET status='prepared',finished_at='' WHERE id=?1", [&f.actor.root_run_id]).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let owner = crate::agent_runtime::multi_agent::budget::root::RootOwner::load_original(&db, &f.actor.root_run_id).unwrap();
    assert!(owner.require_executable(&db).is_err(), "paid exit was reopened by old mutable labels");
    assert!(native_coordinator_initialize_control(&f.context, &f.actor).is_err());
    assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
    assert!(super::tests::application_table_snapshot(&db) == before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}
