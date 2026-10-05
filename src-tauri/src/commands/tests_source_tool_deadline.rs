#[test]
fn source_born_deadline_preserves_paid_pending_finish_and_original_child_without_publication() {
    let (root, db, actor, result) =
        source_born_deadline_checkpoint_fixture(3, 4, "first_tool_finish_received");
    assert!(result.is_err());
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &actor.root_run_id,
            None,
            "model_requests"
        )
        .unwrap()
        .consumed,
        4
    );
    assert_eq!(db.query_row("SELECT count(*) FROM agent_source_model_rounds WHERE root_run_id=?1 AND state='received'",
        [&actor.root_run_id], |r| r.get::<_, i64>(0)).unwrap(), 2);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_source_tool_receipts WHERE tool_name='assignment.finish' AND state='planned'",
        [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_messages WHERE root_run_id=?1 AND kind='source_tool_result'",
        [&actor.root_run_id], |r| r.get::<_, i64>(0)).unwrap(), 0);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_born_deadline_completed_finish_keeps_unpublished_mailbox_and_paid_original_receipt() {
    let (root, db, actor, result) =
        source_born_deadline_checkpoint_fixture(3, 4, "first_tool_finish_settlement_failed");
    assert!(result.is_err());
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &actor.root_run_id,
            None,
            "model_requests"
        )
        .unwrap()
        .consumed,
        4
    );
    assert_eq!(db.query_row("SELECT count(*) FROM agent_source_tool_receipts WHERE tool_name='assignment.finish' AND state='completed'",
        [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_messages WHERE root_run_id=?1 AND kind='source_tool_result'",
        [&actor.root_run_id], |r| r.get::<_, i64>(0)).unwrap(), 0);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_born_deadline_nonterminal_tool_cannot_resume_or_request_another_round() {
    let (root, db, actor, result) =
        source_born_deadline_checkpoint_fixture(3, 3, "first_tool_received");
    assert!(result.is_err());
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &actor.root_run_id,
            None,
            "model_requests"
        )
        .unwrap()
        .consumed,
        3
    );
    let (rounds, deliveries): (i64, i64) = db
        .query_row(
            "SELECT
        (SELECT count(*) FROM agent_source_model_rounds WHERE root_run_id=?1),
        (SELECT count(*) FROM agent_messages WHERE root_run_id=?1 AND kind='source_tool_result')",
            [&actor.root_run_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((rounds, deliveries), (1, 0));
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_saved_finish_rejects_corrupt_authority_and_receipts_without_writes() {
    for mutation in [
        "UPDATE agent_budget_ledger SET reserved_tokens=reserved_tokens+1",
        "UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00'",
        "UPDATE agent_capability_leases SET revoked_at='2000-01-01 00:00:00' WHERE revoked_at=''",
        "DROP TRIGGER agent_source_round_immutable; UPDATE agent_source_model_rounds SET response_hash='corrupt' WHERE round_number=2",
    ] {
        let (root, connection, _lease, result) = source_reviewer_execution_fixture_using_calls(
            "valid", None, 3, None, 4, |root, connection, record, lease| {
                source_reviewer_checkpoint_fixture(root, connection, record, lease, "first_tool_finish_received");
                connection.execute(
                    "UPDATE agent_runs SET started_at='2000-01-01 00:00:00' WHERE id=?1",
                    [&lease.root_run_id],
                ).unwrap();
                connection.execute_batch(mutation).unwrap();
                let before = source_exit_snapshot(connection);
                let result = run_native_source_assessments(
                    &root.join("oviraptor.sqlite3"), &record.scan_id, 1, &root.join("attempt-0001"),
                );
                source_exit_assert_snapshot(connection, lease, &before);
                result
            },
        );
        assert!(result.is_err(), "{mutation}");
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}
