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
            Some(true)
        );
        let evidence_dir = prepare_strix_web_evidence_directory(&root).unwrap();
        let input_manifest = strix_input_manifest(&evidence_dir).unwrap();
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
        assert_eq!(strix_input_manifest(&evidence_dir).unwrap(), input_manifest);
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
    fn detects_strix_cli_capabilities_instead_of_hard_coding_versions() {
        let current_help = r#"
usage: strix [--target TARGET] [--target-list PATH]
  --config FILE
  --instruction-file FILE
  --non-interactive
  --scan-mode MODE
  --scope-mode MODE
  --diff-base BASE
  --workspace-file PATH
  --mcp-config PATH
  --mcp-server NAME
  --mcp-exclude NAME
  --max-budget USD, --max-budget-usd USD
  --max-turns N
"#;
        let current = parse_strix_cli_capabilities(current_help, "strix 1.6.2").unwrap();
        assert!(!current.mount_flag);
        assert!(current.target_flag);
        assert!(current.config_flag);
        assert!(current.max_turns_flag);
        assert!(current.workspace_file_flag);
        assert!(current.mcp_config_flag);
        assert!(current.mcp_server_flag);
        assert!(current.mcp_exclude_flag);
        assert_eq!(current.max_budget_flag.as_deref(), Some("--max-budget-usd"));
        let mut command = Command::new("strix");
        let transport =
            append_strix_local_directory(&mut command, &current, Path::new("/tmp/evidence"))
                .unwrap();
        assert_eq!(transport, "--target");
        assert_eq!(
            command
                .get_args()
                .map(|value| value.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            vec!["--target", "/tmp/evidence"]
        );

        let legacy = parse_strix_cli_capabilities(current_help, "strix 1.5.2").unwrap_err();
        assert!(legacy.contains("1.5.3"));
        let major = parse_strix_cli_capabilities(current_help, "strix 2.0.0").unwrap_err();
        assert!(major.contains("尚未审核的大版本"));
        let future = parse_strix_cli_capabilities(current_help, "strix 1.6.3").unwrap_err();
        assert!(future.contains("新于 Oviraptor 已审核版本"));
        let incompatible = parse_strix_cli_capabilities(
            "usage: strix --target TARGET --non-interactive --scan-mode MODE",
            "strix 1.6.0",
        )
        .unwrap_err();
        assert!(incompatible.contains("--instruction-file"));
    }

    #[test]
    fn stages_web_evidence_and_detects_input_mutation_without_touching_originals() {
        let root = std::env::temp_dir().join(format!("oviraptor-strix-input-{}", Uuid::new_v4()));
        fs::create_dir_all(root.join("frontend-code-slices")).unwrap();
        fs::write(root.join("frontend-evidence.json"), r#"{"value":1}"#).unwrap();
        fs::write(root.join("frontend-code-slices/api.js"), "fetch('/api')").unwrap();
        let stage = prepare_strix_web_evidence_directory(&root).unwrap();
        let before = strix_input_manifest(&stage).unwrap();
        assert_eq!(
            fs::read_to_string(stage.join("frontend-evidence.json")).unwrap(),
            r#"{"value":1}"#
        );
        fs::write(stage.join("frontend-evidence.json"), r#"{"value":2}"#).unwrap();
        let after = strix_input_manifest(&stage).unwrap();
        assert_ne!(before, after);
        assert_eq!(
            fs::read_to_string(root.join("frontend-evidence.json")).unwrap(),
            r#"{"value":1}"#
        );
        let _ = fs::remove_dir_all(root);
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
                [
                    serde_json::json!({"maxCritical":0,"maxHigh":5,"blockRelease":true})
                        .to_string(),
                ],
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
    fn aggregates_usage_across_strix_sub_batches() {
        let root = std::env::temp_dir().join(format!("asset-atlas-batches-{}", Uuid::new_v4()));
        let db_path = db::initialize(&root).unwrap();
        let connection = db::open(&db_path).unwrap();
        connection
            .execute(
                "INSERT INTO sentinel_scans(id,status) VALUES('parent','scanning')",
                [],
            )
            .unwrap();
        for (stage, input, output, cached) in
            [("strix_run:a", 100, 10, 80), ("strix_run:b", 200, 20, 150)]
        {
            let raw=serde_json::json!({"llm_usage":{"requests":1,"input_tokens":input,"output_tokens":output,"total_tokens":input+output,"input_tokens_details":[{"cached_tokens":cached}]}}).to_string();
            connection.execute("INSERT INTO sentinel_checkpoints(scan_id,url,stage,raw_json) VALUES('parent','*',?1,?2)",params![stage,raw]).unwrap();
        }
        assert_eq!(
            aggregate_strix_usage(&connection, "parent").unwrap(),
            (2, 300, 30, 230, 330)
        );
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn clamps_batch_size_and_accepts_only_explicit_proxy_schemes() {
        assert_eq!(
            strix_batch_size(&serde_json::json!({"strixBatchSize":0})),
            1
        );
        assert_eq!(
            strix_batch_size(&serde_json::json!({"strixBatchSize":500})),
            50
        );
        let proxies = approved_strix_proxies(
            &serde_json::json!({"strixProxyEnabled":true,"authorizedProxyPool":["CN|http://127.0.0.1:7890","GLOBAL|socks5://127.0.0.1:1080","bad.example:80"]}),
        );
        assert_eq!(proxies.len(), 2);
    }

    #[test]
    fn cloud_defaults_expand_while_local_deployment_remains_conservative() {
        let adaptive = AdaptiveStrixSettings::from_json(&serde_json::json!({
            "strixQuickTokenLimit": 0,
            "strixStandardTokenLimit": 0,
            "strixDeepTokenLimit": 0
        }));
        assert_eq!(adaptive.quick_tokens, 0);
        assert_eq!(adaptive.standard_tokens, 0);
        assert_eq!(adaptive.deep_tokens, 0);
        assert_eq!(adaptive.limits("deep").1, 0);
        let defaults = AdaptiveStrixSettings::from_json(&serde_json::json!({}));
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
    fn reads_strix_1_3_request_usage_entries_when_aggregate_fields_are_missing() {
        let usage = serde_json::json!({
            "request_usage_entries": [
                {"input_tokens":100,"output_tokens":10,"total_tokens":110,"input_tokens_details":{"cached_tokens":80}},
                {"prompt_tokens":200,"completion_tokens":20,"total_tokens":220,"prompt_tokens_details":{"cached_tokens":150}}
            ],
            "agents": [{"agent_id":"root"},{"agent_id":"child"}]
        });
        assert_eq!(usage_request_count(&usage), 2);
        assert_eq!(usage_input_tokens(&usage), 300);
        assert_eq!(usage_output_tokens(&usage), 30);
        assert_eq!(usage_cached_tokens(&usage), 230);
        assert_eq!(usage_total_tokens(&usage), 330);
    }

    #[test]
    fn reads_new_strix_usage_aliases_and_numeric_strings() {
        let usage = serde_json::json!({
            "requests": "3",
            "inputTokens": "1200",
            "outputTokens": 300,
            "totalTokens": "1500",
            "inputTokensDetails": {"cachedTokens": "900"}
        });
        assert_eq!(usage_request_count(&usage), 3);
        assert_eq!(usage_input_tokens(&usage), 1200);
        assert_eq!(usage_output_tokens(&usage), 300);
        assert_eq!(usage_cached_tokens(&usage), 900);
        assert_eq!(usage_total_tokens(&usage), 1500);
    }

    #[test]
    fn loads_strix_json_envelopes_and_artifact_fallbacks() {
        let root =
            std::env::temp_dir().join(format!("asset-atlas-strix-formats-{}", Uuid::new_v4()));
        fs::create_dir_all(root.join("vulnerabilities")).unwrap();
        fs::write(
            root.join("vulnerabilities.json"),
            serde_json::to_vec(&serde_json::json!({
                "findings": [{"id":"json-1","title":"JSON finding","severity":"high"}]
            }))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(strix_vulnerabilities(&root).len(), 1);
        assert_eq!(
            value_first(&strix_vulnerabilities(&root)[0], &["title"]),
            "JSON finding"
        );

        fs::remove_file(root.join("vulnerabilities.json")).unwrap();
        fs::write(
            root.join("findings.sarif"),
            serde_json::to_vec(&serde_json::json!({
                "runs": [{"tool":{"driver":{"rules":[
                    {"id":"R1","shortDescription":{"text":"SARIF finding"}},
                    {"id":"strix-coverage/xss","shortDescription":{"text":"Coverage: XSS"},"properties":{"tags":["coverage"]}}
                ]}},"results":[
                    {"ruleId":"R1","kind":"fail","level":"error","message":{"text":"message"}},
                    {"ruleId":"strix-coverage/xss","kind":"pass","level":"none","message":{"text":"tested clean"},"properties":{"strix":{"coverage_outcome":"no_issue_found"}}},
                    {"ruleId":"strix-coverage/ssrf","kind":"open","level":"none","message":{"text":"needs follow up"},"properties":{"strix":{"coverage_outcome":"needs_follow_up"}}}
                ]}]
            }))
            .unwrap(),
        )
        .unwrap();
        let sarif = strix_vulnerabilities(&root);
        assert_eq!(sarif.len(), 1);
        assert_eq!(value_first(&sarif[0], &["rule_id"]), "R1");

        fs::remove_file(root.join("findings.sarif")).unwrap();
        fs::write(
            root.join("vulnerabilities.csv"),
            "id,title,severity\ncsv-1,CSV finding,medium\n",
        )
        .unwrap();
        let csv = strix_vulnerabilities(&root);
        assert_eq!(csv.len(), 1);
        assert_eq!(value_first(&csv[0], &["title"]), "CSV finding");

        fs::remove_file(root.join("vulnerabilities.csv")).unwrap();
        fs::write(
            root.join("vulnerabilities/vuln-0001.md"),
            "# Markdown finding\nEvidence",
        )
        .unwrap();
        let markdown = strix_vulnerabilities(&root);
        assert_eq!(markdown.len(), 1);
        assert_eq!(value_first(&markdown[0], &["title"]), "Markdown finding");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn accepts_new_strix_completion_statuses() {
        for status in ["succeeded", "success", "done"] {
            let root =
                std::env::temp_dir().join(format!("asset-atlas-strix-status-{}", Uuid::new_v4()));
            fs::create_dir_all(&root).unwrap();
            fs::write(
                root.join("run.json"),
                serde_json::json!({"status":status}).to_string(),
            )
            .unwrap();
            assert!(strix_run_completed(&root), "status={status}");
            let _ = fs::remove_dir_all(root);
        }
    }
