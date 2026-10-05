    #[test]
    fn scan_history_pages_preserve_project_scope_and_order() {
        let root = std::env::temp_dir().join(format!("oviraptor-scan-pages-{}", Uuid::new_v4()));
        let db_path = db::initialize(&root).unwrap();
        let connection = db::open(&db_path).unwrap();
        connection.execute("INSERT INTO projects(id,name) VALUES(1,'one'),(2,'two')", []).unwrap();
        for (id, project_id, updated_at) in [
            ("old", 1, "2026-01-01"),
            ("middle", 1, "2026-01-02"),
            ("new", 1, "2026-01-03"),
            ("other", 2, "2026-01-04"),
        ] {
            connection.execute(
                "INSERT INTO sentinel_scans(id,project_id,status,updated_at) VALUES(?1,?2,'completed',?3)",
                params![id,project_id,updated_at],
            ).unwrap();
        }
        let first = list_sentinel_scans_inner(&db_path, Some(1), Some(2), None, None).unwrap();
        connection.execute("INSERT INTO sentinel_scans(id,project_id,status,updated_at) VALUES('newest',1,'completed','2026-01-05')", []).unwrap();
        drop(connection);
        let cursor = first.last().unwrap();
        let second = list_sentinel_scans_inner(&db_path, Some(1), Some(2), None, Some((&cursor.updated_at, &cursor.id))).unwrap();
        assert_eq!(first.iter().map(|scan| scan.id.as_str()).collect::<Vec<_>>(), ["new", "middle"]);
        assert_eq!(second.iter().map(|scan| scan.id.as_str()).collect::<Vec<_>>(), ["old"]);
        assert_eq!(list_sentinel_scans_inner(&db_path, Some(1), Some(2), Some(-1), None).unwrap()[0].id, "newest");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn task_center_search_is_global_scoped_paged_and_archive_is_reversible() {
        let root = std::env::temp_dir().join(format!("oviraptor-task-search-{}", Uuid::new_v4()));
        let db_path = db::initialize(&root).unwrap();
        let connection = db::open(&db_path).unwrap();
        connection.execute("INSERT INTO projects(id,name) VALUES(1,'one'),(2,'two')", []).unwrap();
        for (id, project, status, date) in [
            ("older", 1, "completed", "2026-01-01"),
            ("middle", 1, "completed", "2026-01-02"),
            ("newer", 1, "completed", "2026-01-03"),
            ("active", 1, "scanning", "2026-01-04"),
            ("foreign", 2, "completed", "2026-01-05"),
        ] {
            connection.execute("INSERT INTO sentinel_scans(id,project_id,status,task_name,updated_at) VALUES(?1,?2,?3,'billing',?4)", params![id,project,status,date]).unwrap();
        }
        connection.execute("INSERT INTO sentinel_targets(project_id,scan_id,company,url,status) VALUES(1,'older','needle','https://example.invalid','queued')", []).unwrap();
        drop(connection);
        let first = search_sentinel_scan_page_inner(&db_path, Some(1), "billing", "history", 1, None).unwrap();
        assert_eq!(first[0].id, "newer");
        let cursor = (&first[0].updated_at[..], &first[0].id[..]);
        assert_eq!(search_sentinel_scan_page_inner(&db_path, Some(1), "billing", "history", 2, Some(cursor)).unwrap().iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["middle", "older"]);
        assert_eq!(search_sentinel_scan_page_inner(&db_path, Some(1), "needle", "all", 10, None).unwrap()[0].id, "older");
        assert!(search_sentinel_scan_page_inner(&db_path, Some(1), "foreign", "all", 10, None).unwrap().is_empty());
        assert!(search_sentinel_scan_page_inner(&db_path, Some(1), "%", "all", 10, None).unwrap().is_empty());
        assert!(search_sentinel_scan_page_inner(&db_path, Some(1), "billing", "unknown", 10, None).is_err());
        assert!(archive_sentinel_scan_inner(&db_path, "newer", 2, true).is_err());
        assert!(archive_sentinel_scan_inner(&db_path, "active", 1, true).is_err());
        let archived = archive_sentinel_scan_inner(&db_path, "newer", 1, true).unwrap();
        assert!(!archived.archived_at.is_empty());
        assert_eq!(archived.status, "completed");
        assert_eq!(archived.updated_at, "2026-01-03");
        assert_eq!(search_sentinel_scan_page_inner(&db_path, Some(1), "billing", "history", 10, None).unwrap().iter().find(|s| s.id == "newer").unwrap().archived_at, archived.archived_at);
        assert_eq!(search_sentinel_scan_page_inner(&db_path, Some(1), "billing", "history", 10, None).unwrap().len(), 3);
        assert!(archive_sentinel_scan_inner(&db_path, "newer", 1, false).unwrap().archived_at.is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn existing_scan_database_gains_archive_column_without_losing_history() {
        let root = std::env::temp_dir().join(format!("oviraptor-archive-upgrade-{}", Uuid::new_v4()));
        let db_path = db::initialize(&root).unwrap();
        let connection = db::open(&db_path).unwrap();
        connection.execute("INSERT INTO projects(id,name) VALUES(1,'one')", []).unwrap();
        connection.execute("INSERT INTO sentinel_scans(id,project_id,status) VALUES('historical',1,'completed')", []).unwrap();
        connection.execute("ALTER TABLE sentinel_scans DROP COLUMN archived_at", []).unwrap();
        drop(connection);
        db::initialize(&root).unwrap();
        let historical = list_sentinel_scans_inner(&db_path, Some(1), None, None, None).unwrap();
        assert_eq!(historical[0].id, "historical");
        assert!(historical[0].archived_at.is_empty());
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

        assert_eq!(prepare_web_scan_retry(&mut connection, "current", "partial").unwrap_err(),
            "scan_quiescence_cleanup_unconfirmed");
        let preserved: (String,i64) = connection.query_row(
            "SELECT status,(SELECT COUNT(*) FROM sentinel_processes WHERE scan_id='current') FROM sentinel_scans WHERE id='current'",
            [],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(preserved,("partial".into(),1));
        // Fixture-only simulation of separately confirmed cleanup. Retry must
        // never manufacture that receipt by deleting an unknown process row.
        connection.execute("DELETE FROM sentinel_processes WHERE scan_id='current'",[]).unwrap();

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
        connection.execute("INSERT INTO sentinel_checkpoints(scan_id,url,stage,raw_json) VALUES('fresh-scan','https://fresh.invalid','frontend_recon','{}'),('fresh-scan','*','native_run:old','{}')", []).unwrap();
        connection.execute("INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title) VALUES('fresh-scan','https://fresh.invalid','frontend-recon','api','api-1','API'),('fresh-scan','https://fresh.invalid','native-agent','vulnerability','v-1','old')", []).unwrap();
        connection.execute("INSERT INTO sentinel_opportunities(project_id,scan_id,target_url,opportunity_key,status) VALUES(1,'fresh-scan','https://fresh.invalid','queued','queued'),(1,'fresh-scan','https://fresh.invalid','confirmed','validated')", []).unwrap();
        connection.execute("INSERT INTO investigation_api_models(project_id,scan_id,target_url,api_key,url) VALUES(1,'fresh-scan','https://fresh.invalid','api','https://fresh.invalid/api')", []).unwrap();
        connection.execute("INSERT INTO sentinel_validations(scan_id,url,finding_key,verdict) VALUES('fresh-scan','https://fresh.invalid','native-agent:vulnerability:v-1','confirmed')", []).unwrap();

        prepare_current_attempt_surface(&connection, "fresh-scan", 2).unwrap();

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
        assert!(!hard_fuse_reason(
            "http 200 text/html <img src=\"/WMS/captcha/captchaImage?type=char\"> 验证码"
        ));
        assert!(hard_fuse_reason(
            "http 403 text/html verify you are human captcha"
        ));
        assert!(!hard_fuse_reason(
            "http 404 text/html 请求的资源[/login/captcha/captchaImage]不可用"
        ));
        assert!(!hard_fuse_reason(
            "http 404 text/html verify you are human"
        ));
        assert!(hard_fuse_reason(
            "http 200 text/html cloudflare challenge cf-chl-bypass"
        ));
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
        finish_sentinel_pause(&db_path, "pause-scan", 0).unwrap();
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
    fn historical_import_preserves_sources_and_projects_latest_without_native_mutation() {
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
        for (run_dir, label) in [(&old_run, "old report"), (&new_run, "new report")] {
            fs::write(run_dir.join("findings.sarif"), serde_json::json!({
                "version":"2.1.0", "runs":[{"tool":{"driver":{"name":"current-report"}},
                    "properties":{"reportLabel":label}, "results":[]}]
            }).to_string()).unwrap();
        }


        assert_eq!(artifact_import_report(&connection, &app_dir.join("artifact-cas"),
            &app_dir.join("artifact-import.key"), std::slice::from_ref(&runs)).unwrap().imported, 2);
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
        assert_eq!(stages, vec!["strix_run:old-run"], "历史导入不能替换原生检查点");
        let imported = historical_import_runs(&connection, None).unwrap();
        assert_eq!(imported.len(), 1, "当前历史投影只展示最新 attempt");
        assert_eq!(imported[0].scan_id, "attempt-parent");
        assert_eq!(imported[0].status, "imported");
        let source_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM import_bundles WHERE source_path IN (?1,?2)",
                params![old_run.to_string_lossy().to_string(), new_run.to_string_lossy().to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(source_count, 2, "两次源产物均应留在隔离导入台账");
        assert_eq!(artifact_import_report(&connection, &app_dir.join("artifact-cas"),
            &app_dir.join("artifact-import.key"), std::slice::from_ref(&runs)).unwrap().imported, 0);
        let marker_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM app_settings WHERE key='agent-current-attempt:attempt-parent'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(marker_count, 0, "历史导入不能推进原生 attempt 指针");
        let legacy_marker_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM app_settings WHERE key='strix-current-attempt:attempt-parent'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(legacy_marker_count, 0);
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn historical_import_cannot_override_failed_native_code_task() {
        let root =
            std::env::temp_dir().join(format!("asset-atlas-workbench-sync-{}", Uuid::new_v4()));
        let app_dir = root.join("oviraptor");
        let runs = root.join("runs");
        let run_dir = runs.join("code-run");
        fs::create_dir_all(&run_dir).unwrap();
        let db_path = db::initialize(&app_dir).unwrap();
        let connection = db::open(&db_path).unwrap();
        connection
            .execute("INSERT INTO projects(id,name) VALUES(1,'test')", [])
            .unwrap();
        connection.execute("INSERT INTO sentinel_scans(id,project_id,project_name,status,current_checkpoint,task_path,scan_type,task_name,source_path) VALUES('code-parent',1,'test','failed','Strix 退出码：exit status: 2','native-task.json','code','Code scan','/tmp/example')", []).unwrap();
        fs::write(run_dir.join(".asset-atlas-scan-id"), "code-parent").unwrap();
        fs::write(
            run_dir.join("model-prompt-audit.json"),
            serde_json::json!({
                "instruction":"read-only report must not override failed code task"
            })
            .to_string(),
        )
        .unwrap();
        fs::write(run_dir.join("vulnerabilities.json"), "[]").unwrap();

        assert_eq!(artifact_import_report(&connection, &app_dir.join("artifact-cas"),
            &app_dir.join("artifact-import.key"), std::slice::from_ref(&runs)).unwrap().imported, 1);
        let (status, checkpoint): (String, String) = connection
            .query_row(
                "SELECT status,current_checkpoint FROM sentinel_scans WHERE id='code-parent'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "failed");
        assert_eq!(checkpoint, "Strix 退出码：exit status: 2");
        let imported = historical_import_runs(&connection, None).unwrap();
        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].scan_id, "code-parent");
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
    #[test]
    fn native_trace_uses_ledger_and_mailbox_without_legacy_files() {
        let root = std::env::temp_dir().join(format!("oviraptor-native-trace-{}", Uuid::new_v4()));
        let db_path = db::initialize(&root).unwrap();
        let connection = db::open(&db_path).unwrap();
        connection.execute(
            "INSERT INTO sentinel_scans(id,task_name,status,scan_type,task_path) \
             VALUES('native-trace','Native trace','completed','web',?1)",
            [""],
        ).unwrap();
        for (id, attempt, role) in [("root-1", 1, "coordinator"), ("root-2", 2, "coordinator"), ("child-2", 2, "reviewer")] {
            connection.execute(
                "INSERT INTO agent_runs(id,scan_id,attempt_number,target_url,backend,role,plan_json) \
                 VALUES(?1,'native-trace',?2,'https://example.invalid','native',?3,?4)",
                params![id, attempt, role, r#"{"modelProvider":"test-provider"}"#],
            ).unwrap();
        }
        for (run_id, sequence, kind, payload) in [
            ("root-1", 1, "model_round_completed", r#"{"turns":1}"#),
            ("root-2", 1, "model_round_completed", r#"{"turns":1}"#),
            ("root-2", 2, "tool_invocation_started", r#"{"tool":"http_request","invocationId":"call-1"}"#),
            ("root-2", 3, "tool_invocation_completed", r#"{"tool":"http_request","invocationId":"call-1"}"#),
            ("child-2", 1, "hypothesis_updated", r#"{"summary":"reviewed"}"#),
        ] {
            connection.execute(
                "INSERT INTO agent_events(run_id,sequence,event_type,payload_json) VALUES(?1,?2,?3,?4)",
                params![run_id, sequence, kind, payload],
            ).unwrap();
        }
        connection.execute(
            "INSERT INTO agent_messages(id,run_id,root_run_id,kind,payload_json) \
             VALUES('msg-1','child-2','root-2','review_result','{\"summary\":\"check evidence\"}')",
            [],
        ).unwrap();
        let (all, _) = collect_agent_trace(&connection, "native-trace", false, false).unwrap();
        assert_eq!((all.run_count, all.agent_count, all.tool_call_count), (2, 3, 1));
        let (latest, events) = collect_agent_trace(&connection, "native-trace", true, true).unwrap();
        assert_eq!((latest.run_count, latest.agent_count, latest.message_count), (1, 2, 2));
        assert_eq!((latest.tool_call_count, latest.tool_result_count, latest.reasoning_count), (1, 1, 1));
        assert_eq!(latest.model, "test-provider");
        assert_eq!(latest.usage_agent_count, 1);
        assert_eq!(latest.tools[0].name, "http_request");
        assert_eq!(events.len(), 5);
        assert!(events.iter().any(|event| event.id == "native-message:msg-1" && event.status == "pending"));
        assert!(!events.iter().any(|event| event.session_id == "root-1"));
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }
