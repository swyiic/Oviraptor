#[test]
fn http_batch_rechecks_capability_and_ceiling_before_each_send() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };
    let (root, path, _root_run_id, lease) = multi_agent_test_root("http-batch-preflight", 100, 10);
    let connection = db::open(&path).unwrap();
    let child = scheduler::schedule_child(
        &connection, &lease, AgentRole::WebExecutor, AgentLane::TargetTouching,
        "batch-preflight", &serde_json::json!({}), 1, &["replay_http".into()], 10, 1,
    ).unwrap();
    scheduler::mark_child_running(&connection, &lease, &child).unwrap();
    let mut context = test_context(&path, &lease.target_key, vec![AgentIdentity::anonymous()]);
    context.scan_id = lease.scan_id.clone();
    context.run = Some(AgentRunLedger { db_path: path.clone(), run_id: child.run_id.clone() });
    let request = AgentHttpRequest {
        identity: AgentIdentity::anonymous(), method: "GET".into(), url: lease.target_key.clone(),
        extra_headers: Vec::new(), body: None, content_type: None,
        contract_key: String::new(), family: "authorization".into(),
        source: ScopeSource::IdentityComparison, tool: "replay_http".into(), timeout_seconds: 5,
    };
    let mut runtime = AgentToolRuntime {
        target_requests: agent_target_request_ceiling(&context),
        ..Default::default()
    };
    let result = agent_http_exchange(&context, &mut runtime, &request).unwrap_err();
    assert_eq!(result["code"], "request_budget_exhausted");
    assert_eq!(runtime.target_requests, agent_target_request_ceiling(&context));

    runtime.target_requests = 0;
    connection.execute(
        "UPDATE agent_capability_leases SET revoked_at=datetime('now','localtime') WHERE child_run_id=?1",
        [&child.run_id],
    ).unwrap();
    let result = agent_http_exchange(&context, &mut runtime, &request).unwrap_err();
    assert_eq!(result["code"], "request_budget_exhausted", "an already stopped executor cannot reset its stop by changing counters");
    // Independently verify that a fresh executor still checks the revoked lease
    // at the send boundary, without clearing a stopped executor's state.
    let mut runtime = AgentToolRuntime::default();
    let result = agent_http_exchange(&context, &mut runtime, &request).unwrap_err();
    assert_eq!(result["code"], "tool_capability_or_fencing_denied");
    assert_eq!(runtime.target_requests, 0);
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn identity_pair_requires_two_remaining_target_requests() {
    let (_root,path) = temp_database("agent-pair-budget");
    let context = test_context(&path, "https://example.invalid", vec![
        AgentIdentity::scoped("account-a"), AgentIdentity::scoped("account-b"),
    ]);
    let ceiling = agent_target_request_ceiling(&context);
    assert!(ceiling >= 2, "fixture must have a readable shared request ledger");
    let mut runtime = AgentToolRuntime {
        target_requests: ceiling - 1,
        ..Default::default()
    };
    let result = agent_tool_compare_identities(&context, &mut runtime, &serde_json::json!({
        "leftIdentity":"account-a", "rightIdentity":"account-b", "method":"GET",
        "path":"/api/orders/1", "family":"authorization"
    }));
    assert_eq!(result["code"], "comparison_request_budget_unavailable");
    assert_eq!(runtime.requests.len(), 0);
}

#[test]
fn identity_comparison_url_keeps_query_and_rejects_other_origins() {
    let target = "https://app.example.invalid/orders/landing";
    assert_eq!(
        agent_identity_comparison_url(target, "/api/orders?id=42&view=summary").unwrap(),
        "https://app.example.invalid/api/orders?id=42&view=summary"
    );
    assert_eq!(
        agent_identity_comparison_url(target, "api/orders?id=42").unwrap(),
        "https://app.example.invalid/api/orders?id=42"
    );
    assert_eq!(
        agent_identity_comparison_url(target, "https://elsewhere.invalid/api/orders").unwrap_err(),
        "comparison_origin_denied"
    );
    assert_eq!(
        agent_identity_comparison_url(target, "//elsewhere.invalid/api/orders").unwrap_err(),
        "comparison_origin_denied"
    );
    assert_eq!(
        agent_identity_comparison_url(target, "https://account@app.example.invalid/api/orders").unwrap_err(),
        "comparison_origin_denied"
    );
}

