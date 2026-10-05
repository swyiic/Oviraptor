// Financial restoration fixtures do not claim production reassignment or SDK I/O.
#[test]
fn assignment_attempt_worker_budget_receipts_with_same_call_source_stay_separate() {
    use crate::agent_runtime::multi_agent::budget;
    let (root, context, lease, original) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    let usage = late_web_model_response().usage;
    let tx = db.unchecked_transaction().unwrap();
    budget::receipts::record(
        &tx,
        &lease,
        &original.assignment_id,
        "same-call",
        "same-receipt",
        &usage,
        None,
    )
    .unwrap();
    tx.commit().unwrap();
    let (child, first, second) = worker_budget_restored_second(&db, &lease, &original);
    let old = worker_budget_entries(&db, &first);
    let tx = db.unchecked_transaction().unwrap();
    budget::model::reserve(&tx, &lease, &child.assignment_id, 8000, 1).unwrap();
    budget::receipts::record(
        &tx,
        &lease,
        &child.assignment_id,
        "same-call",
        "same-receipt",
        &usage,
        None,
    )
    .unwrap();
    tx.commit().unwrap();
    assert_eq!(worker_budget_entries(&db, &first), old);
    assert!(worker_budget_entries(&db, &second).len() > 4);
    budget::receipts::verify(
        &db,
        &lease,
        &child.assignment_id,
        "same-call",
        "same-receipt",
        &usage,
    )
    .unwrap();
    let stable = application_table_snapshot_for_worker_budget(&db);
    let tx = db.unchecked_transaction().unwrap();
    budget::receipts::record(
        &tx,
        &lease,
        &child.assignment_id,
        "same-call",
        "same-receipt",
        &usage,
        None,
    )
    .unwrap();
    tx.commit().unwrap();
    assert_eq!(application_table_snapshot_for_worker_budget(&db), stable);
    let tx = db.unchecked_transaction().unwrap();
    assert_eq!(
        budget::receipts::record(
            &tx,
            &lease,
            &child.assignment_id,
            "same-call",
            "different-receipt",
            &usage,
            None
        )
        .unwrap_err(),
        "budget_entry_replay_conflict"
    );
    tx.rollback().unwrap();
    assert_eq!(application_table_snapshot_for_worker_budget(&db), stable);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_worker_budget_original_receipt_reads_second_worker_reservation() {
    use crate::agent_runtime::multi_agent::budget;
    let (root, context, lease, original) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    let (child, first, second) = worker_budget_restored_second(&db, &lease, &original);
    let tx = db.unchecked_transaction().unwrap();
    budget::model::reserve(&tx, &lease, &child.assignment_id, 8000, 1).unwrap();
    tx.commit().unwrap();
    expire_worker_deadline(&db, &child.run_id);
    let tx = db.unchecked_transaction().unwrap();
    assert!(
        crate::agent_runtime::multi_agent::attempts::try_expire_original_in_transaction(
            &tx, &lease, &child
        )
        .unwrap()
    );
    tx.commit().unwrap();
    let old = worker_budget_entries(&db, &first);
    let owner =
        budget::receipts::original_owner(&db, &child.run_id, &lease, &child.assignment_id).unwrap();
    assert_eq!(owner.attempt_id(), second);
    let tx = db.unchecked_transaction().unwrap();
    budget::receipts::record_original(
        &tx,
        &owner,
        "second-late-call",
        "second-late-receipt",
        &late_web_model_response().usage,
        true,
    )
    .unwrap();
    tx.commit().unwrap();
    assert!(!owner.is_unresolved(&db).unwrap());
    assert_eq!(worker_budget_entries(&db, &first), old);
    let stable = application_table_snapshot_for_worker_budget(&db);
    let tx = db.unchecked_transaction().unwrap();
    budget::receipts::record_original(
        &tx,
        &owner,
        "second-late-call",
        "second-late-receipt",
        &late_web_model_response().usage,
        true,
    )
    .unwrap();
    tx.commit().unwrap();
    assert_eq!(application_table_snapshot_for_worker_budget(&db), stable);
    assert!(
        crate::agent_runtime::multi_agent::attempts::require_live_for_run(&db, &child.run_id)
            .is_err()
    );
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_worker_budget_old_known_call_does_not_mark_current_unsent_as_sent() {
    use crate::agent_runtime::multi_agent::{budget, specialist};
    let (root, context, lease, original) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    let specialist::Start::Dispatch(call) =
        specialist::start(&db, &lease, &original, &json!({"messages":[]})).unwrap()
    else {
        panic!()
    };
    specialist::record_received(
        &db,
        &call,
        "old known result",
        false,
        &late_web_model_response().usage,
    )
    .unwrap();
    let (child, first, _) = worker_budget_restored_second(&db, &lease, &original);
    let old = worker_budget_entries(&db, &first);
    let tx = db.unchecked_transaction().unwrap();
    budget::model::reserve(&tx, &lease, &child.assignment_id, 8000, 1).unwrap();
    budget::model::release_unsent(&tx, &lease, &child.assignment_id).unwrap();
    tx.commit().unwrap();
    assert_eq!(worker_budget_entries(&db, &first), old);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_worker_budget_old_unknown_cost_blocks_new_worker_reservation() {
    use crate::agent_runtime::multi_agent::budget;
    let (root, context, lease, original) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    budget::model::forfeit_call(&tx, &lease, &original.assignment_id, "old-unknown", None).unwrap();
    tx.commit().unwrap();
    let (child, _, _) = worker_budget_restored_second(&db, &lease, &original);
    let stable = application_table_snapshot_for_worker_budget(&db);
    let tx = db.unchecked_transaction().unwrap();
    assert!(budget::model::reserve(&tx, &lease, &child.assignment_id, 8000, 1).is_err());
    tx.rollback().unwrap();
    assert_eq!(application_table_snapshot_for_worker_budget(&db), stable);
    assert!(budget::admission::require_determinate(&db, &lease.root_run_id).is_err());
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_worker_budget_foreign_namespace_and_last_write_pointer_change_rejected() {
    use crate::agent_runtime::multi_agent::budget;
    let (root, context, lease, original) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    let (child, first, _) = worker_budget_restored_second(&db, &lease, &original);
    let stable = application_table_snapshot_for_worker_budget(&db);
    let tx = db.unchecked_transaction().unwrap();
    assert_eq!(
        budget::append(
            &tx,
            &lease,
            &child.assignment_id,
            "model_requests",
            budget::Kind::Reserve,
            1,
            &format!("worker:{first}:foreign-key"),
            "foreign-key"
        )
        .unwrap_err(),
        "budget_attempt_key_scope_conflict"
    );
    tx.rollback().unwrap();
    assert_eq!(application_table_snapshot_for_worker_budget(&db), stable);
    db.execute_batch(&format!(
        "CREATE TRIGGER worker_budget_change_pointer AFTER INSERT ON agent_budget_entries
        WHEN NEW.lease_attempt_id<>'{first}' AND NEW.dimension='model_requests'
        BEGIN UPDATE agent_assignments SET child_run_id='{}' WHERE id=NEW.assignment_id; END",
        original.run_id
    ))
    .unwrap();
    let tx = db.unchecked_transaction().unwrap();
    assert!(budget::model::reserve(&tx, &lease, &child.assignment_id, 8000, 1).is_err());
    tx.rollback().unwrap();
    assert_eq!(application_table_snapshot_for_worker_budget(&db), stable);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_worker_budget_target_receipt_is_bound_to_second_worker_dispatch() {
    use crate::agent_runtime::multi_agent::budget;
    let (root, context, lease, original) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    let (child, first, _) = worker_budget_restored_second(&db, &lease, &original);
    let old = worker_budget_entries(&db, &first);
    let tx = db.unchecked_transaction().unwrap();
    budget::target::claim(&tx, &lease, &child.assignment_id, "second-target-dispatch").unwrap();
    tx.commit().unwrap();
    expire_worker_deadline(&db, &child.run_id);
    let tx = db.unchecked_transaction().unwrap();
    assert!(
        crate::agent_runtime::multi_agent::attempts::try_expire_original_in_transaction(
            &tx, &lease, &child
        )
        .unwrap()
    );
    tx.commit().unwrap();
    let tx = db.unchecked_transaction().unwrap();
    budget::target::receive_for_run(&tx, &child.run_id, "second-target-dispatch").unwrap();
    tx.commit().unwrap();
    assert_eq!(worker_budget_entries(&db, &first), old);
    let stable = application_table_snapshot_for_worker_budget(&db);
    let tx = db.unchecked_transaction().unwrap();
    budget::target::receive_for_run(&tx, &child.run_id, "second-target-dispatch").unwrap();
    tx.commit().unwrap();
    assert_eq!(application_table_snapshot_for_worker_budget(&db), stable);
    let tx = db.unchecked_transaction().unwrap();
    assert_eq!(
        budget::target::receive_for_run(&tx, &child.run_id, "unknown-dispatch").unwrap_err(),
        "budget_target_claim_missing"
    );
    tx.rollback().unwrap();
    assert_eq!(application_table_snapshot_for_worker_budget(&db), stable);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
