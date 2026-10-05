// Original SDK and Root close before a user pause, while the actual Source
// branch owner has not delivered its result. This is a real producer ordering.
fn source_pause_after_closed_root_fixture(
    mode: &'static str,
    checkpoint: &str,
    calls: usize,
) -> (
    PathBuf,
    rusqlite::Connection,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) {
    let captured = std::rc::Rc::new(std::cell::RefCell::new(None));
    let retain = captured.clone();
    let (root, db, actor, result) = source_exhausted_pending_root_using(
        mode,
        checkpoint,
        calls,
        |root, db, record, actor, busy| {
            drop(busy.take());
            let path = root.join("oviraptor.sqlite3");
            let error = run_native_source_assessments(
                &path,
                &record.scan_id,
                1,
                &root.join("attempt-0001"),
            )
            .unwrap_err();
            assert_eq!(
                error,
                "source_tool_phase_round_budget_exhausted_without_finish"
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
            assert_eq!(
                db.query_row(
                    "SELECT status FROM native_scan_branches WHERE branch='source'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
                "pending"
            );
            let original = source_exit_paid_rows(db, &actor.root_run_id);
            *retain.borrow_mut() = Some(source_exit_snapshot(db));
            request_sentinel_pause(&path, &record.scan_id).unwrap();
            Ok(json!({"paid":original}))
        },
    );
    let payload = result.unwrap();
    assert_eq!(
        json!(source_exit_paid_rows(&db, &actor.root_run_id)),
        payload["paid"]
    );
    let before = captured.borrow_mut().take().unwrap();
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
                "already closed original {name} must remain physical byte/cell exact"
            );
        }
    }
    source_pause_branch_assert_projection_delta(&db, &actor, &before);
    (root, db, actor)
}

#[test]
fn source_pause_actual_root_closed_before_request_consumes_original_without_reclosing() {
    use crate::agent_runtime::{deleted_scan_audit, multi_agent::budget};
    for (mode, checkpoint, calls) in [
        ("tool_exhausted", "first_tool_received", 5),
        ("analyst_exhausted", "first_tool_finish_received", 7),
    ] {
        let (root, db, actor) = source_pause_after_closed_root_fixture(mode, checkpoint, calls);
        let path = root.join("oviraptor.sqlite3");
        let state:String=db.query_row("SELECT s.status||':'||b.status FROM sentinel_scans s JOIN native_scan_branches b ON b.scan_id=s.id AND b.attempt_number=s.attempt_count WHERE s.id=?1 AND b.branch='source'",[&actor.scan_id],|r|r.get(0)).unwrap();
        assert_eq!(
            state, "paused:partial",
            "original Root already closed before pause must still consume its paid failed result"
        );
        let original = source_exit_snapshot(&db);
        let files = source_failed_deletion_cas_files(&root);
        assert!(!files.is_empty());
        let tx = db.unchecked_transaction().unwrap();
        budget::clock::FinalClock::verify_original_exit(&tx, &actor.root_run_id).unwrap();
        deleted_scan_audit::prepare(&tx, &actor.scan_id)
            .unwrap()
            .verify_scope(&tx, &actor.scan_id)
            .unwrap();
        tx.rollback().unwrap();
        assert_eq!(source_exit_snapshot(&db), original);
        assert!(!finish_sentinel_pause(&path, &actor.scan_id, 1).unwrap());
        assert_eq!(source_exit_snapshot(&db), original);
        delete_sentinel_scan_inner(&path, &actor.scan_id).unwrap();
        assert!(deleted_scan_audit::verify_deleted(&db, &actor.scan_id).unwrap());
        for dimension in [
            "model_requests",
            "model_input_tokens",
            "model_output_tokens",
        ] {
            let balance = budget::balance(&db, &actor.root_run_id, None, dimension).unwrap();
            let amount = if dimension == "model_requests" {
                calls as i64
            } else {
                calls as i64 * 10
            };
            assert_eq!(
                (balance.consumed, balance.reserved, balance.indeterminate),
                (amount, 0, 0)
            );
        }
        assert_eq!(source_failed_deletion_cas_files(&root), files);
        let cold = source_exit_snapshot(&db);
        drop(db);
        let reopened = db::open(&path).unwrap();
        assert!(deleted_scan_audit::verify_deleted(&reopened, &actor.scan_id).unwrap());
        assert_eq!(source_exit_snapshot(&reopened), cold);
        drop(reopened);
        fs::remove_dir_all(root).unwrap();
    }
}
