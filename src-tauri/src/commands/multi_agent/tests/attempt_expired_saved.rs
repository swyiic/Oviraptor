// Explicit restored worker fixture; production expiry/reassign is not issued here.
pub(super) fn restore_expired_saved_worker(
    db: &rusqlite::Connection,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
) -> Vec<rusqlite::types::Value> {
    db.execute("UPDATE agent_assignment_attempts SET state='expired',expires_at='2000-01-01',
        finished_at=datetime('now','localtime'),failure_class='worker_lease_expired' WHERE child_run_id=?1 AND state='paused'",
        [&child.run_id]).unwrap();
    db.execute("UPDATE agent_assignments SET failure_class='worker_lease_expired' WHERE id=?1 AND state='paused'",[&child.assignment_id]).unwrap();
    expired_saved_worker_row(db, &child.run_id)
}

pub(super) fn expired_saved_worker_row(
    db: &rusqlite::Connection,
    run: &str,
) -> Vec<rusqlite::types::Value> {
    let mut stmt = db
        .prepare("SELECT * FROM agent_assignment_attempts WHERE child_run_id=?1")
        .unwrap();
    let columns = stmt.column_count();
    stmt.query_row([run], |r| (0..columns).map(|i| r.get(i)).collect())
        .unwrap()
}

pub(super) fn forbid_expired_saved_execution(db: &rusqlite::Connection) {
    db.execute_batch("CREATE TRIGGER forbid_saved_worker_change BEFORE UPDATE ON agent_assignment_attempts
        BEGIN SELECT RAISE(ABORT,'original expired worker must be immutable'); END;
        CREATE TRIGGER forbid_saved_reactivation BEFORE UPDATE ON agent_runs WHEN NEW.status='running'
        BEGIN SELECT RAISE(ABORT,'no reactivation'); END;
        CREATE TRIGGER forbid_saved_grant BEFORE INSERT ON agent_capability_leases
        BEGIN SELECT RAISE(ABORT,'no grant'); END;
        CREATE TRIGGER forbid_saved_renewal BEFORE UPDATE ON agent_capability_leases WHEN NEW.revoked_at=''
        BEGIN SELECT RAISE(ABORT,'no renewal'); END;
        CREATE TRIGGER forbid_saved_coordinator_change BEFORE UPDATE ON agent_coordinator_leases
        BEGIN SELECT RAISE(ABORT,'no coordinator restoration'); END;").unwrap();
}

#[test]
fn assignment_attempt_expired_saved_reviewer_can_close_logical_work_without_changing_worker() {
    use crate::agent_runtime::multi_agent::{attempts, budget};
    let f = review_receipt_fixture();
    let db = db::open(&f.context.db_path).unwrap();
    attempt_saved_call(
        &db,
        &f.lease,
        &f.child,
        &f.candidate,
        &review_receipt_response(),
    );
    finish_failed_review(
        &db,
        &f.lease,
        &f.child,
        "review-receipt",
        "delivery interrupted",
    )
    .unwrap();
    let original = restore_expired_saved_worker(&db, &f.child);
    forbid_expired_saved_execution(&db);
    recover_received_review(
        &db,
        &f.lease,
        "receipt",
        1,
        &f.candidate.to_string(),
        &f.context.target_dir,
    )
    .unwrap();
    assert_review_receipt_completed(&db);
    assert_eq!(expired_saved_worker_row(&db, &f.child.run_id), original);
    assert!(attempts::require_live_for_run(&db, &f.child.run_id).is_err());
    assert_eq!(
        budget::balance(
            &db,
            &f.lease.root_run_id,
            Some(&f.child.assignment_id),
            "model_requests"
        )
        .unwrap()
        .consumed,
        1
    );
    let saved = super::tests::application_table_snapshot(&db);
    assert!(recover_received_review(
        &db,
        &f.lease,
        "receipt",
        1,
        &f.candidate.to_string(),
        &f.context.target_dir
    )
    .is_err());
    assert_eq!(super::tests::application_table_snapshot(&db), saved);
    drop(db);
    fs::remove_dir_all(f.root).unwrap();
}

#[test]
fn assignment_attempt_expired_saved_investigator_releases_only_its_own_slot() {
    use crate::agent_runtime::multi_agent::{attempts, budget};
    let f = gap_receipt_fixture();
    let db = db::open(&f.context.db_path).unwrap();
    attempt_saved_call(
        &db,
        &f.session.lease,
        &f.child,
        &f.input,
        &gap_receipt_response(),
    );
    stop_failed_child_preserving_usage(&db, &f.session.lease, &f.child, "delivery interrupted")
        .unwrap();
    let original = restore_expired_saved_worker(&db, &f.child);
    let sibling = budget::balance(
        &db,
        &f.session.lease.root_run_id,
        Some(&f.session.executor.assignment_id),
        "concurrency_batches",
    )
    .unwrap();
    forbid_expired_saved_execution(&db);
    recover_gap_fixture(&f, &db).unwrap();
    assert_gap_receipt_completed(&db);
    assert_eq!(expired_saved_worker_row(&db, &f.child.run_id), original);
    assert_eq!(
        budget::balance(
            &db,
            &f.session.lease.root_run_id,
            Some(&f.session.executor.assignment_id),
            "concurrency_batches"
        )
        .unwrap(),
        sibling
    );
    assert_eq!(
        budget::balance(
            &db,
            &f.session.lease.root_run_id,
            Some(&f.child.assignment_id),
            "concurrency_batches"
        )
        .unwrap()
        .reserved,
        0
    );
    assert!(attempts::require_live_for_run(&db, &f.child.run_id).is_err());
    drop(db);
    fs::remove_dir_all(f.root).unwrap();
}

#[test]
fn assignment_attempt_expired_saved_mapper_retains_worker_and_publishes_only_once() {
    use crate::agent_runtime::multi_agent::attempts;
    let (root, context, lease, child) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    let usage = attempt_saved_call(
        &db,
        &lease,
        &child,
        &serde_json::json!({}),
        "saved mapper summary",
    );
    stop_failed_child_preserving_usage(&db, &lease, &child, "delivery interrupted").unwrap();
    let original = restore_expired_saved_worker(&db, &child);
    forbid_expired_saved_execution(&db);
    let payload = serde_json::json!({"summary":"saved mapper summary"});
    complete_readonly_assessment(&db, &lease, &child, &usage, &payload).unwrap();
    assert_eq!(expired_saved_worker_row(&db, &child.run_id), original);
    assert!(attempts::require_live_for_run(&db, &child.run_id).is_err());
    let saved = super::tests::application_table_snapshot(&db);
    complete_readonly_assessment(&db, &lease, &child, &usage, &payload).unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), saved);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
