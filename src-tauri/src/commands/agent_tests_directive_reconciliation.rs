fn saved_receipt_fixture(
    valid: bool,
) -> (
    std::path::PathBuf,
    AgentRunContext,
    String,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    LiveProposalGuard,
) {
    use crate::agent_runtime::multi_agent::directive::proposals;
    let (root, context, id, _original_parent) = proposal_fixture("saved-receipt", "@mapper 请分析已有证据");
    let lease = take_human_directives(&context).unwrap().lease.unwrap();
    let connection = db::open(&context.db_path).unwrap();
    let job = proposals::prepare_next(&connection, &lease, &context.evidence)
        .unwrap()
        .unwrap();
    consume_proposal_mailbox(
        &connection,
        &lease,
        &job.child.run_id,
        &job.request_message_id,
        "human_assessment_request",
        &job.input,
    )
    .unwrap();
    assert!(proposals::start(&connection, &lease, &id).unwrap());
    let mut response = validated_human_proposal(valid_proposal_text(), false);
    response["valid"] = serde_json::json!(valid);
    let usage = serde_json::json!({"inputTokens":10,"cachedInputTokens":0,"outputTokens":10,"totalTokens":20,"modelRequests":1});
    proposals::record_response(&connection, &lease, &id, &response, &usage).unwrap();
    finish_coordinator_run(&connection, &lease, &AgentTargetOutcome::Cancelled).unwrap();
    (root, context, id, lease, _original_parent)
}

