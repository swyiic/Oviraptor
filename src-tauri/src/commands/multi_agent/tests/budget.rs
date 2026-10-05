#[test]
fn multi_agent_budget_settlement_is_idempotent_and_conservative() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };

    let (root, db_path, root_run_id, lease) = multi_agent_test_root("budget", 100, 3);
    let connection = db::open(&db_path).unwrap();
    let child = scheduler::schedule_child(
        &connection,
        &lease,
        AgentRole::SpaApiMapper,
        AgentLane::ReadOnlyAnalysis,
        "budget-test",
        &serde_json::json!({"task":"budget"}),
        1,
        &["evidence.read".into()],
        60,
        2,
    )
    .unwrap();
    scheduler::mark_child_running(&connection, &lease, &child).unwrap();
    let usage = AgentTokenUsage {
        input_tokens: 35,
        cached_input_tokens: 5,
        output_tokens: 15,
        total_tokens: 50,
        model_requests: 1,
    };
    settle_child_usage(&connection, &lease, &child, &usage).unwrap();
    settle_child_usage(&connection, &lease, &child, &usage).unwrap();
    assert_eq!(
        settle_child_usage(
            &connection,
            &lease,
            &child,
            &AgentTokenUsage {
                total_tokens: 51,
                ..usage
            }
        )
        .unwrap_err(),
        "child_budget_settlement_replay_conflict"
    );
    let ledger: (i64, i64, i64, i64, i64, i64) = connection
        .query_row(
            "SELECT total_tokens,reserved_tokens,spent_tokens,total_requests,reserved_requests,spent_requests FROM agent_budget_ledger WHERE root_run_id=?1",
            [&root_run_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        )
        .unwrap();
    assert_eq!(ledger, (100, 0, 50, 3, 0, 1));
    let assignment: (i64, i64, String) = connection
        .query_row(
            "SELECT reserved_tokens,reserved_requests,budget_settled_at FROM agent_assignments WHERE id=?1",
            [&child.assignment_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!((assignment.0, assignment.1), (0, 0));
    assert!(!assignment.2.is_empty());
    scheduler::finish_child(&connection, &lease, &child, true, "settled").unwrap();

    let zero_request_child = scheduler::schedule_child(
        &connection,
        &lease,
        AgentRole::SpaApiMapper,
        AgentLane::ReadOnlyAnalysis,
        "zero-request-test",
        &serde_json::json!({"task":"zero-request"}),
        2,
        &["evidence.read".into()],
        20,
        1,
    )
    .unwrap();
    scheduler::mark_child_running(&connection, &lease, &zero_request_child).unwrap();
    settle_child_usage(
        &connection,
        &lease,
        &zero_request_child,
        &AgentTokenUsage {
            total_tokens: 10,
            model_requests: 0,
            ..AgentTokenUsage::default()
        },
    )
    .unwrap();
    let conserved: (i64, i64, i64, i64, i64, i64) = connection
        .query_row(
            "SELECT total_tokens,reserved_tokens,spent_tokens,total_requests,reserved_requests,spent_requests FROM agent_budget_ledger WHERE root_run_id=?1",
            [&root_run_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        )
        .unwrap();
    assert!(conserved.1 + conserved.2 <= conserved.0);
    assert!(conserved.4 + conserved.5 <= conserved.3);
    assert_eq!((conserved.1, conserved.2, conserved.4, conserved.5), (0, 60, 0, 1));
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn lane_capacity_is_atomic_across_connections_and_released_only_at_terminal() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };
    for (label, role, lane, capabilities) in [
        ("read", AgentRole::SpaApiMapper, AgentLane::ReadOnlyAnalysis, vec!["evidence.read".into()]),
        ("review", AgentRole::EvidenceReviewer, AgentLane::Review, vec!["review.write".into()]),
        ("target", AgentRole::WebExecutor, AgentLane::TargetTouching, vec!["replay_http".into()]),
    ] {
        let (root, path, _, lease) = multi_agent_test_root(label, 100, 10);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let attempts = std::thread::scope(|scope| {
            let handles = (0..2).map(|index| {
                let path = path.clone();
                let lease = lease.clone();
                let barrier = barrier.clone();
                let capabilities = capabilities.clone();
                scope.spawn(move || {
                    let connection = db::open(&path).unwrap();
                    barrier.wait();
                    scheduler::schedule_child(
                        &connection, &lease, role, lane, &format!("racer-{index}"),
                        &serde_json::json!({"racer":index}), index + 1, &capabilities, 10, 1,
                    )
                })
            }).collect::<Vec<_>>();
            handles.into_iter().map(|handle| handle.join().unwrap()).collect::<Vec<_>>()
        });
        assert_eq!(attempts.iter().filter(|attempt| attempt.is_ok()).count(), 1, "{label}: {attempts:?}");
        let child = attempts.into_iter().find_map(Result::ok).unwrap();
        let connection = db::open(&path).unwrap();
        let occupied: i64 = connection.query_row(
            "SELECT COUNT(*) FROM agent_lane_leases WHERE scan_id=?1 AND attempt_number=?2 AND target_key=?3 AND lane=?4",
            rusqlite::params![lease.scan_id, lease.attempt_number, lease.target_key, lane.as_str()],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(occupied, 1);
        scheduler::mark_child_running(&connection, &lease, &child).unwrap();
        scheduler::finish_child(&connection, &lease, &child, true, "done").unwrap();
        assert_eq!(connection.query_row("SELECT COUNT(*) FROM agent_lane_leases", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
        let next = scheduler::schedule_child(&connection, &lease, role, lane, "after-release", &serde_json::json!({"next":true}), 3, &capabilities, 10, 1);
        if role == AgentRole::WebExecutor {
            // A free lane does not authorize a second target executor in the
            // same attempt when the first one's effects may be unknown.
            assert_eq!(next.unwrap_err(), "target_execution_recovery_requires_fresh_attempt");
        } else {
            next.unwrap();
        }
        drop(connection);
        std::fs::remove_dir_all(root).unwrap();
    }
}

