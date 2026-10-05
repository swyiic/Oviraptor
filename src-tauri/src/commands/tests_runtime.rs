    #[test]
    fn cloud_agent_plan_expands_only_beyond_soft_budget() {
        let adaptive = AgentBudgetSettings::from_json(&serde_json::json!({}));
        let route = FrontendRoute {
            url: "https://app.example.invalid".into(),
            score: 60,
            mode: "standard".into(),
            surface: "framework_application".into(),
            reasons: Vec::new(),
        };
        let environment = ModelRuntimeEnv {
            llm: "openai/test".into(),
            api_key: String::new(),
            api_base: String::new(),
            deployment: "cloud".into(),
            full_power: false,
            prompt_audit_mode: "off".into(),
        };
        let plan = build_agent_execution_plan(
            &adaptive,
            &route,
            &environment,
            AgentBackendKind::LegacyRemoved,
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
    fn environment_install_rejects_active_or_queued_scans() {
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        connection
            .execute_batch("CREATE TABLE sentinel_scans (id TEXT PRIMARY KEY, status TEXT NOT NULL);")
            .unwrap();
        assert!(ensure_environment_install_idle(&connection).is_ok());
        for status in ["queued", "scanning", "pausing"] {
            connection
                .execute("INSERT INTO sentinel_scans(id,status) VALUES('task',?1)", [status])
                .unwrap();
            assert!(ensure_environment_install_idle(&connection).is_err(), "{status}");
            connection.execute("DELETE FROM sentinel_scans", []).unwrap();
        }
        connection
            .execute("INSERT INTO sentinel_scans(id,status) VALUES('task','partial')", [])
            .unwrap();
        assert!(ensure_environment_install_idle(&connection).is_ok());
    }

    #[test]
    fn environment_preparation_and_scan_activation_are_atomic_across_connections() {
        let root = std::env::temp_dir().join(format!("oviraptor-preparation-{}", Uuid::new_v4()));
        let db_path = db::initialize(&root).unwrap();
        let preparation = db::begin_environment_preparation(&db_path).unwrap();
        let connection = db::open(&db_path).unwrap();
        assert!(connection.execute("INSERT INTO sentinel_scans(id,status) VALUES('new','scanning')", []).is_err());
        connection.execute("INSERT INTO sentinel_scans(id,status) VALUES('draft','draft')", []).unwrap();
        assert!(connection.execute("UPDATE sentinel_scans SET status='queued' WHERE id='draft'", []).is_err());
        assert!(db::begin_environment_preparation(&db_path).is_err());
        preparation.complete().unwrap();
        connection.execute("UPDATE sentinel_scans SET status='queued' WHERE id='draft'", []).unwrap();
        assert!(db::begin_environment_preparation(&db_path).is_err());
        connection.execute("UPDATE sentinel_scans SET status='completed' WHERE id='draft'", []).unwrap();
        let preparation = db::begin_environment_preparation(&db_path).unwrap();
        preparation.complete().unwrap();
        drop(connection);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_abandoned_environment_preparation_remains_fenced_after_restart() {
        let root = std::env::temp_dir().join(format!("oviraptor-abandoned-preparation-{}", Uuid::new_v4()));
        let db_path = db::initialize(&root).unwrap();
        let connection = db::open(&db_path).unwrap();
        connection.execute("INSERT INTO environment_preparation_lease(singleton,owner) VALUES(1,'abandoned')", []).unwrap();
        assert!(connection.execute("INSERT INTO sentinel_scans(id,status) VALUES('blocked','queued')", []).is_err());
        db::initialize(&root).unwrap();
        assert!(connection.execute("INSERT INTO sentinel_scans(id,status) VALUES('still-blocked','queued')", []).is_err());
        assert!(db::begin_environment_preparation(&db_path).err().unwrap().contains("需管理员核对"));
        let status = db::environment_preparation_status(&db_path).unwrap();
        assert_eq!(status.state, "requires_manual_recovery");
        assert_eq!(status.owner.as_deref(), Some("abandoned"));
        assert!(status.created_at.is_some());
        assert!(db::recover_environment_preparation(&db_path, "abandoned", "no").is_err());
        assert!(db::recover_environment_preparation(&db_path, "wrong-owner", "已确认安装子进程停止").is_err());
        assert!(connection.execute("INSERT INTO sentinel_scans(id,status) VALUES('still-blocked-2','queued')", []).is_err());
        db::recover_environment_preparation(&db_path, "abandoned", "已确认安装子进程停止").unwrap();
        assert_eq!(db::environment_preparation_status(&db_path).unwrap().state, "idle");
        let events: i64 = connection.query_row("SELECT COUNT(*) FROM environment_preparation_events WHERE owner='abandoned' AND event='manual_recovered_installer_verified'", [], |row| row.get(0)).unwrap();
        assert_eq!(events, 1);
        connection.execute("INSERT INTO sentinel_scans(id,status) VALUES('recovered','queued')", []).unwrap();
        drop(connection);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn running_preparation_cannot_be_manually_recovered() {
        let root = std::env::temp_dir().join(format!("oviraptor-live-preparation-{}", Uuid::new_v4()));
        let db_path = db::initialize(&root).unwrap();
        let preparation = db::begin_environment_preparation(&db_path).unwrap();
        let status = db::environment_preparation_status(&db_path).unwrap();
        assert_eq!(status.state, "installing");
        assert!(db::recover_environment_preparation(&db_path, status.owner.as_deref().unwrap(), "已确认安装子进程停止").is_err());
        let connection = db::open(&db_path).unwrap();
        assert_eq!(connection.query_row("SELECT COUNT(*) FROM environment_preparation_lease", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
        drop(preparation);
        assert_eq!(db::environment_preparation_status(&db_path).unwrap().state, "requires_manual_recovery");
        assert!(connection.execute("INSERT INTO sentinel_scans(id,status) VALUES('blocked','queued')", []).is_err());
        db::recover_environment_preparation(&db_path, status.owner.as_deref().unwrap(), "已确认安装子进程停止").unwrap();
        assert_eq!(db::environment_preparation_status(&db_path).unwrap().state, "idle");
        drop(connection);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn concurrent_preparation_and_scan_activation_have_only_one_winner() {
        let root = std::env::temp_dir().join(format!("oviraptor-preparation-race-{}", Uuid::new_v4()));
        let db_path = db::initialize(&root).unwrap();
        let ready = std::sync::Arc::new(std::sync::Barrier::new(3));
        let finished = std::sync::Arc::new(std::sync::Barrier::new(3));
        let install = {
            let db_path = db_path.clone();
            let ready = ready.clone();
            let finished = finished.clone();
            std::thread::spawn(move || {
                ready.wait();
                let guard = db::begin_environment_preparation(&db_path);
                let success = guard.is_ok();
                finished.wait();
                success
            })
        };
        let scan = {
            let db_path = db_path.clone();
            let ready = ready.clone();
            let finished = finished.clone();
            std::thread::spawn(move || {
                let connection = db::open(&db_path).unwrap();
                ready.wait();
                let success = connection.execute("INSERT INTO sentinel_scans(id,status) VALUES('race','queued')", []).is_ok();
                finished.wait();
                success
            })
        };
        ready.wait();
        finished.wait();
        assert_ne!(install.join().unwrap(), scan.join().unwrap());
        std::fs::remove_dir_all(root).unwrap();
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
        assert!(model_retryable_provider_failure(
            "HTTP 400: Resource temporarily unavailable (os error 35)"
        ));
        assert!(model_retryable_provider_failure("upstream overloaded"));
        assert!(model_retryable_provider_failure(
            "HTTP 429 too many requests"
        ));
        assert!(!model_retryable_provider_failure("invalid api key"));
    }

    #[test]
    fn model_authentication_failure_is_treated_as_configuration_failure() {
        assert!(model_configuration_failure(
            "模型认证失败：API Key 无效或已失效"
        ));
        assert!(model_configuration_failure(
            "authentication_error: invalid api key"
        ));
        assert!(!model_configuration_failure(
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
            "INSERT INTO agent_skills(name,description,instructions,builtin,enabled) VALUES('专项测试','','SHOULD_NOT_BE_DEFAULT',0,1)",
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
            Some("proof"),
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
