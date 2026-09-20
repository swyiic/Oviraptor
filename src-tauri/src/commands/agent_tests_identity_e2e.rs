    /// Two accounts that see exactly the same shape and the same values.
    fn equal_role_site(request: String) -> (u16, &'static str, String) {
        let _ = &request;
        (
            200,
            "application/json",
            r#"{"userId":"self","roleId":"r-member","role":"member","canWrite":false}"#.to_string(),
        )
    }

    /// The same endpoint answering differently for the two accounts: one of them
    /// holds an authorization field the other does not.
    fn unequal_role_site(request: String) -> (u16, &'static str, String) {
        let cookie = request
            .lines()
            .find(|row| row.to_ascii_lowercase().starts_with("cookie:"))
            .unwrap_or_default()
            .to_string();
        let body = if cookie.contains("cookie-alpha") {
            r#"{"userId":"u-7","roleId":"r-admin","role":"admin","permissions":["write"],"tenantId":"t-1"}"#
        } else {
            r#"{"userId":"u-8","roleId":"r-member","role":"member","tenantId":"t-1"}"#
        };
        (200, "application/json", body.to_string())
    }

    /// §14.3: two accounts with equal rights produce a comparison and no finding.
    #[test]
    fn e2e_two_equal_accounts_close_without_a_finding() {
        let mut harness = agent_harness(
            "e2e-equal",
            equal_role_site,
            vec![
                AgentIdentity::scoped("session-a"),
                AgentIdentity::scoped("session-b"),
            ],
        );
        seed_session(&harness.db_path, "session-a", "cookie-alpha", "Bearer alpha");
        seed_session(&harness.db_path, "session-b", "cookie-beta", "Bearer beta");
        let url = harness.context.target_url.clone();
        retarget_model(
            &mut harness,
            vec![
                model_round(
                    &[(
                        "compare_identities",
                        serde_json::json!({"leftIdentity":"session-a","rightIdentity":"session-b","method":"GET","path":"/api/profile","family":"authorization","contractKey":"idor|/api/profile"}),
                    )],
                    900,
                ),
                model_round(
                    &[(
                        "record_hypothesis_result",
                        serde_json::json!({"hypothesisKey":"h-equal","status":"insufficient_evidence","family":"authorization","contractKey":"idor|/api/profile"}),
                    )],
                    1_100,
                ),
                model_round(
                    &[(
                        "finish_target",
                        serde_json::json!({"coverage": closing_ledger(&["authorization"]), "stopReason": "覆盖队列已收口"}),
                    )],
                    1_300,
                ),
            ],
        );
        let outcome = NativeAgentBackend.execute(&harness.context);
        assert!(
            !matches!(outcome, AgentTargetOutcome::Failed(_)),
            "an equal-rights pair is normal work, never a failure: {:?}",
            outcome.detail()
        );
        let status = match &outcome {
            AgentTargetOutcome::Completed(_) => "completed",
            AgentTargetOutcome::BoundedCompleted(_) => "completed_with_gaps",
            AgentTargetOutcome::Incomplete(_) => "paused",
            other => panic!("unexpected outcome: {:?}", other.detail()),
        };
        let mut tally = AgentPipelineTally::default();
        record_agent_target_outcome(
            &harness.db_path,
            "agent-scan",
            &harness.context.route,
            outcome,
            &mut tally,
        );
        assert_agent_surface(&harness, &url, status, 2, 0);
        let state = NativeAgentState::read(&harness.db_path, "agent-scan", &url).unwrap();
        assert!(
            !state.covered_families.contains(&"authorization".to_string())
                || state.coverage_evidence.iter().any(|entry| {
                    entry.family == "authorization"
                        && entry.evidence_kind == "identity_pair"
                        && entry.result == "partial"
                }),
            "no material difference must not read as a proven boundary: {:?}",
            state.coverage_evidence
        );
        assert_eq!(
            findings_for(&harness.db_path, AGENT_VULNERABILITY_STAGE).len(),
            0,
            "equal rights confirm nothing"
        );
        assert_eq!(
            harness.site_seen.lock().unwrap().len(),
            2,
            "both sides of the pair reached the target once"
        );
    }

    /// §14.4: two accounts with a real permission difference, confirmed through the
    /// bound pair the comparison produced.
    #[test]
    fn e2e_permission_difference_confirms_one_finding() {
        let mut harness = agent_harness(
            "e2e-diff",
            unequal_role_site,
            vec![
                AgentIdentity::scoped("session-a"),
                AgentIdentity::scoped("session-b"),
            ],
        );
        seed_session(&harness.db_path, "session-a", "cookie-alpha", "Bearer alpha");
        seed_session(&harness.db_path, "session-b", "cookie-beta", "Bearer beta");
        let url = harness.context.target_url.clone();
        retarget_model(
            &mut harness,
            vec![
                model_round(
                    &[(
                        "compare_identities",
                        serde_json::json!({"leftIdentity":"session-a","rightIdentity":"session-b","method":"GET","path":"/api/profile","family":"authorization","contractKey":"idor|/api/profile"}),
                    )],
                    900,
                ),
                model_round(
                    &[(
                        "record_hypothesis_result",
                        serde_json::json!({
                            "hypothesisKey":"h-tenant","status":"confirmed","family":"authorization",
                            "contractKey":"idor|/api/profile","title":"成员账号可见管理员权限字段",
                            "severity":"medium","confidence":0.8,
                            "controlRequestId":"req-0001","testRequestId":"req-0002",
                            "responseDifferenceArtifactId":"diff-0002.json",
                            "impact":"低权账号可读取高权配置","reproductionSteps":"用 session-b 重放同一请求并比较 permissions"
                        }),
                    )],
                    1_100,
                ),
                model_round(
                    &[(
                        "finish_target",
                        serde_json::json!({"coverage": closing_ledger(&["authorization"]), "stopReason": "覆盖队列已收口"}),
                    )],
                    1_300,
                ),
            ],
        );
        let outcome = NativeAgentBackend.execute(&harness.context);
        let status = match &outcome {
            AgentTargetOutcome::Completed(_) => "completed",
            AgentTargetOutcome::BoundedCompleted(_) => "completed_with_gaps",
            AgentTargetOutcome::Incomplete(_) => "paused",
            other => panic!("unexpected outcome: {:?}", other.detail()),
        };
        let mut tally = AgentPipelineTally::default();
        record_agent_target_outcome(
            &harness.db_path,
            "agent-scan",
            &harness.context.route,
            outcome,
            &mut tally,
        );
        assert_eq!(
            findings_for(&harness.db_path, AGENT_VULNERABILITY_STAGE).len(),
            1,
            "the bound pair is what earns the finding"
        );
        assert_agent_surface(&harness, &url, status, 2, 1);
        let findings = findings_for(&harness.db_path, AGENT_VULNERABILITY_STAGE);
        let record = json(findings[0].1.clone());
        assert_eq!(
            record["controlRequestId"].as_str(),
            Some("req-0001"),
            "the finding must cite the two executed requests: {record}"
        );
        assert_eq!(record["testRequestId"].as_str(), Some("req-0002"));
        assert_eq!(
            record["responseDifferenceArtifactId"].as_str(),
            Some("diff-0002.json")
        );
    }

    /// §14.5: one side of the pair is missing, so nothing may be concluded.
    #[test]
    fn e2e_missing_side_stops_at_insufficient_evidence() {
        let mut harness = agent_harness(
            "e2e-missing",
            unequal_role_site,
            vec![
                AgentIdentity::scoped("session-a"),
                AgentIdentity::scoped("session-b"),
            ],
        );
        seed_session(&harness.db_path, "session-a", "cookie-alpha", "Bearer alpha");
        let url = harness.context.target_url.clone();
        retarget_model(
            &mut harness,
            vec![
                model_round(
                    &[(
                        "compare_identities",
                        // session-b exists in the task but its captured session is
                        // gone, which is the ordinary "one side missing" shape.
                        serde_json::json!({"leftIdentity":"session-a","rightIdentity":"session-b","method":"GET","path":"/api/profile","family":"authorization","contractKey":"idor|/api/profile"}),
                    )],
                    900,
                ),
                model_round(
                    &[(
                        "record_hypothesis_result",
                        serde_json::json!({"hypothesisKey":"h-missing","status":"insufficient_evidence","family":"authorization","contractKey":"idor|/api/profile"}),
                    )],
                    1_100,
                ),
                model_round(
                    &[(
                        "finish_target",
                        serde_json::json!({"coverage": closing_ledger(&[]), "stopReason": "缺少身份 B 的会话，无法做对照"}),
                    )],
                    1_300,
                ),
            ],
        );
        let outcome = NativeAgentBackend.execute(&harness.context);
        assert!(
            !matches!(outcome, AgentTargetOutcome::Failed(_)),
            "a missing second identity is not a failure: {:?}",
            outcome.detail()
        );
        let status = match &outcome {
            AgentTargetOutcome::Completed(_) => "completed",
            AgentTargetOutcome::BoundedCompleted(_) => "completed_with_gaps",
            AgentTargetOutcome::Incomplete(_) => "paused",
            other => panic!("unexpected outcome: {:?}", other.detail()),
        };
        let mut tally = AgentPipelineTally::default();
        record_agent_target_outcome(
            &harness.db_path,
            "agent-scan",
            &harness.context.route,
            outcome,
            &mut tally,
        );
        assert_agent_surface(&harness, &url, status, 1, 0);
        assert_eq!(
            findings_for(&harness.db_path, AGENT_VULNERABILITY_STAGE).len(),
            0,
            "no pair, no finding"
        );
        let state = NativeAgentState::read(&harness.db_path, "agent-scan", &url).unwrap();
        assert!(
            !state.covered_families.contains(&"authorization".to_string()),
            "authorization cannot be covered by one side: {:?}",
            state.covered_families
        );
        assert_eq!(
            state.target_requests, 1,
            "only the side that resolved reached the target"
        );
        let prompts = harness.model_seen.lock().unwrap().join("\n");
        assert!(
            prompts.contains("insufficient_evidence"),
            "the model must be told the pair is incomplete"
        );
    }

    /// §14: what every mock case has to leave behind on the result surfaces —
    /// target state, attempt lineage, spent requests, tokens, coverage evidence,
    /// findings and the fields the panel reads. §12 adds the last block: the run
    /// ledger is the authority and the checkpoint row is its serialization, so the
    /// two must agree instead of each keeping its own idea of the state.
    pub(super) fn assert_agent_surface(
        harness: &AgentHarness,
        url: &str,
        expected_status: &str,
        expected_target_requests: i64,
        expected_findings: usize,
    ) {
        let connection = db::open(&harness.db_path).unwrap();
        let stored: String = connection
            .query_row(
                "SELECT status FROM sentinel_targets WHERE scan_id='agent-scan' AND url=?1",
                [url],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, expected_status, "the target has one terminal state");
        let state = NativeAgentState::read(&harness.db_path, "agent-scan", url)
            .expect("the attempt leaves a checkpoint");
        assert_eq!(
            state.target_requests, expected_target_requests,
            "the HTTP budget the run really spent survives"
        );
        assert!(state.token_usage.total_tokens > 0, "tokens must be accounted");
        assert_eq!(
            state.confirmed_findings as usize, expected_findings,
            "the ledger counts exactly the confirmed findings"
        );
        let run = read_run_row(&harness.db_path);
        assert_eq!(run.rows, 1, "one coordinator run per scan");
        assert_eq!(run.used_requests, state.token_usage.model_requests);
        assert_eq!(run.used_tokens, state.token_usage.total_tokens);
        let coverage = findings_for(&harness.db_path, AGENT_COVERAGE_STAGE);
        assert_eq!(coverage.len(), 1, "one coverage ledger per target");
        let ledger = json(coverage[0].1.clone());
        for key in [
            "coveredFamilies",
            "uncoveredFamilies",
            "coverageEvidence",
            "confirmedFindings",
            "targetRequests",
            "totalTokens",
        ] {
            assert!(ledger.get(key).is_some(), "the summary must carry {key}: {ledger}");
        }
        assert_eq!(
            ledger["confirmedFindings"].as_i64(),
            Some(expected_findings as i64)
        );
        assert_eq!(
            findings_for(&harness.db_path, AGENT_VULNERABILITY_STAGE).len(),
            expected_findings
        );
        let evidence: Vec<JsonValue> = ledger["coverageEvidence"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for row in evidence
            .iter()
            .filter(|row| row["result"] == "covered")
        {
            assert!(
                !row["requestRecordIds"]
                    .as_array()
                    .map(|ids| ids.is_empty())
                    .unwrap_or(true),
                "a covered entry must cite executed requests: {row}"
            );
        }
        let snapshot: String = connection
            .query_row(
                "SELECT snapshot_json FROM agent_snapshots WHERE run_id=(SELECT id FROM agent_runs WHERE scan_id='agent-scan' ORDER BY rowid DESC LIMIT 1)",
                [],
                |row| row.get(0),
            )
            .unwrap_or_default();
        let snapshot = json(snapshot);
        if !snapshot.is_null() {
            assert_eq!(
                snapshot["targetRequests"].as_i64(),
                Some(state.target_requests),
                "the canonical snapshot and the checkpoint must agree"
            );
            assert_eq!(
                snapshot["usedTokens"].as_i64(),
                Some(state.token_usage.total_tokens)
            );
        }
    }

    /// §11: a target ends in exactly one of the seven terminal states, written
    /// through the one reducer, and a new attempt replaces the current state
    /// instead of adding a second one the UI would have to guess between.
    #[test]
    fn every_target_ends_in_exactly_one_terminal_state() {
        let (_root, db_path) = temp_database("terminal-vocabulary");
        seed_scan(&db_path, "agent-scan", "scanning");
        let route = test_route("standard", "framework_application");
        let url = route.url.clone();
        seed_target(&db_path, &url);
        let statuses: Vec<(AgentTargetOutcome, &str)> = vec![
            (
                AgentTargetOutcome::Completed(AgentCompletion::without_ledger(
                    "覆盖账本已收口",
                    AGENT_STOP_FINISH,
                )),
                "completed",
            ),
            (
                AgentTargetOutcome::BoundedCompleted(AgentCompletion {
                    ledger_reported: true,
                    covered_families: vec!["authorization".to_string()],
                    uncovered_families: vec!["business_flow".to_string()],
                    verified_tool_results: 1,
                    ..AgentCompletion::bounded("预算边界收口，仍有命名缺口")
                }),
                "completed_with_gaps",
            ),
            (AgentTargetOutcome::incomplete("可继续同一尝试"), "paused"),
            (
                AgentTargetOutcome::limited(
                    "目标返回明确的 WAF、验证码或机器人挑战信号（HTTP 403）",
                ),
                "protected_stop",
            ),
            (AgentTargetOutcome::Cancelled, "cancelled"),
            (
                AgentTargetOutcome::resume_incompatible("续跑状态不兼容，需要重新执行"),
                "resume_incompatible",
            ),
            (AgentTargetOutcome::failed("模型配置错误"), "failed"),
        ];
        let vocabulary = [
            "completed",
            "completed_with_gaps",
            "paused",
            "cancelled",
            "protected_stop",
            "resume_incompatible",
            "failed",
        ];
        for (outcome, expected) in &statuses {
            assert_eq!(outcome.terminal_status(), *expected, "{expected}");
            assert!(
                vocabulary.contains(&outcome.terminal_status()),
                "only the §11 vocabulary may reach the target row: {}",
                outcome.terminal_status()
            );
        }
        // One run holds exactly one terminal state, so each case gets its own
        // attempt and target row: the recorder must not need a second source to
        // decide what the target ended as.
        for (outcome, expected) in statuses {
            let (root, case_db) = temp_database(&format!("terminal-{expected}"));
            seed_scan(&case_db, "agent-scan", "scanning");
            let case_route = test_route("standard", "framework_application");
            let case_url = case_route.url.clone();
            seed_target(&case_db, &case_url);
            if outcome == AgentTargetOutcome::Cancelled {
                // Cancellation is written by the pause/cancel path; the recorder
                // deliberately leaves the row alone and reports "not recorded".
                let mut tally = AgentPipelineTally::default();
                assert!(
                    !record_agent_target_outcome(&case_db, "agent-scan", &case_route, outcome, &mut tally),
                    "a cancelled target is not recorded as finished work"
                );
                let _ = root;
                continue;
            }
            if expected == "paused" {
                // A resumable stop is only recognised as such through the
                // checkpoint: without an open ledger the reducer would legitimately
                // close the run.
                let mut open = NativeAgentState::fresh(
                    1,
                    AgentBackendKind::Native,
                    "evidence",
                    "plan",
                    vec!["family:business_flow".to_string()],
                );
                open.target_requests = 1;
                open.persist(&case_db, "agent-scan", &case_url).unwrap();
            }
            let mut tally = AgentPipelineTally::default();
            record_agent_target_outcome(&case_db, "agent-scan", &case_route, outcome, &mut tally);
            let connection = db::open(&case_db).unwrap();
            let rows: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM sentinel_targets WHERE scan_id='agent-scan' AND url=?1",
                    [&case_url],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(rows, 1, "one target keeps one row, not one per attempt");
            let stored: String = connection
                .query_row(
                    "SELECT status FROM sentinel_targets WHERE scan_id='agent-scan' AND url=?1",
                    [&case_url],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(&stored, expected, "the reducer's spelling must reach the row");
            // The run row, the terminal checkpoint and the target row agree, so no
            // second source can disagree about what this attempt ended as.
            let checkpoint =
                read_agent_checkpoint(&case_db, "agent-scan", &case_url, "agent_terminal");
            assert!(!checkpoint.is_null(), "the terminal record must exist");
            assert_eq!(value_first(&checkpoint, &["status"]), stored);
            let reduced: String = connection
                .query_row(
                    "SELECT terminal_state FROM agent_runs WHERE scan_id='agent-scan' AND target_url=?1",
                    [&case_url],
                    |row| row.get(0),
                )
                .unwrap_or_default();
            match crate::agent_runtime::contract::TerminalState::parse(&reduced)
                .map(crate::agent_runtime::contract::TerminalState::to_sentinel_status)
            {
                // A terminal run projects exactly what the reducer decided.
                Some(reduced_status) => assert_eq!(
                    reduced_status,
                    stored.as_str(),
                    "the projection follows the reducer for {expected}"
                ),
                // An open run has no reduced state yet, so the target row keeps the
                // resumable spelling and the run row itself is not terminal.
                None => {
                    assert_eq!(expected, "paused", "only an open run may lack a state");
                    let status: String = connection
                        .query_row(
                            "SELECT status FROM agent_runs WHERE scan_id='agent-scan' AND target_url=?1",
                            [&case_url],
                            |row| row.get(0),
                        )
                        .unwrap();
                    assert_ne!(status, "terminal", "a resumable attempt keeps its run open");
                }
            }
            let _ = root;
        }
        // Ordinary outcomes are not failures: 401/403, an unlogged target, zero
        // findings and a missing second identity all stay non-fatal.
        for reason in [
            "接口返回 HTTP 401 未登录，仅记录权限边界",
            "接口返回 HTTP 403 无权访问，仅记录权限边界",
            "零确认漏洞，覆盖账本已收口",
            "任务只有一个身份，未做账号 A/B 对照",
        ] {
            let outcome = AgentTargetOutcome::incomplete(reason);
            assert_ne!(outcome.terminal_status(), "failed", "{reason}");
            assert!(!outcome.stop().is_some_and(|stop| stop.requires_fuse()));
        }
        assert_ne!(
            AgentTargetOutcome::Completed(AgentCompletion::without_ledger(
                "没有发现漏洞",
                AGENT_STOP_FINISH,
            ))
            .terminal_status(),
            "failed"
        );
    }

    /// Case 16: the model dies mid-run. The evidence stays, the checkpoint stays
    /// resumable and nothing is reported as a failure of the target itself.
    #[test]
    fn e2e_model_interruption_keeps_the_run_resumable() {
        let harness = agent_harness("e2e-model-down", mock_site, vec![AgentIdentity::anonymous()]);
        let AgentHarness {
            db_path,
            mut context,
            site_seen,
            ..
        } = harness;
        let target_url = context.target_url.clone();
        let (model_port, _seen, _stop) = spawn_endpoint(std::sync::Arc::new(|request| {
            if request.contains("POST /v1/chat/completions") {
                return (
                    500,
                    "application/json",
                    "{\"error\":{\"message\":\"upstream unavailable\"}}".to_string(),
                );
            }
            (404, "text/plain", "unexpected path".to_string())
        }));
        context.environment.api_base = format!("http://127.0.0.1:{model_port}/v1");
        let outcome = NativeAgentBackend.execute(&context);
        assert!(
            matches!(outcome, AgentTargetOutcome::Incomplete(_)),
            "a transport stall must stay retryable: {:?}",
            outcome.detail()
        );
        let stored = NativeAgentState::read(&db_path, "agent-scan", &target_url).unwrap();
        assert!(
            stored.terminal_reason.is_empty(),
            "an interrupted run must not look finished"
        );
        assert_eq!(stored.target_requests, 0);
        assert_eq!(site_seen.lock().unwrap().len(), 0);
    }
