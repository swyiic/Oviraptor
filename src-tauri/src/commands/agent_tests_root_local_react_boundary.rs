// Every response below crosses the actual SDK; rejected outputs keep their bill.
fn root_local_wire(calls: JsonValue, text: &str, usage: bool) -> String {
    let mut response = json!({"choices":[{"message":{"role":"assistant","content":text,"tool_calls":calls},"finish_reason":"tool_calls"}]});
    if usage {
        response["usage"] = json!({"prompt_tokens":10,"completion_tokens":10,"total_tokens":20});
    }
    response.to_string()
}
fn root_local_call(id: &str, name: &str, args: JsonValue) -> JsonValue {
    json!({"id":id,"type":"function","function":{"name":name,"arguments":args.to_string()}})
}
#[test]
fn root_local_react_actual_forbidden_tools_arguments_proposals_and_duplicates_keep_one_bill() {
    let marker = "PRIVATE_LOCAL_ARGUMENT_DO_NOT_STORE";
    for calls in [
        json!([root_local_call(
            "call-denied",
            "http.request",
            json!({"url":"https://target.invalid"})
        )]),
        json!([root_local_call(
            "call-path",
            "snapshot.read",
            json!({"path":marker})
        )]),
        json!([root_local_call(
            "call-plan",
            "plan.propose",
            json!({"step":"dispatch:concurrency_tester"})
        )]),
        json!([
            root_local_call("call-duplicate", "snapshot.read", json!({})),
            root_local_call("call-duplicate", "evidence.read", json!({}))
        ]),
        json!((0..5)
            .map(|i| root_local_call(&format!("call-{i}"), "snapshot.read", json!({})))
            .collect::<Vec<_>>()),
    ] {
        let body = root_local_wire(calls, "PRIVATE_ROOT_DRAFT_DO_NOT_STORE", true);
        let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |_| {
            (200, "application/json", body.clone())
        }));
        let f = root_tick_fixture("root-local-reject", &format!("http://127.0.0.1:{port}/v1"));
        let db = db::open(&f.context.db_path).unwrap();
        let error = native_coordinator_tick(&f.context, &f.actor).err().unwrap();
        assert!(error.starts_with("root_local_"), "{error}");
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "decision"), 0);
        assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 0);
        assert_eq!(
            crate::agent_runtime::multi_agent::budget::balance(
                &db,
                &f.actor.root_run_id,
                None,
                "model_requests"
            )
            .unwrap()
            .consumed,
            1
        );
        let before = web_mode_test_rows(&db);
        let all = format!("{before:?}");
        assert!(!all.contains(marker));
        assert!(!all.contains("PRIVATE_ROOT_DRAFT_DO_NOT_STORE"));
        assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
        assert_eq!(seen.lock().unwrap().len(), 1);
        web_mode_assert_rows(&db, &before);
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
}
#[test]
fn root_local_react_actual_four_read_and_proposal_tools_ignore_private_text_and_finish() {
    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = count.clone();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |_| {
        let turn = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let body = if turn == 0 {
            root_local_wire(
                json!([
                    root_local_call("call-snapshot", "snapshot.read", json!({})),
                    root_local_call("call-evidence", "evidence.read", json!({})),
                    root_local_call("call-budget", "capability_budget.read", json!({})),
                    root_local_call("call-proposal", "plan.propose", json!({"step":"defer"})),
                ]),
                "PRIVATE_VALID_TOOL_DRAFT_DO_NOT_STORE",
                true,
            )
        } else {
            proposal_model_response(&root_tick_valid_text(
                "finished after four actual local tools",
            ))
        };
        (200, "application/json", body)
    }));
    let f = root_tick_fixture(
        "root-local-all-tools",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let db = db::open(&f.context.db_path).unwrap();
    let receipt = native_coordinator_tick(&f.context, &f.actor).unwrap();
    assert!(receipt.saved.local_step.is_none());
    assert_eq!(seen.lock().unwrap().len(), 2);
    let requests = seen.lock().unwrap();
    let request: JsonValue =
        serde_json::from_str(requests[1].split_once("\r\n\r\n").unwrap().1).unwrap();
    let messages = request["messages"].as_array().unwrap();
    assert_eq!(messages.iter().filter(|m| m["role"] == "tool").count(), 4);
    let budget: JsonValue = serde_json::from_str(
        messages
            .iter()
            .find(|m| m["tool_call_id"] == "call-budget")
            .unwrap()["content"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(budget["originalHardLimits"]["model_requests"], 20);
    assert_eq!(budget["targetGrant"], false);
    assert!(
        budget.get("availableRequests").is_none(),
        "hard ceiling must not be a fabricated remaining balance"
    );
    let proposal: JsonValue = serde_json::from_str(
        messages
            .iter()
            .find(|m| m["tool_call_id"] == "call-proposal")
            .unwrap()["content"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(proposal["dispatched"], false);
    assert!(!request
        .to_string()
        .contains("PRIVATE_VALID_TOOL_DRAFT_DO_NOT_STORE"));
    drop(requests);
    let before = web_mode_test_rows(&db);
    assert!(!format!("{before:?}").contains("PRIVATE_VALID_TOOL_DRAFT_DO_NOT_STORE"));
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 2);
    assert_eq!(
        db.query_row("SELECT count(*) FROM agent_assignments", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let replay = native_coordinator_tick(&f.context, &f.actor).unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.event_sequence, receipt.event_sequence);
    assert_eq!(seen.lock().unwrap().len(), 2);
    web_mode_assert_rows(&db, &before);
}
#[test]
fn root_local_react_actual_unreported_fee_never_enters_second_model_round() {
    let body = root_local_wire(
        json!([root_local_call("call-unknown", "snapshot.read", json!({}))]),
        "",
        false,
    );
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |_| {
        (200, "application/json", body.clone())
    }));
    let f = root_tick_fixture("root-local-unknown", &format!("http://127.0.0.1:{port}/v1"));
    let db = db::open(&f.context.db_path).unwrap();
    assert_eq!(
        native_coordinator_tick(&f.context, &f.actor).err().unwrap(),
        "budget_indeterminate_requires_reconciliation"
    );
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 0);
    assert!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &f.actor.root_run_id,
            None,
            "model_input_tokens"
        )
        .unwrap()
        .indeterminate
            > 0
    );
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &f.actor.root_run_id,
            None,
            "model_requests"
        )
        .unwrap()
        .consumed,
        1
    );
    let before = web_mode_test_rows(&db);
    assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
    assert_eq!(seen.lock().unwrap().len(), 1);
    web_mode_assert_rows(&db, &before);
}
