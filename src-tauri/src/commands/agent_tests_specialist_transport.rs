struct RealSpecialistTransport(bool);
impl RealSpecialistTransport {
    fn enter() -> Self {
        Self(REAL_CHILD_TRANSPORT.with(|flag| flag.replace(true)))
    }
}
impl Drop for RealSpecialistTransport {
    fn drop(&mut self) {
        REAL_CHILD_TRANSPORT.with(|flag| flag.set(self.0));
    }
}

fn specialist_original_gap_fixture(endpoint: &str) -> (std::path::PathBuf, AgentRunContext, MultiAgentSession) {
    let (root, mut context, _) = specialist_original_context("specialist-http",endpoint);
    let _real = RealSpecialistTransport::enter();
    let mut session = multi_agent_prepare(&mut context).unwrap();
    multi_agent_finish_execution(
        &context,
        &mut session,
        &AgentTargetOutcome::incomplete("fixture"),
    )
    .unwrap();
    let connection = db::open(&context.db_path).unwrap();
    seal_specialist_original_gap_review(&connection, &session.lease, &context);
    (root, context, session)
}

fn assert_specialist_usage_pending(connection: &rusqlite::Connection, role: &str, tokens: i64) {
    let row: (String,String,String,i64,i64,String) = connection.query_row(
        "SELECT a.state,r.status,a.failure_class,a.reserved_tokens,a.reserved_requests,a.budget_settled_at FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE a.role=?1 ORDER BY a.rowid DESC LIMIT 1",
        [role], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).unwrap();
    assert_eq!(
        row,
        (
            "paused".into(),
            "paused".into(),
            "child_usage_reconciliation_required".into(),
            tokens,
            1,
            String::new()
        )
    );
    let active: i64 = connection.query_row("SELECT COUNT(*) FROM agent_capability_leases c JOIN agent_runs r ON r.id=c.child_run_id WHERE r.role=?1 AND c.revoked_at=''", [role], |r| r.get(0)).unwrap();
    assert_eq!(active, 0);
    let lanes: i64 = connection.query_row("SELECT COUNT(*) FROM agent_lane_leases l JOIN agent_assignments a ON a.id=l.assignment_id WHERE a.role=?1", [role], |r| r.get(0)).unwrap();
    assert_eq!(lanes, 1, "unsettled lane must not be silently reused");
}

