    /// §13.6: every parameter of every tool is checked against its full schema —
    /// types, required keys, closed objects, enums, bounds, sizes, nesting and URL
    /// format — and each violation names the JSON path it belongs to.
    #[test]
    fn schema_validation_covers_types_bounds_enums_and_paths() {
        fn codes(name: &str, arguments: JsonValue) -> Vec<String> {
            agent_validate_arguments(name, &arguments)
                .err()
                .map(|rejection| {
                    rejection
                        .errors
                        .iter()
                        .map(|row| {
                            format!(
                                "{}:{}",
                                row["path"].as_str().unwrap_or_default(),
                                row["keyword"].as_str().unwrap_or_default()
                            )
                        })
                        .collect()
                })
                .unwrap_or_default()
        }

        assert!(
            codes(
                "replay_http",
                serde_json::json!({"identity":"a","method":"GET","url":"https://x.example/api"})
            )
            .is_empty(),
            "a well-formed call must not be rejected"
        );
        let problems = codes(
            "replay_http",
            serde_json::json!({"identity":7,"method":"TRACE","url":"not a url","attempt":9}),
        );
        for expected in [
            "$.identity:type",
            "$.method:enum",
            "$.url:format",
            "$.attempt:maximum",
        ] {
            assert!(problems.contains(&expected.to_string()), "{expected} missing from {problems:?}");
        }
        let nested = codes(
            "finish_target",
            serde_json::json!({
                "coverage": [{"family":"authorization","status":"covered","reason":"","extra":1}],
                "stopReason": "x",
                "bogus": true,
            }),
        );
        for expected in [
            "$.coverage[0].reason:minLength",
            "$.coverage[0].extra:additionalProperties",
            "$.bogus:additionalProperties",
        ] {
            assert!(nested.contains(&expected.to_string()), "{expected} missing from {nested:?}");
        }
        assert_eq!(
            codes(
                "finish_target",
                serde_json::json!({"coverage": [], "stopReason": "x"})
            ),
            vec!["$.coverage:minItems".to_string()]
        );
        assert_eq!(
            codes(
                "compare_identities",
                serde_json::json!({
                    "leftIdentity": "",
                    "rightIdentity": "b",
                    "method": "GET",
                    "path": "/api/x",
                    "ownershipHints": (0..13).map(|index| format!("f{index}")).collect::<Vec<_>>(),
                })
            )
            .into_iter()
            .filter(|code| code.ends_with("minLength") || code.ends_with("maxItems"))
            .collect::<Vec<_>>(),
            vec![
                "$.leftIdentity:minLength".to_string(),
                "$.ownershipHints:maxItems".to_string()
            ]
        );
        assert!(codes("no_such_tool", serde_json::json!({})).iter().any(|code| code.contains(":tool")));
    }

    /// §7: a call that fails schema validation is refused before it can reach the
    /// target, costs no request budget, and repeating it counts as no progress.
    #[test]
    fn invalid_arguments_never_reach_the_target() {
        let harness = agent_harness(
            "schema-budget",
            mock_site,
            vec![AgentIdentity::anonymous()],
        );
        let mut runtime = AgentToolRuntime::default();
        let bad = serde_json::json!({"identity":"anonymous","method":"GET"});
        let first = agent_execute_tool(&harness.context, &mut runtime, "replay_http", &bad);
        assert_eq!(first.model_view["code"], "invalid_arguments");
        assert_eq!(runtime.target_requests, 0, "a refused call spends nothing");
        assert!(
            harness.site_seen.lock().unwrap().is_empty(),
            "the target was never contacted"
        );
        let fingerprint = first.model_view["errorFingerprint"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        assert!(!fingerprint.is_empty());
        let second = agent_execute_tool(&harness.context, &mut runtime, "replay_http", &bad);
        assert_eq!(
            second.model_view["errorFingerprint"].as_str(),
            Some(fingerprint.as_str()),
            "the same mistake must produce the same stable code"
        );
        assert!(
            runtime.repeats_argument_error(),
            "a repeated identical rejection counts as no progress"
        );
        // A different mistake is a different fingerprint, and one good call clears
        // the streak.
        let other = agent_execute_tool(
            &harness.context,
            &mut runtime,
            "replay_http",
            &serde_json::json!({"identity":"anonymous","method":"BREW","url":"https://x.example/"}),
        );
        assert_ne!(
            other.model_view["errorFingerprint"].as_str(),
            Some(fingerprint.as_str())
        );
        let good = agent_execute_tool(
            &harness.context,
            &mut runtime,
            "replay_http",
            &serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{}/api/orders", harness.context.target_url)}),
        );
        assert_eq!(
            good.model_view["code"].as_str(),
            None,
            "a valid call executes: {}",
            good.model_view
        );
        assert!(!runtime.repeats_argument_error());
    }

    /// §11: the run row is registered while the backend is still working, carries
    /// the frozen plan's four ceilings, and keeps the real spend even when the
    /// outcome is not a `*Completed` state.
    #[test]
    fn agent_run_row_keeps_budget_and_spend_for_every_terminal_state() {
        let (_root, db_path) = temp_database("agent-runrow");
        seed_scan(&db_path, "agent-scan", "scanning");
        let context = test_context(
            &db_path,
            "https://app.example.invalid",
            vec![AgentIdentity::anonymous()],
        );
        let plan = context.execution_plan.clone();
        persist_agent_execution_plan(&db_path, "agent-scan", 1, &context.target_url, &plan).unwrap();
        let mut seeded = NativeAgentState::fresh(
            1,
            AgentBackendKind::Native,
            "evidence-hash",
            "plan-hash",
            vec!["family:authorization".to_string()],
        );
        seeded.token_usage = AgentTokenUsage {
            input_tokens: 900,
            cached_input_tokens: 100,
            output_tokens: 60,
            total_tokens: 960,
            model_requests: 3,
        };
        seeded.target_requests = 4;
        seeded.persist(&db_path, "agent-scan", &context.target_url).unwrap();

        runtime_open_run(&db_path, "agent-scan", &context.route);
        let running = read_run_row(&db_path);
        assert_eq!(running.status, "running", "the row must exist before the close-out");
        assert_eq!(running.soft_token_budget, plan.soft_uncached_tokens);
        assert_eq!(running.hard_token_budget, plan.hard_total_tokens);
        assert_eq!(running.soft_request_budget, plan.soft_model_requests);
        assert_eq!(running.hard_request_budget, plan.hard_model_requests);

        record_runtime_terminal_facts(
            &db_path,
            "agent-scan",
            &context.route,
            &AgentTargetOutcome::limited(
                "目标返回明确的 WAF、验证码或机器人挑战信号（HTTP 403）",
            ),
        );
        let closed = read_run_row(&db_path);
        assert_eq!(closed.status, "terminal");
        assert_eq!(closed.rows, 1, "the close-out must reuse the open row");
        assert_eq!(closed.used_tokens, 960, "a Limited outcome still owes its spend");
        assert_eq!(closed.used_requests, 3);
        assert_eq!(closed.terminal_code, AGENT_STOP_WAF);
        assert_eq!(closed.pending_contracts, 1);
    }

    /// A resumable outcome leaves the run row open, and the resume reuses that
    /// same row: one run per attempt, with the spend converged rather than forked.
    #[test]
    fn resumable_outcome_keeps_one_run_row_for_the_attempt() {
        let (_root, db_path) = temp_database("agent-resume-run");
        seed_scan(&db_path, "agent-scan", "scanning");
        let context = test_context(
            &db_path,
            "https://app.example.invalid",
            vec![AgentIdentity::anonymous()],
        );
        persist_agent_execution_plan(
            &db_path,
            "agent-scan",
            1,
            &context.target_url,
            &context.execution_plan.clone(),
        ).unwrap();
        let mut seeded = NativeAgentState::fresh(
            1,
            AgentBackendKind::Native,
            "evidence-hash",
            "plan-hash",
            vec!["family:authorization".to_string()],
        );
        seeded.token_usage = AgentTokenUsage {
            input_tokens: 900,
            cached_input_tokens: 100,
            output_tokens: 60,
            total_tokens: 960,
            model_requests: 3,
        };
        seeded.target_requests = 4;
        seeded.persist(&db_path, "agent-scan", &context.target_url).unwrap();

        runtime_open_run(&db_path, "agent-scan", &context.route);
        record_runtime_terminal_facts(
            &db_path,
            "agent-scan",
            &context.route,
            &AgentTargetOutcome::incomplete("模型传输中断"),
        );
        let open = read_run_row(&db_path);
        assert_eq!(open.status, "running", "an open attempt must not settle");
        assert!(open.terminal_code.is_empty(), "{open:?}");
        assert_eq!(open.used_tokens, 960);

        // The resume spends more on the same attempt and then hits a boundary.
        let mut later = seeded.clone();
        later.token_usage.total_tokens = 1_500;
        later.token_usage.model_requests = 5;
        later.target_requests = 6;
        later.terminal_reason = "目标返回明确的 WAF、验证码或机器人挑战信号".to_string();
        later.persist(&db_path, "agent-scan", &context.target_url).unwrap();
        record_runtime_terminal_facts(
            &db_path,
            "agent-scan",
            &context.route,
            &AgentTargetOutcome::limited(
                "目标返回明确的 WAF、验证码或机器人挑战信号（HTTP 403）",
            ),
        );
        let closed = read_run_row(&db_path);
        assert_eq!(closed.rows, 1, "the resume must reuse the open run");
        assert_eq!(closed.status, "terminal");
        assert_eq!(closed.used_tokens, 1_500, "cumulative spend, never re-added");
        assert_eq!(closed.used_requests, 5);
        assert_eq!(closed.terminal_code, AGENT_STOP_WAF);
    }

    // ------------------------------------------------------------------
    // Remediation §13.1 / §13.2 — attempt inheritance and the frozen plan
    // ------------------------------------------------------------------

    /// §13.1: "继续未完成阶段" creates a new attempt whose parent is the previous
    /// unfinished one, and must inherit the parent's spend, contracts and queue.
    #[test]
    fn continue_incomplete_inherits_the_parent_attempt_ledger() {
        let (_root, db_path) = temp_database("remediation-resume");
        seed_scan(&db_path, "agent-scan", "scanning");
        seed_attempt_row(&db_path, 1, "initial");
        seed_attempt_row(&db_path, 2, "resume");
        let context = test_context(
            &db_path,
            "https://app.example.invalid",
            vec![AgentIdentity::anonymous()],
        );
        let evidence_hash = agent_stable_hash(&context.evidence);
        let plan_hash = context.execution_plan.hash();
        let mut parent = NativeAgentState::fresh(
            1,
            AgentBackendKind::Native,
            &evidence_hash,
            &plan_hash,
            vec!["family:authorization".to_string()],
        );
        parent.target_requests = 2;
        parent.observed_requests = vec![
            trace_of("GET", "/api/orders", "authorization", "idor|/api/orders"),
            trace_of("GET", "/api/profile", "information_disclosure", ""),
        ];
        parent.contract_attempts = vec![("idor|/api/orders".to_string(), 1)];
        parent.contract_outcomes = vec![("idor|/api/orders".to_string(), "rejected".to_string())];
        parent.persist(&db_path, "agent-scan", &context.target_url).unwrap();

        // The app's "继续未完成阶段" increments attempt_count and marks the new
        // attempt as a resume of attempt 1.
        seed_attempt_row(&db_path, 1, "initial");
        seed_attempt_row(&db_path, 2, "resume");
        let lineage = agent_attempt_lineage(&db_path, "agent-scan", 2);
        assert_eq!(lineage.resume_kind, AgentResumeKind::ContinueIncomplete);
        assert_eq!(lineage.parent_attempt_number, Some(1));

        let mut next = context.clone();
        next.attempt_number = 2;
        next.resume = true;
        let state =
            NativeAgentState::for_attempt(&db_path, "agent-scan", &next, &evidence_hash, &plan_hash)
                .expect("a compatible parent checkpoint must carry forward");
        assert_eq!(state.attempt_number, 2);
        assert_eq!(state.parent_attempt_number, Some(1));
        assert_eq!(state.target_requests, 2, "spent HTTP budget must survive");
        assert_eq!(state.observed_requests.len(), 2);
        assert_eq!(state.completed_contract_keys, vec!["idor|/api/orders".to_string()]);
        assert_eq!(state.pending_queue, vec!["family:authorization".to_string()]);
    }

    /// §13.1 step 4/5: the inherited contract is never re-executed and the new
    /// attempt's current surface does not show the parent's terminal reason.
    #[test]
    fn continued_attempt_never_reopens_finished_contracts() {
        let (_root, db_path) = temp_database("remediation-resume-contract");
        seed_scan(&db_path, "agent-scan", "scanning");
        let context = test_context(
            &db_path,
            "https://app.example.invalid",
            vec![AgentIdentity::anonymous()],
        );
        let evidence_hash = agent_stable_hash(&context.evidence);
        let plan_hash = context.execution_plan.hash();
        let mut parent = NativeAgentState::fresh(
            1,
            AgentBackendKind::Native,
            &evidence_hash,
            &plan_hash,
            vec!["contract:idor|/api/orders".to_string()],
        );
        parent.target_requests = 1;
        parent.observed_requests = vec![trace_of("GET", "/api/orders", "authorization", "idor|/api/orders")];
        parent.contract_attempts = vec![("idor|/api/orders".to_string(), 1)];
        parent.contract_outcomes = vec![("idor|/api/orders".to_string(), "rejected".to_string())];
        parent.terminal_reason = "连续 3 轮没有新增证据".to_string();
        parent.persist(&db_path, "agent-scan", &context.target_url).unwrap();

        seed_attempt_row(&db_path, 1, "initial");
        seed_attempt_row(&db_path, 2, "resume");
        let mut next = context.clone();
        next.attempt_number = 2;
        next.resume = true;
        let state = NativeAgentState::for_attempt(
            &db_path, "agent-scan", &next, &evidence_hash, &plan_hash,
        )
        .expect("inherited");
        assert!(
            state.terminal_reason.is_empty(),
            "a finished parent must not end the child: {}",
            state.terminal_reason
        );
        let runtime = AgentToolRuntime::restore(&state);
        assert!(
            runtime.queue_key_satisfied(&context, "contract:idor|/api/orders"),
            "the closed contract must be gone from the queue"
        );
        assert_eq!(runtime.attempts_for("idor|/api/orders"), 1);
    }

    /// §13.3: a v1 checkpoint migrates, and the original stays auditable.
    #[test]
    fn v1_checkpoint_migrates_to_v2_without_inventing_evidence() {
        let (_root, db_path) = temp_database("remediation-v1");
        seed_scan(&db_path, "agent-scan", "scanning");
        seed_attempt_row(&db_path, 1, "initial");
        seed_attempt_row(&db_path, 2, "resume");
        let url = "https://app.example.invalid";
        let context = test_context(&db_path, url, vec![AgentIdentity::anonymous()]);
        let evidence_hash = agent_stable_hash(&context.evidence);
        let plan_hash = context.execution_plan.hash();
        // What v1 actually stored: no request ledger, only name lists.
        let v1 = serde_json::json!({
            "schemaVersion": 1,
            "attemptNumber": 1,
            "backend": "native",
            "evidenceHash": evidence_hash,
            "executionPlanHash": plan_hash,
            "completedContractKeys": ["idor|/api/orders"],
            "exhaustedContractKeys": ["sqli|/api/search"],
            "coveredFamilies": ["authorization"],
            "pendingQueue": ["family:business_flow"],
            "progressSignature": "e1:r0:i0:p0:v1:f0",
            "budgetUsage": {"modelRequests": 4, "targetRequests": 3, "discoveryRounds": 1},
            "terminalReason": "连续 3 轮没有新增证据",
            "turns": 4,
            "noProgressStreak": 3,
            "tokenUsage": {"inputTokens": 900, "outputTokens": 60, "totalTokens": 960, "modelRequests": 4},
            "lastExpansionReason": ""
        });
        write_agent_checkpoint(&db_path, "agent-scan", url, "native_agent_state", &v1).unwrap();

        let migrated =
            migrate_native_agent_state(&v1).expect("v1 must migrate, not disappear");
        assert_eq!(migrated.schema_version, 2);
        assert_eq!(migrated.target_requests, 3, "derivable budget is kept");
        assert_eq!(migrated.discovery_rounds, 1);
        assert!(
            migrated.observed_requests.is_empty(),
            "v1 recorded no request ledger, so none may be invented"
        );
        assert_eq!(
            (migrated.evidence_hash.as_str(), migrated.execution_plan_hash.as_str()),
            (evidence_hash.as_str(), plan_hash.as_str()),
            "the frozen hashes must survive the migration"
        );

        // Loading it through the checkpoint path upgrades in place and archives
        // the original.
        let state = NativeAgentState::load(&db_path, "agent-scan", url)
            .expect("migratable")
            .expect("a checkpoint exists");
        assert_eq!(state.inherited_checkpoint_schema, Some(1));
        assert_eq!(
            state.terminal_reason, "连续 3 轮没有新增证据",
            "the parent's own reason stays on its row as history; only the child clears it"
        );
        let archived = read_agent_checkpoint(db_path.as_path(), "agent-scan", url, "native_agent_state_legacy");
        assert_eq!(archived.get("schemaVersion").and_then(JsonValue::as_i64), Some(1));
        let stored = read_agent_checkpoint(db_path.as_path(), "agent-scan", url, "native_agent_state");
        assert_eq!(stored.get("schemaVersion").and_then(JsonValue::as_i64), Some(2));
    }

    /// §4.2: a migrated v1 row cannot support a continuation, because it cannot
    /// say which requests were already spent.
    #[test]
    fn migrated_checkpoint_without_ledger_refuses_continuation() {
        let (_root, db_path) = temp_database("remediation-v1-nostream");
        seed_scan(&db_path, "agent-scan", "scanning");
        seed_attempt_row(&db_path, 1, "initial");
        seed_attempt_row(&db_path, 2, "resume");
        let url = "https://app.example.invalid";
        let context = test_context(&db_path, url, vec![AgentIdentity::anonymous()]);
        let evidence_hash = agent_stable_hash(&context.evidence);
        let plan_hash = context.execution_plan.hash();
        let v1 = serde_json::json!({
            "schemaVersion": 1,
            "attemptNumber": 1,
            "backend": "native",
            "evidenceHash": evidence_hash,
            "executionPlanHash": plan_hash,
            "completedContractKeys": [],
            "exhaustedContractKeys": [],
            "coveredFamilies": [],
            "pendingQueue": [],
            "budgetUsage": {"targetRequests": 5},
            "tokenUsage": {"totalTokens": 10, "modelRequests": 1}
        });
        write_agent_checkpoint(&db_path, "agent-scan", url, "native_agent_state", &v1).unwrap();
        let mut next = context.clone();
        next.attempt_number = 2;
        let error = NativeAgentState::for_attempt(
            &db_path, "agent-scan", &next, &evidence_hash, &plan_hash,
        )
        .expect_err("a ledger-less parent must not be resumed as if it were complete");
        assert_eq!(error.kind, NativeStateRejectionKind::ResumeLedgerIncomplete);
    }

    /// §4.2: a newer schema is refused loudly; a corrupt row never turns into a
    /// fresh run that re-burns tokens.
    #[test]
    fn newer_and_corrupted_checkpoints_are_refused_not_restarted() {
        let (_root, db_path) = temp_database("remediation-refuse");
        seed_scan(&db_path, "agent-scan", "scanning");
        seed_attempt_row(&db_path, 1, "initial");
        seed_attempt_row(&db_path, 2, "resume");
        let url = "https://app.example.invalid";
        let context = test_context(&db_path, url, vec![AgentIdentity::anonymous()]);
        let mut next = context.clone();
        next.attempt_number = 2;
        let evidence_hash = agent_stable_hash(&context.evidence);
        let plan_hash = context.execution_plan.hash();

        write_agent_checkpoint(
            &db_path,
            "agent-scan",
            url,
            "native_agent_state",
            &serde_json::json!({"schemaVersion": 3, "attemptNumber": 1}),
        ).unwrap();
        let error = NativeAgentState::for_attempt(
            &db_path, "agent-scan", &next, &evidence_hash, &plan_hash,
        )
        .expect_err("a newer schema cannot be read as v2");
        assert_eq!(error.kind, NativeStateRejectionKind::SchemaTooNew);

        write_agent_checkpoint(
            &db_path,
            "agent-scan",
            url,
            "native_agent_state",
            &serde_json::json!({"schemaVersion": "two"}),
        ).unwrap();
        let error = NativeAgentState::for_attempt(
            &db_path, "agent-scan", &next, &evidence_hash, &plan_hash,
        )
        .expect_err("an unreadable schema cannot be read as absent");
        assert_eq!(error.kind, NativeStateRejectionKind::Corrupted);

        let connection = db::open(&db_path).unwrap();
        connection
            .execute(
                r#"UPDATE sentinel_checkpoints SET raw_json='{"broken' WHERE scan_id='agent-scan' AND stage='native_agent_state'"#,
                [],
            )
            .unwrap();
        let error = NativeAgentState::for_attempt(
            &db_path, "agent-scan", &next, &evidence_hash, &plan_hash,
        )
        .expect_err("truncated JSON is corruption, not a missing checkpoint");
        assert_eq!(error.kind, NativeStateRejectionKind::Corrupted);
        assert_eq!(error.parent_attempt_number, Some(1));
    }

    /// §13.2: settings edits must not change the plan a continuation runs on.
    #[test]
    fn continuation_reuses_the_frozen_plan_and_fresh_rebuilds_it() {
        let (_root, db_path) = temp_database("remediation-frozen-plan");
        seed_scan(&db_path, "agent-scan", "scanning");
        seed_attempt_row(&db_path, 2, "resume");
        let context = test_context(
            &db_path,
            "https://app.example.invalid",
            vec![AgentIdentity::anonymous()],
        );
        let frozen = serde_json::json!({
            "schemaVersion": 1,
            "owner": "oviraptor",
            "backend": "native",
            "mode": "quick",
            "surface": "framework_application",
            "targetUrl": context.target_url,
            "attemptNumber": 1,
            "budgets": {
                "softUncachedTokens": 11_000,
                "hardTotalTokens": 22_000,
                "softModelRequests": 3,
                "hardModelRequests": 4,
                "maxTurns": 5
            },
            "coverage": {
                "requiredFamilies": AGENT_COVERAGE_FAMILIES,
                "contractLimit": 7,
                "discoveryPasses": 1,
                "verifierLimit": 1
            },
            "stopping": {
                "noProgressWindow": 2,
                "wafChallengeStopsImmediately": true,
                "softBudgetsRequireNoProgress": true,
                "ordinary401Or403DoesNotStopTarget": true
            },
            "allowedOrigins": ["app.example.invalid"],
            "identities": ["anonymous"],
            "modelProvider": "cloud",
            "timeoutSeconds": 60
        });
        write_agent_checkpoint(
            &db_path,
            "agent-scan",
            &context.route.url,
            "agent_execution_plan",
            &frozen,
        ).unwrap();

        let resumed = frozen_plan_for(&db_path, &context, 2);
        assert_eq!(resumed.soft_uncached_tokens, 11_000, "the parent budget stands");
        assert_eq!(resumed.mode, "quick", "a settings edit must not widen the mode");
        assert_eq!(resumed.max_turns, 5);
        assert_eq!(resumed.attempt_number, 2, "only the attempt identity is refreshed");

        seed_attempt_row(&db_path, 3, "fresh");
        let rebuilt = frozen_plan_for(&db_path, &context, 3);
        assert_ne!(
            rebuilt.soft_uncached_tokens, 11_000,
            "重新执行 rebuilds the plan from the current settings"
        );
    }

    /// §3.6: a checkpoint that cannot be safely inherited must stop the
    /// continuation with a specific reason instead of silently restarting.
    #[test]
    fn incompatible_checkpoint_stops_instead_of_restarting_fresh() {
        let (_root, db_path) = temp_database("remediation-incompatible");
        seed_scan(&db_path, "agent-scan", "scanning");
        seed_attempt_row(&db_path, 1, "initial");
        seed_attempt_row(&db_path, 2, "resume");
        let context = test_context(
            &db_path,
            "https://app.example.invalid",
            vec![AgentIdentity::anonymous()],
        );
        let parent = NativeAgentState::fresh(
            1,
            AgentBackendKind::Native,
            "old-evidence-hash",
            "old-plan-hash",
            Vec::new(),
        );
        parent.persist(&db_path, "agent-scan", &context.target_url).unwrap();

        let mut next = context.clone();
        next.attempt_number = 2;
        next.resume = true;
        let error = NativeAgentState::for_attempt(&db_path, "agent-scan", &next, "new-evidence", "new-plan")
            .expect_err("evidence changed must be an explicit rejection");
        assert_eq!(error.kind, NativeStateRejectionKind::EvidenceChanged);
        assert_eq!(error.parent_attempt_number, Some(1));
        // The user-facing wording is fixed: never a generic model failure (§3.6).
        assert!(error.message().starts_with("续跑状态不兼容，需要重新执行"), "{:?}", error.message());
        // The parent ledger stays intact: no silent delete, no fresh restart.
        assert!(NativeAgentState::read(&db_path, "agent-scan", &context.target_url).is_some());
    }
