// Actual provider accepted the original request before these task-only faults.
// Invalid business state cannot erase a known, reported original SDK invoice.
#[test]
fn client_side_actual_paid_task_change_retains_original_invoice_without_semantic_delivery() {
    use crate::agent_runtime::multi_agent::{attempts, budget};
    for mutation in [
        "UPDATE agent_assignments SET task_slice_json=json_set(task_slice_json,'$.phase','changed_after_dispatch') WHERE role='client_side';",
        "UPDATE agent_assignments SET task_slice_json='{}' WHERE role='client_side';",
    ] {
        let mut f = client_hook_fixture("client-paid-task-change", 60000, 20);
        let costs = client_hook_target_costs(&f.db);
        let control = client_hook_rows(&f.db, "agent_root_budget_attempts");
        *f.model_boundary_sql.lock().unwrap() = Some(mutation.into());
        let error = f.finish().unwrap_err();
        assert_eq!(f.model_seen.lock().unwrap().len(), 1, "actual provider response required: {error}");
        assert_eq!(f.site_seen.lock().unwrap().len(), 1);
        let (assignment, run): (String, String) = f.db.query_row(
            "SELECT id,child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND role='client_side'",
            [&f.root], |r| Ok((r.get(0)?, r.get(1)?)),
        ).unwrap();
        let lease = &f.session.as_ref().unwrap().lease;
        let worker = attempts::current(&f.db, lease, &assignment).unwrap();
        assert_eq!(worker.child_run_id, run);
        assert_eq!(worker.coordinator_epoch, lease.lease_epoch);
        assert_eq!(worker.coordinator_fencing_token, lease.fencing_token);
        for (dimension, paid) in [("model_input_tokens",10),("model_cached_tokens",0),("model_output_tokens",10),("model_requests",1)] {
            let actual = budget::balance_for_attempt(&f.db, &f.root, &assignment, &worker.id, dimension).unwrap();
            assert_eq!(actual.consumed, paid, "{dimension}: known paid original invoice was lost: {error}");
            assert_eq!(actual.indeterminate, 0, "reported usage is known, not an unknown resend permission");
        }
        let (state, response, hash): (String, String, String) = f.db.query_row(
            "SELECT state,response_json,response_hash FROM agent_specialist_calls WHERE assignment_id=?1 AND child_run_id=?2",
            params![assignment, run], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
        ).unwrap();
        // Business publication is quarantined; the known invoice is the
        // immutable original fee-only fact, never a fabricated semantic receipt.
        assert_eq!(state, "executing");
        assert_eq!(response, "{}");
        assert_eq!(hash, "");
        let (fact, request_hash): (String, String) = f.db.query_row(
            "SELECT f.fact_json,c.request_hash FROM agent_model_cost_facts f
             JOIN agent_specialist_calls c ON c.child_run_id=f.child_run_id AND c.assignment_id=f.assignment_id
               AND c.root_run_id=f.root_run_id AND c.request_hash=f.request_hash AND c.lease_epoch=f.lease_epoch
               AND c.fencing_token=f.fencing_token
             WHERE f.family='specialist' AND f.root_run_id=?1 AND f.assignment_id=?2 AND f.child_run_id=?3
               AND f.lease_attempt_id=?4 AND f.lease_epoch=?5 AND f.fencing_token=?6
               AND f.round_number=1 AND f.phase='received' AND c.role='client_side'",
            params![f.root,assignment,run,worker.id,lease.lease_epoch,lease.fencing_token],
            |r| Ok((r.get(0)?,r.get(1)?)),
        ).unwrap();
        let fact: JsonValue = serde_json::from_str(&fact).unwrap();
        assert_eq!(fact["usageReported"], true);
        let usage = crate::agent_runtime::store::UsageDelta { input_tokens:10,cached_input_tokens:0,
            output_tokens:10,total_tokens:20,model_requests:1 };
        assert_eq!(fact["usage"], usage.as_json());
        let receipt_hash = fact["responseHash"].as_str().unwrap();
        assert_eq!(receipt_hash.len(), 64);
        budget::receipts::verify(&f.db, lease, &assignment,
            &format!("specialist:{assignment}:{request_hash}"), receipt_hash, &usage).unwrap();
        assert_eq!(f.db.query_row("SELECT count(*) FROM agent_model_cost_facts WHERE child_run_id=?1", [&run],
            |r| r.get::<_,i64>(0)).unwrap(), 1);
        assert_eq!(f.db.query_row("SELECT count(*) FROM agent_messages WHERE assignment_id=?1", [&assignment], |r| r.get::<_,i64>(0)).unwrap(), 0);
        assert_eq!(f.db.query_row("SELECT count(*) FROM agent_assignments WHERE id=?1 AND state='completed'", [&assignment], |r| r.get::<_,i64>(0)).unwrap(), 0);
        assert_eq!(f.db.query_row("SELECT count(*) FROM sentinel_findings", [], |r| r.get::<_,i64>(0)).unwrap(), 0);
        assert_eq!(client_hook_target_costs(&f.db), costs);
        assert_eq!(client_hook_rows(&f.db, "agent_root_budget_attempts"), control);
        client_hook_assert_supplier_closure(&f);
    }
}

#[test]
fn client_side_actual_paid_history_missing_coarse_ledger_cannot_be_recreated_for_new_grant() {
    use crate::agent_runtime::{contract::AgentRole, multi_agent::scheduler};
    let mut f = client_hook_fixture("client-paid-missing-coarse", 60000, 20);
    f.finish().unwrap();
    assert_eq!(f.model_seen.lock().unwrap().len(), 1);
    let task: String = f.db.query_row("SELECT task_slice_json FROM agent_assignments WHERE coordinator_run_id=?1 AND role='client_side'",
        [&f.root], |r| r.get(0)).unwrap();
    assert_eq!(f.db.execute("DELETE FROM agent_budget_ledger WHERE root_run_id=?1", [&f.root]).unwrap(), 1);
    let before = client_delivery_all_rows(&f.db);
    let task: JsonValue = serde_json::from_str(&task).unwrap();
    let error = scheduler::prepare_readonly_child(&f.db, &f.session.as_ref().unwrap().lease,
        AgentRole::ClientSide, "new-grant-after-lost-ledger", &task, 8000).unwrap_err();
    assert_eq!(error, "client_side_budget_ledger_missing_history");
    assert!(client_delivery_all_rows(&f.db) == before, "missing historical ledger must not be backfilled and no original row may change");
    assert_eq!(f.model_seen.lock().unwrap().len(), 1);
    assert_eq!(f.site_seen.lock().unwrap().len(), 1);
    assert_eq!(f.db.query_row("SELECT count(*) FROM agent_budget_ledger WHERE root_run_id=?1", [&f.root], |r| r.get::<_,i64>(0)).unwrap(), 0);
}
