    /// Case 1 — an interactive page without captured APIs still owes the
    /// baseline coverage families instead of stopping at recon_only.
    #[test]
    fn standard_route_without_apis_still_queues_baseline_families() {
        let (_root, db_path) = temp_database("agent-queue");
        seed_scan(&db_path, "agent-scan", "scanning");
        let mut context = test_context(
            &db_path,
            "https://app.example.invalid",
            vec![AgentIdentity::anonymous()],
        );
        context.evidence = serde_json::json!({
            "apiCandidates": [],
            "investigation": {"actions": [{"key": "open-orders", "url": "https://app.example.invalid/orders"}]}
        });
        let queue = agent_initial_queue(&context);
        assert!(queue
            .iter()
            .any(|item| item.starts_with("family:authorization")));
        assert!(queue
            .iter()
            .any(|item| item.starts_with("family:business_flow")));
    }

    /// Case 2 — past the soft token budget but still producing new endpoints.
    #[test]
    fn soft_budget_overrun_continues_while_evidence_advances() {
        let (_root, db_path) = temp_database("agent-soft");
        seed_scan(&db_path, "agent-scan", "scanning");
        let context = test_context(
            &db_path,
            "https://app.example.invalid",
            vec![AgentIdentity::anonymous()],
        );
        let plan = context.execution_plan.clone();
        let runtime = AgentToolRuntime::default();
        let growing = state(4, plan.soft_uncached_tokens * 2, 0);
        assert!(agent_budget_stop(&context, &plan, &growing, &runtime).is_none());
    }

    /// Case 3 — a stalled run closes normally and never reports failure.
    #[test]
    fn no_progress_window_closes_without_failure() {
        let (_root, db_path) = temp_database("agent-nostall");
        seed_scan(&db_path, "agent-scan", "scanning");
        let context = test_context(
            &db_path,
            "https://app.example.invalid",
            vec![AgentIdentity::anonymous()],
        );
        let plan = context.execution_plan.clone();
        let mut runtime = AgentToolRuntime::default();
        runtime.requests.push(executed_request("authorization"));
        runtime.credit_family("authorization");
        let stalled = state(
            plan.soft_model_requests + 1,
            plan.soft_uncached_tokens + 1,
            plan.no_progress_window,
        );
        let (code, _) = agent_budget_stop(&context, &plan, &stalled, &runtime)
            .expect("a stalled run past the soft budget must close");
        assert!(
            matches!(code, AGENT_STOP_SOFT_TOKENS | AGENT_STOP_SOFT_REQUESTS),
            "stalled soft-budget exit must report a soft code, got {code}"
        );
    }

    /// Case 4 — hard ceilings stop even with progress.
    #[test]
    fn hard_limits_stop_regardless_of_progress() {
        let (_root, db_path) = temp_database("agent-hard");
        seed_scan(&db_path, "agent-scan", "scanning");
        let context = test_context(
            &db_path,
            "https://app.example.invalid",
            vec![AgentIdentity::anonymous()],
        );
        let plan = context.execution_plan.clone();
        let runtime = AgentToolRuntime::default();
        let over_tokens = state(4, plan.hard_total_tokens, 0);
        assert!(agent_budget_stop(&context, &plan, &over_tokens, &runtime).is_some());
        let over_requests = NativeAgentState {
            token_usage: AgentTokenUsage {
                model_requests: plan.hard_model_requests,
                ..AgentTokenUsage::default()
            },
            ..state(4, 1_000, 0)
        };
        assert!(agent_budget_stop(&context, &plan, &over_requests, &runtime).is_some());
    }

    #[test]
    fn text_only_reply_preserves_the_actual_stop_boundary() {
        let (_root, db_path) = temp_database("agent-text-stop");
        seed_scan(&db_path, "agent-scan", "scanning");
        let context = test_context(
            &db_path,
            "https://app.example.invalid",
            vec![AgentIdentity::anonymous()],
        );
        let plan = context.execution_plan.clone();
        let runtime = AgentToolRuntime::default();

        let mut tokens = state(1, plan.hard_total_tokens, 1);
        let (code, reason) = agent_text_only_stop(&context, &plan, &mut tokens, &runtime)
            .expect("hard token boundary after a text-only model response");
        assert_eq!(code, AGENT_STOP_HARD_TOKENS);
        assert!(reason.contains("Token"));
        assert_eq!(tokens.stall_checks, 0, "hard boundary must not mutate stall state");

        let mut requests = state(plan.hard_model_requests, 1, 1);
        let (code, _) = agent_text_only_stop(&context, &plan, &mut requests, &runtime)
            .expect("hard model request boundary after a text-only response");
        assert_eq!(code, AGENT_STOP_HARD_REQUESTS);
        assert_eq!(requests.stall_checks, 0);

        let mut max_turns = state(plan.max_turns.max(1), 1, 1);
        let (code, reason) = agent_text_only_stop(&context, &plan, &mut max_turns, &runtime)
            .expect("maximum-turn boundary after a text-only response");
        assert_eq!(code, AGENT_STOP_DERIVED);
        assert!(reason.contains("最大轮数"));
    }

    /// Case 5 — permission boundaries keep running; protection stops.
    #[test]
    fn ordinary_401_403_does_not_fuse_but_waf_and_429_do() {
        for reason in ["HTTP 401 未登录", "HTTP 403 无权访问"] {
            let stop = AgentStop::from_reason(format!("接口返回 {reason}，仅记录权限边界"));
            assert!(!stop.requires_fuse(), "boundary must not fuse: {reason}");
        }
        let waf = AgentStop::from_reason("目标返回明确的 WAF、验证码或机器人挑战信号（HTTP 403）");
        assert_eq!(waf.code, AGENT_STOP_WAF);
        assert!(waf.requires_fuse());
        let limited = AgentStop::from_reason("HTTP 429 Too Many Requests 持续限流");
        assert_eq!(limited.code, AGENT_STOP_RATE_LIMIT);
        assert!(limited.requires_fuse());
    }

    /// Case 6 — A/B identities load their own session material only.
    #[test]
    fn identities_keep_cookies_and_authorization_separate() {
        let (_root, db_path) = temp_database("agent-identity");
        seed_scan(&db_path, "agent-scan", "scanning");
        seed_session(&db_path, "session-a", "cookie-alpha", "Bearer alpha");
        seed_session(&db_path, "session-b", "cookie-beta", "Bearer beta");
        let identities = vec![
            AgentIdentity::scoped("session-a"),
            AgentIdentity::scoped("session-b"),
        ];
        let text = |rows: &[(String, String)]| {
            rows.iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join(";")
        };
        let left = agent_identity_headers(
            &test_context(&db_path, "https://app.example.invalid", identities.clone()),
            &identities[0],
        )
        .unwrap();
        let right = agent_identity_headers(
            &test_context(&db_path, "https://app.example.invalid", identities.clone()),
            &identities[1],
        )
        .unwrap();
        assert_eq!(
            text(&left),
            "cookie=session=cookie-alpha;authorization=Bearer alpha"
        );
        assert_eq!(
            text(&right),
            "cookie=session=cookie-beta;authorization=Bearer beta"
        );
        assert!(!text(&left).contains("beta") && !text(&right).contains("alpha"));
        assert!(agent_identity_headers(
            &test_context(&db_path, "https://app.example.invalid", identities.clone()),
            &AgentIdentity::anonymous(),
        )
        .unwrap()
        .is_empty());
    }

    /// Case 7 — anonymous and single-account tasks never render 账号 A/B.
    #[test]
    fn single_or_anonymous_task_has_no_account_pair_labels() {
        let anonymous = vec![AgentIdentity::anonymous()];
        assert_eq!(agent_identity_label(&anonymous, "anonymous"), "匿名会话");
        let single = vec![AgentIdentity::scoped("session-a")];
        assert_eq!(agent_identity_label(&single, "session-a"), "当前身份");
        let anonymous_and_single = vec![
            AgentIdentity::anonymous(),
            AgentIdentity::scoped("session-a"),
        ];
        assert_eq!(agent_identity_label(&anonymous_and_single, "anonymous"), "匿名会话");
        assert_eq!(agent_identity_label(&anonymous_and_single, "session-a"), "当前身份");
        let pair = vec![
            AgentIdentity::scoped("session-a"),
            AgentIdentity::scoped("session-b"),
        ];
        assert_eq!(agent_identity_label(&pair, "session-b"), "身份 B（已认证）");
    }

    #[test]
    fn a_single_authenticated_identity_never_aliases_anonymous_or_unknown() {
        let (_root, db_path) = temp_database("agent-identity-handle");
        let context = test_context(
            &db_path,
            "https://app.example.invalid",
            vec![AgentIdentity::scoped("session-a")],
        );
        assert_eq!(agent_identity_of(&context, "session-a").unwrap().key, "session-a");
        assert!(agent_identity_of(&context, "anonymous").is_none());
        assert!(agent_identity_of(&context, "session-b").is_none());
        assert!(agent_identity_of(&context, "").is_none());
    }

    #[test]
    fn an_unbound_session_cannot_supply_headers_after_task_reassignment() {
        let (_root, db_path) = temp_database("agent-session-reassigned");
        seed_scan(&db_path, "agent-scan", "scanning");
        seed_session(&db_path, "session-a", "cookie-alpha", "Bearer alpha");
        let context = test_context(
            &db_path,
            "https://app.example.invalid",
            vec![AgentIdentity::scoped("session-a")],
        );
        let connection = db::open(&db_path).unwrap();
        connection.execute(
            "UPDATE browser_auth_sessions SET owner_scan_id='another-scan' WHERE id='session-a'",
            [],
        ).unwrap();
        assert!(agent_identity_headers(&context, &context.identities[0])
            .unwrap_err()
            .contains("解绑"));
    }

    /// Case 8 — telemetry, static assets, foreign hosts and UNKNOWN methods never
    /// reach the wire.
    #[test]
    fn telemetry_static_and_unknown_method_are_not_formal_apis() {
        let (_root, db_path) = temp_database("agent-noise");
        seed_scan(&db_path, "agent-scan", "scanning");
        let context = test_context(&db_path, "http://127.0.0.1:1", vec![AgentIdentity::anonymous()]);
        let mut runtime = AgentToolRuntime::default();
        let unknown = agent_http_request(
            &context,
            &mut runtime,
            "anonymous",
            "UNKNOWN",
            "http://127.0.0.1:1/api",
            Vec::new(),
            None,
            None,
            "",
            "",
            ScopeSource::HttpReplay,
            "test",
        );
        assert_eq!(
            value_first(unknown.as_ref().err().unwrap(), &["code"]),
            "unknown_method"
        );
        for url in [
            "https://o450.example.invalid/api/1/envelope",
            "https://sentry.example.invalid/api/17/envelope/",
            "https://cdn.example.invalid/fonts/x.woff2",
            "https://unrelated.example.org/api/orders",
        ] {
            let denied = agent_http_request(
                &context,
                &mut runtime,
                "anonymous",
                "GET",
                url,
                Vec::new(),
                None,
                None,
                "",
                "",
                ScopeSource::HttpReplay,
                "test",
            );
            assert!(denied.is_err(), "{url} must never be requested");
        }
        assert_eq!(runtime.target_requests, 0);
    }

    /// Case 9 — resume keeps completed contracts; a fresh run clears them.
    #[test]
    fn native_checkpoint_round_trips_and_clear_keeps_history_rows() {
        let (_root, db_path) = temp_database("agent-state");
        seed_scan(&db_path, "agent-scan", "scanning");
        let mut saved = state(6, 12_000, 1);
        saved.completed_contract_keys = vec!["idor|/api/orders".into()];
        saved.contract_outcomes = vec![("idor|/api/orders".into(), "rejected".into())];
        saved.covered_families = vec!["authorization".into()];
        saved.pending_queue = vec!["family:business_flow".into()];
        saved.persist(&db_path, "agent-scan", "https://app.example.invalid").unwrap();
        let read =
            NativeAgentState::read(&db_path, "agent-scan", "https://app.example.invalid").unwrap();
        assert_eq!(read.evidence_hash, saved.evidence_hash);
        assert_eq!(read.execution_plan_hash, saved.execution_plan_hash);
        assert_eq!(read.completed_contracts(), vec!["idor|/api/orders".to_string()]);
        NativeAgentState::clear(&db_path, "agent-scan", "https://app.example.invalid").unwrap();
        assert!(
            NativeAgentState::read(&db_path, "agent-scan", "https://app.example.invalid").is_none()
        );
        // A fresh attempt clears machine-generated state but never the frozen
        // plan row: that is what pins the backend of the attempt about to run.
        let context = test_context(
            &db_path,
            "https://app.example.invalid",
            vec![AgentIdentity::anonymous()],
        );
        persist_agent_execution_plan(
            &db_path,
            "agent-scan",
            context.attempt_number,
            &context.target_url,
            &context.execution_plan,
        ).unwrap();
        NativeAgentState::clear(&db_path, "agent-scan", &context.target_url).unwrap();
        assert!(read_agent_checkpoint(
            &db_path,
            "agent-scan",
            "https://app.example.invalid",
            "agent_execution_plan"
        )
        .is_object());
    }

    /// Case 10 plus Phase 2 §7.1: `auto` defaults a URL web task to Native. A
    /// existing attempt keeps its frozen non-executable marker. Obsolete policy
    /// settings are ignored, never interpreted as a backend migration request.
    #[test]
    fn auto_defaults_to_native_while_a_historical_attempt_stays_retired() {
        let (_root, db_path) = temp_database("agent-fallback");
        seed_scan(&db_path, "agent-scan", "scanning");
        let auto = serde_json::json!({"agentBackendPolicy": "auto"});
        let pinned = AgentExecutionPlan {
            backend: AgentBackendKind::LegacyRemoved,
            ..test_plan("standard")
        };
        seed_retired_attempt_plan(
            &db_path,
            "agent-scan",
            1,
            "https://app.example.invalid",
            &pinned,
        );
        assert_eq!(
            agent_select_backend(
                &db_path,
                "agent-scan",
                1,
                "https://app.example.invalid",
                &auto,
                true
            ),
            AgentBackendKind::LegacyRemoved,
            "the plan written for this attempt must pin the backend"
        );
        assert_eq!(
            agent_select_backend(
                &db_path,
                "other-scan",
                1,
                "https://app.example.invalid",
                &auto,
                true
            ),
            AgentBackendKind::Native,
            "§7.1: a URL web task with no pinned plan defaults to native under auto"
        );
        assert_eq!(
            agent_select_backend(
                &db_path,
                "other-scan",
                1,
                "https://app.example.invalid",
                &serde_json::json!({"agentBackendPolicy": "native"}),
                true
            ),
            AgentBackendKind::Native
        );
        assert_eq!(
            agent_select_backend(
                &db_path,
                "other-scan",
                1,
                "https://app.example.invalid",
                &serde_json::json!({"agentBackendPolicy": "native"}),
                false
            ),
            AgentBackendKind::Native,
            "backend selection never routes a new task to the removed backend"
        );
        assert_eq!(
            agent_select_backend(
                &db_path,
                "third-scan",
                1,
                "https://app.example.invalid",
                &serde_json::json!({"agentBackendPolicy": "strix"}),
                true
            ),
            AgentBackendKind::Native,
            "an eligible task does not start the retired strix backend"
        );
        // §12 Stage 4 lifted this: a greybox run with a repository freezes a snapshot and
        // keeps the web branch beside it, instead of handing the work to another engine.
        assert!(agent_native_eligible("greybox", "/src/app", &["https://a".into()]));
        assert!(agent_native_eligible("greybox", "", &["https://a".into()]));
        assert!(agent_native_eligible("code", "/src/app", &[]));
        assert!(agent_native_eligible("cicd", "/src/app", &[]));
        assert!(agent_native_eligible("web", "", &["https://a".into()]));
        assert!(!agent_native_eligible("code", "   ", &[]));
        assert!(!agent_native_eligible("other", "/src/app", &["https://a".into()]));
    }

    /// Case 11 — a slow local model is neither an auth nor a cloud timeout error.
    #[test]
    fn local_profile_has_no_cloud_timeout_or_key_gate() {
        let mut slow = test_environment("local");
        slow.api_key = String::new();
        let profile = agent_model_profile(&slow, None).unwrap();
        assert!(profile.local);
        assert!(!profile.has_key());
        assert!(profile.max_context_tokens > 0);
        let cloud = agent_model_profile(
            &test_environment("cloud"),
            Some("http://127.0.0.1:7890"),
        )
        .unwrap();
        assert!(!cloud.local);
        assert_eq!(cloud.max_context_tokens, 0);
        let timeout = AgentModelError::Timeout("读取响应超时".into());
        assert!(timeout.detail().contains("超时"));
        assert!(!timeout.detail().contains("认证"));
        assert!(timeout.retryable());
        assert!(!AgentModelError::Authentication("bad key".into()).retryable());
        let outcome = AgentTargetOutcome::incomplete(timeout.detail());
        assert_eq!(outcome.terminal_status(), "paused");
        assert!(!outcome.stop().unwrap().requires_fuse());
    }

    /// Case 12 — native results surface in evidence, vulnerability, coverage and
    /// token ledgers the existing UI already reads.
    #[test]
    fn native_results_land_in_existing_result_surfaces() {
        let (_root, db_path) = temp_database("agent-surface");
        seed_scan(&db_path, "agent-scan", "scanning");
        let context = test_context(
            &db_path,
            "https://app.example.invalid",
            vec![AgentIdentity::anonymous()],
        );
        persist_agent_evidence(
            &context,
            &serde_json::json!({"method":"GET","url":"https://app.example.invalid/api/orders","status":200,"identity":"anonymous"}),
            "replay_http",
        ).unwrap();
        persist_agent_vulnerability(
            &context,
            &serde_json::json!({
                "status":"confirmed","title":"越权读取他人订单","severity":"high","cwe":"CWE-639","cvss":"7.5",
                "confidence":0.9,"confidenceRationale":"两次请求仅身份不同",
                "controlRequest":"GET /api/orders/1","testRequest":"GET /api/orders/2",
                "responseDifference":"返回了另一用户的订单","impact":"越权读取",
                "reproductionSteps":"替换 id 参数","counterEvidenceCheck":"排除缓存",
                "severityChangeConditions":"批量枚举则升高","remediation":"服务端归属校验",
                "fixVerification":"重放测试请求应 403"
            }),
            "idor-orders",
            &ConfirmedPair {
                control_request_id: "req-0001".into(),
                test_request_id: "req-0002".into(),
                difference_artifact_id: "diff-0001.json".into(),
                control_summary: "GET /api/orders [req-0001] 身份 session-a 状态 200".into(),
                test_summary: "GET /api/orders [req-0002] 身份 anonymous 状态 403".into(),
            },
        )
        .unwrap();
        let mut runtime = AgentToolRuntime::default();
        runtime.requests.push(executed_request("authorization"));
        runtime.credit_family("authorization");
        runtime.confirmed_findings = 1;
        let usage = state(4, 20_000, 0);
        persist_agent_coverage(&context, &runtime, &usage, "覆盖 1 个族").unwrap();
        persist_agent_usage(&db_path, "agent-scan", &usage.token_usage).unwrap();
        let connection = db::open(&db_path).unwrap();
        let mut statement = connection
            .prepare("SELECT stage,kind FROM sentinel_findings WHERE scan_id='agent-scan' ORDER BY stage")
            .unwrap();
        let stages = statement
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
            .unwrap()
            .flatten()
            .collect::<Vec<_>>();
        assert_eq!(
            stages,
            vec![
                (AGENT_VULNERABILITY_STAGE.to_string(), "vulnerability".to_string()),
                (AGENT_COVERAGE_STAGE.to_string(), "coverage".to_string()),
                (AGENT_EVIDENCE_STAGE.to_string(), "evidence".to_string()),
            ]
        );
        let raw: String = connection
            .query_row(
                "SELECT record_json FROM sentinel_findings WHERE stage='native-agent' AND kind='vulnerability'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let vulnerability = json(raw);
        for key in [
            "controlRequestId",
            "testRequestId",
            "responseDifferenceArtifactId",
            "controlRequest",
            "testRequest",
            "responseDifference",
            "impact",
            "counterEvidence",
            "severityChangeConditions",
            "recommendation",
            "fixVerification",
            "updateHistory",
            "cwe",
            "cvss",
        ] {
            assert!(
                vulnerability.get(key).is_some(),
                "vulnerability contract must carry {key}"
            );
        }
        let totals: (i64, i64) = connection
            .query_row(
                "SELECT total_tokens,llm_requests FROM sentinel_scans WHERE id='agent-scan'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(totals, (20_000, 4));
    }

    #[test]
    fn tool_schemas_are_strict_and_named_once() {
        let specs = agent_tool_specs();
        let names: Vec<&str> = specs.iter().map(|spec| spec.name).collect();
        // §12 Stage 4: the seven web tools plus exactly the ten source tools §10.2 item 6
        // allows, each named once.
        let mut expected: Vec<&str> = vec![
            "inspect_evidence",
            "replay_http",
            "compare_identities",
            "targeted_discovery",
            "browser_action",
            "record_hypothesis_result",
            "finish_target",
        ];
        expected.extend(crate::native_pipeline::tools::SOURCE_TOOLS.iter().copied());
        assert_eq!(names.len(), expected.len(), "{names:?}");
        for name in &expected {
            assert!(names.contains(name), "{name} 必须注册在工具表里：{names:?}");
        }
        assert_eq!(
            names.iter().filter(|name| agent_is_source_tool(name)).count(),
            10,
            "源码工具必须正好十条，不多不少：{names:?}"
        );
        assert_eq!(agent_source_tool_names().len(), 10);
        for spec in &specs {
            assert_eq!(
                spec.parameters.get("additionalProperties").and_then(JsonValue::as_bool),
                Some(false),
                "{} must reject extra parameters",
                spec.name
            );
        }
        let hash = crate::agent_runtime::model::usage::tool_schema_hash(&specs);
        assert_eq!(hash.len(), 64);
        assert_eq!(
            hash,
            crate::agent_runtime::model::usage::tool_schema_hash(&agent_tool_specs())
        );
        let missing = agent_validate_arguments(
            "replay_http",
            &serde_json::json!({"identity":"x","method":"GET"}),
        )
        .unwrap_err();
        assert!(missing.message.contains("url"), "{:?}", missing.message);
        assert_eq!(
            missing
                .errors
                .iter()
                .filter_map(|row| row.get("path"))
                .filter_map(JsonValue::as_str)
                .collect::<Vec<_>>()
                .join(","),
            "$.url",
            "the violation must name its JSON path"
        );
        assert!(agent_validate_arguments(
            "replay_http",
            &serde_json::json!({"identity":"x","method":"GET","url":"https://y","extra":1})
        )
        .is_err());
        assert_eq!(AgentBackendKind::Native.as_str(), "native");
        assert_eq!(
            AgentBackendKind::parse("NATIVE "),
            Some(AgentBackendKind::Native)
        );
    }

    #[test]
    fn write_requests_require_contract_cleanup_and_approval() {
        let (_root, db_path) = temp_database("agent-write-gate");
        seed_scan(&db_path, "agent-scan", "scanning");
        let mut context = test_context(
            &db_path,
            "https://app.example.invalid",
            vec![AgentIdentity::anonymous()],
        );
        context.evidence["opportunities"] =
            serde_json::json!([{"category": "idor", "endpoint": "/api/orders/1", "maxAttempts": 2}]);
        let mut runtime = AgentToolRuntime::default();
        let url = "https://app.example.invalid/api/orders/1";
        let missing = agent_write_gate(
            &context,
            &mut runtime,
            &serde_json::json!({"identity":"anonymous","method":"POST","url":url}),
            "POST",
            url,
        )
        .unwrap();
        assert_eq!(
            value_first(&missing, &["code"]),
            "mutation_contract_required"
        );
        let unknown_contract = agent_write_gate(
            &context,
            &mut runtime,
            &serde_json::json!({"contractKey":"idor|/api/other","attempt":1,"cleanup":"DELETE /api/orders/1","recoveryCondition":"恢复原值"}),
            "POST",
            url,
        )
        .unwrap();
        assert_eq!(
            value_first(&unknown_contract, &["code"]),
            "contract_unknown"
        );
        let unapproved = agent_write_gate(
            &context,
            &mut runtime,
            &serde_json::json!({"contractKey":"idor|/api/orders/1","attempt":1,"cleanup":"恢复订单归属","recoveryCondition":"重放控制请求应一致"}),
            "POST",
            url,
        )
        .unwrap();
        assert_eq!(
            value_first(&unapproved, &["code"]),
            "mutation_not_approved"
        );
    }

    #[test]
    fn discovery_words_must_be_evidence_derived() {
        let (_root, db_path) = temp_database("agent-words");
        seed_scan(&db_path, "agent-scan", "scanning");
        let context = test_context(&db_path, "http://127.0.0.1:1", vec![AgentIdentity::anonymous()]);
        let mut runtime = AgentToolRuntime::default();
        let rejected = agent_execute_tool(
            &context,
            &mut runtime,
            "targeted_discovery",
            &serde_json::json!({"identity":"anonymous","words":["admin_backup_secret"]}),
        ).model_view;
        assert_eq!(value_first(&rejected, &["status"]), "insufficient_evidence");
        assert_eq!(runtime.target_requests, 0);
        let planned = context.execution_plan.discovery_passes;
        for round in 1..=planned {
            // evidence-derived words, so the round really runs (port 1 refuses)
            let result = agent_execute_tool(
                &context,
                &mut runtime,
                "targeted_discovery",
                &serde_json::json!({"identity":"anonymous","words":["orders","api"]}),
            ).model_view;
            assert_eq!(
                result.get("round").and_then(JsonValue::as_i64),
                Some(round),
                "{result}"
            );
        }
        let over = agent_execute_tool(
            &context,
            &mut runtime,
            "targeted_discovery",
            &serde_json::json!({"identity":"anonymous","words":["orders"]}),
        ).model_view;
        assert_eq!(value_first(&over, &["code"]), "discovery_budget_exhausted");
    }

    /// §10: the historical repair must never downgrade a target whose decision
    /// allowed progressive baseline investigation.
    #[test]
    fn history_repair_keeps_baseline_allowed_targets_partially_open() {
        let (root, db_path) = temp_database("agent-baseline-repair");
        {
            let connection = db::open(&db_path).unwrap();
            connection
                .execute("INSERT INTO projects(id,name) VALUES(9101,'Repair')", [])
                .unwrap();
            for (url, baseline) in [
                ("https://baseline.invalid", true),
                ("https://closed.invalid", false),
            ] {
                connection.execute(
                    "INSERT INTO sentinel_scans(id,project_id,project_name,status,current_checkpoint,scan_type) VALUES(?1,9101,'Repair','partial','旧汇总','web') ON CONFLICT(id) DO NOTHING",
                    ["repair-scan"],
                ).unwrap();
                connection
                    .execute(
                        "INSERT INTO sentinel_targets(project_id,scan_id,company,url,status,routing_reason,scan_mode) VALUES(9101,'repair-scan','Repair',?1,'partial','首轮 no_high_value_hypothesis 收口','standard')",
                        [url],
                    )
                    .unwrap();
                connection.execute(
                    "INSERT INTO investigation_metrics(scan_id,target_url,token_worthy,stop_reason,decision_json) VALUES('repair-scan',?1,0,'progressive_baseline',?2)",
                    params![url, serde_json::json!({
                        "eligibleForModel": false,
                        "standardInvestigationAllowed": false,
                        "baselineInvestigationAllowed": baseline,
                    }).to_string()],
                )
                .unwrap();
            }
        }
        db::initialize(&root).unwrap();
        let connection = db::open(&db_path).unwrap();
        let status = |url: &str| -> String {
            connection
                .query_row(
                    "SELECT status FROM sentinel_targets WHERE url=?1",
                    [url],
                    |row| row.get(0),
                )
                .unwrap()
        };
        assert_eq!(status("https://baseline.invalid"), "partial");
        assert_eq!(status("https://closed.invalid"), "recon_only");
    }

    // ------------------------------------------------------------------
    // End-to-end: a real model tool loop against local mocks. No Strix
    // process and no real site is involved.
    // ------------------------------------------------------------------

    pub(super) type Seen = std::sync::Arc<Mutex<Vec<String>>>;
