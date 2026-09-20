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
        // Adding an authenticated view of the same call makes it a contrast.
        replay(&mut runtime, "session-a", "/api/orders", "authorization");
        assert_eq!(
            agent_family_sufficiency(&runtime, "authorization"),
            ("covered", "identity_contrast_observed")
        );
        assert!(runtime.has_coverage_evidence("authorization"));

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
