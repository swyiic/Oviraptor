#[test]
fn assignment_attempt_expired_saved_invalid_owner_or_receipt_never_publishes() {
    for fault in [
        "UPDATE agent_assignment_attempts SET expires_at='2099-01-01'",
        "UPDATE agent_assignment_attempts SET expires_at='malformed'",
        "UPDATE agent_assignment_attempts SET finished_at=''",
        "UPDATE agent_assignment_attempts SET finished_at='2099-01-01'",
        "UPDATE agent_assignment_attempts SET failure_class='child_execution_failed'",
        "UPDATE agent_assignment_attempts SET worker_id='not-a-worker'",
        "UPDATE agent_specialist_calls SET state='executing'",
        "UPDATE agent_specialist_calls SET response_hash='damaged'",
        "DELETE FROM agent_specialist_calls",
        "DELETE FROM agent_snapshots",
        "UPDATE agent_assignments SET failure_class='unrelated_pause'",
        "UPDATE agent_runs SET cancel_requested_at='cancelled' WHERE role='spa_api_mapper'",
        "UPDATE agent_capability_leases SET revoked_at=''",
        "UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01'",
        "UPDATE agent_runs SET cancel_requested_at='cancelled' WHERE role='coordinator'",
    ] {
        let (root, context, lease, child) = specialist_journal_fixture();
        let db = db::open(&context.db_path).unwrap();
        let usage = attempt_saved_call(&db, &lease, &child, &json!({}), "saved mapper summary");
        stop_failed_child_preserving_usage(&db, &lease, &child, "interrupted").unwrap();
        restore_expired_saved_worker(&db, &child);
        let intact = super::tests::application_table_snapshot(&db);
        if fault.starts_with("UPDATE agent_specialist_calls") {
            assert!(db
                .execute_batch(fault)
                .unwrap_err()
                .to_string()
                .contains("specialist_call_immutable"));
            assert_eq!(super::tests::application_table_snapshot(&db), intact);
            // A corrupt restored fixture is confined to this temporary DB.
            db.execute_batch("DROP TRIGGER agent_specialist_call_immutable")
                .unwrap();
        }
        let injected = db.execute_batch(fault);
        if fault.contains("worker_id=") {
            assert!(injected
                .unwrap_err()
                .to_string()
                .contains("assignment_attempt_identity_immutable"));
            assert_eq!(super::tests::application_table_snapshot(&db), intact);
            drop(db);
            fs::remove_dir_all(root).unwrap();
            continue;
        }
        injected.unwrap();
        let before = super::tests::application_table_snapshot(&db);
        assert!(
            complete_readonly_assessment(
                &db,
                &lease,
                &child,
                &usage,
                &json!({"summary":"saved mapper summary"})
            )
            .is_err(),
            "{fault}"
        );
        assert_eq!(
            super::tests::application_table_snapshot(&db),
            before,
            "{fault}"
        );
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn assignment_attempt_expired_saved_final_publication_keeps_original_proof() {
    for role in ["mapper", "reviewer", "investigator"] {
        for mutation in [
            "heartbeat_at='2000-01-01'",
            "failure_class='changed'",
            "expires_at='1999-01-01'",
        ] {
            let (root, db, lease, child, usage, review, gap) = match role {
                "reviewer" => {
                    let f = review_receipt_fixture();
                    let db = db::open(&f.context.db_path).unwrap();
                    let usage = attempt_saved_call(
                        &db,
                        &f.lease,
                        &f.child,
                        &f.candidate,
                        &review_receipt_response(),
                    );
                    finish_failed_review(&db, &f.lease, &f.child, "review-receipt", "interrupted")
                        .unwrap();
                    (
                        f.root.clone(),
                        db,
                        f.lease.clone(),
                        f.child.clone(),
                        usage,
                        Some(f),
                        None,
                    )
                }
                "investigator" => {
                    let f = gap_receipt_fixture();
                    let db = db::open(&f.context.db_path).unwrap();
                    let usage = attempt_saved_call(
                        &db,
                        &f.session.lease,
                        &f.child,
                        &f.input,
                        &gap_receipt_response(),
                    );
                    stop_failed_child_preserving_usage(
                        &db,
                        &f.session.lease,
                        &f.child,
                        "interrupted",
                    )
                    .unwrap();
                    (
                        f.root.clone(),
                        db,
                        f.session.lease.clone(),
                        f.child.clone(),
                        usage,
                        None,
                        Some(f),
                    )
                }
                _ => {
                    let (root, context, lease, child) = specialist_journal_fixture();
                    let db = db::open(&context.db_path).unwrap();
                    let usage =
                        attempt_saved_call(&db, &lease, &child, &json!({}), "saved mapper summary");
                    stop_failed_child_preserving_usage(&db, &lease, &child, "interrupted").unwrap();
                    (root, db, lease, child, usage, None, None)
                }
            };
            restore_expired_saved_worker(&db, &child);
            let trigger = match role {
                "reviewer" => "AFTER INSERT ON sentinel_findings",
                "investigator" => "AFTER UPDATE OF acknowledged_at ON agent_messages WHEN NEW.kind='proposal_assessed'",
                _ => "AFTER UPDATE OF acknowledged_at ON agent_messages WHEN NEW.kind='evidence_summary'",
            };
            db.execute_batch(&format!(
                "CREATE TRIGGER expired_publication_fault {trigger} BEGIN
                UPDATE agent_assignment_attempts SET {mutation} WHERE child_run_id='{}'; END;",
                child.run_id
            ))
            .unwrap();
            let before = super::tests::application_table_snapshot(&db);
            let result = if let Some(f) = review {
                recover_received_review(
                    &db,
                    &lease,
                    "receipt",
                    1,
                    &f.candidate.to_string(),
                    &f.context.target_dir,
                )
                .map(|_| ())
            } else if let Some(f) = gap {
                recover_gap_fixture(&f, &db)
            } else {
                complete_readonly_assessment(
                    &db,
                    &lease,
                    &child,
                    &usage,
                    &json!({"summary":"saved mapper summary"}),
                )
                .map(|_| ())
            };
            assert!(result.is_err(), "{role}: {mutation}");
            assert_eq!(
                super::tests::application_table_snapshot(&db),
                before,
                "{role}: {mutation}"
            );
            drop(db);
            fs::remove_dir_all(root).unwrap();
        }
    }
}

#[test]
fn assignment_attempt_expired_saved_replay_rejects_changed_expiration_audit() {
    let f = gap_receipt_fixture();
    let db = db::open(&f.context.db_path).unwrap();
    attempt_saved_call(
        &db,
        &f.session.lease,
        &f.child,
        &f.input,
        &gap_receipt_response(),
    );
    stop_failed_child_preserving_usage(&db, &f.session.lease, &f.child, "interrupted").unwrap();
    restore_expired_saved_worker(&db, &f.child);
    recover_gap_fixture(&f, &db).unwrap();
    db.execute(
        "UPDATE agent_assignment_attempts SET expires_at='2099-01-01' WHERE child_run_id=?1",
        [&f.child.run_id],
    )
    .unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert!(recover_gap_fixture(&f, &db).is_err());
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    drop(db);
    fs::remove_dir_all(f.root).unwrap();
}
