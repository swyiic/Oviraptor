fn authorization_fixture() -> (PathBuf, PathBuf, PathBuf, SaveAuthorizationControlInput) {
    let (root, db_path) = temp_database("authorization-control");
    seed_scan(&db_path, "agent-scan", "draft");
    seed_target(&db_path, "https://app.example.invalid");
    seed_session(&db_path, "session-a", "cookie-a", "Bearer a");
    seed_session(&db_path, "session-b", "cookie-b", "Bearer b");
    db::open(&db_path).unwrap().execute(
        "INSERT INTO sentinel_scan_contexts(scan_id,policy_json) VALUES('agent-scan',?1)",
        [serde_json::json!({"authSessionIds":["session-a","session-b"]}).to_string()],
    ).unwrap();
    let app_data_dir = root.join("app-data");
    let input = SaveAuthorizationControlInput {
        scan_id: "agent-scan".into(),
        attempt_number: 2,
        target_url: "https://app.example.invalid".into(),
        contract_key: "idor|/api/orders".into(),
        owner_object_url: "https://app.example.invalid/api/orders?id=owner".into(),
        tester_control_url: "https://app.example.invalid/api/orders?id=tester".into(),
        object_query_key: "id".into(),
        owner_object_value: "owner".into(),
        tester_object_value: "tester".into(),
        response_object_pointer: "/order/id".into(),
        owner_identity: "session-a".into(),
        tester_identity: "session-b".into(),
    };
    (root, db_path, app_data_dir, input)
}

fn freeze_authorization_test_plan(context: &AgentRunContext) {
    persist_agent_execution_plan(
        &context.db_path,
        &context.scan_id,
        context.attempt_number,
        &context.target_url,
        &context.execution_plan,
    ).unwrap();
}

#[test]
fn authorization_control_registers_only_for_next_draft_attempt_and_is_immutable() {
    let (_root, db_path, app_data_dir, mut input) = authorization_fixture();
    let mut connection = db::open(&db_path).unwrap();
    assert_eq!(save_authorization_control_in(&mut connection, &app_data_dir, &input), Ok(()));
    let stored: (i64, String) = connection.query_row(
        "SELECT attempt_number,owner_object_url FROM agent_authorization_controls WHERE scan_id='agent-scan'",
        [], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(stored, (2, input.owner_object_url.clone()));
    input.owner_object_value = "different".into();
    input.owner_object_url = "https://app.example.invalid/api/orders?id=different".into();
    assert_eq!(save_authorization_control_in(&mut connection, &app_data_dir, &input), Err("authorization_control_immutable_conflict".into()));
    let unchanged: String = connection.query_row(
        "SELECT owner_object_url FROM agent_authorization_controls WHERE scan_id='agent-scan'", [], |row| row.get(0),
    ).unwrap();
    assert_eq!(unchanged, stored.1);
}

#[test]
fn authorization_control_refuses_a_fifth_group_before_the_scan_starts() {
    let (_root, db_path, app_data_dir, mut input) = authorization_fixture();
    let mut connection = db::open(&db_path).unwrap();
    for index in 0..4 {
        input.contract_key = format!("orders-{index}");
        save_authorization_control_in(&mut connection, &app_data_dir, &input).unwrap();
    }
    input.contract_key = "orders-4".into();
    assert_eq!(save_authorization_control_in(&mut connection, &app_data_dir, &input),
        Err("authorization_control_group_limit_exceeded".into()));
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_authorization_controls WHERE scan_id=?1", [&input.scan_id],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(count, 4);
}

#[test]
fn authorization_control_rejects_scope_identity_selector_and_started_attempt() {
    let (_root, db_path, app_data_dir, input) = authorization_fixture();
    let mut connection = db::open(&db_path).unwrap();
    let mut candidate = input.clone();
    candidate.owner_object_url = "https://other.example.invalid/api/orders?id=owner".into();
    assert_eq!(save_authorization_control_in(&mut connection, &app_data_dir, &candidate), Err("authorization_control_scope_invalid".into()));
    candidate = input.clone();
    candidate.owner_object_url = "https://app.example.invalid/api/orders?id=owner&id=owner".into();
    assert_eq!(save_authorization_control_in(&mut connection, &app_data_dir, &candidate), Err("authorization_object_selector_missing_or_ambiguous".into()));
    candidate = input.clone();
    candidate.owner_identity = "unbound".into();
    assert_eq!(save_authorization_control_in(&mut connection, &app_data_dir, &candidate), Err("authorization_control_identity_unbound".into()));
    candidate = input.clone();
    candidate.response_object_pointer = "/order/~2bad".into();
    assert_eq!(save_authorization_control_in(&mut connection, &app_data_dir, &candidate), Err("authorization_control_invalid".into()));
    candidate = input.clone();
    candidate.attempt_number = 1;
    assert_eq!(save_authorization_control_in(&mut connection, &app_data_dir, &candidate), Err("authorization_control_task_binding_invalid".into()));
    connection.execute(
        "INSERT INTO agent_runs(id,scan_id,attempt_number,target_url) VALUES('started','agent-scan',2,'https://app.example.invalid')", [],
    ).unwrap();
    assert_eq!(save_authorization_control_in(&mut connection, &app_data_dir, &input), Err("authorization_control_attempt_already_started".into()));
    connection.execute("UPDATE sentinel_scans SET status='scanning' WHERE id='agent-scan'", []).unwrap();
    assert_eq!(save_authorization_control_in(&mut connection, &app_data_dir, &input), Err("authorization_control_task_binding_invalid".into()));
}

#[test]
fn authorization_control_follows_directory_planner_without_creating_an_attempt() {
    let (_root, db_path, app_data_dir, mut input) = authorization_fixture();
    let scan_dir = neutral_scan_work_root(&app_data_dir).join("agent-scan");
    fs::create_dir_all(scan_dir.join("attempt-0005")).unwrap();
    let mut connection = db::open(&db_path).unwrap();
    assert_eq!(save_authorization_control_in(&mut connection, &app_data_dir, &input), Err("authorization_control_task_binding_invalid".into()));
    input.attempt_number = 6;
    assert_eq!(save_authorization_control_in(&mut connection, &app_data_dir, &input), Ok(()));
    assert!(!scan_dir.join("attempt-0006").exists());
}

#[test]
fn authorization_control_table_is_created_for_an_existing_database() {
    let (root, db_path, _app_data_dir, _input) = authorization_fixture();
    let connection = db::open(&db_path).unwrap();
    connection.execute("DROP TABLE agent_authorization_controls", []).unwrap();
    drop(connection);
    let reopened = db::initialize(&root).unwrap();
    assert_eq!(reopened, db_path);
    let connection = db::open(&db_path).unwrap();
    let exists: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='agent_authorization_controls'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(exists, 1);
    let claims: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='agent_authorization_probe_claims'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(claims, 1);
}

#[test]
fn authorization_setup_exposes_only_bound_draft_target_and_valid_identity_handles() {
    let (_root, db_path, app_data_dir, input) = authorization_fixture();
    let mut connection = db::open(&db_path).unwrap();
    let setup = authorization_control_setup_in(
        &connection, &app_data_dir, &input.scan_id, &input.target_url,
    ).unwrap();
    assert_eq!(setup.attempt_number, 2);
    assert_eq!(setup.identity_ids, vec!["session-a", "session-b"]);
    assert!(setup.contract_keys.is_empty());
    let visible = serde_json::to_string(&setup).unwrap();
    assert!(!visible.contains("cookie-a"));
    assert!(!visible.contains("Bearer a"));
    save_authorization_control_in(&mut connection, &app_data_dir, &input).unwrap();
    let setup = authorization_control_setup_in(
        &connection, &app_data_dir, &input.scan_id, &input.target_url,
    ).unwrap();
    assert_eq!(setup.contract_keys, vec!["idor|/api/orders"]);
    assert!(authorization_control_setup_in(
        &connection, &app_data_dir, &input.scan_id, "https://other.example.invalid",
    ).is_err());
    connection.execute("UPDATE sentinel_scans SET status='scanning' WHERE id=?1", [&input.scan_id]).unwrap();
    assert!(authorization_control_setup_in(
        &connection, &app_data_dir, &input.scan_id, &input.target_url,
    ).is_err());
}

#[test]
fn authorization_setup_rejects_single_or_invalid_identity_without_leaking_session_material() {
    let (_root, db_path, app_data_dir, input) = authorization_fixture();
    let connection = db::open(&db_path).unwrap();
    connection.execute(
        "UPDATE sentinel_scan_contexts SET policy_json=?1 WHERE scan_id=?2",
        params![serde_json::json!({"authSessionIds":["session-a"]}).to_string(), input.scan_id],
    ).unwrap();
    assert_eq!(authorization_control_setup_in(
        &connection, &app_data_dir, &input.scan_id, &input.target_url,
    ).unwrap_err(), "authorization_control_requires_two_identities");
    connection.execute(
        "UPDATE sentinel_scan_contexts SET policy_json=?1 WHERE scan_id=?2",
        params![serde_json::json!({"authSessionIds":["session-a","session-b"]}).to_string(), input.scan_id],
    ).unwrap();
    connection.execute("UPDATE browser_auth_sessions SET status='invalid' WHERE id='session-b'", []).unwrap();
    assert!(authorization_control_setup_in(
        &connection, &app_data_dir, &input.scan_id, &input.target_url,
    ).is_err());
}

#[test]
fn authorization_three_side_proof_requires_owner_and_tester_object_controls() {
    let (_root, _db_path, _app_data_dir, control) = authorization_fixture();
    let owner = authorization_test_response(&control, "owner", 200, r#"{"order":{"id":"owner","name":"A"}}"#);
    let cross = authorization_test_response(&control, "cross", 200, r#"{"order":{"id":"owner","name":"A"}}"#);
    let own = authorization_test_response(&control, "tester", 200, r#"{"order":{"id":"tester","name":"B"}}"#);
    assert_eq!(verify_authorization_control_group(&control, [&owner, &cross, &own]), Ok(()));

    let mut personalized = cross.clone();
    personalized.body = r#"{"order":{"id":"tester","name":"B"}}"#.into();
    assert_eq!(verify_authorization_control_group(&control, [&owner, &personalized, &own]), Err("authorization_cross_object_not_observed"));
    let mut denied = cross.clone();
    denied.status = 403;
    assert_eq!(verify_authorization_control_group(&control, [&owner, &denied, &own]), Err("authorization_side_not_successful"));
    let mut redacted = cross.clone();
    redacted.body = r#"{"order":{"id":"owner"}}"#.into();
    assert_eq!(verify_authorization_control_group(&control, [&owner, &redacted, &own]), Err("authorization_cross_object_not_comparable"));
    let echoed = redacted.clone();
    let owner_echo = authorization_test_response(&control, "owner", 200, r#"{"order":{"id":"owner"}}"#);
    assert_eq!(verify_authorization_control_group(&control, [&owner_echo, &echoed, &own]),
        Err("authorization_cross_object_not_comparable"));
    let mut error = cross.clone();
    error.body = r#"{"order":{"id":"owner","name":"A"},"error":"forbidden"}"#.into();
    assert_eq!(verify_authorization_control_group(&control, [&owner, &error, &own]), Err("authorization_side_unusable"));
}

#[test]
fn authorization_three_side_proof_rejects_unbound_incomplete_or_artificial_responses() {
    let (_root, _db_path, _app_data_dir, control) = authorization_fixture();
    let owner = authorization_test_response(&control, "owner", 200, r#"{"order":{"id":"owner"}}"#);
    let cross = authorization_test_response(&control, "cross", 200, r#"{"order":{"id":"owner"}}"#);
    let own = authorization_test_response(&control, "tester", 200, r#"{"order":{"id":"tester"}}"#);
    type Mutation = Box<dyn Fn(&mut AuthorizationProbeResponse)>;
    let mut cases: Vec<(usize, &str, Mutation)> = vec![
        (0, "authorization_side_binding_invalid", Box::new(|r| r.identity = "session-b".into())),
        (1, "authorization_side_binding_invalid", Box::new(|r| r.url.push_str("&extra=1"))),
        (2, "authorization_side_binding_invalid", Box::new(|r| r.attempt_number += 1)),
        (1, "authorization_side_binding_invalid", Box::new(|r| r.contract_key = "other".into())),
        (0, "authorization_side_unusable", Box::new(|r| r.truncated = true)),
        (1, "authorization_side_unusable", Box::new(|r| r.cache_state = "hit".into())),
        (1, "authorization_side_unusable", Box::new(|r| r.redirect_code = "in_scope".into())),
        (2, "authorization_side_unusable", Box::new(|r| r.content_type = "text/html".into())),
        (0, "authorization_object_pointer_missing", Box::new(|r| r.body = "{}".into())),
        (1, "authorization_side_unusable", Box::new(|r| r.body = "<html>login</html>".into())),
    ];
    for (index, expected, mutate) in cases.drain(..) {
        let mut responses = [owner.clone(), cross.clone(), own.clone()];
        mutate(&mut responses[index]);
        assert_eq!(verify_authorization_control_group(&control, [&responses[0], &responses[1], &responses[2]]), Err(expected), "case {index}: {expected}");
    }
}

fn authorization_test_response(
    control: &SaveAuthorizationControlInput, side: &str, status: u16, body: &str,
) -> AuthorizationProbeResponse {
    let (identity, url) = match side {
        "owner" => (&control.owner_identity, &control.owner_object_url),
        "cross" => (&control.tester_identity, &control.owner_object_url),
        "tester" => (&control.tester_identity, &control.tester_control_url),
        _ => panic!("invalid side"),
    };
    AuthorizationProbeResponse {
        attempt_number: control.attempt_number,
        contract_key: control.contract_key.clone(),
        identity: identity.clone(), url: url.clone(), status,
        content_type: "application/json".into(), redirect_code: String::new(),
        cache_state: "miss".into(), truncated: false, body: body.into(),
    }
}

#[test]
fn authorization_broker_only_claims_registered_sides_under_a_live_child_lease() {
    use crate::agent_runtime::{
        contract::{AgentBackendKind, AgentLane, AgentRole, AgentRunStatus, MultiAgentPolicy},
        multi_agent::{lease, scheduler}, store::{self, AgentRunRow},
    };
    let (_root, db_path, app_data_dir, control) = authorization_fixture();
    let mut connection = db::open(&db_path).unwrap();
    let mut context = test_context(&db_path, &control.target_url,
        vec![AgentIdentity::scoped("session-a"), AgentIdentity::scoped("session-b")]);
    context.attempt_number = 2;
    context.execution_plan = context.execution_plan.with_attempt(2);
    let missing_run = context.clone();
    let dummy_child = scheduler::ScheduledChild {
        assignment_id:"absent".into(), run_id:"absent".into(), role:AgentRole::Authorization,
    };
    assert_eq!(claim_authorization_probe(&missing_run, &dummy_child, &control,
        "owner", &control.owner_identity, &control.owner_object_url).unwrap_err(),
        "authorization_side_binding_invalid");
    save_authorization_control_in(&mut connection, &app_data_dir, &control).unwrap();
    connection.execute("UPDATE sentinel_scans SET status='scanning',attempt_count=2 WHERE id=?1", [&control.scan_id]).unwrap();
    let mut root = AgentRunRow::new("authorization-root", &control.scan_id, 2,
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
    assert_eq!(scheduler::schedule_child(&connection, &coordinator, AgentRole::Authorization,
        AgentLane::TargetTouching, "test", &serde_json::json!({}), 1,
        &["replay_http".into()], 0, 3).unwrap_err(), "role_capability_not_allowed");
    assert_eq!(scheduler::schedule_child(&connection, &coordinator, AgentRole::Authorization,
        AgentLane::ReadOnlyAnalysis, "test", &serde_json::json!({}), 1,
        &["authorization_probe".into()], 0, 3).unwrap_err(), "authorization_requires_target_lane");
    let child = scheduler::schedule_child(&connection, &coordinator, AgentRole::Authorization,
        AgentLane::TargetTouching, "test", &serde_json::json!({}), 1,
        &["authorization_probe".into()], 0, 0).unwrap();
    scheduler::mark_child_running(&connection, &coordinator, &child).unwrap();
    context.run = Some(AgentRunLedger { db_path:db_path.clone(), run_id:child.run_id.clone() });
    assert_eq!(claim_authorization_probe(&context, &child, &control,
        "cross", &control.tester_identity, &control.tester_control_url).unwrap_err(),
        "authorization_side_binding_invalid");
    accounting_native(&context,400);
    assert_eq!(claim_authorization_probe(&context, &child, &control,
        "owner", &control.owner_identity, &control.owner_object_url).unwrap_err(),
        "authorization_request_budget_exhausted");
    accounting_native(&context,0);
    claim_authorization_probe(&context, &child, &control, "owner",
        &control.owner_identity, &control.owner_object_url).unwrap();
    let usage = agent_request_accounting(&connection,&context.scan_id,2,&context.target_url).unwrap();
    assert_eq!((usage.authorization.received,usage.authorization.unresolved,usage.recorded,usage.budget_committed),(0,1,0,1));
    assert_eq!(claim_authorization_probe(&context, &child, &control, "owner",
        &control.owner_identity, &control.owner_object_url).unwrap_err(), "authorization_side_already_claimed");
    connection.execute("UPDATE sentinel_scans SET status='paused' WHERE id=?1", [&control.scan_id]).unwrap();
    assert_eq!(claim_authorization_probe(&context, &child, &control, "cross",
        &control.tester_identity, &control.owner_object_url).unwrap_err(),
        "authorization_capability_or_fencing_denied");
    connection.execute("UPDATE sentinel_scans SET status='scanning',attempt_count=3 WHERE id=?1", [&control.scan_id]).unwrap();
    assert_eq!(claim_authorization_probe(&context, &child, &control, "cross",
        &control.tester_identity, &control.owner_object_url).unwrap_err(),
        "authorization_capability_or_fencing_denied");
    connection.execute("UPDATE sentinel_scans SET attempt_count=2 WHERE id=?1", [&control.scan_id]).unwrap();
    connection.execute("UPDATE agent_runs SET status='terminal' WHERE id=?1", [&coordinator.root_run_id]).unwrap();
    assert_eq!(claim_authorization_probe(&context, &child, &control, "cross",
        &control.tester_identity, &control.owner_object_url).unwrap_err(),
        "authorization_capability_or_fencing_denied");
    connection.execute("UPDATE agent_runs SET status='running' WHERE id=?1", [&coordinator.root_run_id]).unwrap();
    connection.execute("UPDATE agent_capability_leases SET revoked_at=datetime('now','localtime') WHERE child_run_id=?1", [&child.run_id]).unwrap();
    assert_eq!(claim_authorization_probe(&context, &child, &control, "cross",
        &control.tester_identity, &control.owner_object_url).unwrap_err(),
        "authorization_capability_or_fencing_denied");
}

#[test]
fn authorization_control_reaches_an_independent_reviewer_before_publication() {
    run_authorization_control_to_review(false, false, 0, 0, false, false, false);
}

#[test]
fn rejected_review_completes_the_child_without_publishing_a_finding() {
    run_authorization_control_to_review(false, false, 0, 0, true, false, false);
}

#[test]
fn denied_cross_object_control_does_not_create_a_candidate_or_finding() {
    run_authorization_control_to_review(true, false, 0, 0, false, false, false);
}

#[test]
fn authorization_mailbox_failure_releases_target_lane_and_reservation() {
    run_authorization_control_to_review(false, true, 0, 0, false, false, false);
}

#[test]
fn pausing_after_the_owner_probe_prevents_the_cross_and_control_requests() {
    run_authorization_control_to_review(false, false, 1, 0, false, false, false);
}

#[test]
fn pausing_after_the_cross_probe_prevents_the_control_request() {
    run_authorization_control_to_review(false, false, 2, 0, false, false, false);
}

#[test]
fn pausing_after_the_control_response_prevents_finding_publication() {
    run_authorization_control_to_review(false, false, 3, 0, false, false, false);
}

#[test]
fn disconnected_owner_control_is_not_replayed_or_reviewed() {
    run_authorization_control_to_review(false, false, 0, 1, false, false, false);
}

#[test]
fn disconnected_cross_control_is_not_replayed_or_reviewed() {
    run_authorization_control_to_review(false, false, 0, 2, false, false, false);
}

#[test]
fn disconnected_tester_control_is_not_replayed_or_reviewed() {
    run_authorization_control_to_review(false, false, 0, 3, false, false, false);
}

#[test]
fn pause_during_candidate_write_cannot_stage_or_publish_authorization() {
    run_authorization_control_to_review(false, false, 0, 0, false, true, false);
}

#[test]
fn pause_after_authorization_before_review_cannot_publish() {
    run_authorization_control_to_review(false, false, 0, 0, false, false, true);
}

fn run_authorization_control_to_review(
    deny_cross: bool,
    fail_mailbox: bool,
    pause_after_probe: usize,
    disconnect_probe: usize,
    reject_review: bool,
    pause_on_candidate: bool,
    pause_before_review: bool,
) {
    use crate::agent_runtime::{
        contract::{AgentBackendKind, AgentLane, AgentRole, AgentRunStatus, MultiAgentPolicy},
        store::{self, AgentRunRow},
    };
    use std::net::TcpListener;
    let (_root, db_path, app_data_dir, mut control) = authorization_fixture();
    let server_db_path = db_path.clone();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let target = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(12);
        let mut seen = Vec::new();
        while seen.len() < if pause_after_probe > 0 { pause_after_probe } else if disconnect_probe > 0 { disconnect_probe } else { 3 }
            && std::time::Instant::now() < deadline {
            let Ok((mut stream, _)) = listener.accept() else {
                std::thread::sleep(Duration::from_millis(10)); continue;
            };
            // Accepted sockets may inherit the listener's nonblocking mode on
            // some platforms; the fixture expects to read a complete request.
            stream.set_nonblocking(false).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut buffer = [0_u8; 4096];
            let length = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..length]);
            let first_line = request.lines().next().unwrap_or_default().to_string();
            let denied = deny_cross && first_line.contains("id=owner")
                && request.to_ascii_lowercase().contains("cookie-b");
            let object = if first_line.contains("id=tester") { "tester" } else { "owner" };
            let body = format!(r#"{{"order":{{"id":"{object}","name":"order-{object}"}}}}"#);
            let status = if denied { "403 Forbidden" } else { "200 OK" };
            let response = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            if pause_after_probe > 0 && seen.len() + 1 == pause_after_probe {
                db::open(&server_db_path).unwrap().execute(
                    "UPDATE sentinel_scans SET status='paused' WHERE id='agent-scan'", [],
                ).unwrap();
            }
            if disconnect_probe > 0 && seen.len() + 1 == disconnect_probe {
                // The request may have reached the target even though its
                // result is unknown; deliberately break a framed response.
                stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 999\r\nConnection: close\r\n\r\n{").unwrap();
            } else {
                stream.write_all(response.as_bytes()).unwrap();
            }
            seen.push(first_line);
        }
        seen
    });
    let mut connection = db::open(&db_path).unwrap();
    connection.execute("UPDATE sentinel_targets SET url=?1 WHERE scan_id='agent-scan'", [&target]).unwrap();
    connection.execute("UPDATE browser_auth_sessions SET entry_url=?1,session_json=json_set(session_json,'$.entryUrl',?1,'$.scopeHosts[0]','127.0.0.1')", [&target]).unwrap();
    control.target_url = target.clone();
    control.owner_object_url = format!("{target}/api/orders?id=owner");
    control.tester_control_url = format!("{target}/api/orders?id=tester");
    save_authorization_control_in(&mut connection, &app_data_dir, &control).unwrap();
    connection.execute("UPDATE sentinel_scans SET status='scanning',attempt_count=2 WHERE id='agent-scan'", []).unwrap();
    let root_id = "authorization-review-root";
    let mut context = test_context(&db_path, &target,
        vec![AgentIdentity::scoped("session-a"), AgentIdentity::scoped("session-b")]);
    context.attempt_number = 2;
    context.execution_plan = context.execution_plan.with_attempt(2);
    let mut root = AgentRunRow::new(root_id, &control.scan_id, 2, &target,
        AgentBackendKind::Native, AgentRole::Coordinator,
        context.execution_plan.hash(), "evidence")
        .with_budget(60_000, 120_000, 40, 80);
    root.status = AgentRunStatus::Running;
    root.root_run_id = root.id.clone();
    root.orchestration_policy = MultiAgentPolicy::Multi;
    root.lane = Some(AgentLane::ReadOnlyAnalysis);
    store::create_run(&connection, &root).unwrap();
    freeze_authorization_test_plan(&context);
    if reject_review {
        context.evidence["forceReviewerRejectedForTest"] = serde_json::json!(true);
    }
    context.target_dir = app_data_dir.join("authorization-review-artifacts");
    context.run = Some(AgentRunLedger { db_path:db_path.clone(), run_id:root_id.into() });
    let mut session = multi_agent_prepare(&mut context).unwrap();
    let outcome = AgentTargetOutcome::incomplete("fixture executor finished without a finding");
    multi_agent_finish_execution(&context, &mut session, &outcome).unwrap();
    let before: i64 = connection.query_row("SELECT COUNT(*) FROM sentinel_findings WHERE scan_id='agent-scan'", [], |row| row.get(0)).unwrap();
    assert_eq!(before, 0);
    if pause_on_candidate {
        connection.execute_batch("CREATE TRIGGER pause_authorization_candidate BEFORE INSERT ON agent_finding_candidates \
            BEGIN UPDATE sentinel_scans SET status='paused' WHERE id='agent-scan'; END;").unwrap();
    }
    if fail_mailbox || pause_after_probe > 0 || disconnect_probe > 0 || pause_on_candidate {
        if fail_mailbox {
            connection.execute_batch(
                "CREATE TRIGGER reject_authorization_result BEFORE INSERT ON agent_messages \
                 WHEN NEW.kind='authorization_result' BEGIN SELECT RAISE(ABORT,'authorization mailbox unavailable'); END;",
            ).unwrap();
        }
        let error = multi_agent_authorization(&context, &mut session).unwrap_err();
        if fail_mailbox {
            assert!(error.contains("authorization mailbox unavailable"), "{error}");
        } else if pause_on_candidate {
            assert!(error.contains("authorization_attempt_cancelled"), "{error}");
        } else if disconnect_probe > 0 {
            assert!(error.contains("authorization_response_read_failed")
                || error.contains("authorization_request_failed"), "{error}");
        } else {
            assert_eq!(error, "agent_attempt_not_active");
        }
        let state: String = connection.query_row(
            "SELECT state FROM agent_assignments WHERE role='authorization' AND coordinator_run_id=?1",
            [root_id], |row| row.get(0),
        ).unwrap();
        assert_eq!(state, "failed");
        let lanes: i64 = connection.query_row(
            "SELECT COUNT(*) FROM agent_lane_leases WHERE scan_id='agent-scan' AND lane='target_touching'",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(lanes, 0);
        let reserved: i64 = connection.query_row(
            "SELECT reserved_requests FROM agent_budget_ledger WHERE root_run_id=?1",
            [root_id], |row| row.get(0),
        ).unwrap();
        assert_eq!(reserved, 0);
        let published: i64 = connection.query_row(
            "SELECT COUNT(*) FROM sentinel_findings WHERE scan_id='agent-scan'",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(published, 0);
        if pause_after_probe > 0 {
            let late_results: i64 = connection.query_row(
                "SELECT COUNT(*) FROM agent_messages WHERE root_run_id=?1 AND kind='authorization_result'",
                [root_id], |row| row.get(0),
            ).unwrap();
            assert_eq!(late_results, 0, "pause must not publish a normal Authorization result");
        }
        let claims: i64 = connection.query_row(
            "SELECT COUNT(*) FROM agent_authorization_probe_claims WHERE scan_id='agent-scan'",
            [], |row| row.get(0),
        ).unwrap();
        let expected_claims = if pause_after_probe > 0 { pause_after_probe as i64 }
            else if disconnect_probe > 0 { disconnect_probe as i64 } else { 3 };
        assert_eq!(claims, expected_claims);
        if pause_after_probe > 0 || disconnect_probe > 0 || pause_on_candidate {
            let pending: i64 = connection.query_row(
                "SELECT COUNT(*) FROM agent_finding_candidates WHERE root_run_id=?1 AND status='pending'",
                [root_id], |row| row.get(0),
            ).unwrap();
            assert_eq!(pending, 0);
        }
        if disconnect_probe > 0 {
            let failed_side: (String, String) = connection.query_row(
                "SELECT side,artifact_id FROM agent_authorization_probe_claims \
                 WHERE scan_id='agent-scan' AND side=?1", [["owner", "cross", "tester"][disconnect_probe - 1]],
                |row| Ok((row.get(0)?, row.get(1)?)),
            ).unwrap();
            assert_eq!(failed_side.0, ["owner", "cross", "tester"][disconnect_probe - 1]);
            assert!(failed_side.1.is_empty(), "an incomplete response must not be usable evidence");
            let _ = multi_agent_authorization(&context, &mut session).unwrap_err();
            let after_retry: i64 = connection.query_row(
                "SELECT COUNT(*) FROM agent_authorization_probe_claims WHERE scan_id='agent-scan'",
                [], |row| row.get(0),
            ).unwrap();
        assert_eq!(after_retry, expected_claims, "a claimed network call must not be reissued in the same attempt");
            let usage = agent_request_accounting(&connection,&context.scan_id,2,&context.target_url).unwrap();
            assert_eq!(usage.authorization.unresolved,1);
            assert_eq!(usage.authorization.received,expected_claims-1);
            assert_eq!(usage.budget_committed,expected_claims);
        }
        assert_eq!(server.join().unwrap().len(), expected_claims as usize);
        return;
    }
    multi_agent_authorization(&context, &mut session).unwrap();
    let model_usage: (i64,i64) = connection.query_row(
        "SELECT a.reserved_requests,r.used_requests FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE a.role='authorization' AND a.coordinator_run_id=?1",
        [root_id],|r|Ok((r.get(0)?,r.get(1)?)),
    ).unwrap();
    assert_eq!(model_usage,(0,0),"target traffic must not spend model requests");
    let pending: i64 = connection.query_row("SELECT COUNT(*) FROM agent_finding_candidates WHERE root_run_id=?1 AND status='pending'", [root_id], |row| row.get(0)).unwrap();
    assert_eq!(pending, if deny_cross { 0 } else { 1 });
    let before_review: i64 = connection.query_row("SELECT COUNT(*) FROM sentinel_findings WHERE scan_id='agent-scan'", [], |row| row.get(0)).unwrap();
    assert_eq!(before_review, 0);
    if pause_before_review {
        connection.execute("UPDATE sentinel_scans SET status='paused' WHERE id='agent-scan'", []).unwrap();
        assert!(matches!(multi_agent_review(&context, &mut session, outcome), AgentTargetOutcome::Cancelled));
        let reviewers: i64 = connection.query_row(
            "SELECT COUNT(*) FROM agent_runs WHERE root_run_id=?1 AND role='evidence_reviewer'",
            [root_id], |row| row.get(0),
        ).unwrap();
        assert_eq!(reviewers, 0, "a paused attempt must not start a new Reviewer");
        let published: i64 = connection.query_row("SELECT COUNT(*) FROM sentinel_findings WHERE scan_id='agent-scan'", [], |row| row.get(0)).unwrap();
        assert_eq!(published, 0, "a paused attempt must not publish after Authorization finished");
        assert_eq!(server.join().unwrap().len(), 3);
        return;
    }
    let _reviewed = multi_agent_review(&context, &mut session, outcome.clone());
    let published: i64 = connection.query_row("SELECT COUNT(*) FROM sentinel_findings WHERE scan_id='agent-scan' AND record_json LIKE '%authorization_cross_object_read%'", [], |row| row.get(0)).unwrap();
    assert_eq!(published, if deny_cross || reject_review { 0 } else { 1 });
    if reject_review {
        let decision: (String, String, String, String) = connection.query_row(
            "SELECT q.status,d.verdict,fc.status,a.state FROM agent_review_requests q \
             JOIN agent_review_decisions d ON d.id=q.decision_id \
             JOIN agent_finding_candidates fc ON fc.root_run_id=q.root_run_id AND fc.candidate_revision=q.candidate_revision \
             JOIN agent_assignments a ON a.id=q.assignment_id WHERE q.root_run_id=?1",
            [root_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
        ).unwrap();
        assert_eq!(decision, ("rejected".into(), "rejected".into(), "rejected".into(), "completed".into()));
        let status = native_scan_status(&connection, "agent-scan").unwrap();
        assert_eq!(status["reviewGateSatisfied"], true);
        assert_eq!(status["reviewStatus"], "rejected");
        // This fixture bypasses a real WebExecutor tool invocation, so the
        // entire orchestration must remain unready even though review closed.
        assert_eq!(status["multiAgentReady"], false);
        connection.execute("UPDATE agent_messages SET acknowledged_at='' WHERE root_run_id=?1 AND kind='review_decision'", [root_id]).unwrap();
        let unacknowledged = native_scan_status(&connection, "agent-scan").unwrap();
        assert_eq!(unacknowledged["reviewGateSatisfied"], false);
        assert_eq!(unacknowledged["multiAgentReady"], false);
        connection.execute("UPDATE agent_messages SET acknowledged_at=datetime('now','localtime') WHERE root_run_id=?1 AND kind='review_decision'", [root_id]).unwrap();
        connection.execute("UPDATE agent_finding_candidates SET candidate_revision=candidate_revision+1 WHERE root_run_id=?1", [root_id]).unwrap();
        assert_eq!(native_scan_status(&connection, "agent-scan").unwrap()["reviewGateSatisfied"], false,
            "a stale decision must not satisfy a newer candidate revision");
        connection.execute("UPDATE agent_finding_candidates SET candidate_revision=candidate_revision-1 WHERE root_run_id=?1", [root_id]).unwrap();
        connection.execute("UPDATE agent_assignments SET state='failed' WHERE coordinator_run_id=?1 AND role='evidence_reviewer'", [root_id]).unwrap();
        assert_eq!(native_scan_status(&connection, "agent-scan").unwrap()["reviewGateSatisfied"], false,
            "a failed child cannot turn its historical decision into a completed review");
    }
    let replayed = multi_agent_review(&context, &mut session, outcome);
    assert!(matches!(replayed, AgentTargetOutcome::Incomplete(_)), "{replayed:?}");
    let reviews: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_runs WHERE root_run_id=?1 AND role='evidence_reviewer'",
        [root_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(reviews, if deny_cross { 0 } else { 1 },
        "denied control has no candidate to review; publication must not trigger a second Reviewer call");
    let roles: (i64, i64) = connection.query_row(
        "SELECT COUNT(CASE WHEN role='authorization' AND terminal_state='completed' THEN 1 END), \
         COUNT(CASE WHEN role='evidence_reviewer' AND terminal_state='completed' THEN 1 END) \
         FROM agent_runs WHERE root_run_id=?1", [root_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(roles, (1, if deny_cross { 0 } else { 1 }));
    let acknowledged: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_messages WHERE root_run_id=?1 AND kind IN ('authorization_result','review_decision') AND acknowledged_at<>''",
        [root_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(acknowledged, if deny_cross { 1 } else { 2 });
    assert_eq!(server.join().unwrap(), vec![
        "GET /api/orders?id=owner HTTP/1.1", "GET /api/orders?id=owner HTTP/1.1",
        "GET /api/orders?id=tester HTTP/1.1",
    ]);
}
