#[test]
fn final_elapsed_actual_live_successor_cannot_adopt_original_root_financial_owner() {
    let (directory, db, _root, lease) = final_elapsed_fixture("final-elapsed-live-successor");
    let mut successor = lease.clone();
    successor.lease_epoch += 1;
    successor.fencing_token = uuid::Uuid::new_v4().to_string();
    db.execute(
        "UPDATE agent_coordinator_leases SET lease_epoch=?1,fencing_token=?2 WHERE root_run_id=?3",
        params![
            successor.lease_epoch,
            successor.fencing_token,
            successor.root_run_id
        ],
    )
    .unwrap();
    crate::agent_runtime::multi_agent::lease::validate_coordinator_lease(&db, &successor).unwrap();
    let physical = final_elapsed_physical_rows(&db);
    let before = super::tests::application_table_snapshot(&db);
    assert!(finish_coordinator_run(&db, &successor, &AgentTargetOutcome::Cancelled).is_err());
    assert!(super::tests::application_table_snapshot(&db) == before);
    assert_eq!(final_elapsed_physical_rows(&db), physical);
    drop(db);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn final_elapsed_actual_root_without_original_owner_never_backfills_control() {
    let (directory, path, root, lease) =
        multi_agent_test_root("final-elapsed-missing-control", 1000, 10);
    let db = db::open(&path).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    crate::agent_runtime::multi_agent::budget::limits::initialize(&tx, &lease).unwrap();
    tx.commit().unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_root_budget_attempts WHERE root_run_id=?1",
            [&root],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let before = super::tests::application_table_snapshot(&db);
    assert!(finish_coordinator_run(&db, &lease, &AgentTargetOutcome::Cancelled).is_err());
    assert!(super::tests::application_table_snapshot(&db) == before);
    drop(db);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn final_elapsed_actual_paused_scan_records_original_cost_without_new_execution() {
    use crate::agent_runtime::multi_agent::budget;
    let (directory, db, root, lease) = final_elapsed_fixture("final-elapsed-paused-scan");
    db.execute(
        "UPDATE sentinel_scans SET status='paused' WHERE id=?1",
        [&lease.scan_id],
    )
    .unwrap();
    finish_coordinator_run(&db, &lease, &AgentTargetOutcome::Cancelled).unwrap();
    assert!(
        budget::balance(&db, &root, None, "wall_time_ms")
            .unwrap()
            .consumed
            >= 2800
    );
    let before = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    assert!(budget::root::RootOwner::initialize_control(&tx, &root).is_err());
    tx.rollback().unwrap();
    assert!(super::tests::application_table_snapshot(&db) == before);
    // This does not prove resumable pause: the current outer production reducer
    // still treats executor pause as Cancelled. The unchanged original origin
    // and the preserved elapsed charge do not authorize a resume/new owner.
    drop(db);
    fs::remove_dir_all(directory).unwrap();
}
