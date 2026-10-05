// Actual financial finisher calls only; no target/model/browser transport.
fn exceptional_original_rows(db: &rusqlite::Connection) -> Vec<(String, Vec<String>)> {
    super::tests::application_table_snapshot(db)
        .into_iter()
        .filter(|(table, _)| table != "agent_root_elapsed_facts")
        .collect()
}

fn exceptional_short_root() -> (
    std::path::PathBuf,
    rusqlite::Connection,
    String,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) {
    let (directory, path, root, lease) = multi_agent_test_root("elapsed-real-expiry", 1000, 10);
    let db = db::open(&path).unwrap();
    let mut plan = test_plan_for("standard", &lease.target_key).with_attempt(lease.attempt_number);
    plan.timeout_seconds = 1;
    db.execute(
        "UPDATE agent_runs SET plan_hash=?2,plan_json=?3,
        created_at=strftime('%Y-%m-%d %H:%M:%f','now','localtime') WHERE id=?1",
        params![root, plan.hash(), plan.as_json().to_string()],
    )
    .unwrap();
    let tx = db.unchecked_transaction().unwrap();
    crate::agent_runtime::multi_agent::budget::root::RootOwner::initialize_financial_fixture_for_test(&tx, &root)
        .unwrap();
    tx.commit().unwrap();
    // Actual natural expiration of the frozen 1000 ms origin/hard contract.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    (directory, db, root, lease)
}

#[test]
fn exceptional_elapsed_actual_overhard_financial_hook_saves_full_fact_without_clamping_or_new_fees()
{
    use crate::agent_runtime::multi_agent::budget::{self, clock::elapsed_fact};
    let (directory, db, root, lease) = exceptional_short_root();
    let old = exceptional_original_rows(&db);
    let physical = final_elapsed_physical_rows(&db);
    // Once Stage B implements terminal consumption, this producer contract
    // still isolates the sole legal financial-fact difference in all rows.
    elapsed_fact::record_if_exceptional(&db, &lease)
        .unwrap()
        .unwrap();
    let fact = elapsed_fact::read(&db, &root)
        .unwrap()
        .expect("actual finalizer must not lose over-hard elapsed");
    assert!(fact.elapsed >= 1050, "{fact:?}");
    assert_eq!(fact.hard, 1000);
    assert!(fact.exhausted);
    assert_eq!(fact.unsettled, fact.elapsed - fact.journaled);
    assert_eq!(fact.journaled, 0);
    assert_eq!(fact.owner_kind, "root_control");
    assert_eq!(fact.epoch, lease.lease_epoch);
    assert_eq!(fact.fence, lease.fencing_token);
    assert_eq!(exceptional_original_rows(&db), old);
    assert_eq!(final_elapsed_physical_rows(&db), physical);
    assert_eq!(
        budget::balance(&db, &root, None, "wall_time_ms")
            .unwrap()
            .consumed,
        0,
        "do not insert a clipped or over-hard consume into the original ledger"
    );
    assert!(budget::admission::require_determinate(&db, &root).is_err());
    let saved = super::tests::application_table_snapshot(&db);
    std::thread::sleep(std::time::Duration::from_millis(35));
    elapsed_fact::record_if_exceptional(&db, &lease)
        .unwrap()
        .unwrap();
    assert!(
        super::tests::application_table_snapshot(&db) == saved,
        "financial replay uses its original cutoff"
    );
    drop(db);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn exceptional_elapsed_actual_expired_original_c_saves_only_original_financial_fact() {
    use crate::agent_runtime::multi_agent::budget::clock::elapsed_fact;
    let (directory, db, root, lease) = final_elapsed_fixture("elapsed-expired-original-c");
    db.execute("UPDATE agent_coordinator_leases SET lease_expires_at=datetime('now','-1 day','localtime') WHERE root_run_id=?1",[&root]).unwrap();
    let old = exceptional_original_rows(&db);
    let physical = final_elapsed_physical_rows(&db);
    assert_eq!(
        finish_coordinator_run(&db, &lease, &AgentTargetOutcome::Cancelled).unwrap_err(),
        "stale_coordinator_fencing_token"
    );
    let fact = elapsed_fact::read(&db, &root)
        .unwrap()
        .expect("original elapsed survives C expiration");
    assert!(!fact.exhausted);
    assert!(fact.elapsed >= 2800);
    assert_eq!(fact.epoch, lease.lease_epoch);
    assert_eq!(fact.fence, lease.fencing_token);
    assert_eq!(
        exceptional_original_rows(&db),
        old,
        "no terminal/result/asset/capability or other original row changes"
    );
    assert_eq!(final_elapsed_physical_rows(&db), physical);
    let snapshot = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    assert!(
        crate::agent_runtime::multi_agent::budget::root::RootOwner::initialize_control(&tx, &root)
            .is_err()
    );
    tx.rollback().unwrap();
    assert!(super::tests::application_table_snapshot(&db) == snapshot);
    assert!(
        crate::agent_runtime::multi_agent::budget::admission::require_determinate(&db, &root)
            .is_err(),
        "financial original costs never reopen any new worker/model grant"
    );
    drop(db);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn exceptional_elapsed_actual_original_c_after_replacement_is_financial_only_but_successor_c_cannot_adopt(
) {
    use crate::agent_runtime::multi_agent::budget::clock::elapsed_fact;
    let (directory, db, root, lease) = final_elapsed_fixture("elapsed-financial-after-replacement");
    let mut successor = lease.clone();
    successor.lease_epoch += 1;
    successor.fencing_token = uuid::Uuid::new_v4().to_string();
    db.execute(
        "UPDATE agent_coordinator_leases SET lease_epoch=?2,fencing_token=?3 WHERE root_run_id=?1",
        params![root, successor.lease_epoch, successor.fencing_token],
    )
    .unwrap();
    crate::agent_runtime::multi_agent::lease::validate_coordinator_lease(&db, &successor).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert!(finish_coordinator_run(&db, &successor, &AgentTargetOutcome::Cancelled).is_err());
    assert!(
        super::tests::application_table_snapshot(&db) == before,
        "current live C cannot claim an old financial owner"
    );
    let old = exceptional_original_rows(&db);
    assert_eq!(
        finish_coordinator_run(&db, &lease, &AgentTargetOutcome::Cancelled).unwrap_err(),
        "stale_coordinator_fencing_token"
    );
    let fact = elapsed_fact::read(&db, &root)
        .unwrap()
        .expect("only original owner may preserve its past elapsed");
    assert_eq!(fact.epoch, lease.lease_epoch);
    assert_eq!(fact.fence, lease.fencing_token);
    assert_ne!(fact.fence, successor.fencing_token);
    assert_eq!(exceptional_original_rows(&db), old);
    drop(db);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn exceptional_elapsed_actual_closed_root_replay_records_saved_cutoff_without_post_close_cost() {
    use crate::agent_runtime::multi_agent::budget::{self, clock::elapsed_fact};
    let (directory, db, root, lease) = final_elapsed_fixture("elapsed-closed-original-fact");
    finish_coordinator_run(&db, &lease, &AgentTargetOutcome::Cancelled).unwrap();
    let physical = final_elapsed_physical_rows(&db);
    let finished: String = db
        .query_row(
            "SELECT finished_at FROM agent_runs WHERE id=?1",
            [&root],
            |r| r.get(0),
        )
        .unwrap();
    db.execute("UPDATE agent_coordinator_leases SET lease_expires_at=datetime('now','-1 day','localtime') WHERE root_run_id=?1",[&root]).unwrap();
    let old = exceptional_original_rows(&db); // Original C expiry is an external state change, never repaired.
    std::thread::sleep(std::time::Duration::from_millis(35));
    assert!(finish_coordinator_run(&db, &lease, &AgentTargetOutcome::Cancelled).is_err());
    let fact = elapsed_fact::read(&db, &root).unwrap().unwrap();
    assert_eq!(fact.cutoff, finished);
    assert_eq!(fact.unsettled, 0);
    assert_eq!(
        fact.elapsed,
        budget::balance(&db, &root, None, "wall_time_ms")
            .unwrap()
            .consumed
    );
    assert_eq!(exceptional_original_rows(&db), old);
    assert_eq!(final_elapsed_physical_rows(&db), physical);
    drop(db);
    fs::remove_dir_all(directory).unwrap();
}