// Compare all affected rows, including event sequences and timestamps, across
// failures and replays. A count-only assertion would miss partial settlement.
fn receipt_database_snapshot(
    connection: &rusqlite::Connection,
) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    [
        "agent_user_directives",
        "agent_directive_proposals",
        "agent_assignments",
        "agent_assignment_attempts",
        "agent_runs",
        "agent_messages",
        "agent_budget_ledger",
        "agent_budget_entries",
        "agent_budget_limits",
        "agent_budget_clock_origins",
        "agent_web_model_journal",
        "agent_capability_leases",
        "agent_lane_leases",
        "agent_coordinator_leases",
        "agent_contract_owners",
        "agent_collaboration_events",
    ]
    .iter()
    .map(|table| {
        let mut query = connection
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
            .unwrap();
        let columns = query.column_count();
        query
            .query_map([], |row| {
                (0..columns).map(|column| row.get(column)).collect()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    })
    .collect()
}

#[test]
fn directive_reconciliation_saved_assessment_settles_once_without_reopening_root() {
    use crate::agent_runtime::multi_agent::directive::reconciliation::reconcile_received;
    for valid in [true, false] {
        let (root, context, id, lease, _original_parent) = saved_receipt_fixture(valid);
        let connection = db::open(&context.db_path).unwrap();
        connection
            .execute(
                "UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00'",
                [],
            )
            .unwrap();
        let original: (String, String, String) = connection.query_row(
            "SELECT json_extract(d.payload_json,'$.taskClosure'),d.finished_at,r.terminal_code FROM agent_user_directives d JOIN agent_runs r ON r.id=d.root_run_id WHERE d.id=?1",
            [&id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
        let result =
            reconcile_received(&connection, &lease.scan_id, lease.attempt_number, &id).unwrap();
        assert_eq!(result["status"], if valid { "completed" } else { "failed" });
        assert_eq!(result["modelRequests"], 0);
        assert_eq!(result["targetRequests"], 0);
        assert_eq!(result["advisoryOnly"], true);
        let after: (String, String, String) = connection.query_row(
            "SELECT json_extract(d.payload_json,'$.taskClosure'),d.finished_at,r.terminal_code FROM agent_user_directives d JOIN agent_runs r ON r.id=d.root_run_id WHERE d.id=?1",
            [&id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
        assert_eq!(original, after);
        let budget: (i64,i64,i64,i64) = connection.query_row("SELECT reserved_tokens,reserved_requests,spent_tokens,spent_requests FROM agent_budget_ledger", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(budget, (0, 0, 20, 1));
        let deliveries: i64 = connection
            .query_row(
                "SELECT delivery_attempts FROM agent_messages WHERE id=?1",
                [result["resultMessageId"].as_str().unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(deliveries, 1);
        let before_replay = receipt_database_snapshot(&connection);
        assert_eq!(
            result,
            reconcile_received(&connection, &lease.scan_id, lease.attempt_number, &id).unwrap()
        );
        assert_eq!(before_replay, receipt_database_snapshot(&connection));
        let status = native_scan_status(&connection, &lease.scan_id).unwrap();
        let item = status["timeline"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == id)
            .unwrap();
        assert_eq!(item["localReconciliation"], result);
        for expression in [
            "json_remove(payload_json,'$.localReconciliation')",
            "json_set(payload_json,'$.localReconciliation.status','forged')",
        ] {
            assert!(connection
                .execute(
                    &format!(
                        "UPDATE agent_user_directives SET payload_json={expression} WHERE id=?1"
                    ),
                    [&id]
                )
                .unwrap_err()
                .to_string()
                .contains("directive_local_reconciliation_immutable"));
        }
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn directive_reconciliation_failure_at_each_write_rolls_back_every_row() {
    use crate::agent_runtime::multi_agent::directive::reconciliation::reconcile_received;
    for (table, operation) in [
        ("agent_budget_ledger", "UPDATE"),
        ("agent_runs", "UPDATE"),
        ("agent_assignments", "UPDATE"),
        ("agent_lane_leases", "DELETE"),
        ("agent_messages", "INSERT"),
        ("agent_messages", "UPDATE"),
        ("agent_directive_proposals", "UPDATE"),
        ("agent_user_directives", "UPDATE"),
    ] {
        let (root, context, id, lease, _original_parent) = saved_receipt_fixture(true);
        let connection = db::open(&context.db_path).unwrap();
        connection.execute_batch(&format!("CREATE TRIGGER injected_failure BEFORE {operation} ON {table} BEGIN SELECT RAISE(ABORT,'injected_receipt_failure'); END;")).unwrap();
        let before = receipt_database_snapshot(&connection);
        let error =
            reconcile_received(&connection, &lease.scan_id, lease.attempt_number, &id).unwrap_err();
        assert!(
            error.contains("injected_receipt_failure"),
            "{table} {operation}: {error}"
        );
        assert_eq!(
            before,
            receipt_database_snapshot(&connection),
            "{table} {operation}"
        );
        connection
            .execute_batch("DROP TRIGGER injected_failure")
            .unwrap();
        reconcile_received(&connection, &lease.scan_id, lease.attempt_number, &id).unwrap();
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn directive_reconciliation_rejects_wrong_scope_and_corrupted_request_without_writes() {
    use crate::agent_runtime::multi_agent::directive::reconciliation::reconcile_received;
    for mutation in [
        "UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1,fencing_token='replaced'",
        "UPDATE sentinel_scans SET attempt_count=attempt_count+1",
        "UPDATE agent_runs SET status='running' WHERE role='coordinator'",
        "UPDATE agent_messages SET payload_json='{}' WHERE kind='human_assessment_request'",
        "UPDATE agent_messages SET acknowledged_at='' WHERE kind='human_assessment_request'",
        "UPDATE agent_assignments SET reserved_tokens=1",
    ] {
        let (root, context, id, lease, _original_parent) = saved_receipt_fixture(true);
        let connection = db::open(&context.db_path).unwrap();
        connection.execute_batch(mutation).unwrap();
        let before = receipt_database_snapshot(&connection);
        assert!(
            reconcile_received(&connection, &lease.scan_id, lease.attempt_number, &id).is_err(),
            "{mutation}"
        );
        assert_eq!(before, receipt_database_snapshot(&connection));
        assert!(reconcile_received(&connection, "other-scan", lease.attempt_number, &id).is_err());
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn directive_reconciliation_replay_revalidates_acknowledgement_and_settlement() {
    use crate::agent_runtime::multi_agent::directive::reconciliation::reconcile_received;
    for mutation in [
        "UPDATE agent_messages SET payload_json='{}' WHERE kind='human_assessment_result'",
        "UPDATE agent_messages SET acknowledged_at='' WHERE kind='human_assessment_result'",
        "UPDATE agent_runs SET used_tokens=0 WHERE role='spa_api_mapper'",
        "UPDATE agent_assignments SET budget_settled_at=''",
        "UPDATE agent_capability_leases SET revoked_at=''",
    ] {
        let (root, context, id, lease, _original_parent) = saved_receipt_fixture(true);
        let connection = db::open(&context.db_path).unwrap();
        reconcile_received(&connection, &lease.scan_id, lease.attempt_number, &id).unwrap();
        connection.execute_batch(mutation).unwrap();
        let before = receipt_database_snapshot(&connection);
        assert!(
            reconcile_received(&connection, &lease.scan_id, lease.attempt_number, &id).is_err(),
            "{mutation}"
        );
        assert_eq!(before, receipt_database_snapshot(&connection));
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn directive_reconciliation_concurrent_requests_share_one_receipt() {
    use crate::agent_runtime::multi_agent::directive::reconciliation::reconcile_received;
    let (root, context, id, lease, _original_parent) = saved_receipt_fixture(true);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let path = context.db_path.clone();
            let id = id.clone();
            let lease = lease.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let connection = db::open(&path).unwrap();
                barrier.wait();
                reconcile_received(&connection, &lease.scan_id, lease.attempt_number, &id).unwrap()
            })
        })
        .collect();
    barrier.wait();
    let receipts: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(receipts[0], receipts[1]);
    let connection = db::open(&context.db_path).unwrap();
    let totals: (i64,i64) = connection.query_row("SELECT COUNT(*),SUM(delivery_attempts) FROM agent_messages WHERE kind='human_assessment_result'", [], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(totals, (1, 1));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn directive_reconciliation_only_accepts_saved_terminal_assessments() {
    use crate::agent_runtime::multi_agent::directive::{
        proposals, reconciliation::reconcile_received,
    };
    for state in ["prepared", "executing", "uncertain", "received_active"] {
        let (root, context, id, _original_parent) = proposal_fixture("receipt-ineligible", "@mapper 请分析已有证据");
        let lease = take_human_directives(&context).unwrap().lease.unwrap();
        let connection = db::open(&context.db_path).unwrap();
        let job = proposals::prepare_next(&connection, &lease, &context.evidence)
            .unwrap()
            .unwrap();
        if state != "prepared" {
            consume_proposal_mailbox(
                &connection,
                &lease,
                &job.child.run_id,
                &job.request_message_id,
                "human_assessment_request",
                &job.input,
            )
            .unwrap();
            proposals::start(&connection, &lease, &id).unwrap();
        }
        if state == "uncertain" {
            proposals::mark_uncertain(&connection, &lease, &id).unwrap();
        }
        if state == "received_active" {
            proposals::record_response(&connection, &lease, &id,
                &validated_human_proposal(valid_proposal_text(), false),
                &serde_json::json!({"inputTokens":10,"cachedInputTokens":0,"outputTokens":10,"totalTokens":20,"modelRequests":1})).unwrap();
        } else {
            finish_coordinator_run(&connection, &lease, &AgentTargetOutcome::Cancelled).unwrap();
        }
        let before = receipt_database_snapshot(&connection);
        assert!(
            reconcile_received(&connection, &lease.scan_id, lease.attempt_number, &id).is_err(),
            "{state}"
        );
        assert_eq!(before, receipt_database_snapshot(&connection));
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn directive_reconciliation_rejects_deleted_scans_and_changed_source() {
    use crate::agent_runtime::multi_agent::directive::reconciliation::reconcile_received;
    for mutation in [
        "INSERT INTO sentinel_deleted_scans(scan_id) SELECT id FROM sentinel_scans",
        "UPDATE agent_user_directives SET confirmed_hash='damaged'",
        "UPDATE agent_user_directives SET text_redacted='changed instruction'",
        "UPDATE agent_assignments SET evidence_revision=evidence_revision+1",
        "UPDATE agent_assignments SET task_slice_json='{}'",
        "UPDATE agent_runs SET target_url='https://other.invalid' WHERE role='spa_api_mapper'",
    ] {
        let (root, context, id, lease, _original_parent) = saved_receipt_fixture(true);
        let connection = db::open(&context.db_path).unwrap();
        connection.execute_batch(mutation).unwrap();
        let before = receipt_database_snapshot(&connection);
        assert!(
            reconcile_received(&connection, &lease.scan_id, lease.attempt_number, &id).is_err(),
            "{mutation}"
        );
        assert_eq!(before, receipt_database_snapshot(&connection));
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn directive_reconciliation_revalidates_fence_after_writer_contention() {
    use crate::agent_runtime::multi_agent::directive::reconciliation::reconcile_received;
    use std::sync::atomic::{AtomicBool, Ordering};
    static WAITING: AtomicBool = AtomicBool::new(false);
    WAITING.store(false, Ordering::SeqCst);
    let (root, context, id, lease, _original_parent) = saved_receipt_fixture(true);
    let connection = db::open(&context.db_path).unwrap();
    let worker = db::open(&context.db_path).unwrap();
    worker
        .busy_handler(Some(|attempt| {
            WAITING.store(true, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(1));
            attempt < 5_000
        }))
        .unwrap();
    let tx =
        rusqlite::Transaction::new_unchecked(&connection, rusqlite::TransactionBehavior::Immediate)
            .unwrap();
    tx.execute(
        "UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1,fencing_token='replaced'",
        [],
    )
    .unwrap();
    let handle = std::thread::spawn(move || {
        reconcile_received(&worker, &lease.scan_id, lease.attempt_number, &id)
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !WAITING.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    let reached_writer = WAITING.load(Ordering::SeqCst);
    tx.commit().unwrap();
    let before = receipt_database_snapshot(&connection);
    let error = handle.join().unwrap().unwrap_err();
    assert!(reached_writer);
    assert_eq!(error, "proposal_reconciliation_scope_unavailable");
    assert_eq!(before, receipt_database_snapshot(&connection));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn directive_reconciliation_requires_committed_result_ack_not_just_update_success() {
    use crate::agent_runtime::multi_agent::directive::reconciliation::reconcile_received;
    let (root, context, id, lease, _original_parent) = saved_receipt_fixture(true);
    let connection = db::open(&context.db_path).unwrap();
    connection.execute_batch("CREATE TRIGGER ignore_receipt_ack BEFORE UPDATE OF acknowledged_at ON agent_messages WHEN NEW.kind='human_assessment_result' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    let before = receipt_database_snapshot(&connection);
    let result = reconcile_received(&connection, &lease.scan_id, lease.attempt_number, &id);
    assert!(
        result.is_err(),
        "a silently skipped acknowledgement is not a completion receipt"
    );
    assert_eq!(before, receipt_database_snapshot(&connection));
    let _ = fs::remove_dir_all(root);
}
