// Exercise the actual specialist HTTP transport, durable receipt, pause and
// local mailbox consumer together. This is intentionally not a comparison of
// two calls to the redactor: the original bug surfaced after receipt storage.
#[test]
fn specialist_redaction_receipt_live_delivery_does_not_fall_into_recovery() {
    use std::sync::{atomic::Ordering, Arc};
    let (port, seen, stop) = spawn_endpoint(Arc::new(|_| {
        (
            200,
            "application/json",
            proposal_model_response(
                r#"{"summary":"login evidence","authToken":"synthetic-test-credential","session":"anonymous","cookie":"<redacted:auth:012345678901>"}"#,
            ),
        )
    }));
    let (root, mut context, lease, child) = specialist_journal_fixture();
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let (text, usage) = multi_agent_child_round_transport(
        &context,
        &lease,
        &child,
        "readonly",
        serde_json::json!({"evidence":"fixed"}),
    )
    .unwrap();
    let connection = db::open(&context.db_path).unwrap();
    let payload = serde_json::json!({"summary":text});
    assert_eq!(
        deliver_readonly_assessment(&connection, &lease, &child, &usage, &payload, None).unwrap(),
        payload
    );
    let code: String = connection
        .query_row(
            "SELECT terminal_code FROM agent_runs WHERE id=?1",
            [&child.run_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        code, "child_completed",
        "normal delivery must not require cleanup/recovery"
    );
    let state: (i64, i64, i64) = connection.query_row(
        "SELECT spent_tokens,spent_requests,(SELECT count(*) FROM agent_messages WHERE acknowledged_at<>'' AND delivery_attempts=1) FROM agent_budget_ledger",
        [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).unwrap();
    assert_eq!(state, (usage.total_tokens, 1, 1));
    assert_eq!(seen.lock().unwrap().len(), 1);
    stop.store(true, Ordering::Relaxed);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn specialist_redaction_receipt_delivers_after_pause_and_database_reopen_once() {
    use std::sync::{atomic::Ordering, Arc};
    for raw in [
        r#"{"summary":"login evidence","authToken":"synthetic-test-credential"}"#,
        r#"{"summary":"login evidence","session":"anonymous"}"#,
        r#"{"summary":"login evidence","cookie":"<redacted:auth:012345678901>"}"#,
    ] {
        let response = raw.to_string();
        let (port, seen, stop) = spawn_endpoint(Arc::new(move |_| {
            (200, "application/json", proposal_model_response(&response))
        }));
        let (root, mut context, lease, child) = specialist_journal_fixture();
        context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
        let (text, usage) = multi_agent_child_round_transport(
            &context,
            &lease,
            &child,
            "readonly",
            serde_json::json!({"evidence":"fixed"}),
        )
        .unwrap();
        let payload = serde_json::json!({"summary":text});
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert!(!text.contains("synthetic-test-credential"));
        let connection = db::open(&context.db_path).unwrap();
        stop_failed_child_preserving_usage(
            &connection,
            &lease,
            &child,
            "local delivery interrupted",
        )
        .unwrap();
        assert_specialist_usage_pending(&connection, "spa_api_mapper", 8000);
        drop(connection);

        // Recovery is solely from persisted state; no second model round or
        // capability reactivation is needed after the connection is reopened.
        let connection = db::open(&context.db_path).unwrap();
        let before = super::tests::application_table_snapshot(&connection);
        let changed = serde_json::json!({"summary":format!("{text} changed evidence")});
        assert!(
            complete_readonly_assessment(&connection, &lease, &child, &usage, &changed)
                .unwrap_err()
                .contains("receipt_mismatch")
        );
        let mut wrong_usage = usage;
        wrong_usage.total_tokens += 1;
        assert!(
            complete_readonly_assessment(&connection, &lease, &child, &wrong_usage, &payload)
                .unwrap_err()
                .contains("receipt_mismatch")
        );
        let mut other_scan = lease.clone();
        other_scan.scan_id = format!("{}-other", lease.scan_id);
        assert!(
            complete_readonly_assessment(&connection, &other_scan, &child, &usage, &payload)
                .is_err()
        );
        let mut other_child = child.clone();
        other_child.run_id = format!("{}-other", child.run_id);
        assert!(
            complete_readonly_assessment(&connection, &lease, &other_child, &usage, &payload)
                .is_err()
        );
        assert_eq!(
            super::tests::application_table_snapshot(&connection),
            before
        );

        assert_eq!(
            complete_readonly_assessment(&connection, &lease, &child, &usage, &payload).unwrap(),
            payload
        );
        let delivered = super::tests::application_table_snapshot(&connection);
        for _ in 0..2 {
            assert_eq!(
                complete_readonly_assessment(&connection, &lease, &child, &usage, &payload)
                    .unwrap(),
                payload
            );
            assert_eq!(
                super::tests::application_table_snapshot(&connection),
                delivered
            );
        }
        let result: (i64, i64, i64, i64, i64) = connection.query_row(
            "SELECT spent_tokens,spent_requests,\
             (SELECT count(*) FROM agent_messages WHERE acknowledged_at<>'' AND delivery_attempts=1),\
             (SELECT count(*) FROM agent_lane_leases),\
             (SELECT count(*) FROM agent_capability_leases WHERE revoked_at='') FROM agent_budget_ledger",
            [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)),
        ).unwrap();
        assert_eq!(result, (usage.total_tokens, 1, 1, 0, 0));
        let saved: String = connection.query_row(
            "SELECT payload_json FROM agent_messages WHERE from_run_id=?1 AND kind='evidence_summary'",
            [&child.run_id], |row| row.get(0),
        ).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&saved).unwrap(),
            payload
        );
        assert_eq!(
            seen.lock().unwrap().len(),
            1,
            "recovery must not reissue the paid request"
        );
        stop.store(true, Ordering::Relaxed);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}
