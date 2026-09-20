    /// Case 15 (整改后语义): 暂停发生在 attempt 2，续跑发生在 attempt 3 并显式继承
    /// attempt 2，重新执行发生在 attempt 4 且必须从零预算开始。
    #[test]
    fn e2e_pause_resume_and_reexecute_keep_the_budget() {
        let harness = agent_harness("e2e-pause", mock_site, vec![AgentIdentity::anonymous()]);
        let AgentHarness {
            db_path,
            mut context,
            site_seen,
            ..
        } = harness;
        let target_url = context.target_url.clone();
        let evidence_hash = agent_stable_hash(&context.evidence);
        let plan_hash = context.execution_plan.hash();
        seed_attempt_row(&db_path, 1, "initial");
        seed_attempt_row(&db_path, 2, "resume");
        let mut parent = NativeAgentState::fresh(
            1,
            AgentBackendKind::Native,
            &evidence_hash,
            &plan_hash,
            vec!["family:authorization".to_string(), "family:business_flow".to_string()],
        );
        parent.target_requests = 5;
        parent.observed_requests = vec![trace_of("GET", "/api/orders", "authorization", "")];
        parent.persist(&db_path, "agent-scan", &target_url).unwrap();

        // The user pauses while attempt 2 is being prepared: the inherited ledger
        // survives and nothing settles.
        context.attempt_number = 2;
        context.resume = true;
        set_scan_status(&db_path, "pausing");
        let outcome = NativeAgentBackend.execute(&context);
        assert!(matches!(outcome, AgentTargetOutcome::Cancelled));
        let paused = NativeAgentState::read(&db_path, "agent-scan", &target_url).unwrap();
        assert_eq!(paused.attempt_number, 2);
        assert_eq!(paused.parent_attempt_number, Some(1));
        assert_eq!(paused.target_requests, 5, "a pause must not lose the spend");
        assert!(
            paused.terminal_reason.is_empty(),
            "a pause must stay resumable: {}",
            paused.terminal_reason
        );
        assert_eq!(site_seen.lock().unwrap().len(), 0);
        // The runtime row is written by the one terminal-outcome path; a pause
        // must leave it open, not settled (§11).
        record_runtime_terminal_facts(&db_path, "agent-scan", &context.route, &outcome);
        let paused_run = read_run_row(&db_path);
        assert_eq!(
            paused_run.status, "paused",
            "the runtime row must not settle on a pause"
        );

        // Continuing creates attempt 3, which inherits attempt 2 explicitly.
        set_scan_status(&db_path, "scanning");
        seed_attempt_row(&db_path, 3, "resume");
        let (model_port, _seen, _stop) = spawn_model(vec![model_round(
            &[(
                "replay_http",
                serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{target_url}/api/cart"),"family":"business_flow"}),
            )],
            500,
        )]);
        context.environment.api_base = format!("http://127.0.0.1:{model_port}/v1");
        context.attempt_number = 3;
        let outcome = NativeAgentBackend.execute(&context);
        assert!(
            matches!(outcome, AgentTargetOutcome::Incomplete(_)),
            "{:?}",
            outcome.detail()
        );
        let resumed = NativeAgentState::read(&db_path, "agent-scan", &target_url).unwrap();
        assert_eq!(resumed.parent_attempt_number, Some(2));
        assert_eq!(
            resumed.target_requests, 6,
            "the resumed run must build on the spent budget"
        );
        assert_eq!(
            site_seen.lock().unwrap().len(),
            1,
            "only the new request may reach the target"
        );
        assert!(resumed.covered_families.contains(&"business_flow".to_string()));
        assert!(
            !resumed.covered_families.contains(&"authorization".to_string()),
            "the inherited anonymous request still cannot cover authorization: {:?}",
            resumed.covered_families
        );
        assert!(
            resumed
                .coverage_evidence
                .iter()
                .any(|entry| entry.family == "authorization" && entry.result == "partial"),
            "the inherited partial work must survive into the child attempt"
        );

        // 重新执行: a new attempt starts from a clean ledger on purpose.
        seed_attempt_row(&db_path, 4, "fresh");
        let (model_port, _seen, _stop) = spawn_model(vec![model_round(
            &[(
                "replay_http",
                serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{target_url}/api/cart"),"family":"business_flow"}),
            )],
            500,
        )]);
        context.environment.api_base = format!("http://127.0.0.1:{model_port}/v1");
        context.attempt_number = 4;
        context.resume = false;
        NativeAgentBackend.execute(&context);
        let retried = NativeAgentState::read(&db_path, "agent-scan", &target_url).unwrap();
        assert_eq!(retried.attempt_number, 4);
        assert_eq!(retried.parent_attempt_number, None, "fresh inherits nothing");
        assert_eq!(retried.target_requests, 1, "a new attempt gets a fresh budget");
        assert!(retried.pending_queue.contains(&"family:authorization".to_string()));
    }

    /// A local deployment must keep working through the LLM hook that makes
    /// cancellation real: the hook forwards to the configured server and leaves its
    /// own usage ledger beside the target.
    #[test]
    fn e2e_local_deployment_runs_through_the_cancellable_hook() {
        let mut harness = agent_harness(
            "e2e-local-hook",
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
                model_round(&[("replay_http", replay("session-a"))], 900),
                model_round(&[("replay_http", replay("anonymous"))], 900),
                model_round(
                    &[(
                        "finish_target",
                        serde_json::json!({
                            "coverage": closing_ledger(&["authorization"]),
                            "stopReason":"本地模型收口"
                        }),
                    )],
                    900,
                ),
            ],
        );
        harness.context.environment.deployment = "local".to_string();
        let target_dir = harness.context.target_dir.clone();
        let outcome = NativeAgentBackend.execute(&harness.context);
        assert!(
            matches!(outcome, AgentTargetOutcome::Completed(_)),
            "{:?}",
            outcome.detail()
        );
        assert_eq!(
            harness.model_seen.lock().unwrap().len(),
            3,
            "the hook forwards every round to the configured server"
        );
        assert!(
            target_dir.join("llm-hook.jsonl").is_file(),
            "the run went through the hook"
        );
        assert_eq!(
            harness.site_seen.lock().unwrap().len(),
            2,
            "both sides of the identity contrast reached the target"
        );
    }

    /// A stand-in for the bundled CDP probe: reads the scripted input on stdin and
    /// answers with a runtime capture, so the browser path is testable without a
    /// browser installed here.
    fn scripted_cdp_helper(root: &Path, available: bool) -> PathBuf {
        let path = root.join("fake_runtime_probe.cjs");
        let script = if available {
            r#"
const input = JSON.parse(require("fs").readFileSync(0, "utf8"));
const origin = new URL(String(input.url || "http://127.0.0.1")).origin;
const out = { available: true, actions: [], requests: [], stopReason: "scripted", captureStatus: "complete" };
if (input.targetAction) {
  out.actions = [{ id: "action-1", label: String(input.targetAction), stateChanged: true, afterUrl: origin + "/orders" }];
  out.requests = [
    { url: origin + "/api/orders?page=2", method: "GET", status: 200, actionId: "action-1", resourceType: "xhr" },
    { url: "https://cdn.example.invalid/static/app.js", method: "GET", status: 200, actionId: "navigation", resourceType: "script" },
    { url: "https://tracker.example.com/collect", method: "POST", status: 204, actionId: "action-1", resourceType: "xhr" },
    { url: origin + "/legacy/report", method: "GET", status: 301, actionId: "action-1", resourceType: "document", redirectedTo: "https://other-host.example.org/legacy/report" }
  ];
}
process.stdout.write(JSON.stringify(out));
"#
        } else {
            r#"
JSON.parse(require("fs").readFileSync(0, "utf8"));
process.stdout.write(JSON.stringify({ available: false, errors: ["no browser binary"], actions: [], requests: [] }));
"#
        };
        fs::write(&path, script).unwrap();
        path
    }

    fn browser_runtime(helper: PathBuf) -> AgentBrowserRuntime {
        AgentBrowserRuntime {
            helper,
            runtime_path: std::env::var_os("PATH").unwrap_or_default(),
            no_proxy: "127.0.0.1,localhost".to_string(),
        }
    }

    /// `browser_action` performs the located control in a real browser session and
    /// reports only the requests that step produced — counted against the target
    /// budget, because the target really served them.
    #[test]
    fn browser_action_drives_the_probe_and_reports_its_delta() {
        let harness = agent_harness("agent-browser-act", mock_site, vec![AgentIdentity::anonymous()]);
        let AgentHarness {
            root,
            mut context,
            site_seen,
            ..
        } = harness;
        context.browser = Some(browser_runtime(scripted_cdp_helper(&root, true)));
        let mut runtime = AgentToolRuntime::default();
        let report = agent_execute_tool(
            &context,
            &mut runtime,
            "browser_action",
            &serde_json::json!({"identity":"anonymous","actionKey":"open-orders","family":"business_flow"}),
        ).model_view;
        assert_eq!(
            report.get("browserDriven").and_then(JsonValue::as_bool),
            Some(true),
            "{report}"
        );
        let delta = report
            .get("networkDelta")
            .and_then(JsonValue::as_array)
            .cloned()
            .unwrap_or_default();
        assert_eq!(delta.len(), 1, "only the action's own request: {report}");
        assert_eq!(value_first(&delta[0], &["path"]), "/api/orders");
        assert_eq!(
            delta[0]
                .get("parameters")
                .and_then(JsonValue::as_array)
                .map(|rows| rows.len()),
            Some(1)
        );
        assert_eq!(runtime.target_requests, 1, "browser traffic spent the budget");
        assert!(runtime.touched("GET", "/api/orders"));
        assert!(runtime.families.contains("business_flow"));
        assert!(
            runtime.queue_key_satisfied(&context, "api:GET|/api/orders"),
            "a browser-observed endpoint closes the queue"
        );
        assert_eq!(
            site_seen.lock().unwrap().len(),
            0,
            "the tool itself makes no extra HTTP request"
        );
    }

    /// With no browser runtime the tool must say it did not drive a browser,
    /// and must label the capture-derived APIs as attribution, not observation.
    #[test]
    fn browser_action_says_when_it_could_not_drive_a_browser() {
        let harness =
            agent_harness("agent-browser-none", mock_site, vec![AgentIdentity::anonymous()]);
        let AgentHarness {
            root,
            mut context,
            site_seen,
            ..
        } = harness;
        let mut runtime = AgentToolRuntime::default();
        let report = agent_execute_tool(
            &context,
            &mut runtime,
            "browser_action",
            &serde_json::json!({"identity":"anonymous","actionKey":"open-orders","family":"business_flow"}),
        ).model_view;
        assert_eq!(
            report.get("browserDriven").and_then(JsonValue::as_bool),
            Some(false),
            "{report}"
        );
        assert_eq!(value_first(&report, &["browserAttempt"]), "no_browser_runtime");
        assert!(report.get("attributedApis").is_some());
        assert!(report.get("newApiDelta").is_none(), "attribution is not observation");

        // A probe that reports no browser also degrades, and says what happened.
        context.browser = Some(browser_runtime(scripted_cdp_helper(&root, false)));
        let mut retry = AgentToolRuntime::default();
        let report = agent_execute_tool(
            &context,
            &mut retry,
            "browser_action",
            &serde_json::json!({"identity":"anonymous","actionKey":"open-orders","family":"business_flow"}),
        ).model_view;
        assert_eq!(
            report.get("browserDriven").and_then(JsonValue::as_bool),
            Some(false),
            "{report}"
        );
        assert_eq!(value_first(&report, &["browserAttempt"]), "no_browser_runtime");
        assert_eq!(
            site_seen.lock().unwrap().len(),
            2,
            "each degraded call contributes exactly one direct navigation"
        );
    }

    /// Case 17: the runtime store mirrors the loop while it runs. The pause is
    /// flipped from inside the second model round, so round one is already fully
    /// audited in `agent_events`/`tool_invocations`/`agent_snapshots` before the
    /// loop stops, and the refused round-two call is audited too.
    #[test]
    fn live_run_facts_land_before_the_terminal_close() {
        let harness = agent_harness("e2e-live-events", mock_site, vec![AgentIdentity::anonymous()]);
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
        ).unwrap();
        let ledger = runtime_open_run(&db_path, "agent-scan", &context.route).expect("run row");
        context.run = Some(ledger.clone());

        let pause_db = db_path.clone();
        let served = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let served_counter = served.clone();
        let replay_url = target_url.clone();
        let (model_port, _seen, _stop) = spawn_endpoint(std::sync::Arc::new(move |_request| {
            if served_counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= 1 {
                if let Ok(connection) = db::open(&pause_db) {
                    let _ = connection.execute(
                        "UPDATE sentinel_scans SET status='pausing' WHERE id='agent-scan'",
                        [],
                    );
                }
            }
            (
                200,
                "application/json",
                model_round(
                    &[(
                        "replay_http",
                        serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{replay_url}/api/orders"),"family":"authorization"}),
                    )],
                    500,
                ),
            )
        }));
        context.environment.api_base = format!("http://127.0.0.1:{model_port}/v1");

        let outcome = NativeAgentBackend.execute(&context);
        assert!(
            matches!(outcome, AgentTargetOutcome::Cancelled),
            "{:?}",
            outcome.detail()
        );
        assert_eq!(site_seen.lock().unwrap().len(), 1, "the pause stops later rounds");

        let connection = db::open(&db_path).unwrap();
        let rounds: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM agent_events WHERE run_id=?1 AND event_type='model_round_completed'",
                [&ledger.run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(rounds, 2, "each served round appends one event");
        let completed: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM tool_invocations WHERE run_id=?1 AND tool_name='replay_http' AND status='completed'",
                [&ledger.run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(completed, 1, "the executed call is audited live");
        let refused: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM tool_invocations WHERE run_id=?1 AND status='refused' AND error_class='cancelled'",
                [&ledger.run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(refused, 1, "a refusal is auditable too");
        let snapshot: String = connection
            .query_row(
                "SELECT snapshot_json FROM agent_snapshots WHERE run_id=?1",
                [&ledger.run_id],
                |row| row.get(0),
            )
            .unwrap();
        let snapshot = json(snapshot);
        assert_eq!(
            snapshot.get("targetRequests").and_then(JsonValue::as_i64),
            Some(1)
        );
        assert_eq!(snapshot.get("terminal").map(|v| v.is_null()), Some(true));
        let status: String = connection
            .query_row(
                "SELECT status FROM agent_runs WHERE id=?1",
                [&ledger.run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "running", "a pause must not settle the run row");
    }

    // ------------------------------------------------------------------
    // Remediation §13.4 — one scope gate for HTTP, browser entry and CDP output
    // ------------------------------------------------------------------

    /// A browser entry outside the frozen origins must be refused before Chrome is
    /// ever started.
    #[test]
    fn browser_entry_outside_frozen_scope_is_refused() {
        let harness =
            agent_harness("remediation-entry", mock_site, vec![AgentIdentity::anonymous()]);
        let AgentHarness {
            root,
            mut context,
            site_seen,
            ..
        } = harness;
        context.browser = Some(browser_runtime(scripted_cdp_helper(&root, true)));
        // The evidence action points at a host that was never approved.
        context.evidence["investigation"]["actions"] = serde_json::json!([
            {"key": "external-panel", "type": "link", "label": "第三方客服", "url": "https://vendor-support.example.com/chat"}
        ]);
        let mut runtime = AgentToolRuntime::default();
        let report = agent_execute_tool(
            &context,
            &mut runtime,
            "browser_action",
            &serde_json::json!({"identity":"anonymous","actionKey":"external-panel"}),
        ).model_view;
        assert_eq!(value_first(&report, &["code"]), "scope_denied", "{report}");
        assert_eq!(runtime.target_requests, 0);
        assert_eq!(site_seen.lock().unwrap().len(), 0);
    }

    /// Requests a browser observed on a third party must never become formal APIs,
    /// even when the page itself is authorized.
    #[test]
    fn cdp_observed_third_party_never_enters_the_ledger() {
        let harness =
            agent_harness("remediation-cdp", mock_site, vec![AgentIdentity::anonymous()]);
        let AgentHarness {
            root,
            mut context,
            site_seen,
            ..
        } = harness;
        context.browser = Some(browser_runtime(scripted_cdp_helper(&root, true)));
        let mut runtime = AgentToolRuntime::default();
        let report = agent_execute_tool(
            &context,
            &mut runtime,
            "browser_action",
            &serde_json::json!({"identity":"anonymous","actionKey":"open-orders","family":"business_flow"}),
        ).model_view;
        let rejected = report
            .get("rejectedObservations")
            .and_then(JsonValue::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(
            rejected.iter().any(|row| value_first(row, &["host"]) == "tracker.example.com"),
            "the third-party XHR must be audited and refused: {report}"
        );
        assert!(!runtime.touched("POST", "/collect"), "no third-party endpoint in the ledger");
        assert_eq!(
            runtime.target_requests, 1,
            "only the authorized business API counts as spend"
        );
        assert_eq!(site_seen.lock().unwrap().len(), 0);
    }

    /// A cross-origin redirect out of the frozen scope stops that action; an
    /// identity-provider jump is recorded but never auto-tested.
    #[test]
    fn cross_origin_redirects_stop_the_action_and_idp_jumps_are_only_observed() {
        let (_root, db_path) = temp_database("remediation-redirect");
        seed_scan(&db_path, "agent-scan", "scanning");
        let port = spawn_redirect_server(
            "https://unrelated-host.example.org/stolen",
            "https://login.idp.example.com/o/authorize?client_id=abc&redirect_uri=x&response_type=code",
        );
        let base = format!("http://127.0.0.1:{port}");
        let context = test_context(&db_path, &base, vec![AgentIdentity::anonymous()]);
        let mut runtime = AgentToolRuntime::default();
        let hijack = agent_execute_tool(
            &context,
            &mut runtime,
            "replay_http",
            &serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{base}/api/hijack"),"family":"authorization"}),
        ).model_view;
        assert_eq!(value_first(&hijack, &["redirectCode"]), "redirect_out_of_scope", "{hijack}");
        assert!(
            !runtime.families.contains("authorization"),
            "a refused redirect cannot credit coverage"
        );

        let sso = agent_execute_tool(
            &context,
            &mut runtime,
            "replay_http",
            &serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{base}/api/sso"),"family":"authentication_session"}),
        ).model_view;
        assert_eq!(value_first(&sso, &["redirectCode"]), "identity_provider_redirect", "{sso}");
        assert!(
            !runtime.families.contains("authentication_session"),
            "an identity-provider jump is observation, not coverage"
        );
    }

    /// §13.5: a tool result that carries a credential must leave nothing of it in
    /// the model view, and the raw copy must be a different file with different
    /// content — the split the whole redaction contract depends on.
    #[test]
    fn tool_results_are_redacted_before_reaching_the_model() {
        const LEAKY_JWT: &str =
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.6-6Sp7A_qFVoXcCwZDmenP";
        let site = move |request: String| {
            if request.contains("/api/whoami") {
                return (
                    200,
                    "application/json",
                    format!(
                        "{{\"token\":\"{LEAKY_JWT}\",\"authorization\":\"Bearer {LEAKY_JWT}\",\"email\":\"ops@example.com\",\"phone\":\"13800138000\",\"role\":\"admin\",\"orders\":12}}"
                    ),
                );
            }
            (404, "text/plain", "not found".to_string())
        };
        let harness = agent_harness("redact-view", site, vec![AgentIdentity::anonymous()]);
        let target_url = harness.context.target_url.clone();
        let mut runtime = AgentToolRuntime::default();
        let result = agent_execute_tool(
            &harness.context,
            &mut runtime,
            "replay_http",
            &serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{target_url}/api/whoami"),"family":"authentication_session"}),
        );
        let view = result.model_view.to_string();
        for secret in [LEAKY_JWT, "ops@example.com", "13800138000"] {
            assert!(
                !view.contains(secret),
                "the model view leaked {secret}: {view}"
            );
        }
        assert!(
            view.contains("<redacted:"),
            "markers must stay comparable: {view}"
        );
        // What the contract says the model still needs: structure, not payloads.
        assert_eq!(result.model_view["status"].as_i64(), Some(200));
        assert_eq!(result.model_view["contentType"].as_str(), Some("application/json"));
        assert!(view.contains("\"role\""), "field paths survive: {view}");
        assert!(view.contains("\"bodySha256\""), "hashes survive: {view}");
        assert!(view.contains("\"truncated\""), "length facts survive: {view}");
        assert!(!result.artifact_id.is_empty());
        assert_eq!(
            result.model_view["rawArtifactId"].as_str(),
            Some(result.artifact_id.as_str())
        );

        let directory = harness.context.target_dir.join("agent-http");
        let metadata = fs::read_to_string(directory.join(&result.artifact_id)).unwrap();
        assert!(!metadata.contains(LEAKY_JWT), "raw bytes leaked into metadata");
        assert!(!metadata.contains("rawBody"));
        let raw = fs::read_to_string(directory.join("0001.body")).unwrap();
        assert!(raw.contains(LEAKY_JWT), "the audit copy must stay complete");
        assert_ne!(raw, view, "raw evidence and model view cannot be the same text");
        // The request metadata keeps names, never values, and a bodyless GET has
        // no raw request payload at all.
        let stored: JsonValue = json(metadata);
        let names = stored["request"]["headerNames"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert!(
            names.iter().all(|name| !name
                .as_str()
                .unwrap_or_default()
                .contains("Bearer")),
            "header values must not be persisted as metadata: {stored}"
        );
        assert_eq!(stored["request"]["bodyBytes"], JsonValue::Null);
        assert!(!directory.join("0001.request").exists());
        let permissions = {
            use std::os::unix::fs::PermissionsExt;
            fs::metadata(directory.join("0001.body")).unwrap().permissions().mode() & 0o777
        };
        assert_eq!(permissions, 0o600, "raw evidence is readable only by this user");
    }

    /// §14.10: the loop must never hand a credential to the model, whichever
    /// provider class is configured — so what the mock endpoint actually received
    /// is the thing under test, not what the tool returned.
    #[test]
    fn e2e_model_history_never_carries_a_raw_credential() {
        const LEAKY_JWT: &str =
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.6-6Sp7A_qFVoXcCwZDmenP";
        let mut harness = agent_harness(
            "e2e-redact",
            move |request: String| {
                if request.contains("/api/orders") {
                    return (
                        200,
                        "application/json",
                        format!(
                            "{{\"sessionToken\":\"{LEAKY_JWT}\",\"owner\":\"ops@example.com\",\"items\":[{{\"id\":1}}]}}"
                        ),
                    );
                }
                (404, "text/plain", "not found".to_string())
            },
            vec![AgentIdentity::anonymous()],
        );
        let target_url = harness.context.target_url.clone();
        retarget_model(
            &mut harness,
            vec![
                model_round(
                    &[(
                        "replay_http",
                        serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{target_url}/api/orders"),"family":"authorization"}),
                    )],
                    900,
                ),
                model_round(&[], 1_100),
            ],
        );
        let outcome = NativeAgentBackend.execute(&harness.context);
        assert!(
            matches!(outcome, AgentTargetOutcome::Incomplete(_) | AgentTargetOutcome::Completed(_)),
            "unexpected outcome: {:?}",
            outcome.detail()
        );
        let prompts = harness.model_seen.lock().unwrap().join("\n");
        assert!(
            prompts.contains("POST /v1/chat/completions"),
            "the second round must have run"
        );
        assert!(
            !prompts.contains(LEAKY_JWT),
            "a raw session token reached the model provider"
        );
        assert!(!prompts.contains("ops@example.com"));
        assert!(
            prompts.contains("<redacted:"),
            "the tool result the model saw must be the marked view"
        );
        // The credential is still recoverable for audit, from the raw side only.
        let raw = fs::read_to_string(
            harness
                .context
                .target_dir
                .join("agent-http")
                .join("0001.body"),
        )
        .unwrap();
        assert!(raw.contains(LEAKY_JWT));
    }
