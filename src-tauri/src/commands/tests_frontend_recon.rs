    #[test]
    fn static_script_recon_does_not_fetch_an_unapproved_origin() {
        use std::io::Write as _;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let reached = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let seen = reached.clone();
        let server = std::thread::spawn(move || {
            let until = std::time::Instant::now() + Duration::from_secs(2);
            while std::time::Instant::now() < until {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        seen.store(true, std::sync::atomic::Ordering::SeqCst);
                        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                        break;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    Err(error) => panic!("fixture accept: {error}"),
                }
            }
        });
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(1))
            .build()
            .unwrap();
        let (files, _, _, _, _) = static_frontend_intelligence(
            &client,
            "http://127.0.0.1:34567/index",
            &format!("<script src='http://127.0.0.1:{port}/outside.js'></script>"),
            &serde_json::json!({}),
            Path::new("/nonexistent/runtime-probe.cjs"),
            &OsString::new(),
            &NativeReconControl::new(Duration::from_secs(3)),
        );
        server.join().unwrap();
        assert!(!reached.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(files[0]["reason"], "outside_target_origin");
    }

    #[test]
    fn static_recon_client_does_not_follow_a_cross_origin_redirect() {
        use std::io::{Read as _, Write as _};
        let entry = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let outside = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        outside.set_nonblocking(true).unwrap();
        let entry_port = entry.local_addr().unwrap().port();
        let outside_port = outside.local_addr().unwrap().port();
        let source = std::thread::spawn(move || {
            let (mut stream, _) = entry.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request).unwrap();
            let redirect = format!(
                "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{outside_port}/outside.js\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            stream.write_all(redirect.as_bytes()).unwrap();
        });
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .redirect(native_recon_redirect_policy())
            .build()
            .unwrap();
        let response = client
            .get(format!("http://127.0.0.1:{entry_port}/index"))
            .send()
            .unwrap();
        source.join().unwrap();
        assert_eq!(response.status().as_u16(), 302);
        assert_eq!(response.url().port(), Some(entry_port));
        let until = std::time::Instant::now() + Duration::from_millis(250);
        while std::time::Instant::now() < until {
            match outside.accept() {
                Ok(_) => panic!("static recon escaped to a different origin"),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("fixture accept: {error}"),
            }
        }
    }

    #[test]
    fn frontend_recon_watchdog_is_one_budget_per_target_url() {
        assert_eq!(
            frontend_recon_hard_timeout_seconds(
                0,
                &FrontendReconConfig {
                    hard_timeout_seconds: 120,
                    browser_request_timeout_seconds: 30,
                    exploration_timeout_seconds: 90
                }
            ),
            120
        );
        assert_eq!(
            frontend_recon_hard_timeout_seconds(
                1,
                &FrontendReconConfig {
                    hard_timeout_seconds: 120,
                    browser_request_timeout_seconds: 30,
                    exploration_timeout_seconds: 90
                }
            ),
            120
        );
        assert_eq!(
            frontend_recon_hard_timeout_seconds(
                2,
                &FrontendReconConfig {
                    hard_timeout_seconds: 120,
                    browser_request_timeout_seconds: 30,
                    exploration_timeout_seconds: 90
                }
            ),
            120
        );
        assert_eq!(
            frontend_recon_hard_timeout_seconds(
                8,
                &FrontendReconConfig {
                    hard_timeout_seconds: 120,
                    browser_request_timeout_seconds: 30,
                    exploration_timeout_seconds: 90
                }
            ),
            120
        );
    }

    #[test]
    fn frontend_recon_exploration_budget_is_shared_by_identities() {
        let config = FrontendReconConfig {
            hard_timeout_seconds: 120,
            browser_request_timeout_seconds: 30,
            exploration_timeout_seconds: 90,
        };
        assert_eq!(frontend_recon_exploration_timeout_seconds(1, &config), 90);
        assert_eq!(frontend_recon_exploration_timeout_seconds(2, &config), 50);
        assert_eq!(frontend_recon_exploration_timeout_seconds(8, &config), 15);
        assert_eq!(frontend_recon_exploration_timeout_seconds(0, &config), 90);
    }

    #[test]
    fn bounded_frontend_keeps_coordinator_turns_before_no_progress_fuse() {
        assert_eq!(no_progress_request_threshold(true, 3), 3);
        assert_eq!(no_progress_request_threshold(false, 4), 4);
        assert!(!no_progress_fuse_allowed(true, 4, 3, 1, true, 3));
        assert!(!no_progress_fuse_allowed(true, 4, 3, 1, false, 3));
        assert!(no_progress_fuse_allowed(true, 4, 3, 0, false, 3));
    }

    #[test]
    fn native_frontend_contract_keeps_browser_observed_request_shape() {
        let request = serde_json::json!({
            "resourceType":"Fetch","url":"https://example.test/api/profile?id=7","method":"GET",
            "status":200,"queryKeys":["id"],"responseKeys":["id","name"],
            "effectiveRequestHeaders":{"x-client":"web"},"effectiveResponseHeaders":{"content-type":"application/json"}
        });
        let api =
            api_from_runtime(&request).expect("runtime request must become a native API contract");
        assert_eq!(api["method"], "GET");
        assert_eq!(api["path"], "/api/profile");
        assert_eq!(api["source"], "browser-runtime");
        assert_eq!(
            api.pointer("/verification/verified")
                .and_then(JsonValue::as_bool),
            Some(true)
        );
        assert_eq!(
            api.pointer("/requestHeaders/x-client")
                .and_then(JsonValue::as_str),
            Some("web")
        );
    }

    #[test]
    fn process_registry_tracks_frontend_and_native_agent_at_the_same_time() {
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
        sentinel_process_set(&db_path, "pipeline", 202, "native-agent", &root);
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
            let raw = serde_json::json!({
                "url":url,
                "statusCode":200,
                "analysisSummary":{"identityReplayVersion":1,"reconCacheVersion":4}
            })
            .to_string();
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
        let connection = db::open(&db_path).unwrap();
        connection.execute(
            "INSERT INTO sentinel_checkpoints(scan_id,url,stage,raw_json) VALUES('retry','https://stale.invalid','frontend_recon','{\"url\":\"https://stale.invalid\",\"analysisSummary\":{\"identityReplayVersion\":1,\"reconCacheVersion\":1}}')",
            [],
        ).unwrap();
        connection.execute(
            "INSERT INTO sentinel_checkpoints(scan_id,url,stage,raw_json) VALUES('retry','https://legacy.invalid','frontend_recon','{\"url\":\"https://legacy.invalid\"}')",
            [],
        ).unwrap();
        drop(connection);
        assert!(cached_frontend_recon(&db_path, "retry", "https://stale.invalid").is_none());
        assert!(cached_frontend_recon(&db_path, "retry", "https://legacy.invalid").is_none());
        let connection = db::open(&db_path).unwrap();
        connection.execute("UPDATE sentinel_checkpoints SET raw_json=json_set(raw_json,'$.analysisSummary.reconCacheVersion',2) WHERE scan_id='retry' AND url='https://retry.invalid'", []).unwrap();
        drop(connection);
        assert!(cached_frontend_recon(&db_path, "retry", "https://retry.invalid").is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn compact_frontend_evidence_keeps_business_scripts_and_caps_candidates() {
        let adaptive = AgentBudgetSettings::from_json(&serde_json::json!({}));
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
        assert_eq!(evidence["apiCandidates"].as_array().unwrap().len(), 6);
        assert_eq!(evidence["sensitiveCandidates"].as_array().unwrap().len(), 1);
        assert_eq!(evidence["runtimeSignals"].as_array().unwrap().len(), 8);
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
        let adaptive = AgentBudgetSettings::from_json(&serde_json::json!({}));
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
        let adaptive = AgentBudgetSettings::from_json(&serde_json::json!({}));
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

        let exact_probe = AssetQuery {
            probe_view: "browser_review".into(),
            probe_outcome_view: "web_restricted".into(),
            ..AssetQuery::default()
        };
        let (probe_sql, probe_values) = asset_filter(&exact_probe, true);
        assert!(probe_sql.contains("a.probe_outcome=?"));
        assert!(probe_values
            .iter()
            .any(|value| value == &SqlValue::Text("web_restricted".into())));
    }
