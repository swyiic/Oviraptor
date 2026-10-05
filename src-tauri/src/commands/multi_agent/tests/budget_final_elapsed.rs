// Financial finalization only: temporary SQLite, no model/target/browser calls.
fn final_elapsed_fixture(
    tag: &str,
) -> (
    std::path::PathBuf,
    rusqlite::Connection,
    String,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) {
    use crate::agent_runtime::multi_agent::budget::root::RootOwner;
    let (directory, path, root, lease) = multi_agent_test_root(tag, 1000, 10);
    let db = db::open(&path).unwrap();
    // Set the actual original Root time before the first immutable freeze.
    db.execute(
        "UPDATE agent_runs SET created_at=datetime('now','-3 seconds','localtime') WHERE id=?1",
        [&root],
    )
    .unwrap();
    let tx = db.unchecked_transaction().unwrap();
    RootOwner::initialize_financial_fixture_for_test(&tx, &root).unwrap();
    tx.commit().unwrap();
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(&db, &root, None, "wall_time_ms")
            .unwrap(),
        Default::default()
    );
    (directory, db, root, lease)
}

#[test]
fn final_elapsed_actual_finisher_charges_every_outcome_without_a_fake_child() {
    use crate::agent_runtime::multi_agent::budget;
    let outcomes = [
        AgentTargetOutcome::Completed(AgentCompletion::without_ledger(
            "done",
            terminal_code::LEDGER_COMPLETE,
        )),
        AgentTargetOutcome::BoundedCompleted(AgentCompletion::bounded("bounded")),
        AgentTargetOutcome::failed("original failure"),
        AgentTargetOutcome::Cancelled,
        AgentTargetOutcome::incomplete("still incomplete"),
        AgentTargetOutcome::Limited(AgentStop::new(
            terminal_code::HARD_WALL_TIME_BUDGET,
            "bounded other work",
        )),
        AgentTargetOutcome::ResumeIncompatible(AgentStop::new(
            terminal_code::RESUME_INCOMPATIBLE,
            "resume incompatible",
        )),
    ];
    for outcome in outcomes {
        let (directory, db, root, lease) = final_elapsed_fixture("final-elapsed-outcomes");
        let origin: String = db
            .query_row(
                "SELECT started_at FROM agent_budget_clock_origins WHERE root_run_id=?1",
                [&root],
                |r| r.get(0),
            )
            .unwrap();
        let result = finish_coordinator_run(&db, &lease, &outcome);
        assert!(result.is_ok(), "{}: {result:?}", outcome.terminal_code());
        let charge = budget::balance(&db, &root, None, "wall_time_ms").unwrap();
        assert_eq!((charge.reserved, charge.indeterminate), (0, 0));
        assert!(
            charge.consumed >= 2800,
            "{}: {charge:?}",
            outcome.terminal_code()
        );
        let actual: i64 = db.query_row("SELECT CAST((julianday(r.finished_at)-julianday(o.started_at))*86400000 AS INTEGER) FROM agent_runs r JOIN agent_budget_clock_origins o ON o.root_run_id=r.id WHERE r.id=?1", [&root], |r|r.get(0)).unwrap();
        assert_eq!(
            charge.consumed, actual,
            "the financial cutoff must be the actual persisted terminal time"
        );
        assert_eq!(
            db.query_row(
                "SELECT started_at FROM agent_budget_clock_origins WHERE root_run_id=?1",
                [&root],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            origin
        );
        for table in [
            "agent_assignments",
            "agent_assignment_attempts",
            "agent_lane_leases",
            "agent_capability_leases",
        ] {
            assert_eq!(
                db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                0,
                "no fake worker/slot/grant: {table}"
            );
        }
        drop(db);
        fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn final_elapsed_actual_cancelled_replay_preserves_all_saved_rows() {
    let (directory, db, root, lease) = final_elapsed_fixture("final-elapsed-replay");
    finish_coordinator_run(&db, &lease, &AgentTargetOutcome::Cancelled).unwrap();
    let physical = final_elapsed_physical_rows(&db);
    let before = super::tests::application_table_snapshot(&db);
    std::thread::sleep(std::time::Duration::from_millis(35));
    finish_coordinator_run(&db, &lease, &AgentTargetOutcome::Cancelled).unwrap();
    assert!(
        super::tests::application_table_snapshot(&db) == before,
        "closed Root must not charge post-closure time on replay"
    );
    assert_eq!(final_elapsed_physical_rows(&db), physical);
    assert!(
        crate::agent_runtime::multi_agent::budget::root::RootOwner::load_single(&db, &root)
            .is_err(),
        "Multi owner must not become Single"
    );
    drop(db);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn final_elapsed_actual_legacy_child_cancelled_samples_the_remaining_interval() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{budget, scheduler},
    };
    let (directory, path, root, lease) = multi_agent_test_root("final-elapsed-old-child", 1000, 10);
    let db = db::open(&path).unwrap();
    let child = scheduler::schedule_child(
        &db,
        &lease,
        AgentRole::SpaApiMapper,
        AgentLane::ReadOnlyAnalysis,
        "old-clock-owner",
        &json!({}),
        1,
        &["evidence.read".into()],
        10,
        1,
    )
    .unwrap();
    scheduler::finish_child(&db, &lease, &child, false, "never dispatched").unwrap();
    let previous = budget::balance(&db, &root, None, "wall_time_ms")
        .unwrap()
        .consumed;
    let original_worker =
        crate::agent_runtime::multi_agent::attempts::current(&db, &lease, &child.assignment_id)
            .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(35));
    finish_coordinator_run(&db, &lease, &AgentTargetOutcome::Cancelled).unwrap();
    let final_charge = budget::balance(&db, &root, None, "wall_time_ms").unwrap();
    assert!(
        final_charge.consumed >= previous + 20,
        "{previous} -> {final_charge:?}"
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_root_budget_attempts WHERE root_run_id=?1",
            [&root],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0,
        "legacy owner is not upgraded"
    );
    assert_eq!(
        crate::agent_runtime::multi_agent::attempts::current(&db, &lease, &child.assignment_id)
            .unwrap(),
        original_worker
    );
    drop(db);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn final_elapsed_actual_finisher_faults_rollback_entire_business_and_financial_state() {
    for target in ["reserve", "consume", "root-terminal"] {
        for effect in ["IGNORE", "ABORT", "FAIL", "business", "other-root"] {
            let (directory, db, root, lease) = final_elapsed_fixture("final-elapsed-fault");
            // A real second Root row makes collateral effects observable.
            let mut sibling = crate::agent_runtime::store::AgentRunRow::new(
                format!("{root}-sibling"),
                lease.scan_id.clone(),
                1,
                lease.target_key.clone(),
                crate::agent_runtime::contract::AgentBackendKind::Native,
                crate::agent_runtime::contract::AgentRole::Coordinator,
                "sibling-plan",
                "sibling-evidence",
            );
            sibling.root_run_id = sibling.id.clone();
            crate::agent_runtime::store::create_run(&db, &sibling).unwrap();
            let trigger = if target == "root-terminal" {
                format!("BEFORE UPDATE OF status ON agent_runs WHEN NEW.id='{root}' AND NEW.status='terminal'")
            } else {
                format!("BEFORE INSERT ON agent_budget_entries WHEN NEW.dimension='wall_time_ms' AND NEW.kind='{target}'")
            };
            let body=match effect {
                "IGNORE"=>"SELECT RAISE(IGNORE)".to_string(),
                "ABORT"=>"SELECT RAISE(ABORT,'final-elapsed-fault')".to_string(),
                "FAIL"=>"SELECT RAISE(FAIL,'final-elapsed-fault')".to_string(),
                "business"=>"INSERT INTO projects(name) VALUES('unapproved-elapsed-side-effect')".to_string(),
                _=>"UPDATE agent_runs SET terminal_reason='unauthorized-collateral' WHERE id<>NEW.id".to_string(),
            };
            // For entry triggers use a real unrelated business update, not a
            // reference to a nonexistent NEW.id field that always fails SQL.
            let body = if effect == "other-root" && target != "root-terminal" {
                "UPDATE sentinel_scans SET status='completed'".to_string()
            } else {
                body
            };
            db.execute_batch(&format!(
                "CREATE TRIGGER final_elapsed_fault {trigger} BEGIN {body}; END;"
            ))
            .unwrap();
            let physical = final_elapsed_physical_rows(&db);
            let before = super::tests::application_table_snapshot(&db);
            let result = finish_coordinator_run(&db, &lease, &AgentTargetOutcome::Cancelled);
            assert!(result.is_err(), "{target}/{effect}: {result:?}");
            assert!(
                super::tests::application_table_snapshot(&db) == before,
                "{target}/{effect}"
            );
            assert_eq!(
                final_elapsed_physical_rows(&db),
                physical,
                "{target}/{effect}"
            );
            drop(db);
            fs::remove_dir_all(directory).unwrap();
        }
    }
}

#[test]
fn final_elapsed_actual_finisher_rejects_origin_corruption_without_backfill() {
    let (directory, db, root, lease) = final_elapsed_fixture("final-elapsed-origin-drift");
    db.execute(
        "UPDATE agent_runs SET created_at=datetime('now','localtime') WHERE id=?1",
        [&root],
    )
    .unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert!(finish_coordinator_run(&db, &lease, &AgentTargetOutcome::Cancelled).is_err());
    assert!(super::tests::application_table_snapshot(&db) == before);
    drop(db);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn final_elapsed_actual_finisher_never_adopts_successor_coordinator_authority() {
    let (directory, db, root, lease) = final_elapsed_fixture("final-elapsed-stale-c");
    db.execute("UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1,fencing_token=?1 WHERE root_run_id=?2",params![uuid::Uuid::new_v4().to_string(),lease.root_run_id]).unwrap();
    let before = exceptional_original_rows(&db);
    let physical = final_elapsed_physical_rows(&db);
    assert!(finish_coordinator_run(&db, &lease, &AgentTargetOutcome::Cancelled).is_err());
    assert_eq!(exceptional_original_rows(&db), before);
    assert_eq!(final_elapsed_physical_rows(&db), physical);
    let fact = crate::agent_runtime::multi_agent::budget::clock::elapsed_fact::read(&db, &root)
        .unwrap()
        .unwrap();
    assert_eq!(fact.epoch, lease.lease_epoch);
    assert_eq!(fact.fence, lease.fencing_token);
    drop(db);
    fs::remove_dir_all(directory).unwrap();
}

fn final_elapsed_physical_rows(db: &rusqlite::Connection) -> Vec<Vec<String>> {
    [
        "agent_runs",
        "agent_root_budget_attempts",
        "agent_budget_clock_origins",
        "agent_budget_limits",
        "agent_budget_entries",
    ]
    .into_iter()
    .map(|table| {
        let mut q = db
            .prepare(&format!("SELECT rowid,* FROM {table} ORDER BY rowid"))
            .unwrap();
        let count = q.column_count();
        q.query_map([], |r| {
            let values = (0..count)
                .map(|i| r.get::<_, rusqlite::types::Value>(i))
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(format!("{values:?}"))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
    })
    .collect()
}

#[test]
fn final_elapsed_actual_legacy_completed_replay_does_not_bill_post_close() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };
    let (directory, path, _root, lease) =
        multi_agent_test_root("final-elapsed-completed-replay", 1000, 10);
    let db = db::open(&path).unwrap();
    let child = scheduler::schedule_child(
        &db,
        &lease,
        AgentRole::SpaApiMapper,
        AgentLane::ReadOnlyAnalysis,
        "completed-clock-owner",
        &json!({}),
        1,
        &["evidence.read".into()],
        10,
        1,
    )
    .unwrap();
    scheduler::finish_child(&db, &lease, &child, false, "never dispatched").unwrap();
    let outcome = AgentTargetOutcome::Completed(AgentCompletion::without_ledger(
        "done",
        terminal_code::LEDGER_COMPLETE,
    ));
    finish_coordinator_run(&db, &lease, &outcome).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let physical = final_elapsed_physical_rows(&db);
    std::thread::sleep(std::time::Duration::from_millis(35));
    finish_coordinator_run(&db, &lease, &outcome).unwrap();
    assert!(
        super::tests::application_table_snapshot(&db) == before,
        "completed replay used now instead of its saved cutoff"
    );
    assert_eq!(final_elapsed_physical_rows(&db), physical);
    drop(db);
    fs::remove_dir_all(directory).unwrap();
}
