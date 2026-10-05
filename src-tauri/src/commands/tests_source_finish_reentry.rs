#[test]
fn source_completed_finish_reentry_settles_then_continues_without_replaying_tool() {
    let (root, connection, lease, result) = source_reviewer_execution_fixture_using(
        "valid", None, 3, None, |root, connection, record, lease| {
            source_reviewer_checkpoint_fixture(
                root, connection, record, lease, "first_tool_finish_settlement_failed",
            );
            run_native_source_assessments(
                &root.join("oviraptor.sqlite3"), &record.scan_id, 1, &root.join("attempt-0001"),
            )
        },
    );
    let report = result.expect("saved finish must be settled before a new model request");
    assert_eq!(report["modelRequests"], 7);
    let (first_receipts, first_messages): (i64, i64) = connection.query_row(
        "SELECT (SELECT count(*) FROM agent_source_tool_receipts r JOIN agent_assignments a ON a.id=r.assignment_id
                WHERE a.coordinator_run_id=?1 AND a.role='repo_mapper' AND r.tool_name='assignment.finish' AND r.state='completed'),
                (SELECT count(*) FROM agent_messages m JOIN agent_assignments a ON a.id=m.assignment_id
                WHERE a.coordinator_run_id=?1 AND a.role='repo_mapper' AND m.kind='source_tool_result' AND m.acknowledged_at<>'')",
        [&lease.root_run_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!((first_receipts, first_messages), (1, 1));
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_completed_finish_reentry_rejects_corruption_without_writes() {
    for (name, mutation) in [
        ("receipt", "DROP TRIGGER agent_source_tool_receipt_immutable;
            UPDATE agent_source_tool_receipts SET receipt_hash='tampered' WHERE tool_name='assignment.finish'"),
        ("ledger", "UPDATE agent_budget_ledger SET spent_tokens=spent_tokens+1"),
        ("capability", "UPDATE agent_capability_leases SET revoked_at=datetime('now','localtime')
            WHERE assignment_id IN (SELECT id FROM agent_assignments WHERE trigger_code='source_tools_ready' AND state='running')"),
        ("lease", "UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00'"),
    ] {
        let (root, connection, _, result) = source_reviewer_execution_fixture_using_calls(
            "valid", None, 3, None, 4, |root, connection, record, lease| {
                source_reviewer_checkpoint_fixture(
                    root, connection, record, lease, "first_tool_finish_settlement_failed",
                );
                connection.execute_batch(mutation).unwrap();
                let before = source_exit_snapshot(connection);
                let result = run_native_source_assessments(
                    &root.join("oviraptor.sqlite3"), &record.scan_id, 1, &root.join("attempt-0001"),
                );
                assert!(result.is_err(), "{name}");
                source_exit_assert_snapshot(connection, lease, &before);
                result
            },
        );
        assert!(result.is_err(), "{name}");
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}