#[test]
fn tool_audit_rejects_missing_or_replayed_completion_without_orphan_event() {
    let (root, path, root_run_id, _) = multi_agent_test_root("tool-audit", 100, 10);
    let run = AgentRunLedger { db_path: path.clone(), run_id: root_run_id.clone() };
    let arguments = serde_json::json!({"kind":"all"});
    let row = run.begin_tool("inspect_evidence", &arguments, "inv-1-001").unwrap();
    let result = serde_json::json!({"code":"tool_capability_or_fencing_denied"});
    assert_eq!(run.finish_tool(Some(row + 100), "inspect_evidence", &arguments, &result, "inv-1-001", 0).unwrap_err(),
        "tool_invocation_not_running_or_missing");
    let connection = db::open(&path).unwrap();
    let before: (String, i64) = connection.query_row(
        "SELECT status,(SELECT COUNT(*) FROM agent_events WHERE run_id=?1 AND event_type='tool_invocation_completed') \
         FROM tool_invocations WHERE id=?2",
        rusqlite::params![root_run_id, row],
        |r| Ok((r.get(0)?, r.get(1)?)),
    ).unwrap();
    assert_eq!(before, ("running".into(), 0));
    run.finish_tool(Some(row), "inspect_evidence", &arguments, &result, "inv-1-001", 0).unwrap();
    let policy: (String, String) = connection.query_row(
        "SELECT status,policy_decision FROM tool_invocations WHERE id=?1", [row],
        |r| Ok((r.get(0)?, r.get(1)?)),
    ).unwrap();
    assert_eq!(policy, ("refused".into(), "deny".into()));
    assert_eq!(run.finish_tool(Some(row), "inspect_evidence", &arguments, &result, "inv-1-001", 0).unwrap_err(),
        "tool_invocation_not_running_or_missing");
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn scheduler_refuses_reviewer_target_tools_and_wrong_lanes_before_writing() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };

    let (root, db_path, root_run_id, lease) = multi_agent_test_root("role-lane-gate", 100, 3);
    let connection = db::open(&db_path).unwrap();
    for (role, lane, capabilities) in [
        (
            AgentRole::EvidenceReviewer,
            AgentLane::TargetTouching,
            vec!["evidence.read".into(), "review.write".into()],
        ),
        (
            AgentRole::EvidenceReviewer,
            AgentLane::Review,
            vec!["evidence.read".into(), "http_request".into()],
        ),
        (
            AgentRole::SpaApiMapper,
            AgentLane::TargetTouching,
            vec!["evidence.read".into()],
        ),
        (
            AgentRole::SpaApiMapper,
            AgentLane::ReadOnlyAnalysis,
            vec!["browser_action".into()],
        ),
        (
            AgentRole::IdentitySession,
            AgentLane::TargetTouching,
            vec!["evidence.read".into()],
        ),
        (
            AgentRole::IdentitySession,
            AgentLane::ReadOnlyAnalysis,
            vec!["replay_http".into()],
        ),
        (
            AgentRole::DeepInvestigator,
            AgentLane::TargetTouching,
            vec!["replay_http".into()],
        ),
        (
            AgentRole::DeepInvestigator,
            AgentLane::ReadOnlyAnalysis,
            vec!["replay_http".into()],
        ),
        (
            AgentRole::WebExecutor,
            AgentLane::ReadOnlyAnalysis,
            vec!["evidence.read".into()],
        ),
        (
            AgentRole::ExternalSurface,
            AgentLane::Review,
            vec!["evidence.read".into()],
        ),
    ] {
        assert!(
            scheduler::schedule_child(
                &connection,
                &lease,
                role,
                lane,
                "invalid-role-lane",
                &serde_json::json!({"task":"invalid"}),
                1,
                &capabilities,
                10,
                1,
            )
            .is_err(),
            "{role:?} must not schedule in {lane:?} with {capabilities:?}"
        );
    }
    let assignments: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM agent_assignments WHERE coordinator_run_id=?1",
            [&root_run_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(assignments, 0);
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

