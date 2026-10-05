// Started local work is not proof that any model request was dispatched.
fn ordered_closure_started_without_dispatch(f: &RootTickFixture) -> crate::agent_runtime::multi_agent::directive::ordered_execution::ActionJob {
    use crate::agent_runtime::multi_agent::directive::ordered_execution as ordered;
    let job = ordered_closure_prepared(f);
    let db = db::open(&f.context.db_path).unwrap();
    ordered::consume_request(&db, &job.scope, &job, |_| Ok(())).unwrap();
    assert!(ordered::start(&db, &job.scope, &job.directive_id, job.action.order, |_| Ok(())).unwrap());
    ordered::load(&db, &job.directive_id, job.action.order).unwrap().unwrap()
}
#[test]
fn ordered_no_dispatch_closure_started_original_worker_releases_reservation_and_replay_is_pure() {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| (200, "application/json", proposal_model_response(valid_proposal_text()))));
    let (f, id) = ordered_exec_fixture("ordered-no-dispatch", &format!("http://127.0.0.1:{port}/v1"));
    let job = ordered_closure_started_without_dispatch(&f);
    let db = db::open(&f.context.db_path).unwrap();
    let native = ordered_exec_native(&db, &f.actor.root_run_id);
    let original = crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,
        "SELECT rowid,* FROM agent_directive_ordered_actions WHERE directive_id=?1", [&id]).unwrap();
    assert_eq!(job.state, "executing");
    assert_eq!(ordered_exec_count(&db, "agent_specialist_calls"), 0);
    let outcome = AgentTargetOutcome::incomplete("local worker started, model dispatch never created");
    finish_coordinator_run(&db, &f.actor, &outcome).unwrap();
    ordered_closure_assert_cancelled(&db, &job);
    for dimension in ["model_input_tokens", "model_cached_tokens", "model_output_tokens", "model_requests"] {
        let balance = crate::agent_runtime::multi_agent::budget::balance(&db, &f.actor.root_run_id, Some(&job.child.assignment_id), dimension).unwrap();
        assert_eq!((balance.reserved, balance.consumed, balance.indeterminate), (0,0,0));
    }
    assert!(crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,
        "SELECT rowid,* FROM agent_directive_ordered_actions WHERE directive_id=?1", [&id]).unwrap()==original);
    assert_eq!(ordered_exec_projection(&db, &id)["actions"][0]["state"], "cancelled_before_dispatch");
    assert!(ordered_exec_receipts(&db, &id).is_empty());
    assert_eq!(ordered_exec_native(&db, &f.actor.root_run_id), native);
    let before = super::tests::application_table_snapshot(&db);
    finish_coordinator_run(&db, &f.actor, &outcome).unwrap();
    assert!(super::tests::application_table_snapshot(&db)==before);
    assert_eq!(seen.lock().unwrap().len(), 0);
}

#[test]
fn ordered_no_dispatch_closure_second_started_preserves_first_actual_paid_receipt() {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| (200, "application/json", proposal_model_response(valid_proposal_text()))));
    let (f, id) = ordered_exec_fixture("ordered-no-dispatch-second", &format!("http://127.0.0.1:{port}/v1"));
    let db = db::open(&f.context.db_path).unwrap();
    ordered_exec_apply(&f).unwrap();
    let paid = ordered_exec_receipts(&db, &id);
    let job = ordered_closure_started_without_dispatch(&f);
    assert_eq!(job.action.order, 2);
    finish_coordinator_run(&db, &f.actor, &AgentTargetOutcome::incomplete("second local start never dispatched")).unwrap();
    ordered_closure_assert_cancelled(&db, &job);
    assert_eq!(ordered_exec_receipts(&db, &id), paid);
    let p = ordered_exec_projection(&db, &id);
    assert_eq!(p["completedAssessments"], 1);
    assert_eq!(p["actions"][1]["state"], "cancelled_before_dispatch");
    let used: (i64,i64) = db.query_row("SELECT spent_tokens,spent_requests FROM agent_budget_ledger WHERE root_run_id=?1", [&f.actor.root_run_id], |r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(used, (20,1));
    assert_eq!(seen.lock().unwrap().len(), 1);
}
#[test]
fn ordered_no_dispatch_closure_damaged_original_proof_cannot_release_or_close_root() {
    for damage in [
        "UPDATE agent_messages SET acknowledged_at='' WHERE kind='human_ordered_assessment_request'",
        "UPDATE agent_runs SET used_requests=1 WHERE assignment_id<>''",
        "UPDATE agent_assignment_attempts SET state='paused'",
        "UPDATE agent_assignments SET reserved_tokens=3999",
    ] {
        let (f, _) = ordered_exec_fixture("ordered-no-dispatch-damage", "http://127.0.0.1:9/v1");
        let _job = ordered_closure_started_without_dispatch(&f);
        let db = db::open(&f.context.db_path).unwrap();
        db.execute_batch(damage).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        assert!(finish_coordinator_run(&db, &f.actor, &AgentTargetOutcome::incomplete("damaged no-dispatch proof")).is_err(), "{damage}");
        assert!(super::tests::application_table_snapshot(&db)==before, "{damage}: original terminal must entirely roll back");
    }
}
#[test]
fn ordered_no_dispatch_closure_silent_release_or_collateral_write_rolls_back_started_worker() {
    for fault in [
        "CREATE TRIGGER local_start_close_fault BEFORE UPDATE OF state ON agent_assignments WHEN NEW.state='cancelled' BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER local_start_close_fault BEFORE INSERT ON agent_budget_entries WHEN NEW.kind='release' BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER local_start_close_fault BEFORE UPDATE OF state ON agent_assignment_attempts WHEN NEW.state='cancelled' BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER local_start_close_fault AFTER UPDATE OF state ON agent_assignments WHEN NEW.state='cancelled' BEGIN UPDATE projects SET name='unauthorized-local-start'; END;",
    ] {
        let (f, _) = ordered_exec_fixture("ordered-no-dispatch-fault", "http://127.0.0.1:9/v1");
        let _job = ordered_closure_started_without_dispatch(&f);
        let db = db::open(&f.context.db_path).unwrap();
        db.execute_batch(fault).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        assert!(finish_coordinator_run(&db, &f.actor, &AgentTargetOutcome::incomplete("faulted local start closure")).is_err(), "{fault}");
        assert!(super::tests::application_table_snapshot(&db)==before, "{fault}: original terminal must entirely roll back");
    }
}
#[test]
fn ordered_no_dispatch_cancellation_helper_rejects_actual_sent_unknown_call_without_changing_any_row() {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| (503, "application/json", r#"{"error":{"message":"fixture unavailable"}}"#.into())));
    let (f, id) = ordered_exec_fixture("ordered-no-dispatch-sent", &format!("http://127.0.0.1:{port}/v1"));
    let db = db::open(&f.context.db_path).unwrap();
    assert!(ordered_exec_apply(&f).is_err());
    let job = crate::agent_runtime::multi_agent::directive::ordered_execution::load(&db, &id, 1).unwrap().unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate).unwrap();
    let error = crate::agent_runtime::multi_agent::scheduler::cancel_started_before_model_dispatch_in_transaction(&tx, &job.scope, &job.child).unwrap_err();
    assert_eq!(error, "child_cancel_requires_original_no_dispatch_proof");
    tx.rollback().unwrap();
    assert!(super::tests::application_table_snapshot(&db)==before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}
