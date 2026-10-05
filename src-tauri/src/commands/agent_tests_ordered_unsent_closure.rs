// Original terminal path and original SDK/fees; temporary SQLite only.
fn ordered_closure_prepared(f: &RootTickFixture) -> crate::agent_runtime::multi_agent::directive::ordered_execution::ActionJob {
    use crate::agent_runtime::multi_agent::directive::ordered_execution as ordered;
    let mut inbox = take_human_directives(&f.context).unwrap();
    let db = db::open(&f.context.db_path).unwrap();
    let job = ordered::prepare_next(&db, inbox.lease.as_ref().unwrap(), &f.context.evidence, |_| Ok(())).unwrap().unwrap();
    inbox.items.clear();
    job
}
fn ordered_closure_assert_cancelled(db: &rusqlite::Connection, job: &crate::agent_runtime::multi_agent::directive::ordered_execution::ActionJob) {
    let state: (String, String, String, i64, i64) = db.query_row("SELECT a.state,r.status,w.state,a.reserved_tokens,a.reserved_requests FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id JOIN agent_assignment_attempts w ON w.assignment_id=a.id AND w.child_run_id=r.id WHERE a.id=?1", [&job.child.assignment_id], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
    assert_eq!(state, ("cancelled".into(), "terminal".into(), "cancelled".into(), 0, 0));
    let active: i64 = db.query_row("SELECT (SELECT COUNT(*) FROM agent_lane_leases WHERE assignment_id=?1)+(SELECT COUNT(*) FROM agent_capability_leases WHERE assignment_id=?1 AND revoked_at='')", [&job.child.assignment_id], |r|r.get(0)).unwrap();
    assert_eq!(active, 0);
}
#[test]
fn ordered_unsent_closure_original_terminal_releases_only_prepared_worker_and_replay_is_pure() {
    let (f, id) = ordered_exec_fixture("ordered-close-prepared", "http://127.0.0.1:9/v1");
    let job = ordered_closure_prepared(&f);
    let db = db::open(&f.context.db_path).unwrap();
    let native = ordered_exec_native(&db, &f.actor.root_run_id);
    let outcome = AgentTargetOutcome::incomplete("prepared ordered action closed");
    finish_coordinator_run(&db, &f.actor, &outcome).unwrap();
    ordered_closure_assert_cancelled(&db, &job);
    assert_eq!(ordered_exec_count(&db, "agent_specialist_calls"), 0);
    assert_eq!(ordered_exec_projection(&db, &id)["actions"][0]["state"], "cancelled_before_dispatch");
    assert_eq!(ordered_exec_projection(&db, &id)["completedAssessments"], 0);
    assert!(ordered_exec_receipts(&db, &id).is_empty());
    for dimension in ["model_input_tokens", "model_cached_tokens", "model_output_tokens", "model_requests"] {
        let balance = crate::agent_runtime::multi_agent::budget::balance(&db, &f.actor.root_run_id, Some(&job.child.assignment_id), dimension).unwrap();
        assert_eq!((balance.reserved, balance.consumed, balance.indeterminate), (0,0,0));
    }
    assert_eq!(ordered_exec_native(&db, &f.actor.root_run_id), native);
    let before = receipt_database_snapshot(&db);
    finish_coordinator_run(&db, &f.actor, &outcome).unwrap();
    assert!(receipt_database_snapshot(&db) == before, "original terminal replay changes no stored rows");
}
#[test]
fn ordered_unsent_closure_second_prepared_worker_keeps_first_real_paid_receipt() {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| (200, "application/json", proposal_model_response(valid_proposal_text()))));
    let (f, id) = ordered_exec_fixture("ordered-close-second", &format!("http://127.0.0.1:{port}/v1"));
    let db = db::open(&f.context.db_path).unwrap();
    ordered_exec_apply(&f).unwrap();
    let paid = ordered_exec_receipts(&db, &id);
    let job = ordered_closure_prepared(&f);
    assert_eq!(job.action.order, 2);
    finish_coordinator_run(&db, &f.actor, &AgentTargetOutcome::incomplete("second ordered action closed")).unwrap();
    ordered_closure_assert_cancelled(&db, &job);
    assert_eq!(ordered_exec_receipts(&db, &id), paid);
    assert_eq!(seen.lock().unwrap().len(), 1);
    let projection = ordered_exec_projection(&db, &id);
    assert_eq!(projection["completedAssessments"], 1);
    assert_eq!(projection["actions"][1]["state"], "cancelled_before_dispatch");
    let used: (i64,i64) = db.query_row("SELECT spent_tokens,spent_requests FROM agent_budget_ledger WHERE root_run_id=?1", [&f.actor.root_run_id], |r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(used, (20,1));
}

