    /// The attempt row the app writes for "继续未完成阶段" (resume) or
    /// "重新执行" (fresh). Lineage is read from this row, never from UI wording.
    fn seed_attempt_row(db_path: &Path, attempt_number: i64, execution_mode: &str) {
        let connection = db::open(db_path).unwrap();
        connection
            .execute(
                "INSERT OR REPLACE INTO sentinel_scan_attempts(scan_id,attempt_number,execution_mode) VALUES('agent-scan',?1,?2)",
                params![attempt_number, execution_mode],
            )
            .unwrap();
    }

    fn frozen_plan_for(
        db_path: &Path,
        context: &AgentRunContext,
        attempt_number: i64,
    ) -> AgentExecutionPlan {
        let adaptive = AdaptiveStrixSettings::from_json(&serde_json::json!({}));
        agent_frozen_plan(
            db_path,
            "agent-scan",
            &context.route.url,
            &context.route,
            &context.environment,
            &adaptive,
            AgentBackendKind::Native,
            attempt_number,
        )
        .expect("a plan must resolve for this attempt")
    }

    fn set_scan_status(db_path: &Path, status: &str) {
        let connection = db::open(db_path).unwrap();
        connection
            .execute(
                "UPDATE sentinel_scans SET status=?1 WHERE id='agent-scan'",
                params![status],
            )
            .unwrap();
    }

    /// Regression for the queue bug: a call that carries no path, url, actionKey
    /// or family must not consume anything, and a full URL must still match the
    /// queued `/path`.
    #[test]
    fn queue_entries_are_consumed_only_by_matching_work() {
        let (_root, db_path) = temp_database("agent-queue");
        seed_scan(&db_path, "agent-scan", "scanning");
        let context = test_context(
            &db_path,
            "https://app.example.invalid",
            vec![AgentIdentity::anonymous()],
        );
        let queue = agent_initial_queue(&context);
        assert!(!queue.is_empty());
        let runtime = AgentToolRuntime::default();
        assert!(
            queue
                .iter()
                .filter(|key| runtime.queue_key_satisfied(&context, key))
                .count()
                == 0,
            "an untouched runtime must consume nothing"
        );

        let mut runtime = AgentToolRuntime::default();
        runtime
            .requests
            .push(trace_of("GET", "/api/orders", "authorization", ""));
        assert!(runtime.queue_key_satisfied(&context, "api:GET|/api/orders"));
        assert!(runtime.queue_key_satisfied(&context, "family:authorization"));
        assert!(
            !runtime.queue_key_satisfied(&context, "family:business_flow"),
            "an unrelated family must stay queued"
        );
        assert!(!runtime.queue_key_satisfied(&context, "api:POST|/api/orders"));
    }

    /// A contract is closed by a verdict or by spending its own attempt cap —
    /// never by a tool call that was rejected before it reached the target.
    #[test]
    fn refused_write_never_spends_or_closes_a_contract() {
        let (_root, db_path) = temp_database("agent-refused");
        seed_scan(&db_path, "agent-scan", "scanning");
        let mut context = test_context(
            &db_path,
            "https://app.example.invalid",
            vec![AgentIdentity::anonymous()],
        );
        context.evidence["opportunities"] = serde_json::json!([
            {"category": "idor", "endpoint": "/api/orders", "maxAttempts": 2}
        ]);
        let mut runtime = AgentToolRuntime::default();
        let result = agent_execute_tool(
            &context,
            &mut runtime,
            "replay_http",
            &serde_json::json!({
                "identity":"anonymous","method":"POST","url":"https://app.example.invalid/api/orders",
                "contractKey":"idor|/api/orders","attempt":1,
                "cleanup":"删除测试订单","recoveryCondition":"订单不再存在"
            }),
        ).model_view;
        assert_eq!(value_first(&result, &["code"]), "mutation_not_approved");
        assert_eq!(runtime.target_requests, 0, "a refused call costs no HTTP budget");
        assert!(runtime.contract_attempts.is_empty());
        assert!(runtime.contract_outcomes.is_empty());

        let mut state = NativeAgentState::fresh(1, AgentBackendKind::Native, "e", "p", Vec::new());
        native_sync(
            &context,
            &mut state,
            &runtime,
            &["contract:idor|/api/orders".to_string()],
        );
        assert!(state.completed_contract_keys.is_empty());
        assert!(state.exhausted_contract_keys.is_empty());
        assert_eq!(state.pending_queue.len(), 1, "the contract must still be owed");
    }

    /// A verdict cannot close a contract the run never executed.
    #[test]
    fn verdict_requires_executed_contract() {
        let (_root, db_path) = temp_database("agent-verdict");
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
            "record_hypothesis_result",
            &serde_json::json!({
                "hypothesisKey":"h-1","status":"rejected","contractKey":"idor|/api/orders"
            }),
        ).model_view;
        assert_eq!(value_first(&result, &["code"]), "contract_not_executed");
        assert!(runtime.contract_outcomes.is_empty());

        runtime
            .requests
            .push(trace_of("GET", "/api/orders", "authorization", "idor|/api/orders"));
        runtime.contract_attempts.insert("idor|/api/orders".into(), 1);
        let result = agent_execute_tool(
            &context,
            &mut runtime,
            "record_hypothesis_result",
            &serde_json::json!({
                "hypothesisKey":"h-1","status":"rejected","contractKey":"idor|/api/orders"
            }),
        ).model_view;
        assert_eq!(value_first(&result, &["status"]), "rejected");
        assert_eq!(
            runtime.contract_outcomes.get("idor|/api/orders").map(String::as_str),
            Some("rejected")
        );
    }

    /// §13.9: a confirmed finding is its two executed requests plus the response
    /// difference record between them. Free text, wrong ids, a missing contrast or
    /// an unrelated artifact all downgrade it to insufficient evidence.
    #[test]
    fn confirmed_finding_must_bind_two_executed_requests() {
        let harness = agent_harness(
            "confirm-binding",
            |request: String| {
                let cookie = request
                    .lines()
                    .find(|row| row.to_ascii_lowercase().starts_with("cookie:"))
                    .unwrap_or_default()
                    .to_string();
                let body = if cookie.contains("cookie-alpha") {
                    r#"{"userId":"u-7","roleId":"r-admin","role":"admin","canWrite":true}"#
                } else {
                    r#"{"userId":"u-8","roleId":"r-member","role":"member","canWrite":false}"#
                };
                (200, "application/json", body.to_string())
            },
            vec![
                AgentIdentity::scoped("session-a"),
                AgentIdentity::scoped("session-b"),
            ],
        );
        seed_session(&harness.db_path, "session-a", "cookie-alpha", "Bearer alpha");
        seed_session(&harness.db_path, "session-b", "cookie-beta", "Bearer beta");
        let mut runtime = AgentToolRuntime::default();
        let pair = agent_execute_tool(
            &harness.context,
            &mut runtime,
            "compare_identities",
            &serde_json::json!({
                "leftIdentity":"session-a","rightIdentity":"session-b",
                "method":"GET","path":"/api/profile","family":"authorization","contractKey":"idor|/api/profile"
            }),
        )
        .model_view;
        assert_eq!(
            pair["materialDifference"],
            serde_json::json!(true),
            "roleId/role differ between the two accounts: {pair}"
        );
        let control = pair["left"]["requestId"].as_str().unwrap_or_default().to_string();
        let test = pair["right"]["requestId"].as_str().unwrap_or_default().to_string();
        let artifact = value_first(&pair, &["responseDifferenceArtifactId"]);
        assert!(!control.is_empty() && !test.is_empty() && !artifact.is_empty());

        let finding = |control: &str, test: &str, artifact: &str| {
            serde_json::json!({
                "hypothesisKey":"h-idor","status":"confirmed","family":"authorization",
                "contractKey":"idor|/api/profile",
                "title":"越权读取他人配置","severity":"high","confidence":0.9,
                "controlRequestId":control,"testRequestId":test,"responseDifferenceArtifactId":artifact,
                "impact":"跨账号读取","reproductionSteps":"1. 用 B 身份重放 2. 观察 A 的字段",
                "counterEvidenceCheck":"已排除缓存与登录跳转"
            })
        };
        let confirmed = agent_execute_tool(
            &harness.context,
            &mut runtime,
            "record_hypothesis_result",
            &finding(&control, &test, &artifact),
        )
        .model_view;
        assert_eq!(confirmed["status"].as_str(), Some("confirmed"), "{confirmed}");
        assert_eq!(runtime.confirmed_findings, 1);
        let stored = findings_for(&harness.db_path, AGENT_VULNERABILITY_STAGE);
        assert_eq!(stored.len(), 1);
        let record = json(stored[0].1.clone());
        assert_eq!(value_first(&record, &["controlRequestId"]), control);
        assert_eq!(value_first(&record, &["testRequestId"]), test);
        assert_eq!(
            value_first(&record, &["responseDifferenceArtifactId"]),
            artifact
        );

        // The old shape — three paragraphs of prose — is no longer accepted at all.
        let prose = agent_execute_tool(
            &harness.context,
            &mut runtime,
            "record_hypothesis_result",
            &serde_json::json!({
                "hypothesisKey":"h-prose","status":"confirmed","family":"authorization",
                "controlRequest":"GET /api/profile 身份 A","testRequest":"GET /api/profile 身份 B",
                "responseDifference":"B 看到了 A 的配置","impact":"跨账号","reproductionSteps":"重放"
            }),
        )
        .model_view;
        assert_eq!(prose["code"].as_str(), Some("invalid_arguments"), "{prose}");
        assert_eq!(runtime.confirmed_findings, 1, "the prose claim added nothing");

        for (label, control_id, test_id, artifact_id, expected) in [
            ("same request twice", control.clone(), control.clone(), artifact.clone(), "confirmed_requires_distinct_requests"),
            ("invented ids", "req-9999".to_string(), test.clone(), artifact.clone(), "confirmed_request_not_executed"),
            ("artifact from another pair", control.clone(), test.clone(), "diff-0009.json".to_string(), "difference_artifact_missing"),
        ] {
            let refused = agent_execute_tool(
                &harness.context,
                &mut runtime,
                "record_hypothesis_result",
                &finding(&control_id, &test_id, &artifact_id),
            )
            .model_view;
            assert_eq!(
                refused["status"].as_str(),
                Some("insufficient_evidence"),
                "{label} must downgrade: {refused}"
            );
            assert_eq!(
                refused["missingEvidence"][0].as_str(),
                Some(expected),
                "{label}: {refused}"
            );
            assert_eq!(refused["downgradedFrom"].as_str(), Some("confirmed"));
            assert_eq!(runtime.confirmed_findings, 1, "{label} must not count");
        }
        assert_eq!(
            findings_for(&harness.db_path, AGENT_VULNERABILITY_STAGE).len(),
            1,
            "only the bound finding was ever persisted"
        );
    }

    /// §10.2: a pair with no identity or parameter contrast, a cached reply or a
    /// login redirect cannot support a confirmation even when both ids are real.
    #[test]
    fn confirmed_finding_refuses_a_pair_without_a_contrast() {
        let harness = agent_harness(
            "confirm-contrast",
            coverage_site,
            vec![AgentIdentity::anonymous()],
        );
        let base = harness.context.target_url.clone();
        let mut runtime = AgentToolRuntime::default();
        let first = agent_execute_tool(
            &harness.context,
            &mut runtime,
            "replay_http",
            &serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{base}/api/orders"),"family":"authorization","contractKey":"idor|/api/orders"}),
        )
        .model_view;
        let second = agent_execute_tool(
            &harness.context,
            &mut runtime,
            "replay_http",
            &serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{base}/api/orders"),"family":"authorization","contractKey":"idor|/api/orders"}),
        )
        .model_view;
        let control = value_first(&first, &["requestId"]);
        let test = value_first(&second, &["requestId"]);
        assert!(!control.is_empty() && control != test, "{first} / {second}");
        // A difference record has to exist, so build one from the two responses.
        let record = agent_response_field_difference(&first, &second);
        let artifact = agent_write_diff_record(
            &harness.context,
            runtime.target_requests,
            &serde_json::json!({
                "left": {"requestId": control},
                "right": {"requestId": test},
                "materialDifference": agent_difference_is_material(&record, 200, 200),
                "fieldDifference": record,
            }),
        )
        .unwrap();
        let refused = agent_execute_tool(
            &harness.context,
            &mut runtime,
            "record_hypothesis_result",
            &serde_json::json!({
                "hypothesisKey":"h-flat","status":"confirmed","family":"authorization",
                "contractKey":"idor|/api/orders",
                "controlRequestId":control,"testRequestId":test,"responseDifferenceArtifactId":artifact,
                "impact":"读别人的单","reproductionSteps":"重放两次"
            }),
        )
        .model_view;
        assert_eq!(
            refused["status"].as_str(),
            Some("insufficient_evidence"),
            "two identical anonymous requests prove nothing: {refused}"
        );
        assert_eq!(runtime.confirmed_findings, 0);
    }


    /// Resume must not hand the run a second HTTP budget.
    #[test]
    fn restore_keeps_spent_budget_and_closed_contracts() {
        let (_root, db_path) = temp_database("agent-restore");
        seed_scan(&db_path, "agent-scan", "scanning");
        let context = test_context(
            &db_path,
            "https://app.example.invalid",
            vec![AgentIdentity::anonymous()],
        );
        let seeded = NativeAgentState {
            target_requests: 7,
            discovery_rounds: 2,
            observed_requests: vec![trace_of("GET", "/api/orders", "authorization", "idor|/api/orders")],
            contract_attempts: vec![("idor|/api/orders".to_string(), 1)],
            contract_outcomes: vec![("idor|/api/orders".to_string(), "rejected".to_string())],
            confirmed_findings: 3,
            verdict_keys: vec!["h-1:rejected".to_string()],
            ..state(4, 5_000, 1)
        };
        let runtime = AgentToolRuntime::restore(&seeded);
        assert_eq!(runtime.target_requests, 7);
        assert_eq!(runtime.discovery_rounds, 2);
        assert_eq!(runtime.confirmed_findings, 3);
        assert_eq!(runtime.attempts_for("idor|/api/orders"), 1);
        assert!(runtime.families.contains("authorization"));
        assert!(runtime.queue_key_satisfied(&context, "contract:idor|/api/orders"));
        assert!(runtime.queue_key_satisfied(&context, "api:GET|/api/orders"));
        assert!(runtime.touched("get", "/api/orders"));

        let mut round_trip = seeded.clone();
        round_trip.completed_contract_keys = round_trip.completed_contracts();
        let text = round_trip.as_json();
        let read = NativeAgentState::from_json(&text).expect("the ledger must round-trip");
        assert_eq!(read.target_requests, 7);
        assert_eq!(read.observed_requests, seeded.observed_requests);
        assert_eq!(read.completed_contract_keys, vec!["idor|/api/orders".to_string()]);
        assert_eq!(read.attempts_for("idor|/api/orders"), 1);
    }

    /// Rotating ids, timestamps and per-account objects are not a difference.
    #[test]
    fn identity_noise_is_not_a_material_difference() {
        let left = field_summary(
            r#"{"requestId":"a1","createdAt":"2026-09-19T00:00:00","token":"eyJhbGciOiJI.eyJzdWIiOiIx.SflKxwRJ","code":0,"items":[{"id":1,"role":"member"}]}"#,
        );
        let right = field_summary(
            r#"{"requestId":"b2","createdAt":"2026-09-19T00:00:07","token":"eyJhbGciOiJI.eyJzdWIiOiIy.XkL9m2Qz","code":0,"items":[{"id":9,"role":"member"}]}"#,
        );
        let diff = agent_response_field_difference(&left, &right);
        assert!(
            !agent_difference_is_material(&diff, 200, 200),
            "only volatile and ownership fields differ: {diff}"
        );
        assert!(diff
            .get("valueDiffers")
            .and_then(JsonValue::as_array)
            .map(|rows| rows.is_empty())
            .unwrap_or(false));
    }

    /// A field one identity cannot see, a business code split or a status split
    /// is a real difference.
    #[test]
    fn identity_privilege_change_is_a_material_difference() {
        let member = field_summary(r#"{"code":0,"role":"member"}"#);
        let admin = field_summary(r#"{"code":403,"role":"admin","permissions":["write"]}"#);
        let diff = agent_response_field_difference(&member, &admin);
        assert!(agent_difference_is_material(&diff, 200, 200), "{diff}");
        assert!(agent_difference_is_material(&JsonValue::Null, 200, 403));
        assert!(!agent_difference_is_material(&JsonValue::Null, 200, 200));
    }

    /// §13.7: a field whose name ends in `id` is not automatically ownership noise.
    /// `roleId`/`tenantId`/`departmentId`/`permissionId` are the authorization
    /// decision itself; `userId`/`accountId` stay in the ownership bucket that
    /// only confirms whose object each response is.
    #[test]
    fn authorization_fields_are_material_and_ownership_stays_separate() {
        let member = field_summary(
            r#"{"userId":"7","accountId":"a-7","roleId":"r-member","role":"member","departmentId":"d1","tenantId":"t1","permissionId":"p-read"}"#,
        );
        let admin = field_summary(
            r#"{"userId":"8","accountId":"a-8","roleId":"r-admin","role":"admin","departmentId":"d2","tenantId":"t1","permissionId":"p-write"}"#,
        );
        let diff = agent_response_field_difference(&member, &admin);
        let paths = |key: &str| -> Vec<String> {
            diff.get(key)
                .and_then(JsonValue::as_array)
                .cloned()
                .unwrap_or_default()
                .iter()
                .filter_map(|row| row.get("path").and_then(JsonValue::as_str))
                .map(str::to_string)
                .collect()
        };
        let authorization = paths("materialAuthorizationDiffs");
        for field in ["roleId", "role", "departmentId", "permissionId"] {
            assert!(
                authorization.iter().any(|path| path == field),
                "{field} must be a material authorization difference, got {authorization:?}"
            );
        }
        let ownership = paths("subjectDiffs");
        assert!(
            ownership.contains(&"userId".to_string())
                && ownership.contains(&"accountId".to_string()),
            "per-account ids stay visible as ownership, got {ownership:?}"
        );
        assert!(
            paths("valueDiffers").is_empty(),
            "no business field changed here: {diff}"
        );
        assert_eq!(
            diff.get("ownershipOnly").and_then(JsonValue::as_array),
            None,
            "the old catch-all bucket is gone: {diff}"
        );
        assert!(
            !authorization.contains(&"tenantId".to_string()),
            "an unchanged authorization field must not be reported as a diff"
        );
        assert!(agent_difference_is_material(&diff, 200, 200));
        // Ownership alone is never material.
        let only_owner_left = field_summary(r#"{"userId":"7","amount":"num:10"}"#);
        let only_owner_right = field_summary(r#"{"userId":"8","amount":"num:10"}"#);
        let owner_diff =
            agent_response_field_difference(&only_owner_left, &only_owner_right);
        assert!(
            !agent_difference_is_material(&owner_diff, 200, 200),
            "a different subject id is not an authorization difference: {owner_diff}"
        );
        // A field only one identity can see, when it carries permissions, is the
        // strongest form of the same signal.
        let no_perms = field_summary(r#"{"role":"member"}"#);
        let with_perms = field_summary(r#"{"role":"member","permissions":["write"]}"#);
        let presence = agent_response_field_difference(&no_perms, &with_perms);
        assert!(
            presence["materialAuthorizationDiffs"]
                .as_array()
                .map(|rows| rows.iter().any(|row| {
                    row["path"]
                        .as_str()
                        .unwrap_or_default()
                        .starts_with("permissions")
                        && row["left"] == "absent"
                }))
                .unwrap_or(false),
            "a permission field only one side can see must be material: {presence}"
        );
    }

    /// §8: an A/B claim needs two complete records from this chain. A side that
    /// was never collected is `insufficient_evidence`, not a difference.
    #[test]
    fn ab_comparison_refuses_an_incomplete_pair() {
        let harness = agent_harness("ab-pair", mock_site, vec![
            AgentIdentity::scoped("session-a"),
            AgentIdentity::scoped("session-b"),
        ]);
        seed_session(&harness.db_path, "session-a", "cookie-alpha", "Bearer alpha");
        seed_session(&harness.db_path, "session-b", "cookie-beta", "Bearer beta");
        let mut runtime = AgentToolRuntime::default();
        let missing = agent_execute_tool(
            &harness.context,
            &mut runtime,
            "compare_identities",
            &serde_json::json!({
                "leftIdentity":"session-a","rightIdentity":"ghost","method":"GET","path":"/api/orders","family":"authorization"
            }),
        );
        assert_eq!(
            missing.model_view["status"].as_str(),
            Some("insufficient_evidence"),
            "{}",
            missing.model_view
        );
        assert!(!runtime.families.contains("authorization"));
        assert_eq!(
            runtime.target_requests, 0,
            "an unresolvable identity must not spend either side's request"
        );

        let complete = agent_execute_tool(
            &harness.context,
            &mut runtime,
            "compare_identities",
            &serde_json::json!({
                "leftIdentity":"session-a","rightIdentity":"session-b","method":"GET","path":"/api/orders","family":"authorization"
            }),
        );
        assert_eq!(
            complete.model_view["code"].as_str(),
            None,
            "a real pair must compare: {}",
            complete.model_view
        );
        let left = &complete.model_view["left"];
        let right = &complete.model_view["right"];
        for (side, row) in [("left", left), ("right", right)] {
            assert_ne!(row["requestId"].as_str().unwrap_or_default(), "", "{side}");
            assert_ne!(row["artifactId"].as_str().unwrap_or_default(), "", "{side}");
            assert_ne!(row["structureHash"].as_str().unwrap_or_default(), "", "{side}");
            assert!(row["status"].as_i64().unwrap_or_default() > 0, "{side}");
        }
        assert_ne!(left["requestId"], right["requestId"]);
        assert_ne!(left["artifactId"], right["artifactId"]);
        assert_eq!(runtime.target_requests, 2, "the pair spent exactly two requests");
    }
