#[test]
fn failed_surface_inventory_write_keeps_endpoint_retryable() {
    let (_root, db_path) = temp_database("surface-inventory-retry");
    seed_scan(&db_path, "agent-scan", "scanning");
    let context = test_context(
        &db_path,
        "https://app.example.invalid",
        vec![AgentIdentity::anonymous()],
    );
    let connection = db::open(&db_path).unwrap();
    connection.execute_batch(
        "CREATE TRIGGER deny_inventory BEFORE INSERT ON sentinel_findings \
         WHEN NEW.stage='frontend-recon' AND NEW.kind='api' \
         BEGIN SELECT RAISE(ABORT,'injected inventory failure'); END;",
    ).unwrap();
    let mut runtime = AgentToolRuntime::default();
    assert!(persist_agent_surface_api(&context, &mut runtime, "GET", "/api/orders", "observed-request").is_err());
    assert!(runtime.discovered_api_keys.is_empty(), "a failed write must not claim discovery");
    assert!(runtime.pending_api_queue.is_empty(), "a failed write must not enqueue a phantom endpoint");
    connection.execute_batch("DROP TRIGGER deny_inventory").unwrap();
    assert!(persist_agent_surface_api(&context, &mut runtime, "GET", "/api/orders", "observed-request").unwrap());
    assert!(!persist_agent_surface_api(&context, &mut runtime, "GET", "/api/orders", "observed-request").unwrap());
    assert_eq!(runtime.drain_pending_apis(), vec!["GET|/api/orders"]);
    assert_eq!(findings_for(&db_path, "frontend-recon").len(), 1);
}

#[test]
fn observed_nonstatic_request_reports_inventory_failure_after_http_receipt() {
    let harness = agent_harness(
        "observed-inventory-failure",
        |_| (200, "application/json", "{\"ok\":true}".into()),
        vec![AgentIdentity::anonymous()],
    );
    let connection = db::open(&harness.db_path).unwrap();
    connection.execute_batch(
        "CREATE TRIGGER deny_inventory BEFORE INSERT ON sentinel_findings \
         WHEN NEW.stage='frontend-recon' AND NEW.kind='api' \
         BEGIN SELECT RAISE(ABORT,'injected inventory failure'); END;",
    ).unwrap();
    let mut runtime = AgentToolRuntime::default();
    let result = agent_execute_tool(
        &harness.context,
        &mut runtime,
        "replay_http",
        &serde_json::json!({
            "identity":"anonymous", "method":"GET",
            "url":format!("{}/api/orders", harness.context.target_url),
            "family":"business_flow",
        }),
    );
    assert_eq!(result.model_view["code"], "finding_persist_failed");
    assert_eq!(runtime.target_requests, 1, "the request was sent and must remain charged");
    assert_eq!(harness.site_seen.lock().unwrap().len(), 1, "failure must not replay HTTP");
    assert!(runtime.discovered_api_keys.is_empty());
    assert!(findings_for(&harness.db_path, "frontend-recon").is_empty());
}

#[test]
fn ignored_inventory_insert_is_not_a_discovered_api() {
    let (_root, db_path) = temp_database("surface-inventory-ignored");
    seed_scan(&db_path, "agent-scan", "scanning");
    let context = test_context(
        &db_path,
        "https://app.example.invalid",
        vec![AgentIdentity::anonymous()],
    );
    let connection = db::open(&db_path).unwrap();
    connection.execute_batch(
        "CREATE TRIGGER ignore_inventory BEFORE INSERT ON sentinel_findings \
         WHEN NEW.stage='frontend-recon' AND NEW.kind='api' \
         BEGIN SELECT RAISE(IGNORE); END;",
    ).unwrap();
    let mut runtime = AgentToolRuntime::default();
    assert!(persist_agent_surface_api(&context, &mut runtime, "GET", "/api/orders", "observed-request").is_err());
    assert!(runtime.discovered_api_keys.is_empty());
    assert!(runtime.pending_api_queue.is_empty());
    assert!(findings_for(&db_path, "frontend-recon").is_empty());
    connection.execute_batch("DROP TRIGGER ignore_inventory").unwrap();
    assert!(persist_agent_surface_api(&context, &mut runtime, "GET", "/api/orders", "observed-request").unwrap());
    assert_eq!(runtime.drain_pending_apis(), vec!["GET|/api/orders"]);
}

#[test]
fn mutated_inventory_insert_rolls_back_without_claiming_discovery() {
    let (_root, db_path) = temp_database("surface-inventory-mutated-insert");
    seed_scan(&db_path, "agent-scan", "scanning");
    let context = test_context(
        &db_path,
        "https://app.example.invalid",
        vec![AgentIdentity::anonymous()],
    );
    let connection = db::open(&db_path).unwrap();
    connection.execute_batch(
        "CREATE TRIGGER corrupt_inventory AFTER INSERT ON sentinel_findings \
         WHEN NEW.stage='frontend-recon' AND NEW.kind='api' \
         BEGIN UPDATE sentinel_findings SET title='corrupted' WHERE id=NEW.id; END;",
    ).unwrap();
    let mut runtime = AgentToolRuntime::default();
    assert!(persist_agent_surface_api(&context, &mut runtime, "GET", "/api/orders", "observed-request").is_err());
    assert!(runtime.discovered_api_keys.is_empty());
    assert!(runtime.pending_api_queue.is_empty());
    assert!(findings_for(&db_path, "frontend-recon").is_empty(), "the mutated write must roll back");
    connection.execute_batch("DROP TRIGGER corrupt_inventory").unwrap();
    assert!(persist_agent_surface_api(&context, &mut runtime, "GET", "/api/orders", "observed-request").unwrap());
}

#[test]
fn mutated_inventory_upsert_preserves_prior_evidence() {
    let (_root, db_path) = temp_database("surface-inventory-mutated-update");
    seed_scan(&db_path, "agent-scan", "scanning");
    let context = test_context(
        &db_path,
        "https://app.example.invalid",
        vec![AgentIdentity::anonymous()],
    );
    let mut runtime = AgentToolRuntime::default();
    assert!(persist_agent_surface_api(&context, &mut runtime, "GET", "/api/orders", "observed-request").unwrap());
    let original: (String, String) = connection_inventory_record(&db_path);
    let connection = db::open(&db_path).unwrap();
    connection.execute_batch(
        "CREATE TRIGGER corrupt_inventory_update AFTER UPDATE ON sentinel_findings \
         WHEN NEW.stage='frontend-recon' AND NEW.kind='api' AND NEW.severity!='corrupted' \
         BEGIN UPDATE sentinel_findings SET severity='corrupted' WHERE id=NEW.id; END;",
    ).unwrap();
    let mut another_runtime = AgentToolRuntime::default();
    assert!(persist_agent_surface_api(&context, &mut another_runtime, "GET", "/api/orders", "source-derived").is_err());
    assert!(another_runtime.discovered_api_keys.is_empty());
    assert!(another_runtime.pending_api_queue.is_empty());
    assert_eq!(connection_inventory_record(&db_path), original, "the prior evidence must survive the failed upsert");
}

fn connection_inventory_record(db_path: &std::path::Path) -> (String, String) {
    db::open(db_path).unwrap().query_row(
        "SELECT severity,record_json FROM sentinel_findings WHERE stage='frontend-recon' AND kind='api'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap()
}

#[test]
fn finding_savepoint_rolls_back_only_the_mutated_write_inside_publication() {
    let (_root, db_path) = temp_database("finding-nested-publication");
    seed_scan(&db_path, "agent-scan", "scanning");
    let connection = db::open(&db_path).unwrap();
    let publication = rusqlite::Transaction::new_unchecked(
        &connection,
        rusqlite::TransactionBehavior::Immediate,
    ).unwrap();
    let original = serde_json::json!({"source":"first"});
    insert_finding(&publication, "agent-scan", "https://app.example.invalid",
        "frontend-recon", "api", "nested", "original", "info", &original).unwrap();
    publication.execute_batch(
        "CREATE TRIGGER mutate_nested AFTER UPDATE ON sentinel_findings \
         WHEN NEW.record_key='nested' AND NEW.severity!='corrupted' \
         BEGIN UPDATE sentinel_findings SET severity='corrupted' WHERE id=NEW.id; END;",
    ).unwrap();
    assert_eq!(insert_finding(&publication, "agent-scan", "https://app.example.invalid",
        "frontend-recon", "api", "nested", "replacement", "medium",
        &serde_json::json!({"source":"second"})).unwrap_err(),
        "sentinel_finding_write_mismatch");
    let unchanged: (String, String, String) = publication.query_row(
        "SELECT title,severity,record_json FROM sentinel_findings WHERE record_key='nested'",
        [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).unwrap();
    assert_eq!(unchanged, ("original".into(), "info".into(), original.to_string()));
    publication.commit().unwrap();
    assert_eq!(connection_inventory_record(&db_path), ("info".into(), original.to_string()));
}
