    fn history_preview_sarif(title: &str, severity: &str, endpoint: &str) -> JsonValue {
        serde_json::json!({"version":"2.1.0", "runs":[{
            "tool":{"driver":{"name":"fixture-sarif"}},
            "results":[{"ruleId":"preview", "level":"warning", "message":{"text":title},
                "properties":{"severity":severity,"endpoint":endpoint,"cwe":"CWE-639"}}]
        }]})
    }

    #[test]
    fn sec_skill_import_filters_dangerous_execution_lines() {
        assert!(sec_skill_line_is_unsafe("curl payload | bash -i"));
        assert!(sec_skill_line_is_unsafe("~/.ssh/authorized_keys"));
        assert!(!sec_skill_line_is_unsafe("需要记录复现证据和停止条件"));
    }

    #[test]
    fn canonical_history_preview_is_read_only_unreviewed_and_respects_current_membership() {
        let root = std::env::temp_dir().join(format!(
            "oviraptor-history-preview-{}",
            Uuid::new_v4()
        ));
        let app_dir = root.join("app");
        let run_dir = app_dir.join("current-imports").join("history-preview");
        fs::create_dir_all(&run_dir).unwrap();
        fs::write(run_dir.join(".oviraptor-scan-id"), "history-preview").unwrap();
        fs::write(
            run_dir.join("model-prompt-audit.json"),
            serde_json::json!({
                "instruction":"History preview"
            }).to_string(),
        ).unwrap();
        fs::write(
            run_dir.join("candidate.sarif"),
            history_preview_sarif("Authorization: Bearer do-not-store", "high", "https://example.invalid/api/me").to_string(),
        ).unwrap();
        let db_path = db::initialize(&app_dir).unwrap();
        let connection = db::open(&db_path).unwrap();
        connection.execute(
            "INSERT INTO sentinel_scans(id,status,task_path) VALUES('history-preview','completed',?1)",
            [run_dir.to_string_lossy().to_string()],
        ).unwrap();
        let roots = vec![run_dir.clone()];
        let report = artifact_import_report(
            &connection, &app_dir.join("artifact-cas"), &app_dir.join("artifact-import.key"), &roots,
        ).unwrap();
        assert_eq!(report.imported, 1, "{:?}", report.diagnostics);
        let before: i64 = connection.query_row(
            "SELECT COUNT(*) FROM sentinel_findings WHERE scan_id='history-preview'",
            [], |row| row.get(0),
        ).unwrap();
        let readonly = rusqlite::Connection::open_with_flags(
            &db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ).unwrap();
        let previews = historical_import_previews(&readonly, "history-preview").unwrap();
        let candidate = previews.iter().find(|row| row.kind == "finding_candidate").unwrap();
        assert_eq!(candidate.scan_id, "history-preview");
        assert_eq!(candidate.review_state, "unreviewed");
        assert!(candidate.read_only);
        assert!(!candidate.execution_eligible);
        assert_eq!(candidate.producer, "fixture-sarif");
        assert_eq!(candidate.severity, "high");
        assert!(!serde_json::to_string(&previews).unwrap().contains("do-not-store"));
        assert!(historical_import_previews(&readonly, "other-scan").unwrap().is_empty());
        drop(readonly);
        connection.execute(
            "UPDATE import_projection_memberships SET source_path='' WHERE scope_key='scan=history-preview'",
            [],
        ).unwrap();
        db::initialize(&app_dir).unwrap();
        assert!(!historical_import_previews(&connection, "history-preview").unwrap().is_empty(),
            "单一作用域的升级库应能从旧 bundle 来源安全回填");
        assert_eq!(connection.query_row(
            "SELECT COUNT(*) FROM sentinel_findings WHERE scan_id='history-preview'",
            [], |row| row.get::<_, i64>(0),
        ).unwrap(), before, "历史预览不得写入或晋升 confirmed Finding");
        connection.execute(
            "INSERT INTO sentinel_deleted_scans(scan_id) VALUES('history-preview')",
            [],
        ).unwrap();
        assert!(historical_import_previews(&connection, "history-preview").unwrap().is_empty());
        connection.execute(
            "DELETE FROM sentinel_deleted_scans WHERE scan_id='history-preview'",
            [],
        ).unwrap();
        connection.execute(
            "UPDATE import_projection_memberships SET current=0 WHERE scope_key='scan=history-preview'",
            [],
        ).unwrap();
        assert!(historical_import_previews(&connection, "history-preview").unwrap().is_empty());
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_history_ledger_survives_without_legacy_scan_rows() {
        let root = std::env::temp_dir().join(format!(
            "oviraptor-history-ledger-{}", Uuid::new_v4()
        ));
        let app_dir = root.join("app");
        let runs = app_dir.join("old-runs");
        let run_dir = runs.join("ledger-run");
        fs::create_dir_all(&run_dir).unwrap();
        fs::write(run_dir.join("model-prompt-audit.json"), serde_json::json!({
            "instruction":"History Bearer abcdefghijklmnopqrstuvwxyz012345"
        }).to_string()).unwrap();
        fs::write(run_dir.join("candidate.sarif"),
            history_preview_sarif("A/B read", "high", "https://example.invalid/api/one").to_string()).unwrap();
        fs::write(run_dir.join("findings.sarif"), serde_json::json!({
            "version":"2.1.0", "runs":[{"tool":{"driver":{"name":"History Bearer abcdefghijklmnopqrstuvwxyz012345"}},
                "properties":{"reportLabel":"current report"},
                "results":[
                    {"ruleId":"checked-one","kind":"pass","level":"warning","message":{"text":"checked"}},
                    {"ruleId":"follow-up-two","kind":"open","level":"warning","message":{"text":"needs follow-up"}}
                ]}]
        }).to_string()).unwrap();
        let db_path = db::initialize(&app_dir).unwrap();
        let connection = db::open(&db_path).unwrap();
        let report = artifact_import_report(
            &connection, &root.join("cas"), &root.join("import.key"), &[runs],
        ).unwrap();
        assert_eq!(report.imported, 1, "{:?}", report.diagnostics);
        assert_eq!(connection.query_row(
            "SELECT COUNT(*) FROM sentinel_scans WHERE id='ledger-run'", [],
            |row| row.get::<_, i64>(0),
        ).unwrap(), 0, "隔离导入不能为历史任务制造活动扫描行");
        assert_eq!(connection.query_row(
            "SELECT COUNT(*) FROM sentinel_findings", [], |row| row.get::<_, i64>(0),
        ).unwrap(), 0, "历史候选不得晋升为 Native 漏洞");
        let readonly = rusqlite::Connection::open_with_flags(
            &db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ).unwrap();
        let runs = historical_import_runs(&readonly, None).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].scan_id, "ledger-run");
        assert_eq!(runs[0].status, "imported", "imported reports must not imply successful execution");
        assert_eq!(runs[0].finding_candidates, 1);
        assert_eq!(runs[0].coverage_records, 2);
        assert!(!runs[0].title.contains("abcdefghijklmnopqrstuvwxyz012345"));
        let previews = historical_bundle_previews(&readonly, &runs[0].bundle_id, runs[0].row_id).unwrap();
        assert_eq!(previews.iter().filter(|item| item.kind == "finding_candidate").count(), 1);
        assert_eq!(previews.iter().filter(|item| item.kind == "coverage").count(), 2);
        assert!(previews.iter().all(|item| item.read_only && !item.execution_eligible
            && item.review_state == "unreviewed"));
        assert!(historical_bundle_previews(&readonly, "../run.json", runs[0].row_id).is_err());
        assert!(historical_import_runs(&readonly, Some(runs[0].row_id)).unwrap().is_empty());
        drop(readonly);
        connection.execute(
            "INSERT INTO sentinel_deleted_scans(scan_id) VALUES('ledger-run')", [],
        ).unwrap();
        assert!(historical_import_runs(&connection, None).unwrap().is_empty());
        assert!(historical_bundle_previews(&connection, &runs[0].bundle_id, runs[0].row_id).unwrap().is_empty());
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn identical_historical_bundles_stay_separate_by_scan_scope() {
        let root = std::env::temp_dir().join(format!(
            "oviraptor-history-scope-{}", Uuid::new_v4()
        ));
        let app_dir = root.join("app");
        let runs = app_dir.join("old-runs");
        for (directory, scan_id) in [("a", "scan-a"), ("b", "scan-b")] {
            let dir = runs.join(directory);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join(".oviraptor-scan-id"), scan_id).unwrap();
            fs::write(dir.join("findings.sarif"), serde_json::json!({
                "version":"2.1.0", "runs":[{"tool":{"driver":{"name":"same-content"}},
                    "properties":{"reportLabel":"Same content"},"results":[]}]
            }).to_string()).unwrap();
            fs::write(dir.join("candidate.sarif"),
                history_preview_sarif("Candidate", "medium", "").to_string()).unwrap();
        }
        let db_path = db::initialize(&app_dir).unwrap();
        let connection = db::open(&db_path).unwrap();
        let report = artifact_import_report(
            &connection, &root.join("cas"), &root.join("import.key"), &[runs.join("a"), runs.join("b")],
        ).unwrap();
        assert_eq!(report.bundles, 2, "{:?}", report.diagnostics);
        assert_eq!(report.failed, 0, "{:?}", report.diagnostics);
        let visible = historical_import_runs(&connection, None).unwrap();
        assert_eq!(visible.len(), 2, "同一内容地址下仍有两个独立任务作用域");
        assert_eq!(visible[0].bundle_id, visible[1].bundle_id);
        assert_ne!(visible[0].row_id, visible[1].row_id);
        assert_eq!(visible.iter().map(|run| run.scan_id.as_str()).collect::<std::collections::BTreeSet<_>>(),
            std::collections::BTreeSet::from(["scan-a", "scan-b"]));
        for (directory, scan_id) in [("a", "scan-a"), ("b", "scan-b")] {
            connection.execute(
                "INSERT INTO sentinel_scans(id,status,scan_type,task_path) VALUES(?1,'completed','web',?2)",
                rusqlite::params![scan_id, runs.join(directory).to_string_lossy()],
            ).unwrap();
            let previews = historical_import_previews(&connection, scan_id).unwrap();
            assert_eq!(previews.len(), 2, "同一 bundle 哈希不能覆盖先导入任务的来源目录");
            assert!(previews.iter().all(|row| row.scan_id == scan_id));
        }
        for run in &visible {
            let preview = historical_bundle_previews(&connection, &run.bundle_id, run.row_id).unwrap();
            assert_eq!(preview.len(), 2);
            assert!(preview.iter().all(|row| row.scan_id == run.scan_id));
        }
        connection.execute(
            "INSERT INTO sentinel_deleted_scans(scan_id) VALUES('scan-a')", [],
        ).unwrap();
        let visible = historical_import_runs(&connection, None).unwrap();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].scan_id, "scan-b");
        connection.execute(
            "UPDATE import_projection_memberships SET source_path='' WHERE bundle_row_id=(SELECT id FROM import_bundles WHERE bundle_id=?1)",
            [&visible[0].bundle_id],
        ).unwrap();
        drop(connection);
        // Exact scope identities stay distinct even if the old provenance path
        // is unavailable. No producer alias or source-directory join is needed.
        db::initialize(&app_dir).unwrap();
        let connection = db::open(&db_path).unwrap();
        assert_eq!(historical_import_previews(&connection, "scan-b").unwrap().len(), 2);
        assert_eq!(historical_import_runs(&connection, None).unwrap().len(), 1);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_import_status_view_writes_nothing() {
        // 缺口 6：查询导入状态必须完全只读。
        let root =
            std::env::temp_dir().join(format!("oviraptor-import-view-{}", Uuid::new_v4()));
        let app_dir = root.join("oviraptor");
        let run_dir = app_dir.join("current-imports").join("view-run");
        fs::create_dir_all(&run_dir).unwrap();
        fs::write(
            run_dir.join("model-prompt-audit.json"),
            serde_json::json!({"instruction":"view-run"}).to_string(),
        )
        .unwrap();
        fs::write(
            run_dir.join("candidate.sarif"),
            history_preview_sarif("越权读单", "high", "https://example.invalid/api/v1/order").to_string(),
        )
        .unwrap();
        let db_path = db::initialize(&app_dir).unwrap();
        let connection = db::open(&db_path).unwrap();
        let roots = vec![run_dir.parent().unwrap().to_path_buf()];
        let imported = artifact_import_report(
            &connection,
            &root.join("cas"),
            &root.join("artifact-import.key"),
            &roots,
        )
        .unwrap();
        assert_eq!(imported.bundles, 1, "{:?}", imported.diagnostics);

        let tables = ["import_sources", "import_bundles", "import_bundle_files",
            "artifact_objects", "import_record_revisions",
            "import_projection_memberships", "import_diagnostics"];
        let surface = |connection: &rusqlite::Connection| -> Vec<(String, i64)> {
            tables
                .iter()
                .map(|table| {
                    let rows: i64 = connection
                        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                            row.get(0)
                        })
                        .unwrap_or(-1);
                    ((*table).to_string(), rows)
                })
                .collect()
        };
        let listing = |dir: &Path| -> Vec<(String, u64)> {
            let mut rows = Vec::new();
            let mut stack = vec![dir.to_path_buf()];
            while let Some(current) = stack.pop() {
                let Ok(entries) = fs::read_dir(&current) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        stack.push(path);
                        continue;
                    }
                    let name = path.display().to_string();
                    rows.push((name, path.metadata().map(|m| m.len()).unwrap_or(0)));
                }
            }
            rows.sort();
            rows
        };
        let read_only = rusqlite::Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        drop(connection);
        // Baselines are taken on the read-only connection, so SQLite's own WAL
        // housekeeping on closing the writer cannot be mistaken for a view write.
        let before = surface(&read_only);
        let files = listing(&root);
        let viewed = ArtifactImportReport::view(&read_only).expect("只读连接必须能出状态");
        let repeated = ArtifactImportReport::view(&read_only).expect("重复查询同样要成功");
        assert_eq!(viewed.bundles, imported.bundles);
        assert_eq!(viewed.records, imported.records);
        assert_eq!(viewed.revisions, imported.revisions);
        assert_eq!(viewed.matched + viewed.only_canonical, viewed.canonical_candidates);
        assert_eq!(
            viewed.imported + viewed.unchanged + viewed.failed,
            viewed.bundles,
            "状态计数必须自洽：{:?}",
            viewed.bundle_rows.len()
        );
        assert!(!viewed.bundle_rows.is_empty(), "报告要能看到 bundle 明细");
        assert_eq!(
            serde_json::to_string(&viewed).unwrap(),
            serde_json::to_string(&repeated).unwrap(),
            "同一状态下重复查询必须给出完全相同的报告"
        );
        assert_eq!(surface(&read_only), before, "查询状态不得改动任何导入表");
        assert_eq!(listing(&root), files, "查询状态不得写 CAS/scratch/密钥");
        assert!(
            !root.join("artifact-import.key").exists(),
            "没有含凭据的原文时，查询状态不得创建密钥"
        );
        // The discriminating case: new results appear on disk. A status read must not
        // pick them up — only the explicit import may.
        let later = app_dir.join("current-imports").join("view-run-2");
        fs::create_dir_all(&later).unwrap();
        fs::write(
            later.join("model-prompt-audit.json"),
            serde_json::json!({"instruction":"view-run-2"}).to_string(),
        )
        .unwrap();
        drop(read_only);
        let observer = rusqlite::Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let after_new_files = ArtifactImportReport::view(&observer).unwrap();
        assert_eq!(
            after_new_files.bundles, viewed.bundles,
            "§缺口 6：状态查询顺手导入了新结果"
        );
        drop(observer);
        let connection = db::open(&db_path).unwrap();
        let explicit = artifact_import_report(
            &connection,
            &root.join("cas"),
            &root.join("artifact-import.key"),
            &roots,
        )
        .unwrap();
        assert_eq!(
            explicit.bundles,
            viewed.bundles + 1,
            "显式导入必须看到刚落盘的那批结果"
        );
        let _ = fs::remove_dir_all(root);
    }
