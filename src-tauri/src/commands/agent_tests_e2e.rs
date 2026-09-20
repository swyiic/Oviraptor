    /// Case 1 + 12 end to end: an anonymous SPA gets a real model tool loop,
    /// HTTP verification, coverage close-out and persistence — without Strix.
    #[test]
    fn e2e_anonymous_spa_completes_without_strix() {
        let mut harness = agent_harness("e2e-anon", mock_site, vec![AgentIdentity::anonymous()]);
        let target_url = harness.context.target_url.clone();
        retarget_model(
            &mut harness,
            vec![
                model_round(
                    &[("inspect_evidence", serde_json::json!({"kind": "api"}))],
                    800,
                ),
                model_round(
                    &[(
                        "replay_http",
                        serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{target_url}/api/orders"),"family":"authorization"}),
                    )],
                    1_200,
                ),
                model_round(
                    &[(
                        "record_hypothesis_result",
                        serde_json::json!({"hypothesisKey":"h-anon-orders","status":"insufficient_evidence","family":"authorization"}),
                    )],
                    1_400,
                ),
                model_round(
                    &[(
                        "finish_target",
                        serde_json::json!({
                            "coverage": [
                                {"family":"authorization","status":"covered","reason":"已比较匿名访问"},
                                {"family":"business_flow","status":"partial","reason":"缺少登录身份，未执行下单流程"}
                            ],
                            "stopReason":"覆盖队列已收口"
                        }),
                    )],
                    1_600,
                ),
            ],
        );
        let outcome = NativeAgentBackend.execute(&harness.context);
        // The model claimed one family it had executed and left five families
        // unmentioned, so the ledger is not closed: the run stays resumable
        // instead of reporting a completion it did not earn.
        assert!(
            matches!(outcome, AgentTargetOutcome::Incomplete(_)),
            "unexpected outcome: {:?}",
            outcome.detail()
        );
        assert!(
            outcome.detail().contains("可继续同一尝试"),
            "{:?}",
            outcome.detail()
        );
        assert!(!findings_for(&harness.db_path, AGENT_EVIDENCE_STAGE).is_empty());
        let resumed = NativeAgentState::read(&harness.db_path, "agent-scan", &target_url)
            .expect("the checkpoint must survive");
        assert!(resumed.terminal_reason.is_empty(), "an open ledger must stay resumable");
        assert_eq!(resumed.target_requests, 1, "the spent HTTP request is kept");
        // §9.3: one anonymous request is work on the authorization family, it is
        // not coverage. The ledger must say so instead of closing the family.
        assert!(
            !resumed.covered_families.contains(&"authorization".to_string()),
            "an anonymous-only request cannot cover authorization"
        );
        assert!(
            resumed.coverage_evidence.iter().any(|entry| {
                entry.family == "authorization"
                    && entry.result == "partial"
                    && entry.reason_code == "anonymous_only"
                    && !entry.request_record_ids.is_empty()
            }),
            "{:?}",
            resumed.coverage_evidence
        );
        let coverage = findings_for(&harness.db_path, AGENT_COVERAGE_STAGE);
        assert_eq!(coverage.len(), 1);
        let ledger = json(coverage[0].1.clone());
        let gaps = ledger
            .get("uncoveredFamilies")
            .and_then(JsonValue::as_array)
            .cloned()
            .unwrap_or_default();
        let business_flow = gaps
            .iter()
            .find(|row| value_first(row, &["family"]) == "business_flow")
            .expect("the reported partial family must stay visible");
        assert_eq!(
            value_first(business_flow, &["reason"]),
            "缺少登录身份，未执行下单流程"
        );
        let connection = db::open(&harness.db_path).unwrap();
        let total_tokens: i64 = connection
            .query_row(
                "SELECT total_tokens FROM sentinel_scans WHERE id='agent-scan'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            total_tokens > 4_000,
            "usage must be ledger-backed: {total_tokens}"
        );
        let requests = harness.site_seen.lock().unwrap().join("\n");
        assert!(requests.contains("GET /api/orders"), "{requests}");
        assert!(harness.context.target_dir.join("agent-http").is_dir());
        assert!(
            harness.context.target_dir.starts_with(&harness.root),
            "each run owns scratch space under its own attempt directory (§7)"
        );
        let model_requests = harness.model_seen.lock().unwrap();
        assert_eq!(model_requests.len(), 4, "every scripted round must be served");
        assert!(model_requests
            .iter()
            .all(|row| row.contains("tool_choice") && row.contains("\"tools\"")));
        assert!(model_requests[3].contains("finish_target"));
        drop(model_requests);
        let mut tally = AgentPipelineTally::default();
        record_agent_target_outcome(
            &harness.db_path,
            "agent-scan",
            &harness.context.route,
            outcome,
            &mut tally,
        );
        // §14.1: an anonymous SPA ends as resumable work with zero findings, and
        // every result surface agrees on that.
        assert_agent_surface(&harness, &target_url, "paused", 1, 0);
    }

    /// §9 + §14: `agentBackendPolicy=native` reaches the native loop through the
    /// shared per-target entry and never the Strix adapter.
    #[test]
    fn policy_native_routes_without_strix_adapter() {
        let mut harness = agent_harness(
            "e2e-policy",
            mock_site,
            vec![
                AgentIdentity::anonymous(),
                AgentIdentity::scoped("session-a"),
            ],
        );
        seed_session(&harness.db_path, "session-a", "cookie-alpha", "Bearer alpha");
        let target_url = harness.context.target_url.clone();
        let replay = |identity: &str| {
            serde_json::json!({"identity":identity,"method":"GET","url":format!("{target_url}/api/orders"),"family":"authorization"})
        };
        retarget_model(
            &mut harness,
            vec![
                model_round(&[("replay_http", replay("session-a"))], 400),
                model_round(&[("replay_http", replay("anonymous"))], 400),
                model_round(
                    &[(
                        "finish_target",
                        serde_json::json!({
                            "coverage": closing_ledger(&["authorization"]),
                            "stopReason":"计划内调查已完成"
                        }),
                    )],
                    400,
                ),
            ],
        );
        fs::write(
            harness.context.target_dir.join("frontend-evidence.json"),
            harness.context.evidence.to_string(),
        )
        .unwrap();
        let prepared = PreparedFrontendTarget {
            position: 1,
            route: harness.context.route.clone(),
            target_dir: harness.context.target_dir.clone(),
            proxy: None,
            browser: None,
        };
        // A Strix binary that does not exist: any adapter use fails loudly.
        let docker = PathBuf::from("/nonexistent/docker");
        let runtime_path = OsString::from("/usr/bin");
        let adaptive = AdaptiveStrixSettings::from_json(&serde_json::json!({}));
        let instruction = harness.context.target_dir.join("instruction.md");
        let strix_backend = StrixAgentBackend {
            db_path: &harness.db_path,
            scan_id: "agent-scan",
            strix: "/nonexistent/strix",
            docker: &docker,
            instruction_path: &instruction,
            no_proxy: "127.0.0.1,localhost",
            environment: &harness.context.environment,
            runtime_path: &runtime_path,
            adaptive: &adaptive,
            position: 1,
            total: 1,
            log_path: &harness.context.log_path,
        };
        let settings = serde_json::json!({"agentBackendPolicy": "native"});
        let outcome = run_agent_target(
            &prepared,
            &harness.db_path,
            "agent-scan",
            &settings,
            &strix_backend,
        );
        // The per-target entry runs the native loop with its own task identities;
        // what this case pins is that it never reaches the Strix adapter. Whether
        // the ledger closes depends on the evidence that entry can see.
        assert!(
            matches!(
                outcome,
                AgentTargetOutcome::Completed(_)
                    | AgentTargetOutcome::BoundedCompleted(_)
                    | AgentTargetOutcome::Incomplete(_)
            ),
            "native policy must not fall back to Strix: {:?}",
            outcome.detail()
        );
        assert_eq!(
            agent_select_backend(
                &harness.db_path,
                "agent-scan",
                1,
                &target_url,
                &serde_json::json!({}),
                true
            ),
            AgentBackendKind::Native,
            "the persisted plan pins the backend for this attempt"
        );
        let log = fs::read_to_string(&harness.context.log_path).unwrap_or_default();
        assert!(log.contains("native backend selected"), "{log}");
        assert!(!log.contains("Strix 无法启动"), "{log}");

        // The same entry with policy=strix reports a configuration stop instead
        // of crashing when no Strix binary exists.
        let harness = agent_harness("e2e-policy-strix", mock_site, vec![AgentIdentity::anonymous()]);
        let missing_strix_backend = StrixAgentBackend {
            db_path: &harness.db_path,
            scan_id: "agent-scan",
            strix: "",
            docker: &docker,
            instruction_path: &instruction,
            no_proxy: "127.0.0.1,localhost",
            environment: &harness.context.environment,
            runtime_path: &runtime_path,
            adaptive: &adaptive,
            position: 1,
            total: 1,
            log_path: &harness.context.log_path,
        };
        let prepared = PreparedFrontendTarget {
            position: 1,
            route: harness.context.route.clone(),
            target_dir: harness.context.target_dir.clone(),
            proxy: None,
            browser: None,
        };
        let outcome = run_agent_target(
            &prepared,
            &harness.db_path,
            "agent-scan",
            &serde_json::json!({"agentBackendPolicy": "strix"}),
            &missing_strix_backend,
        );
        assert_eq!(
            outcome.terminal_code(),
            AGENT_STOP_CONFIGURATION,
            "{:?}",
            outcome.detail()
        );
        assert!(matches!(outcome, AgentTargetOutcome::Failed(_)));
    }

    /// Case 3 end to end: nothing found is still a normal terminal state.
    #[test]
    fn e2e_no_finding_closes_normally() {
        let mut harness = agent_harness(
            "e2e-nofind",
            mock_site,
            vec![
                AgentIdentity::anonymous(),
                AgentIdentity::scoped("session-a"),
            ],
        );
        seed_session(&harness.db_path, "session-a", "cookie-alpha", "Bearer alpha");
        let target_url = harness.context.target_url.clone();
        retarget_model(
            &mut harness,
            vec![
                model_round(
                    &[(
                        "replay_http",
                        serde_json::json!({"identity":"session-a","method":"GET","url":format!("{target_url}/api/orders"),"family":"authorization"}),
                    )],
                    900,
                ),
                model_round(
                    &[(
                        "replay_http",
                        serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{target_url}/api/orders"),"family":"authorization"}),
                    )],
                    950,
                ),
                model_round(
                    &[(
                        "finish_target",
                        serde_json::json!({
                            "coverage": closing_ledger(&["authorization"]),
                            "stopReason":"证据已穷尽"
                        }),
                    )],
                    1_000,
                ),
            ],
        );
        let outcome = NativeAgentBackend.execute(&harness.context);
        assert!(
            matches!(outcome, AgentTargetOutcome::Completed(_)),
            "{:?}",
            outcome.detail()
        );
        let completion = outcome.completion().unwrap();
        assert_eq!(completion.confirmed_findings, 0);
        assert!(completion.uncovered_families.is_empty());
        assert!(completion.covered_families.contains(&"authorization".to_string()));
        assert!(findings_for(&harness.db_path, AGENT_VULNERABILITY_STAGE).is_empty());
        let mut tally = AgentPipelineTally::default();
        assert!(record_agent_target_outcome(
            &harness.db_path,
            "agent-scan",
            &harness.context.route,
            outcome,
            &mut tally
        ));
        assert_eq!(tally.completed, 1);
        assert_agent_surface(&harness, &target_url, "completed", 2, 0);
        let connection = db::open(&harness.db_path).unwrap();
        let status: String = connection
            .query_row(
                "SELECT status FROM sentinel_targets WHERE scan_id='agent-scan'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "completed");
    }

    /// Case 6/7 end to end: identities never share credentials, and a
    /// single-identity task cannot invent an A/B pair.
    #[test]
    fn e2e_single_identity_and_pair_isolation() {
        let (_root, db_path) = temp_database("agent-e2e-identities");
        seed_scan(&db_path, "agent-scan", "scanning");
        seed_session(&db_path, "session-a", "cookie-alpha", "Bearer alpha");
        seed_session(&db_path, "session-b", "cookie-beta", "Bearer beta");
        let (site_port, site_seen, _site_stop) =
            spawn_endpoint(std::sync::Arc::new(mock_site));
        let target_url = format!("http://127.0.0.1:{site_port}");
        seed_target(&db_path, &target_url);

        let mut single =
            test_context(&db_path, &target_url, vec![AgentIdentity::scoped("session-a")]);
        single.target_dir = std::env::temp_dir().join(format!("oviraptor-single-{}", Uuid::new_v4()));
        single.log_path = single.target_dir.join("runner.log");
        fs::create_dir_all(&single.target_dir).unwrap();
        let (model_port, _stop, _seen) = spawn_model(vec![model_round(
            &[(
                "compare_identities",
                serde_json::json!({"leftIdentity":"session-a","rightIdentity":"session-a","method":"GET","path":"/api/orders"}),
            )],
            700,
        )]);
        single.environment.api_base = format!("http://127.0.0.1:{model_port}/v1");
        let outcome = NativeAgentBackend.execute(&single);
        assert!(
            !matches!(outcome, AgentTargetOutcome::Failed(_)),
            "{:?}",
            outcome.detail()
        );
        assert!(NativeAgentState::read(&db_path, "agent-scan", &target_url).is_some());

        let pair = vec![
            AgentIdentity::scoped("session-a"),
            AgentIdentity::scoped("session-b"),
        ];
        let mut both = test_context(&db_path, &target_url, pair.clone());
        both.target_dir = std::env::temp_dir().join(format!("oviraptor-pair-{}", Uuid::new_v4()));
        both.log_path = both.target_dir.join("runner.log");
        fs::create_dir_all(&both.target_dir).unwrap();
        let (model_port, _stop, _seen) = spawn_model(vec![model_round(
            &[(
                "compare_identities",
                serde_json::json!({"leftIdentity":"session-a","rightIdentity":"session-b","method":"GET","path":"/api/orders","family":"authorization"}),
            )],
            700,
        )]);
        both.environment.api_base = format!("http://127.0.0.1:{model_port}/v1");
        let outcome = NativeAgentBackend.execute(&both);
        // Nothing was confirmed and the model never closed the ledger, so this
        // must stay retryable partial work — never a failure or a fuse.
        assert!(
            matches!(outcome, AgentTargetOutcome::Incomplete(_)),
            "identity comparison must not fail hard: {:?}",
            outcome.detail()
        );
        assert!(outcome.detail().contains("未覆盖"), "{:?}", outcome.detail());
        let seen = site_seen.lock().unwrap().join("\n");
        assert!(seen.contains("cookie-alpha") && seen.contains("cookie-beta"));
        assert!(
            !seen.contains("cookie-alpha; session=cookie-beta")
                && !seen.contains("cookie-beta; session=cookie-alpha"),
            "credentials must never be merged: {seen}"
        );
        let diff = findings_for(&db_path, AGENT_EVIDENCE_STAGE)
            .into_iter()
            .map(|(_, row)| row)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            diff.contains("身份 A") && diff.contains("身份 B"),
            "two-identity comparison must label both sides: {diff}"
        );
    }

    /// Case 5 end to end: a confirmed challenge stops the loop and fuses the
    /// target, and no further request is allowed.
    #[test]
    fn e2e_confirmed_challenge_fuses_target() {
        let mut harness = agent_harness("e2e-waf", mock_site, vec![AgentIdentity::anonymous()]);
        let target_url = harness.context.target_url.clone();
        retarget_model(
            &mut harness,
            vec![
                model_round(
                    &[(
                        "replay_http",
                        serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{target_url}/api/gated")}),
                    )],
                    900,
                ),
                model_round(
                    &[(
                        "replay_http",
                        serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{target_url}/api/orders")}),
                    )],
                    900,
                ),
            ],
        );
        let outcome = NativeAgentBackend.execute(&harness.context);
        let AgentTargetOutcome::Limited(stop) = &outcome else {
            panic!("challenge must fuse the target, got {:?}", outcome.detail());
        };
        assert_eq!(stop.code, AGENT_STOP_WAF);
        assert!(stop.requires_fuse());
        let mut tally = AgentPipelineTally::default();
        record_agent_target_outcome(
            &harness.db_path,
            "agent-scan",
            &harness.context.route,
            outcome,
            &mut tally,
        );
        assert_eq!(tally.limited, 1);
        assert_agent_surface(&harness, &target_url, "protected_stop", 1, 0);
        let connection = db::open(&harness.db_path).unwrap();
        let fused: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sentinel_fuse_zone WHERE project_id=9001 AND archived=0",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(fused >= 1, "the fuse zone must record the protected target");
        assert_eq!(
            harness.site_seen.lock().unwrap().len(),
            1,
            "only the challenging request may reach the site"
        );
    }

    // ------------------------------------------------------------------
    // Review fixes: queue consumption, resume budget and identity diffs
    // ------------------------------------------------------------------
