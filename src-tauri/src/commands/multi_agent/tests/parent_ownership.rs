#[test]
fn assignment_attempt_parent_ownership_duplicate_service_has_no_control_authority() {
    use crate::agent_runtime::multi_agent::supervisor::WorkerSupervisor;
    let (root, context, lease, _) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    let first = WorkerSupervisor::start(&context.db_path, &lease).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let duplicate = WorkerSupervisor::start(&context.db_path, &lease);
    assert!(
        duplicate.is_err(),
        "one Root must have one living supervisor"
    );
    first.check().unwrap();
    assert!(super::tests::application_table_snapshot(&db) == before);
    drop(duplicate);
    drop(first);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn assignment_attempt_parent_ownership_database_alias_and_failed_start_preserve_owner() {
    use crate::agent_runtime::multi_agent::supervisor::WorkerSupervisor;
    let (root, context, lease, _) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    let alias = root.join("alias.sqlite3");
    std::os::unix::fs::symlink(&context.db_path, &alias).unwrap();
    let first = WorkerSupervisor::start(&context.db_path, &lease).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert!(WorkerSupervisor::start(&alias, &lease).is_err());
    let mut successor = lease.clone();
    successor.fencing_token = uuid::Uuid::new_v4().to_string();
    db.execute(
        "UPDATE agent_coordinator_leases SET fencing_token=?2 WHERE root_run_id=?1",
        params![lease.root_run_id, successor.fencing_token],
    )
    .unwrap();
    let replaced = super::tests::application_table_snapshot(&db);
    // A valid new control token does not overlap a Root whose old service
    // still owns its process lock, even after the old thread notices fencing.
    assert!(WorkerSupervisor::start(&alias, &successor).is_err());
    assert!(super::tests::application_table_snapshot(&db) == replaced);
    drop(first);
    let current = WorkerSupervisor::start(&alias, &successor).unwrap();
    current.check().unwrap();
    assert!(super::tests::application_table_snapshot(&db) == replaced);
    assert!(before != replaced);
    drop(current);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_parent_ownership_start_failure_releases_process_lock_without_writes() {
    use crate::agent_runtime::multi_agent::supervisor::WorkerSupervisor;
    let (root, context, lease, child) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    expire_worker_deadline(&db, &child.run_id);
    db.execute_batch("CREATE TRIGGER failed_parent_start BEFORE UPDATE ON agent_assignment_attempts BEGIN SELECT RAISE(ABORT,'start-fault'); END;").unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert!(WorkerSupervisor::start(&context.db_path, &lease).is_err());
    assert!(super::tests::application_table_snapshot(&db) == before);
    // The failed constructor must release its OS owner, not orphan a lock
    // until this long-running application process exits.
    db.execute_batch("DROP TRIGGER failed_parent_start;")
        .unwrap();
    let guard = WorkerSupervisor::start(&context.db_path, &lease).unwrap();
    guard.check().unwrap();
    drop(guard);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