#[test]
fn ordered_unsent_closure_silent_release_or_collateral_write_rolls_back_original_terminal() {
    for fault in [
        "CREATE TRIGGER ordered_close_fault BEFORE UPDATE OF state ON agent_assignments WHEN NEW.state='cancelled' BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER ordered_close_fault AFTER UPDATE OF state ON agent_assignments WHEN NEW.state='cancelled' BEGIN UPDATE projects SET name='unauthorized-closure'; END;",
        "CREATE TRIGGER ordered_close_fault BEFORE INSERT ON agent_budget_entries WHEN NEW.kind='release' BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER ordered_close_fault BEFORE DELETE ON agent_lane_leases BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER ordered_close_fault BEFORE UPDATE OF revoked_at ON agent_capability_leases BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER ordered_close_fault BEFORE UPDATE OF state ON agent_assignment_attempts WHEN NEW.state='cancelled' BEGIN SELECT RAISE(IGNORE); END;",
    ] {
        let (f, id) = ordered_exec_fixture("ordered-close-fault", "http://127.0.0.1:9/v1");
        let _job = ordered_closure_prepared(&f);
        let db = db::open(&f.context.db_path).unwrap();
        db.execute_batch(fault).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        assert!(finish_coordinator_run(&db, &f.actor, &AgentTargetOutcome::incomplete("faulted ordered closure")).is_err(), "{fault}");
        assert!(super::tests::application_table_snapshot(&db) == before, "{fault}: all stored rows must roll back");
        assert!(ordered_exec_receipts(&db, &id).is_empty());
        assert_eq!(ordered_exec_count(&db, "agent_specialist_calls"), 0);
    }
}
#[test]
fn ordered_unsent_closure_actual_sent_unknown_keeps_original_debt_and_is_not_refunded() {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| (503, "application/json", r#"{"error":{"message":"fixture unavailable"}}"#.into())));
    let (f, id) = ordered_exec_fixture("ordered-close-unknown", &format!("http://127.0.0.1:{port}/v1"));
    let db = db::open(&f.context.db_path).unwrap();
    assert!(ordered_exec_apply(&f).is_err());
    let assignment: String = db.query_row("SELECT assignment_id FROM agent_directive_ordered_actions WHERE directive_id=?1", [&id], |r|r.get(0)).unwrap();
    let fee = crate::agent_runtime::multi_agent::budget::balance(&db, &f.actor.root_run_id, Some(&assignment), "model_requests").unwrap();
    assert_eq!(fee.indeterminate, 1);
    finish_coordinator_run(&db, &f.actor, &AgentTargetOutcome::incomplete("unknown ordered call closed")).unwrap();
    let after = crate::agent_runtime::multi_agent::budget::balance(&db, &f.actor.root_run_id, Some(&assignment), "model_requests").unwrap();
    assert_eq!((after.reserved, after.consumed, after.indeterminate), (fee.reserved, fee.consumed, fee.indeterminate));
    assert!(ordered_exec_receipts(&db, &id).is_empty());
    let projection = ordered_exec_projection(&db, &id);
    assert_eq!(projection["actions"][0]["state"], "outcome_unknown");
    assert_eq!(projection["actions"][1]["state"], "not_started");
    let disposition: String = db.query_row("SELECT json_extract(payload_json,'$.taskClosure.disposition') FROM agent_user_directives WHERE id=?1", [&id], |r|r.get(0)).unwrap();
    assert_eq!(disposition, "reconciliation_required");
    assert!(ordered_exec_apply(&f).is_err());
    assert_eq!(seen.lock().unwrap().len(), 1);
}
