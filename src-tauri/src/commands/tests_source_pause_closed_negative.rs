// Hold the original Source parent caller inode after the actual SDK/Root exit.
// The branch Drop cannot publish pause until this local owner leaves.
fn source_pause_closed_pending_fixture() -> (
    PathBuf,
    rusqlite::Connection,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    crate::agent_runtime::execution_owner::NativeInvocationOwner,
) {
    let saved = std::rc::Rc::new(std::cell::RefCell::new(None));
    let retain = saved.clone();
    let (root, db, actor, result) = source_exhausted_pending_root_using(
        "tool_exhausted",
        "first_tool_received",
        5,
        |root, _db, record, actor, busy| {
            drop(busy.take());
            let path = root.join("oviraptor.sqlite3");
            assert_eq!(
                run_native_source_assessments(
                    &path,
                    &record.scan_id,
                    1,
                    &root.join("attempt-0001")
                )
                .unwrap_err(),
                "source_tool_phase_round_budget_exhausted_without_finish"
            );
            request_sentinel_pause(&path, &record.scan_id).unwrap();
            *retain.borrow_mut() = crate::agent_runtime::execution_owner::probe_native_invocation(
                &path,
                &actor.scan_id,
                1,
                "source-model",
                "source",
            )
            .unwrap();
            assert!(retain.borrow().is_some());
            Ok(json!({}))
        },
    );
    result.unwrap();
    assert_eq!(
        db.query_row(
            "SELECT status FROM sentinel_scans WHERE id=?1",
            [&actor.scan_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "pausing"
    );
    assert_eq!(
        db.query_row(
            "SELECT status||':'||terminal_state FROM agent_runs WHERE id=?1",
            [&actor.root_run_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "terminal:paused"
    );
    let owner = saved.borrow_mut().take().unwrap();
    (root, db, actor, owner)
}

#[test]
fn source_pause_closed_original_damage_cannot_reopen_or_consume_the_paid_root() {
    for sql in [
        "UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00'",
        "UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1",
        "UPDATE agent_assignment_attempts SET failure_class='changed' WHERE state='failed'",
        "UPDATE agent_budget_ledger SET spent_requests=spent_requests-1",
        "UPDATE agent_runs SET terminal_reason='changed' WHERE role='coordinator'",
        "UPDATE agent_runs SET finished_at='2000-01-01 00:00:00' WHERE role='coordinator'",
        "missing_original_exit",
        r#"UPDATE native_scan_branches SET report_json='{"independentReviewCompleted":true}' WHERE branch='source'"#,
        "UPDATE sentinel_scans SET source_path=source_path||'-changed'",
        "UPDATE sentinel_targets SET last_attempt_number=99",
        "UPDATE native_branch_dispatches SET claim_id='',claimed_at='' WHERE branch='source'",
        "DROP TRIGGER source_runtime_no_update;UPDATE source_runtime_contracts SET contract_json=json_set(contract_json,'$.budget.tokenLimit',1)",
    ] {
        let (root,db,actor,busy)=source_pause_closed_pending_fixture();drop(busy);let path=root.join("oviraptor.sqlite3");
        if sql=="missing_original_exit" {
            let original:String=db.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='multi_exit_no_delete'",[],|r|r.get(0)).unwrap();
            db.execute_batch("DROP TRIGGER multi_exit_no_delete;DELETE FROM agent_multi_exit_receipts").unwrap();assert!(db.changes()>0);db.execute_batch(&original).unwrap();
        } else {db.execute_batch(sql).unwrap();assert!(db.changes()>0,"actual original must change: {sql}");}
        let damaged=source_exit_snapshot(&db);let files=source_failed_deletion_cas_files(&root);
        assert!(finish_sentinel_pause(&path,&actor.scan_id,1).is_err(),"{sql}");assert_eq!(source_exit_snapshot(&db),damaged,"refusal must preserve original closed Root/fees and pending business: {sql}");assert_eq!(source_failed_deletion_cas_files(&root),files);
        drop(db);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_pause_closed_projection_faults_preserve_original_exit_finance_and_assets() {
    let (root, db, actor, busy) = source_pause_closed_pending_fixture();
    drop(busy);
    let path = root.join("oviraptor.sqlite3");
    let files = source_failed_deletion_cas_files(&root);
    assert!(!files.is_empty());
    db.execute_batch("CREATE TABLE closed_pause_business(payload TEXT);INSERT INTO closed_pause_business VALUES('preserve');INSERT INTO assets(id,asset_key) VALUES(987654325,'closed-pause-preserve')").unwrap();
    for sql in [
        "CREATE TRIGGER closed_pause_fault BEFORE UPDATE OF status ON native_scan_branches BEGIN SELECT RAISE(IGNORE);END",
        "CREATE TRIGGER closed_pause_fault BEFORE UPDATE OF status ON sentinel_targets BEGIN SELECT RAISE(IGNORE);END",
        "CREATE TRIGGER closed_pause_fault BEFORE UPDATE OF status ON sentinel_scans BEGIN SELECT RAISE(ABORT,'refuse');END",
        "CREATE TRIGGER closed_pause_fault AFTER UPDATE OF status ON native_scan_branches BEGIN UPDATE agent_runs SET finished_at='forged' WHERE role='coordinator';END",
        "CREATE TRIGGER closed_pause_fault AFTER UPDATE OF status ON sentinel_targets BEGIN UPDATE agent_budget_ledger SET spent_requests=0;END",
        "CREATE TRIGGER closed_pause_fault AFTER UPDATE OF status ON sentinel_scan_attempts BEGIN DELETE FROM assets;END",
        "CREATE TRIGGER closed_pause_fault AFTER UPDATE OF status ON sentinel_scans BEGIN UPDATE closed_pause_business SET payload='changed';END",
        "CREATE TRIGGER closed_pause_fault AFTER UPDATE OF status ON sentinel_targets BEGIN UPDATE native_scan_branches SET checkpoint='forged';END",
    ] {
        db.execute_batch(sql).unwrap();let before=source_exit_snapshot(&db);
        assert!(finish_sentinel_pause(&path,&actor.scan_id,1).is_err(),"{sql}");assert_eq!(source_exit_snapshot(&db),before,"all original closed Root/Exit/cost and business rows must roll back: {sql}");assert_eq!(source_failed_deletion_cas_files(&root),files);db.execute_batch("DROP TRIGGER closed_pause_fault").unwrap();
    }
    let before = source_exit_snapshot(&db);
    assert!(finish_sentinel_pause(&path, &actor.scan_id, 1).unwrap());
    let after = source_exit_snapshot(&db);
    for (name, rows) in &before {
        if ![
            "sentinel_scans",
            "sentinel_scan_attempts",
            "native_scan_branches",
            "sentinel_targets",
        ]
        .contains(&name.as_str())
        {
            assert_eq!(
                &after.iter().find(|(n, _)| n == name).unwrap().1,
                rows,
                "pure terminal replay must preserve {name}"
            );
        }
    }
    source_pause_branch_assert_projection_delta(&db, &actor, &before);
    assert_eq!(source_failed_deletion_cas_files(&root), files);
    assert!(!finish_sentinel_pause(&path, &actor.scan_id, 1).unwrap());
    assert_eq!(source_exit_snapshot(&db), after);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_pause_closed_consumed_original_is_not_selected_for_another_terminal_write() {
    let (root, db, actor) =
        source_pause_after_closed_root_fixture("tool_exhausted", "first_tool_received", 5);
    let before = source_exit_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    assert!(
        known_source_pause_root_in(&tx, &actor.scan_id, 1)
            .unwrap()
            .is_none(),
        "a consumed paid Source result must not be selected again"
    );
    tx.rollback().unwrap();
    assert_eq!(source_exit_snapshot(&db), before);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
