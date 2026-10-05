// Actual native_model_transport and real localhost HTTP, never an SDK/logger shim.
pub(super) fn sdk_log_rows(db: &rusqlite::Connection) -> Vec<JsonValue> {
    let installed: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='native_sdk_log_owners')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    if !installed {
        return Vec::new();
    }
    db.prepare("SELECT json_object('ownerId',o.owner_id,'domain',o.domain,'dispatchKey',o.dispatch_key,
        'scanId',o.scan_id,'attempt',o.attempt_number,'rootRunId',o.root_run_id,'runId',o.run_id,
        'assignmentId',o.assignment_id,'leaseAttemptId',o.lease_attempt_id,'workerId',o.worker_id,
        'round',o.round_number,'requestHash',o.request_hash,'sequence',r.sequence,'ordinal',r.ordinal,
        'stage',r.stage,'costPhase',r.cost_phase,'terminalState',r.terminal_state)
        FROM native_sdk_log_rows r JOIN native_sdk_log_owners o ON o.owner_id=r.owner_id ORDER BY r.sequence")
        .unwrap().query_map([],|r|r.get::<_,String>(0)).unwrap().map(|r|serde_json::from_str(&r.unwrap()).unwrap()).collect()
}

pub(super) fn sdk_exact_model_body(text: &str) -> String {
    json!({"choices":[{"message":{"role":"assistant","content":text},"finish_reason":"stop"}],
        "usage":{"prompt_tokens":7,"completion_tokens":5,"total_tokens":12,"prompt_tokens_details":{"cached_tokens":2}}}).to_string()
}

#[test]
fn native_sdk_log_single_real_gate_commits_before_reply_and_preserves_exact_result_and_original_cost(
) {
    use std::sync::{mpsc, Arc, Mutex};
    let raw = " \n密码 password=private-model-response Bearer sdk-response-token 窃蛋龙\0 ";
    let body = sdk_exact_model_body(raw);
    let (arrive, arrived) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let released = Arc::new(Mutex::new(released));
    let (port, seen, _stop) = spawn_endpoint(Arc::new(move |_| {
        let _ = arrive.send(());
        let _ = released
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5));
        (200, "application/json", body.clone())
    }));
    let mut h = root_budget_owned_fixture_for_test("sdk-single-live");
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let client = AgentModelClient::new(
        agent_model_profile(&h.context.environment, None).unwrap(),
        &[],
    );
    let db = db::open(&h.db_path).unwrap();
    let run = h.context.run.as_ref().unwrap().run_id.clone();
    let native: String = db
        .query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [&run],
            |r| r.get(0),
        )
        .unwrap();
    let (before, returned) = std::thread::scope(|scope| {
        let (done, rx) = mpsc::channel();
        let ctx = &h.context;
        let client = &client;
        scope.spawn(move || {
            let _ = done.send(native_model_transport(
                ctx,
                client,
                vec![json!({"role":"user","content":"private-sdk-prompt"})],
                &[],
                1,
            ));
        });
        arrived.recv_timeout(Duration::from_secs(5)).unwrap();
        let before = sdk_log_rows(&db);
        assert!(
            rx.try_recv().is_err(),
            "provider gate must still hold the real SDK call"
        );
        let _ = release.send(());
        let returned = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        (before, returned)
    });
    let response = returned.unwrap_or_else(|_| panic!("original SDK result unexpectedly changed"));
    assert_eq!(response.text.as_bytes(), raw.as_bytes());
    assert_eq!(
        (
            response.usage.input_tokens,
            response.usage.cached_input_tokens,
            response.usage.output_tokens,
            response.usage.total_tokens,
            response.usage.model_requests
        ),
        (7, 2, 5, 12, 1)
    );
    assert!(response.usage_reported);
    assert_eq!(
        before
            .iter()
            .map(|r| r["stage"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["prepared", "sent"],
        "live constants must already be committed before provider response"
    );
    assert!(before.iter().all(|r| r["domain"] == "root"
        && r["runId"] == run
        && r["rootRunId"] == run
        && r["workerId"].is_null()
        && r["assignmentId"].is_null()));
    let rows = sdk_log_rows(&db);
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
    let claim:(String,String,String)=db.query_row("SELECT call_id,lease_attempt_id,request_hash FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='dispatch'",[&run],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(rows[0]["dispatchKey"], claim.0);
    assert_eq!(rows[0]["leaseAttemptId"], claim.1);
    assert_eq!(rows[0]["requestHash"], claim.2);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='received'",[&run],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(&db, &run, Some(""), "model_requests")
            .unwrap()
            .consumed,
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [&run],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        native
    );
    let data = json!(rows).to_string();
    for secret in [
        "private-sdk-prompt",
        "private-model-response",
        "sdk-response-token",
        "mock-key",
        "Authorization",
    ] {
        assert!(
            !data.contains(secret),
            "SDK channel must not store model bodies or credentials"
        );
    }
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert!(h.site_seen.lock().unwrap().is_empty());
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}

#[test]
fn native_sdk_log_single_unknown_keeps_one_dispatch_and_original_debt_without_validating() {
    let mut h = root_budget_owned_fixture_for_test("sdk-single-unknown");
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            503,
            "application/json",
            "{\"error\":\"private-provider-body\"}".into(),
        )
    }));
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let client = AgentModelClient::new(
        agent_model_profile(&h.context.environment, None).unwrap(),
        &[],
    );
    assert!(native_model_transport(&h.context, &client, vec![], &[], 1).is_err());
    let db = db::open(&h.db_path).unwrap();
    let root = &h.context.run.as_ref().unwrap().run_id;
    let rows = sdk_log_rows(&db);
    assert_eq!(
        rows.iter()
            .map(|r| r["stage"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "prepared",
            "sent",
            "response_received",
            "cost_saved",
            "terminal"
        ]
    );
    assert_eq!(rows.last().unwrap()["terminalState"], "uncertain");
    assert_eq!(rows.last().unwrap()["costPhase"], "uncertain");
    let cost =
        crate::agent_runtime::multi_agent::budget::balance(&db, root, Some(""), "model_requests")
            .unwrap();
    assert_eq!(
        (cost.reserved, cost.consumed, cost.indeterminate),
        (0, 0, 1)
    );
    let old = sdk_log_rows(&db);
    let original = super::tests::application_table_snapshot(&db);
    assert!(native_model_transport(&h.context, &client, vec![], &[], 1).is_err());
    assert!(super::tests::application_table_snapshot(&db) == original);
    assert_eq!(sdk_log_rows(&db), old);
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert!(!json!(old).to_string().contains("private-provider-body"));
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}