#[test]
fn specialist_transport_gap_503_is_one_request_and_retains_unknown_usage() {
    let (port, seen, _stop) = specialist_original_endpoint(std::sync::Arc::new(|_| {
        (
            503,
            "application/json",
            "{\"error\":\"unavailable\"}".into(),
        )
    }));
    let (root, mut context, session) = specialist_original_gap_fixture(&format!("http://127.0.0.1:{port}/v1"));
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let _real = RealSpecialistTransport::enter();
    let error = multi_agent_investigate_review_gap(
        &context,
        &session,
        "candidate-gap",
        2,
        &serde_json::json!(["missing control"]),
    )
    .unwrap_err();
    assert!(!error.is_empty());
    assert_eq!(
        specialist_child_call_count(&seen),
        3,
        "one child reservation cannot fund implicit provider retries"
    );
    let connection = db::open(&context.db_path).unwrap();
    assert_specialist_usage_pending(&connection, "deep_investigator", 4_000);
    assert!(multi_agent_investigate_review_gap(
        &context,
        &session,
        "candidate-gap",
        2,
        &serde_json::json!(["missing control"])
    )
    .is_err());
    assert_eq!(
        specialist_child_call_count(&seen),
        3,
        "reentry cannot retry an unknown outcome"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn specialist_transport_gap_settlement_error_retains_reservation_and_original_error() {
    let (port, seen, _stop) = specialist_original_endpoint(std::sync::Arc::new(|_| {
        (200, "application/json", proposal_model_response("{}"))
    }));
    let (root, mut context, session) = specialist_original_gap_fixture(&format!("http://127.0.0.1:{port}/v1"));
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let connection = db::open(&context.db_path).unwrap();
    connection.execute_batch("CREATE TRIGGER refuse_specialist_settlement BEFORE UPDATE OF spent_tokens ON agent_budget_ledger WHEN NEW.spent_tokens>OLD.spent_tokens BEGIN SELECT RAISE(ABORT,'injected_specialist_settlement'); END;").unwrap();
    let _real = RealSpecialistTransport::enter();
    let error = multi_agent_investigate_review_gap(
        &context,
        &session,
        "candidate-gap",
        2,
        &serde_json::json!(["missing control"]),
    )
    .unwrap_err();
    assert!(error.contains("injected_specialist_settlement"), "{error}");
    assert_eq!(specialist_child_call_count(&seen), 3);
    assert_specialist_usage_pending(&connection, "deep_investigator", 4_000);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn specialist_transport_mapper_failure_retains_usage_and_does_not_dispatch_executor() {
    let (port, seen, _stop) = specialist_original_mapper_failure_endpoint(std::sync::Arc::new(|_| {
        (
            401,
            "application/json",
            "{\"error\":\"unauthorized\"}".into(),
        )
    }));
    let (root, mut context, _) = specialist_original_context("mapper-http-failure", &format!("http://127.0.0.1:{port}/v1"));

    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let _real = RealSpecialistTransport::enter();
    assert!(multi_agent_prepare(&mut context).is_err());
    assert_eq!(specialist_child_call_count(&seen), 1);
    let connection = db::open(&context.db_path).unwrap();
    assert_specialist_usage_pending(&connection, "spa_api_mapper", 8_000);
    let executors: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM agent_runs WHERE role='web_executor'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(executors, 0);
    let _ = fs::remove_dir_all(root);
}

fn specialist_review_fixture(endpoint: &str) -> (std::path::PathBuf, AgentRunContext, MultiAgentSession) {
    let (root, mut context, _) = specialist_original_context("reviewer-http",endpoint);
    stage_agent_finding(
        &context,
        "web",
        "evidence",
        "specialist-review",
        "candidate",
        "info",
        &serde_json::json!({"fact":"fixture"}),
    )
    .unwrap();
    let _real = RealSpecialistTransport::enter();
    let mut session = multi_agent_prepare(&mut context).unwrap();
    multi_agent_finish_execution(
        &context,
        &mut session,
        &AgentTargetOutcome::incomplete("fixture"),
    )
    .unwrap();
    (root, context, session)
}

#[test]
fn specialist_transport_reviewer_failure_is_atomic_and_preserves_cleanup_errors() {
    for fail_cleanup in [false, true] {
        let (port, seen, _stop) = specialist_original_endpoint(std::sync::Arc::new(|_| {
            (
                503,
                "application/json",
                "{\"error\":\"unavailable\"}".into(),
            )
        }));
        let (root, mut context, mut session) = specialist_review_fixture(&format!("http://127.0.0.1:{port}/v1"));
        context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
        let connection = db::open(&context.db_path).unwrap();
        if fail_cleanup {
            connection.execute_batch("CREATE TRIGGER refuse_reviewer_pause BEFORE UPDATE OF state ON agent_assignments WHEN NEW.role='evidence_reviewer' AND NEW.state='paused' BEGIN SELECT RAISE(ABORT,'injected_reviewer_pause'); END;").unwrap();
        }
        let _real = RealSpecialistTransport::enter();
        let outcome = multi_agent_review(
            &context,
            &mut session,
            AgentTargetOutcome::incomplete("fixture"),
        );
        assert_eq!(specialist_child_call_count(&seen), 2);
        assert!(
            outcome.detail().contains("review_gate_failed"),
            "{outcome:?}"
        );
        let state: String = connection
            .query_row(
                "SELECT status FROM agent_review_requests ORDER BY rowid DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        if fail_cleanup {
            assert!(
                outcome.detail().contains("injected_reviewer_pause"),
                "{outcome:?}"
            );
            assert_eq!(
                state, "running",
                "request failure must roll back when child cleanup fails"
            );
        } else {
            assert_eq!(state, "failed");
            assert_specialist_usage_pending(&connection, "evidence_reviewer", 8_000);
            let again = multi_agent_review(
                &context,
                &mut session,
                AgentTargetOutcome::incomplete("fixture"),
            );
            assert!(!matches!(again, AgentTargetOutcome::Completed(_)));
            assert_eq!(
                specialist_child_call_count(&seen),
                2,
                "failed review cannot silently retry"
            );
        }
        let _ = fs::remove_dir_all(root);
    }
}

fn configure_specialist_identity(
    connection: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) {
    let project_id: i64 = connection
        .query_row(
            "SELECT project_id FROM sentinel_scans WHERE id=?1",
            [&lease.scan_id],
            |r| r.get(0),
        )
        .unwrap();
    connection.execute("INSERT INTO browser_auth_sessions(id,project_id,owner_scan_id,name,entry_url,status,session_json,expires_at) VALUES('specialist-identity',?1,?2,'Scoped',?3,'valid',?4,?5)",
        rusqlite::params![project_id,lease.scan_id,lease.target_key,serde_json::json!({"scopeHosts":["authorized.example.test"],"cookies":[{"name":"session","value":"specialist-credential-secret"}]}).to_string(),
            (chrono::Utc::now()+chrono::Duration::hours(8)).to_rfc3339()]).unwrap();
    connection
        .execute(
            "INSERT INTO sentinel_scan_contexts(scan_id,policy_json) VALUES(?1,?2) ON CONFLICT(scan_id) DO UPDATE SET policy_json=json_set(sentinel_scan_contexts.policy_json,'$.authSessionIds',json_extract(excluded.policy_json,'$.authSessionIds'))",
            rusqlite::params![
                lease.scan_id,
                serde_json::json!({"authSessionIds":["specialist-identity"]}).to_string()
            ],
        )
        .unwrap();
}

#[test]
fn specialist_transport_identity_failure_does_not_refund_unknown_usage() {
    let (port, seen, _stop) = specialist_original_endpoint(std::sync::Arc::new(|request| {
        if request.contains("SPA/API Mapper") {
            (
                200,
                "application/json",
                proposal_model_response("{\"summary\":\"mapped evidence\"}"),
            )
        } else {
            (
                503,
                "application/json",
                "{\"error\":\"unavailable\"}".into(),
            )
        }
    }));
    let (root, mut context, lease) = specialist_original_context("identity-http-failure", &format!("http://127.0.0.1:{port}/v1"));
    let connection = db::open(&context.db_path).unwrap();
    configure_specialist_identity(&connection, &lease);

    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let _real = RealSpecialistTransport::enter();
    assert!(multi_agent_prepare(&mut context).is_err());
    assert_eq!(specialist_child_call_count(&seen), 2);
    assert_specialist_usage_pending(&connection, "identity_session", 4_000);
    let spent: (i64, i64) = connection
        .query_row(
            "SELECT spent_tokens,spent_requests FROM agent_budget_ledger",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(spent, (20, 1), "only the known mapper call can be settled");
    assert!(!seen
        .lock()
        .unwrap()
        .join("\n")
        .contains("specialist-credential-secret"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn specialist_transport_four_real_roles_have_separate_calls_usage_and_mailbox() {
    let (port, seen, _stop) = specialist_original_endpoint(std::sync::Arc::new(|request| {
        let body: JsonValue =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        let system = body["messages"][0]["content"].as_str().unwrap();
        let response = if system.contains("Evidence Reviewer") {
            serde_json::json!({"verdict":"insufficient_evidence","reasonCodes":["missing_control"],"missingEvidence":["owner control"],"confidence":0.0,"summary":"Need verified owner control"})
        } else if system.contains("Deep Investigator") {
            let input: JsonValue =
                serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
            serde_json::json!({"summary":"Request a new approved control group","nextStep":"request_new_contract","gapCode":"missing_owner_control",
                "supportingFactRefs":[],"missingEvidence":input["reviewerMissingEvidence"],"prerequisites":["operator control group"],
                "proposedContracts":["new_attempt_control_group_request"],"expectedInformationGain":0.5,"impactCeiling":"medium",
                "estimatedCost":{"modelTokens":1000,"modelRequests":1,"targetRequests":3},"sideEffectClass":"read_only","overlapKeys":["owner-control"],
                "falsificationCondition":"owner access absent","stopCondition":"scope rejection"})
        } else {
            serde_json::json!({"summary":"Read supplied evidence only","observedAuthentication":["validated metadata"],
                "evidenceGaps":["no target identity comparison"],"authorizationProven":false})
        };
        (
            200,
            "application/json",
            proposal_model_response(&response.to_string()),
        )
    }));
    let (root, mut context, lease) = specialist_original_context("specialist-real-chain", &format!("http://127.0.0.1:{port}/v1"));
    let connection = db::open(&context.db_path).unwrap();
    configure_specialist_identity(&connection, &lease);
    let root_run_id = context.run.as_ref().unwrap().run_id.clone();

    stage_agent_finding(
        &context,
        "web",
        "evidence",
        "real-specialist-chain",
        "candidate",
        "info",
        &serde_json::json!({"fact":"fixture"}),
    )
    .unwrap();
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let _real = RealSpecialistTransport::enter();
    let mut session = multi_agent_prepare(&mut context).unwrap();
    multi_agent_finish_execution(
        &context,
        &mut session,
        &AgentTargetOutcome::incomplete("fixture"),
    )
    .unwrap();
    let outcome = multi_agent_review(
        &context,
        &mut session,
        AgentTargetOutcome::incomplete("fixture"),
    );
    assert!(
        matches!(outcome, AgentTargetOutcome::Incomplete(_)),
        "{outcome:?}"
    );
    assert!(
        outcome
            .detail()
            .contains("review_gate_insufficient_evidence"),
        "{outcome:?}"
    );
    assert_eq!(specialist_child_call_count(&seen), 4);
    assert_eq!(seen.lock().unwrap().len(), 9, "four independent children plus five actual Root SDK calls");
    assert_eq!(crate::agent_runtime::multi_agent::budget::balance(
        &connection, &root_run_id, Some(""), "model_requests",
    ).unwrap().consumed, 5, "Root invoices remain separate from the four child invoices");
    for role in [
        "spa_api_mapper",
        "identity_session",
        "evidence_reviewer",
        "deep_investigator",
    ] {
        let usage: (i64,i64,String) = connection.query_row("SELECT used_tokens,used_requests,status FROM agent_runs WHERE root_run_id=?1 AND role=?2", rusqlite::params![root_run_id,role], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(usage, (20, 1, "terminal".into()), "{role}");
    }
    let spent: (i64, i64) = connection
        .query_row(
            "SELECT spent_tokens,spent_requests FROM agent_budget_ledger",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(spent, (80, 4));
    let gaps: i64 = connection.query_row("SELECT COUNT(*) FROM agent_messages WHERE kind IN ('gap_proposed','proposal_assessed') AND acknowledged_at<>''", [], |r| r.get(0)).unwrap();
    assert_eq!(gaps, 2);
    assert!(!seen
        .lock()
        .unwrap()
        .join("\n")
        .contains("specialist-credential-secret"));
    let registered = crate::agent_runtime::multi_agent::budget::root::model::tick::local::schemas();
    let mut root_round = 0;
    for request in seen.lock().unwrap().iter() {
        let body: JsonValue =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        if body["messages"][0]["content"].as_str().unwrap().contains("You are the Root Coordinator") {
            root_round += 1;
            let tools = body["tools"].as_array().unwrap();
            assert_eq!(tools.len(), 4);
            for (tool, spec) in tools.iter().zip(&registered) {
                let spec = spec.as_function_spec();
                assert_eq!(tool["type"], spec["type"]);
                assert_eq!(tool["function"]["name"], spec["function"]["name"]);
                assert_eq!(tool["function"]["parameters"], spec["function"]["parameters"]);
            }
            let original: String = connection.query_row(
                "SELECT fact_json FROM agent_root_tick_receipts WHERE root_run_id=?1 AND round=?2 AND phase='request'",
                rusqlite::params![root_run_id,root_round], |r| r.get(0),
            ).unwrap();
            let original: JsonValue = serde_json::from_str(&original).unwrap();
            assert_eq!(original["request"]["request"]["tools"],body["tools"]);
        } else {
            assert!(body.get("tools").is_none_or(|tools| tools.as_array().is_some_and(Vec::is_empty)));
        }
    }
    assert_eq!(root_round,5);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn specialist_transport_reviewer_silent_cleanup_skip_rolls_back_request_failure() {
    for trigger in [
        "CREATE TRIGGER skip_cleanup BEFORE UPDATE OF state ON agent_assignments WHEN NEW.role='evidence_reviewer' AND NEW.state='paused' BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER skip_cleanup BEFORE UPDATE OF status ON agent_runs WHEN NEW.role='evidence_reviewer' AND NEW.status='paused' BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER skip_cleanup BEFORE UPDATE OF revoked_at ON agent_capability_leases WHEN OLD.revoked_at='' BEGIN SELECT RAISE(IGNORE); END;",
    ] {
        let (port, seen, _stop) = specialist_original_endpoint(std::sync::Arc::new(|_| (503,"application/json","{\"error\":\"unavailable\"}".into())));
        let (root, mut context, mut session) = specialist_review_fixture(&format!("http://127.0.0.1:{port}/v1"));
        let connection = db::open(&context.db_path).unwrap();
        connection.execute_batch(trigger).unwrap();
        context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
        let _real = RealSpecialistTransport::enter();
        let outcome = multi_agent_review(&context, &mut session, AgentTargetOutcome::incomplete("fixture"));
        assert_eq!(specialist_child_call_count(&seen), 2);
        assert!(outcome.detail().contains("child_failure_cleanup_postcondition"), "{trigger}: {outcome:?}");
        let state: (String,String,String,i64,i64) = connection.query_row(
            "SELECT q.status,a.state,r.status,a.reserved_tokens,a.reserved_requests FROM agent_review_requests q JOIN agent_assignments a ON a.child_run_id=q.reviewer_run_id JOIN agent_runs r ON r.id=a.child_run_id ORDER BY q.rowid DESC LIMIT 1",
            [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
        assert_eq!(state, ("running".into(),"running".into(),"running".into(),8_000,1), "cleanup must roll back as a unit");
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn specialist_transport_reviewer_settlement_failure_retains_received_usage() {
    let (port, seen, _stop) = specialist_original_endpoint(std::sync::Arc::new(|_| {
        (200, "application/json", proposal_model_response("{}"))
    }));
    let (root, mut context, mut session) = specialist_review_fixture(&format!("http://127.0.0.1:{port}/v1"));
    let connection = db::open(&context.db_path).unwrap();
    connection.execute_batch("CREATE TRIGGER refuse_review_settlement BEFORE UPDATE OF spent_tokens ON agent_budget_ledger WHEN NEW.spent_tokens>OLD.spent_tokens BEGIN SELECT RAISE(ABORT,'injected_review_settlement'); END;").unwrap();
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let _real = RealSpecialistTransport::enter();
    let outcome = multi_agent_review(
        &context,
        &mut session,
        AgentTargetOutcome::incomplete("fixture"),
    );
    assert!(
        outcome.detail().contains("injected_review_settlement"),
        "{outcome:?}"
    );
    assert_eq!(specialist_child_call_count(&seen), 2);
    assert_specialist_usage_pending(&connection, "evidence_reviewer", 8_000);
    let run_id: String = connection
        .query_row(
            "SELECT id FROM agent_runs WHERE role='evidence_reviewer'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let snapshot = crate::agent_runtime::checkpoint::RunState::from_json(
        &crate::agent_runtime::store::read_snapshot(&connection, &run_id)
            .unwrap()
            .unwrap()
            .snapshot,
    );
    assert_eq!((snapshot.used_tokens, snapshot.model_requests), (20, 1));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn specialist_transport_invalid_readonly_response_preserves_received_usage() {
    for tool_call in [false, true] {
        let (port, seen, _stop) = specialist_original_endpoint(std::sync::Arc::new(move |_| {
            let mut body: JsonValue =
                serde_json::from_str(&proposal_model_response(if tool_call {
                    "attempt tool"
                } else {
                    ""
                }))
                .unwrap();
            if tool_call {
                body["choices"][0]["message"]["tool_calls"] = serde_json::json!([{"id":"forbidden","type":"function","function":{"name":"http_request","arguments":"{}"}}]);
            }
            (200, "application/json", body.to_string())
        }));
        let (root, mut context, session) = specialist_original_gap_fixture(&format!("http://127.0.0.1:{port}/v1"));
        context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
        let _real = RealSpecialistTransport::enter();
        let error = multi_agent_investigate_review_gap(
            &context,
            &session,
            "candidate-gap",
            2,
            &serde_json::json!(["missing control"]),
        )
        .unwrap_err();
        assert!(
            error.contains(if tool_call {
                "unexpected_tool_calls"
            } else {
                "返回空结果"
            }),
            "{error}"
        );
        assert_eq!(specialist_child_call_count(&seen), 3);
        let connection = db::open(&context.db_path).unwrap();
        assert_specialist_usage_pending(&connection, "deep_investigator", 4_000);
        let run_id: String = connection
            .query_row(
                "SELECT id FROM agent_runs WHERE role='deep_investigator'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let snapshot = crate::agent_runtime::checkpoint::RunState::from_json(
            &crate::agent_runtime::store::read_snapshot(&connection, &run_id)
                .unwrap()
                .unwrap()
                .snapshot,
        );
        assert_eq!((snapshot.used_tokens, snapshot.model_requests), (20, 1));
        let _ = fs::remove_dir_all(root);
    }
}
