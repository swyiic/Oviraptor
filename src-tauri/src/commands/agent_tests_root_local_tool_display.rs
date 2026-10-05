// Genuine creator/SDK -> immutable paid local step -> pure chat projection.
#[test]
fn root_decision_chat_local_tools_actual_paid_step_is_named_and_replay_is_readonly() {
    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let n = count.clone();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |_| {
        let body = if n.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            root_local_wire(
                json!([
                    {"id":"display-budget","type":"function","function":{"name":"capability_budget.read","arguments":"{}"}},
                    {"id":"display-snapshot","type":"function","function":{"name":"snapshot.read","arguments":"{}"}}
                ]),
                "PRIVATE_ASSISTANT_DRAFT",
                true,
            )
        } else {
            proposal_model_response(&root_tick_valid_text("public final decision"))
        };
        (200, "application/json", body)
    }));
    let f = root_tick_fixture(
        "local-tools-public-display",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let db = db::open(&f.context.db_path).unwrap();
    let native: String = db
        .query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [&f.actor.root_run_id],
            |r| r.get(0),
        )
        .unwrap();
    native_coordinator_tick(&f.context, &f.actor).unwrap();
    assert_eq!(seen.lock().unwrap().len(), 2);
    let before = web_mode_test_rows(&db);
    let status = native_scan_status(&db, &f.context.scan_id).unwrap();
    let rows: Vec<_> = status["timeline"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["eventType"] == "root_decision")
        .collect();
    assert_eq!(rows.len(), 2);
    let local = rows
        .iter()
        .find(|r| r["decisionRecord"]["round"] == 1)
        .unwrap();
    assert_eq!(
        local["decisionRecord"]["localTools"],
        json!(["capability_budget.read", "snapshot.read"])
    );
    assert_eq!(local["decisionRecord"].as_object().unwrap().len(), 8);
    let final_row = rows
        .iter()
        .find(|r| r["decisionRecord"]["round"] == 2)
        .unwrap();
    assert_eq!(final_row["decisionRecord"].as_object().unwrap().len(), 7);
    for forbidden in [
        "PRIVATE_ASSISTANT_DRAFT",
        "display-budget",
        "display-snapshot",
        "arguments",
        "budgetSnapshot",
        "localStep",
    ] {
        assert!(
            !local["decisionRecord"].to_string().contains(forbidden),
            "{forbidden}"
        );
    }
    let history = native_scan_timeline_page(
        &db,
        &f.context.scan_id,
        1,
        status["latestSequence"].as_i64().unwrap() + 1,
    )
    .unwrap();
    assert!(history["timeline"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r == *local));
    assert_eq!(native_scan_status(&db, &f.context.scan_id).unwrap(), status);
    assert!(
        native_coordinator_tick(&f.context, &f.actor)
            .unwrap()
            .replayed
    );
    web_mode_assert_rows(&db, &before);
    assert_eq!(seen.lock().unwrap().len(), 2);
    assert_eq!(
        db.query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [&f.actor.root_run_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        native
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM agent_http_request_claims", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap(),
        0
    );
}
