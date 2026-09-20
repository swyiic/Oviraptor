    #[test]
    fn writes_one_private_strix_config_per_process_and_removes_it_after_use() {
        let root = std::env::temp_dir().join(format!("oviraptor-strix-runtime-{}", Uuid::new_v4()));
        let environment = StrixRuntimeEnv {
            llm: "openai/runtime-model".into(),
            api_key: "current-profile-key".into(),
            api_base: "https://provider.example.invalid/v1".into(),
            image: DEFAULT_STRIX_SANDBOX_IMAGE.into(),
            deployment: "cloud".into(),
            full_power: false,
            prompt_audit_mode: "off".into(),
        };
        let runtime =
            write_strix_runtime_config(&root, &environment, Some("http://127.0.0.1:48765/v1"))
                .unwrap();
        let path = runtime.path().to_path_buf();
        let payload: JsonValue = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            payload
                .pointer("/env/OPENAI_API_KEY")
                .and_then(JsonValue::as_str),
            Some("current-profile-key")
        );
        assert_eq!(
            payload
                .pointer("/env/OPENAI_BASE_URL")
                .and_then(JsonValue::as_str),
            Some("http://127.0.0.1:48765/v1")
        );
        assert_eq!(
            payload
                .pointer("/env/LLM_API_KEY")
                .and_then(JsonValue::as_str),
            Some("current-profile-key")
        );
        assert_eq!(
            payload
                .pointer("/env/LLM_API_BASE")
                .and_then(JsonValue::as_str),
            Some("http://127.0.0.1:48765/v1")
        );
        drop(runtime);
        assert!(!path.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn active_strix_profile_overrides_legacy_settings() {
        let root =
            std::env::temp_dir().join(format!("asset-atlas-strix-profile-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let settings = serde_json::json!({
            "strixLlm": "openai/legacy-model",
            "strixApiKey": "legacy-key",
            "strixApiBase": "https://legacy.example.invalid/v1",
            "strixActiveLlmProfileId": "profile-b",
            "strixLlmProfiles": [
                {
                    "id": "profile-a",
                    "name": "Model A",
                    "llm": "openai/model-a",
                    "apiKey": "key-a",
                    "apiBase": "https://a.example.invalid/v1"
                },
                {
                    "id": "profile-b",
                    "name": "Model B",
                    "llm": "openai/model-b",
                    "apiKey": "key-b",
                    "apiBase": "https://b.example.invalid/v1"
                }
            ]
        });

        let environment = strix_runtime_env(&settings, &root).unwrap();
        assert_eq!(environment.llm, "openai/model-b");
        assert_eq!(environment.api_key, "key-b");
        assert_eq!(environment.api_base, "https://b.example.invalid/v1");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn missing_active_strix_profile_falls_back_to_first_profile() {
        let root =
            std::env::temp_dir().join(format!("asset-atlas-strix-fallback-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let settings = serde_json::json!({
            "strixActiveLlmProfileId": "missing-profile",
            "strixLlmProfiles": [{
                "id": "profile-a",
                "llm": "openai/model-a",
                "apiKey": "key-a",
                "apiBase": "https://a.example.invalid/v1"
            }]
        });

        let environment = strix_runtime_env(&settings, &root).unwrap();
        assert_eq!(environment.llm, "openai/model-a");
        assert_eq!(environment.api_key, "key-a");
        assert_eq!(environment.api_base, "https://a.example.invalid/v1");
        let _ = fs::remove_dir_all(root);
    }

    /// §8.1: when both spellings exist, the neutral keys win; the Strix-named ones
    /// are only a read fallback for a profile that has not been opened since the
    /// migration.
    #[test]
    fn neutral_model_settings_are_the_authority() {
        let root =
            std::env::temp_dir().join(format!("asset-atlas-neutral-model-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let settings = serde_json::json!({
            "strixActiveLlmProfileId": "old",
            "strixLlmProfiles": [{
                "id": "old",
                "llm": "openai/old-model",
                "apiKey": "old-key",
                "apiBase": "https://old.example.invalid/v1"
            }],
            "activeModelProfileId": "new",
            "modelProfiles": [{
                "id": "new",
                "llm": "openai/new-model",
                "apiKey": "new-key",
                "apiBase": "https://new.example.invalid/v1",
                "deployment": "cloud"
            }]
        });

        let environment = strix_runtime_env(&settings, &root).unwrap();
        assert_eq!(environment.llm, "openai/new-model");
        assert_eq!(environment.api_key, "new-key");
        assert_eq!(environment.api_base, "https://new.example.invalid/v1");
        let _ = fs::remove_dir_all(root);
    }

    /// §8.1 and §8.2: a flat neutral configuration works without any profile list,
    /// and a local deployment may never borrow the cloud key.
    #[test]
    fn flat_neutral_settings_configure_a_local_endpoint_without_the_cloud_key() {
        let root =
            std::env::temp_dir().join(format!("asset-atlas-neutral-local-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let settings = serde_json::json!({
            "modelDeployment": "local",
            "modelApiBase": "http://127.0.0.1:18080/v1",
            "modelApiKey": "cloud-key-must-not-be-used",
            "localApiKey": "local-token",
            "strixLlm": "openai/kept-name"
        });

        let environment = strix_runtime_env(&settings, &root).unwrap();
        assert_eq!(environment.deployment, "local");
        assert_eq!(environment.llm, "openai/kept-name", "legacy name is still readable");
        assert_eq!(environment.api_base, "http://127.0.0.1:18080/v1");
        assert_eq!(environment.api_key, "local-token");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn legacy_and_unspecified_profiles_remain_cloud_governed() {
        let root = std::env::temp_dir().join(format!("asset-atlas-strix-cloud-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let settings = serde_json::json!({
            "strixActiveLlmProfileId": "cloud",
            "strixLocalFullPower": true,
            "strixLlmProfiles": [{
                "id": "cloud",
                "llm": "openai/cloud-model",
                "apiKey": "cloud-key"
            }]
        });

        let environment = strix_runtime_env(&settings, &root).unwrap();
        assert_eq!(environment.deployment, "cloud");
        assert!(!environment.full_power);
        assert_eq!(environment.prompt_audit_mode, "off");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn self_hosted_profile_allows_blank_key_and_explicit_full_power() {
        let root = std::env::temp_dir().join(format!("asset-atlas-strix-local-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let settings = serde_json::json!({
            "strixActiveLlmProfileId": "local",
            "strixLocalFullPower": true,
            "strixPromptAuditMode": "full",
            "strixLlmProfiles": [{
                "id": "local",
                "deployment": "local",
                "llm": "openai/local-model",
                "apiBase": "http://127.0.0.1:11434/v1",
                "apiKey": ""
            }]
        });

        let environment = strix_runtime_env(&settings, &root).unwrap();
        assert_eq!(environment.deployment, "local");
        assert!(environment.full_power);
        assert_eq!(environment.api_key, "local");
        assert_eq!(environment.prompt_audit_mode, "full");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn strix_process_overrides_inherited_generic_llm_credentials() {
        let environment = StrixRuntimeEnv {
            llm: "openai/local-27b".into(),
            api_key: "local-service-key".into(),
            api_base: "http://127.0.0.1:18080/v1".into(),
            image: DEFAULT_STRIX_SANDBOX_IMAGE.into(),
            deployment: "local".into(),
            full_power: true,
            prompt_audit_mode: "off".into(),
        };
        let mut command = Command::new("strix");
        command
            .env("LLM_API_KEY", "inherited-cloud-key")
            .env("LLM_API_BASE", "https://cloud.example.invalid/v1");
        command_strix_env(&mut command, &environment);
        command_strix_hook_env(&mut command, "http://127.0.0.1:49152/v1");
        let values = command
            .get_envs()
            .filter_map(|(key, value)| {
                Some((
                    key.to_string_lossy().into_owned(),
                    value?.to_string_lossy().into_owned(),
                ))
            })
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(
            values.get("OPENAI_API_KEY").map(String::as_str),
            Some("local-service-key")
        );
        assert_eq!(
            values.get("LLM_API_KEY").map(String::as_str),
            Some("local-service-key")
        );
        for key in ["OPENAI_BASE_URL", "OPENAI_API_BASE", "LLM_API_BASE"] {
            assert_eq!(
                values.get(key).map(String::as_str),
                Some("http://127.0.0.1:49152/v1")
            );
        }
    }

    #[test]
    fn self_hosted_profile_uses_only_its_separate_optional_key() {
        let root = std::env::temp_dir().join(format!("oviraptor-local-key-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let settings = serde_json::json!({
            "strixActiveLlmProfileId": "local",
            "strixLlmProfiles": [{
                "id": "local",
                "deployment": "local",
                "llm": "openai/local-model",
                "apiBase": "http://127.0.0.1:18080/v1",
                "apiKey": "stale-cloud-key",
                "localApiKey": "self-hosted-key"
            }]
        });
        let environment = strix_runtime_env(&settings, &root).unwrap();
        assert_eq!(environment.api_key, "self-hosted-key");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn local_model_policy_serializes_large_models_and_keeps_small_models_bounded() {
        let large = StrixRuntimeEnv {
            llm: "openai/Qwen3-27B-Instruct".into(),
            api_key: "local".into(),
            api_base: "http://127.0.0.1:11434/v1".into(),
            image: "strix:latest".into(),
            deployment: "local".into(),
            full_power: true,
            prompt_audit_mode: "off".into(),
        };
        let large_policy = local_model_runtime_policy(&large);
        assert_eq!(large_policy.parameter_billions, Some(27));
        assert_eq!(large_policy.max_concurrent_requests, 1);
        assert_eq!(large_policy.max_output_tokens, Some(3_072));
        assert_eq!(strix_startup_timeouts(&large), (240, 1_200));

        let large_on_64 = local_model_runtime_policy_for_memory(&large, 64);
        assert_eq!(large_on_64.max_context_tokens, 65_536);
        assert_eq!(large_on_64.memory_guard_tier, "balanced");
        assert_eq!(large_on_64.frontend_packet_budget_bytes, 12 * 1024);

        let small = StrixRuntimeEnv {
            llm: "mlx-community/Local-9B-4bit".into(),
            ..large.clone()
        };
        let small_policy = local_model_runtime_policy(&small);
        assert_eq!(small_policy.parameter_billions, Some(9));
        assert_eq!(small_policy.max_concurrent_requests, 1);
        assert_eq!(small_policy.max_output_tokens, Some(2_048));
        let small_on_16 = local_model_runtime_policy_for_memory(&small, 16);
        assert_eq!(small_on_16.max_context_tokens, 49_152);
        assert_eq!(small_on_16.memory_guard_tier, "aggressive");
        assert_eq!(small_on_16.frontend_packet_budget_bytes, 6 * 1024);

        let moe = StrixRuntimeEnv {
            llm: "openai/Qwen3-30B-A3B".into(),
            ..large
        };
        assert_eq!(model_parameter_billions(&moe.llm), Some(30));
        assert_eq!(local_model_runtime_policy(&moe).max_concurrent_requests, 1);
    }

    #[test]
    fn local_web_instruction_keeps_contract_and_task_requirements_compact() {
        let policy = serde_json::json!({
            "webModeCeiling": "standard",
            "additionalInstruction": "Prioritize the observed account lookup contract.",
            "capabilities": {"controlledWrite": {"available": true}}
        });
        let large_skill = (0..200)
            .map(|index| format!("## Section {index}\n{}", "detail ".repeat(80)))
            .collect::<Vec<_>>()
            .join("\n");
        let compact = render_web_investigation_instruction(&policy, &large_skill, true);
        assert!(compact.contains("Authorized internal defensive SRC assessment"));
        assert!(compact.contains("Prioritize the observed account lookup contract."));
        assert!(compact.contains("at most 32"));
        assert!(compact.contains(
            "No difference, no finding and an exhausted bounded branch are completion states"
        ));
        assert!(compact.contains("must finish by calling `finish_scan` exactly once"));
        assert!(compact.chars().count() < 6_000);

        let cloud = render_web_investigation_instruction(&policy, &large_skill, false);
        assert!(cloud.len() > compact.len());
        assert!(cloud.contains("Effective capability manifest"));
    }

    #[test]
    fn local_strix_process_disables_duplicate_timeout_retries() {
        let local = StrixRuntimeEnv {
            llm: "openai/Local-9B".into(),
            api_key: "local".into(),
            api_base: "http://127.0.0.1:8000/v1".into(),
            image: "strix:latest".into(),
            deployment: "local".into(),
            full_power: false,
            prompt_audit_mode: "off".into(),
        };
        let mut command = Command::new("true");
        command_strix_env(&mut command, &local);
        let debug = format!("{command:?}");
        assert!(debug.contains("LLM_TIMEOUT=\"86400\""));
        assert!(debug.contains("LLM_STREAM_IDLE_TIMEOUT=\"86400\""));
        assert!(debug.contains("STRIX_LLM_MAX_RETRIES=\"0\""));
        assert!(debug.contains("STRIX_MEMORY_COMPRESSOR_TIMEOUT=\"14400\""));
        assert!(debug.contains("STRIX_TELEMETRY=\"0\""));

        let root =
            std::env::temp_dir().join(format!("oviraptor-local-strix-runtime-{}", Uuid::new_v4()));
        let runtime = write_strix_runtime_config(&root, &local, None).unwrap();
        let payload: JsonValue =
            serde_json::from_slice(&fs::read(runtime.path()).unwrap()).unwrap();
        assert_eq!(
            payload
                .pointer("/env/LLM_TIMEOUT")
                .and_then(JsonValue::as_str),
            Some("86400")
        );
        assert_eq!(
            payload
                .pointer("/env/STRIX_LLM_MAX_RETRIES")
                .and_then(JsonValue::as_str),
            Some("0")
        );
        assert_eq!(
            payload
                .pointer("/env/STRIX_TELEMETRY")
                .and_then(JsonValue::as_str),
            Some("0")
        );
        drop(runtime);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn self_hosted_profile_requires_an_explicit_local_base_url() {
        let root =
            std::env::temp_dir().join(format!("asset-atlas-strix-local-url-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let settings = serde_json::json!({
            "strixActiveLlmProfileId": "local",
            "strixLlmProfiles": [{
                "id": "local",
                "deployment": "local",
                "llm": "openai/local-model",
                "apiBase": "",
                "apiKey": ""
            }]
        });

        let error = strix_runtime_env(&settings, &root)
            .err()
            .unwrap_or_default();
        assert!(error.contains("OPENAI_BASE_URL"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn prompt_audit_modes_store_only_the_selected_capture_level() {
        let root =
            std::env::temp_dir().join(format!("asset-atlas-prompt-audit-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let instruction = "Authorized test\nAuthorization: should-not-be-stored";
        let mut environment = StrixRuntimeEnv {
            llm: "openai/local-model".into(),
            api_key: "local".into(),
            api_base: "http://127.0.0.1:11434/v1".into(),
            image: DEFAULT_STRIX_SANDBOX_IMAGE.into(),
            deployment: "local".into(),
            full_power: true,
            prompt_audit_mode: "off".into(),
        };

        write_strix_prompt_audit(&root, instruction, &environment).unwrap();
        assert!(!root.join("strix-prompt-audit.json").exists());

        environment.prompt_audit_mode = "metadata".into();
        write_strix_prompt_audit(&root, instruction, &environment).unwrap();
        let metadata: StrixPromptAudit =
            serde_json::from_slice(&fs::read(root.join("strix-prompt-audit.json")).unwrap())
                .unwrap();
        assert_eq!(metadata.capture_level, "generated_instruction");
        assert!(!metadata.exact_model_request);
        assert!(metadata.instruction.is_none());
        assert_eq!(
            metadata.instruction_chars,
            instruction.chars().count() as i64
        );

        environment.prompt_audit_mode = "full".into();
        write_strix_prompt_audit(&root, instruction, &environment).unwrap();
        let full: StrixPromptAudit =
            serde_json::from_slice(&fs::read(root.join("strix-prompt-audit.json")).unwrap())
                .unwrap();
        assert!(!full.exact_model_request);
        assert!(full
            .instruction
            .as_deref()
            .unwrap_or_default()
            .contains("should-not-be-stored"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn database_migrates_legacy_strix_settings_to_model_profile() {
        let root =
            std::env::temp_dir().join(format!("asset-atlas-strix-migration-{}", Uuid::new_v4()));
        let db_path = db::initialize(&root).unwrap();
        {
            let connection = db::open(&db_path).unwrap();
            connection
                .execute(
                    "UPDATE config_profiles SET settings_json=json_remove(json_set(settings_json,'$.strixLlm','openai/legacy-model','$.strixApiBase','https://legacy.example.invalid/v1','$.strixApiKey','legacy-key'),'$.strixLlmProfiles','$.strixActiveLlmProfileId')",
                    [],
                )
                .unwrap();
        }

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
        assert_eq!(
            settings
                .pointer("/strixLlmProfiles/0/id")
                .and_then(JsonValue::as_str),
            Some("legacy-default")
        );
        assert_eq!(
            settings
                .pointer("/strixLlmProfiles/0/llm")
                .and_then(JsonValue::as_str),
            Some("openai/legacy-model")
        );
        assert_eq!(
            settings
                .get("strixActiveLlmProfileId")
                .and_then(JsonValue::as_str),
            Some("legacy-default")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn database_removes_retired_local_context_overrides() {
        let root = std::env::temp_dir().join(format!(
            "oviraptor-mlx-context-migration-{}",
            Uuid::new_v4()
        ));
        let db_path = db::initialize(&root).unwrap();
        {
            let connection = db::open(&db_path).unwrap();
            connection.execute(
                r#"UPDATE config_profiles SET settings_json=json_set(
                    settings_json,
                    '$.strixActiveLlmProfileId','local-profile',
                    '$.strixLlmProfiles',json('[{"id":"local-profile","name":"MLX","deployment":"local","llm":"openai/local","apiBase":"http://127.0.0.1:18080/v1","contextWindow":40960,"maxOutputTokens":4096}]')
                )"#,
                [],
            ).unwrap();
        }

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
        assert_eq!(
            settings
                .pointer("/strixActiveLlmProfileId")
                .and_then(JsonValue::as_str),
            Some("local-profile")
        );
        assert!(settings
            .pointer("/strixLlmProfiles/0/contextWindow")
            .is_none());
        assert!(settings
            .pointer("/strixLlmProfiles/0/maxOutputTokens")
            .is_none());
        assert_eq!(
            settings
                .pointer("/strixLlmProfiles/0/apiBase")
                .and_then(JsonValue::as_str),
            Some("http://127.0.0.1:18080/v1")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn associated_strix_sync_preserves_and_repairs_native_task_state() {
        let root = std::env::temp_dir().join(format!("asset-atlas-associated-{}", Uuid::new_v4()));
        let app_dir = root.join("oviraptor");
        let runs = root.join("runs");
        let run_dir = runs.join("limited-run");
        fs::create_dir_all(&run_dir).unwrap();
        let db_path = db::initialize(&app_dir).unwrap();
        let connection = db::open(&db_path).unwrap();
        connection
            .execute(
                "UPDATE config_profiles SET settings_json=json_set(settings_json,'$.strixRunsDirectory',?1)",
                [runs.to_string_lossy().to_string()],
            )
            .unwrap();
        connection
            .execute("INSERT INTO projects(id,name) VALUES(1,'test')", [])
            .unwrap();
        connection.execute("INSERT INTO sentinel_scans(id,project_id,project_name,status,current_checkpoint,task_path,scan_type) VALUES('parent',1,'test','partial','Strix 实时 · 50.0 KB 事件 · 0 个漏洞 · 100 Token','native-task.json','web')", []).unwrap();
        connection.execute("INSERT INTO sentinel_targets(project_id,scan_id,company,url,status,value_score,scan_mode,routing_reason) VALUES(1,'parent','test','https://example.invalid','partial',100,'deep','高价值前端；自动熔断：连续 12 次模型调用无有效工具')", []).unwrap();
        connection.execute("INSERT INTO sentinel_processes(scan_id,process_id,engine,work_dir) VALUES('parent',999999,'strix-adaptive','/tmp')", []).unwrap();
        fs::write(run_dir.join(".asset-atlas-scan-id"), "parent").unwrap();
        fs::write(
            run_dir.join("run.json"),
            serde_json::json!({
                "run_id":"limited-run",
                "status":"interrupted",
                "targets_info":[{"original":"https://example.invalid"}],
                "llm_usage":{"requests":12,"total_tokens":100}
            })
            .to_string(),
        )
        .unwrap();
        let state = AppState {
            db_path: db_path.clone(),
            app_data_dir: app_dir,
            legacy_icon_dirs: vec![root.join("legacy")],
            export_dir: root.join("exports"),
            cancellations: Arc::new(Mutex::new(HashMap::new())),
            active_jobs: Arc::new(AtomicUsize::new(0)),
            worker_service: crate::worker::WorkerServiceControl::default(),
        };

        assert_eq!(sync_strix_results(&connection, &state).unwrap(), 1);
        let (status, checkpoint): (String, String) = connection
            .query_row(
                "SELECT status,current_checkpoint FROM sentinel_scans WHERE id='parent'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "partial");
        assert!(checkpoint.contains("任务累计状态"));
        assert!(checkpoint.contains("报错细节"));
        assert!(checkpoint.contains("https://example.invalid"));
        assert!(checkpoint.contains("连续 12 次模型调用无有效工具"));
        assert!(!checkpoint.starts_with("Strix 实时"));
        let target_status: String = connection
            .query_row(
                "SELECT status FROM sentinel_targets WHERE scan_id='parent'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(target_status, "limited");
        let fuse_entries: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sentinel_fuse_zone WHERE source_scan_id='parent'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(fuse_entries, 1);
        let processes: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sentinel_processes WHERE scan_id='parent'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(processes, 0);
        connection
            .execute(
                "UPDATE sentinel_scans SET current_checkpoint='扫描异常结束：自动验证 0，保留部分结果 0，熔断 1' WHERE id='parent'",
                [],
            )
            .unwrap();
        assert_eq!(sync_strix_results(&connection, &state).unwrap(), 0);
        let repaired_checkpoint: String = connection
            .query_row(
                "SELECT current_checkpoint FROM sentinel_scans WHERE id='parent'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(repaired_checkpoint.contains("报错细节"));
        assert!(repaired_checkpoint.contains("连续 12 次模型调用无有效工具"));
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn parses_git_checkout_progress_without_requiring_lfs_output() {
        assert_eq!(
            git_progress_percent("Updating files: 17% (10362/58827)"),
            Some(17)
        );
        assert_eq!(
            git_progress_percent("Receiving objects: 100% (25/25), done."),
            Some(100)
        );
        assert_eq!(
            git_progress_percent("git-lfs filter-process: git-lfs not found"),
            None
        );
    }

    #[test]
    fn project_impact_blocks_strix_only_workspace_deletion() {
        let root =
            std::env::temp_dir().join(format!("oviraptor-project-impact-{}", Uuid::new_v4()));
        let db_path = db::initialize(&root).unwrap();
        let connection = db::open(&db_path).unwrap();
        connection
            .execute("INSERT INTO projects(id,name) VALUES(91,'Strix only')", [])
            .unwrap();
        connection.execute("INSERT INTO sentinel_scans(id,project_id,project_name,status,scan_type) VALUES('impact-scan',91,'Strix only','completed','code')", []).unwrap();
        connection.execute("INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title,severity) VALUES('impact-scan','/repo','strix','vulnerability','v-1','Test finding','high')", []).unwrap();
        connection.execute("INSERT INTO sentinel_validations(scan_id,url,finding_key,finding_kind,verdict,severity) VALUES('impact-scan','/repo','strix:vulnerability:v-1','vulnerability','true_positive','high')", []).unwrap();
        connection
            .execute(
                "INSERT INTO saved_views(project_id,name) VALUES(91,'Only view')",
                [],
            )
            .unwrap();
        let impact = project_impact_for_connection(&connection, 91).unwrap();
        assert_eq!(impact.asset_count, 0);
        assert_eq!(impact.sentinel_scan_count, 1);
        assert_eq!(impact.finding_count, 1);
        assert_eq!(impact.validation_count, 1);
        assert_eq!(impact.saved_view_count, 1);
        assert!(impact.total_records >= 4);
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }
