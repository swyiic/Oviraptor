#[test]
fn source_pause_recovery_expired_ttl_consumes_only_original_result_without_renewal() {
    let (root, db, actor) =
        source_pause_recovery_fixture_with_ttl("tool_exhausted", "first_tool_received", 5, Some(6));
    let path = root.join("oviraptor.sqlite3");
    let expiry: String = db
        .query_row(
            "SELECT lease_expires_at FROM agent_coordinator_leases WHERE root_run_id=?1",
            [&actor.root_run_id],
            |r| r.get(0),
        )
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while crate::agent_runtime::multi_agent::lease::validate_coordinator_lease(&db, &actor).is_ok()
    {
        assert!(
            std::time::Instant::now() < deadline,
            "actual short original TTL must expire"
        );
        thread::sleep(Duration::from_millis(100));
    }
    assert!(
        crate::agent_runtime::multi_agent::lease::validate_coordinator_lease(&db, &actor).is_err()
    );
    let tx = db.unchecked_transaction().unwrap();
    crate::agent_runtime::multi_agent::budget::clock::FinalClock::verify_original_exit(
        &tx,
        &actor.root_run_id,
    )
    .unwrap();
    tx.rollback().unwrap();
    let before = source_exit_snapshot(&db);
    let files = source_failed_deletion_cas_files(&root);
    assert!(
        recover_native_source_pause_result_inner(&path, &actor.scan_id, 1)
            .unwrap()
            .changed
    );
    let after = source_exit_snapshot(&db);
    for (name, rows) in &before {
        if !["native_scan_branches", "sentinel_targets"].contains(&name.as_str()) {
            assert_eq!(
                &after.iter().find(|(n, _)| n == name).unwrap().1,
                rows,
                "{name}"
            );
        }
    }
    source_pause_branch_assert_projection_delta(&db, &actor, &before);
    assert!(
        !recover_native_source_pause_result_inner(&path, &actor.scan_id, 1)
            .unwrap()
            .changed
    );
    assert_eq!(source_exit_snapshot(&db), after);
    assert!(
        run_native_source_assessments(&path, &actor.scan_id, 1, &root.join("attempt-0001"))
            .is_err()
    );
    assert_eq!(source_exit_snapshot(&db), after);
    assert_eq!(source_failed_deletion_cas_files(&root), files);
    assert_eq!(
        db.query_row(
            "SELECT lease_expires_at FROM agent_coordinator_leases WHERE root_run_id=?1",
            [&actor.root_run_id],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        expiry
    );
    delete_sentinel_scan_inner(&path, &actor.scan_id).unwrap();
    let cold = source_exit_snapshot(&db);
    drop(db);
    let db = db::open(&path).unwrap();
    assert!(crate::agent_runtime::deleted_scan_audit::verify_deleted(&db, &actor.scan_id).unwrap());
    assert_eq!(source_exit_snapshot(&db), cold);
    assert_eq!(source_failed_deletion_cas_files(&root), files);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_pause_recovery_original_damage_or_replacement_preserves_every_row() {
    for sql in [
        "UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00'",
        "UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1",
        "UPDATE agent_coordinator_leases SET fencing_token='replacement'",
        "UPDATE agent_runs SET cancel_requested_at='revoked' WHERE role='coordinator'",
        "UPDATE agent_runs SET terminal_reason='changed' WHERE role='coordinator'",
        "UPDATE agent_runs SET finished_at='2000-01-01 00:00:00' WHERE role='coordinator'",
        "missing_original_exit",
        "UPDATE agent_assignment_attempts SET failure_class='changed' WHERE state='failed'",
        "UPDATE agent_budget_ledger SET spent_requests=spent_requests-1",
        "DROP TRIGGER agent_source_round_immutable; UPDATE agent_source_model_rounds SET state='uncertain' WHERE round_number=3",
        "DROP TRIGGER agent_source_round_immutable; UPDATE agent_source_model_rounds SET response_hash='changed' WHERE round_number=3",
        "UPDATE sentinel_scans SET attempt_count=2",
        "UPDATE sentinel_scans SET source_path=source_path||'-changed'",
        "UPDATE sentinel_targets SET last_attempt_number=99",
        r#"UPDATE native_scan_branches SET report_json='{"independentReviewCompleted":true}' WHERE branch='source'"#,
        "UPDATE native_scan_branches SET checkpoint='foreign' WHERE branch='source'",
        "UPDATE native_branch_dispatches SET claim_id='',claimed_at='' WHERE branch='source'",
        "DROP TRIGGER source_runtime_no_update; UPDATE source_runtime_contracts SET contract_json=json_set(contract_json,'$.budget.tokenLimit',1)",
        "DROP TRIGGER source_ci_policy_no_update; UPDATE source_ci_policies SET max_high=max_high+1",
    ] {
        let (root,db,actor)=source_pause_recovery_fixture("tool_exhausted","first_tool_received",5);
        if sql=="missing_original_exit" {
            let original:String=db.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='multi_exit_no_delete'",[],|r|r.get(0)).unwrap();
            db.execute_batch("DROP TRIGGER multi_exit_no_delete; DELETE FROM agent_multi_exit_receipts").unwrap();
            assert!(db.changes()>0);db.execute_batch(&original).unwrap();
        } else {db.execute_batch(sql).unwrap();assert!(db.changes()>0,"actual original must change: {sql}");}
        let before=source_exit_snapshot(&db);let files=source_failed_deletion_cas_files(&root);
        let refusal=recover_native_source_pause_result_inner(&root.join("oviraptor.sqlite3"),&actor.scan_id,1).unwrap_err();
        eprintln!("original recovery refused {sql}: {refusal}");
        assert_eq!(source_exit_snapshot(&db),before,"{sql}");
        assert_eq!(source_failed_deletion_cas_files(&root),files,"{sql}");
        drop(db);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_pause_recovery_private_projection_faults_rollback_all_finance_and_assets() {
    let (root, db, actor) =
        source_pause_recovery_fixture("tool_exhausted", "first_tool_received", 5);
    let path = root.join("oviraptor.sqlite3");
    db.execute_batch("CREATE TABLE recovery_business(payload TEXT); INSERT INTO recovery_business VALUES('preserve'); INSERT INTO assets(id,asset_key) VALUES(987654326,'recovery-preserve')").unwrap();
    let files = source_failed_deletion_cas_files(&root);
    assert!(!files.is_empty());
    for sql in [
        "CREATE TRIGGER recovery_fault BEFORE UPDATE OF status ON native_scan_branches BEGIN SELECT RAISE(IGNORE);END",
        "CREATE TRIGGER recovery_fault BEFORE UPDATE OF status ON sentinel_targets BEGIN SELECT RAISE(IGNORE);END",
        "CREATE TRIGGER recovery_fault BEFORE UPDATE OF checkpoint ON native_scan_branches BEGIN SELECT RAISE(ABORT,'refuse');END",
        "CREATE TRIGGER recovery_fault AFTER UPDATE OF status ON native_scan_branches BEGIN UPDATE agent_runs SET finished_at='forged' WHERE role='coordinator';END",
        "CREATE TRIGGER recovery_fault AFTER UPDATE OF status ON sentinel_targets BEGIN UPDATE agent_budget_ledger SET spent_requests=0;END",
        "CREATE TRIGGER recovery_fault AFTER UPDATE OF status ON native_scan_branches BEGIN DELETE FROM assets;END",
        "CREATE TRIGGER recovery_fault AFTER UPDATE OF status ON sentinel_targets BEGIN UPDATE recovery_business SET payload='changed';END",
        "CREATE TRIGGER recovery_fault AFTER UPDATE OF status ON native_scan_branches BEGIN UPDATE sentinel_scans SET status='scanning';END",
        "CREATE TRIGGER recovery_fault AFTER UPDATE OF status ON sentinel_targets BEGIN UPDATE native_scan_branches SET checkpoint='forged';END",
    ] {
        db.execute_batch(sql).unwrap();let before=source_exit_snapshot(&db);
        assert!(recover_native_source_pause_result_inner(&path,&actor.scan_id,1).is_err(),"{sql}");
        assert_eq!(source_exit_snapshot(&db),before,"{sql}");
        assert_eq!(source_failed_deletion_cas_files(&root),files);
        db.execute_batch("DROP TRIGGER recovery_fault").unwrap();
    }
    assert!(
        recover_native_source_pause_result_inner(&path, &actor.scan_id, 1)
            .unwrap()
            .changed
    );
    assert_eq!(
        db.query_row("SELECT payload FROM recovery_business", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "preserve"
    );
    assert_eq!(
        db.query_row("SELECT asset_key FROM assets WHERE id=987654326", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "recovery-preserve"
    );
    assert_eq!(source_failed_deletion_cas_files(&root), files);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_pause_recovery_busy_or_missing_original_exit_never_recreates_authority() {
    let (root, db, actor) =
        source_pause_recovery_fixture("tool_exhausted", "first_tool_received", 5);
    let path = root.join("oviraptor.sqlite3");
    let child: String = db
        .query_row(
            "SELECT child_run_id FROM agent_assignments WHERE state='failed'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let initial: String = db
        .query_row(
            "SELECT child_run_id FROM agent_specialist_calls ORDER BY rowid LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    for (kind, key) in [
        ("source-model", "source".to_string()),
        ("source-round-sdk", child),
        ("specialist-sdk", initial),
    ] {
        let owner = crate::agent_runtime::execution_owner::probe_native_invocation(
            &path,
            &actor.scan_id,
            1,
            kind,
            &key,
        )
        .unwrap()
        .unwrap();
        let before = source_exit_snapshot(&db);
        let files = source_failed_deletion_cas_files(&root);
        assert!(
            recover_native_source_pause_result_inner(&path, &actor.scan_id, 1).is_err(),
            "{kind}"
        );
        assert_eq!(source_exit_snapshot(&db), before, "{kind}");
        assert_eq!(source_failed_deletion_cas_files(&root), files);
        drop(owner);
    }
    use sha2::Digest;
    let canonical = fs::canonicalize(&path).unwrap();
    let mut directory = canonical.file_name().unwrap().to_os_string();
    directory.push(".invocations");
    let key = serde_json::to_vec(&(&actor.scan_id, 1, "source-model", "source")).unwrap();
    let original = canonical
        .with_file_name(directory)
        .join(format!("{:x}.lock", sha2::Sha256::digest(key)));
    assert!(original.is_file());
    fs::remove_file(&original).unwrap();
    let before = source_exit_snapshot(&db);
    assert_eq!(
        recover_native_source_pause_result_inner(&path, &actor.scan_id, 1).unwrap_err(),
        "scan_quiescence_original_source_exit_missing"
    );
    assert!(!original.exists());
    assert_eq!(source_exit_snapshot(&db), before);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_pause_recovery_unclosed_original_root_keeps_paid_pending_obligations() {
    let (root, db, actor, busy) = source_known_pause_pending_fixture();
    drop(busy);
    let path = root.join("oviraptor.sqlite3");
    let tx = db.unchecked_transaction().unwrap();
    let owners = claim_scan_quiescence_in(&tx, &path, &actor.scan_id).unwrap();
    publish_sentinel_pause_in(&tx, &actor.scan_id, 1).unwrap();
    tx.commit().unwrap();
    drop(owners);
    let before = source_exit_snapshot(&db);
    let files = source_failed_deletion_cas_files(&root);
    assert_eq!(
        recover_native_source_pause_result_inner(&path, &actor.scan_id, 1).unwrap_err(),
        "source_pause_recovery_original_closed_failure_required"
    );
    assert_eq!(source_exit_snapshot(&db), before);
    assert_eq!(source_failed_deletion_cas_files(&root), files);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
