// All failures happen after actual known SDK usage and original child closure.
#[test]
fn source_recovered_exhausted_root_rejects_cutover_corruption_busy_and_silent_writes() {
    for fault in [
        "cancel",
        "prefix",
        "round",
        "ledger",
        "worker",
        "ignore_root",
        "busy",
    ] {
        let (root, db, actor, result) = source_reviewer_execution_fixture_using_calls(
            "tool_exhausted",
            None,
            4,
            None,
            5,
            |root, db, record, actor| {
                let path = root.join("oviraptor.sqlite3");
                source_reviewer_checkpoint_fixture(root, db, record, actor, "first_tool_received");
                let snapshot = std::rc::Rc::new(std::cell::RefCell::new(None));
                let captured = snapshot.clone();
                let busy = std::rc::Rc::new(std::cell::RefCell::new(None));
                let retained = busy.clone();
                let (hook_path, hook_actor) = (path.clone(), actor.clone());
                source_failure_cutover_once_for_test(move || {
                    let db = rusqlite::Connection::open(&hook_path).unwrap();
                    match fault {
                        "cancel"=> {db.execute("UPDATE agent_runs SET cancel_requested_at='cutover-cancel' WHERE id=?1",[&hook_actor.root_run_id]).unwrap();},
                        "prefix"=> {db.execute("UPDATE agent_messages SET payload_json='{}' WHERE root_run_id=?1 AND kind='evidence_summary'",[&hook_actor.root_run_id]).unwrap();},
                        "round"=> {db.execute_batch("DROP TRIGGER agent_source_round_immutable").unwrap(); db.execute("UPDATE agent_source_model_rounds SET state='uncertain' WHERE root_run_id=?1 AND round_number=3",[&hook_actor.root_run_id]).unwrap();},
                        "ledger"=> {db.execute("UPDATE agent_budget_ledger SET spent_tokens=spent_tokens+1 WHERE root_run_id=?1",[&hook_actor.root_run_id]).unwrap();},
                        "worker"=> {db.execute("UPDATE agent_assignment_attempts SET failure_class='unconfirmed' WHERE root_run_id=?1 AND state='failed'",[&hook_actor.root_run_id]).unwrap();},
                        "ignore_root"=> db.execute_batch("CREATE TRIGGER recovered_root_ignore BEFORE UPDATE OF status ON agent_runs WHEN NEW.role='coordinator' AND NEW.status='terminal' BEGIN SELECT RAISE(IGNORE); END;").unwrap(),
                        "busy"=> {
                            let child:String=db.query_row("SELECT child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND state='failed'",[&hook_actor.root_run_id],|r|r.get(0)).unwrap();
                            *retained.borrow_mut()=Some(crate::agent_runtime::execution_owner::claim_native_invocation(&hook_path,&hook_actor.scan_id,1,"source-round-sdk",&child).unwrap());
                        },
                        _=>unreachable!(),
                    }
                    *captured.borrow_mut() = Some(source_exit_snapshot(&db));
                });
                let result = run_native_source_assessments(
                    &path,
                    &record.scan_id,
                    1,
                    &root.join("attempt-0001"),
                );
                let error = result.as_ref().unwrap_err();
                assert!(error.starts_with("source_tool_phase_round_budget_exhausted_without_finish;source_coordinator_close:"),"{fault}: {error}");
                let expected = match fault {
                    "cancel" => "coordinator_not_executable",
                    "prefix" => "source_completion_mailbox_receipt_mismatch",
                    "round" => "source_round_receipt_invalid",
                    "ledger" => "source_recovered_exhausted_accounting_invalid",
                    "worker" => "source_recovered_exhausted_original_worker_required",
                    "ignore_root" => "coordinator_terminal_fencing_or_state_conflict",
                    "busy" => "source_round_transport_not_idle",
                    _ => unreachable!(),
                };
                assert!(error.contains(expected), "{fault}: {error}");
                source_exit_assert_snapshot(db, actor, snapshot.borrow().as_ref().unwrap());
                let state: (String, String) = db
                    .query_row(
                        "SELECT status,terminal_state FROM agent_runs WHERE id=?1",
                        [&actor.root_run_id],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .unwrap();
                assert_eq!(state, ("running".into(), "".into()), "{fault}");
                assert_eq!(
                    db.query_row(
                        "SELECT count(*) FROM agent_multi_exit_receipts WHERE root_run_id=?1",
                        [&actor.root_run_id],
                        |r| r.get::<_, i64>(0)
                    )
                    .unwrap(),
                    0,
                    "{fault}"
                );
                assert_eq!(
                    db.query_row(
                        "SELECT count(*) FROM agent_source_model_rounds WHERE root_run_id=?1",
                        [&actor.root_run_id],
                        |r| r.get::<_, i64>(0)
                    )
                    .unwrap(),
                    3
                );
                drop(busy.borrow_mut().take());
                result
            },
        );
        assert!(result.is_err());
        drop(db);
        fs::remove_dir_all(root).unwrap();
        let _ = actor;
    }
}
