#[test]
fn authorization_child_sends_only_three_registered_gets_and_keeps_raw_bodies_private() {
    use crate::agent_runtime::{
        contract::{AgentBackendKind, AgentLane, AgentRole, AgentRunStatus, MultiAgentPolicy},
        multi_agent::{lease, scheduler}, store::{self, AgentRunRow},
    };
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let target = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let server = std::thread::spawn(move || {
        let mut seen = Vec::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        while seen.len() < 3 && std::time::Instant::now() < deadline {
            let Ok((mut stream, _)) = listener.accept() else {
                std::thread::sleep(Duration::from_millis(10)); continue;
            };
            // A socket accepted from a nonblocking listener may also be
            // nonblocking on some platforms. Read the fixture request with
            // the bounded timeout below instead of racing a WouldBlock.
            stream.set_nonblocking(false).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut buffer = [0_u8; 4096];
            let count = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..count]).to_string();
            let first = request.lines().next().unwrap_or_default().to_string();
            seen.push(first.clone());
            let object = if first.contains("id=tester") { "tester" } else { "owner" };
            let body = format!(r#"{{"order":{{"id":"{object}","name":"record-{object}"}}}}"#);
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            stream.write_all(response.as_bytes()).unwrap();
        }
        seen
    });
    let (_root, db_path, app_data_dir, mut control) = authorization_fixture();
    let mut connection = db::open(&db_path).unwrap();
    connection.execute("UPDATE sentinel_targets SET url=?1 WHERE scan_id='agent-scan'", [&target]).unwrap();
    connection.execute("UPDATE browser_auth_sessions SET entry_url=?1,session_json=json_set(session_json,'$.entryUrl',?1,'$.scopeHosts[0]','127.0.0.1')", [&target]).unwrap();
    control.target_url = target.clone();
    control.owner_object_url = format!("{target}/api/orders?id=owner");
    control.tester_control_url = format!("{target}/api/orders?id=tester");
    save_authorization_control_in(&mut connection, &app_data_dir, &control).unwrap();
    connection.execute("UPDATE sentinel_scans SET status='scanning',attempt_count=2 WHERE id='agent-scan'", []).unwrap();
    let mut context = test_context(&db_path, &target,
        vec![AgentIdentity::scoped("session-a"), AgentIdentity::scoped("session-b")]);
    context.attempt_number = 2;
    context.execution_plan = context.execution_plan.with_attempt(2);
    let mut root = AgentRunRow::new("authorization-http-root", &control.scan_id, 2,
        &control.target_url, AgentBackendKind::Native, AgentRole::Coordinator,
        context.execution_plan.hash(), "evidence")
        .with_budget(100, 1000, 2, 10);
    root.status = AgentRunStatus::Running;
    root.root_run_id = root.id.clone();
    root.orchestration_policy = MultiAgentPolicy::Multi;
    root.lane = Some(AgentLane::ReadOnlyAnalysis);
    store::create_run(&connection, &root).unwrap();
    freeze_authorization_test_plan(&context);
    let coordinator = lease::acquire_coordinator_lease(&connection, &control.scan_id, 2,
        &control.target_url, &root.id, 600).unwrap();
    let child = scheduler::schedule_child(&connection, &coordinator, AgentRole::Authorization,
        AgentLane::TargetTouching, "http-test", &serde_json::json!({}), 1,
        &["authorization_probe".into()], 0, 0).unwrap();
    scheduler::mark_child_running(&connection, &coordinator, &child).unwrap();
    context.target_dir = app_data_dir.join("private-evidence");
    context.run = Some(AgentRunLedger { db_path:db_path.clone(), run_id:child.run_id.clone() });
    let frozen = context.execution_plan.as_json();
    let mut changed = frozen.clone();
    changed["mode"] = serde_json::json!("deep");
    let mut unknown_surface = frozen.clone();
    unknown_surface["executionSurface"] = serde_json::json!("host_linux");
    for (stored, expected) in [
        (serde_json::json!({}), "tool_execution_plan_unavailable"),
        (changed, "tool_execution_surface_denied"),
        (unknown_surface, "tool_execution_surface_denied"),
    ] {
        connection.execute("UPDATE agent_runs SET plan_json=?1 WHERE id=?2",
            rusqlite::params![stored.to_string(), root.id]).unwrap();
        for (side, identity, url) in [
            ("owner", &control.owner_identity, &control.owner_object_url),
            ("cross", &control.tester_identity, &control.owner_object_url),
            ("tester", &control.tester_identity, &control.tester_control_url),
        ] {
            assert_eq!(execute_authorization_side(&context, &child, &control, side, identity, url)
                .unwrap_err(), expected);
            let claims: i64 = connection.query_row(
                "SELECT COUNT(*) FROM agent_authorization_probe_claims WHERE child_run_id=?1",
                [&child.run_id], |row| row.get(0),
            ).unwrap();
            assert_eq!(claims, 0, "{side} must not reserve a request for an invalid plan");
        }
    }
    connection.execute("UPDATE agent_runs SET plan_json=?1 WHERE id=?2",
        rusqlite::params![frozen.to_string(), root.id]).unwrap();
    let (a, _) = execute_authorization_side(&context, &child, &control, "owner",
        &control.owner_identity, &control.owner_object_url).unwrap();
    let (b, _) = execute_authorization_side(&context, &child, &control, "cross",
        &control.tester_identity, &control.owner_object_url).unwrap();
    let (c, _) = execute_authorization_side(&context, &child, &control, "tester",
        &control.tester_identity, &control.tester_control_url).unwrap();
    assert_eq!(verify_authorization_control_group(&control, [&a,&b,&c]), Ok(()));
    let usage = agent_request_accounting(&connection,&context.scan_id,2,&context.target_url).unwrap();
    assert_eq!((usage.authorization.received,usage.authorization.unresolved,usage.recorded,usage.budget_committed),(3,0,3,3));
    let charged=crate::agent_runtime::multi_agent::budget::balance(&connection,&root.id,None,"target_requests").unwrap();
    assert_eq!((charged.reserved,charged.consumed,charged.indeterminate),(0,3,0));
    assert_eq!(claim_authorization_probe(&context, &child, &control, "owner",
        &control.owner_identity, &control.owner_object_url).unwrap_err(), "authorization_request_budget_exhausted");
    let seen = server.join().unwrap();
    assert_eq!(seen, vec![
        "GET /api/orders?id=owner HTTP/1.1", "GET /api/orders?id=owner HTTP/1.1",
        "GET /api/orders?id=tester HTTP/1.1",
    ]);
    let audit = fs::read_to_string(context.target_dir.join(AGENT_HTTP_DIRECTORY).join("0001.json")).unwrap();
    assert!(!audit.contains("cookie-a"));
    assert!(!audit.contains("Bearer a"));
    assert!(context.target_dir.join(AGENT_HTTP_DIRECTORY).join("0001.body").exists());
}

