    /// A site with a reflecting endpoint and a login page, for the coverage rules.
    fn coverage_site(request: String) -> (u16, &'static str, String) {
        let line = request.lines().next().unwrap_or_default().to_string();
        if line.contains("/api/echo") {
            let query = line.split_whitespace().nth(1).unwrap_or("");
            let probe = query
                .split("probe=")
                .nth(1)
                .map(|tail| tail.split('&').next().unwrap_or(tail).to_string())
                .unwrap_or_default();
            return (
                200,
                "application/json",
                format!("{{\"echo\":\"{probe}\"}}"),
            );
        }
        if line.contains("/login") {
            return (
                200,
                "text/html",
                "<html><form action=/login><input name=user></form></html>".to_string(),
            );
        }
        if line.contains("/api/orders") {
            return (
                200,
                "application/json",
                "{\"items\":[{\"id\":1}],\"total\":1}".to_string(),
            );
        }
        (404, "text/plain", "not found".to_string())
    }

    /// §13.8: each coverage family needs evidence strong enough for the claim. An
    /// anonymous call, a login page or an unrecovered input proves less than it
    /// looks like it proves.
    #[test]
    fn coverage_requires_evidence_strong_enough_for_the_claim() {
        let harness = agent_harness(
            "coverage-rules",
            coverage_site,
            vec![AgentIdentity::anonymous(), AgentIdentity::scoped("session-a")],
        );
        seed_session(&harness.db_path, "session-a", "cookie-alpha", "Bearer alpha");
        let base = harness.context.target_url.clone();
        let mut runtime = AgentToolRuntime::default();
        let replay = |runtime: &mut AgentToolRuntime, identity: &str, path: &str, family: &str| {
            agent_execute_tool(
                &harness.context,
                runtime,
                "replay_http",
                &serde_json::json!({"identity":identity,"method":"GET","url":format!("{base}{path}"),"family":family}),
            )
        };

        // One anonymous request is work on authorization, never coverage.
        replay(&mut runtime, "anonymous", "/api/orders", "authorization");
        assert_eq!(
            agent_family_sufficiency(&runtime, "authorization"),
            ("partial", "anonymous_only")
        );
        // Two independent replays, even on the same path, are not a validated
        // A/B pair and may not close authorization coverage.
        replay(&mut runtime, "session-a", "/api/orders", "authorization");
        assert_eq!(
            agent_family_sufficiency(&runtime, "authorization"),
            ("partial", "no_verified_identity_pair")
        );
        assert!(!runtime.has_coverage_evidence("authorization"));

        // A login page alone cannot prove a session works.
        let mut login_only = AgentToolRuntime::default();
        login_only.credit_coverage("authentication_session", "request", "", &[]);
        assert_eq!(
            agent_family_sufficiency(&login_only, "authentication_session"),
            ("partial", "login_page_only")
        );
        replay(&mut runtime, "anonymous", "/login", "authentication_session");
        assert_eq!(
            agent_family_sufficiency(&runtime, "authentication_session"),
            ("partial", "login_page_only"),
            "an anonymous fetch of the login page is not session coverage"
        );
        replay(&mut runtime, "session-a", "/api/orders", "authentication_session");
        assert_eq!(
            agent_family_sufficiency(&runtime, "authentication_session"),
            ("covered", "authenticated_response_observed")
        );

        // XSS needs the input to come back.
        let mut silent = AgentToolRuntime::default();
        assert_eq!(
            agent_family_sufficiency(&silent, "input_reflection_xss"),
            ("partial", "no_reflection_point")
        );
        silent.requests.push(executed_request("input_reflection_xss"));
        assert_eq!(
            agent_family_sufficiency(&silent, "input_reflection_xss"),
            ("partial", "no_reflection_point")
        );
        replay(&mut runtime, "anonymous", "/api/echo?probe=xssprobe42", "input_reflection_xss");
        assert_eq!(
            agent_family_sufficiency(&runtime, "input_reflection_xss"),
            ("covered", "reflection_point_observed"),
            "the reflected parameter value is the input→response chain"
        );

        // A discovery round that found nothing new closes that round only.
        let rounds = AgentToolRuntime {
            discovery_rounds: 1,
            ..AgentToolRuntime::default()
        };
        assert_eq!(
            agent_family_sufficiency(&rounds, "hidden_interface_discovery"),
            ("partial", "discovery_round_closed")
        );
    }

    /// §9.4/§9.5: `finish_target` may only cite evidence ids the runtime issued,
    /// and the three gap states are shown apart from each other.
    #[test]
    fn finish_target_accepts_only_evidence_the_runtime_issued() {
        let harness = agent_harness(
            "coverage-claims",
            coverage_site,
            vec![AgentIdentity::anonymous()],
        );
        let base = harness.context.target_url.clone();
        let mut runtime = AgentToolRuntime::default();
        agent_execute_tool(
            &harness.context,
            &mut runtime,
            "replay_http",
            &serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{base}/api/orders"),"family":"business_flow"}),
        );
        let issued: Vec<String> = runtime
            .coverage
            .iter()
            .filter(|entry| entry.family == "business_flow")
            .map(|entry| entry.id.clone())
            .collect();
        assert!(!issued.is_empty(), "the executed request must leave evidence");
        assert!(
            runtime
                .coverage
                .iter()
                .any(|entry| entry.tool_invocation_ids.iter().any(|id| id.starts_with("inv-1-"))),
            "a coverage entry must cite the tool invocation that produced it: {:?}",
            runtime.coverage
        );

        let closed = agent_execute_tool(
            &harness.context,
            &mut runtime,
            "finish_target",
            &serde_json::json!({
                "coverage": [
                    {"family":"business_flow","status":"covered","reason":"已执行下单接口","evidenceIds":[issued[0].clone()]},
                    {"family":"authorization","status":"covered","reason":"自称已覆盖","evidenceIds":["cov-9999"]},
                    {"family":"error_handling","status":"not_applicable","reason":"该目标不返回详细错误"},
                ],
                "stopReason": "覆盖队列已收口"
            }),
        );
        let view = &closed.model_view;
        assert_eq!(view["accepted"], serde_json::json!(true));
        assert_eq!(
            view["coveredFamilies"]
                .as_array()
                .map(|rows| rows.iter().filter_map(JsonValue::as_str).collect::<Vec<_>>())
                .unwrap_or_default(),
            vec!["business_flow"],
            "only the family with real evidence is covered: {view}"
        );
        let unsupported = view["unsupportedCoverageClaims"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert!(
            unsupported.iter().any(|row| {
                row["family"] == "authorization"
                    && (row["problem"] == "unknown_evidence_id"
                        || row["problem"] == "covered_without_evidence")
            }),
            "an invented citation must be refused: {unsupported:?}"
        );
        assert!(
            view["notCoveredFamilies"]
                .as_array()
                .map(|rows| rows.iter().any(|row| row == "authorization"))
                .unwrap_or(false),
            "an over-claimed family with no executed request stays not covered: {view}"
        );
        assert!(
            view["notApplicableFamilies"]
                .as_array()
                .map(|rows| rows.iter().any(|row| row == "error_handling"))
                .unwrap_or(false),
            "an honest not-applicable declaration stays its own state: {view}"
        );
        assert!(
            view["notCoveredFamilies"]
                .as_array()
                .map(|rows| !rows.is_empty())
                .unwrap_or(false),
            "families never worked at all are shown as not covered: {view}"
        );
        let text = view.to_string();
        assert!(
            !text.contains("未发现风险") && !text.contains("无风险"),
            "a gap must never be phrased as an absence of risk: {text}"
        );
        for row in view["coverage"].as_array().cloned().unwrap_or_default() {
            if row["status"] == "covered" {
                assert!(
                    row["evidence"]
                        .as_array()
                        .map(|rows| rows.iter().any(|entry| {
                            !entry["requestRecordIds"]
                                .as_array()
                                .map(|ids| ids.is_empty())
                                .unwrap_or(true)
                        }))
                        .unwrap_or(false),
                    "a covered row must carry request-backed evidence: {row}"
                );
            }
        }
    }

    /// Unexecuted coverage claims cannot close the ledger.
    #[test]
    fn finish_target_rejects_coverage_without_evidence() {
        let (_root, db_path) = temp_database("agent-claims");
        seed_scan(&db_path, "agent-scan", "scanning");
        let context = test_context(
            &db_path,
            "https://app.example.invalid",
            vec![AgentIdentity::anonymous()],
        );
        let mut runtime = AgentToolRuntime::default();
        let result = agent_execute_tool(
            &context,
            &mut runtime,
            "finish_target",
            &serde_json::json!({
                "coverage": [
                    {"family":"authorization","status":"covered","reason":"未做过"},
                    {"family":"business_flow","status":"not_applicable","reason":"无业务流程"}
                ],
                "stopReason":"收口"
            }),
        ).model_view;
        assert_eq!(
            result
                .get("unsupportedCoverageClaims")
                .and_then(JsonValue::as_array)
                .map(|rows| rows.len()),
            Some(1)
        );
        assert!(result.get("coveredFamilies").map(|v| v.is_null()).unwrap_or(false)
            || value_first(&result, &["coveredFamilies"]).is_empty());
        assert!(runtime.families.is_empty(), "a claim alone is not coverage");
        assert!(runtime.not_applicable.contains("business_flow"));
    }

    #[test]
    fn premature_finish_target_is_rejected_while_budget_remains() {
        let (_root, db_path) = temp_database("agent-premature-finish");
        seed_scan(&db_path, "agent-scan", "scanning");
        let context = test_context(
            &db_path,
            "https://app.example.invalid",
            vec![AgentIdentity::anonymous()],
        );
        let mut runtime = AgentToolRuntime::default();
        // Schema only allows covered/partial/not_applicable on the claim row;
        // omitting families (or claiming nothing for them) makes Rust derive
        // not_covered when the ledger has no evidence.
        let accepted = agent_execute_tool(
            &context,
            &mut runtime,
            "finish_target",
            &serde_json::json!({
                "coverage": [
                    {"family":"business_flow","status":"not_applicable","reason":"无业务流程"}
                ],
                "stopReason":"想收口"
            }),
        )
        .model_view;
        assert_eq!(
            accepted.get("accepted").and_then(JsonValue::as_bool),
            Some(true),
            "view={accepted}"
        );
        let never_tried = accepted
            .get("notCoveredFamilies")
            .and_then(JsonValue::as_array)
            .map(|rows| rows.len())
            .unwrap_or(0);
        assert!(never_tried > 0, "fixture must leave never-tried families: {accepted}");
        assert!(runtime.finished.is_some());

        let plan = context.execution_plan.clone();
        let early = state(3, 1_000, 0);
        let rejected = agent_reject_premature_finish(
            &context,
            &plan,
            &early,
            &mut runtime,
            &["family:authorization".to_string()],
            &accepted,
        )
        .expect("must reject while budget and turns remain");
        assert_eq!(rejected.get("accepted").and_then(JsonValue::as_bool), Some(false));
        assert_eq!(
            rejected.get("code").and_then(JsonValue::as_str),
            Some("premature_finish_target")
        );
        assert!(runtime.finished.is_none());

        // Calling finish again does not unlock an early stop. Hard ceilings do.
        runtime.finished = Some(accepted.clone());
        assert!(agent_reject_premature_finish(
            &context,
            &plan,
            &early,
            &mut runtime,
            &["family:authorization".to_string()],
            &accepted,
        )
        .is_some());

        runtime.finished = Some(accepted.clone());
        let near_cap = state(plan.max_turns.max(1) - 1, 1_000, 0);
        assert!(
            agent_reject_premature_finish(
                &context,
                &plan,
                &near_cap,
                &mut runtime,
                &["family:authorization".to_string()],
                &accepted,
            )
            .is_none()
        );
    }

    #[test]
    fn version_disclosure_headers_become_observation_findings() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("server", "Microsoft-IIS/10.0".parse().unwrap());
        headers.insert("x-powered-by", "ASP.NET".parse().unwrap());
        headers.insert("x-aspnet-version", "4.0.30319".parse().unwrap());
        let signals = agent_security_relevant_headers(&headers);
        assert!(
            signals
                .iter()
                .any(|(n, v)| n == "x-aspnet-version" && v.contains("4.0")),
            "{signals:?}"
        );
        assert!(agent_header_looks_versioned("server", "Microsoft-IIS/10.0"));
        assert!(agent_header_looks_versioned("x-powered-by", "ASP.NET"));

        let (_root, db_path) = temp_database("agent-observe-headers");
        seed_scan(&db_path, "agent-scan", "scanning");
        let context = test_context(
            &db_path,
            "https://app.example.invalid",
            vec![AgentIdentity::anonymous()],
        );
        let mut runtime = AgentToolRuntime::default();
        runtime.record_request(executed_request("authorization"));
        let request_id = runtime.requests.last().unwrap().id.clone();
        let wrote = persist_agent_observation_finding(
            &context,
            &mut runtime,
            &request_id,
            "https://app.example.invalid/api",
            "x-aspnet-version",
            "4.0.30319",
        )
        .expect("persist");
        assert!(wrote);
        assert_eq!(runtime.confirmed_findings, 1);
        assert!(!runtime.observation_finding_keys.is_empty());
        assert!(runtime.families.contains("information_disclosure"));
        let findings = findings_for(&db_path, AGENT_VULNERABILITY_STAGE);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].0, "vulnerability");
        assert!(
            findings[0].1.contains("x-aspnet-version"),
            "{}",
            findings[0].1
        );
        let again = persist_agent_observation_finding(
            &context,
            &mut runtime,
            &request_id,
            "https://app.example.invalid/api",
            "x-aspnet-version",
            "4.0.30319",
        )
        .expect("dedupe");
        assert!(!again);
        assert_eq!(runtime.confirmed_findings, 1);
    }

    #[test]
    fn outdated_jquery_and_stack_helpers() {
        assert_eq!(
            agent_outdated_jquery_from_url("https://x/JS/jquery-1.7.1.js")
                .map(|(v, _)| v),
            Some("1.7.1".into())
        );
        assert_eq!(
            agent_outdated_jquery_from_url("https://x/ModalArea/js/jquery-1.11.3.min.js")
                .map(|(v, _)| v),
            Some("1.11.3".into())
        );
        assert!(agent_outdated_jquery_from_url("https://x/jquery-3.6.0.min.js").is_none());
        assert!(agent_detect_stack_trace_leak("Hello").is_none());
        assert!(agent_detect_stack_trace_leak(
            "Server Error in '/' Application.\r\nStack Trace:\r\nat System.Web.Http"
        )
        .is_some());
    }

    #[test]
    fn client_md5_and_script_src_helpers() {
        let login_html = r##"
            <script src="/JS/jquery-1.7.1.js"></script>
            <script src="/JS/jquery.md5.js"></script>
            <input id="PassWord" name="PassWord" type="password" />
            <script>
            $("#PassWord").val($.md5($("#PassWord").val()));
            </script>
        "##;
        assert!(agent_detect_client_md5_password(login_html).is_some());
        assert!(agent_detect_client_md5_password("<html>no secrets</html>").is_none());
        let srcs = agent_extract_script_srcs(login_html);
        assert!(srcs.iter().any(|s| s.contains("jquery-1.7.1")), "{srcs:?}");
        let abs = agent_resolve_against("https://www.jschxx.com/JschxxMain/LoginUser", "/JS/jquery-1.7.1.js")
            .expect("resolve");
        assert_eq!(
            agent_outdated_jquery_from_url(&abs).map(|(v, _)| v),
            Some("1.7.1".into())
        );
    }

    #[test]
    fn account_enum_helpers_ok_vs_no() {
        assert!(agent_looks_like_account_check(
            "https://www.jschxx.com/JschxxMain/checkAccount?Account=admin"
        ));
        assert!(!agent_looks_like_account_check("https://www.jschxx.com/"));
        assert_eq!(agent_membership_token(r#"{"R":"OK"}"#).as_deref(), Some("OK"));
        assert_eq!(agent_membership_token(r#"{"R":"NO"}"#).as_deref(), Some("NO"));
        let probe = agent_account_probe_url(
            "https://www.jschxx.com/JschxxMain/checkAccount?Account=admin",
        )
        .expect("probe");
        assert!(probe.contains("oviraptor_no_such_user_7f2a"), "{probe}");
        assert!(!probe.contains("Account=admin"), "{probe}");
    }

    #[test]
    fn agent_harvest_surface_paths_finds_onclick_and_ajax() {
        let html = r##"
            <img onclick="turll('/JschxxMain/Index','0')">
            <script>
            url: "/JschxxMain/CheckLogin",
            url: "JschxxMain/getCommission",
            window.location = "/JschxxMain/LoginUser?param=5";
            </script>
        "##;
        let paths = agent_harvest_surface_paths(html, "https://www.jschxx.com/");
        let joined = paths
            .iter()
            .map(|(m, p)| format!("{m}|{p}"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(joined.contains("/JschxxMain/Index"), "{joined}");
        assert!(joined.contains("/JschxxMain/CheckLogin"), "{joined}");
        assert!(joined.contains("LoginUser") || joined.contains("/JschxxMain/LoginUser"), "{joined}");
    }
