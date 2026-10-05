fn invocation_test_snapshot(connection: &rusqlite::Connection) -> Vec<String> {
    [
        "agent_runs",
        "agent_assignments",
        "agent_specialist_calls",
        "agent_events",
        "agent_snapshots",
        "agent_messages",
        "agent_user_directives",
        "agent_budget_ledger",
        "agent_capability_leases",
        "agent_coordinator_leases",
        "sentinel_targets",
        "sentinel_scans",
        "sentinel_checkpoints",
    ]
    .iter()
    .map(|table| {
        let mut statement = connection
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
            .unwrap();
        let columns = statement.column_count();
        let rows = statement
            .query_map([], |row| {
                (0..columns)
                    .map(|i| row.get::<_, rusqlite::types::Value>(i))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        format!("{table}:{rows:?}")
    })
    .collect()
}

#[test]
fn native_invocation_outer_observer_cannot_finalize_dispatch_owner() {
    // Actual signed creator, current mode receipt and original financial Root.
    // A hand-seeded scan is no longer positive execution authority.
    let mut harness = fresh_multi_production_harness("outer-invocation-observer");
    let (arrived_tx, arrived_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release_rx = std::sync::Mutex::new(release_rx);
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(move |request| {
        if let Some(summary) = fresh_multi_root_wire_response(&request) {
            return (200, "application/json", summary);
        }
        arrived_tx.send(()).unwrap();
        release_rx
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(30))
            .unwrap();
        (401, "application/json", r#"{"error":{"message":"fixture owner provider failure","type":"authentication_error"}}"#.into())
    }));
    harness.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    fs::write(
        harness.context.target_dir.join("frontend-evidence.json"),
        harness.context.evidence.to_string(),
    )
    .unwrap();
    let adaptive = AgentBudgetSettings::from_json(&json!({}));
    harness.context.execution_plan = build_agent_execution_plan(
        &adaptive, &harness.context.route, &harness.context.environment,
        AgentBackendKind::Native, &harness.db_path, "agent-scan",
    ).with_attempt(1);
    freeze_fresh_multi_production_harness(&mut harness);
    let owner = harness.context.clone();
    let (returned_tx, returned) = mpsc::channel();
    let worker = thread::spawn(move || {
        let _real = RealSpecialistTransport::enter();
        let prepared = PreparedFrontendTarget {
            position: 1,
            route: owner.route.clone(),
            target_dir: owner.target_dir.clone(),
            proxy: None,
            browser: None,
        };
        let result = run_agent_target(
            &prepared,
            AgentTargetExecution {
                db_path: &owner.db_path,
                scan_id: "agent-scan",
                attempt_number: 1,
                settings: &json!({"agentBackendPolicy":"native"}),
                environment: &owner.environment,
                adaptive: &AgentBudgetSettings::from_json(&json!({})),
                log_path: &owner.log_path,
            },
        );
        let _ = returned_tx.send(result.as_ref().map(|owned| format!("{:?}", owned.outcome)).map_err(Clone::clone));
        result
    });
    arrived_rx.recv_timeout(Duration::from_secs(30))
        .unwrap_or_else(|error| panic!("provider arrival: {error}; original executor returned: {:?}", returned.try_recv()));
    let connection = db::open(&harness.db_path).unwrap();
    let before = invocation_test_snapshot(&connection);
    let prepared = PreparedFrontendTarget {
        position: 1,
        route: harness.context.route.clone(),
        target_dir: harness.context.target_dir.clone(),
        proxy: None,
        browser: None,
    };
    // A competing caller even proposes another plan; admission must precede
    // plan freezing, run reuse, SRC receiver setup and all failure finalization.
    let observer = run_agent_target(
        &prepared,
        AgentTargetExecution {
            db_path: &harness.db_path,
            scan_id: "agent-scan",
            attempt_number: 1,
            settings: &json!({"agentBackendPolicy":"native", "agentMaxTurns":1}),
            environment: &harness.context.environment,
            adaptive: &AgentBudgetSettings::from_json(&json!({})),
            log_path: &harness.context.log_path,
        },
    );
    let after = invocation_test_snapshot(&connection);
    release_tx.send(()).unwrap();
    let owned = worker.join().unwrap().unwrap();
    assert!(observer
        .unwrap_err()
        .starts_with("native_invocation_not_owned:"));
    assert_eq!(
        before, after,
        "non-owner must not publish or clean up anything"
    );
    assert_eq!(seen.lock().unwrap().len(), 2);
    assert!(
        matches!(owned.outcome, AgentTargetOutcome::Failed(_)),
        "the owner's actual provider failure is retained"
    );
    // Execution ownership survives returning from run_agent_target, through the
    // caller's projection write; dropping it before projection would race.
    assert!(claim_native_invocation(
        &harness.db_path,
        "agent-scan",
        1,
        "target",
        &harness.context.target_url
    )
    .is_err());
    let mut tally = AgentPipelineTally::default();
    record_agent_target_outcome(
        &harness.db_path,
        "agent-scan",
        &harness.context.route,
        owned.outcome.clone(),
        &mut tally,
    );
    drop(owned);
    let before = invocation_test_snapshot(&connection);
    let replay = run_agent_target(
        &prepared,
        AgentTargetExecution {
            db_path: &harness.db_path,
            scan_id: "agent-scan",
            attempt_number: 1,
            settings: &json!({"agentBackendPolicy":"native"}),
            environment: &harness.context.environment,
            adaptive: &AgentBudgetSettings::from_json(&json!({})),
            log_path: &harness.context.log_path,
        },
    );
    assert_eq!(replay.unwrap_err(), "agent_target_already_terminal");
    assert_eq!(before, invocation_test_snapshot(&connection));
    assert_eq!(seen.lock().unwrap().len(), 2);
}

#[test]
fn native_invocation_inactive_target_admission_is_non_mutating() {
    for mutation in [
        "UPDATE sentinel_scans SET status='paused'",
        "UPDATE sentinel_scans SET attempt_count=2",
    ] {
        let harness = agent_harness(
            "inactive-invocation",
            mock_site,
            vec![AgentIdentity::anonymous()],
        );
        let connection = db::open(&harness.db_path).unwrap();
        connection.execute_batch(mutation).unwrap();
        let before = invocation_test_snapshot(&connection);
        let prepared = PreparedFrontendTarget {
            position: 1,
            route: harness.context.route.clone(),
            target_dir: harness.context.target_dir.clone(),
            proxy: None,
            browser: None,
        };
        let result = run_agent_target(
            &prepared,
            AgentTargetExecution {
                db_path: &harness.db_path,
                scan_id: "agent-scan",
                attempt_number: 1,
                settings: &json!({"agentBackendPolicy":"native"}),
                environment: &harness.context.environment,
                adaptive: &AgentBudgetSettings::from_json(&json!({})),
                log_path: &harness.context.log_path,
            },
        );
        assert_eq!(result.unwrap_err(), "native_attempt_stopped_or_replaced");
        assert_eq!(before, invocation_test_snapshot(&connection));
    }
}
