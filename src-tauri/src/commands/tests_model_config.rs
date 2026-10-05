    #[test]
    fn retired_model_profiles_cannot_activate_even_with_an_explicit_selection() {
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

        assert!(model_runtime_env(&settings).err().unwrap().contains("模型未配置"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn missing_active_native_profile_falls_back_to_first_current_profile() {
        let root =
            std::env::temp_dir().join(format!("asset-atlas-strix-fallback-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let settings = serde_json::json!({
            "activeModelProfileId": "missing-profile",
            "modelProfiles": [{
                "id": "profile-a",
                "llm": "openai/model-a",
                "apiKey": "key-a",
                "apiBase": "https://a.example.invalid/v1"
            }]
        });

        let environment = model_runtime_env(&settings).unwrap();
        assert_eq!(environment.llm, "openai/model-a");
        assert_eq!(environment.api_key, "key-a");
        assert_eq!(environment.api_base, "https://a.example.invalid/v1");
        let _ = fs::remove_dir_all(root);
    }

    /// Current configuration is authoritative; retired spellings are discarded,
    /// never read as fallback credentials or a model selection.
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

        let environment = model_runtime_env(&settings).unwrap();
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
            "modelName": "openai/current-name",
            "strixLlm": "openai/retired-name"
        });

        let environment = model_runtime_env(&settings).unwrap();
        assert_eq!(environment.deployment, "local");
        assert_eq!(environment.llm, "openai/current-name");
        assert_eq!(environment.api_base, "http://127.0.0.1:18080/v1");
        assert_eq!(environment.api_key, "local-token");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn current_profiles_without_deployment_remain_cloud_governed() {
        let root = std::env::temp_dir().join(format!("asset-atlas-strix-cloud-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let settings = serde_json::json!({
            "activeModelProfileId": "cloud",
            "strixLocalFullPower": true,
            "modelProfiles": [{
                "id": "cloud",
                "llm": "openai/cloud-model",
                "apiKey": "cloud-key"
            }]
        });

        let environment = model_runtime_env(&settings).unwrap();
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
            "activeModelProfileId": "local",
            "agentLocalFullPower": true,
            "agentPromptAuditMode": "full",
            "modelProfiles": [{
                "id": "local",
                "deployment": "local",
                "llm": "openai/local-model",
                "apiBase": "http://127.0.0.1:11434/v1",
                "apiKey": ""
            }]
        });

        let environment = model_runtime_env(&settings).unwrap();
        assert_eq!(environment.deployment, "local");
        assert!(environment.full_power);
        assert_eq!(environment.api_key, "local");
        assert_eq!(environment.prompt_audit_mode, "full");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn self_hosted_profile_uses_only_its_separate_optional_key() {
        let root = std::env::temp_dir().join(format!("oviraptor-local-key-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let settings = serde_json::json!({
            "activeModelProfileId": "local",
            "modelProfiles": [{
                "id": "local",
                "deployment": "local",
                "llm": "openai/local-model",
                "apiBase": "http://127.0.0.1:18080/v1",
                "apiKey": "stale-cloud-key",
                "localApiKey": "self-hosted-key"
            }]
        });
        let environment = model_runtime_env(&settings).unwrap();
        assert_eq!(environment.api_key, "self-hosted-key");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn local_model_policy_serializes_large_models_and_keeps_small_models_bounded() {
        let large = ModelRuntimeEnv {
            llm: "openai/Qwen3-27B-Instruct".into(),
            api_key: "local".into(),
            api_base: "http://127.0.0.1:11434/v1".into(),
            deployment: "local".into(),
            full_power: true,
            prompt_audit_mode: "off".into(),
        };
        let large_policy = local_model_runtime_policy(&large);
        assert_eq!(large_policy.parameter_billions, Some(27));
        assert_eq!(large_policy.max_concurrent_requests, 1);
        assert_eq!(large_policy.max_output_tokens, Some(3_072));
        assert_eq!(model_startup_timeouts(&large), (240, 1_200));

        let large_on_64 = local_model_runtime_policy_for_memory(&large, 64);
        assert_eq!(large_on_64.max_context_tokens, 65_536);
        assert_eq!(large_on_64.memory_guard_tier, "balanced");
        assert_eq!(large_on_64.frontend_packet_budget_bytes, 12 * 1024);

        let small = ModelRuntimeEnv {
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

        let moe = ModelRuntimeEnv {
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
        assert!(compact.contains("Native Agent"));
        assert!(!compact.to_ascii_lowercase().contains("strix"));
        assert!(compact.chars().count() < 6_000);

        let cloud = render_web_investigation_instruction(&policy, &large_skill, false);
        assert!(cloud.len() > compact.len());
        assert!(cloud.contains("Effective capability manifest"));
        assert!(cloud.contains("Native Agent"));
        assert!(!cloud.to_ascii_lowercase().contains("strix"));
    }

    #[test]
    fn self_hosted_profile_requires_an_explicit_local_base_url() {
        let root =
            std::env::temp_dir().join(format!("asset-atlas-strix-local-url-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let settings = serde_json::json!({
            "activeModelProfileId": "local",
            "modelProfiles": [{
                "id": "local",
                "deployment": "local",
                "llm": "openai/local-model",
                "apiBase": "",
                "apiKey": ""
            }]
        });

        let error = model_runtime_env(&settings)
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
        let mut environment = ModelRuntimeEnv {
            llm: "openai/local-model".into(),
            api_key: "local".into(),
            api_base: "http://127.0.0.1:11434/v1".into(),
            deployment: "local".into(),
            full_power: true,
            prompt_audit_mode: "off".into(),
        };

        write_model_prompt_audit(&root, instruction, &environment).unwrap();
        assert!(!root.join("model-prompt-audit.json").exists());

        environment.prompt_audit_mode = "metadata".into();
        write_model_prompt_audit(&root, instruction, &environment).unwrap();
        let metadata: ModelPromptAudit =
            serde_json::from_slice(&fs::read(root.join("model-prompt-audit.json")).unwrap())
                .unwrap();
        assert_eq!(metadata.capture_level, "generated_instruction");
        assert!(!metadata.exact_model_request);
        assert!(metadata.instruction.is_none());
        assert_eq!(
            metadata.instruction_chars,
            instruction.chars().count() as i64
        );

        environment.prompt_audit_mode = "full".into();
        write_model_prompt_audit(&root, instruction, &environment).unwrap();
        let full: ModelPromptAudit =
            serde_json::from_slice(&fs::read(root.join("model-prompt-audit.json")).unwrap())
                .unwrap();
        assert!(!full.exact_model_request);
        assert!(full
            .instruction
            .as_deref()
            .unwrap_or_default()
            .contains("Authorized test"));
        assert!(!full
            .instruction
            .as_deref()
            .unwrap_or_default()
            .contains("should-not-be-stored"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn database_preserves_retired_settings_without_activating_a_native_model() {
        let root =
            std::env::temp_dir().join(format!("oviraptor-settings-retirement-{}", Uuid::new_v4()));
        let db_path = db::initialize(&root).unwrap();
        {
            let connection = db::open(&db_path).unwrap();
            connection
                .execute(
                    "UPDATE config_profiles SET settings_json=json_object('strixLlm','openai/legacy-model','strixApiBase','https://legacy.example.invalid/v1','strixApiKey','legacy-key')",
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
        for key in ["modelProfiles", "modelName", "modelApiBase", "modelApiKey", "activeModelProfileId"] {
            assert!(settings.get(key).is_none(), "retired config created {key}");
        }
        assert!(model_runtime_env(&settings).err().unwrap().contains("模型未配置"));
        for legacy_key in ["strixLlm", "strixApiBase", "strixApiKey"] {
            assert!(settings.get(legacy_key).is_some(), "startup deleted {legacy_key}");
        }
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
                    json_remove(settings_json,'$.modelProfiles','$.activeModelProfileId','$.modelDeployment','$.modelApiBase','$.modelApiKey','$.localApiKey'),
                    '$.activeModelProfileId','local-profile',
                    '$.modelProfiles',json('[{"id":"local-profile","name":"MLX","deployment":"local","llm":"openai/local","apiBase":"http://127.0.0.1:18080/v1","contextWindow":40960,"maxOutputTokens":4096}]')
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
                .pointer("/activeModelProfileId")
                .and_then(JsonValue::as_str),
            Some("local-profile")
        );
        assert!(settings
            .pointer("/modelProfiles/0/contextWindow")
            .is_none());
        assert!(settings
            .pointer("/modelProfiles/0/maxOutputTokens")
            .is_none());
        assert_eq!(
            settings
                .pointer("/modelProfiles/0/apiBase")
                .and_then(JsonValue::as_str),
            Some("http://127.0.0.1:18080/v1")
        );
        assert!(settings.get("strixLlmProfiles").is_none());
        assert!(settings.get("strixActiveLlmProfileId").is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn historical_import_cannot_repair_or_mutate_native_task_state() {
        let root = std::env::temp_dir().join(format!("asset-atlas-associated-{}", Uuid::new_v4()));
        let app_dir = root.join("oviraptor");
        let runs = root.join("runs");
        let run_dir = runs.join("limited-run");
        fs::create_dir_all(&run_dir).unwrap();
        let db_path = db::initialize(&app_dir).unwrap();
        let connection = db::open(&db_path).unwrap();
        connection
            .execute("INSERT INTO projects(id,name) VALUES(1,'test')", [])
            .unwrap();
        connection.execute("INSERT INTO sentinel_scans(id,project_id,project_name,status,current_checkpoint,task_path,scan_type) VALUES('parent',1,'test','partial','Strix 实时 · 50.0 KB 事件 · 0 个漏洞 · 100 Token','native-task.json','web')", []).unwrap();
        connection.execute("INSERT INTO sentinel_targets(project_id,scan_id,company,url,status,value_score,scan_mode,routing_reason) VALUES(1,'parent','test','https://example.invalid','partial',100,'deep','高价值前端；自动熔断：连续 12 次模型调用无有效工具')", []).unwrap();
        connection.execute("INSERT INTO sentinel_processes(scan_id,process_id,engine,work_dir) VALUES('parent',999999,'strix-adaptive','/tmp')", []).unwrap();
        fs::write(run_dir.join(".asset-atlas-scan-id"), "parent").unwrap();
        fs::write(
            run_dir.join("model-prompt-audit.json"),
            serde_json::json!({
                "instruction":"read-only report must not repair Native task state"
            })
            .to_string(),
        )
        .unwrap();

        assert_eq!(artifact_import_report(&connection, &app_dir.join("artifact-cas"),
            &app_dir.join("artifact-import.key"), std::slice::from_ref(&runs)).unwrap().imported, 1);
        let (status, checkpoint): (String, String) = connection
            .query_row(
                "SELECT status,current_checkpoint FROM sentinel_scans WHERE id='parent'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "partial");
        assert_eq!(checkpoint, "Strix 实时 · 50.0 KB 事件 · 0 个漏洞 · 100 Token");
        let target_status: String = connection
            .query_row(
                "SELECT status FROM sentinel_targets WHERE scan_id='parent'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(target_status, "partial");
        let fuse_entries: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sentinel_fuse_zone WHERE source_scan_id='parent'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(fuse_entries, 0);
        let processes: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sentinel_processes WHERE scan_id='parent'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(processes, 1);
        let imported = historical_import_runs(&connection, None).unwrap();
        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].scan_id, "parent");
        connection
            .execute(
                "UPDATE sentinel_scans SET current_checkpoint='扫描异常结束：自动验证 0，保留部分结果 0，熔断 1' WHERE id='parent'",
                [],
            )
            .unwrap();
        assert_eq!(artifact_import_report(&connection, &app_dir.join("artifact-cas"),
            &app_dir.join("artifact-import.key"), std::slice::from_ref(&runs)).unwrap().imported, 0);
        let repaired_checkpoint: String = connection
            .query_row(
                "SELECT current_checkpoint FROM sentinel_scans WHERE id='parent'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(repaired_checkpoint, "扫描异常结束：自动验证 0，保留部分结果 0，熔断 1");
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
    #[test]
    fn model_runtime_honors_the_settings_written_by_the_configuration_ui() {
        let settings = serde_json::json!({
            "modelName":"local/test", "modelDeployment":"local",
            "modelApiBase":"http://127.0.0.1:18080/v1",
            "agentLocalFullPower":true, "agentPromptAuditMode":"full"
        });
        let environment = model_runtime_env(&settings).unwrap();
        assert!(environment.full_power);
        assert_eq!(environment.prompt_audit_mode, "full");
        let mut disabled = settings;
        disabled["agentLocalFullPower"] = serde_json::json!(false);
        disabled["agentPromptAuditMode"] = serde_json::json!("off");
        disabled["strixLocalFullPower"] = serde_json::json!(true);
        disabled["strixPromptAuditMode"] = serde_json::json!("full");
        let environment = model_runtime_env(&disabled).unwrap();
        assert!(!environment.full_power, "explicit neutral false must win over old true");
        assert_eq!(environment.prompt_audit_mode, "off");
    }

    #[test]
    fn selected_model_profile_cannot_resurrect_flat_credentials_or_endpoint() {
        for empty_key in [serde_json::json!(""), JsonValue::Null] {
            let mut settings = serde_json::json!({
                "modelName":"stale-model", "modelApiBase":"http://127.0.0.1:18081/v1",
                "localApiKey":"stale-local-key", "modelApiKey":"stale-cloud-key",
                "modelProfiles":[{"id":"current", "llm":"current-model", "deployment":"local",
                    "apiBase":"http://127.0.0.1:18080/v1", "localApiKey":empty_key}],
                "activeModelProfileId":"current"
            });
            let environment = model_runtime_env(&settings).unwrap();
            assert_eq!(environment.api_key, "local", "cleared profile key must not borrow the previous model's key");
            assert_eq!(environment.llm, "current-model");
            settings["modelProfiles"][0]["apiBase"] = serde_json::json!("");
            assert!(model_runtime_env(&settings).is_err(), "cleared endpoint must not borrow the previous model's URL");
            settings["modelProfiles"][0]["llm"] = serde_json::json!("");
            assert!(model_runtime_env(&settings).err().unwrap().contains("模型未配置"));
        }
    }

    #[test]
    fn cleared_profile_list_cannot_resurrect_the_flat_model_cache() {
        for profiles in [serde_json::json!([]), JsonValue::Null, serde_json::json!({})] {
            let settings = serde_json::json!({
                "modelProfiles":profiles, "modelName":"stale-model", "modelDeployment":"local",
                "modelApiBase":"http://127.0.0.1:18080/v1", "localApiKey":"stale-key",
                "strixLlmProfiles":[{"llm":"old-model"}]
            });
            assert!(model_runtime_env(&settings).err().unwrap().contains("模型未配置"));
        }
    }
    #[test]
    fn configuration_api_normalizes_on_read_and_write_and_rolls_back_failures() {
        let mut connection = rusqlite::Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE config_profiles(id INTEGER PRIMARY KEY, name TEXT NOT NULL, description TEXT NOT NULL,
            is_default INTEGER NOT NULL, settings_json TEXT NOT NULL, created_at TEXT NOT NULL DEFAULT '', updated_at TEXT NOT NULL DEFAULT '');").unwrap();
        let input = |id, settings| ConfigProfileInput {
            id, name: "profile".into(), description: "".into(), is_default: true, settings,
        };
        let legacy = serde_json::json!({"strixLlm":"model","strixApiKey":"old-key","strixRunsDirectory":"/archive",
            "agentLocalFullPower":false,"strixLocalFullPower":true});
        let id = persist_config_profile(&mut connection, input(None, legacy.clone())).unwrap();
        let profiles = read_config_profiles(&connection).unwrap();
        let expected = serde_json::json!({"agentLocalFullPower":false});
        assert_eq!(profiles[0].settings, expected);
        let stored: String = connection.query_row("SELECT settings_json FROM config_profiles WHERE id=?1", [id], |row| row.get(0)).unwrap();
        assert_eq!(serde_json::from_str::<JsonValue>(&stored).unwrap(), expected);

        // An imported profile is normalized even before the next launch migration.
        connection.execute("UPDATE config_profiles SET settings_json=?1 WHERE id=?2", params![legacy.to_string(), id]).unwrap();
        assert_eq!(read_config_profiles(&connection).unwrap()[0].settings, expected);
        assert!(persist_config_profile(&mut connection, input(Some(id + 1), serde_json::json!({}))).is_err());
        assert!(persist_config_profile(&mut connection, input(Some(id), serde_json::json!([]))).is_err());
        connection.execute_batch("CREATE TRIGGER reject_save BEFORE UPDATE OF settings_json ON config_profiles
            BEGIN SELECT RAISE(ABORT,'injected save failure'); END;").unwrap();
        assert!(persist_config_profile(&mut connection, input(Some(id), serde_json::json!({}))).unwrap_err().contains("injected save failure"));
        let (is_default, unchanged): (i64, String) = connection.query_row("SELECT is_default,settings_json FROM config_profiles WHERE id=?1", [id], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
        assert_eq!(is_default, 1, "failed saves must not clear the default profile");
        assert_eq!(serde_json::from_str::<JsonValue>(&unchanged).unwrap(), legacy);
    }
