// Positive fixtures use an actual new creator and keep its signed work_dir.
fn fresh_multi_production_harness(tag: &str) -> AgentHarness {
    let (site_port, site_seen, _) = spawn_endpoint(std::sync::Arc::new(mock_site));
    let (model_port, model_seen, _) = spawn_endpoint(std::sync::Arc::new(|_| {
        (200, "application/json", model_round(&[], 10))
    }));
    let root = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("oviraptor-new-multi-{tag}-{}", Uuid::new_v4()));
    let db_path = db::initialize(&root).unwrap();
    let mut db = db::open(&db_path).unwrap();
    db.execute(
        "INSERT INTO projects(id,name) VALUES(9001,'Fresh Multi fixture')",
        [],
    )
    .unwrap();
    let target = format!("http://127.0.0.1:{site_port}");
    let scan = with_web_creator_test_id("agent-scan", || {
        web_mode_test_draft(&db, Some("multi"), &target).unwrap()
    });
    let work = web_mode_test_start(&root, &mut db, &scan.id, WebStartMode::Confirm).unwrap();
    let mut context = test_context(&db_path, &target, vec![AgentIdentity::anonymous()]);
    context.environment.api_base = format!("http://127.0.0.1:{model_port}/v1");
    context.environment.api_key = "mock-key".into();
    context.target_dir = work.join("url-pipeline/target-00001");
    context.log_path = work.join("runner.log");
    fs::create_dir_all(&context.target_dir).unwrap();
    fs::write(context.target_dir.join(".oviraptor-scan-id"), &scan.id).unwrap();
    fs::write(
        context.target_dir.join("frontend-evidence.json"),
        context.evidence.to_string(),
    )
    .unwrap();
    context.execution_plan = build_agent_execution_plan(
        &AgentBudgetSettings::from_json(&serde_json::json!({})),
        &context.route,
        &context.environment,
        AgentBackendKind::Native,
        &db_path,
        &scan.id,
    )
    .with_attempt(1);
    AgentHarness {
        root,
        db_path,
        context,
        site_seen,
        model_seen,
    }
}

fn freeze_fresh_multi_production_harness(h: &mut AgentHarness) {
    let c = &mut h.context;
    persist_frozen_web_execution_plan(&h.db_path, &c.scan_id, 1, &c.target_url, &c.execution_plan)
        .unwrap();
    c.run = Some(runtime_open_run(&h.db_path, &c.scan_id, &c.route).unwrap());
    bind_agent_evidence_location(c).unwrap();
    assert_eq!(
        native_frozen_web_root_mode(c).unwrap(),
        crate::agent_runtime::web_mode::WebMode::Multi
    );
}

fn fresh_multi_root_wire_response(request: &str) -> Option<String> {
    request.contains("You are the Root Coordinator").then(|| {
        proposal_model_response(
            &serde_json::json!({"schemaVersion":1,"observed":["frozen local frontend evidence"],
            "missing":[],"suggestions":if human_root_wire_input(request).is_some() {vec!["assess:human_directive"]} else if request.contains("budget-allocation") {vec!["assess:budget_allocation"]} else if request.contains("mapper-output") {vec!["dispatch:web_executor"]} else if request.contains("bootstrapDispatch") {vec!["dispatch:spa_api_mapper"]} else {vec![]},"costNotes":[],"risks":[]})
            .to_string(),
        )
    })
}

// Final elapsed sampling may append truthful wall_time_ms receipts on failure.
// This snapshot protects original model/child financial authority, not time.
fn fresh_multi_model_financial_rows(
    db: &rusqlite::Connection,
) -> Vec<(String, Vec<Vec<rusqlite::types::Value>>)> {
    [
        "agent_root_budget_attempts",
        "agent_budget_limits",
        "agent_budget_clock_origins",
        "agent_budget_ledger",
        "agent_budget_entries",
    ]
    .into_iter()
    .map(|table| {
        let condition = if table == "agent_budget_entries" {
            " WHERE dimension<>'wall_time_ms'"
        } else {
            ""
        };
        let mut q = db
            .prepare(&format!("SELECT * FROM {table}{condition} ORDER BY rowid"))
            .unwrap();
        let count = q.column_count();
        let rows = q
            .query_map([], |r| {
                (0..count)
                    .map(|i| r.get(i))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        (table.into(), rows)
    })
    .collect()
}

fn fresh_multi_wire_system(raw: &str) -> String {
    let request: JsonValue = serde_json::from_str(raw.split_once("\r\n\r\n").unwrap().1).unwrap();
    request["messages"][0]["content"]
        .as_str()
        .unwrap_or_default()
        .into()
}
