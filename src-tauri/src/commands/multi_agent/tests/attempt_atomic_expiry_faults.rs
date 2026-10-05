#[test]
fn assignment_attempt_atomic_expiry_rejects_non_native_or_single_root_without_writes() {
    for change in ["backend='retired_fixture'", "orchestration_policy='single'"] {
        let (root, context, lease, child) = specialist_journal_fixture();
        let db = db::open(&context.db_path).unwrap();
        expire_worker_deadline(&db, &child.run_id);
        db.execute(
            &format!("UPDATE agent_runs SET {change} WHERE id=?1"),
            [&lease.root_run_id],
        )
        .unwrap();
        let before = super::tests::application_table_snapshot(&db);
        assert!(
            stop_failed_child_preserving_usage(&db, &lease, &child, "expired").is_err(),
            "{change}"
        );
        assert_eq!(super::tests::application_table_snapshot(&db), before);
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn assignment_attempt_atomic_expiry_partial_writes_and_collateral_changes_roll_back() {
    let mut faults = Vec::new();
    for table in [
        "agent_assignment_attempts",
        "agent_assignments",
        "agent_runs",
        "agent_capability_leases",
    ] {
        for action in ["ABORT,'expiry-fault'", "IGNORE"] {
            faults.push(format!(
                "BEFORE UPDATE ON {table} BEGIN SELECT RAISE({action}); END;"
            ));
        }
    }
    faults.push(
        "BEFORE INSERT ON agent_collaboration_events BEGIN SELECT RAISE(IGNORE); END;".into(),
    );
    for change in [
        "UPDATE agent_assignment_attempts SET heartbeat_at='changed' WHERE id=NEW.id",
        "UPDATE agent_assignment_attempts SET expires_at='1999-01-01' WHERE id=NEW.id",
        "UPDATE agent_runs SET cancel_requested_at='changed' WHERE role='coordinator'",
        "UPDATE agent_coordinator_leases SET lease_expires_at='2099-01-01'",
        "UPDATE agent_budget_ledger SET reserved_requests=reserved_requests+1",
        "DELETE FROM agent_lane_leases",
        "DELETE FROM agent_specialist_calls",
    ] {
        faults.push(format!("AFTER UPDATE OF state ON agent_assignment_attempts WHEN NEW.state='expired' BEGIN {change}; END;"));
    }
    for fault in faults {
        use crate::agent_runtime::multi_agent::specialist;
        let (root, context, lease, child) = specialist_journal_fixture();
        let db = db::open(&context.db_path).unwrap();
        let specialist::Start::Dispatch(_) =
            specialist::start(&db, &lease, &child, &json!({"messages":[]})).unwrap()
        else {
            panic!()
        };
        expire_worker_deadline(&db, &child.run_id);
        db.execute_batch(&format!("CREATE TRIGGER atomic_expiry_fault {fault}"))
            .unwrap();
        let before = super::tests::application_table_snapshot(&db);
        assert!(
            stop_failed_child_preserving_usage(&db, &lease, &child, "expired").is_err(),
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
fn assignment_attempt_atomic_expiry_same_root_successor_withdraws_without_adopting_cost() {
    use crate::agent_runtime::multi_agent::{attempts, specialist};
    for different_root in [false, true] {
        let (root, context, old, child) = specialist_journal_fixture();
        let db = db::open(&context.db_path).unwrap();
        let specialist::Start::Dispatch(call) =
            specialist::start(&db, &old, &child, &json!({"messages":[]})).unwrap()
        else {
            panic!()
        };
        expire_worker_deadline(&db, &child.run_id);
        let successor = replace_target_cost_coordinator(&db, &old, different_root);
        let before = super::tests::application_table_snapshot(&db);
        assert!(stop_failed_child_preserving_usage(&db, &old, &child, "expired").is_err());
        assert_eq!(super::tests::application_table_snapshot(&db), before);
        let result = stop_failed_child_preserving_usage(&db, &successor, &child, "expired");
        if different_root {
            assert!(result.is_err());
            assert_eq!(super::tests::application_table_snapshot(&db), before);
        } else {
            result.unwrap();
            let worker = attempts::current(&db, &old, &child.assignment_id).unwrap();
            assert_eq!(worker.state, "expired");
            assert_eq!(worker.coordinator_epoch, old.lease_epoch);
            assert_eq!(worker.coordinator_fencing_token, old.fencing_token);
            assert_expiry_preserved_resources(&db, &before);
            let closed = super::tests::application_table_snapshot(&db);
            stop_failed_child_preserving_usage(&db, &successor, &child, "repeat").unwrap();
            assert_eq!(super::tests::application_table_snapshot(&db), closed);
            let original = expired_saved_worker_row(&db, &child.run_id);
            assert!(specialist::record_received(
                &db,
                &call,
                "late original",
                false,
                &late_web_model_response().usage
            )
            .is_err());
            let epoch: i64 = db
                .query_row(
                    "SELECT lease_epoch FROM agent_model_cost_facts WHERE child_run_id=?1",
                    [&child.run_id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(epoch, old.lease_epoch);
            assert_eq!(expired_saved_worker_row(&db, &child.run_id), original);
        }
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn assignment_attempt_atomic_expiry_non_dispatch_owner_cannot_withdraw_original_worker() {
    let (root, context, lease, child) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    expire_worker_deadline(&db, &child.run_id);
    let before = super::tests::application_table_snapshot(&db);
    let reason = "specialist_call_outcome_unknown_requires_reconciliation";
    assert_eq!(failed_specialist_error(&db, &lease, &child, reason), reason);
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    stop_failed_child_preserving_usage(&db, &lease, &child, "owned timeout").unwrap();
    let closed = super::tests::application_table_snapshot(&db);
    stop_failed_child_preserving_usage(&db, &lease, &child, "repeat").unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), closed);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_atomic_expiry_repeat_rejects_corrupted_logical_closure() {
    for fault in [
        "UPDATE agent_assignments SET finished_at='2000-01-01'",
        "UPDATE agent_runs SET finished_at='2000-01-01' WHERE role='spa_api_mapper'",
        "UPDATE agent_runs SET terminal_state='completed' WHERE role='spa_api_mapper'",
        "DELETE FROM agent_lane_leases",
    ] {
        let (root, context, lease, child) = specialist_journal_fixture();
        let db = db::open(&context.db_path).unwrap();
        expire_worker_deadline(&db, &child.run_id);
        stop_failed_child_preserving_usage(&db, &lease, &child, "expired").unwrap();
        db.execute_batch(fault).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        assert!(
            stop_failed_child_preserving_usage(&db, &lease, &child, "repeat").is_err(),
            "{fault}"
        );
        assert_eq!(super::tests::application_table_snapshot(&db), before);
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn assignment_attempt_atomic_expiry_cannot_change_candidate_projection() {
    let f = review_receipt_fixture();
    let db = db::open(&f.context.db_path).unwrap();
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM agent_finding_candidates WHERE root_run_id=?1",
            [&f.lease.root_run_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        count > 0,
        "fixture must contain a real candidate projection"
    );
    expire_worker_deadline(&db, &f.child.run_id);
    db.execute_batch(
        "CREATE TRIGGER expiry_candidate_fault AFTER UPDATE OF state ON agent_assignment_attempts
        WHEN NEW.state='expired' BEGIN UPDATE agent_finding_candidates SET title='collateral change'
        WHERE root_run_id=NEW.root_run_id; END;",
    )
    .unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert!(stop_failed_child_preserving_usage(&db, &f.lease, &f.child, "expired").is_err());
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    drop(db);
    fs::remove_dir_all(f.root).unwrap();
}
