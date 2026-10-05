#[test]
fn assignment_attempt_reassignment_supervised_bootstrap_starts_saved_replacement_without_new_grant()
{
    use crate::agent_runtime::multi_agent::{attempts, scheduler};
    let (root, context, lease, old) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    expire_worker_deadline(&db, &old.run_id);
    let ready = scheduler::reassign_undispatched_expired(&db, &lease, &old).unwrap();
    assert_eq!(
        attempts::current(&db, &lease, &ready.assignment_id)
            .unwrap()
            .state,
        "leased"
    );
    let old_audit = expired_saved_worker_row(&db, &old.run_id);
    let worker = attempts::current(&db, &lease, &ready.assignment_id).unwrap();
    let entries = worker_budget_entries(&db, &worker.id);
    let resumed=scheduler::prepare_supervised_readonly_child(&db,&lease,old.role,"journal-test",&json!({"fixture":true}),8000)
        .expect("a fully issued prepared replacement resumes its own start without another identity or reservation");
    assert_eq!(resumed, ready);
    assert_eq!(
        attempts::current(&db, &lease, &ready.assignment_id)
            .unwrap()
            .id,
        worker.id
    );
    assert_eq!(
        attempts::current(&db, &lease, &ready.assignment_id)
            .unwrap()
            .state,
        "running"
    );
    assert_eq!(worker_budget_entries(&db, &worker.id), entries);
    assert_eq!(expired_saved_worker_row(&db, &old.run_id), old_audit);
    let stable = super::tests::application_table_snapshot(&db);
    assert_eq!(
        scheduler::prepare_supervised_readonly_child(
            &db,
            &lease,
            old.role,
            "journal-test",
            &json!({"fixture":true}),
            8000
        )
        .unwrap(),
        ready
    );
    assert!(super::tests::application_table_snapshot(&db) == stable);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_reassignment_supervised_bootstrap_unknown_stays_read_only() {
    use crate::agent_runtime::multi_agent::{scheduler, specialist};
    let (root, context, lease, child) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    assert!(matches!(
        specialist::start(&db, &lease, &child, &json!({"messages":[]})).unwrap(),
        specialist::Start::Dispatch(_)
    ));
    expire_worker_deadline(&db, &child.run_id);
    stop_failed_child_preserving_usage(&db, &lease, &child, "expired").unwrap();
    let stable = super::tests::application_table_snapshot(&db);
    assert!(scheduler::prepare_supervised_readonly_child(
        &db,
        &lease,
        child.role,
        "journal-test",
        &json!({"fixture":true}),
        8000
    )
    .is_err());
    assert!(super::tests::application_table_snapshot(&db) == stable);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_reassignment_supervised_bootstrap_saved_receipt_does_not_regrant() {
    use crate::agent_runtime::multi_agent::scheduler;
    let (root, context, lease, child) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    let usage = attempt_saved_call(&db, &lease, &child, &json!({}), "saved mapper summary");
    expire_worker_deadline(&db, &child.run_id);
    stop_failed_child_preserving_usage(&db, &lease, &child, "expired").unwrap();
    let stable = super::tests::application_table_snapshot(&db);
    let recovered = scheduler::prepare_supervised_readonly_child(
        &db,
        &lease,
        child.role,
        "journal-test",
        &json!({"fixture":true}),
        8000,
    )
    .unwrap();
    assert_eq!(recovered, child);
    assert!(super::tests::application_table_snapshot(&db) == stable);
    complete_readonly_assessment(
        &db,
        &lease,
        &child,
        &usage,
        &json!({"summary":"saved mapper summary"}),
    )
    .unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_assignment_attempts WHERE assignment_id=?1",
            [&child.assignment_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
