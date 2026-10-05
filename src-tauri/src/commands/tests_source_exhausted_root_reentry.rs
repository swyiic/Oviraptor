// A pending Root is produced by the real recovery entry and an actual original
// SDK inode lock. No failed child, usage receipt or elapsed fact is fabricated.
fn source_exhausted_pending_root_using(
    mode: &'static str,
    checkpoint: &str,
    calls: usize,
    work: impl FnOnce(
        &Path,
        &rusqlite::Connection,
        &WorkbenchStartRecord,
        &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
        &mut Option<crate::agent_runtime::execution_owner::NativeInvocationOwner>,
    ) -> Result<JsonValue, String>,
) -> (
    PathBuf,
    rusqlite::Connection,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    Result<JsonValue, String>,
) {
    source_reviewer_execution_fixture_using_calls(
        mode,
        None,
        4,
        None,
        calls,
        |root, db, record, actor| {
            let path = root.join("oviraptor.sqlite3");
            let branch =
                claim_workbench_pipeline_branch(&path, &record.scan_id, 1, "source").unwrap();
            source_reviewer_checkpoint_fixture(root, db, record, actor, checkpoint);
            let busy = std::rc::Rc::new(std::cell::RefCell::new(None));
            let retained = busy.clone();
            let hook_path = path.clone();
            let hook_actor = actor.clone();
            source_failure_cutover_once_for_test(move || {
                let db = rusqlite::Connection::open(&hook_path).unwrap();
                let child:String=db.query_row("SELECT child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND state='failed'",
                [&hook_actor.root_run_id],|r|r.get(0)).unwrap();
                *retained.borrow_mut() = Some(
                    crate::agent_runtime::execution_owner::claim_native_invocation(
                        &hook_path,
                        &hook_actor.scan_id,
                        1,
                        "source-round-sdk",
                        &child,
                    )
                    .unwrap(),
                );
            });
            let error = run_native_source_assessments(
                &path,
                &record.scan_id,
                1,
                &root.join("attempt-0001"),
            )
            .unwrap_err();
            assert!(error.contains("source_round_transport_not_idle"), "{error}");
            assert_eq!(
                db.query_row(
                    "SELECT status FROM agent_runs WHERE id=?1",
                    [&actor.root_run_id],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
                "running"
            );
            assert_eq!(db.query_row("SELECT count(*) FROM agent_assignments WHERE coordinator_run_id=?1 AND state='failed' AND budget_settled_at<>'' AND reserved_tokens=0 AND reserved_requests=0",[&actor.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),1);
            assert_eq!(
                db.query_row(
                    "SELECT spent_requests FROM agent_budget_ledger WHERE root_run_id=?1",
                    [&actor.root_run_id],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                calls as i64
            );
            let mut original_owner = busy.borrow_mut().take();
            assert!(original_owner.is_some());
            let result = work(root, db, record, actor, &mut original_owner);
            drop(original_owner);
            drop(branch);
            result
        },
    )
}

#[test]
fn source_exhausted_pending_root_reentry_closes_original_failure_after_actual_sdk_exit_without_resending(
) {
    use crate::agent_runtime::multi_agent::budget;
    for (mode, checkpoint, calls) in [
        ("tool_exhausted", "first_tool_received", 5),
        ("analyst_exhausted", "first_tool_finish_received", 7),
    ] {
        let (root, db, actor, result) = source_exhausted_pending_root_using(
            mode,
            checkpoint,
            calls,
            |root, db, record, actor, busy| {
                let path = root.join("oviraptor.sqlite3");
                let work = root.join("attempt-0001");
                let before = source_exit_snapshot(db);
                assert!(run_native_source_assessments(&path, &record.scan_id, 1, &work).is_err());
                source_exit_assert_snapshot(db, actor, &before);
                assert_eq!(
                    db.query_row(
                        "SELECT status FROM agent_runs WHERE id=?1",
                        [&actor.root_run_id],
                        |r| r.get::<_, String>(0)
                    )
                    .unwrap(),
                    "running"
                );
                drop(busy.take());
                let paid = source_exit_paid_rows(db, &actor.root_run_id);
                let original = source_exit_snapshot(db);
                let elapsed = budget::clock::elapsed_fact::read(db, &actor.root_run_id).unwrap();
                let result = run_native_source_assessments(&path, &record.scan_id, 1, &work);
                let state: (String, String) = db
                    .query_row(
                        "SELECT status,terminal_state FROM agent_runs WHERE id=?1",
                        [&actor.root_run_id],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .unwrap();
                assert_eq!(state,("terminal".into(),"paused".into()),"original known failed work must close after the actual blocking SDK exits: {result:?}");
                assert_eq!(
                    result.as_ref().unwrap_err(),
                    "source_tool_phase_round_budget_exhausted_without_finish"
                );
                assert_eq!(
                    source_exit_paid_rows(db, &actor.root_run_id),
                    paid,
                    "no tool, model, mailbox or paid receipt replay"
                );
                let after = source_exit_snapshot(db);
                for name in [
                    "agent_model_cost_facts",
                    "agent_root_budget_attempts",
                    "agent_coordinator_leases",
                    "agent_assignments",
                    "agent_assignment_attempts",
                ] {
                    assert_eq!(
                        original.iter().find(|(n, _)| n == name),
                        after.iter().find(|(n, _)| n == name),
                        "original {name}"
                    );
                }
                if let Some(elapsed) = elapsed {
                    assert_eq!(
                        db.query_row(
                            "SELECT finished_at FROM agent_runs WHERE id=?1",
                            [&actor.root_run_id],
                            |r| r.get::<_, String>(0)
                        )
                        .unwrap(),
                        elapsed.cutoff,
                        "reuse an existing original exceptional cutoff"
                    );
                }
                let tx = db.unchecked_transaction().unwrap();
                budget::clock::FinalClock::verify_original_exit(&tx, &actor.root_run_id).unwrap();
                tx.rollback().unwrap();
                for (dimension, amount) in [
                    ("model_requests", calls as i64),
                    ("model_input_tokens", calls as i64 * 10),
                    ("model_output_tokens", calls as i64 * 10),
                ] {
                    let b = budget::balance(db, &actor.root_run_id, None, dimension).unwrap();
                    assert_eq!((b.consumed, b.reserved, b.indeterminate), (amount, 0, 0));
                }
                assert!(finish_native_source_branch(
                    &path,
                    &record.scan_id,
                    1,
                    &json!({"error":result.as_ref().unwrap_err()})
                )
                .unwrap());
                assert_eq!(
                    db.query_row(
                        "SELECT status FROM sentinel_scans WHERE id=?1",
                        [&record.scan_id],
                        |r| r.get::<_, String>(0)
                    )
                    .unwrap(),
                    "partial"
                );
                let closed = source_exit_snapshot(db);
                assert!(run_native_source_assessments(&path, &record.scan_id, 1, &work).is_err());
                assert_eq!(source_exit_snapshot(db), closed);
                result
            },
        );
        assert!(result.is_err());
        drop(db);
        fs::remove_dir_all(root).unwrap();
        let _ = actor;
    }
}

#[test]
fn source_exhausted_pending_root_reentry_rechecks_original_proof_and_pause_cutover_atomically() {
    for fault in [
        "cancel",
        "prefix",
        "round",
        "ledger",
        "worker",
        "ignore_root",
    ] {
        let (root, db, actor, result) = source_exhausted_pending_root_using(
            "tool_exhausted",
            "first_tool_received",
            5,
            |root, db, record, actor, busy| {
                drop(busy.take());
                let path = root.join("oviraptor.sqlite3");
                let work = root.join("attempt-0001");
                let snapshot = std::rc::Rc::new(std::cell::RefCell::new(None));
                let captured = snapshot.clone();
                let hook_path = path.clone();
                let hook_actor = actor.clone();
                source_failure_cutover_once_for_test(move || {
                    let db = rusqlite::Connection::open(&hook_path).unwrap();
                    match fault {
                    "cancel"=> {assert_eq!(request_sentinel_pause(&hook_path,&hook_actor.scan_id).unwrap(),1);},
                    "prefix"=> {db.execute("UPDATE agent_messages SET payload_json='{}' WHERE root_run_id=?1 AND kind='evidence_summary'",[&hook_actor.root_run_id]).unwrap();},
                    "round"=> {db.execute_batch("DROP TRIGGER agent_source_round_immutable").unwrap();db.execute("UPDATE agent_source_model_rounds SET response_hash='damaged' WHERE root_run_id=?1 AND round_number=3",[&hook_actor.root_run_id]).unwrap();},
                    "ledger"=> {db.execute("UPDATE agent_budget_ledger SET spent_tokens=spent_tokens+1 WHERE root_run_id=?1",[&hook_actor.root_run_id]).unwrap();},
                    "worker"=> {db.execute("UPDATE agent_assignment_attempts SET failure_class='unconfirmed' WHERE root_run_id=?1 AND state='failed'",[&hook_actor.root_run_id]).unwrap();},
                    "ignore_root"=> db.execute_batch("CREATE TRIGGER pending_root_ignore BEFORE UPDATE OF status ON agent_runs WHEN NEW.role='coordinator' AND NEW.status='terminal' BEGIN SELECT RAISE(IGNORE); END;").unwrap(),
                    _=>unreachable!(),
                }
                    *captured.borrow_mut() = Some(source_exit_snapshot(&db));
                });
                let result = run_native_source_assessments(&path, &record.scan_id, 1, &work);
                let error = result.as_ref().unwrap_err();
                let expected = match fault {
                    "cancel" => "agent_attempt_not_active",
                    "prefix" => "source_completion_mailbox_receipt_mismatch",
                    "round" => "source_round_receipt_invalid",
                    "ledger" => "source_recovered_exhausted_accounting_invalid",
                    "worker" => "source_recovered_exhausted_original_worker_required",
                    "ignore_root" => "coordinator_terminal_fencing_or_state_conflict",
                    _ => unreachable!(),
                };
                assert_eq!(error,&format!("source_tool_phase_round_budget_exhausted_without_finish;source_coordinator_close:{expected}"),"{fault}");
                source_exit_assert_snapshot(db, actor, snapshot.borrow().as_ref().unwrap());
                assert_eq!(
                    db.query_row(
                        "SELECT status FROM agent_runs WHERE id=?1",
                        [&actor.root_run_id],
                        |r| r.get::<_, String>(0)
                    )
                    .unwrap(),
                    "running",
                    "{fault}"
                );
                assert_eq!(
                    db.query_row(
                        "SELECT count(*) FROM agent_multi_exit_receipts WHERE root_run_id=?1",
                        [&actor.root_run_id],
                        |r| r.get::<_, i64>(0)
                    )
                    .unwrap(),
                    0
                );
                let fee = crate::agent_runtime::multi_agent::budget::balance(
                    db,
                    &actor.root_run_id,
                    None,
                    "model_requests",
                )
                .unwrap();
                assert_eq!((fee.consumed, fee.reserved, fee.indeterminate), (5, 0, 0));
                result
            },
        );
        assert!(result.is_err());
        drop(db);
        fs::remove_dir_all(root).unwrap();
        let _ = actor;
    }
}
