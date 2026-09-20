    #[test]
    fn database_upgrade_preserves_zero_budget_and_refreshes_builtin_strategy() {
        let root =
            std::env::temp_dir().join(format!("asset-atlas-config-upgrade-{}", Uuid::new_v4()));
        let db_path = db::initialize(&root).unwrap();
        let connection = db::open(&db_path).unwrap();
        connection
            .execute(
                "UPDATE config_profiles SET settings_json=json_set(settings_json,'$.strixDeepTokenLimit',0)",
                [],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE strix_skills SET instructions='Read Oviraptor frontend-evidence.json first when present.' WHERE name='业务前端深度分析' AND builtin=1",
                [],
            )
            .unwrap();
        drop(connection);

        db::initialize(&root).unwrap();
        let connection = db::open(&db_path).unwrap();
        let settings: String = connection
            .query_row(
                "SELECT settings_json FROM config_profiles ORDER BY id LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let settings: JsonValue = serde_json::from_str(&settings).unwrap();
        assert_eq!(settings["strixDeepTokenLimit"], 0);
        let instructions: String = connection
            .query_row(
                "SELECT instructions FROM strix_skills WHERE name='业务前端深度分析' AND builtin=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(instructions.contains("## 默认测试流程"));
        assert!(instructions.contains("## 学习规则"));
        assert!(instructions.contains("apiPrefix"));
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn database_upgrade_repairs_full_power_ui_policy_signature() {
        let root = std::env::temp_dir().join(format!("oviraptor-policy-repair-{}", Uuid::new_v4()));
        let db_path = db::initialize(&root).unwrap();
        let connection = db::open(&db_path).unwrap();
        connection
            .execute_batch(
                r#"
            UPDATE config_profiles SET settings_json=json_set(
                settings_json,
                '$.strixBudgetPolicyVersion',2,
                '$.strixBatchSize',50,
                '$.strixQuickScore',1,
                '$.strixStandardScore',2,
                '$.strixDeepScore',3,
                '$.strixQuickTimeout',3600,
                '$.strixStandardTimeout',7200,
                '$.strixDeepTimeout',14400,
                '$.strixQuickTokenLimit',0,
                '$.strixStandardTokenLimit',0,
                '$.strixDeepTokenLimit',0,
                '$.strixQuickRequestLimit',100,
                '$.strixStandardRequestLimit',200,
                '$.strixDeepRequestLimit',300,
                '$.strixNoToolTurnLimit',100
            );
            "#,
            )
            .unwrap();
        drop(connection);

        db::initialize(&root).unwrap();
        let connection = db::open(&db_path).unwrap();
        let settings: String = connection
            .query_row(
                "SELECT settings_json FROM config_profiles ORDER BY id LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let settings: JsonValue = serde_json::from_str(&settings).unwrap();
        assert_eq!(settings["strixBudgetPolicyVersion"], 6);
        assert_eq!(settings["strixQuickRequestLimit"], 8);
        assert_eq!(settings["strixDeepRequestLimit"], 16);
        assert_eq!(settings["strixNoToolTurnLimit"], 6);
        assert_eq!(settings["strixDeepTokenLimit"], 800_000);
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn database_upgrade_repairs_completed_target_without_http_tool_evidence() {
        let root = std::env::temp_dir().join(format!(
            "oviraptor-false-strix-completion-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let db_path = db::initialize(&root).unwrap();
        let connection = db::open(&db_path).unwrap();
        connection
            .execute("INSERT INTO projects(id,name) VALUES(411,'Repair')", [])
            .unwrap();
        connection.execute("INSERT INTO sentinel_scans(id,project_id,project_name,status,current_checkpoint,scan_type) VALUES('false-complete',411,'Repair','completed','扫描完成：自动验证 1','web')", []).unwrap();
        connection.execute("INSERT INTO sentinel_targets(project_id,scan_id,url,status,scan_mode,routing_reason) VALUES(411,'false-complete','https://example.invalid','completed','standard','自动验证已按边界收口（本轮未形成新的工具证据）：模型只完成了本地证据准备，没有取得目标请求/响应，未将其记为自动验证完成；可重试未完成阶段')", []).unwrap();
        connection
            .execute(
                "DELETE FROM app_settings WHERE key='strix_false_completion_repair_version'",
                [],
            )
            .unwrap();
        drop(connection);

        let migrated_path = db::initialize(&root).unwrap();
        assert_eq!(migrated_path, db_path);
        let connection = db::open(&db_path).unwrap();
        let target_status: String = connection
            .query_row(
                "SELECT status FROM sentinel_targets WHERE scan_id='false-complete'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let (scan_status, checkpoint): (String, String) = connection
            .query_row(
                "SELECT status,current_checkpoint FROM sentinel_scans WHERE id='false-complete'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(target_status, "partial");
        assert_eq!(scan_status, "partial");
        assert!(checkpoint.contains("未取得目标请求/响应"));
        assert!(checkpoint.contains("保留待验证 1"));
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn directory_discovery_guard_recognizes_direct_and_shell_wrapped_tools() {
        assert!(is_directory_discovery_tool("ffuf", ""));
        assert!(is_directory_discovery_tool(
            "exec_command",
            "python -m dirsearch -u https://example.invalid"
        ));
        assert!(!is_directory_discovery_tool(
            "browser_request",
            "/api/login"
        ));
    }

    #[test]
    fn large_skill_context_is_compacted_before_prompt_injection() {
        let body = (0..200)
            .map(|index| format!("## 方法 {index}\n验证步骤 {index}。"))
            .collect::<Vec<_>>()
            .join("\n");
        let compacted = compact_skill_context(&body, 1_000);

        assert!(compacted.chars().count() <= 1_000);
        assert!(compacted.contains("## 方法 0"));
        assert!(compacted.contains("本地大方法包"));
        assert!(!compacted.contains("## 方法 199"));
    }

    #[test]
    fn compact_manual_deep_dive_keeps_core_fields_without_bloating_model_input() {
        let decision = serde_json::json!({
            "schemaVersion":3,
            "eligibleForModel":true,
            "manualDeepDive":[
                {"rank":1,"category":"authorization","title":"同级账号权限","priority":"critical","reason":"需要双账号对象对照","evidence":["GET /api/users/1","GET /api/users/2","ignored"],"missingEvidence":"两个平权账号","steps":["创建各自对象","交叉重放","比较响应"],"stopCondition":"三个对象均无差异"},
                {"rank":2,"category":"business_flow","title":"业务状态机","priority":"high","reason":"需要业务不变量","evidence":["POST /api/order"],"missingEvidence":"可回滚订单","steps":["建立基线"],"stopCondition":"状态受控"},
                {"rank":3,"category":"file_handling","title":"文件处理","priority":"high","reason":"需要样本","evidence":[],"missingEvidence":"无害文件","steps":["上传并清理"],"stopCondition":"清理完成"},
                {"rank":4,"category":"api_inventory","title":"影子 API","priority":"medium","reason":"不会进入模型","evidence":[],"missingEvidence":"移动端流量","steps":["补流量"],"stopCondition":"无新增"}
            ]
        });
        let rows = compact_manual_deep_dive(Some(&decision));
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0]["classification"], "coverage_gap_not_vulnerability");
        assert_eq!(rows[0]["evidence"].as_array().unwrap().len(), 2);
        assert_eq!(rows[0]["steps"].as_array().unwrap().len(), 2);
        let compact = compact_incremental_decision(Some(&decision));
        assert_eq!(compact["schemaVersion"], 3);
        assert!(compact.get("manualDeepDive").is_none());
    }

    #[test]
    fn unknown_static_paths_never_enter_model_evidence_or_raise_target_score() {
        let adaptive = AdaptiveStrixSettings::from_json(&serde_json::json!({}));
        let target = serde_json::json!({
            "url":"https://app.example.invalid",
            "statusCode":200,
            "fingerprint":{"frontend":{"framework":"Vue","confidence":"high"}},
            "jsFiles":[{"url":"https://app.example.invalid/main.js","type":"application","statusCode":200}],
            "apis":[],
            "apiCandidates":[{
                "url":"https://app.example.invalid/api/general/search",
                "method":"UNKNOWN",
                "confidence":"medium",
                "extractionEngine":"string-heuristic",
                "verification":{"verified":false,"reason":"probe_budget"}
            }],
            "opportunities":[{
                "score":90,
                "method":"UNKNOWN",
                "endpoint":"https://app.example.invalid/api/general/search",
                "source":"string-heuristic",
                "readiness":{"stage":"needs_contract"}
            }],
            "routes":[],
            "sensitiveInfo":[]
        });
        let route = score_frontend_target(&target, "https://app.example.invalid", &adaptive);
        let evidence = compact_frontend_evidence(&target, &route.url, &route, 20 * 1024);
        assert_eq!(route.mode, "skip");
        assert!(evidence["apiCandidates"].as_array().unwrap().is_empty());
        assert!(evidence["opportunities"].as_array().unwrap().is_empty());
    }

    #[test]
    fn detects_no_tool_loops_without_counting_todo_updates() {
        let root =
            std::env::temp_dir().join(format!("asset-atlas-live-metrics-{}", Uuid::new_v4()));
        let run = root.join("strix_runs/test-run");
        fs::create_dir_all(&run).unwrap();
        fs::write(
            run.join("run.json"),
            serde_json::json!({"llm_usage":{"requests":12,"input_tokens":490000,"output_tokens":10000,"total_tokens":500000}}).to_string(),
        ).unwrap();
        fs::write(
            run.join("strix.log"),
            "Invoking tool create_todo\nInvoking tool update_todo\nInvoking tool create_agent\nInvoking tool create_note\nInvoking tool create_vulnerability_report\nInvoking tool finish_scan\n",
        )
        .unwrap();
        let metrics = live_strix_metrics(&root);
        assert_eq!(metrics.requests, 12);
        assert_eq!(metrics.total_tokens, 500000);
        assert_eq!(metrics.meaningful_tools, 0);
        fs::write(
            run.join("strix.log"),
            "Tool create_todo completed.\nTool list_requests completed.\nTool create_vulnerability_report completed.\n",
        )
        .unwrap();
        assert_eq!(live_strix_metrics(&root).meaningful_tools, 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn distinct_commands_are_not_counted_as_repeated_tool_invocations() {
        let list = strix_tool_invocation_key("exec_command", r#"{"cmd":"ls -la"}"#);
        let read = strix_tool_invocation_key("exec_command", r#"{"cmd":"cat evidence.json"}"#);
        let same_with_spacing =
            strix_tool_invocation_key("exec_command", r#"{ "cmd" : "ls -la" }"#);
        assert_ne!(list, read);
        assert_eq!(list, same_with_spacing);
        assert!(!is_target_verification_tool(
            "exec_command",
            r#"{"cmd":"cat frontend-evidence.json"}"#,
        ));
        assert!(is_target_verification_tool(
            "exec_command",
            r#"{"cmd":"curl -sS https://example.test/api/profile"}"#,
        ));
        assert!(is_target_verification_tool(
            "repeat_request",
            r#"{"request_id":"123"}"#,
        ));
        assert!(is_target_verification_tool(
            "exec_command",
            r#"{"cmd":"agent-browser open https://example.test/profile"}"#,
        ));
        assert!(target_verification_output_is_usable(
            "repeat_request",
            r#"{"success":true,"status":"DONE","response":{"status_code":200}}"#,
        ));
        assert!(!target_verification_output_is_usable(
            "repeat_request",
            r#"{"success":false,"error":"Request 123 not found"}"#,
        ));
        assert!(!target_verification_output_is_usable(
            "exec_command",
            "Chunk ID: x\nProcess exited with code 7\nFinal output:\ncurl: failed to connect",
        ));
        assert!(target_verification_output_is_usable(
            "exec_command",
            "Chunk ID: x\nProcess exited with code 0\nFinal output:\nHTTP/2 200\n{\"ok\":true}",
        ));
    }
