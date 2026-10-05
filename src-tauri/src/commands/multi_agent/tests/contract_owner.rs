#[test]
fn scheduled_child_holds_exactly_one_contract_row_and_reschedule_is_idempotent() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };
    let (root, db_path, root_run_id, lease) = multi_agent_test_root("contract-wire", 60_000, 20);
    let connection = crate::db::open(&db_path).unwrap();
    let child = scheduler::schedule_child(
        &connection,
        &lease,
        AgentRole::SpaApiMapper,
        AgentLane::ReadOnlyAnalysis,
        "contract-wire",
        &serde_json::json!({}),
        1,
        &["evidence.read".into()],
        8_000,
        1,
    )
    .unwrap();
    let row: (String, i64, String, String) = connection
        .query_row(
            "SELECT assignment_id,lease_epoch,fencing_token,state FROM agent_contract_owners WHERE root_run_id=?1",
            [&root_run_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(row.0, child.assignment_id);
    assert_eq!(
        (row.1, row.2.as_str(), row.3.as_str()),
        (lease.lease_epoch, lease.fencing_token.as_str(), "held")
    );
    // Same inputs replay the same owner instead of minting a second row.
    scheduler::schedule_child(
        &connection,
        &lease,
        AgentRole::SpaApiMapper,
        AgentLane::ReadOnlyAnalysis,
        "contract-wire",
        &serde_json::json!({}),
        1,
        &["evidence.read".into()],
        8_000,
        1,
    )
    .unwrap();
    let rows: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM agent_contract_owners WHERE root_run_id=?1",
            [&root_run_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rows, 1);
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn rotated_contract_fencing_blocks_a_replayed_schedule() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{contract_owner, scheduler},
    };
    let (root, db_path, root_run_id, lease) = multi_agent_test_root("contract-fence", 60_000, 20);
    let connection = crate::db::open(&db_path).unwrap();
    let child = scheduler::schedule_child(
        &connection,
        &lease,
        AgentRole::SpaApiMapper,
        AgentLane::ReadOnlyAnalysis,
        "contract-fence",
        &serde_json::json!({}),
        1,
        &["evidence.read".into()],
        8_000,
        1,
    )
    .unwrap();
    // The owner rotates fencing forward out-of-band; the original lease struct
    // is now stale at the contract step even though the coordinator lease row
    // itself is untouched.
    let key = contract_owner::schedule_contract_key(
        lease.attempt_number,
        &lease.target_key,
        AgentRole::SpaApiMapper.as_str(),
        "contract-fence",
        1,
    );
    contract_owner::acquire_contract_owner(
        &connection,
        &root_run_id,
        &key,
        &child.assignment_id,
        lease.lease_epoch + 1,
        "rotated-fence",
    )
    .unwrap();
    let error = scheduler::schedule_child(
        &connection,
        &lease,
        AgentRole::SpaApiMapper,
        AgentLane::ReadOnlyAnalysis,
        "contract-fence",
        &serde_json::json!({}),
        1,
        &["evidence.read".into()],
        8_000,
        1,
    )
    .expect_err("stale fencing replay must fail closed");
    assert_eq!(error, "stale_contract_fencing");
    // The failed replay rolls back lane, budget and run writes with it.
    let lanes: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM agent_lane_leases WHERE assignment_id=?1",
            [&child.assignment_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(lanes, 1, "only the first schedule may hold the lane");
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}
