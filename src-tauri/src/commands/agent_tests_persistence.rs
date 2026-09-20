    /// Phase 2 §2.3 cases 1 and 2: a fresh re-run picks the backend from today's
    /// settings, in both directions, and the previous attempt's plan survives.
    #[test]
    fn fresh_rerun_switches_the_backend_both_ways() {
        let (_root, db_path) = temp_database("phase2-fresh-switch");
        seed_scan(&db_path, "agent-scan", "scanning");
        seed_attempt_row(&db_path, 1, "initial");
        seed_attempt_row(&db_path, 2, "fresh");
        let url = "https://app.example.invalid";
        let native = serde_json::json!({"agentBackendPolicy": "native"});
        let strix = serde_json::json!({"agentBackendPolicy": "strix"});

        // attempt 1 ran on Strix and froze that backend.
        persist_agent_execution_plan(
            &db_path,
            "agent-scan",
            1,
            url,
            &AgentExecutionPlan {
                backend: AgentBackendKind::Strix,
                ..test_plan_for("standard", url)
            },
        ).unwrap();
        assert_eq!(
            agent_select_backend(&db_path, "agent-scan", 1, url, &native, true),
            AgentBackendKind::Strix,
            "the attempt that already ran stays pinned"
        );
        assert_eq!(
            agent_select_backend(&db_path, "agent-scan", 2, url, &native, true),
            AgentBackendKind::Native,
            "a fresh attempt must follow the policy the user just changed"
        );
        // …and the other way round, with a native attempt 1.
        persist_agent_execution_plan(
            &db_path,
            "agent-scan",
            1,
            url,
            &AgentExecutionPlan {
                backend: AgentBackendKind::Native,
                ..test_plan_for("standard", url)
            },
        ).unwrap();
        let switched = temp_database("phase2-fresh-switch-back");
        let _ = switched.0;
        seed_scan(&switched.1, "agent-scan", "scanning");
        seed_attempt_row(&switched.1, 1, "initial");
        seed_attempt_row(&switched.1, 2, "fresh");
        persist_agent_execution_plan(
            &switched.1,
            "agent-scan",
            1,
            url,
            &AgentExecutionPlan {
                backend: AgentBackendKind::Native,
                ..test_plan_for("standard", url)
            },
        ).unwrap();
        assert_eq!(
            agent_select_backend(&switched.1, "agent-scan", 2, url, &strix, true),
            AgentBackendKind::Strix,
            "native then explicit strix on a fresh attempt must not stay native"
        );
        // §2.2: switching is never achieved by wiping history.
        let connection = db::open(&db_path).unwrap();
        let pinned: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM agent_runs WHERE scan_id='agent-scan' AND attempt_number=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(pinned >= 1, "attempt 1 keeps its own plan row");
    }

    /// Phase 2 §5.3: a durable write that cannot be made ends the attempt. No
    /// further model round and no further target request may be spent on state the
    /// runtime cannot record.
    #[test]
    fn persistence_failure_stops_before_the_next_model_round() {
        use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
        let mut harness = agent_harness(
            "p05-usage-write",
            mock_site,
            vec![AgentIdentity::anonymous()],
        );
        let target_url = harness.context.target_url.clone();
        let scripted = vec![
            model_round(
                &[(
                    "replay_http",
                    serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{target_url}/api/orders"),"family":"business_flow"}),
                )],
                900,
            ),
            model_round(&[("finish_target", serde_json::json!({"coverage": closing_ledger(&["business_flow"]), "stopReason":"收口"}))], 900),
        ];
        // The first round is served normally; the checkpoint table then disappears,
        // so the commit that must follow the tool result cannot be made. The rows
        // the round already earned stay, which is exactly why the loop may not spend
        // another round on state it cannot record (§5.2).
        let database = harness.db_path.clone();
        let counter = std::sync::Arc::new(AtomicUsize::new(0));
        let rows = scripted.clone();
        let (model_port, model_seen, _model_stop) =
            spawn_endpoint(std::sync::Arc::new(move |request| {
                if !request.contains("POST /v1/chat/completions") {
                    return (404, "text/plain", "unexpected path".to_string());
                }
                let index = counter.fetch_add(1, AtomicOrdering::SeqCst);
                if index == 0 {
                    let connection = db::open(&database).unwrap();
                    connection
                        .execute("DROP TABLE sentinel_checkpoints", [])
                        .unwrap();
                }
                (
                    200,
                    "application/json",
                    rows.get(index)
                        .cloned()
                        .unwrap_or_else(|| model_round(&[], 10)),
                )
            }));
        harness.context.environment.api_base = format!("http://127.0.0.1:{model_port}/v1");
        harness.model_seen = model_seen;
        let outcome = NativeAgentBackend.execute(&harness.context);
        assert_eq!(
            outcome.terminal_status(),
            "persistence_failure",
            "an unwritable ledger is its own end state: {:?}",
            outcome.detail()
        );
        assert_eq!(
            outcome.terminal_code(),
            crate::agent_runtime::contract::terminal_code::PERSISTENCE_FAILURE
        );
        assert!(
            outcome
                .detail()
                .contains("本地记录失败，已停止以避免重复消耗"),
            "the reason names the local record failure: {:?}",
            outcome.detail()
        );
        let prompts = harness.model_seen.lock().unwrap().join("\n");
        assert_eq!(
            prompts.matches("POST /v1/chat/completions").count(),
            1,
            "the attempt must stop before a second model round"
        );
        assert_eq!(
            harness.site_seen.lock().unwrap().len(),
            1,
            "and never spend a second target request"
        );
    }

    /// Phase 2 §5.3: the opposite half — the database is fine but the raw artifact
    /// cannot be written, so the tool refuses and the loop stops.
    #[test]
    fn artifact_write_failure_refuses_the_tool_and_stops_the_run() {
        let mut harness = agent_harness(
            "p05-artifact",
            mock_site,
            vec![AgentIdentity::anonymous()],
        );
        let target_url = harness.context.target_url.clone();
        retarget_model(
            &mut harness,
            vec![
                model_round(
                    &[(
                        "replay_http",
                        serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{target_url}/api/orders"),"family":"business_flow"}),
                    )],
                    900,
                ),
                model_round(&[("finish_target", serde_json::json!({"coverage": closing_ledger(&[]), "stopReason":"收口"}))], 900),
            ],
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&harness.context.target_dir, fs::Permissions::from_mode(0o500))
                .unwrap();
        }
        let outcome = NativeAgentBackend.execute(&harness.context);
        assert_eq!(
            outcome.terminal_status(),
            "persistence_failure",
            "{:?}",
            outcome.detail()
        );
        assert_eq!(
            harness.model_seen.lock().unwrap().len(),
            1,
            "no evidence, no second round"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(
                &harness.context.target_dir,
                fs::Permissions::from_mode(0o700),
            );
        }
    }

    /// Phase 2 §5.2: a locked database gets a short, bounded retry; anything else
    /// fails immediately instead of waiting forever.
    #[test]
    fn locked_writes_retry_bounded_and_other_errors_do_not() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let attempts = AtomicUsize::new(0);
        let ok = with_db_retry("检查点写入", || {
            let seen = attempts.fetch_add(1, Ordering::SeqCst);
            if seen < 2 {
                return Err("database is locked".to_string());
            }
            Ok(())
        });
        assert!(ok.is_ok(), "a lock that clears inside the budget still commits");
        assert_eq!(attempts.load(Ordering::SeqCst), 3);

        let disk = AtomicUsize::new(0);
        let failed: Result<(), String> = with_db_retry("检查点写入", || {
            disk.fetch_add(1, Ordering::SeqCst);
            Err("attempt to write a readonly database".to_string())
        });
        assert!(failed.is_err());
        assert_eq!(
            disk.load(Ordering::SeqCst),
            1,
            "a non-lock failure is reported at once, never retried"
        );
    }

    /// §11: the new terminal state is spelled one way everywhere, is never called a
    /// model failure, and is not picked up by an automatic resume.
    #[test]
    fn persistence_failure_is_its_own_terminal_state() {
        let outcome = AgentTargetOutcome::persistence_failure("本地记录写入失败");
        assert_eq!(outcome.terminal_status(), "persistence_failure");
        assert_ne!(outcome.terminal_status(), "failed");
        assert!(
            !outcome.stop().is_some_and(|stop| stop.requires_fuse()),
            "a local write failure never fuses the target"
        );
        let (_root, db_path) = temp_database("p05-resume-scope");
        seed_scan(&db_path, "agent-scan", "scanning");
        seed_target(&db_path, "https://app.example.invalid");
        let connection = db::open(&db_path).unwrap();
        connection
            .execute(
                "UPDATE sentinel_targets SET status='persistence_failure' WHERE scan_id='agent-scan'",
                [],
            )
            .unwrap();
        let resumable: i64 = connection
            .query_row(SENTINEL_RESUME_COUNT_SQL, ["agent-scan"], |row| row.get(0))
            .unwrap();
        assert_eq!(resumable, 0, "only 重新执行 may pick it up again");
    }

    /// Phase 2 §6.2 and §6.3: a stop that lands while a generation is in flight has
    /// to settle the transport first, say so in the ledger with real timestamps, and
    /// never bill or continue a round the runtime never read.
    #[test]
    fn a_stop_during_a_model_round_settles_the_transport_and_is_never_billed() {
        use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
        let harness = agent_harness("p06-cancel", mock_site, vec![AgentIdentity::anonymous()]);
        let AgentHarness {
            db_path,
            mut context,
            site_seen,
            ..
        } = harness;
        let target_url = context.target_url.clone();
        persist_agent_execution_plan(
            &db_path,
            "agent-scan",
            context.attempt_number,
            &target_url,
            &context.execution_plan.clone(),
        )
        .unwrap();
        let ledger = runtime_open_run(&db_path, "agent-scan", &context.route).expect("run row");
        context.run = Some(ledger.clone());
        let pause_db = db_path.clone();
        let served = std::sync::Arc::new(AtomicUsize::new(0));
        let served_counter = served.clone();
        let (model_port, _seen, _stop) = spawn_endpoint(std::sync::Arc::new(move |_request| {
            if served_counter.fetch_add(1, AtomicOrdering::SeqCst) == 0 {
                // The user stops the task while this generation is still running; the
                // provider answers five seconds later, which is exactly the request
                // that used to keep running after the UI said "stopped".
                if let Ok(connection) = db::open(&pause_db) {
                    let _ = connection.execute(
                        "UPDATE sentinel_scans SET status='cancelled' WHERE id='agent-scan'",
                        [],
                    );
                }
                std::thread::sleep(std::time::Duration::from_secs(5));
            }
            (
                200,
                "application/json",
                model_round(
                    &[(
                        "finish_target",
                        serde_json::json!({"coverage": closing_ledger(&[]), "stopReason":"收口"}),
                    )],
                    900,
                ),
            )
        }));
        context.environment.api_base = format!("http://127.0.0.1:{model_port}/v1");

        let started = std::time::Instant::now();
        let outcome = NativeAgentBackend.execute(&context);
        let elapsed = started.elapsed();
        assert!(
            matches!(outcome, AgentTargetOutcome::Cancelled),
            "{:?}",
            outcome.detail()
        );
        assert!(
            elapsed < std::time::Duration::from_secs(4),
            "the stop waited for the provider instead of closing it: {elapsed:?}"
        );
        assert_eq!(
            site_seen.lock().unwrap().len(),
            0,
            "a round that never returned may not spend a target request"
        );

        let connection = db::open(&db_path).unwrap();
        let settled: Option<(i64, String, String)> = connection
            .query_row(
                "SELECT sequence,json_extract(payload_json,'$.cancelRequestedAt'),json_extract(payload_json,'$.transportClosedAt') FROM agent_events WHERE run_id=?1 AND event_type='transport_cancelled' ORDER BY sequence LIMIT 1",
                [&ledger.run_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .ok();
        let (cancel_sequence, requested, closed) =
            settled.expect("the ledger holds when the stop settled");
        assert!(!requested.is_empty() && !closed.is_empty());
        let later_rounds: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM agent_events WHERE run_id=?1 AND sequence>?2 AND event_type='model_round_completed'",
                params![&ledger.run_id, cancel_sequence],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            later_rounds, 0,
            "the transport must settle before the round is closed out"
        );
        let (run_status, terminal_state): (String, String) = connection
            .query_row(
                "SELECT status,terminal_state FROM agent_runs WHERE id=?1",
                [&ledger.run_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_ne!(
            run_status, "terminal",
            "a stop keeps the attempt resumable instead of inventing an end"
        );
        assert!(terminal_state.is_empty(), "{terminal_state}");
        assert_eq!(
            connection
                .query_row(
                    "SELECT used_tokens FROM agent_runs WHERE id=?1",
                    [&ledger.run_id],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0,
            "an unread round is not billed"
        );
    }

    /// Phase 2 §3.2: the matrix classifies **every** target before any dependency is
    /// resolved, so a mixed scan is known up front and an all-native one needs no
    /// Strix, Docker or Python at all.
    #[test]
    fn backend_matrix_covers_every_target_before_dependencies() {
        let (_root, db_path) = temp_database("phase2-matrix");
        seed_scan(&db_path, "agent-scan", "scanning");
        seed_attempt_row(&db_path, 1, "initial");
        let urls = vec![
            "https://a.example.invalid".to_string(),
            "https://b.example.invalid".to_string(),
            "https://c.example.invalid".to_string(),
        ];
        let native = serde_json::json!({"agentBackendPolicy": "native"});
        let matrix = plan_scan_backends(&db_path, "agent-scan", 1, &urls, &native, true).unwrap();
        assert_eq!(matrix.targets.len(), 3, "one entry per queued target");
        assert!(
            matrix
                .targets
                .iter()
                .all(|target| target.backend == AgentBackendKind::Native),
            "{matrix:?}"
        );
        assert!(!matrix.requires_strix, "an all-native matrix needs no Strix");
        assert!(!matrix.requires_docker, "…and no Docker");
        assert!(matrix.requires_node, "web recon still needs the Node probe");
        assert!(matrix.requires_browser);

        // A matrix is frozen once built, so re-running it cannot change a target.
        let rebuilt = plan_scan_backends(&db_path, "agent-scan", 1, &urls, &serde_json::json!({"agentBackendPolicy":"strix"}), true).unwrap();
        assert_eq!(
            rebuilt, matrix,
            "replanning the same attempt must be a no-op"
        );

        // Mixed case: one target already pinned Strix for this attempt before the
        // matrix is first built. The matrix must report the whole dependency set,
        // not just what the first URL says.
        let (_mixed_root, mixed_db) = temp_database("phase2-matrix-mixed");
        seed_scan(&mixed_db, "agent-scan", "scanning");
        seed_attempt_row(&mixed_db, 1, "initial");
        persist_agent_execution_plan(
            &mixed_db,
            "agent-scan",
            1,
            &urls[1],
            &AgentExecutionPlan {
                backend: AgentBackendKind::Strix,
                ..test_plan_for("standard", &urls[1])
            },
        ).unwrap();
        let rebuilt = plan_scan_backends(&mixed_db, "agent-scan", 1, &urls, &native, true).unwrap();
        assert_eq!(rebuilt.targets[1].backend, AgentBackendKind::Strix);
        assert_eq!(rebuilt.targets[0].backend, AgentBackendKind::Native);
        assert!(rebuilt.requires_strix && rebuilt.requires_docker);
        let blocked: Vec<&str> = rebuilt
            .targets
            .iter()
            .filter(|target| target.backend == AgentBackendKind::Strix)
            .map(|target| target.url.as_str())
            .collect();
        assert_eq!(blocked, vec!["https://b.example.invalid"]);
    }

    /// Phase 2 §3.2: a settings edit in the middle of an attempt cannot move a target
    /// that the frozen matrix already classified.
    #[test]
    fn backend_matrix_is_frozen_for_the_attempt() {
        let (_root, db_path) = temp_database("phase2-matrix-frozen");
        seed_scan(&db_path, "agent-scan", "scanning");
        seed_attempt_row(&db_path, 1, "initial");
        let urls = vec!["https://a.example.invalid".to_string()];
        let matrix = plan_scan_backends(
            &db_path,
            "agent-scan",
            1,
            &urls,
            &serde_json::json!({"agentBackendPolicy": "native"}),
            true,
        ).unwrap();
        assert_eq!(matrix.targets[0].backend, AgentBackendKind::Native);
        // The user switches the policy to Strix while the attempt is running.
        let (backend, reason) = agent_backend_choice(
            &db_path,
            "agent-scan",
            1,
            &urls[0],
            &serde_json::json!({"agentBackendPolicy": "strix"}),
            true,
        );
        assert_eq!(
            backend,
            AgentBackendKind::Native,
            "the frozen matrix must win over today's settings: {reason}"
        );
        // A different attempt is a different matrix and does follow the new policy.
        seed_attempt_row(&db_path, 2, "fresh");
        assert_eq!(
            agent_select_backend(
                &db_path,
                "agent-scan",
                2,
                &urls[0],
                &serde_json::json!({"agentBackendPolicy": "strix"}),
                true
            ),
            AgentBackendKind::Strix
        );
    }

    /// Phase 2 §2.3 case 3: a continuation inherits the parent's frozen backend even
    /// when the settings now say otherwise.
    #[test]
    fn resume_inherits_the_parents_backend_and_never_the_new_settings() {
        let (_root, db_path) = temp_database("phase2-resume-backend");
        seed_scan(&db_path, "agent-scan", "scanning");
        seed_attempt_row(&db_path, 1, "initial");
        seed_attempt_row(&db_path, 2, "resume");
        let url = "https://app.example.invalid";
        persist_agent_execution_plan(
            &db_path,
            "agent-scan",
            1,
            url,
            &AgentExecutionPlan {
                backend: AgentBackendKind::Native,
                ..test_plan_for("standard", url)
            },
        ).unwrap();
        assert_eq!(
            agent_select_backend(&db_path, "agent-scan", 2, url, &serde_json::json!({"agentBackendPolicy":"strix"}), true),
            AgentBackendKind::Native,
            "a continuation cannot change backend mid-chain"
        );
    }

    /// Phase 2 §2.3 case 4: a continuation with no parent plan stops as
    /// `resume_incompatible` before spending a model or target request.
    #[test]
    fn resume_without_a_parent_plan_spends_nothing() {
        let mut harness = agent_harness(
            "phase2-no-parent-plan",
            mock_site,
            vec![AgentIdentity::anonymous()],
        );
        let url = harness.context.target_url.clone();
        seed_attempt_row(&harness.db_path, 1, "initial");
        seed_attempt_row(&harness.db_path, 2, "resume");
        harness.context.attempt_number = 2;
        harness.context.resume = true;
        let outcome = NativeAgentBackend.execute(&harness.context);
        assert_eq!(outcome.terminal_status(), "resume_incompatible", "{:?}", outcome.detail());
        assert!(harness.site_seen.lock().unwrap().is_empty(), "the target was never touched");
        assert!(harness.model_seen.lock().unwrap().is_empty(), "the model was never asked");
        assert_eq!(
            NativeAgentState::read(&harness.db_path, "agent-scan", &url)
                .map(|state| state.attempt_number),
            None,
            "no state may be created by a refused continuation"
        );
    }

    /// Phase 2 §2.3 case 5: fresh means nothing carries over — not tokens, queue,
    /// terminal reason, no-progress streak or exhausted contracts.
    #[test]
    fn fresh_attempt_inherits_nothing_from_its_predecessor() {
        let (_root, db_path) = temp_database("phase2-fresh-nothing");
        seed_scan(&db_path, "agent-scan", "scanning");
        seed_attempt_row(&db_path, 1, "initial");
        seed_attempt_row(&db_path, 2, "fresh");
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
        parent.target_requests = 7;
        parent.discovery_rounds = 3;
        parent.turns = 9;
        parent.no_progress_streak = 4;
        parent.terminal_reason = "连续多轮没有新增证据".to_string();
        parent.exhausted_contract_keys = vec!["idor|/api/orders".to_string()];
        parent.contract_outcomes = vec![("idor|/api/orders".to_string(), "exhausted".to_string())];
        parent.token_usage.total_tokens = 120_000;
        parent.covered_families = vec!["authorization".to_string()];
        parent.coverage_evidence = vec![CoverageEvidence {
            id: "cov-0001".to_string(),
            family: "authorization".to_string(),
            evidence_kind: "request".to_string(),
            result: "covered".to_string(),
            request_record_ids: vec!["req-0001".to_string()],
            ..Default::default()
        }];
        parent.persist(&db_path, "agent-scan", &context.target_url).unwrap();

        let mut next = context.clone();
        next.attempt_number = 2;
        next.resume = false;
        let started = NativeAgentState::for_attempt(&db_path, "agent-scan", &next, &evidence_hash, &plan_hash)
            .expect("a fresh attempt always starts");
        assert_eq!(started.resume_kind, AgentResumeKind::Fresh);
        assert_eq!(started.parent_attempt_number, None, "fresh records no parent state");
        assert_eq!(started.target_requests, 0);
        assert_eq!(started.discovery_rounds, 0);
        assert_eq!(started.turns, 0);
        assert_eq!(started.no_progress_streak, 0);
        assert!(started.terminal_reason.is_empty());
        assert!(started.exhausted_contract_keys.is_empty());
        assert!(started.contract_outcomes.is_empty());
        assert!(started.covered_families.is_empty());
        assert!(started.coverage_evidence.is_empty());
        assert_eq!(started.token_usage.total_tokens, 0);
        assert!(
            started.pending_queue.is_empty(),
            "a fresh attempt inherits no queue; the loop rebuilds it from the plan"
        );
        // §2.2: switching the backend is never achieved by destroying history. Only
        // the live working row is reset; both attempts keep their own plan rows.
        assert_eq!(started.attempt_number, 2);
        let connection = db::open(&db_path).unwrap();
        let attempts: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sentinel_scan_attempts WHERE scan_id='agent-scan'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(attempts, 2, "both attempt rows survive");
        assert_eq!(
            NativeAgentState::read(&db_path, "agent-scan", &context.target_url)
                .map(|state| state.attempt_number),
            None,
            "only the live working row is reset; it is rewritten when the attempt commits"
        );
    }
