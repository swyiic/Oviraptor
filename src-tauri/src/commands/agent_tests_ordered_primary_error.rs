// Provider errors must remain primary; temporary SQLite and real localhost SDK only.
#[test]
fn ordered_execution_transport_error_retains_primary_reason_and_original_unknown_fee() {
    for status in [401, 503] {
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(move |_|
            (status, "application/json", r#"{"error":{"message":"fixture rejected","type":"fixture_error"}}"#.into())));
        let (f, id) = ordered_exec_fixture("ordered-primary-transport", &format!("http://127.0.0.1:{port}/v1"));
        let db = db::open(&f.context.db_path).unwrap();
        let native = ordered_exec_native(&db, &f.actor.root_run_id);
        let error = ordered_exec_apply(&f).unwrap_err();
        assert!(error.starts_with("deep_investigator 子智能体模型调用失败："), "{status}: {error}");
        assert!(error.contains(";ordered_proposal_receipt:"), "{error}");
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert!(ordered_exec_receipts(&db, &id).is_empty());
        let assignment: String = db.query_row("SELECT assignment_id FROM agent_directive_ordered_actions WHERE directive_id=?1", [&id], |r|r.get(0)).unwrap();
        let fee = crate::agent_runtime::multi_agent::budget::balance(&db, &f.actor.root_run_id, Some(&assignment), "model_requests").unwrap();
        assert_eq!(fee.indeterminate, 1);
        assert_eq!(fee.consumed, 0);
        assert_eq!(ordered_exec_native(&db, &f.actor.root_run_id), native);
        let before = receipt_database_snapshot(&db);
        let _ = apply_ordered_human_actions(&f.context, &f.actor);
        assert!(receipt_database_snapshot(&db) == before, "local paid replay cannot change stored rows");
        assert_eq!(seen.lock().unwrap().len(), 1, "unknown provider outcome cannot be resent");
    }
}

#[test]
fn ordered_execution_unreported_usage_retains_original_reconciliation_reason() {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        let mut body: JsonValue = serde_json::from_str(&proposal_model_response(valid_proposal_text())).unwrap();
        body.as_object_mut().unwrap().remove("usage");
        (200, "application/json", body.to_string())
    }));
    let (f, id) = ordered_exec_fixture("ordered-primary-usage", &format!("http://127.0.0.1:{port}/v1"));
    let db = db::open(&f.context.db_path).unwrap();
    let error = ordered_exec_apply(&f).unwrap_err();
    assert!(error.starts_with("budget_indeterminate_requires_reconciliation;ordered_proposal_receipt:"), "{error}");
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert!(ordered_exec_receipts(&db, &id).is_empty());
    let assignment: String = db.query_row("SELECT assignment_id FROM agent_directive_ordered_actions WHERE directive_id=?1", [&id], |r|r.get(0)).unwrap();
    let fee = crate::agent_runtime::multi_agent::budget::balance(&db, &f.actor.root_run_id, Some(&assignment), "model_requests").unwrap();
    assert_eq!((fee.consumed, fee.indeterminate), (1, 0));
    let tokens = crate::agent_runtime::multi_agent::budget::balance(&db, &f.actor.root_run_id, Some(&assignment), "model_input_tokens").unwrap();
    assert_eq!(tokens.indeterminate, 4000);
    let before = receipt_database_snapshot(&db);
    let _ = apply_ordered_human_actions(&f.context, &f.actor);
    assert!(receipt_database_snapshot(&db) == before, "local reconciliation replay cannot change stored rows");
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn ordered_execution_pause_keeps_actual_sdk_cancellation_primary_over_parent_check() {
    use std::sync::{mpsc, Arc, Mutex};
    let (arrive_tx, arrive) = mpsc::channel();
    let (release_tx, release) = mpsc::channel();
    let release = Arc::new(Mutex::new(release));
    let (port, _seen, _stop) = spawn_endpoint(Arc::new(move |_| {
        arrive_tx.send(()).unwrap();
        let _ = release.lock().unwrap().recv_timeout(Duration::from_secs(8));
        (200, "application/json", proposal_model_response(valid_proposal_text()))
    }));
    let (f, id) = ordered_exec_fixture("ordered-primary-pause", &format!("http://127.0.0.1:{port}/v1"));
    let db = db::open(&f.context.db_path).unwrap();
    let error = std::thread::scope(|scope| {
        let (done_tx, done) = mpsc::channel();
        let fixture = &f;
        scope.spawn(move || { let _ = done_tx.send(ordered_exec_apply(fixture)); });
        arrive.recv_timeout(Duration::from_secs(5)).unwrap();
        db.execute("UPDATE sentinel_scans SET status='pausing' WHERE id=?1", [&f.actor.scan_id]).unwrap();
        let early = done.recv_timeout(Duration::from_secs(3));
        release_tx.send(()).unwrap();
        early.unwrap().unwrap_err()
    });
    assert!(error.starts_with("deep_investigator 子智能体模型调用失败："), "{error}");
    assert!(error.contains(";ordered_proposal_receipt:"), "{error}");
    assert!(ordered_exec_receipts(&db, &id).is_empty());
    let assignment: String = db.query_row("SELECT assignment_id FROM agent_directive_ordered_actions WHERE directive_id=?1", [&id], |r|r.get(0)).unwrap();
    let fee = crate::agent_runtime::multi_agent::budget::balance(&db, &f.actor.root_run_id, Some(&assignment), "model_requests").unwrap();
    assert_eq!(fee.indeterminate, 1);
    assert_eq!(ordered_exec_count(&db, "agent_directive_ordered_actions"), 1);
}
