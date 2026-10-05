fn schedule_authority_fixture(
    db: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<crate::agent_runtime::multi_agent::scheduler::ScheduledChild, String> {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };
    scheduler::schedule_child(
        db,
        lease,
        AgentRole::SpaApiMapper,
        AgentLane::ReadOnlyAnalysis,
        "authority",
        &serde_json::json!({"task":"inventory"}),
        1,
        &["evidence.read".into(), "mailbox.write".into()],
        100,
        1,
    )
}

#[test]
fn scheduler_authority_valid_replay_is_read_only_and_keeps_running_child() {
    use crate::agent_runtime::multi_agent::scheduler;
    let (root, path, _, lease) = multi_agent_test_root("scheduler-authority-read", 1000, 10);
    let db = db::open(&path).unwrap();
    let child = schedule_authority_fixture(&db, &lease).unwrap();
    scheduler::mark_child_running(&db, &lease, &child).unwrap();
    db.execute_batch(
        "CREATE TRIGGER no_replay_assignment BEFORE UPDATE ON agent_assignments
        BEGIN SELECT RAISE(ABORT,'replay must not update assignment'); END;
        CREATE TRIGGER no_replay_budget BEFORE UPDATE ON agent_budget_ledger
        BEGIN SELECT RAISE(ABORT,'replay must not update budget'); END;",
    )
    .unwrap();
    let before = receipt_database_snapshot(&db);
    assert_eq!(schedule_authority_fixture(&db, &lease).unwrap(), child);
    assert_eq!(receipt_database_snapshot(&db), before);
    drop(db);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn scheduler_authority_replay_never_repairs_missing_expired_or_changed_authority() {
    for mutation in [
        "UPDATE agent_assignments SET fencing_token='other-worker'",
        "UPDATE agent_assignments SET lease_expires_at='2000-01-01 00:00:00'",
        "DELETE FROM agent_runs WHERE role='spa_api_mapper'",
        "UPDATE agent_runs SET cancel_requested_at='cancelled' WHERE role='spa_api_mapper'",
        "UPDATE agent_capability_leases SET revoked_at='revoked'",
        "DELETE FROM agent_lane_leases",
        "UPDATE agent_contract_owners SET state='released'",
        "UPDATE agent_budget_ledger SET fencing_token='other-worker'",
    ] {
        let (root, path, _, lease) = multi_agent_test_root("scheduler-authority-replay", 1000, 10);
        let db = db::open(&path).unwrap();
        schedule_authority_fixture(&db, &lease).unwrap();
        db.execute_batch(mutation).unwrap();
        let before = receipt_database_snapshot(&db);
        assert!(
            schedule_authority_fixture(&db, &lease).is_err(),
            "must refuse {mutation}"
        );
        assert_eq!(
            receipt_database_snapshot(&db),
            before,
            "must preserve {mutation}"
        );
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn scheduler_authority_fresh_partial_grants_roll_back_all_resources() {
    for trigger in [
        "CREATE TRIGGER partial_grant BEFORE INSERT ON agent_capability_leases
         WHEN NEW.capability='mailbox.write' BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER partial_grant AFTER INSERT ON agent_capability_leases
         WHEN NEW.capability='mailbox.write' BEGIN UPDATE agent_lane_leases SET lane='review'; END;",
        "CREATE TRIGGER partial_grant AFTER INSERT ON agent_capability_leases
         WHEN NEW.capability='mailbox.write' BEGIN UPDATE agent_runs SET cancel_requested_at='cancelled'
         WHERE id=NEW.root_run_id; END;",
        "CREATE TRIGGER partial_grant AFTER INSERT ON agent_capability_leases
         WHEN NEW.capability='mailbox.write' BEGIN UPDATE agent_assignments SET fencing_token='other-worker'; END;",
        "CREATE TRIGGER partial_grant AFTER INSERT ON agent_capability_leases
         WHEN NEW.capability='mailbox.write' BEGIN UPDATE agent_runs SET lease_expires_at='2099-01-01 00:00:00'
         WHERE id=NEW.child_run_id; END;",
        "CREATE TRIGGER partial_grant BEFORE UPDATE ON agent_assignments
         WHEN NEW.state='leased' BEGIN SELECT RAISE(IGNORE); END;",
    ] {
        let (root, path, _, lease) = multi_agent_test_root("scheduler-authority-partial", 1000, 10);
        let db = db::open(&path).unwrap();
        db.execute_batch(trigger).unwrap();
        let before = receipt_database_snapshot(&db);
        assert!(schedule_authority_fixture(&db, &lease).is_err(), "must refuse {trigger}");
        assert_eq!(receipt_database_snapshot(&db), before, "must roll back {trigger}");
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn scheduler_authority_same_root_new_epoch_cannot_adopt_old_child() {
    use crate::agent_runtime::multi_agent::lease;
    let (root, path, _, old) = multi_agent_test_root("scheduler-authority-epoch", 1000, 10);
    let db = db::open(&path).unwrap();
    schedule_authority_fixture(&db, &old).unwrap();
    db.execute_batch("UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00'")
        .unwrap();
    let new = lease::acquire_coordinator_lease(
        &db,
        &old.scan_id,
        old.attempt_number,
        &old.target_key,
        &old.root_run_id,
        600,
    )
    .unwrap();
    assert!(new.lease_epoch > old.lease_epoch);
    let before = receipt_database_snapshot(&db);
    assert!(schedule_authority_fixture(&db, &new).is_err());
    assert_eq!(receipt_database_snapshot(&db), before);
    drop(db);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn scheduler_authority_fresh_child_uses_current_lease_deadline_without_renewal() {
    let (root, path, _, lease) = multi_agent_test_root("scheduler-authority-deadline", 1000, 10);
    let db = db::open(&path).unwrap();
    // A heartbeat can extend the same owner while callers retain the earlier
    // lease handle. New grants must read the committed deadline, not renew it.
    db.execute_batch("UPDATE agent_coordinator_leases SET lease_expires_at='2099-01-01 00:00:00'")
        .unwrap();
    let child = schedule_authority_fixture(&db, &lease).unwrap();
    let uniform: bool = db
        .query_row(
            "SELECT a.lease_expires_at=r.lease_expires_at
        AND a.lease_expires_at=c.lease_expires_at AND a.lease_expires_at='2099-01-01 00:00:00'
        FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
        JOIN agent_coordinator_leases c ON c.root_run_id=a.coordinator_run_id WHERE a.id=?1",
            [&child.assignment_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(uniform);
    drop(db);
    std::fs::remove_dir_all(root).unwrap();
}
