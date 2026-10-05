#[test]
fn source_round_unsent_cleanup_after_known_round_keeps_spend_and_remaining_resources() {
    use crate::agent_runtime::multi_agent::{budget, source_rounds as rounds};
    let (root, db, context, lease, child, request) = source_round_fixture();
    let check = |db: &rusqlite::Connection| {
        agent_native_source_tool_authority(db, &context, "assignment.finish")
            .map(|_| ())
            .map_err(str::to_string)
    };
    let rounds::Start::Dispatch(first) =
        rounds::start_authorized(&db, &lease, &child, 1, &request, 8_000, check).unwrap()
    else {
        panic!()
    };
    rounds::record_received(
        &db,
        &first,
        &source_round_result("known-local-tool", "repo.inventory", json!({})),
    )
    .unwrap();
    rounds::execute_tool(&db, &first, 0, check, |_, _, _| Ok(json!({"inventory":[]}))).unwrap();
    let next = rounds::continuation(&db, &first).unwrap();
    let rounds::Start::Dispatch(second) =
        rounds::start_authorized(&db, &lease, &child, 2, &next, 8_000, check).unwrap()
    else {
        panic!()
    };
    rounds::record_not_sent(&db, &second, "user_cancelled").unwrap();
    let cost = budget::balance(
        &db,
        &lease.root_run_id,
        Some(&child.assignment_id),
        "model_input_tokens",
    )
    .unwrap();
    stop_failed_child_preserving_usage(&db, &lease, &child, "second round not sent").unwrap();
    assert_eq!(
        budget::balance(
            &db,
            &lease.root_run_id,
            Some(&child.assignment_id),
            "model_input_tokens"
        )
        .unwrap(),
        cost
    );
    assert!(cost.consumed > 0);
    assert!(cost.reserved > 0);
    assert_eq!(
        db.query_row(
            "SELECT state FROM agent_assignments WHERE id=?1",
            [&child.assignment_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "paused"
    );
    assert_eq!(
        budget::balance(
            &db,
            &lease.root_run_id,
            Some(&child.assignment_id),
            "concurrency_batches"
        )
        .unwrap()
        .reserved,
        1
    );
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_round_unsent_cleanup_last_write_fake_evidence_rolls_back_all_refunds() {
    use crate::agent_runtime::multi_agent::source_rounds as rounds;
    let (root, db, context, lease, child, request) = source_round_fixture();
    let check = |db: &rusqlite::Connection| {
        agent_native_source_tool_authority(db, &context, "assignment.finish")
            .map(|_| ())
            .map_err(str::to_string)
    };
    let rounds::Start::Dispatch(call) =
        rounds::start_authorized(&db, &lease, &child, 1, &request, 8_000, check).unwrap()
    else {
        panic!()
    };
    rounds::record_not_sent(&db, &call, "user_cancelled").unwrap();
    db.execute_batch("CREATE TRIGGER source_unsent_refund_fault AFTER INSERT ON agent_budget_entries
        WHEN NEW.kind='release' AND NEW.dimension='model_requests'
        BEGIN INSERT INTO agent_evidence_nodes(id,root_run_id,kind,natural_key_hash,created_by_run_id)
          VALUES('unsent-refund-phantom',NEW.root_run_id,'source','refund-phantom',(SELECT child_run_id FROM agent_assignments WHERE id=NEW.assignment_id)); END").unwrap();
    let before = application_table_snapshot(&db);
    assert!(
        stop_failed_child_preserving_usage(&db, &lease, &child, "first round not sent").is_err()
    );
    assert_eq!(application_table_snapshot(&db), before);
    db.execute_batch("DROP TRIGGER source_unsent_refund_fault")
        .unwrap();
    stop_failed_child_preserving_usage(&db, &lease, &child, "first round not sent").unwrap();
    assert_eq!(
        db.query_row(
            "SELECT state FROM agent_assignments WHERE id=?1",
            [&child.assignment_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "failed"
    );
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_round_competing_observer_does_not_pause_the_original_dispatch_owner() {
    use crate::agent_runtime::multi_agent::source_rounds as rounds;
    let (root, db, context, lease, child, request) = source_round_fixture();
    let check = |db: &rusqlite::Connection| {
        agent_native_source_tool_authority(db, &context, "assignment.finish")
            .map(|_| ())
            .map_err(str::to_string)
    };
    let rounds::Start::Dispatch(call) =
        rounds::start_authorized(&db, &lease, &child, 1, &request, 8_000, check).unwrap()
    else {
        panic!()
    };
    let error =
        rounds::start_authorized(&db, &lease, &child, 1, &request, 8_000, check).unwrap_err();
    let before = application_table_snapshot(&db);
    assert_eq!(failed_specialist_error(&db, &lease, &child, &error), error);
    assert_eq!(
        application_table_snapshot(&db),
        before,
        "a caller that did not own dispatch cannot revoke the in-flight owner"
    );
    rounds::record_not_sent(&db, &call, "user_cancelled").unwrap();
    let error =
        rounds::start_authorized(&db, &lease, &child, 1, &request, 8_000, check).unwrap_err();
    let before = application_table_snapshot(&db);
    assert_eq!(failed_specialist_error(&db, &lease, &child, &error), error);
    assert_eq!(
        application_table_snapshot(&db),
        before,
        "observing a no-send proof is not ownership of cleanup"
    );
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
