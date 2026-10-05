#[test]
fn builtin_src_adapter_stages_tools_and_records_oast_callbacks() {
    let root = std::env::temp_dir().join(format!("oviraptor-src-adapter-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("frontend-evidence.json"), "{\"schemaVersion\":1}").unwrap();
    let receiver = stage_builtin_src_assurance("http://127.0.0.1", &root).unwrap();
    assert!(root.join(SRC_ASSURANCE_ADAPTER_NAME).is_file());
    let capabilities = json(fs::read_to_string(root.join("src-capabilities.json")).unwrap());
    assert_eq!(
        capabilities
            .pointer("/adapter/rawHttp/available")
            .and_then(JsonValue::as_bool),
        Some(false)
    );
    let callback = reqwest::blocking::get(&receiver.base_url).unwrap();
    assert_eq!(callback.status().as_u16(), 204);
    let events = serde_json::from_str::<JsonValue>(
        &reqwest::blocking::get(&receiver.poll_url)
            .unwrap()
            .text()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(events.as_array().map(Vec::len), Some(1));
    assert_eq!(
        events
            .pointer("/0/tokenMatched")
            .and_then(JsonValue::as_bool),
        Some(true)
    );
    drop(receiver);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn src_adapter_target_parser_keeps_exact_authority() {
    assert_eq!(
        target_host_port("https://example.invalid:8443/path"),
        Some(("example.invalid".into(), 8443))
    );
    assert_eq!(
        target_host_port("http://127.0.0.1/api"),
        Some(("127.0.0.1".into(), 80))
    );
}

#[test]
fn source_inventory_detects_tauri_vue_typescript_and_rust_from_repository() {
    let root = std::env::temp_dir().join(format!("asset-atlas-source-{}", Uuid::new_v4()));
    fs::create_dir_all(root.join("src-tauri/src")).unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("package.json"),
        r#"{"dependencies":{"vue":"^3"},"devDependencies":{"vite":"^6","typescript":"^5"}}"#,
    )
    .unwrap();
    fs::write(
        root.join("src-tauri/Cargo.toml"),
        "[dependencies]\ntauri = \"2\"\n",
    )
    .unwrap();
    fs::write(root.join("src-tauri/tauri.conf.json"), "{}").unwrap();
    fs::write(root.join("src-tauri/src/lib.rs"), "pub fn run() {}\n").unwrap();
    fs::write(root.join("src/App.vue"), "<template><main /></template>\n").unwrap();
    fs::write(
        root.join("src/main.ts"),
        "// bootstrap application\n\nexport const app = true; // inline comments count as code\n",
    )
    .unwrap();

    let inventory = inspect_source_tree(&root);
    assert_eq!(
        inventory.get("architecture").and_then(JsonValue::as_str),
        Some("Tauri desktop application")
    );
    let languages = inventory
        .get("languages")
        .and_then(JsonValue::as_array)
        .unwrap();
    for expected in ["Rust", "TypeScript", "Vue SFC"] {
        assert!(languages
            .iter()
            .any(|item| item.get("name").and_then(JsonValue::as_str) == Some(expected)));
    }
    assert!(!languages
        .iter()
        .any(|item| item.get("name").and_then(JsonValue::as_str) == Some("JSON")));
    let manifests = inventory
        .get("manifests")
        .and_then(JsonValue::as_array)
        .unwrap();
    assert!(manifests
        .iter()
        .any(|item| item.as_str() == Some("src-tauri/Cargo.toml")));
    let line_stats = inventory.get("lineStats").unwrap();
    assert_eq!(
        line_stats.get("physical").and_then(JsonValue::as_u64),
        Some(5)
    );
    assert_eq!(line_stats.get("code").and_then(JsonValue::as_u64), Some(3));
    assert_eq!(
        line_stats.get("comments").and_then(JsonValue::as_u64),
        Some(1)
    );
    assert_eq!(line_stats.get("blank").and_then(JsonValue::as_u64), Some(1));
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn source_inventory_does_not_follow_links_outside_selected_root() {
    use std::os::unix::fs::symlink;

    let root = std::env::temp_dir().join(format!("oviraptor-inventory-{}", Uuid::new_v4()));
    let outside = std::env::temp_dir().join(format!("oviraptor-outside-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(root.join("inside.rs"), "fn inside() {}\n").unwrap();
    fs::write(outside.join("outside.rs"), "fn outside() {}\n").unwrap();
    fs::write(
        outside.join("package.json"),
        r#"{"dependencies":{"react":"^19"}}"#,
    )
    .unwrap();
    symlink(&outside, root.join("linked_directory")).unwrap();
    symlink(outside.join("outside.rs"), root.join("linked_file.rs")).unwrap();
    symlink(outside.join("package.json"), root.join("package.json")).unwrap();

    let inventory = inspect_source_tree(&root);
    assert_eq!(inventory["totalFiles"], 1);
    assert_eq!(inventory["codeFiles"], 1);
    assert_eq!(inventory["lineStats"]["physical"], 1);
    assert_eq!(inventory["architecture"], "Source repository");
    assert_eq!(inventory["manifests"], serde_json::json!([]));
    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(outside).unwrap();
}

#[test]
fn correlates_sast_and_dast_with_weighted_endpoint_parameter_and_data_flow() {
    let candidate = |source_type: &str, file: &str, data_flow: bool| AppSecCandidate {
        finding_id: if source_type == "sast" { 1 } else { 2 },
        source_key: source_type.into(),
        source_types: vec![source_type.into()],
        engine: source_type.into(),
        title: "SQL injection".into(),
        vulnerability_type: "SQL Injection".into(),
        severity: "high".into(),
        confidence: "high".into(),
        url: "https://test.example/api/user/search".into(),
        method: "POST".into(),
        parameter: "id".into(),
        file: file.into(),
        symbol: "searchUser".into(),
        start_line: if file.is_empty() { 0 } else { 120 },
        cwe: "CWE-89".into(),
        has_data_flow: data_flow,
        evidence: JsonValue::Null,
    };
    let (score, detail) = appsec_correlation(
        &candidate("sast", "UserController.java", true),
        &candidate("dast", "", false),
    );
    assert_eq!(score, 100);
    assert_eq!(detail["type"]["matched"], true);
    assert_eq!(detail["url"]["matched"], true);
    assert_eq!(detail["parameter"]["matched"], true);
    assert_eq!(detail["dataFlow"]["matched"], true);
    assert_eq!(detail["codeLocation"]["matched"], false);
}

#[test]
fn cicd_gate_blocks_release_when_critical_threshold_is_exceeded() {
    let root = std::env::temp_dir().join(format!("asset-atlas-gate-{}", Uuid::new_v4()));
    let db_path = db::initialize(&root).unwrap();
    let connection = db::open(&db_path).unwrap();
    connection
        .execute("INSERT INTO projects(id,name) VALUES(1,'test')", [])
        .unwrap();
    connection
        .execute(
            "INSERT INTO sentinel_scans(id,project_id,scan_type) VALUES('ci-scan',1,'cicd')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO sentinel_scan_contexts(scan_id,policy_json) VALUES('ci-scan',?1)",
            [serde_json::json!({"maxCritical":0,"maxHigh":5,"blockRelease":true}).to_string()],
        )
        .unwrap();
    connection.execute("INSERT INTO appsec_vulnerabilities(id,project_id,fingerprint,title,vulnerability_type,severity) VALUES(1,1,'critical-1','Critical issue','SQL Injection','critical')",[]).unwrap();
    connection.execute("INSERT INTO appsec_vulnerability_sources(vulnerability_id,scan_id,source_type,source_key) VALUES(1,'ci-scan','sast','semgrep:test')",[]).unwrap();

    evaluate_appsec_gate(&connection, "ci-scan").unwrap();
    let (status, reason): (String, String) = connection
        .query_row(
            "SELECT gate_status,gate_reason FROM sentinel_scan_contexts WHERE scan_id='ci-scan'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, "blocked");
    assert!(reason.contains("Critical 1/0"));
    drop(connection);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn clamps_batch_size_and_accepts_only_explicit_proxy_schemes() {
    assert_eq!(
        agent_batch_size(&serde_json::json!({"agentBatchSize":0})),
        1
    );
    assert_eq!(
        agent_batch_size(&serde_json::json!({"agentBatchSize":500})),
        50
    );
    let proxies = approved_agent_proxies(
        &serde_json::json!({"agentProxyEnabled":true,"authorizedProxyPool":["CN|http://127.0.0.1:7890","GLOBAL|socks5://127.0.0.1:1080","bad.example:80"]}),
    );
    assert_eq!(proxies.len(), 2);
}

#[test]
fn cloud_defaults_expand_while_local_deployment_remains_conservative() {
    let adaptive = AgentBudgetSettings::from_json(&serde_json::json!({
        "agentQuickTokenLimit": 0,
        "agentStandardTokenLimit": 0,
        "agentDeepTokenLimit": 0
    }));
    assert_eq!(adaptive.quick_tokens, 0);
    assert_eq!(adaptive.standard_tokens, 0);
    assert_eq!(adaptive.deep_tokens, 0);
    assert_eq!(adaptive.limits("deep").1, 0);
    let defaults = AgentBudgetSettings::from_json(&serde_json::json!({}));
    assert_eq!(defaults.quick_tokens, 200_000);
    assert_eq!(defaults.standard_tokens, 400_000);
    assert_eq!(defaults.deep_tokens, 800_000);
    assert_eq!(defaults.quick_requests, 6);
    assert_eq!(defaults.deep_requests, 24);
    assert_eq!(defaults.no_tool_turn_limit, 6);
    let mut local = defaults.clone();
    local.apply_deployment("local");
    assert_eq!(local.quick_tokens, 200_000);
    assert_eq!(local.standard_tokens, 400_000);
    assert_eq!(local.deep_tokens, 700_000);
    assert_eq!(local.quick_requests, 6);
    assert_eq!(local.deep_requests, 16);
    assert_eq!(local.no_tool_turn_limit, 6);
    assert_eq!(
        frontend_packet_budget(&serde_json::json!({}), "cloud"),
        24 * 1024
    );
    assert_eq!(
        frontend_packet_budget(&serde_json::json!({}), "local"),
        12 * 1024
    );
}

#[test]
fn current_hook_usage_keeps_provider_token_and_cache_fields() {
    let records = vec![
        serde_json::json!({"kind":"model_call", "requestId":"one", "usage":{
            "input_tokens":100,"output_tokens":10,"total_tokens":110,"input_tokens_details":{"cached_tokens":80}
        }}),
        serde_json::json!({"kind":"model_call", "requestId":"two", "usage":{
            "prompt_tokens":200,"completion_tokens":20,"total_tokens":220,"prompt_tokens_details":{"cached_tokens":150}
        }}),
    ];
    let totals = llm_hook::usage_from_records(&records);
    assert_eq!(totals.requests, 2);
    assert_eq!(totals.input_tokens, 300);
    assert_eq!(totals.output_tokens, 30);
    assert_eq!(totals.cached_tokens, 230);
    assert_eq!(totals.total_tokens, 330);
}
