// The provider is real local HTTP. Only its bounded public answer is scripted;
// creator, parent, Mapper, Reviewer, Investigator, costs and mailbox are real.
fn paid_proposal_response(request: &str, invalid: bool) -> String {
    if request.contains("You are the Root Coordinator") && request.contains("investigator-proposal")
    {
        return proposal_model_response(
            &json!({"schemaVersion":1,"observed":["original closed paid proposal"],
            "missing":["new approved control requires another attempt"],
            "suggestions":[if invalid {"dispatch:web_executor"}else {"assess:gap_proposal"}],
            "costNotes":["this Root SDK is charged to its original control"],
            "risks":["proposal estimates are not verified target facts"]})
            .to_string(),
        );
    }
    if request.contains("独立的只读 Deep Investigator") {
        let body: JsonValue =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        let input: JsonValue =
            serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
        return proposal_model_response(&json!({"summary":"high utility is only an advisory estimate",
            "nextStep":"request_new_contract","gapCode":"new_control_required","supportingFactRefs":[],
            "missingEvidence":input["reviewerMissingEvidence"],"prerequisites":["operator approves another attempt"],
            "proposedContracts":["new_attempt_control_group_request"],"expectedInformationGain":1.0,
            "impactCeiling":"critical","estimatedCost":{"modelTokens":4000,"modelRequests":1,"targetRequests":3},
            "sideEffectClass":"read_only","overlapKeys":["independent-control"],
            "falsificationCondition":"a newly authorized control refutes the hypothesis",
            "stopCondition":"no approved fresh attempt exists"}).to_string());
    }
    changed_fact_reviewer_response(request, true)
}
fn paid_proposal_review(f: &mut RootTickFixture) -> MultiAgentSession {
    drop(f.parent.take());
    let mut session = multi_agent_prepare(&mut f.context).unwrap();
    stage_agent_finding(
        &f.context,
        AGENT_VULNERABILITY_STAGE,
        "vulnerability",
        "unverified-proposal",
        "claim still needs independently authorized control",
        "info",
        &json!({"claim":"independent evidence absent"}),
    )
    .unwrap();
    let outcome = AgentTargetOutcome::incomplete("no verified target control");
    multi_agent_finish_execution(&f.context, &mut session, &outcome).unwrap();
    let reviewed = multi_agent_review(&f.context, &mut session, outcome);
    assert!(
        !matches!(reviewed, AgentTargetOutcome::Completed(_)),
        "{reviewed:?}"
    );
    session
}
fn paid_proposal_candidate(db: &rusqlite::Connection) -> (String, i64, JsonValue) {
    let (id, revision, missing): (String, i64, String) = db
        .query_row(
            "SELECT candidate_id,candidate_revision,missing_evidence_json
        FROM agent_review_decisions WHERE verdict='insufficient_evidence'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    (id, revision, serde_json::from_str(&missing).unwrap())
}
#[test]
fn coordinator_actual_closed_paid_proposal_returns_to_root_and_replay_spends_zero_sdk() {
    let _real = RealSpecialistTransport::enter();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|request| {
        (
            200,
            "application/json",
            paid_proposal_response(&request, false),
        )
    }));
    let mut f = root_tick_fixture(
        "paid-proposal-feedback",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let session = paid_proposal_review(&mut f);
    let db = db::open(&f.context.db_path).unwrap();
    assert_eq!(
        seen.lock().unwrap().len(),
        7,
        "four actual Root SDK plus Mapper/Reviewer/Investigator"
    );
    assert_eq!(
        root_tick_count(&db, &session.lease.root_run_id, "publication"),
        4
    );
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &session.lease.root_run_id,
            Some(""),
            "model_requests"
        )
        .unwrap()
        .consumed,
        4
    );
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &session.lease.root_run_id,
            None,
            "model_requests"
        )
        .unwrap()
        .consumed,
        7
    );
    let assessment: String = db
        .query_row(
            "SELECT payload_json FROM agent_messages WHERE kind='proposal_assessed'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let assessment: JsonValue = serde_json::from_str(&assessment).unwrap();
    assert_eq!(assessment["targetRequestsGranted"], 0);
    assert_eq!(assessment["newAttemptRequired"], true);
    assert_eq!(
        assessment["reasonCode"],
        "operator_approval_new_attempt_required"
    );
    let physical = web_mode_test_rows(&db);
    let (candidate, revision, missing) = paid_proposal_candidate(&db);
    multi_agent_investigate_review_gap(&f.context, &session, &candidate, revision, &missing)
        .unwrap();
    assert_eq!(seen.lock().unwrap().len(), 7);
    web_mode_assert_rows(&db, &physical);
    let requests = seen.lock().unwrap();
    let last = requests.last().unwrap();
    assert!(last.contains("investigator-proposal") && last.contains("existingRustAssessment"));
    let body: JsonValue = serde_json::from_str(last.split_once("\r\n\r\n").unwrap().1).unwrap();
    let registered = crate::agent_runtime::multi_agent::budget::root::model::tick::local::schemas();
    let actual_tools = body["tools"].as_array().unwrap();
    assert_eq!(actual_tools.len(), registered.len());
    for (actual, spec) in actual_tools.iter().zip(&registered) {
        let expected = spec.as_function_spec();
        assert_eq!(actual["type"], expected["type"]);
        assert_eq!(actual["function"]["name"], expected["function"]["name"]);
        assert_eq!(actual["function"]["parameters"], expected["function"]["parameters"]);
    }
    assert_eq!(actual_tools[2]["function"]["description"],
        "Read the captured pre-dispatch ledger snapshot: Root plus child consumption, active reservations, indeterminate cost, gross model headroom and original deadline. This observation is frozen for this paid call, conveys no grant and cannot authorize execution; Rust checks current balances at admission.");
    for i in [0, 1, 3] {
        assert_eq!(actual_tools[i], registered[i].as_function_spec());
    }
    assert_eq!(body["tool_choice"],"auto");
    let original: String = db.query_row(
        "SELECT fact_json FROM agent_root_tick_receipts WHERE root_run_id=?1 AND round=4 AND phase='request'",
        [&session.lease.root_run_id], |r| r.get(0),
    ).unwrap();
    let original: JsonValue = serde_json::from_str(&original).unwrap();
    assert_eq!(original["request"]["request"]["tools"], body["tools"]);
    let input: JsonValue =
        serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
    assert_eq!(
        input["basis"]["changedFact"]["semantic"]["existingRustAssessment"],
        assessment
    );
    assert_eq!(
        input["basis"]["changedFact"]["semantic"]["newTargetEvidenceProven"],
        false
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM sentinel_findings WHERE kind='vulnerability'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(requests);
    drop(session);
}
#[test]
fn coordinator_proposal_paid_worker_mutation_rejects_publication_then_exact_restore_reuses_fee() {
    let _real = RealSpecialistTransport::enter();
    let path = std::sync::Arc::new(std::sync::Mutex::new(None::<PathBuf>));
    let saved = std::sync::Arc::new(std::sync::Mutex::new(None::<(String, String)>));
    let (p, s) = (path.clone(), saved.clone());
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |request| {
        if request.contains("You are the Root Coordinator")
            && request.contains("investigator-proposal")
        {
            let db = db::open(p.lock().unwrap().as_ref().unwrap()).unwrap();
            let original: (String, String) = db
                .query_row(
                    "SELECT w.id,w.finished_at FROM agent_assignment_attempts w
                JOIN agent_runs r ON r.id=w.child_run_id WHERE r.role='deep_investigator'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert!(!original.1.is_empty());
            db.execute(
                "UPDATE agent_assignment_attempts SET finished_at='' WHERE id=?1",
                [&original.0],
            )
            .unwrap();
            *s.lock().unwrap() = Some(original);
        }
        (
            200,
            "application/json",
            paid_proposal_response(&request, false),
        )
    }));
    let mut f = root_tick_fixture(
        "paid-proposal-mutated",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    *path.lock().unwrap() = Some(f.context.db_path.clone());
    let session = paid_proposal_review(&mut f);
    let db = db::open(&f.context.db_path).unwrap();
    assert_eq!(
        seen.lock().unwrap().len(),
        7,
        "actual seventh provider response must be paid"
    );
    assert_eq!(
        root_tick_count(&db, &session.lease.root_run_id, "publication"),
        3,
        "original closed worker changed before local publication"
    );
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &session.lease.root_run_id,
            Some(""),
            "model_requests"
        )
        .unwrap()
        .consumed,
        4
    );
    let (id, finished) = saved.lock().unwrap().take().unwrap();
    db.execute(
        "UPDATE agent_assignment_attempts SET finished_at=?2 WHERE id=?1",
        params![id, finished],
    )
    .unwrap();
    let (candidate, revision, missing) = paid_proposal_candidate(&db);
    multi_agent_investigate_review_gap(&f.context, &session, &candidate, revision, &missing)
        .unwrap();
    assert_eq!(
        seen.lock().unwrap().len(),
        7,
        "exact original restoration recovers local publication without SDK retry"
    );
    assert_eq!(
        root_tick_count(&db, &session.lease.root_run_id, "publication"),
        4
    );
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &session.lease.root_run_id,
            Some(""),
            "model_requests"
        )
        .unwrap()
        .consumed,
        4
    );
    drop(session);
}
#[test]
fn coordinator_proposal_root_advisory_web_step_cannot_issue_another_target_grant() {
    let _real = RealSpecialistTransport::enter();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|request| {
        (
            200,
            "application/json",
            paid_proposal_response(&request, true),
        )
    }));
    let mut f = root_tick_fixture(
        "paid-proposal-forbidden-step",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let session = paid_proposal_review(&mut f);
    let db = db::open(&f.context.db_path).unwrap();
    assert_eq!(seen.lock().unwrap().len(), 7);
    let before = web_mode_test_rows(&db);
    let (candidate, revision, missing) = paid_proposal_candidate(&db);
    assert_eq!(
        multi_agent_investigate_review_gap(&f.context, &session, &candidate, revision, &missing)
            .unwrap_err(),
        "root_proposal_step_not_bounded"
    );
    web_mode_assert_rows(&db, &before);
    assert_eq!(seen.lock().unwrap().len(), 7);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_assignments WHERE role='web_executor'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM sentinel_findings WHERE kind='vulnerability'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let count: i64 = db
        .query_row(
            "SELECT coalesce(sum(json_extract(payload_json,'$.targetRequestsGranted')),0)
        FROM agent_messages WHERE kind='proposal_assessed'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    drop(session);
}
