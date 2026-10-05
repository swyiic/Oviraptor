#[test]
fn assignment_attempt_parent_governance_natural_root_deadline_stops_without_callback_or_refund() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{budget, scheduler, specialist, supervisor::WorkerSupervisor},
    };
    let (root, path, run, lease) = multi_agent_test_root("parent-hard-clock", 60_000, 20);
    let db = db::open(&path).unwrap();
    // Freeze a valid short Root contract before the first child initializes its
    // limits/origin. Do not rewrite an existing origin, limit or prior worker.
    db.execute(
        "UPDATE agent_runs SET plan_json=json_set(plan_json,'$.timeoutSeconds',3) WHERE id=?1",
        [&run],
    )
    .unwrap();
    let child = scheduler::schedule_child(
        &db,
        &lease,
        AgentRole::SpaApiMapper,
        AgentLane::ReadOnlyAnalysis,
        "root-hard-clock",
        &json!({"fixture":true}),
        1,
        &["evidence.read".into(), "mailbox.write".into()],
        8_000,
        1,
    )
    .unwrap();
    scheduler::start_child_or_release(&db, &lease, &child).unwrap();
    assert!(matches!(
        specialist::start(&db, &lease, &child, &json!({"messages":[]})).unwrap(),
        specialist::Start::Dispatch(_)
    ));
    let guard = WorkerSupervisor::start(&path, &lease).unwrap();
    let ticket = guard.ticket();
    let before = super::tests::application_table_snapshot(&db);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while budget::clock::remaining(&db, &run).is_ok() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    let settle = std::time::Instant::now() + Duration::from_millis(500);
    while ticket.check().is_ok() && std::time::Instant::now() < settle {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        ticket.check().is_err(),
        "an idle parent must not stay healthy after the original Root hard deadline"
    );
    assert_eq!(guard.check().unwrap_err(), "budget_wall_time_exhausted");
    assert!(super::tests::application_table_snapshot(&db) == before);
    assert_eq!(
        budget::balance(&db, &run, Some(&child.assignment_id), "concurrency_batches")
            .unwrap()
            .reserved,
        1
    );
    assert_eq!(
        budget::balance(&db, &run, Some(&child.assignment_id), "model_requests")
            .unwrap()
            .reserved,
        1
    );
    assert!(scheduler::schedule_child(
        &db,
        &lease,
        AgentRole::SpaApiMapper,
        AgentLane::ReadOnlyAnalysis,
        "after-root-hard-clock",
        &json!({}),
        1,
        &["evidence.read".into()],
        1_000,
        1
    )
    .is_err());
    assert!(super::tests::application_table_snapshot(&db) == before);
    drop(guard);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_parent_governance_renewed_parent_can_grant_without_mutating_original_worker()
{
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{attempts, scheduler, supervisor::WorkerSupervisor},
    };
    let (root, context, lease, old) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    let original = expired_saved_worker_row(&db, &old.run_id);
    db.execute("UPDATE agent_coordinator_leases SET lease_expires_at=datetime('now','+15 seconds','localtime') WHERE root_run_id=?1", [&lease.root_run_id]).unwrap();
    let service = WorkerSupervisor::start(&context.db_path, &lease).unwrap();
    let next = scheduler::schedule_child(
        &db,
        &lease,
        AgentRole::EvidenceReviewer,
        AgentLane::Review,
        "after-parent-heartbeat",
        &json!({}),
        1,
        &["evidence.read".into(), "review.write".into()],
        1000,
        1,
    )
    .unwrap();
    scheduler::start_child_or_release(&db, &lease, &next).unwrap();
    let worker = attempts::current(&db, &lease, &next.assignment_id).unwrap();
    let expiry: String = db
        .query_row(
            "SELECT lease_expires_at FROM agent_coordinator_leases WHERE root_run_id=?1",
            [&lease.root_run_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(worker.expires_at, expiry);
    assert_eq!(expired_saved_worker_row(&db, &old.run_id), original);
    let ticket = service.ticket();
    ticket.check_actor(&context.db_path, &lease).unwrap();
    drop(service);
    assert!(ticket.check().is_err());
    db.execute("UPDATE agent_coordinator_leases SET lease_expires_at=datetime('now','+15 seconds','localtime') WHERE root_run_id=?1", [&lease.root_run_id]).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    std::thread::sleep(Duration::from_millis(350));
    assert!(super::tests::application_table_snapshot(&db) == before);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_parent_governance_renews_only_live_original_coordinator_within_root_deadline()
{
    use crate::agent_runtime::multi_agent::supervisor::WorkerSupervisor;
    let (root, context, lease, _) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    db.execute("UPDATE agent_coordinator_leases SET lease_expires_at=datetime('now','+15 seconds','localtime') WHERE root_run_id=?1", [&lease.root_run_id]).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let old: String = db
        .query_row(
            "SELECT lease_expires_at FROM agent_coordinator_leases WHERE root_run_id=?1",
            [&lease.root_run_id],
            |r| r.get(0),
        )
        .unwrap();
    let guard = WorkerSupervisor::start(&context.db_path, &lease).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_millis(700);
    let renewed = loop {
        guard.check().unwrap();
        let actual: String = db
            .query_row(
                "SELECT lease_expires_at FROM agent_coordinator_leases WHERE root_run_id=?1",
                [&lease.root_run_id],
                |r| r.get(0),
            )
            .unwrap();
        if actual > old {
            break actual;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "a live parent must renew its own near-expiry lease without a worker callback"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    let bounded: bool = db.query_row("SELECT c.lease_expires_at<=datetime(o.started_at,printf('+%f seconds',l.hard_limit/1000.0))
        AND c.lease_epoch=?2 AND c.fencing_token=?3 FROM agent_coordinator_leases c
        JOIN agent_budget_clock_origins o ON o.root_run_id=c.root_run_id JOIN agent_budget_limits l ON l.root_run_id=c.root_run_id
        AND l.dimension='wall_time_ms' WHERE c.root_run_id=?1", params![lease.root_run_id,lease.lease_epoch,lease.fencing_token], |r|r.get(0)).unwrap();
    assert!(bounded, "{renewed}");
    let after = super::tests::application_table_snapshot(&db);
    for (table, rows) in before
        .iter()
        .filter(|(name, _)| name != "agent_coordinator_leases")
    {
        assert_eq!(
            after
                .iter()
                .find(|(name, _)| name == table)
                .map(|(_, rows)| rows),
            Some(rows),
            "{table}"
        );
    }
    drop(guard);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_parent_governance_pristine_clock_is_readonly_and_never_repairs_history() {
    use crate::agent_runtime::multi_agent::{budget, supervisor::WorkerSupervisor};
    for fault in [
        "pristine",
        "past",
        "future",
        "bad-plan",
        "partial-limits",
        "usage",
        "coarse-spend",
    ] {
        let (root, path, run, lease) =
            multi_agent_test_root(&format!("parent-pristine-{fault}"), 60_000, 20);
        let db = db::open(&path).unwrap();
        match fault {
            "past" => {
                db.execute(
                    "UPDATE agent_runs SET created_at='2000-01-01' WHERE id=?1",
                    [&run],
                )
                .unwrap();
            }
            "future" => {
                db.execute("UPDATE agent_runs SET created_at=datetime('now','+1 day','localtime') WHERE id=?1", [&run]).unwrap();
            }
            "bad-plan" => {
                db.execute("UPDATE agent_runs SET plan_json='{}' WHERE id=?1", [&run])
                    .unwrap();
            }
            "partial-limits" => {
                db.execute("INSERT INTO agent_budget_limits(root_run_id,dimension,hard_limit) VALUES(?1,'wall_time_ms',600000)", [&run]).unwrap();
            }
            "usage" => {
                db.execute("UPDATE agent_runs SET used_requests=1 WHERE id=?1", [&run])
                    .unwrap();
            }
            "coarse-spend" => {
                db.execute("INSERT INTO agent_budget_ledger(root_run_id,total_tokens,total_requests,spent_requests,lease_epoch,fencing_token) VALUES(?1,60000,20,1,?2,?3)", params![run,lease.lease_epoch,lease.fencing_token]).unwrap();
            }
            _ => {}
        }
        let before = super::tests::application_table_snapshot(&db);
        let service = WorkerSupervisor::start(&path, &lease);
        if fault == "pristine" {
            let service = service.unwrap();
            service.check().unwrap();
            assert!(
                budget::clock::supervision_remaining(&db, &run)
                    .unwrap()
                    .as_secs()
                    > 1
            );
            drop(service);
        } else {
            assert!(service.is_err(), "{fault}");
        }
        assert!(
            super::tests::application_table_snapshot(&db) == before,
            "{fault}"
        );
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn assignment_attempt_parent_governance_invalid_frozen_origin_is_rejected_without_writes() {
    use crate::agent_runtime::multi_agent::supervisor::WorkerSupervisor;
    let (root, context, lease, _) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    db.execute(
        "UPDATE agent_runs SET created_at=datetime('now','+1 day','localtime') WHERE id=?1",
        [&lease.root_run_id],
    )
    .unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert!(WorkerSupervisor::start(&context.db_path, &lease).is_err());
    assert!(super::tests::application_table_snapshot(&db) == before);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
