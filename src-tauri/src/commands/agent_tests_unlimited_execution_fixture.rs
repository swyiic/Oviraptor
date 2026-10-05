// Current creation-only Native contract, actual local SDK transport and original Broker.
fn unlimited_execution_fixture(limits: (i64, i64), reply: u8) -> BudgetTriggerFixture {
    let mut h = fresh_multi_production_harness("unlimited-execution");
    let target = h.context.target_url.clone();
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(move |wire| {
        if wire.contains("You are the Root Coordinator") && wire.contains("client-side-output") {
            return (200,"application/json",proposal_model_response(&json!({"schemaVersion":1,"observed":["paid original readonly configuration"],"missing":[],"suggestions":["assess:client_side_configuration"],"costNotes":[],"risks":[]}).to_string()));
        }
        if reply == 4
            && wire.contains("You are the Root Coordinator")
            && wire.contains("budget-allocation")
        {
            return (200,"application/json",proposal_model_response(&json!({"schemaVersion":1,"observed":[],"missing":[],"suggestions":["dispatch:web_executor"],"costNotes":["free_model_calls=true"],"risks":[]}).to_string()));
        }
        if let Some(response) = fresh_multi_root_wire_response(&wire) {
            if reply == 1 && wire.contains("budget-allocation") {
                return (
                    503,
                    "application/json",
                    json!({"error":{"message":"temporary budget assessment failure"}}).to_string(),
                );
            }
            return (200, "application/json", response);
        }
        if wire.contains("SPA/API Mapper") {
            return (
                200,
                "application/json",
                proposal_model_response(
                    r#"{"summary":"original frozen map","priorityContracts":[],"risks":[]}"#,
                ),
            );
        }
        if wire.contains("独立ClientSide配置事实专家") {
            let body: JsonValue =
                serde_json::from_str(wire.split_once("\r\n\r\n").unwrap().1).unwrap();
            let input: JsonValue =
                serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
            let refs: Vec<_> = input["observations"]
                .as_array()
                .unwrap()
                .iter()
                .map(|o| o["id"].clone())
                .collect();
            return (200,"application/json",proposal_model_response(&json!({"summary":"original readonly HTTP configuration; no impact proof","observationRefs":refs,"gaps":["missing_browser_validation"],"candidates":[]}).to_string()));
        }
        if reply == 2 {
            return (
                503,
                "application/json",
                json!({"error":{"message":"temporary Web executor failure"}}).to_string(),
            );
        }
        let response = if calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            model_round(
                &[(
                    "replay_http",
                    json!({"identity":"anonymous","method":"GET","url":format!("{target}/"),"family":"authorization"}),
                )],
                10,
            )
        } else {
            model_round(&[], 10)
        };
        (200, "application/json", response)
    }));
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    h.model_seen = seen;
    h.context.execution_plan.hard_total_tokens = limits.0;
    h.context.execution_plan.soft_uncached_tokens = limits.0.min(30000);
    h.context.execution_plan.hard_model_requests = limits.1;
    h.context.execution_plan.soft_model_requests = limits.1.min(10);
    h.context.execution_plan.max_turns = 8;
    freeze_fresh_multi_production_harness(&mut h);
    let session = multi_agent_prepare(&mut h.context)
        .expect("current Native unlimited creation must not be rejected as unknown model cost");
    BudgetTriggerFixture {
        h,
        session: Some(session),
        boundary: std::sync::Arc::new(std::sync::Mutex::new(None)),
        stop,
    }
}
