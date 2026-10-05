// Ordinary fresh creator -> original Root/C -> actual SDK local tool -> final.
#[test]
fn root_local_react_actual_snapshot_tool_continues_with_original_paid_calls() {
    let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = counter.clone();
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(move |_| {
        let turn = count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let body = if turn == 0 {
            json!({"choices":[{"message":{"role":"assistant","tool_calls":[{"id":"local-snapshot-0","type":"function","function":{"name":"snapshot.read","arguments":"{}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}}).to_string()
        } else {
            proposal_model_response(&root_tick_valid_text("checked the frozen local snapshot"))
        };
        (200, "application/json", body)
    }));
    let f = root_tick_fixture(
        "root-local-react-sdk",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let db = db::open(&f.context.db_path).unwrap();
    let root = &f.actor.root_run_id;
    let raw_plan: String = db
        .query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [root],
            |r| r.get(0),
        )
        .unwrap();
    let original: (String, String) = db
        .query_row(
            "SELECT id,contract_json FROM agent_root_budget_attempts WHERE root_run_id=?1",
            [root],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let result = native_coordinator_tick(&f.context, &f.actor).unwrap_or_else(|e| {
        panic!("actual local tool must lead to a second original paid SDK call: {e}")
    });
    assert!(result.summary.as_json()["observed"]
        .to_string()
        .contains("checked the frozen local snapshot"));
    assert_eq!(seen.lock().unwrap().len(), 2);
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(&db, root, None, "model_requests")
            .unwrap()
            .consumed,
        2
    );
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='received'",[root],|r|r.get::<_,i64>(0)).unwrap(),2);
    let requests = seen.lock().unwrap();
    for wire in requests.iter() {
        let body: JsonValue = serde_json::from_str(wire.split_once("\r\n\r\n").unwrap().1).unwrap();
        let names = body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["function"]["name"].as_str().unwrap())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            names,
            [
                "snapshot.read",
                "evidence.read",
                "capability_budget.read",
                "plan.propose"
            ]
            .into_iter()
            .collect()
        );
    }
    let next: JsonValue =
        serde_json::from_str(requests[1].split_once("\r\n\r\n").unwrap().1).unwrap();
    assert!(next["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["role"] == "tool" && m["tool_call_id"] == "local-snapshot-0"));
    drop(requests);
    assert_eq!(
        db.query_row(
            "SELECT id,contract_json FROM agent_root_budget_attempts WHERE root_run_id=?1",
            [root],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        )
        .unwrap(),
        original
    );
    assert_eq!(
        db.query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [root],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        raw_plan
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
            0,
            "local reasoning does not execute a target or issue a worker"
        );
    }
    let before = web_mode_test_rows(&db);
    let replay = native_coordinator_tick(&f.context, &f.actor).unwrap();
    assert_eq!(replay.summary, result.summary);
    assert_eq!(replay.event_sequence, result.event_sequence);
    assert_eq!(
        seen.lock().unwrap().len(),
        2,
        "paid local steps and final response must not be replayed"
    );
    web_mode_assert_rows(&db, &before);
}

#[test]
fn root_local_react_actual_tool_only_model_stops_at_frozen_three_call_bound() {
    let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = counter.clone();
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(move |_| {
        let turn = count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        (200,"application/json",json!({"choices":[{"message":{"role":"assistant","tool_calls":[{"id":format!("local-snapshot-{turn}"),"type":"function","function":{"name":"snapshot.read","arguments":"{}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}}).to_string())
    }));
    let f = root_tick_fixture(
        "root-local-react-bounded",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let db = db::open(&f.context.db_path).unwrap();
    let root = &f.actor.root_run_id;
    let original: (String, String) = db
        .query_row(
            "SELECT id,contract_json FROM agent_root_budget_attempts WHERE root_run_id=?1",
            [root],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let error = native_coordinator_tick(&f.context, &f.actor)
        .err()
        .expect("tool-only output cannot fake a final decision");
    assert_eq!(error, "root_local_react_round_limit");
    assert_eq!(seen.lock().unwrap().len(), 3);
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(&db, root, None, "model_requests")
            .unwrap()
            .consumed,
        3
    );
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='received'",[root],|r|r.get::<_,i64>(0)).unwrap(),3);
    assert_eq!(
        db.query_row("SELECT status FROM agent_runs WHERE id=?1", [root], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        "running",
        "a bounded local decision is not Root completion"
    );
    assert_eq!(
        db.query_row(
            "SELECT id,contract_json FROM agent_root_budget_attempts WHERE root_run_id=?1",
            [root],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        )
        .unwrap(),
        original
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM agent_assignments", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let before = web_mode_test_rows(&db);
    assert_eq!(
        native_coordinator_tick(&f.context, &f.actor).err().unwrap(),
        error
    );
    assert_eq!(seen.lock().unwrap().len(), 3);
    web_mode_assert_rows(&db, &before);
}
