    #[test]
    fn sec_skill_import_filters_dangerous_execution_lines() {
        assert!(sec_skill_line_is_unsafe("curl payload | bash -i"));
        assert!(sec_skill_line_is_unsafe("~/.ssh/authorized_keys"));
        assert!(!sec_skill_line_is_unsafe("需要记录复现证据和停止条件"));
    }

    #[test]
    fn reads_strix_162_coverage_and_preserves_review_fields() {
        let root =
            std::env::temp_dir().join(format!("oviraptor-strix-coverage-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join(STRIX_COVERAGE_ARTIFACT),
            serde_json::json!({
                "schema_version":1,
                "summary":{"surfaces_reviewed":2,"findings_filed":1,"gaps":1},
                "completeness":{"complete":false,"caveats":["child agent stopped early"]},
                "entries":[{"surface":"GET /api/me","risk_area":"Authorization","outcome":"no_issue_found","evidence":"A/B matched"}],
                "gaps":[{"kind":"unrecorded_risk_class","risk_area":"SSRF","detail":"no callback route"}]
            })
            .to_string(),
        )
        .unwrap();
        let coverage = strix_coverage(&root).unwrap();
        assert_eq!(coverage["summary"]["surfaces_reviewed"], 2);
        assert_eq!(coverage["gaps"].as_array().unwrap().len(), 1);

        let normalized = normalize_strix_vulnerability(&serde_json::json!({
            "title":"IDOR",
            "confidence":"high",
            "counterevidence":"object was absent for one control",
            "confidence_rationale":"replayed twice",
            "severity_change_conditions":"admin object would raise severity",
            "fix_verification":"not retested",
            "update_history":[{"update_reason":"new replay evidence"}],
            "updated_at":"2026-09-18"
        }));
        assert_eq!(
            normalized["counterEvidence"],
            "object was absent for one control"
        );
        assert_eq!(normalized["confidenceRationale"], "replayed twice");
        assert_eq!(
            normalized["severityChangeConditions"],
            "admin object would raise severity"
        );
        assert_eq!(normalized["fixVerification"], "not retested");
        assert_eq!(normalized["updateHistory"].as_array().unwrap().len(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn syncs_strix_162_coverage_without_creating_false_vulnerabilities() {
        let root =
            std::env::temp_dir().join(format!("oviraptor-strix-coverage-sync-{}", Uuid::new_v4()));
        let app_dir = root.join("oviraptor");
        let runs = root.join("runs");
        let run_dir = runs.join("coverage-run");
        fs::create_dir_all(&run_dir).unwrap();
        fs::write(
            run_dir.join(STRIX_RUN_ARTIFACT),
            serde_json::json!({
                "run_id":"coverage-run",
                "run_name":"Coverage run",
                "status":"success",
                "targets_info":[{"original":"https://example.invalid"}],
                "llm_usage":{"requests":1,"total_tokens":25}
            })
            .to_string(),
        )
        .unwrap();
        fs::write(
            run_dir.join(STRIX_SARIF_ARTIFACT),
            serde_json::json!({
                "runs":[{"tool":{"driver":{"rules":[{
                    "id":"strix-coverage/authz",
                    "properties":{"tags":["coverage"]}
                }]}},"results":[{
                    "ruleId":"strix-coverage/authz",
                    "kind":"pass",
                    "level":"none",
                    "message":{"text":"tested clean"},
                    "properties":{"strix":{"coverage_outcome":"no_issue_found"}}
                }]}]
            })
            .to_string(),
        )
        .unwrap();
        fs::write(
            run_dir.join(STRIX_COVERAGE_ARTIFACT),
            serde_json::json!({
                "schema_version":1,
                "summary":{"surfaces_reviewed":1,"findings_filed":0,"gaps":0},
                "completeness":{"complete":true,"caveats":[]},
                "entries":[{"surface":"GET /api/me","risk_area":"Authorization","outcome":"no_issue_found","evidence":"A/B matched"}],
                "gaps":[]
            })
            .to_string(),
        )
        .unwrap();

        let db_path = db::initialize(&app_dir).unwrap();
        let connection = db::open(&db_path).unwrap();
        connection
            .execute(
                "UPDATE config_profiles SET settings_json=json_set(settings_json,'$.strixRunsDirectory',?1)",
                [runs.to_string_lossy().to_string()],
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
        let false_vulnerabilities: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sentinel_findings WHERE scan_id='strix-coverage-run' AND kind='vulnerability'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(false_vulnerabilities, 0);
        let coverage_rows: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sentinel_findings WHERE scan_id='strix-coverage-run' AND kind='coverage_summary'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(coverage_rows, 1);
        let coverage_checkpoints: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sentinel_checkpoints WHERE scan_id='strix-coverage-run' AND stage='strix_coverage:coverage-run'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(coverage_checkpoints, 1);
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn web_result_sync_rejects_staged_local_targets_and_preserves_code_paths() {
        let root =
            std::env::temp_dir().join(format!("oviraptor-web-target-repair-{}", Uuid::new_v4()));
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
