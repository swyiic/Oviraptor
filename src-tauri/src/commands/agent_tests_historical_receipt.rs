// Historical saved receipts have their own read-only and recovery contracts.
#[test]
fn old_attempt_saved_receipt_remains_visible_as_read_only_debt() {
    use crate::agent_runtime::multi_agent::directive::reconciliation::{reconcile_historical_received, reconcile_received};
    let (root, context, id, lease, _original_parent) = saved_receipt_fixture(true);
    let connection = db::open(&context.db_path).unwrap();
    assert_eq!(native_scan_status(&connection, &lease.scan_id).unwrap()["historicalPendingReceipts"], 0);
    connection.execute("UPDATE sentinel_scans SET attempt_count=attempt_count+1 WHERE id=?1", [&lease.scan_id]).unwrap();
    let before = receipt_database_snapshot(&connection);
    let status = native_scan_status(&connection, &lease.scan_id).unwrap();
    assert_eq!(status["historicalPendingReceipts"], 1);
    assert_eq!(status["historicalReceiptItems"].as_array().unwrap().len(), 1);
    assert_eq!(status["historicalReceiptItems"][0]["directiveId"], id);
    assert_eq!(status["historicalReceiptItems"][0]["attemptNumber"], lease.attempt_number);
    assert_eq!(status["historicalReceiptItems"][0]["verified"], false);
    assert!(!status["timeline"].as_array().unwrap().iter().any(|item| item["id"] == id));
    assert_eq!(before, receipt_database_snapshot(&connection));
    assert!(reconcile_received(&connection, &lease.scan_id, lease.attempt_number, &id).is_err());
    assert_eq!(before, receipt_database_snapshot(&connection));
    let receipt = reconcile_historical_received(&connection, &lease.scan_id, lease.attempt_number, &id).unwrap();
    assert_eq!(receipt["status"], "completed");
    assert_eq!(receipt["modelRequests"], 0);
    assert_eq!(receipt["targetRequests"], 0);
    assert_eq!(native_scan_status(&connection, &lease.scan_id).unwrap()["historicalPendingReceipts"], 0);
    assert!(native_scan_status(&connection, &lease.scan_id).unwrap()["historicalReceiptItems"].as_array().unwrap().is_empty());
    let after = receipt_database_snapshot(&connection);
    assert_eq!(receipt, reconcile_historical_received(&connection, &lease.scan_id, lease.attempt_number, &id).unwrap());
    assert_eq!(after, receipt_database_snapshot(&connection));
    let original_root: (String,String) = connection.query_row("SELECT status,terminal_code FROM agent_runs WHERE id=?1", [&lease.root_run_id], |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
    assert_eq!(original_root.0, "terminal");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn historical_saved_receipt_rejects_rebound_or_unproven_source_without_writes() {
    use crate::agent_runtime::multi_agent::directive::reconciliation::reconcile_historical_received;
    for mutation in [
        "UPDATE agent_coordinator_leases SET fencing_token='replacement'",
        "UPDATE agent_user_directives SET confirmed_hash='changed'",
        "UPDATE agent_assignments SET task_slice_json='{}'",
        "DELETE FROM agent_events WHERE event_type='model_round_completed'",
        "UPDATE agent_budget_ledger SET fencing_token='other'",
        "INSERT INTO sentinel_deleted_scans(scan_id) SELECT id FROM sentinel_scans",
    ] {
        let (root, context, id, lease, _original_parent) = saved_receipt_fixture(true);
        let connection = db::open(&context.db_path).unwrap();
        connection.execute("UPDATE sentinel_scans SET attempt_count=attempt_count+1 WHERE id=?1", [&lease.scan_id]).unwrap();
        connection.execute_batch(mutation).unwrap();
        let before = receipt_database_snapshot(&connection);
        assert!(reconcile_historical_received(&connection, &lease.scan_id, lease.attempt_number, &id).is_err(), "{mutation}");
        assert_eq!(before, receipt_database_snapshot(&connection), "{mutation}");
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn historical_saved_receipt_concurrent_replays_settle_once() {
    use crate::agent_runtime::multi_agent::directive::reconciliation::reconcile_historical_received;
    let (root, context, id, lease, _original_parent) = saved_receipt_fixture(true);
    let connection = db::open(&context.db_path).unwrap();
    connection.execute("UPDATE sentinel_scans SET attempt_count=attempt_count+1 WHERE id=?1", [&lease.scan_id]).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let handles: Vec<_> = (0..2).map(|_| {
        let path = context.db_path.clone();
        let id = id.clone();
        let lease = lease.clone();
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            let connection = db::open(&path).unwrap();
            barrier.wait();
            reconcile_historical_received(&connection, &lease.scan_id, lease.attempt_number, &id).unwrap()
        })
    }).collect();
    barrier.wait();
    let receipts: Vec<_> = handles.into_iter().map(|handle| handle.join().unwrap()).collect();
    assert_eq!(receipts[0], receipts[1]);
    let counts: (i64,i64,i64) = connection.query_row(
        "SELECT (SELECT COUNT(*) FROM agent_messages WHERE kind='human_assessment_result'),
                (SELECT SUM(delivery_attempts) FROM agent_messages WHERE kind='human_assessment_result'),
                (SELECT spent_tokens FROM agent_budget_ledger WHERE root_run_id=?1)",
        [&lease.root_run_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).unwrap();
    assert_eq!(counts, (1,1,20));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn historical_saved_receipt_write_failure_keeps_prior_attempt_intact() {
    use crate::agent_runtime::multi_agent::directive::reconciliation::reconcile_historical_received;
    let (root, context, id, lease, _original_parent) = saved_receipt_fixture(true);
    let connection = db::open(&context.db_path).unwrap();
    connection.execute("UPDATE sentinel_scans SET attempt_count=attempt_count+1 WHERE id=?1", [&lease.scan_id]).unwrap();
    connection.execute_batch("CREATE TRIGGER historical_result_failure BEFORE INSERT ON agent_messages WHEN NEW.kind='human_assessment_result' BEGIN SELECT RAISE(ABORT,'historical_result_failure'); END;").unwrap();
    let before = receipt_database_snapshot(&connection);
    assert!(reconcile_historical_received(&connection, &lease.scan_id, lease.attempt_number, &id).unwrap_err().contains("historical_result_failure"));
    assert_eq!(before, receipt_database_snapshot(&connection));
    connection.execute_batch("DROP TRIGGER historical_result_failure").unwrap();
    assert_eq!(reconcile_historical_received(&connection, &lease.scan_id, lease.attempt_number, &id).unwrap()["status"], "completed");
    let _ = fs::remove_dir_all(root);
}

