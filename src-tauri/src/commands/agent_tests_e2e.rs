    /// Case 1 + 12 end to end: an anonymous SPA gets a real model tool loop,
    /// HTTP verification, coverage close-out and persistence — without Strix.
    #[test]
    fn e2e_anonymous_spa_completes_without_strix() {
        let mut harness = fresh_single_production_harness("e2e-anon", mock_site);
        freeze_fresh_single_production_harness(&mut harness);
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
                                {"family":"business_flow","status":"not_applicable","reason":"缺少登录身份，未执行下单流程"},
                                {"family":"information_disclosure","status":"not_applicable","reason":"本轮未覆盖该族"},
                                {"family":"error_handling","status":"not_applicable","reason":"本轮未覆盖该族"},
                                {"family":"authentication_session","status":"not_applicable","reason":"本轮未覆盖该族"},
                                {"family":"input_reflection_xss","status":"not_applicable","reason":"本轮未覆盖该族"},
                                {"family":"hidden_interface_discovery","status":"not_applicable","reason":"本轮未覆盖该族"}
                            ],
                            "stopReason":"覆盖队列已收口"
                        }),
                    )],
                    1_600,
                ),
            ],
        );
        let owned = execute_fresh_single_production_harness(&mut harness);
        let outcome = &owned.outcome;
        // Early finish_target is refused while families and budget remain. The
        // script then goes quiet, so the attempt ends on the no-progress window
        // as bounded work, not as a pause waiting for a person to click resume.
        assert!(
            matches!(outcome, AgentTargetOutcome::BoundedCompleted(_)),
            "unexpected outcome: {:?}",
            outcome.detail()
        );
        assert!(
            !outcome.detail().contains("可继续同一尝试"),
            "{:?}",
            outcome.detail()
        );
        assert!(!findings_for(&harness.db_path, AGENT_EVIDENCE_STAGE).is_empty());
        let resumed = NativeAgentState::read(&harness.db_path, "agent-scan", &target_url)
            .expect("the checkpoint must survive");
        assert!(!resumed.terminal_reason.is_empty(), "a bounded stop records why it ended");
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
        // The actual user entry registers the original Single Root before SDK
        // dispatch. Its four model dimensions must match the checkpoint and
        // the provider calls, checked by assert_fresh_single_surface below.
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
        assert!(
            harness.model_seen.lock().unwrap().len() > 4,
            "提前收口被拒绝后必须继续请求模型"
        );
        let requests = harness.site_seen.lock().unwrap().join("\n");
        assert!(requests.contains("GET /api/orders"), "{requests}");
        assert!(harness.context.target_dir.join("agent-http").is_dir());
        assert!(
            harness.context.target_dir.starts_with(&harness.root),
            "each run owns scratch space under its own attempt directory (§7)"
        );
        let model_requests = harness.model_seen.lock().unwrap();
        assert!(model_requests.len() > 4, "every scripted round must be served, then the loop continues");
        assert!(model_requests
            .iter()
            .all(|row| row.contains("tool_choice") && row.contains("\"tools\"")));
        assert!(model_requests[3].contains("finish_target"));
        drop(model_requests);
        let mut tally = AgentPipelineTally::default();
        assert!(record_owned_agent_target_outcome(
            &harness.db_path,
            &harness.context.scan_id,
            &harness.context.route,
            &owned,
            &mut tally,
        ));
        // §14.1: an anonymous SPA ends as resumable work with zero findings, and
        // every result surface agrees on that.
        assert_fresh_single_surface(&harness, &target_url, "completed_with_gaps", 1, 0);
    }

    include!("agent_tests_native_policy_original.rs");

    /// Case 3 end to end: nothing found is still a normal terminal state.
    #[test]
    fn e2e_no_finding_closes_normally() {
        let mut harness = fresh_single_production_harness_with_sessions(
            "e2e-nofind",
            mock_site,
            &[("session-a", "cookie-alpha", "Bearer alpha")],
        );
        freeze_fresh_single_production_harness(&mut harness);
        let target_url = harness.context.target_url.clone();
        retarget_model(
            &mut harness,
            vec![
                model_round(
                    &[(
                        "compare_identities",
                        serde_json::json!({"leftIdentity":"session-a","rightIdentity":"anonymous","method":"GET","path":"/api/orders","family":"authorization"}),
                    )],
                    900,
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
        let owned = execute_fresh_single_production_harness(&mut harness);
        let outcome = &owned.outcome;
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
        let requests = harness.site_seen.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests.iter().all(|wire| wire.starts_with("GET /api/orders HTTP/")));
        let authenticated: Vec<_> = requests.iter().filter(|wire| wire.contains("cookie-alpha")).collect();
        assert_eq!(authenticated.len(), 1);
        assert!(authenticated[0].contains("Bearer alpha"));
        let anonymous: Vec<_> = requests.iter().filter(|wire| !wire.contains("cookie-alpha")).collect();
        assert_eq!(anonymous.len(), 1);
        assert!(!anonymous[0].to_ascii_lowercase().contains("cookie:"));
        assert!(!anonymous[0].to_ascii_lowercase().contains("authorization:"));
        drop(requests);
        let models = harness.model_seen.lock().unwrap().join("\n");
        assert!(!models.contains("cookie-alpha") && !models.contains("Bearer alpha"));
        let mut tally = AgentPipelineTally::default();
        assert!(record_owned_agent_target_outcome(
            &harness.db_path,
            &harness.context.scan_id,
            &harness.context.route,
            &owned,
            &mut tally
        ));
        assert_eq!(tally.completed, 1);
        assert_fresh_single_surface(&harness, &target_url, "completed", 2, 0);
        let connection = db::open(&harness.db_path).unwrap();
        let status: String = connection
            .query_row(
                "SELECT status FROM sentinel_targets WHERE scan_id='agent-scan'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "completed");
        let before = single_finally_physical(&connection);
        let calls = harness.model_seen.lock().unwrap().len();
        assert!(record_owned_agent_target_outcome(
            &harness.db_path, &harness.context.scan_id, &harness.context.route,
            &owned, &mut tally,
        ));
        assert_eq!(single_finally_physical(&connection), before);
        assert_eq!(tally.counted(), 1);
        assert_eq!(harness.model_seen.lock().unwrap().len(), calls);
        assert_eq!(harness.site_seen.lock().unwrap().len(), 2);
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
        single.target_dir = std::env::temp_dir().canonicalize().unwrap()
            .join(format!("oviraptor-single-{}", Uuid::new_v4()));
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
        both.target_dir = std::env::temp_dir().canonicalize().unwrap()
            .join(format!("oviraptor-pair-{}", Uuid::new_v4()));
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
        // The script stops after one comparison. The loop keeps going until the
        // no-progress window, then closes with the gaps still named.
        assert!(
            matches!(outcome, AgentTargetOutcome::BoundedCompleted(_)),
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
        let mut harness = fresh_single_production_harness("e2e-waf", mock_site);
        freeze_fresh_single_production_harness(&mut harness);
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
        let owned = execute_fresh_single_production_harness(&mut harness);
        let outcome = &owned.outcome;
        let AgentTargetOutcome::Limited(stop) = &outcome else {
            panic!("challenge must fuse the target, got {:?}", outcome.detail());
        };
        assert_eq!(stop.code, AGENT_STOP_WAF);
        assert!(stop.requires_fuse());
        let mut tally = AgentPipelineTally::default();
        assert!(record_owned_agent_target_outcome(
            &harness.db_path,
            &harness.context.scan_id,
            &harness.context.route,
            &owned,
            &mut tally,
        ));
        assert_eq!(tally.limited, 1);
        assert_fresh_single_surface(&harness, &target_url, "protected_stop", 1, 0);
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
