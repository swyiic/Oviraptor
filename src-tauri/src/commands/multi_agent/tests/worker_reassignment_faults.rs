#[test]
fn assignment_attempt_reassignment_partial_and_collateral_writes_roll_back() {
    use crate::agent_runtime::multi_agent::scheduler;
    let mut faults = Vec::new();
    for table in [
        "agent_assignment_attempts",
        "agent_runs",
        "agent_capability_leases",
        "agent_assignment_replacements",
        "agent_budget_entries",
        "agent_collaboration_events",
    ] {
        for action in ["IGNORE", "ABORT,'replacement-fault'"] {
            faults.push(format!(
                "BEFORE INSERT ON {table} BEGIN SELECT RAISE({action}); END;"
            ));
        }
    }
    for action in ["IGNORE", "ABORT,'replacement-fault'"] {
        faults.push(format!(
            "BEFORE UPDATE OF child_run_id ON agent_assignments BEGIN SELECT RAISE({action}); END;"
        ));
    }
    for change in [
        "UPDATE agent_budget_ledger SET reserved_tokens=reserved_tokens+1",
        "UPDATE agent_runs SET cancel_requested_at='fault' WHERE role='coordinator'",
        "UPDATE agent_runs SET plan_json='{\"fault\":true}' WHERE id=(SELECT child_run_id FROM agent_assignment_attempts WHERE id=NEW.replacement_attempt_id)",
        "UPDATE agent_coordinator_leases SET lease_expires_at='2099-01-01'",
        "UPDATE agent_assignment_attempts SET expires_at='2099-01-01' WHERE id=NEW.replacement_attempt_id",
        "UPDATE agent_assignment_attempts SET heartbeat_at='1999-01-01' WHERE id=NEW.original_attempt_id",
        "DELETE FROM agent_lane_leases",
        "UPDATE agent_contract_owners SET state='released'",
    ] {
        faults.push(format!("AFTER INSERT ON agent_assignment_replacements BEGIN {change}; END;"));
    }
    for fault in faults {
        let (root, context, lease, child) = specialist_journal_fixture();
        let db = db::open(&context.db_path).unwrap();
        expire_worker_deadline(&db, &child.run_id);
        stop_failed_child_preserving_usage(&db, &lease, &child, "expired").unwrap();
        db.execute_batch(&format!("CREATE TRIGGER replacement_fault {fault}"))
            .unwrap();
        let snapshot = super::tests::application_table_snapshot(&db);
        assert!(
            scheduler::reassign_undispatched_expired(&db, &lease, &child).is_err(),
            "{fault}"
        );
        assert!(
            super::tests::application_table_snapshot(&db) == snapshot,
            "rollback: {fault}"
        );
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn assignment_attempt_reassignment_concurrent_callers_share_one_replacement() {
    use crate::agent_runtime::multi_agent::scheduler;
    let (root, context, lease, child) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    expire_worker_deadline(&db, &child.run_id);
    stop_failed_child_preserving_usage(&db, &lease, &child, "expired").unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let threads = (0..2)
        .map(|_| {
            let path = context.db_path.clone();
            let lease = lease.clone();
            let child = child.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let connection = db::open(&path).unwrap();
                barrier.wait();
                scheduler::reassign_undispatched_expired(&connection, &lease, &child)
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let children = threads
        .into_iter()
        .map(|t| t.join().unwrap().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(children[0], children[1]);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_assignment_attempts WHERE assignment_id=?1",
            [&child.assignment_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_assignment_replacements WHERE assignment_id=?1",
            [&child.assignment_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_reassignment_provenance_is_immutable_and_old_corruption_blocks_lookup() {
    use crate::agent_runtime::multi_agent::scheduler;
    let (root, context, lease, child) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    expire_worker_deadline(&db, &child.run_id);
    let next = scheduler::reassign_undispatched_expired(&db, &lease, &child).unwrap();
    for sql in [
        "UPDATE agent_assignment_replacements SET original_hash='changed'",
        "DELETE FROM agent_assignment_replacements",
        "INSERT OR REPLACE INTO agent_assignment_replacements SELECT * FROM agent_assignment_replacements",
    ] {
        let stable=super::tests::application_table_snapshot(&db);
        assert!(db.execute_batch(sql).is_err());
        assert!(super::tests::application_table_snapshot(&db)==stable);
    }
    db.execute(
        "UPDATE agent_assignment_attempts SET heartbeat_at='1999-01-01' WHERE child_run_id=?1",
        [&child.run_id],
    )
    .unwrap();
    let stable = super::tests::application_table_snapshot(&db);
    assert!(scheduler::reassign_undispatched_expired(&db, &lease, &child).is_err());
    assert!(scheduler::mark_child_running(&db, &lease, &next).is_err());
    assert!(super::tests::application_table_snapshot(&db) == stable);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_reassignment_successor_cannot_adopt_original_generation() {
    use crate::agent_runtime::multi_agent::scheduler;
    let (root, context, old, child) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    expire_worker_deadline(&db, &child.run_id);
    let successor = replace_target_cost_coordinator(&db, &old, false);
    stop_failed_child_preserving_usage(&db, &successor, &child, "expired").unwrap();
    let stable = super::tests::application_table_snapshot(&db);
    assert!(scheduler::reassign_undispatched_expired(&db, &successor, &child).is_err());
    assert!(scheduler::reassign_undispatched_expired(&db, &old, &child).is_err());
    assert!(super::tests::application_table_snapshot(&db) == stable);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_reassignment_supervised_bootstrap_requires_frozen_reservation() {
    use crate::agent_runtime::multi_agent::scheduler;
    let (root, context, lease, child) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    let stable = super::tests::application_table_snapshot(&db);
    for tokens in [7999, 8001] {
        assert!(
            scheduler::prepare_supervised_readonly_child(
                &db,
                &lease,
                child.role,
                "journal-test",
                &json!({"fixture":true}),
                tokens
            )
            .is_err(),
            "live recovery must compare the whole frozen contract"
        );
        assert!(super::tests::application_table_snapshot(&db) == stable);
    }
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
