#[test]
fn assignment_attempt_parent_governance_renewal_faults_cannot_modify_money_assets_or_other_parents()
{
    use crate::agent_runtime::{
        contract::{AgentBackendKind, AgentRole},
        multi_agent::{lease as leases, supervisor::WorkerSupervisor},
        store::{self, AgentRunRow},
    };
    for fault in [
        "ignore",
        "abort",
        "budget",
        "assets",
        "new-row",
        "fence",
        "origin",
        "deadline",
        "other-parent",
        "expired",
    ] {
        let (root, context, lease, _) = specialist_journal_fixture();
        let db = db::open(&context.db_path).unwrap();
        db.execute("UPDATE agent_coordinator_leases SET lease_expires_at=datetime('now','+15 seconds','localtime') WHERE root_run_id=?1", [&lease.root_run_id]).unwrap();
        let mut other = AgentRunRow::new(
            "other-parent",
            &lease.scan_id,
            lease.attempt_number,
            "https://other.invalid",
            AgentBackendKind::Native,
            AgentRole::Coordinator,
            "other-plan",
            "other-evidence",
        );
        other.root_run_id = other.id.clone();
        store::create_run(&db, &other).unwrap();
        leases::acquire_coordinator_lease(
            &db,
            &lease.scan_id,
            lease.attempt_number,
            &other.target_url,
            &other.id,
            600,
        )
        .unwrap();
        let body = match fault {
            "ignore" => "SELECT RAISE(IGNORE);",
            "abort" => "SELECT RAISE(ABORT,'parent-renewal-fault');",
            "budget" => "UPDATE agent_budget_ledger SET spent_requests=spent_requests+1;",
            "assets" => "DELETE FROM assets;",
            "new-row" => "INSERT INTO projects(name) VALUES ('unapproved-parent-effect');",
            "fence" => "UPDATE agent_coordinator_leases SET fencing_token='other-fence' WHERE root_run_id=NEW.root_run_id;",
            "origin" => "UPDATE agent_runs SET created_at='2000-01-01' WHERE id=NEW.root_run_id;",
            "deadline" => "UPDATE agent_coordinator_leases SET lease_expires_at='2050-01-01' WHERE root_run_id=NEW.root_run_id;",
            "other-parent" => "UPDATE agent_coordinator_leases SET updated_at='collateral' WHERE root_run_id='other-parent';",
            _ => "SELECT 1;",
        };
        if fault == "expired" {
            db.execute("UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01' WHERE root_run_id=?1", [&lease.root_run_id]).unwrap();
        } else {
            db.execute_batch(&format!("CREATE TRIGGER parent_renewal_fault {} UPDATE ON agent_coordinator_leases WHEN OLD.root_run_id<>'other-parent' BEGIN {body} END;", if matches!(fault,"ignore"|"abort") {"BEFORE"} else {"AFTER"})).unwrap();
        }
        let before = super::tests::application_table_snapshot(&db);
        let service = WorkerSupervisor::start(&context.db_path, &lease);
        assert!(service.is_err(), "{fault}");
        assert!(
            super::tests::application_table_snapshot(&db) == before,
            "{fault}"
        );
        drop(service);
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn assignment_attempt_parent_governance_writer_contention_retries_original_lease_without_adoption()
{
    use crate::agent_runtime::multi_agent::supervisor::WorkerSupervisor;
    let (root, context, lease, _) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    let guard = WorkerSupervisor::start(&context.db_path, &lease).unwrap();
    db.execute("UPDATE agent_coordinator_leases SET lease_expires_at=datetime('now','+15 seconds','localtime') WHERE root_run_id=?1", [&lease.root_run_id]).unwrap();
    let writer = db.unchecked_transaction().unwrap();
    writer
        .execute(
            "UPDATE agent_coordinator_leases SET heartbeat_at=heartbeat_at WHERE root_run_id=?1",
            [&lease.root_run_id],
        )
        .unwrap();
    let original: String = writer
        .query_row(
            "SELECT lease_expires_at FROM agent_coordinator_leases WHERE root_run_id=?1",
            [&lease.root_run_id],
            |r| r.get(0),
        )
        .unwrap();
    let before = super::tests::application_table_snapshot(&writer);
    std::thread::sleep(Duration::from_millis(650));
    guard.check().unwrap();
    assert!(super::tests::application_table_snapshot(&writer) == before);
    writer.commit().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        guard.check().unwrap();
        let changed: bool = db
            .query_row(
                "SELECT lease_expires_at>?2 AND lease_epoch=?3 AND fencing_token=?4
            FROM agent_coordinator_leases WHERE root_run_id=?1",
                params![
                    lease.root_run_id,
                    original,
                    lease.lease_epoch,
                    lease.fencing_token
                ],
                |r| r.get(0),
            )
            .unwrap();
        if changed {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(guard);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_parent_governance_renewal_authorizer_is_cleared_before_worker_withdrawal() {
    use crate::agent_runtime::multi_agent::{attempts, supervisor::WorkerSupervisor};
    let (root, context, lease, child) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    db.execute("UPDATE agent_coordinator_leases SET lease_expires_at=datetime('now','+15 seconds','localtime') WHERE root_run_id=?1", [&lease.root_run_id]).unwrap();
    expire_worker_deadline(&db, &child.run_id);
    let service = WorkerSupervisor::start(&context.db_path, &lease).unwrap();
    service.check().unwrap();
    assert_eq!(
        attempts::current(&db, &lease, &child.assignment_id)
            .unwrap()
            .state,
        "expired"
    );
    drop(service);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
