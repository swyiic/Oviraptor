#[test]
fn source_pause_branch_original_projection_damage_is_not_silently_overwritten() {
    for sql in [
        r#"UPDATE native_scan_branches SET report_json='{"independentReviewCompleted":true}' WHERE branch='source'"#,
        "UPDATE native_scan_branches SET checkpoint='not original' WHERE branch='source'",
        "UPDATE sentinel_scans SET source_path=source_path||'-changed'",
        "UPDATE sentinel_targets SET url=url||'-changed'",
        "UPDATE sentinel_targets SET last_attempt_number=99",
        "DELETE FROM sentinel_targets",
        "UPDATE native_scan_branches SET status='completed' WHERE branch='source'",
        "UPDATE native_branch_dispatches SET claim_id='',claimed_at='' WHERE branch='source'",
        "DELETE FROM native_scan_branches WHERE branch='source'",
    ] {
        let (root, db, actor, busy) = source_known_pause_pending_fixture();
        drop(busy);
        db.execute_batch(sql).unwrap();
        assert!(
            db.changes() > 0,
            "actual original projection must change: {sql}"
        );
        let damaged = source_exit_snapshot(&db);
        assert!(
            finish_sentinel_pause(&root.join("oviraptor.sqlite3"), &actor.scan_id, 1).is_err(),
            "unknown or conflicting original projection must not be replaced: {sql}"
        );
        assert_eq!(source_exit_snapshot(&db), damaged, "{sql}");
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_pause_branch_private_projection_faults_rollback_every_original_and_keep_assets() {
    let (root, db, actor, busy) = source_known_pause_pending_fixture();
    drop(busy);
    let path = root.join("oviraptor.sqlite3");
    let files = source_failed_deletion_cas_files(&root);
    assert!(!files.is_empty());
    db.execute_batch("CREATE TABLE pause_branch_business(payload TEXT);INSERT INTO pause_branch_business VALUES('preserve');
        INSERT INTO assets(id,asset_key) VALUES(987654324,'pause-branch-preserve');").unwrap();
    for sql in [
        "CREATE TRIGGER pause_branch_fault BEFORE UPDATE OF status ON native_scan_branches BEGIN SELECT RAISE(IGNORE); END",
        "CREATE TRIGGER pause_branch_fault BEFORE UPDATE OF status ON sentinel_targets BEGIN SELECT RAISE(IGNORE); END",
        "CREATE TRIGGER pause_branch_fault BEFORE UPDATE OF status ON native_scan_branches BEGIN SELECT RAISE(ABORT,'refuse'); END",
        "CREATE TRIGGER pause_branch_fault AFTER UPDATE OF status ON native_scan_branches BEGIN UPDATE sentinel_targets SET status='completed'; END",
        "CREATE TRIGGER pause_branch_fault AFTER UPDATE OF status ON sentinel_targets BEGIN UPDATE native_scan_branches SET checkpoint='forged'; END",
        "CREATE TRIGGER pause_branch_fault AFTER UPDATE OF status ON native_scan_branches BEGIN UPDATE pause_branch_business SET payload='changed'; END",
        "CREATE TRIGGER pause_branch_fault AFTER UPDATE OF status ON native_scan_branches BEGIN DELETE FROM assets; END",
        "CREATE TRIGGER pause_branch_fault AFTER UPDATE OF status ON sentinel_targets BEGIN UPDATE agent_budget_ledger SET spent_requests=0; END",
    ] {
        db.execute_batch(sql).unwrap();let before=source_exit_snapshot(&db);
        assert!(finish_sentinel_pause(&path,&actor.scan_id,1).is_err(),"{sql}");assert_eq!(source_exit_snapshot(&db),before,"Root/wall/exit/branch/target/pause must roll back together: {sql}");
        assert_eq!(source_failed_deletion_cas_files(&root),files);db.execute_batch("DROP TRIGGER pause_branch_fault").unwrap();
    }
    let before = source_exit_snapshot(&db);
    let paid = source_exit_paid_rows(&db, &actor.root_run_id);
    assert!(finish_sentinel_pause(&path, &actor.scan_id, 1).unwrap());
    source_pause_branch_assert_projection_delta(&db, &actor, &before);
    assert_eq!(source_exit_paid_rows(&db, &actor.root_run_id), paid);
    assert_eq!(source_failed_deletion_cas_files(&root), files);
    let closed = source_exit_snapshot(&db);
    assert!(!finish_sentinel_pause(&path, &actor.scan_id, 1).unwrap());
    assert_eq!(source_exit_snapshot(&db), closed);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_pause_branch_current_target_damage_cannot_supply_original_cold_deletion() {
    use crate::agent_runtime::deleted_scan_audit;
    let (root, db, actor) =
        source_pause_branch_original_fixture("analyst_exhausted", "first_tool_finish_received", 7);
    let path = root.join("oviraptor.sqlite3");
    let original = source_exit_snapshot(&db);
    for sql in [
        "UPDATE sentinel_targets SET status='completed'",
        "UPDATE sentinel_targets SET last_attempt_number=99",
        "UPDATE sentinel_targets SET url=url||'-changed'",
        "UPDATE sentinel_scans SET source_path=source_path||'-changed'",
    ] {
        let tx = db.unchecked_transaction().unwrap();
        tx.execute_batch(sql).unwrap();
        assert!(tx.changes() > 0);
        let damaged = source_exit_snapshot(&db);
        let proof = deleted_scan_audit::prepare(&tx, &actor.scan_id);
        assert!(
            proof.is_err(),
            "conflicting current target cannot become an original failed Source archive: {sql}"
        );
        assert_eq!(source_exit_snapshot(&db), damaged);
        drop(proof);
        tx.rollback().unwrap();
        assert_eq!(source_exit_snapshot(&db), original);
    }
    let files = source_failed_deletion_cas_files(&root);
    delete_sentinel_scan_inner(&path, &actor.scan_id).unwrap();
    let cold = source_exit_snapshot(&db);
    assert!(deleted_scan_audit::verify_deleted(&db, &actor.scan_id).unwrap());
    assert_eq!(source_exit_snapshot(&db), cold);
    assert_eq!(source_failed_deletion_cas_files(&root), files);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
