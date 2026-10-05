#[test]
fn coordinator_takeover_fences_every_old_writer() {
    use crate::agent_runtime::{
        contract::{AgentBackendKind, AgentLane, AgentRole, AgentRunStatus, MultiAgentPolicy},
        multi_agent::{lease, scheduler},
        store::{self, AgentRunRow},
    };

    let (root, db_path, first_root, old_lease) = multi_agent_test_root("takeover", 100, 4);
    let connection = db::open(&db_path).unwrap();
    let second_root = "root-takeover-second".to_string();
    let mut second_run = AgentRunRow::new(
        &second_root,
        "scan-takeover",
        1,
        "https://authorized.example.test",
        AgentBackendKind::Native,
        AgentRole::Coordinator,
        "plan-2",
        "evidence-2",
    )
    .with_budget(50, 100, 2, 4);
    second_run.status = AgentRunStatus::Running;
    second_run.root_run_id = second_root.clone();
    second_run.orchestration_policy = MultiAgentPolicy::Multi;
    second_run.lane = Some(AgentLane::ReadOnlyAnalysis);
    store::create_run(&connection, &second_run).unwrap();
    // Each newly created Coordinator needs its own frozen Native plan. This
    // fixture adopts the same target contract, not the previous run's authority.
    connection.execute("UPDATE agent_runs SET plan_json=(SELECT plan_json FROM agent_runs WHERE id=?1) WHERE id=?2",
        rusqlite::params![first_root,second_root]).unwrap();
    assert!(lease::acquire_coordinator_lease(
        &connection,
        "scan-takeover",
        1,
        "https://authorized.example.test",
        &second_root,
        600,
    )
    .unwrap_err()
    .starts_with("coordinator_lease_held:"));

    let old_child = scheduler::schedule_child(
        &connection,
        &old_lease,
        AgentRole::SpaApiMapper,
        AgentLane::ReadOnlyAnalysis,
        "stale-child",
        &serde_json::json!({}),
        1,
        &["evidence.read".into()],
        10,
        1,
    )
    .unwrap();
    scheduler::mark_child_running(&connection, &old_lease, &old_child).unwrap();
    connection
        .execute(
            "UPDATE agent_coordinator_leases SET lease_expires_at=datetime('now','-1 second','localtime') WHERE root_run_id=?1",
            [&first_root],
        )
        .unwrap();
    let new_lease = lease::acquire_coordinator_lease(
        &connection,
        "scan-takeover",
        1,
        "https://authorized.example.test",
        &second_root,
        600,
    )
    .unwrap();
    assert_eq!(new_lease.lease_epoch, old_lease.lease_epoch + 1);
    assert_ne!(new_lease.fencing_token, old_lease.fencing_token);
    let stale = "stale_coordinator_fencing_token";
    assert_eq!(
        finish_coordinator_run(
            &connection,
            &old_lease,
            &AgentTargetOutcome::failed("stale")
        )
        .unwrap_err(),
        stale
    );
    assert_eq!(
        settle_child_usage(
            &connection,
            &old_lease,
            &old_child,
            &AgentTokenUsage::default()
        )
        .unwrap_err(),
        stale
    );
    assert_eq!(
        settle_agent_finding_candidates(&connection, &old_lease, &old_child.run_id, 1, "confirmed", &root)
            .unwrap_err(),
        stale
    );
    assert_eq!(
        finish_coordinator_run(&connection, &old_lease, &AgentTargetOutcome::Cancelled).unwrap_err(),
        stale
    );
    assert_eq!(
        persist_review_decision(
            &connection,
            &old_lease,
            &old_child,
            "stale-request",
            "stale-candidate",
            1,
            "rejected",
            &serde_json::json!([]),
            &serde_json::json!([]),
            0.0,
            "stale",
            &root,
        )
        .unwrap_err(),
        stale
    );

    let new_child = scheduler::schedule_child(
        &connection,
        &new_lease,
        AgentRole::EvidenceReviewer,
        AgentLane::Review,
        "new-review",
        &serde_json::json!({}),
        1,
        &["evidence.read".into(), "review.write".into()],
        10,
        1,
    )
    .unwrap();
    scheduler::mark_child_running(&connection, &new_lease, &new_child).unwrap();
    settle_child_usage(
        &connection,
        &new_lease,
        &new_child,
        &AgentTokenUsage::default(),
    )
    .unwrap();
    open_review_request(
        &connection,
        &new_lease,
        &new_child,
        "new-request",
        "new-candidate",
        1,
        "{}",
        &root,
    )
    .unwrap();
    persist_review_decision(
        &connection,
        &new_lease,
        &new_child,
        "new-request",
        "new-candidate",
        1,
        "rejected",
        &serde_json::json!(["not_confirmed"]),
        &serde_json::json!([]),
        0.9,
        "new owner decision",
        &root,
    )
    .unwrap();
    scheduler::finish_child(
        &connection,
        &new_lease,
        &new_child,
        false,
        "review rejected",
    )
    .unwrap();
    finish_coordinator_run(
        &connection,
        &new_lease,
        &AgentTargetOutcome::incomplete("new owner completed review"),
    )
    .unwrap();
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}
