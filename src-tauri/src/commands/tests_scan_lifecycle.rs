    #[test]
    fn native_interruption_is_partial_unless_oviraptor_explicitly_paused() {
        assert_eq!(imported_strix_run_status("interrupted"), "partial");
        assert_eq!(imported_strix_run_status("stopped"), "partial");
        assert_eq!(imported_strix_run_status("cancelled"), "partial");
        assert_eq!(imported_strix_run_status("completed"), "completed");
        assert_eq!(imported_strix_run_status("succeeded"), "completed");
        assert_eq!(imported_strix_run_status("success"), "completed");
        assert_eq!(imported_strix_run_status("done"), "completed");
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
        let retry_mode: String = connection
            .query_row(
                "SELECT value FROM app_settings WHERE key='sentinel-next-attempt-mode:current'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(retry_mode, "resume");
        let scan_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM sentinel_scans", [], |row| row.get(0))
            .unwrap();
        assert_eq!(scan_count, 1);
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn fresh_attempt_rebuilds_current_result_surface_but_keeps_confirmed_work() {
        let root = std::env::temp_dir().join(format!(
            "oviraptor-fresh-attempt-surface-{}",
            Uuid::new_v4()
        ));
        let db_path = db::initialize(&root).unwrap();
        let connection = db::open(&db_path).unwrap();
        connection
            .execute("INSERT INTO projects(id,name) VALUES(1,'test')", [])
            .unwrap();
        connection.execute("INSERT INTO sentinel_scans(id,project_id,status,scan_type,attempt_count) VALUES('fresh-scan',1,'scanning','web',2)", []).unwrap();
        connection.execute("INSERT INTO sentinel_targets(project_id,scan_id,company,url,status,last_attempt_number) VALUES(1,'fresh-scan','test','https://fresh.invalid','queued',2)", []).unwrap();
        connection.execute("INSERT INTO sentinel_scan_attempts(scan_id,attempt_number,execution_mode,status) VALUES('fresh-scan',2,'fresh','scanning')", []).unwrap();
        connection.execute("INSERT INTO sentinel_checkpoints(scan_id,url,stage,raw_json) VALUES('fresh-scan','https://fresh.invalid','frontend_recon','{}'),('fresh-scan','*','strix_run:old','{}')", []).unwrap();
        connection.execute("INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title) VALUES('fresh-scan','https://fresh.invalid','frontend-recon','api','api-1','API'),('fresh-scan','https://fresh.invalid','strix','vulnerability','v-1','old')", []).unwrap();
        connection.execute("INSERT INTO sentinel_opportunities(project_id,scan_id,target_url,opportunity_key,status) VALUES(1,'fresh-scan','https://fresh.invalid','queued','queued'),(1,'fresh-scan','https://fresh.invalid','confirmed','validated')", []).unwrap();
        connection.execute("INSERT INTO investigation_api_models(project_id,scan_id,target_url,api_key,url) VALUES(1,'fresh-scan','https://fresh.invalid','api','https://fresh.invalid/api')", []).unwrap();
        connection.execute("INSERT INTO sentinel_validations(scan_id,url,finding_key,verdict) VALUES('fresh-scan','https://fresh.invalid','strix:vulnerability:v-1','confirmed')", []).unwrap();

        prepare_latest_strix_attempt(&connection, "fresh-scan", 2).unwrap();

        for table in [
            "sentinel_checkpoints",
            "sentinel_findings",
            "investigation_api_models",
        ] {
            let count: i64 = connection
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE scan_id='fresh-scan'"),
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 0, "{table} should be rebuilt by a fresh attempt");
        }
        let opportunities: Vec<String> = {
            let mut statement = connection
                .prepare("SELECT status FROM sentinel_opportunities WHERE scan_id='fresh-scan'")
                .unwrap();
            statement
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert_eq!(opportunities, vec!["validated"]);
        let validations: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sentinel_validations WHERE scan_id='fresh-scan'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(validations, 1);
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
    fn only_confirmed_protection_blocks_enter_the_fuse_zone() {
        assert!(hard_fuse_reason(
            "confirmed WAF challenge with sustained 429 rate limit"
        ));
        assert!(hard_fuse_reason("页面出现验证码和人机验证"));
        assert!(!hard_fuse_reason("模型调用达到软预算且没有新增工具结果"));
        assert!(!hard_fuse_reason("上下文窗口不足，已保存检查点"));
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
                "UPDATE sentinel_fuse_zone SET archived=1 WHERE project_id=?1",
                [project_id],
            )
            .unwrap();
        let retry_count: i64 = connection
            .query_row(SENTINEL_RESCAN_COUNT_SQL, params!["fuse-scan", 0], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(retry_count, 1);
        let historical_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sentinel_fuse_zone WHERE project_id=?1 AND archived=1",
                [project_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(historical_count, 1);
        drop(connection);
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
        fs::write(root.join(".strix/cli-config.json"), serde_json::json!({"env":{"OPENAI_API_KEY":"test-key","OPENAI_BASE_URL":"https://api.example.invalid/v1","STRIX_IMAGE":"ghcr.io/usestrix/strix-sandbox:1.3.0"}}).to_string()).unwrap();
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
        assert_eq!(environment.image, DEFAULT_STRIX_SANDBOX_IMAGE);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn partial_rescan_selects_all_incomplete_targets() {
        for status in ["partial", "failed", "limited", "cancelled"] {
            assert!(retry_only_incomplete_targets(status));
        }
        for status in ["completed", "recon_only", "manual_review"] {
            assert!(!retry_only_incomplete_targets(status));
        }
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
    fn associated_web_sync_only_imports_latest_attempt_and_stays_idempotent() {
        let root =
            std::env::temp_dir().join(format!("oviraptor-latest-strix-attempt-{}", Uuid::new_v4()));
        let app_dir = root.join("oviraptor");
        let runs = root.join("runs");
        let old_attempt = runs.join("scan/attempt-0001");
        let new_attempt = runs.join("scan/attempt-0002");
        let old_run = old_attempt.join("url-pipeline/target-00001/strix_runs/old-run");
        let new_run = new_attempt.join("url-pipeline/target-00001/strix_runs/new-run");
        fs::create_dir_all(&old_run).unwrap();
        fs::create_dir_all(&new_run).unwrap();

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
        connection.execute("INSERT INTO sentinel_scans(id,project_id,project_name,status,current_checkpoint,task_path,scan_type,attempt_count) VALUES('attempt-parent',1,'test','completed','已完成','native-task.json','web',2)", []).unwrap();
        connection.execute("INSERT INTO sentinel_targets(project_id,scan_id,company,url,status) VALUES(1,'attempt-parent','test','https://example.invalid','completed')", []).unwrap();
        connection.execute(
            "INSERT INTO sentinel_scan_attempts(scan_id,attempt_number,status,work_dir) VALUES('attempt-parent',1,'partial',?1),('attempt-parent',2,'completed',?2)",
            params![
                old_attempt.to_string_lossy().to_string(),
                new_attempt.to_string_lossy().to_string()
            ],
        ).unwrap();
        connection.execute("INSERT INTO sentinel_checkpoints(scan_id,url,stage,raw_json) VALUES('attempt-parent','*','strix_run:old-run','{\"stale\":true}')", []).unwrap();

        for run_dir in [&old_run, &new_run] {
            fs::write(run_dir.join(".asset-atlas-scan-id"), "attempt-parent").unwrap();
        }
        fs::write(
            old_run.join("run.json"),
            serde_json::json!({
                "run_id":"old-run",
                "status":"interrupted",
                "targets_info":[{"original":"https://example.invalid"}],
                "llm_usage":{"requests":1,"total_tokens":10}
            })
            .to_string(),
        )
        .unwrap();
        fs::write(
            new_run.join("run.json"),
            serde_json::json!({
                "run_id":"new-run",
                "status":"completed",
                "targets_info":[{"original":"https://example.invalid"}],
                "llm_usage":{"requests":2,"total_tokens":20}
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
        let stages: Vec<String> = {
            let mut statement = connection
                .prepare("SELECT stage FROM sentinel_checkpoints WHERE scan_id='attempt-parent' AND stage LIKE 'strix_run:%' ORDER BY stage")
                .unwrap();
            statement
                .query_map([], |row| row.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        assert_eq!(stages, vec!["strix_run:new-run"]);
        assert_eq!(sync_strix_results(&connection, &state).unwrap(), 0);
        let marker: String = connection
            .query_row(
                "SELECT value FROM app_settings WHERE key='strix-current-attempt:attempt-parent'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(marker, "2");
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
        connection.execute("UPDATE sentinel_scans SET current_checkpoint='任务累计状态：自动验证 1，确定性侦察收口 2' WHERE id='attempt-test'", []).unwrap();
        sync_sentinel_attempt(&connection, "attempt-test");
        let preserved: (String, String) = connection.query_row(
            "SELECT checkpoint,stop_reason FROM sentinel_scan_attempts WHERE scan_id='attempt-test' AND attempt_number=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(preserved.0, "结果同步完成");
        assert_eq!(preserved.1, "结果同步完成");
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }
