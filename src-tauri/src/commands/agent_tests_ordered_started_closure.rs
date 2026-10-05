// Closing execution authority never refunds an already sent model request.
#[test]
fn ordered_started_closure_actual_unknown_call_releases_execution_leases_and_keeps_every_fee() {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| (503, "application/json", r#"{"error":{"message":"fixture unavailable"}}"#.into())));
    let (f, id) = ordered_exec_fixture("ordered-started-close", &format!("http://127.0.0.1:{port}/v1"));
    let db = db::open(&f.context.db_path).unwrap();
    assert!(ordered_exec_apply(&f).unwrap_err().contains("子智能体模型调用失败"));
    let assignment: String = db.query_row("SELECT assignment_id FROM agent_directive_ordered_actions WHERE directive_id=?1", [&id], |r|r.get(0)).unwrap();
    let original = crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,
        "SELECT rowid,* FROM agent_budget_entries WHERE assignment_id=?1 ORDER BY rowid", [&assignment]).unwrap();
    let call = crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,
        "SELECT rowid,* FROM agent_specialist_calls WHERE assignment_id=?1 ORDER BY rowid", [&assignment]).unwrap();
    let outcome = AgentTargetOutcome::incomplete("sent ordered call ended without result");
    finish_coordinator_run(&db, &f.actor, &outcome).unwrap();
    let active: i64 = db.query_row("SELECT (SELECT COUNT(*) FROM agent_lane_leases WHERE assignment_id=?1)+(SELECT COUNT(*) FROM agent_capability_leases WHERE assignment_id=?1 AND revoked_at='')", [&assignment], |r|r.get(0)).unwrap();
    assert_eq!(active, 0, "original terminal must stop execution authority without releasing unknown fees");
    let state: (String,String,String) = db.query_row("SELECT a.state,r.status,w.state FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id JOIN agent_assignment_attempts w ON w.assignment_id=a.id AND w.child_run_id=r.id WHERE a.id=?1", [&assignment], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(state, ("paused".into(), "paused".into(), "paused".into()));
    assert!(crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,
        "SELECT rowid,* FROM agent_budget_entries WHERE assignment_id=?1 ORDER BY rowid", [&assignment]).unwrap() == original, "original expenses changed");
    assert!(crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,
        "SELECT rowid,* FROM agent_specialist_calls WHERE assignment_id=?1 ORDER BY rowid", [&assignment]).unwrap() == call, "original dispatch changed");
    let reserve: (i64,i64) = db.query_row("SELECT reserved_tokens,reserved_requests FROM agent_assignments WHERE id=?1", [&assignment], |r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(reserve,(4000,1));
    assert!(ordered_exec_receipts(&db, &id).is_empty());
    assert_eq!(ordered_exec_projection(&db, &id)["actions"][0]["state"], "outcome_unknown");
    assert_eq!(seen.lock().unwrap().len(), 1);
    let before = super::tests::application_table_snapshot(&db);
    finish_coordinator_run(&db, &f.actor, &outcome).unwrap();
    assert!(super::tests::application_table_snapshot(&db)==before, "terminal replay must not mutate stored rows");
}

#[test]
fn ordered_started_closure_saved_paid_response_is_paused_without_publishing_or_refunding() {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| (200, "application/json", proposal_model_response(valid_proposal_text()))));
    let (f, id) = ordered_exec_fixture("ordered-started-paid", &format!("http://127.0.0.1:{port}/v1"));
    let db = db::open(&f.context.db_path).unwrap();
    db.execute_batch("CREATE TRIGGER hold_ordered_receipt BEFORE INSERT ON agent_directive_ordered_receipts BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    assert!(ordered_exec_apply(&f).is_err());
    db.execute_batch("DROP TRIGGER hold_ordered_receipt").unwrap();
    let job = crate::agent_runtime::multi_agent::directive::ordered_execution::load(&db, &id, 1).unwrap().unwrap();
    assert_eq!(job.state, "received");
    let original = crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,
        "SELECT rowid,* FROM agent_budget_entries WHERE assignment_id=?1 ORDER BY rowid", [&job.child.assignment_id]).unwrap();
    finish_coordinator_run(&db, &f.actor, &AgentTargetOutcome::incomplete("paid assessment awaits local receipt")).unwrap();
    assert!(crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,
        "SELECT rowid,* FROM agent_budget_entries WHERE assignment_id=?1 ORDER BY rowid", [&job.child.assignment_id]).unwrap() == original);
    let active: i64 = db.query_row("SELECT (SELECT COUNT(*) FROM agent_lane_leases WHERE assignment_id=?1)+(SELECT COUNT(*) FROM agent_capability_leases WHERE assignment_id=?1 AND revoked_at='')", [&job.child.assignment_id], |r|r.get(0)).unwrap();
    assert_eq!(active, 0);
    assert!(ordered_exec_receipts(&db, &id).is_empty());
    assert_eq!(ordered_exec_projection(&db, &id)["actions"][0]["state"], "received");
    let results: i64 = db.query_row("SELECT COUNT(*) FROM agent_messages WHERE assignment_id=?1 AND kind='human_ordered_assessment_result'", [&job.child.assignment_id], |r|r.get(0)).unwrap();
    assert_eq!(results, 0, "terminal cleanup cannot fabricate delivery or ACK");
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn ordered_started_closure_second_unknown_keeps_first_real_paid_receipt_and_identity() {
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = calls.clone();
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(move |_| {
        if counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst)==0 {
            (200, "application/json", proposal_model_response(valid_proposal_text()))
        } else { (503, "application/json", r#"{"error":{"message":"fixture unavailable"}}"#.into()) }
    }));
    let (f, id) = ordered_exec_fixture("ordered-started-second", &format!("http://127.0.0.1:{port}/v1"));
    let db = db::open(&f.context.db_path).unwrap();
    ordered_exec_apply(&f).unwrap();
    let first = ordered_exec_receipts(&db, &id);
    assert!(ordered_exec_apply(&f).is_err());
    finish_coordinator_run(&db, &f.actor, &AgentTargetOutcome::incomplete("second assessment outcome unknown")).unwrap();
    assert_eq!(ordered_exec_receipts(&db, &id), first);
    assert_eq!(ordered_exec_projection(&db, &id)["actions"][1]["state"], "outcome_unknown");
    let second = crate::agent_runtime::multi_agent::directive::ordered_execution::load(&db, &id, 2).unwrap().unwrap();
    let active: i64 = db.query_row("SELECT (SELECT COUNT(*) FROM agent_lane_leases WHERE assignment_id=?1)+(SELECT COUNT(*) FROM agent_capability_leases WHERE assignment_id=?1 AND revoked_at='')", [&second.child.assignment_id], |r|r.get(0)).unwrap();
    assert_eq!(active, 0);
    assert_ne!(second.child.run_id, first[0]["childRunId"].as_str().unwrap());
    assert_eq!(seen.lock().unwrap().len(), 2);
}

#[test]
fn ordered_started_closure_silent_or_collateral_writes_roll_back_whole_root_and_preserve_original_cost() {
    for fault in [
        "CREATE TRIGGER started_close_fault BEFORE UPDATE OF state ON agent_assignments WHEN NEW.failure_class='ordered_task_ended_reconciliation_required' BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER started_close_fault BEFORE UPDATE OF revoked_at ON agent_capability_leases BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER started_close_fault BEFORE DELETE ON agent_lane_leases BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER started_close_fault BEFORE UPDATE OF state ON agent_assignment_attempts WHEN NEW.failure_class='ordered_task_ended_reconciliation_required' BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER started_close_fault AFTER UPDATE OF state ON agent_assignments WHEN NEW.failure_class='ordered_task_ended_reconciliation_required' BEGIN UPDATE projects SET name='unauthorized-started-closure'; END;",
        "CREATE TRIGGER started_close_fault AFTER UPDATE OF state ON agent_assignments WHEN NEW.failure_class='ordered_task_ended_reconciliation_required' BEGIN UPDATE agent_budget_ledger SET reserved_requests=0; END;",
    ] {
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| (503, "application/json", r#"{"error":{"message":"fixture unavailable"}}"#.into())));
        let (f, id) = ordered_exec_fixture("ordered-started-fault", &format!("http://127.0.0.1:{port}/v1"));
        let db = db::open(&f.context.db_path).unwrap();
        assert!(ordered_exec_apply(&f).is_err());
        db.execute_batch(fault).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        assert!(finish_coordinator_run(&db, &f.actor, &AgentTargetOutcome::incomplete("faulted started closure")).is_err(), "{fault}");
        assert!(super::tests::application_table_snapshot(&db)==before, "{fault}: original root transaction must roll back all rows");
        assert!(ordered_exec_receipts(&db, &id).is_empty());
        assert_eq!(seen.lock().unwrap().len(), 1);
    }
}
