#[test]
fn source_born_deadline_retains_saved_initial_receipt_without_new_sdk_or_publication() {
    let (root, db, actor, result) =
        source_born_deadline_checkpoint_fixture(3, 1, "first_assessment_received");
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
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_specialist_calls WHERE root_run_id=?1 AND state='received'",
            [&actor.root_run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_messages WHERE root_run_id=?1",
            [&actor.root_run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(db.query_row("SELECT count(*) FROM agent_assignments WHERE coordinator_run_id=?1 AND budget_settled_at<>''",
        [&actor.root_run_id], |r| r.get::<_, i64>(0)).unwrap(), 0);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_saved_candidate_review_reentry_ignores_mutable_started_at_without_new_model_work() {
    let (root, connection, lease, result) = source_reviewer_execution_fixture_using_calls(
        "confirmed",
        None,
        3,
        None,
        7,
        |root, connection, record, lease| {
            source_reviewer_checkpoint_fixture(root, connection, record, lease, "received");
            connection
                .execute(
                    "UPDATE agent_runs SET started_at='2000-01-01 00:00:00' WHERE id=?1",
                    [&lease.root_run_id],
                )
                .unwrap();
            assert!(
                crate::agent_runtime::multi_agent::budget::clock::remaining(
                    connection,
                    &lease.root_run_id
                )
                .is_ok(),
                "mutable started_at never changes original deadline"
            );
            run_native_source_assessments(
                &root.join("oviraptor.sqlite3"),
                &record.scan_id,
                1,
                &root.join("attempt-0001"),
            )
        },
    );
    let result = result.expect("delivered candidate review requires only local closure");
    assert_eq!(result["modelRequests"], 7);
    let delivered: i64 = connection.query_row(
        "SELECT count(*) FROM agent_messages WHERE root_run_id=?1 AND kind='source_review_result' AND acknowledged_at<>''",
        [&lease.root_run_id], |row| row.get(0),
    ).unwrap();
    let (spent, reserved): (i64, i64) = connection
        .query_row(
            "SELECT spent_requests,reserved_requests FROM agent_budget_ledger WHERE root_run_id=?1",
            [&lease.root_run_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((delivered, spent, reserved), (1, 7, 0));
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_saved_coverage_review_reentry_ignores_mutable_started_at_without_new_model_work() {
    let (root, connection, lease, result) = source_reviewer_execution_fixture_using_calls(
        "valid",
        None,
        4,
        None,
        8,
        |root, connection, record, lease| {
            source_coverage_checkpoint_fixture(root, connection, record, lease, "received");
            connection
                .execute(
                    "UPDATE agent_runs SET started_at='2000-01-01 00:00:00' WHERE id=?1",
                    [&lease.root_run_id],
                )
                .unwrap();
            assert!(
                crate::agent_runtime::multi_agent::budget::clock::remaining(
                    connection,
                    &lease.root_run_id
                )
                .is_ok(),
                "mutable started_at never changes original deadline"
            );
            run_native_source_assessments(
                &root.join("oviraptor.sqlite3"),
                &record.scan_id,
                1,
                &root.join("attempt-0001"),
            )
        },
    );
    let result = result.expect("delivered coverage review requires only local closure");
    assert_eq!(result["modelRequests"], 8);
    let delivered: i64 = connection.query_row(
        "SELECT count(*) FROM agent_messages WHERE root_run_id=?1 AND kind='source_coverage_review_result' AND acknowledged_at<>''",
        [&lease.root_run_id], |row| row.get(0),
    ).unwrap();
    let (spent, reserved): (i64, i64) = connection
        .query_row(
            "SELECT spent_requests,reserved_requests FROM agent_budget_ledger WHERE root_run_id=?1",
            [&lease.root_run_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((delivered, spent, reserved), (1, 8, 0));
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_saved_reviewer_cannot_settle_mismatched_accounting() {
    for (schema, calls, checkpoint) in [(3, 7, "received"), (4, 8, "coverage_received")] {
        let (root, connection, _lease, result) = source_reviewer_execution_fixture_using_calls(
            "valid",
            None,
            schema,
            None,
            calls,
            |root, connection, record, lease| {
                if checkpoint == "coverage_received" {
                    source_coverage_checkpoint_fixture(root, connection, record, lease, "received");
                } else {
                    source_reviewer_checkpoint_fixture(root, connection, record, lease, checkpoint);
                }
                connection
                    .execute(
                        "UPDATE agent_runs SET started_at='2000-01-01 00:00:00' WHERE id=?1",
                        [&lease.root_run_id],
                    )
                    .unwrap();
                connection.execute(
                    "UPDATE agent_budget_ledger SET spent_requests=spent_requests-1 WHERE root_run_id=?1",
                    [&lease.root_run_id],
                ).unwrap();
                let before = source_exit_snapshot(connection);
                let result = run_native_source_assessments(
                    &root.join("oviraptor.sqlite3"),
                    &record.scan_id,
                    1,
                    &root.join("attempt-0001"),
                );
                source_exit_assert_snapshot(connection, lease, &before);
                result
            },
        );
        assert!(result
            .unwrap_err()
            .contains("source_review_pending_ledger_or_extra_work_mismatch"));
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_born_deadline_never_dispatches_unstarted_independent_reviews() {
    for (schema, calls, checkpoint) in [(3, 6, "before_review"), (4, 7, "before_coverage")] {
        let (root, db, actor, result) =
            source_born_deadline_checkpoint_fixture(schema, calls, checkpoint);
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
            calls as i64
        );
        let forbidden = if schema == 3 {
            "source_candidates_ready"
        } else {
            "source_coverage_ready"
        };
        assert_eq!(db.query_row("SELECT count(*) FROM agent_assignments WHERE coordinator_run_id=?1 AND trigger_code=?2",
            params![actor.root_run_id, forbidden], |r| r.get::<_, i64>(0)).unwrap(), 0);
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}
