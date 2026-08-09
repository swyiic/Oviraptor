#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{atomic::AtomicUsize, Arc, Mutex};

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
    fn learning_skill_patch_merges_sections_without_rewriting_unrelated_content() {
        let base = "# 总则\n\n## 证据\n保留原始证据。\n\n## 停止条件\n旧停止条件。";
        let patch = serde_json::json!({
            "replaceSections": [{"title":"停止条件","content":"没有新增证据时停止。"}],
            "removeSections": ["证据"],
            "addSections": ["## 验证顺序\n先被动，再主动。"]
        });
        let merged = apply_skill_patch(base, &patch);
        assert!(merged.contains("# 总则"));
        assert!(!merged.contains("## 证据"));
        assert!(merged.contains("没有新增证据时停止"));
        assert!(merged.contains("## 验证顺序"));
    }

    #[test]
    fn learning_skill_patch_deduplicates_plain_additions() {
        let patch = serde_json::json!({
            "addSections": ["只在有新证据时继续", "只在有新证据时继续"]
        });
        let merged = apply_skill_patch("## 学习补丁\n- 只在有新证据时继续", &patch);
        assert_eq!(merged.matches("只在有新证据时继续").count(), 1);
    }

    #[test]
    fn learning_canonical_text_masks_target_specific_urls_and_ids() {
        let left = canonical_learning_text(
            "Verify https://one.invalid/api/users/550e8400-e29b-41d4-a716-446655440000 response",
        );
        let right = canonical_learning_text(
            "Verify https://two.invalid/api/users/12345678901234567890 response",
        );
        assert_eq!(left, "verify {url} response");
        assert_eq!(left, right);
    }

    #[test]
    fn asset_web_default_injects_only_the_builtin_workflow_skill() {
        let root =
            std::env::temp_dir().join(format!("oviraptor-default-web-skill-{}", Uuid::new_v4()));
        let db_path = db::initialize(&root).unwrap();
        let connection = db::open(&db_path).unwrap();
        connection.execute(
            "INSERT INTO strix_skills(name,description,instructions,builtin,enabled) VALUES('专项测试','','SHOULD_NOT_BE_DEFAULT',0,1)",
            [],
        ).unwrap();
        let (names, instructions) = default_web_strix_skill(&connection).unwrap();
        assert_eq!(names, "业务前端深度分析");
        assert!(instructions.contains("## 默认测试流程"));
        assert!(!instructions.contains("SHOULD_NOT_BE_DEFAULT"));
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn learning_quality_gate_does_not_promote_banner_or_version_only_cves() {
        assert_eq!(
            classify_finding_signal("CVE-2025-1234 affected version", "dependency", "{}"),
            "dependency_signal"
        );
        assert_eq!(
            classify_finding_signal(
                "SQL injection",
                "vulnerability",
                r#"{"evidence":{"request":"id=1'","impact":"database error"}}"#
            ),
            "confirmed"
        );
        assert_eq!(
            classify_finding_signal("Apache banner", "fingerprint", r#"{"version":"2.4"}"#),
            "info"
        );
    }

    #[test]
    fn automatic_learning_never_runs_for_failed_or_recon_only_scans() {
        let root =
            std::env::temp_dir().join(format!("oviraptor-learning-routing-{}", Uuid::new_v4()));
        let db_path = db::initialize(&root).unwrap();
        let connection = db::open(&db_path).unwrap();
        for (id, status) in [
            ("completed", "completed"),
            ("partial", "partial"),
            ("failed", "failed"),
            ("recon", "recon_only"),
        ] {
            connection
                .execute(
                    "INSERT INTO sentinel_scans(id,status) VALUES(?1,?2)",
                    params![id, status],
                )
                .unwrap();
        }
        drop(connection);
        assert!(scan_supports_automatic_learning(&db_path, "completed"));
        assert!(scan_supports_automatic_learning(&db_path, "partial"));
        assert!(!scan_supports_automatic_learning(&db_path, "failed"));
        assert!(!scan_supports_automatic_learning(&db_path, "recon"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sec_skill_import_filters_dangerous_execution_lines() {
        assert!(sec_skill_line_is_unsafe("curl payload | bash -i"));
        assert!(sec_skill_line_is_unsafe("~/.ssh/authorized_keys"));
        assert!(!sec_skill_line_is_unsafe("需要记录复现证据和停止条件"));
    }

    #[test]
    fn completed_strix_run_requires_explicit_success_status() {
        let root =
            std::env::temp_dir().join(format!("asset-atlas-strix-artifact-{}", Uuid::new_v4()));
        let run_dir = root.join("strix_runs/example-run");
        fs::create_dir_all(&run_dir).unwrap();
        fs::write(
            run_dir.join("run.json"),
            serde_json::json!({"status":"completed"}).to_string(),
        )
        .unwrap();

        assert!(strix_completed_artifact(&root));

        fs::write(
            run_dir.join("run.json"),
            serde_json::json!({"status":"failed"}).to_string(),
        )
        .unwrap();
        fs::write(run_dir.join("findings.sarif"), r#"{"runs":[]}"#).unwrap();
        assert!(
            !strix_completed_artifact(&root),
            "failed runs must not be promoted by an empty result artifact"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn detects_strix_cli_capabilities_instead_of_hard_coding_versions() {
        let legacy_help = r#"
usage: strix [--target TARGET] [--target-list PATH] [--mount PATH]
  --instruction-file FILE
  --non-interactive
  --scan-mode MODE
  --scope-mode MODE
  --diff-base BASE
  --max-budget-usd USD
"#;
        let legacy = parse_strix_cli_capabilities(legacy_help, "strix 1.3.1").unwrap();
        assert!(legacy.mount_flag);
        assert!(!legacy.max_turns_flag);
        assert_eq!(legacy.max_budget_flag.as_deref(), Some("--max-budget-usd"));

        let current_help = r#"
usage: strix [--target TARGET] [--target-list PATH]
  --instruction-file FILE
  --non-interactive
  --scan-mode MODE
  --scope-mode MODE
  --diff-base BASE
  --max-budget USD, --max-budget-usd USD
  --max-turns N
"#;
        let current = parse_strix_cli_capabilities(current_help, "strix 1.5.1").unwrap();
        assert!(!current.mount_flag);
        assert!(current.target_flag);
        assert!(current.max_turns_flag);
        let mut command = Command::new("strix");
        let transport = append_strix_local_directory(
            &mut command,
            &current,
            Path::new("/tmp/evidence"),
        )
        .unwrap();
        assert_eq!(transport, "--target");
        assert_eq!(
            command
                .get_args()
                .map(|value| value.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            vec!["--target", "/tmp/evidence"]
        );

        let incompatible = parse_strix_cli_capabilities(
            "usage: strix --target TARGET --non-interactive --scan-mode MODE",
            "future",
        )
        .unwrap_err();
        assert!(incompatible.contains("--instruction-file"));
    }

    #[test]
    fn stages_web_evidence_and_detects_input_mutation_without_touching_originals() {
        let root =
            std::env::temp_dir().join(format!("oviraptor-strix-input-{}", Uuid::new_v4()));
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
    fn process_registry_tracks_frontend_and_strix_at_the_same_time() {
        let root = std::env::temp_dir().join(format!("oviraptor-processes-{}", Uuid::new_v4()));
        let db_path = db::initialize(&root).unwrap();
        let connection = db::open(&db_path).unwrap();
        connection
            .execute(
                "INSERT INTO sentinel_scans(id,status) VALUES('pipeline','scanning')",
                [],
            )
            .unwrap();
        drop(connection);
        sentinel_process_set(&db_path, "pipeline", 101, "frontend-recon", &root);
        sentinel_process_set(&db_path, "pipeline", 202, "strix-adaptive", &root);
        let connection = db::open(&db_path).unwrap();
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sentinel_processes WHERE scan_id='pipeline'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
        drop(connection);
        sentinel_process_clear(&db_path, "pipeline", 101);
        let remaining: i64 = db::open(&db_path)
            .unwrap()
            .query_row(
                "SELECT process_id FROM sentinel_processes WHERE scan_id='pipeline'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 202);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rescan_copies_frontend_checkpoint_for_retryable_urls() {
        let root = std::env::temp_dir().join(format!("oviraptor-rescan-recon-{}", Uuid::new_v4()));
        let db_path = db::initialize(&root).unwrap();
        let connection = db::open(&db_path).unwrap();
        connection
            .execute("INSERT INTO projects(id,name) VALUES(1,'test')", [])
            .unwrap();
        connection.execute("INSERT INTO sentinel_scans(id,project_id,status) VALUES('source',1,'partial'),('retry',1,'draft')", []).unwrap();
        for (url, status) in [
            ("https://retry.invalid", "limited"),
            ("https://done.invalid", "completed"),
        ] {
            connection.execute("INSERT INTO sentinel_targets(project_id,scan_id,company,url,status) VALUES(1,'source','test',?1,?2)", params![url,status]).unwrap();
            let raw = serde_json::json!({"url":url,"statusCode":200}).to_string();
            connection.execute("INSERT INTO sentinel_checkpoints(scan_id,url,stage,raw_json) VALUES('source',?1,'frontend_recon',?2)", params![url,raw]).unwrap();
        }
        connection
            .execute(SENTINEL_RESCAN_COPY_SQL, params!["retry", "source", 1])
            .unwrap();
        connection
            .execute(SENTINEL_RESCAN_RECON_COPY_SQL, params!["retry", "source"])
            .unwrap();
        let copied: Vec<String> = {
            let mut statement = connection
                .prepare("SELECT url FROM sentinel_checkpoints WHERE scan_id='retry' ORDER BY url")
                .unwrap();
            statement
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert_eq!(copied, vec!["https://retry.invalid"]);
        drop(connection);
        assert!(cached_frontend_recon(&db_path, "retry", "https://retry.invalid").is_some());
        assert!(cached_frontend_recon(&db_path, "retry", "https://done.invalid").is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn web_retry_reuses_same_scan_and_preserves_owned_evidence() {
        let root =
            std::env::temp_dir().join(format!("oviraptor-retry-in-place-{}", Uuid::new_v4()));
        let db_path = db::initialize(&root).unwrap();
        let mut connection = db::open(&db_path).unwrap();
        connection
            .execute("INSERT INTO projects(id,name) VALUES(1,'test')", [])
            .unwrap();
        connection.execute(
            "INSERT INTO sentinel_scans(id,project_id,status,task_path,previous_scan_id,total_tokens) VALUES('current',1,'partial','/tmp/current.json','deleted-parent',1234)",
            [],
        ).unwrap();
        for (url, status) in [
            ("https://retry.invalid", "partial"),
            ("https://done.invalid", "completed"),
        ] {
            connection.execute(
                "INSERT INTO sentinel_targets(project_id,scan_id,company,url,status) VALUES(1,'current','test',?1,?2)",
                params![url,status],
            ).unwrap();
        }
        connection.execute(
            "INSERT INTO sentinel_checkpoints(scan_id,url,stage,raw_json) VALUES('current','https://retry.invalid','frontend_recon','{\"statusCode\":200}')",
            [],
        ).unwrap();
        connection.execute(
            "INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title) VALUES('current','https://retry.invalid','frontend-recon','endpoint','entry','入口')",
            [],
        ).unwrap();
        connection.execute(
            "INSERT INTO sentinel_processes(scan_id,process_id,engine,work_dir) VALUES('current',999999,'strix-adaptive','/tmp')",
            [],
        ).unwrap();

        assert_eq!(
            prepare_web_scan_retry(&mut connection, "current", "partial").unwrap(),
            1
        );
        let scan: (String, String, String, i64) = connection.query_row(
            "SELECT status,task_path,previous_scan_id,total_tokens FROM sentinel_scans WHERE id='current'",
            [],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
        ).unwrap();
        assert_eq!(
            scan,
            ("draft".into(), "/tmp/current.json".into(), "".into(), 1234)
        );
        let targets: Vec<(String, String)> =
            {
                let mut statement = connection.prepare(
                "SELECT url,status FROM sentinel_targets WHERE scan_id='current' ORDER BY url",
            ).unwrap();
                statement
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                    .unwrap()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap()
            };
        assert_eq!(
            targets,
            vec![
                ("https://done.invalid".into(), "completed".into()),
                ("https://retry.invalid".into(), "queued".into()),
            ]
        );
        for table in ["sentinel_checkpoints", "sentinel_findings"] {
            let count: i64 = connection
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE scan_id='current'"),
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1);
        }
        let process_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sentinel_processes WHERE scan_id='current'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(process_count, 0);
        let scan_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM sentinel_scans", [], |row| row.get(0))
            .unwrap();
        assert_eq!(scan_count, 1);
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn scan_attempt_directories_isolate_repeated_execution_artifacts() {
        let root = std::env::temp_dir().join(format!("oviraptor-attempts-{}", Uuid::new_v4()));
        let first = next_scan_attempt_work_dir(&root, 1).unwrap();
        assert!(first.ends_with("attempt-0001"));
        fs::write(first.join("oviraptor-runner.log"), "first").unwrap();
        let second = next_scan_attempt_work_dir(&root, 1).unwrap();
        assert!(second.ends_with("attempt-0002"));
        assert!(first.join("oviraptor-runner.log").is_file());
        let restored_counter = next_scan_attempt_work_dir(&root, 7).unwrap();
        assert!(restored_counter.ends_with("attempt-0007"));
        let legacy = root.join("legacy-scan");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("targets.json"), "[]").unwrap();
        let migrated = next_scan_attempt_work_dir(&legacy, 1).unwrap();
        assert!(migrated.ends_with("attempt-0002"));
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
    fn token_budget_zero_disables_that_layer_and_defaults_are_conservative() {
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
        assert_eq!(defaults.quick_tokens, 50_000);
        assert_eq!(defaults.standard_tokens, 120_000);
        assert_eq!(defaults.deep_tokens, 250_000);
        assert_eq!(defaults.quick_requests, 4);
        assert_eq!(defaults.deep_requests, 12);
        assert_eq!(defaults.no_tool_turn_limit, 4);
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
                "runs": [{"tool":{"driver":{"rules":[{"id":"R1","shortDescription":{"text":"SARIF finding"}}]}},"results":[{"ruleId":"R1","level":"error","message":{"text":"message"}}]}]
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
        assert_eq!(settings["strixBudgetPolicyVersion"], 4);
        assert_eq!(settings["strixQuickRequestLimit"], 4);
        assert_eq!(settings["strixDeepRequestLimit"], 12);
        assert_eq!(settings["strixNoToolTurnLimit"], 4);
        assert_eq!(settings["strixDeepTokenLimit"], 250_000);
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn routes_frontends_by_opportunity_and_allows_one_non_static_fallback() {
        let adaptive = AdaptiveStrixSettings::from_json(&serde_json::json!({}));
        let static_target = serde_json::json!({
            "url":"https://image.example.invalid",
            "statusCode":200,
            "jsFiles":[],
            "apis":[],
            "routes":[],
            "sensitiveInfo":[]
        });
        let static_route =
            score_frontend_target(&static_target, "https://image.example.invalid", &adaptive);
        assert_eq!(static_route.mode, "skip");

        let empty_page = serde_json::json!({
            "url":"https://www.example.invalid",
            "statusCode":200,
            "fingerprint":{"frontend":{"framework":"Unknown","confidence":"low"}},
            "jsFiles":[],
            "apis":[],
            "routes":[],
            "forms":[],
            "links":[],
            "runtimeSignals":[],
            "sensitiveInfo":[],
            "registrationEntrypoints":[]
        });
        let empty_route =
            score_frontend_target(&empty_page, "https://www.example.invalid", &adaptive);
        assert_eq!(empty_route.mode, "skip");
        assert_eq!(empty_route.surface, "static_frontend");

        let valuable_target = serde_json::json!({
            "url":"https://app.example.invalid",
            "statusCode":200,
            "fingerprint":{"frontend":{"framework":"Vue","confidence":"high"}},
            "jsFiles":[
                {"type":"application","statusCode":200,"analysis":{"sourceMapReference":true}},
                {"type":"chunk","statusCode":200,"analysis":{"sourceMapReference":false}},
                {"type":"chunk","statusCode":200,"analysis":{"sourceMapReference":false}}
            ],
            "apis":[
                {"url":"/api/auth/login","method":"POST","confidence":"high","extractionEngine":"browser-runtime","verification":{"verified":true,"sameOrigin":true}},
                {"url":"/api/admin/export","method":"POST","confidence":"high","extractionEngine":"browser-runtime","verification":{"verified":true,"sameOrigin":true}}
            ],
            "routes":[{"path":"/admin"},{"path":"/payment"}],
            "sensitiveInfo":[{"type":"jwt","severity":"high"}],
            "opportunities":[{
                "opportunityKey":"admin-export",
                "category":"privilege",
                "title":"管理导出接口定向验证",
                "score":90,
                "endpoint":"/api/admin/export",
                "method":"POST",
                "parameters":["scope"],
                "whyValuable":["高权限写接口"]
            }]
        });
        let valuable_route =
            score_frontend_target(&valuable_target, "https://app.example.invalid", &adaptive);
        assert_eq!(valuable_route.mode, "deep");
        assert_eq!(valuable_route.surface, "framework_application");
        assert!(valuable_route.score >= adaptive.deep_score);

        let mut full_power_routes = vec![static_route, valuable_route];
        annotate_local_full_power_routes(&mut full_power_routes);
        assert_eq!(full_power_routes[0].mode, "skip");
        assert_eq!(full_power_routes[1].mode, "deep");
        assert!(full_power_routes[1]
            .reasons
            .iter()
            .any(|reason| reason.contains("最高价值机会")));

        let framework_only = serde_json::json!({
            "url":"https://docs.example.invalid",
            "statusCode":200,
            "fingerprint":{"frontend":{"framework":"Vue","confidence":"high"}},
            "jsFiles":[{"type":"application","statusCode":200}],
            "apis":[],
            "routes":[{"path":"/docs"}],
            "sensitiveInfo":[],
            "registrationEntrypoints":[],
            "aiFallback":{"enabled":true,"snippets":[{"context":"webpack chunk"}]}
        });
        let mut guarded = vec![score_frontend_target(
            &framework_only,
            "https://docs.example.invalid",
            &adaptive,
        )];
        assert_eq!(guarded[0].surface, "framework_application");
        assert_eq!(guarded[0].mode, "skip");
        annotate_local_full_power_routes(&mut guarded);
        assert_eq!(guarded[0].mode, "skip");
        let full_power_limits = adaptive_target_limits(&adaptive, &full_power_routes[1], true);
        assert_eq!(full_power_limits, (600, 250_000, 10, 500_000));
        let evidence =
            compact_frontend_evidence(&framework_only, &guarded[0].url, &guarded[0], 20 * 1024);
        assert!(!evidence["aiFallback"].as_object().unwrap().is_empty());
        assert_eq!(evidence["verificationPlan"]["boundedFallbackDiscoveryAllowed"], true);
        assert!(evidence["stopRule"]
            .as_str()
            .unwrap()
            .contains("one targeted discovery pass"));

        let post_only_login = serde_json::json!({
            "url":"https://legacy.example.invalid:8666",
            "statusCode":200,
            "fingerprint":{"frontend":{"framework":"RequireJS/AMD","confidence":"medium"}},
            "jsFiles": (0..8).map(|index| serde_json::json!({
                "url":format!("https://legacy.example.invalid:8666/app-{index}.js"),
                "type":"application",
                "statusCode":200
            })).collect::<Vec<_>>(),
            "apis":[],
            "apiCandidates":[{
                "url":"https://legacy.example.invalid:8666/getephone_login.do",
                "path":"/getephone_login.do",
                "method":"POST",
                "confidence":"high",
                "extractionEngine":"babel-ast",
                "verification":{
                    "status":"rejected",
                    "verified":false,
                    "sameOrigin":true,
                    "probeMethod":"GET",
                    "reason":"unstructured_response"
                }
            }],
            "routes":[],
            "sensitiveInfo":[],
            "aiFallback":{"enabled":true,"snippets":[{"context":"$.ajax login"}]}
        });
        let login_route = score_frontend_target(
            &post_only_login,
            "https://legacy.example.invalid:8666",
            &adaptive,
        );
        assert_eq!(login_route.surface, "ordinary_web");
        assert_eq!(login_route.mode, "quick");
        assert!(login_route
            .reasons
            .iter()
            .any(|reason| reason.contains("一次性兜底发现")));
        let login_evidence =
            compact_frontend_evidence(&post_only_login, &login_route.url, &login_route, 20 * 1024);
        assert_eq!(login_evidence["apiCandidates"][0]["method"], "POST");
        assert_eq!(
            login_evidence["verificationPlan"]["strategy"],
            "opportunity-guided-bounded-validation"
        );
        assert_eq!(
            login_evidence["verificationPlan"]["boundedFallbackDiscoveryAllowed"],
            true
        );
        assert_eq!(
            login_evidence["verificationPlan"]["maxAttemptsPerCandidate"],
            3
        );

        let ordinary_page = serde_json::json!({
            "url":"https://legacy-web.example.invalid",
            "statusCode":200,
            "fingerprint":{"frontend":{"framework":"Unknown","confidence":"low"}},
            "links":[{"url":"/catalog"}],
            "forms":[],
            "jsFiles":[],
            "apis":[],
            "routes":[],
            "sensitiveInfo":[]
        });
        let ordinary_route = score_frontend_target(
            &ordinary_page,
            "https://legacy-web.example.invalid",
            &adaptive,
        );
        assert_eq!(ordinary_route.surface, "ordinary_web");
        assert_eq!(ordinary_route.mode, "quick");

        let authenticated_page = serde_json::json!({
            "url":"https://legacy-web.example.invalid/private",
            "statusCode":401,
            "fingerprint":{"frontend":{"framework":"Unknown","confidence":"low"}},
            "links":[{"url":"/login"}],
            "forms":[],
            "jsFiles":[],
            "apis":[],
            "routes":[],
            "sensitiveInfo":[]
        });
        let authenticated_route = score_frontend_target(
            &authenticated_page,
            "https://legacy-web.example.invalid/private",
            &adaptive,
        );
        assert_eq!(authenticated_route.surface, "ordinary_web");
        assert_eq!(authenticated_route.mode, "quick");
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
    fn compact_frontend_evidence_keeps_business_scripts_and_caps_candidates() {
        let adaptive = AdaptiveStrixSettings::from_json(&serde_json::json!({}));
        let target = serde_json::json!({
            "url": "https://app.example.invalid",
            "jsFiles": [
                {"url":"https://app.example.invalid/vendor.12345678.js","type":"vendor","size":900000},
                {"url":"https://app.example.invalid/main.12345678.js","type":"application","size":120000,"analysis":{"businessScore":1}},
                {"url":"https://app.example.invalid/lazy.87654321.js","type":"chunk","size":90000,"analysis":{"businessScore":9}}
            ],
            "apis": (0..30).map(|index| serde_json::json!({
                "url": format!("/api/item/{index}"),
                "method":"GET",
                "confidence":"high",
                "extractionEngine":"browser-runtime",
                "verification":{"sameOrigin":true}
            })).collect::<Vec<_>>(),
            "routes": [{"path":"/admin"}],
            "sensitiveInfo": [{"type":"jwt","severity":"high","context":"token"}],
            "cryptoSignals": [{"algorithm":"AES","category":"symmetric","localOnly":true}],
            "aiFallback": {
                "enabled":true,"framework":"React","reason":"low quality",
                "maxSliceReads":3,"maxCumulativeSliceChars":72000,
                "snippets":[{"sliceId":"abc123","source":"main.js","marker":"fetch","context":"fetch('/api/'+tenantId)"}],
                "codeSlices":[{"id":"abc123","source":"main.js","kind":"network-call","marker":"fetch","start":120,"end":620,"context":"function load(){ return fetch('/api/'+tenantId); }".repeat(10)}]
            },
            "runtimeSignals": (0..20).map(|index| serde_json::json!({"type":if index==0{"cryptojs"}else if index==1{"runtime_hook_plan"}else{"anti_debug"},"hook":if index==1{"cryptojs"}else{""},"source":format!("chunk-{index}.js")})).collect::<Vec<_>>(),
            "runtimeHookRecommended": true,
            "runtimeHookPlan": {"hook":"cryptojs","reason":"legacy crypto hook"},
            "fingerprint": {"frontend":{"framework":"Vue","confidence":"high"}}
        });
        let route = score_frontend_target(&target, "https://app.example.invalid", &adaptive);
        let evidence = compact_frontend_evidence(&target, &route.url, &route, 20 * 1024);
        assert_eq!(evidence["applicationScripts"].as_array().unwrap().len(), 2);
        assert!(evidence["applicationScripts"][0]["url"]
            .as_str()
            .unwrap()
            .contains("lazy"));
        assert_eq!(route.surface, "framework_application");
        assert_eq!(evidence["apiCandidates"].as_array().unwrap().len(), 3);
        assert_eq!(evidence["sensitiveCandidates"].as_array().unwrap().len(), 1);
        assert_eq!(evidence["runtimeSignals"].as_array().unwrap().len(), 4);
        assert_eq!(evidence["runtimeSignals"][0]["type"], "anti_debug");
        assert!(evidence.get("cryptoSignals").is_none());
        assert!(evidence["aiFallback"]["enabled"].as_bool().unwrap());
        assert_eq!(
            evidence["aiFallback"]["snippets"].as_array().unwrap().len(),
            1
        );
        assert_eq!(
            evidence["aiFallback"]["sliceIndex"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(evidence["aiFallback"]["sliceIndex"][0]
            .get("context")
            .is_none());
        assert!(!evidence["runtimeHookRecommended"].as_bool().unwrap());
        assert!(evidence["runtimeHookPlan"].as_object().unwrap().is_empty());
        assert!(evidence["runtimeSignals"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["type"] != "cryptojs"));
        assert!(serde_json::to_vec(&evidence).unwrap().len() <= 20 * 1024);
        let root = std::env::temp_dir().join(format!("asset-atlas-code-slices-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let written = write_frontend_code_slices(&target, &root, 20 * 1024);
        assert!(written <= 20 * 1024);
        assert!(root.join("frontend-code-index.json").is_file());
        assert!(root.join("frontend-code-slices/abc123.js").is_file());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn frontend_evidence_budget_is_strict_for_oversized_nested_fields() {
        let adaptive = AdaptiveStrixSettings::from_json(&serde_json::json!({}));
        let large = "x".repeat(80_000);
        let target = serde_json::json!({
            "url":"https://large.example.invalid",
            "statusCode":200,
            "fingerprint":{"raw":large},
            "techStack":{"raw":"y".repeat(80_000)},
            "analysisSummary":{"raw":"z".repeat(80_000)},
            "apis":[{"url":"/api/admin","evidence":"a".repeat(80_000)}],
            "routes":[{"path":"/admin","context":"b".repeat(80_000)}],
            "runtimeSignals":[{"type":"runtime_hook_plan","source":"main.js","context":"c".repeat(80_000)}]
        });
        let route = score_frontend_target(&target, "https://large.example.invalid", &adaptive);
        let evidence = compact_frontend_evidence(&target, &route.url, &route, 20 * 1024);
        assert!(serde_json::to_vec(&evidence).unwrap().len() <= 20 * 1024);
    }

    #[test]
    fn frontend_packet_files_respect_the_combined_byte_budget() {
        fn artifact_bytes(path: &Path) -> u64 {
            let mut total = 0;
            if let Ok(entries) = fs::read_dir(path) {
                for entry in entries.flatten() {
                    let child = entry.path();
                    if child.is_dir() {
                        total += artifact_bytes(&child);
                    } else if matches!(
                        child.file_name().and_then(|value| value.to_str()),
                        Some("frontend-evidence.json" | "frontend-code-index.json")
                    ) || child
                        .parent()
                        .and_then(Path::file_name)
                        .and_then(|value| value.to_str())
                        == Some("frontend-code-slices")
                    {
                        total += fs::metadata(child).unwrap().len();
                    }
                }
            }
            total
        }

        let budget = 6 * 1024;
        let url = "https://budget.example.invalid";
        let target = serde_json::json!({
            "url": url,
            "analysisSummary": {"raw":"a".repeat(20_000)},
            "apis": [{"url":"/api/register","evidence":"b".repeat(20_000)}],
            "aiFallback": {
                "enabled": true,
                "codeSlices": (0..8).map(|index| serde_json::json!({
                    "id": format!("slice-{index}"),
                    "source": format!("chunk-{index}.js"),
                    "kind": "network-call",
                    "marker": "fetch",
                    "start": 1,
                    "end": 20_000,
                    "context": "fetch('/api/register', payload);".repeat(2_000)
                })).collect::<Vec<_>>()
            }
        });
        let adaptive = AdaptiveStrixSettings::from_json(&serde_json::json!({}));
        let route = score_frontend_target(&target, url, &adaptive);
        let root = std::env::temp_dir().join(format!("oviraptor-packet-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let recon_path = root.join("recon.json");
        fs::write(
            &recon_path,
            serde_json::to_vec(&serde_json::json!({"targets":[target]})).unwrap(),
        )
        .unwrap();
        let output = root.join("output");
        fs::create_dir_all(&output).unwrap();
        write_frontend_evidence(&recon_path, url, &output, &route, budget, None, "");
        assert!(artifact_bytes(&output) <= budget as u64);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cooperative_pause_finishes_current_target_and_resume_selects_only_pending_urls() {
        let root = std::env::temp_dir().join(format!("asset-atlas-pause-{}", Uuid::new_v4()));
        let db_path = db::initialize(&root).unwrap();
        let connection = db::open(&db_path).unwrap();
        connection
            .execute("INSERT INTO projects(name) VALUES('Pause Test')", [])
            .unwrap();
        let project_id = connection.last_insert_rowid();
        connection.execute("INSERT INTO sentinel_scans(id,project_id,project_name,status,current_checkpoint) VALUES('pause-scan',?1,'Pause Test','pausing','pause requested')", [project_id]).unwrap();
        for (url, status) in [
            ("https://done.invalid", "completed"),
            ("https://recon.invalid", "recon_only"),
            ("https://fused.invalid", "limited"),
            ("https://failed.invalid", "failed"),
            ("https://excluded.invalid", "fuse_excluded"),
            ("https://next.invalid", "routed"),
            ("https://queued.invalid", "queued"),
        ] {
            connection.execute("INSERT INTO sentinel_targets(project_id,scan_id,company,url,status) VALUES(?1,'pause-scan','Example',?2,?3)", params![project_id,url,status]).unwrap();
        }
        drop(connection);
        assert!(sentinel_scan_pause_requested(&db_path, "pause-scan"));
        sentinel_scan_update(&db_path, "pause-scan", "scanning", "late progress update");
        let connection = db::open(&db_path).unwrap();
        let preserved: (String, String) = connection
            .query_row(
                "SELECT status,current_checkpoint FROM sentinel_scans WHERE id='pause-scan'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(preserved, ("pausing".into(), "pause requested".into()));
        drop(connection);
        finish_sentinel_pause(&db_path, "pause-scan", "current target saved");
        let connection = db::open(&db_path).unwrap();
        let status: String = connection
            .query_row(
                "SELECT status FROM sentinel_scans WHERE id='pause-scan'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "paused");
        let remaining: i64 = connection
            .query_row(SENTINEL_RESUME_COUNT_SQL, ["pause-scan"], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, 2);
        let mut statement = connection.prepare(SENTINEL_RESUME_TARGETS_SQL).unwrap();
        let urls = statement
            .query_map(["pause-scan"], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(urls, vec!["https://next.invalid", "https://queued.invalid"]);
        drop(statement);
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn fused_urls_are_persisted_and_excluded_from_rescan() {
        let root = std::env::temp_dir().join(format!("asset-atlas-fuse-zone-{}", Uuid::new_v4()));
        let db_path = db::initialize(&root).unwrap();
        let connection = db::open(&db_path).unwrap();
        connection
            .execute("INSERT INTO projects(name) VALUES('Fuse Test')", [])
            .unwrap();
        let project_id = connection.last_insert_rowid();
        connection.execute("INSERT INTO sentinel_scans(id,project_id,project_name,status) VALUES('fuse-scan',?1,'Fuse Test','scanning')", [project_id]).unwrap();
        connection.execute("INSERT INTO sentinel_targets(project_id,scan_id,company,url,status) VALUES(?1,'fuse-scan','Example','https://app.example.invalid/','limited')", [project_id]).unwrap();
        drop(connection);

        add_target_to_fuse_zone(
            &db_path,
            "fuse-scan",
            "https://app.example.invalid/",
            "no progress",
        );
        let connection = db::open(&db_path).unwrap();
        let fuse_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sentinel_fuse_zone WHERE project_id=?1",
                [project_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(fuse_count, 1);
        let retry_count: i64 = connection
            .query_row(SENTINEL_RESCAN_COUNT_SQL, params!["fuse-scan", 0], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(retry_count, 0);
        connection
            .execute(
                "DELETE FROM sentinel_fuse_zone WHERE project_id=?1",
                [project_id],
            )
            .unwrap();
        let retry_count: i64 = connection
            .query_row(SENTINEL_RESCAN_COUNT_SQL, params!["fuse-scan", 0], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(retry_count, 1);
        drop(connection);
        let _ = fs::remove_dir_all(root);
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
    fn resolves_strix_runtime_from_shell_and_cli_config() {
        let root = std::env::temp_dir().join(format!("asset-atlas-strix-env-{}", Uuid::new_v4()));
        fs::create_dir_all(root.join(".strix")).unwrap();
        fs::write(
            root.join(".zshrc"),
            "export STRIX_LLM='openai/test-model'\n",
        )
        .unwrap();
        fs::write(root.join(".strix/cli-config.json"), serde_json::json!({"env":{"OPENAI_API_KEY":"test-key","OPENAI_BASE_URL":"https://api.example.invalid/v1"}}).to_string()).unwrap();
        assert_eq!(
            shell_assignment(&root.join(".zshrc"), "STRIX_LLM").as_deref(),
            Some("openai/test-model")
        );
        assert_eq!(
            strix_cli_env(&root)
                .get("OPENAI_BASE_URL")
                .and_then(JsonValue::as_str),
            Some("https://api.example.invalid/v1")
        );
        let environment = strix_runtime_env(&serde_json::json!({"strixLlm":"openai/test-model","strixApiKey":"test-key","strixApiBase":"https://api.example.invalid/v1"}), &root).unwrap();
        assert_eq!(environment.llm, "openai/test-model");
        assert_eq!(environment.api_key, "test-key");
        assert_eq!(environment.api_base, "https://api.example.invalid/v1");
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
    fn partial_rescan_selects_all_incomplete_targets() {
        let root = std::env::temp_dir().join(format!("asset-atlas-rescan-{}", Uuid::new_v4()));
        let db_path = db::initialize(&root).unwrap();
        let connection = db::open(&db_path).unwrap();
        connection
            .execute("INSERT INTO projects(id,name) VALUES(1,'test')", [])
            .unwrap();
        connection
            .execute(
                "INSERT INTO sentinel_scans(id,project_id,status) VALUES('parent',1,'partial')",
                [],
            )
            .unwrap();
        for (index, status) in [
            "limited",
            "deferred",
            "failed",
            "queued",
            "completed",
            "recon_only",
        ]
        .into_iter()
        .enumerate()
        {
            connection
                .execute(
                    "INSERT INTO sentinel_targets(project_id,scan_id,url,status) VALUES(1,'parent',?1,?2)",
                    params![format!("https://{index}.example.invalid"), status],
                )
                .unwrap();
        }
        let incomplete: i64 = connection
            .query_row(SENTINEL_RESCAN_COUNT_SQL, params!["parent", 1], |row| {
                row.get(0)
            })
            .unwrap();
        let all: i64 = connection
            .query_row(SENTINEL_RESCAN_COUNT_SQL, params!["parent", 0], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(incomplete, 4);
        assert_eq!(all, 6);
        drop(connection);
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
        assert!(checkpoint.contains("队列已完成"));
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
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn associated_workbench_sync_replaces_runner_failed_status_when_run_completed() {
        let root =
            std::env::temp_dir().join(format!("asset-atlas-workbench-sync-{}", Uuid::new_v4()));
        let app_dir = root.join("oviraptor");
        let runs = root.join("runs");
        let run_dir = runs.join("code-run");
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
        connection.execute("INSERT INTO sentinel_scans(id,project_id,project_name,status,current_checkpoint,task_path,scan_type,task_name,source_path) VALUES('code-parent',1,'test','failed','Strix 退出码：exit status: 2','native-task.json','code','Code scan','/tmp/example')", []).unwrap();
        fs::write(run_dir.join(".asset-atlas-scan-id"), "code-parent").unwrap();
        fs::write(
            run_dir.join("run.json"),
            serde_json::json!({
                "run_id":"code-run",
                "status":"completed",
                "targets_info":[{"original":"/tmp/example"}],
                "llm_usage":{"requests":3,"total_tokens":1000}
            })
            .to_string(),
        )
        .unwrap();
        fs::write(run_dir.join("vulnerabilities.json"), "[]").unwrap();
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
                "SELECT status,current_checkpoint FROM sentinel_scans WHERE id='code-parent'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "completed");
        assert!(checkpoint.contains("0 个漏洞"));
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn web_result_sync_rejects_staged_local_targets_and_preserves_code_paths() {
        let root = std::env::temp_dir().join(format!(
            "oviraptor-web-target-repair-{}",
            Uuid::new_v4()
        ));
        let db_path = db::initialize(&root).unwrap();
        let connection = db::open(&db_path).unwrap();
        connection
            .execute("INSERT INTO projects(id,name) VALUES(1,'Web')", [])
            .unwrap();
        connection.execute("INSERT INTO sentinel_scans(id,project_id,project_name,status,scan_type) VALUES('web-scan',1,'Web','failed','web'),('code-scan',1,'Web','completed','code')", []).unwrap();
        connection.execute("INSERT INTO sentinel_targets(project_id,scan_id,company,url,status) VALUES(1,'web-scan','Real Co','https://example.invalid','failed'),(1,'web-scan','','/tmp/strix-jobs/web-scan/attempt-0001/strix-evidence-input','failed'),(1,'code-scan','Source','/tmp/source-repository','completed')", []).unwrap();
        connection.execute("INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title) VALUES('web-scan','/tmp/strix-jobs/web-scan/attempt-0001/strix-evidence-input','strix','vulnerability','v-1','Test')", []).unwrap();

        repair_web_target_pollution(&connection).unwrap();

        let web_targets: Vec<String> = {
            let mut statement = connection
                .prepare("SELECT url FROM sentinel_targets WHERE scan_id='web-scan' ORDER BY id")
                .unwrap();
            statement
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert_eq!(web_targets, vec!["https://example.invalid"]);
        let finding_target: String = connection
            .query_row(
                "SELECT target_url FROM sentinel_findings WHERE scan_id='web-scan'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(finding_target, "https://example.invalid");
        let code_target: String = connection
            .query_row(
                "SELECT url FROM sentinel_targets WHERE scan_id='code-scan'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(code_target, "/tmp/source-repository");
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn imports_native_strix_artifacts_without_losing_report_fields() {
        let root = std::env::temp_dir().join(format!("asset-atlas-strix-{}", Uuid::new_v4()));
        let app_dir = root.join("oviraptor");
        let runs = root.join("runs");
        let run_dir = runs.join("smoke-run");
        fs::create_dir_all(&run_dir).unwrap();
        let db_path = db::initialize(&app_dir).unwrap();
        let connection = db::open(&db_path).unwrap();
        connection
            .execute(
                "UPDATE config_profiles SET settings_json=json_set(settings_json,'$.strixRunsDirectory',?1)",
                [runs.to_string_lossy().to_string()],
            )
            .unwrap();
        fs::write(
            run_dir.join("run.json"),
            serde_json::to_vec(&serde_json::json!({
                "run_id":"smoke-run",
                "run_name":"smoke-run",
                "status":"completed",
                "targets_info":[{"type":"web_application","original":"https://example.invalid","details":{"target_url":"https://example.invalid"}}],
                "llm_usage":{"requests":4,"input_tokens":1200,"output_tokens":300,"total_tokens":1500,"input_tokens_details":[{"cached_tokens":900}]},
                "scan_results":{"executive_summary":"done"}
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            run_dir.join("vulnerabilities.json"),
            serde_json::to_vec(&serde_json::json!([{
                "id":"vuln-0001",
                "title":"Verified issue",
                "severity":"high",
                "target":"https://example.invalid",
                "endpoint":"/api/test",
                "method":"POST",
                "cvss":8.1,
                "cwe":"CWE-79",
                "technical_analysis":"analysis",
                "poc_description":"reproduce",
                "poc_script_code":"curl example.invalid",
                "remediation_steps":"fix it",
                "code_locations":[{"file":"app.js","start_line":7}]
            }]))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            run_dir.join("events.jsonl"),
            "{\"name\":\"scan.start\"}\n{\"name\":\"scan.end\"}\n",
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
                "SELECT status,current_checkpoint FROM sentinel_scans WHERE id='strix-smoke-run'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "completed");
        assert!(checkpoint.contains("1 个漏洞"));
        let tokens: (i64, i64, i64) = connection
            .query_row(
                "SELECT input_tokens,output_tokens,total_tokens FROM sentinel_scans WHERE id='strix-smoke-run'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(tokens, (1200, 300, 1500));
        let record: String = connection
            .query_row(
                "SELECT record_json FROM sentinel_findings WHERE scan_id='strix-smoke-run' AND kind='vulnerability'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let record: JsonValue = serde_json::from_str(&record).unwrap();
        assert_eq!(
            record.get("source").and_then(JsonValue::as_str),
            Some("strix")
        );
        assert_eq!(
            record.get("recommendation").and_then(JsonValue::as_str),
            Some("fix it")
        );
        assert_eq!(
            record.get("cwe").and_then(JsonValue::as_str),
            Some("CWE-79")
        );
        assert!(record.get("code_locations").is_some());
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
    fn reconstructs_strix_trace_and_creates_target_neutral_knowledge() {
        let root = std::env::temp_dir().join(format!("asset-atlas-trace-{}", Uuid::new_v4()));
        let db_path = db::initialize(&root).unwrap();
        let task_dir = root.join("strix-jobs/trace-test");
        let run_dir = task_dir.join("strix_runs/example_1234");
        fs::create_dir_all(run_dir.join(".state")).unwrap();
        fs::write(
            run_dir.join("run.json"),
            serde_json::to_vec(&serde_json::json!({
                "instruction":"verify reproducible issues",
                "targets_info":[{"original":"https://example.test"}],
                "llm_usage":{"requests":3,"input_tokens":1200,"output_tokens":300,"cached_tokens":800,"total_tokens":1500}
            })).unwrap(),
        ).unwrap();
        let agent_db = rusqlite::Connection::open(run_dir.join(".state/agents.db")).unwrap();
        agent_db.execute_batch("CREATE TABLE agent_sessions(session_id TEXT PRIMARY KEY);CREATE TABLE agent_messages(id INTEGER PRIMARY KEY,session_id TEXT NOT NULL,message_data TEXT NOT NULL,created_at TEXT NOT NULL);").unwrap();
        agent_db
            .execute("INSERT INTO agent_sessions(session_id) VALUES('root')", [])
            .unwrap();
        for (id, message) in [
            (
                1,
                serde_json::json!({"type":"message","role":"assistant","content":[],"provider_data":{"model":"deepseek/test"}}),
            ),
            (
                2,
                serde_json::json!({"type":"reasoning","summary":"bounded analysis"}),
            ),
            (
                3,
                serde_json::json!({"type":"function_call","name":"browser_request","call_id":"call-1","status":"completed","arguments":"{\"url\":\"https://example.test\",\"password\":\"do-not-store\"}"}),
            ),
            (
                4,
                serde_json::json!({"type":"function_call_output","call_id":"call-1","status":"completed","output":"Cookie: session=do-not-store\nHTTP 200 verified"}),
            ),
        ] {
            agent_db.execute("INSERT INTO agent_messages(id,session_id,message_data,created_at) VALUES(?1,'root',?2,'2026-07-20 12:00:00')",params![id,message.to_string()]).unwrap();
        }
        drop(agent_db);
        let connection = db::open(&db_path).unwrap();
        connection.execute("INSERT INTO sentinel_scans(id,project_name,status,task_path,scan_type,task_name) VALUES('trace-test','Test','completed',?1,'web','Trace test')",[task_dir.to_string_lossy().to_string()]).unwrap();
        connection.execute("INSERT INTO sentinel_findings(scan_id,stage,kind,title,severity) VALUES('trace-test','strix','vulnerability','SQL Injection','high')",[]).unwrap();

        let (trace, events) = collect_strix_trace(&connection, "trace-test", true).unwrap();
        assert_eq!(trace.model, "deepseek/test");
        assert_eq!(trace.agent_count, 1);
        assert_eq!(trace.message_count, 4);
        assert_eq!(trace.reasoning_count, 1);
        assert_eq!(trace.tool_call_count, 1);
        assert_eq!(trace.tool_result_count, 1);
        assert_eq!(trace.tools[0].name, "browser_request");
        assert_eq!(trace.tools[0].results, 1);
        assert_eq!(trace.total_tokens, 1500);
        assert_eq!(events[2].call_id, "call-1");
        assert_eq!(events[2].target_url, "https://example.test");
        assert_eq!(events.len(), 4);
        assert!(events[2].detail.contains("do-not-store"));
        assert!(events[3].detail.contains("Cookie: session=do-not-store"));
        assert!(events[3].detail.contains("HTTP 200 verified"));
        let live = live_strix_metrics(&task_dir);
        assert_eq!(live.requests, 3);
        assert_eq!(live.meaningful_tools, 1);
        assert_eq!(live.unique_tool_results, 1);
        assert_eq!(live.max_tool_repeats, 1);
        assert!(live.latest_event.contains("browser_request"));
        assert!(events[3].detail.contains("session="));
        assert!(!trace.instruction_hash.is_empty());

        let patterns =
            serde_json::json!({"tools":["browser_request"],"findingClasses":["SQL Injection"]});
        connection.execute("INSERT INTO strix_knowledge_entries(scan_id,title,summary,patterns_json,skill_instructions,source_hash) VALUES('trace-test','Trace knowledge','No credentials',?1,'Verify evidence safely','hash')",[patterns.to_string()]).unwrap();
        let entry = connection.query_row(&format!("SELECT {KNOWLEDGE_COLUMNS} FROM strix_knowledge_entries WHERE scan_id='trace-test'"),[],knowledge_row).unwrap();
        assert_eq!(entry.title, "Trace knowledge");
        assert_eq!(entry.patterns["tools"][0], "browser_request");
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn asset_review_filter_separates_confirmed_from_needs_evidence() {
        let review = AssetQuery {
            deleted_view: "active".into(),
            decision_view: "review".into(),
            ..AssetQuery::default()
        };
        let (review_sql, _) = asset_filter(&review, true);
        assert!(review_sql.contains("pa.is_deleted=0"));
        assert!(review_sql.contains("'pending','uncertain'"));

        let confirmed = AssetQuery {
            deleted_view: "trash".into(),
            decision_view: "confirmed".into(),
            ..AssetQuery::default()
        };
        let (confirmed_sql, _) = asset_filter(&confirmed, true);
        assert!(confirmed_sql.contains("pa.is_deleted=1"));
        assert!(confirmed_sql.contains("pa.decision='confirmed'"));
        assert!(!confirmed_sql.contains("'pending','uncertain'"));
    }

    #[test]
    fn sentinel_attempt_ledger_tracks_incremental_cost_and_terminal_reason() {
        let root =
            std::env::temp_dir().join(format!("oviraptor-attempt-ledger-{}", Uuid::new_v4()));
        let db_path = db::initialize(&root).unwrap();
        let connection = db::open(&db_path).unwrap();
        connection.execute("INSERT INTO sentinel_scans(id,status,current_checkpoint,attempt_count,llm_requests,input_tokens,output_tokens,cached_tokens,total_tokens) VALUES('attempt-test','scanning','正在启动 Strix Agent',1,2,100,20,60,120)", []).unwrap();
        let work_dir = root.join("strix-jobs/attempt-test/attempt-1");
        fs::create_dir_all(&work_dir).unwrap();
        record_sentinel_attempt_start(&connection, "attempt-test", 1, &work_dir).unwrap();
        connection.execute("UPDATE sentinel_scans SET status='completed',current_checkpoint='结果同步完成',llm_requests=5,input_tokens=340,output_tokens=75,cached_tokens=210,total_tokens=415 WHERE id='attempt-test'", []).unwrap();
        sync_sentinel_attempt(&connection, "attempt-test");
        let row: (String, String, i64, i64, i64, String) = connection.query_row("SELECT status,stage,llm_requests_delta,input_tokens_delta,total_tokens_delta,stop_reason FROM sentinel_scan_attempts WHERE scan_id='attempt-test' AND attempt_number=1", [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?))).unwrap();
        assert_eq!(row.0, "completed");
        assert_eq!(row.1, "complete");
        assert_eq!(row.2, 3);
        assert_eq!(row.3, 240);
        assert_eq!(row.4, 295);
        assert_eq!(row.5, "结果同步完成");
        drop(connection);
        let _ = fs::remove_dir_all(root);
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
}
