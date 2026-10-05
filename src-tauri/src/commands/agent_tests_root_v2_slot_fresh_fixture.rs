// Preserve the original test Native limits/evidence; only creation becomes real.
fn root_v2_slot_fresh_harness(tag: &str, batches: i64) -> AgentHarness {
    let (site_port, site_seen, _) = spawn_endpoint(std::sync::Arc::new(mock_site));
    let (model_port, _, model_seen) = spawn_model(vec![]);
    let root = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("oviraptor-fresh-v2-{tag}-{}", Uuid::new_v4()));
    let db_path = db::initialize(&root).unwrap();
    let mut db = db::open(&db_path).unwrap();
    db.execute(
        "INSERT INTO projects(id,name) VALUES(9001,'Fresh v2 fixture')",
        [],
    )
    .unwrap();
    let target = format!("http://127.0.0.1:{site_port}");
    let scan = with_web_creator_test_id("agent-scan", || {
        web_mode_test_draft(&db, Some("multi"), &target).unwrap()
    });
    let work = web_mode_test_start(&root, &mut db, &scan.id, WebStartMode::Confirm).unwrap();
    // Same fallback plan/evidence as agent_harness, before any Root exists.
    let mut context = test_context(&db_path, &target, vec![AgentIdentity::anonymous()]);
    let original_native = context.execution_plan.as_json().to_string();
    let original_evidence = context.evidence.to_string();
    context.environment.api_base = format!("http://127.0.0.1:{model_port}/v1");
    context.environment.api_key = "mock-key".into();
    context.target_dir = work.join("url-pipeline/target-00001");
    context.log_path = work.join("runner.log");
    fs::create_dir_all(&context.target_dir).unwrap();
    fs::write(context.target_dir.join(".oviraptor-scan-id"), &scan.id).unwrap();
    fs::write(
        context.target_dir.join("frontend-evidence.json"),
        &original_evidence,
    )
    .unwrap();
    let proof = private_web_mode_on(&db, &scan.id, 1).unwrap();
    let mode = crate::agent_runtime::web_mode::root::NewRootModeDeclaration::from_verified(
        &proof,
        &target,
        &context.execution_plan.hash(),
    )
    .unwrap();
    let budget = root_v2_slot_declaration(&context, batches);
    crate::agent_runtime::store::record_attempt_plan_with_declarations(
        &db,
        &scan.id,
        1,
        &target,
        context.execution_plan.backend,
        &context.execution_plan.hash(),
        &context.execution_plan.as_json(),
        Some(&budget),
        Some(&mode),
    )
    .unwrap();
    context.run = Some(runtime_open_run(&db_path, &scan.id, &context.route).unwrap());
    bind_agent_evidence_location(&context).unwrap();
    let root_id = &context.run.as_ref().unwrap().run_id;
    assert_eq!(
        native_frozen_web_root_mode(&context).unwrap(),
        crate::agent_runtime::web_mode::WebMode::Multi
    );
    assert_eq!(
        db.query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [root_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        original_native
    );
    assert_eq!(
        db.query_row(
            "SELECT work_dir FROM sentinel_scan_attempts WHERE scan_id=?1 AND attempt_number=1",
            [&scan.id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        work.to_string_lossy().to_string()
    );
    assert_eq!(
        fs::read_to_string(context.target_dir.join("frontend-evidence.json")).unwrap(),
        original_evidence
    );
    AgentHarness {
        root,
        db_path,
        context,
        site_seen,
        model_seen,
    }
}

fn root_v2_slot_retarget_model(h: &mut AgentHarness, script: Vec<String>) {
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
    let next_child = Arc::new(AtomicUsize::new(0));
    let (port, seen, _) = spawn_endpoint(Arc::new(move |request| {
        if !request.contains("POST /v1/chat/completions") {
            return (404, "text/plain", "unexpected SDK path".into());
        }
        let wire: JsonValue = serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        let system = wire["messages"][0]["content"].as_str().unwrap_or_default();
        let response = if system.contains("You are the Root Coordinator") {
            proposal_model_response(&json!({"schemaVersion":1,
                "observed":["frozen local evidence"],"missing":[],"suggestions":if request.contains("mapper-output") {vec!["dispatch:web_executor"]} else {vec![]},
                "costNotes":[],"risks":[]}).to_string())
        } else {
            let index = next_child.fetch_add(1, Ordering::SeqCst);
            script.get(index).cloned().unwrap_or_else(|| model_round(&[], 10))
        };
        (200, "application/json", response)
    }));
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    h.model_seen = seen;
}
