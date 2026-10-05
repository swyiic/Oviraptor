// Reproduce the old publisher only after real SDK/Root exit and actual OS
// quiescence. No failed worker, Root terminal, fee or Exit fact is inserted.
fn source_pause_recovery_fixture(
    mode: &'static str,
    checkpoint: &str,
    calls: usize,
) -> (
    PathBuf,
    rusqlite::Connection,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) {
    source_pause_recovery_fixture_with_ttl(mode, checkpoint, calls, None)
}

fn source_pause_recovery_fixture_with_ttl(
    mode: &'static str,
    checkpoint: &str,
    calls: usize,
    short_ttl: Option<i64>,
) -> (
    PathBuf,
    rusqlite::Connection,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) {
    let saved = std::rc::Rc::new(std::cell::RefCell::new(None));
    let retain = saved.clone();
    let (root, db, actor, result) = source_exhausted_pending_root_using(
        mode,
        checkpoint,
        calls,
        |root, db, record, actor, busy| {
            drop(busy.take());
            if let Some(seconds) = short_ttl {
                assert_eq!(db.execute("UPDATE agent_coordinator_leases SET lease_expires_at=datetime('now',printf('+%d seconds',?2),'localtime') WHERE root_run_id=?1",params![actor.root_run_id,seconds]).unwrap(),1);
            }
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
    drop(saved.borrow_mut().take());
    let path = root.join("oviraptor.sqlite3");
    let tx = db.unchecked_transaction().unwrap();
    let owners = claim_scan_quiescence_in(&tx, &path, &actor.scan_id).unwrap();
    crate::agent_runtime::multi_agent::budget::clock::FinalClock::verify_original_exit(
        &tx,
        &actor.root_run_id,
    )
    .unwrap();
    // This is the prior production publisher for a Root that closed first.
    publish_sentinel_pause_in(&tx, &actor.scan_id, 1).unwrap();
    tx.commit().unwrap();
    drop(owners);
    assert_eq!(db.query_row("SELECT s.status||':'||b.status FROM sentinel_scans s JOIN native_scan_branches b ON b.scan_id=s.id WHERE s.id=?1 AND b.branch='source'",[&actor.scan_id],|r|r.get::<_,String>(0)).unwrap(),"paused:pending");
    (root, db, actor)
}

#[test]
fn source_pause_recovery_original_paid_result_is_consumed_without_new_attempt_or_sdk() {
    for (mode, checkpoint, calls) in [
        ("tool_exhausted", "first_tool_received", 5),
        ("analyst_exhausted", "first_tool_finish_received", 7),
    ] {
        let (root, db, actor) = source_pause_recovery_fixture(mode, checkpoint, calls);
        let path = root.join("oviraptor.sqlite3");
        let before = source_exit_snapshot(&db);
        let tx = db.unchecked_transaction().unwrap();
        let refusal = crate::agent_runtime::deleted_scan_audit::prepare(&tx, &actor.scan_id)
            .err()
            .expect("pending original result must block deletion");
        eprintln!("original paid paused deletion refused: {refusal}");
        tx.rollback().unwrap();
        assert_eq!(source_exit_snapshot(&db), before);
        let files = source_failed_deletion_cas_files(&root);
        assert!(!files.is_empty());
        let receipt = recover_native_source_pause_result_inner(&path, &actor.scan_id, 1).unwrap();
        assert_eq!(
            receipt,
            SourcePauseResultReceipt {
                schema_version: 1,
                scan_id: actor.scan_id.clone(),
                attempt_number: 1,
                root_run_id: actor.root_run_id.clone(),
                scan_status: "paused",
                branch_status: "partial",
                changed: true,
                execution_replayed: false
            }
        );
        let after = source_exit_snapshot(&db);
        for (name, rows) in &before {
            if !["native_scan_branches", "sentinel_targets"].contains(&name.as_str()) {
                assert_eq!(
                    &after.iter().find(|(n, _)| n == name).unwrap().1,
                    rows,
                    "only original result cells may change: {name}"
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
        assert!(run_native_source_assessments(
            &path,
            &actor.scan_id,
            1,
            &root.join("attempt-0001")
        )
        .is_err());
        assert_eq!(source_exit_snapshot(&db), after);
        assert_eq!(source_failed_deletion_cas_files(&root), files);
        delete_sentinel_scan_inner(&path, &actor.scan_id).unwrap();
        let cold = source_exit_snapshot(&db);
        assert!(
            crate::agent_runtime::deleted_scan_audit::verify_deleted(&db, &actor.scan_id).unwrap()
        );
        drop(db);
        let db = db::open(&path).unwrap();
        assert!(
            crate::agent_runtime::deleted_scan_audit::verify_deleted(&db, &actor.scan_id).unwrap()
        );
        assert_eq!(source_exit_snapshot(&db), cold);
        assert_eq!(source_failed_deletion_cas_files(&root), files);
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}
