    #[test]
    fn investigation_gate_requires_an_explicit_ready_hypothesis() {
        assert!(!investigation_model_gate_open(
            false,
            &serde_json::json!({"eligibleForModel": true, "readyHypotheses": 1}),
        ));
        assert!(!investigation_model_gate_open(
            true,
            &serde_json::json!({"eligibleForModel": false, "readyHypotheses": 3}),
        ));
        assert!(!investigation_model_gate_open(
            true,
            &serde_json::json!({"eligibleForModel": true, "readyHypotheses": 0}),
        ));
        assert!(investigation_model_gate_open(
            true,
            &serde_json::json!({"eligibleForModel": true, "readyHypotheses": 1}),
        ));
    }

    #[test]
    fn investigation_gate_fails_closed_when_contract_fields_are_missing() {
        assert!(!investigation_model_gate_open(
            true,
            &serde_json::json!({"baseline": {"available": false}}),
        ));
    }

    #[test]
    fn progressive_baseline_requires_an_explicit_deterministic_decision() {
        assert!(investigation_baseline_gate_open(&serde_json::json!({
            "baselineInvestigationAllowed": true
        })));
        assert!(!investigation_baseline_gate_open(&serde_json::json!({
            "runtimeProbeAvailable": true
        })));
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
                "source":"runtime-request",
                "readiness":{"stage":"agent_ready"},
                "riskEvidence":{"present":true,"signals":[{"type":"security_relevant_mutation"}]},
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
        let environment = StrixRuntimeEnv {
            llm: "openai/test".into(),
            api_key: String::new(),
            api_base: String::new(),
            image: String::new(),
            deployment: "local".into(),
            full_power: true,
            prompt_audit_mode: "off".into(),
        };
        let full_power_plan =
            build_agent_execution_plan(
            &adaptive,
            &full_power_routes[1],
            &environment,
            AgentBackendKind::Strix,
            Path::new("/nonexistent/oviraptor.sqlite3"),
            "plan-scan",
        );
        assert_eq!(full_power_plan.timeout_seconds, 1_800);
        assert_eq!(full_power_plan.soft_uncached_tokens, 800_000);
        assert_eq!(full_power_plan.hard_total_tokens, 2_800_000);
        assert_eq!(full_power_plan.hard_model_requests, 32);
        let evidence =
            compact_frontend_evidence(&framework_only, &guarded[0].url, &guarded[0], 20 * 1024);
        assert!(!evidence["aiFallback"].as_object().unwrap().is_empty());
        assert_eq!(
            evidence["verificationPlan"]["boundedFallbackDiscoveryAllowed"],
            false
        );
        assert!(evidence["stopRule"]
            .as_str()
            .unwrap()
            .contains("finish the target"));

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
    fn compact_frontend_evidence_keeps_replayable_baseline_without_auth_secrets() {
        let adaptive = AdaptiveStrixSettings::from_json(&serde_json::json!({}));
        let target = serde_json::json!({
            "url":"https://app.example.invalid",
            "jsFiles":[{"url":"https://app.example.invalid/app.js","type":"application"}],
            "apis":[{
                "url":"https://app.example.invalid/check_login",
                "path":"/check_login",
                "method":"POST",
                "confidence":"high",
                "extractionEngine":"browser-runtime",
                "verification":{"status":"observed_runtime","httpStatus":200,"probeMethod":"POST","resolvedUrl":"https://app.example.invalid/check_login"},
                "identityObservations":[{
                    "identityKey":"identity-a",
                    "observed":true,
                    "method":"POST",
                    "url":"https://app.example.invalid/check_login",
                    "status":200,
                    "contentType":"application/json",
                    "requestBody":"link=42&password=real-secret",
                    "requestHeaders":{"Content-Type":"application/x-www-form-urlencoded","Cookie":"sid=secret","X-CSRF-Token":"secret","X-Business-Mode":"review"},
                    "responseBody":"{\"ok\":true,\"item\":42}",
                    "responseKeys":["ok","item"],
                    "responseBytes":21
                }]
            }],
            "routes":[],
            "sensitiveInfo":[]
        });
        let route = score_frontend_target(&target, "https://app.example.invalid", &adaptive);
        let evidence = compact_frontend_evidence(&target, &route.url, &route, 20 * 1024);
        let api = &evidence["apiCandidates"][0];
        assert_eq!(api["verification"]["statusCode"], 200);
        assert_eq!(api["verification"]["method"], "POST");
        assert_eq!(
            api["observations"][0]["request"]["body"],
            "link=42&password=<auth-session>"
        );
        assert_eq!(
            api["observations"][0]["request"]["authMaterialRef"],
            "/workspace/strix-evidence-input/auth-session.json"
        );
        assert_eq!(
            api["observations"][0]["request"]["headers"]["X-Business-Mode"],
            "review"
        );
        assert!(api["observations"][0]["request"]["headers"]
            .get("Cookie")
            .is_none());
        assert!(api["observations"][0]["request"]["headers"]
            .get("X-CSRF-Token")
            .is_none());
        assert_eq!(
            api["observations"][0]["response"]["body"],
            "{\"ok\":true,\"item\":42}"
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

        let (trace, events) = collect_strix_trace(&connection, "trace-test", true, false).unwrap();
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
        assert_eq!(live.verification_tool_results, 1);
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