#[test]
fn native_sdk_log_single_actual_scan_stop_commits_uncertain_terminal_before_return_without_regrant()
{
    use std::sync::{mpsc, Arc, Mutex};
    let (arrive, arrived) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let released = Arc::new(Mutex::new(released));
    let (port, seen, _stop) = spawn_endpoint(Arc::new(move |_| {
        let _ = arrive.send(());
        let _ = released
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5));
        (
            200,
            "application/json",
            sdk_exact_model_body("late SDK result"),
        )
    }));
    let mut h = root_budget_owned_fixture_for_test("sdk-single-stop");
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let client = AgentModelClient::new(
        agent_model_profile(&h.context.environment, None).unwrap(),
        &[],
    );
    let db = db::open(&h.db_path).unwrap();
    let (original, after) = std::thread::scope(|scope| {
        let (done, rx) = mpsc::channel();
        let ctx = &h.context;
        let client = &client;
        scope.spawn(move || {
            let _ = done.send(native_model_transport(ctx, client, vec![], &[], 1));
        });
        arrived.recv_timeout(Duration::from_secs(5)).unwrap();
        let original = sdk_log_rows(&db);
        db.execute(
            "UPDATE sentinel_scans SET status='paused' WHERE id=?1",
            [&ctx.scan_id],
        )
        .unwrap();
        let result = rx.recv_timeout(Duration::from_secs(3));
        let after = sdk_log_rows(&db);
        let _ = release.send(());
        assert!(result
            .expect("actual SDK cancel must finish before provider replies")
            .is_err());
        (original, after)
    });
    assert_eq!(
        original
            .iter()
            .map(|r| r["stage"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["prepared", "sent"]
    );
    assert_eq!(after.last().unwrap()["terminalState"], "uncertain");
    assert!(!after.iter().any(|r| r["stage"] == "validated"));
    assert!(after.iter().all(|r| r["ownerId"] == original[0]["ownerId"]
        && r["dispatchKey"] == original[0]["dispatchKey"]));
    let root = &h.context.run.as_ref().unwrap().run_id;
    let cost =
        crate::agent_runtime::multi_agent::budget::balance(&db, root, Some(""), "model_requests")
            .unwrap();
    assert_eq!(
        (cost.reserved, cost.consumed, cost.indeterminate),
        (0, 0, 1)
    );
    assert_eq!(
        db.query_row(
            "SELECT status FROM sentinel_scans WHERE id=?1",
            [&h.context.scan_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "paused"
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    while seen.lock().unwrap().is_empty() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(seen.lock().unwrap().len(), 1);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}
