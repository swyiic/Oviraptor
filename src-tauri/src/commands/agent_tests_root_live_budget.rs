// Real paid Root tool reads a frozen current-ledger observation, never a grant.
#[test]
fn root_local_live_budget_actual_sdk_observes_each_round_and_replays_original_snapshot() {
    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let n = count.clone();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |_| {
        let body = if n.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            root_local_wire(
                json!([{"id":"budget-observation-0","type":"function","function":{"name":"capability_budget.read","arguments":"{}"}}]),
                "",
                true,
            )
        } else {
            proposal_model_response(&root_tick_valid_text("observed original ledger snapshot"))
        };
        (200, "application/json", body)
    }));
    let f = root_tick_fixture(
        "root-live-budget-sdk",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let db = db::open(&f.context.db_path).unwrap();
    let raw: String = db
        .query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [&f.actor.root_run_id],
            |r| r.get(0),
        )
        .unwrap();
    let result = native_coordinator_tick(&f.context, &f.actor).unwrap();
    let requests = seen.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let wire = |i: usize| -> JsonValue {
        serde_json::from_str(requests[i].split_once("\r\n\r\n").unwrap().1).unwrap()
    };
    let first = wire(0);
    let second = wire(1);
    let basis = |body: &JsonValue| -> JsonValue {
        serde_json::from_str::<JsonValue>(body["messages"][1]["content"].as_str().unwrap()).unwrap()
            ["basis"]
            .clone()
    };
    let original = basis(&first);
    let next = basis(&second);
    assert_eq!(
        original["budgetSnapshot"]["remainingModelRequests"], 20,
        "hard ceilings cannot substitute for an actual pre-dispatch balance"
    );
    assert_eq!(next["budgetSnapshot"]["remainingModelRequests"], 19);
    assert_eq!(original["budgetSnapshot"]["remainingModelTokens"], 60000);
    assert_eq!(next["budgetSnapshot"]["remainingModelTokens"], 59980);
    assert_eq!(
        next["budgetSnapshot"]["dimensions"]["model_requests"]["consumed"],
        1
    );
    assert_eq!(
        original["budgetSnapshot"]["dimensions"]
            .as_object()
            .unwrap()
            .len(),
        10
    );
    assert_eq!(original["budgetSnapshot"]["executionGrant"], false);
    let tools = second["messages"].as_array().unwrap();
    let reply = tools
        .iter()
        .find(|m| m["role"] == "tool" && m["tool_call_id"] == "budget-observation-0")
        .unwrap();
    let value: JsonValue = serde_json::from_str(reply["content"].as_str().unwrap()).unwrap();
    assert_eq!(
        value["budgetSnapshot"], original["budgetSnapshot"],
        "paid local tool returns its original observation, not a later or fabricated balance"
    );
    drop(requests);
    let before = web_mode_test_rows(&db);
    let replay = native_coordinator_tick(&f.context, &f.actor).unwrap();
    assert_eq!(replay.summary, result.summary);
    assert_eq!(replay.event_sequence, result.event_sequence);
    assert_eq!(seen.lock().unwrap().len(), 2);
    web_mode_assert_rows(&db, &before);
    assert_eq!(
        db.query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [&f.actor.root_run_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        raw
    );
    for table in [
        "agent_assignments",
        "agent_capability_leases",
        "agent_http_request_claims",
        "tool_invocations",
    ] {
        assert_eq!(
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
