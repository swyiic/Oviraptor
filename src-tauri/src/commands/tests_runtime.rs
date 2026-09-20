    #[test]
    fn cloud_agent_plan_expands_only_beyond_soft_budget() {
        let adaptive = AdaptiveStrixSettings::from_json(&serde_json::json!({}));
        let route = FrontendRoute {
            url: "https://app.example.invalid".into(),
            score: 60,
            mode: "standard".into(),
            surface: "framework_application".into(),
            reasons: Vec::new(),
        };
        let environment = StrixRuntimeEnv {
            llm: "openai/test".into(),
            api_key: String::new(),
            api_base: String::new(),
            image: String::new(),
            deployment: "cloud".into(),
            full_power: false,
            prompt_audit_mode: "off".into(),
        };
        let plan = build_agent_execution_plan(
            &adaptive,
            &route,
            &environment,
            AgentBackendKind::Strix,
            Path::new("/nonexistent/oviraptor.sqlite3"),
            "plan-scan",
        );
        assert_eq!(plan.soft_uncached_tokens, 400_000);
        assert_eq!(plan.hard_total_tokens, 2_000_000);
        assert_eq!(plan.soft_model_requests, 14);
        assert_eq!(plan.hard_model_requests, 36);
        assert_eq!(plan.contract_limit, 32);
        assert_eq!(plan.discovery_passes, 2);
        assert!(plan.hard_total_tokens > plan.soft_uncached_tokens);
    }

    #[test]
    fn target_detection_does_not_treat_urls_as_cidr() {
        assert_eq!(detect_target("https://example.com/admin/login"), "domain");
        assert_eq!(detect_target("http://127.0.0.1:8080/health"), "domain");
        assert_eq!(detect_target("203.0.113.0/24"), "cidr");
        assert_eq!(detect_target("2001:db8::/48"), "cidr");
        assert_eq!(detect_target("203.0.113.7"), "ip");
        assert_eq!(detect_target("Example Company"), "company");
    }

    #[test]
    fn repair_reclassifies_closed_model_gate_and_recomputes_pipeline_summary() {
        let root = std::env::temp_dir().join(format!("oviraptor-route-repair-{}", Uuid::new_v4()));
        let db_path = db::initialize(&root).unwrap();
        let connection = db::open(&db_path).unwrap();
        connection
            .execute(
                "INSERT INTO projects(id,name) VALUES(301,'Route repair')",
                [],
            )
            .unwrap();
        connection
            .execute("INSERT INTO sentinel_scans(id,project_id,project_name,status,current_checkpoint,scan_type) VALUES('route-repair',301,'Route repair','partial','旧汇总','web')", [])
            .unwrap();
        connection
            .execute("INSERT INTO sentinel_targets(project_id,scan_id,url,status,scan_mode,routing_reason) VALUES(301,'route-repair','https://closed.invalid','partial','quick','本地调查停止：no_high_value_hypothesis')", [])
            .unwrap();
        connection
            .execute("INSERT INTO sentinel_targets(project_id,scan_id,url,status,scan_mode,routing_reason) VALUES(301,'route-repair','https://kept.invalid','partial','evidence_guided','调查图谱新增高价值证据')", [])
            .unwrap();
        connection
            .execute("INSERT INTO sentinel_targets(project_id,scan_id,url,status,scan_mode,routing_reason) VALUES(301,'route-repair','https://standard.invalid','partial','standard','真实运行时 API 进入标准扫描')", [])
            .unwrap();
        connection
            .execute("INSERT INTO sentinel_targets(project_id,scan_id,url,status,scan_mode,routing_reason) VALUES(301,'route-repair','https://baseline.invalid','partial','standard','渐进式基础覆盖调查')", [])
            .unwrap();
        connection
            .execute(r#"INSERT INTO investigation_metrics(scan_id,target_url,token_worthy,stop_reason,decision_json) VALUES('route-repair','https://closed.invalid',0,'no_high_value_hypothesis','{"eligibleForModel":false,"readyHypotheses":0}')"#, [])
            .unwrap();
        connection
            .execute(r#"INSERT INTO investigation_metrics(scan_id,target_url,token_worthy,stop_reason,decision_json) VALUES('route-repair','https://standard.invalid',0,'runtime_api_baseline','{"eligibleForModel":false,"standardInvestigationAllowed":true,"verifiedRuntimeApiCount":2}')"#, [])
            .unwrap();
        connection
            .execute(r#"INSERT INTO investigation_metrics(scan_id,target_url,token_worthy,stop_reason,decision_json) VALUES('route-repair','https://baseline.invalid',0,'progressive_baseline','{"eligibleForModel":false,"standardInvestigationAllowed":false,"baselineInvestigationAllowed":true}')"#, [])
            .unwrap();
        repair_associated_scan_state(&connection, "route-repair").unwrap();
        let closed: (String, String) = connection
            .query_row(
                "SELECT status,scan_mode FROM sentinel_targets WHERE url='https://closed.invalid'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(closed, ("recon_only".into(), "skip".into()));
        let standard: (String, String) = connection
            .query_row("SELECT status,scan_mode FROM sentinel_targets WHERE url='https://standard.invalid'", [], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap();
        assert_eq!(standard, ("partial".into(), "standard".into()));
        let baseline: (String, String) = connection
            .query_row("SELECT status,scan_mode FROM sentinel_targets WHERE url='https://baseline.invalid'", [], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap();
        assert_eq!(baseline, ("partial".into(), "standard".into()));
        let scan: (String, String) = connection
            .query_row(
                "SELECT status,current_checkpoint FROM sentinel_scans WHERE id='route-repair'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(scan.0, "partial");
        assert!(scan.1.contains("确定性侦察收口 1"));
        assert!(scan.1.contains("待补充验证 3"));
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn source_mapped_readonly_contract_opens_the_bounded_route_gate() {
        let decision = serde_json::json!({
            "standardInvestigationAllowed": true,
            "verifiedRuntimeApiCount": 0,
            "sourceMappedReadOnlyApiCount": 2,
            "sourceGuidedInvestigationAllowed": true
        });
        assert!(investigation_standard_gate_open(&decision));
        assert!(!investigation_standard_gate_open(&serde_json::json!({
            "standardInvestigationAllowed": true,
            "verifiedRuntimeApiCount": 0,
            "sourceMappedReadOnlyApiCount": 0
        })));
    }

    #[test]
    fn task_list_keeps_the_requested_web_mode_separate_from_target_routing() {
        let root = std::env::temp_dir().join(format!("oviraptor-task-mode-{}", Uuid::new_v4()));
        let db_path = db::initialize(&root).unwrap();
        let connection = db::open(&db_path).unwrap();
        connection
            .execute("INSERT INTO projects(id,name) VALUES(401,'Mode test')", [])
            .unwrap();
        connection.execute("INSERT INTO sentinel_scans(id,project_id,project_name,status,scan_type) VALUES('mode-test',401,'Mode test','draft','web')", []).unwrap();
        connection.execute("INSERT INTO sentinel_scan_contexts(scan_id,policy_json) VALUES('mode-test','{\"webModeCeiling\":\"deep\"}')", []).unwrap();
        let scan = sentinel_scan_by_id(&connection, "mode-test").unwrap();
        assert_eq!(scan.requested_scan_mode, "deep");
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn llm_test_uses_the_openai_chat_model_name_that_strix_sends() {
        assert_eq!(
            openai_chat_completion_model("openai/deepseek-v4-pro"),
            "deepseek-v4-pro"
        );
        assert_eq!(
            openai_chat_completion_model("deepseek/deepseek-v4-pro"),
            "deepseek/deepseek-v4-pro"
        );
    }

    #[test]
    fn temporary_provider_errors_are_retryable_not_auth_failures() {
        assert!(strix_retryable_provider_failure(
            "HTTP 400: Resource temporarily unavailable (os error 35)"
        ));
        assert!(strix_retryable_provider_failure("upstream overloaded"));
        assert!(strix_retryable_provider_failure(
            "HTTP 429 too many requests"
        ));
        assert!(!strix_retryable_provider_failure("invalid api key"));
    }

    #[test]
    fn model_authentication_failure_is_treated_as_configuration_failure() {
        assert!(strix_configuration_failure(
            "模型认证失败：API Key 无效或已失效"
        ));
        assert!(strix_configuration_failure(
            "authentication_error: invalid api key"
        ));
        assert!(!strix_configuration_failure(
            "目标返回 HTTP 500，且没有生成扫描产物"
        ));
    }

    #[test]
    fn unified_web_policy_injects_builtin_and_selected_skills() {
        let root =
            std::env::temp_dir().join(format!("oviraptor-default-web-skill-{}", Uuid::new_v4()));
        let db_path = db::initialize(&root).unwrap();
        let connection = db::open(&db_path).unwrap();
        connection.execute(
            "INSERT INTO strix_skills(name,description,instructions,builtin,enabled) VALUES('专项测试','','SHOULD_NOT_BE_DEFAULT',0,1)",
            [],
        ).unwrap();
        let selected_id = connection.last_insert_rowid();
        let policy = build_web_investigation_policy(
            Some("deep"),
            Some(15.0),
            Vec::new(),
            &[selected_id],
            "重点检查业务权限",
            "strix-workbench",
        )
        .unwrap();
        let (effective, names, instructions) = effective_web_policy(
            &connection,
            &policy,
            &serde_json::json!({"strixAttackChainEnabled":true}),
        )
        .unwrap();
        assert!(names.contains("业务前端深度分析"));
        assert!(names.contains("专项测试"));
        assert!(instructions.contains("## 默认测试流程"));
        assert!(instructions.contains("SHOULD_NOT_BE_DEFAULT"));
        assert_eq!(
            effective.get("schemaVersion").and_then(JsonValue::as_i64),
            Some(5)
        );
        assert_eq!(
            effective
                .pointer("/automation/contractLimit")
                .and_then(JsonValue::as_i64),
            Some(64)
        );
        assert_eq!(
            effective
                .get("additionalInstruction")
                .and_then(JsonValue::as_str),
            Some("重点检查业务权限")
        );
        let catalog = effective
            .get("coverageCatalog")
            .and_then(JsonValue::as_array)
            .unwrap();
        assert!(catalog.len() >= 15);
        assert!(catalog.iter().any(|item| item["key"] == "business_flow"));
        assert!(catalog.iter().any(|item| item["key"] == "api_inventory"));
        assert!(catalog.iter().all(|item| item.get("manualFocus").is_some()));
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }
