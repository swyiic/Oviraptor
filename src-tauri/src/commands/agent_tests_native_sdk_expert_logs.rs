// Existing real Web expert entry; provider remains gated while SQLite is read.
#[test]
fn native_sdk_log_expert_actual_gate_uses_original_worker_and_replay_never_mints_dispatch() {
    use std::sync::{mpsc, Arc, Mutex};
    let (arrive, arrived) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let released = Arc::new(Mutex::new(released));
    let raw = "  专家原结果\nbyte exact  ";
    let (port, seen, _stop) = spawn_endpoint(Arc::new(move |_| {
        let _ = arrive.send(());
        let _ = released
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5));
        (200, "application/json", sdk_exact_model_body(raw))
    }));
    let (root, mut context, lease, child) = specialist_journal_fixture();
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let connection = db::open(&context.db_path).unwrap();
    // The lower-level specialist journal fixture predates persisted attempt rows.
    // Supply the original test attempt before I/O; do not relax the public reader.
    connection.execute(
        "INSERT INTO sentinel_scan_attempts(scan_id,attempt_number,status) VALUES(?1,?2,'scanning')",
        params![context.scan_id,context.attempt_number],
    ).unwrap();
    let original_plan: String = connection
        .query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [&lease.root_run_id],
            |r| r.get(0),
        )
        .unwrap();
    let input = json!({"evidence":"private-sdk-expert-input"});
    let (live, result) = std::thread::scope(|scope| {
        let (done, rx) = mpsc::channel();
        let context = &context;
        let lease = &lease;
        let child = &child;
        let input = &input;
        scope.spawn(move || {
            let _ = done.send(multi_agent_child_round_transport(
                context,
                lease,
                child,
                "private-sdk-expert-system",
                input.clone(),
            ));
        });
        arrived.recv_timeout(Duration::from_secs(5)).unwrap();
        let live = sdk_log_rows(&connection);
        let public = serde_json::to_value(
            crate::agent_runtime::model::diagnostics::replay::read(
                &context.db_path,
                &context.scan_id,
                1,
                None,
                None,
                0,
                300,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(public["rows"].as_array().unwrap().len(), 2);
        assert!(public["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["runId"] == child.run_id));

        assert!(
            rx.try_recv().is_err(),
            "the production SDK call must still be live"
        );
        let _ = release.send(());
        let result = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        (live, result)
    });
    let result = result.unwrap();
    assert_eq!(result.0.as_bytes(), raw.as_bytes());
    assert_eq!(result.1.total_tokens, 12);
    assert_eq!(
        live.iter()
            .map(|r| r["stage"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["prepared", "sent"]
    );
    let worker: (String, String) = connection
        .query_row(
            "SELECT id,worker_id FROM agent_assignment_attempts WHERE child_run_id=?1",
            [&child.run_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(live.iter().all(|r| r["domain"] == "specialist"
        && r["runId"] == child.run_id
        && r["rootRunId"] == lease.root_run_id
        && r["assignmentId"] == child.assignment_id
        && r["leaseAttemptId"] == worker.0
        && r["workerId"] == worker.1));
    let rows = sdk_log_rows(&connection);
    let public = serde_json::to_value(
        crate::agent_runtime::model::diagnostics::replay::read(
            &context.db_path,
            &context.scan_id,
            1,
            None,
            None,
            0,
            300,
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(public["rows"].as_array().unwrap().len(), 6);
    assert_eq!(public["rows"][5]["terminalState"], "returned");

    assert_eq!(
        rows.iter()
            .map(|r| r["stage"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "prepared",
            "sent",
            "response_received",
            "cost_saved",
            "validated",
            "terminal"
        ]
    );
    assert_eq!(rows.last().unwrap()["terminalState"], "returned");
    let request_hash: String = connection
        .query_row(
            "SELECT request_hash FROM agent_specialist_calls WHERE child_run_id=?1",
            [&child.run_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rows[0]["requestHash"], request_hash);
    let expected = crate::agent_runtime::store::stable_hash(
        &json!([
            "specialist",
            child.assignment_id,
            child.run_id,
            1,
            request_hash
        ])
        .to_string(),
    );
    assert_eq!(rows[0]["dispatchKey"], expected);
    let fee:(i64,i64)=connection.query_row("SELECT COUNT(*),COALESCE(SUM(amount),0) FROM agent_budget_entries WHERE assignment_id=?1 AND lease_attempt_id=?2 AND dimension='model_requests' AND kind='consume'",params![child.assignment_id,worker.0],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(fee, (1, 1));
    let before = super::tests::application_table_snapshot(&connection);
    let replay = multi_agent_child_round_transport(
        &context,
        &lease,
        &child,
        "private-sdk-expert-system",
        input,
    )
    .unwrap();
    assert_eq!(replay, result);
    assert_eq!(sdk_log_rows(&connection), rows);
    assert!(super::tests::application_table_snapshot(&connection) == before);
    assert_eq!(
        connection
            .query_row(
                "SELECT plan_json FROM agent_runs WHERE id=?1",
                [&lease.root_run_id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        original_plan
    );
    let persisted = json!(rows).to_string();
    for secret in [
        raw,
        "private-sdk-expert-input",
        "private-sdk-expert-system",
        "mock-key",
    ] {
        assert!(!persisted.contains(secret));
    }
    assert_eq!(seen.lock().unwrap().len(), 1);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}
