#[test]
fn review_gate_insufficiency_triggers_gap_run_without_releasing_target_capabilities() {
    let (root, db_path, root_run_id, lease) = multi_agent_test_root("review-gap-trigger", 60_000, 20);
    let connection = db::open(&db_path).unwrap();
    let mut context = test_context(&db_path, &lease.target_key, vec![AgentIdentity::anonymous()]);
    context.scan_id = lease.scan_id.clone();
    context.evidence["forceReviewerInsufficientForTest"] = serde_json::json!(true);
    context.run = Some(AgentRunLedger { db_path: db_path.clone(), run_id: root_run_id.clone() });
    stage_agent_finding(&context, "web", "evidence", "gap-trigger", "candidate", "info",
        &serde_json::json!({"fact":"fixture"})).unwrap();
    let mut session = multi_agent_prepare(&mut context).unwrap();
    let outcome = AgentTargetOutcome::incomplete("missing fixture control");
    multi_agent_finish_execution(&context, &mut session, &outcome).unwrap();
    let reviewed = multi_agent_review(&context, &mut session, outcome);
    assert!(matches!(reviewed, AgentTargetOutcome::Incomplete(_)), "{reviewed:?}");
    let gap_runs: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_runs WHERE root_run_id=?1 AND role='deep_investigator' AND lane='read_only_analysis'",
        [&root_run_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(gap_runs, 1);
    let target_leases: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_capability_leases WHERE root_run_id=?1 AND capability IN ('replay_http','compare_identities') AND revoked_at='' AND lease_expires_at>datetime('now','localtime')",
        [&root_run_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(target_leases, 0);
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn reviewer_and_replay_preserve_manual_execution_stop_without_gap_autostart() {
    for code in [terminal_code::REQUEST_RECONCILIATION_REQUIRED, terminal_code::EXECUTION_AUTHORIZATION_DENIED] {
        let (_root, db_path, root_run_id, lease) = multi_agent_test_root("review-execution-stop", 60_000, 20);
        let connection = db::open(&db_path).unwrap();
        let mut context = test_context(&db_path, &lease.target_key, vec![AgentIdentity::anonymous()]);
        context.scan_id = lease.scan_id.clone();
        context.evidence["forceReviewerInsufficientForTest"] = serde_json::json!(true);
        context.run = Some(AgentRunLedger { db_path: db_path.clone(), run_id: root_run_id.clone() });
        stage_agent_finding(&context, "web", "evidence", "manual-stop", "candidate", "info",
            &serde_json::json!({"fact":"fixture"})).unwrap();
        let mut session = multi_agent_prepare(&mut context).unwrap();
        let outcome = AgentTargetOutcome::Incomplete(AgentStop::new(code, "manual resolution required"));
        multi_agent_finish_execution(&context, &mut session, &outcome).unwrap();
        for _ in 0..2 {
            let reviewed = multi_agent_review(&context, &mut session, outcome.clone());
            assert_eq!(reviewed.terminal_code(), code, "{reviewed:?}");
        }
        let (decisions,gaps):(i64,i64)=connection.query_row(
            "SELECT (SELECT COUNT(*) FROM agent_review_decisions WHERE root_run_id=?1), \
             (SELECT COUNT(*) FROM agent_runs WHERE root_run_id=?1 AND role='deep_investigator')",
            [&root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!((decisions,gaps),(1,0), "the Reviewer records its decision, but cannot auto-resolve an execution stop");
    }
}

#[test]
fn coverage_only_result_never_fabricates_a_reviewer_or_a_confirmed_decision() {
    let (root, db_path, root_run_id, lease) =
        multi_agent_test_root("coverage-no-reviewer", 60_000, 20);
    let connection = db::open(&db_path).unwrap();
    let mut context = test_context(&db_path, &lease.target_key, vec![AgentIdentity::anonymous()]);
    context.scan_id = lease.scan_id.clone();
    context.run = Some(AgentRunLedger { db_path: db_path.clone(), run_id: root_run_id.clone() });
    let mut session = multi_agent_prepare(&mut context).unwrap();
    let outcome = AgentTargetOutcome::incomplete("uncovered family");
    multi_agent_finish_execution(&context, &mut session, &outcome).unwrap();
    assert!(matches!(multi_agent_review(&context, &mut session, outcome),
        AgentTargetOutcome::Incomplete(_)));
    let (reviewers, requests, decisions): (i64, i64, i64) = connection.query_row(
        "SELECT (SELECT COUNT(*) FROM agent_runs WHERE root_run_id=?1 AND role='evidence_reviewer'), \
         (SELECT COUNT(*) FROM agent_review_requests WHERE root_run_id=?1), \
         (SELECT COUNT(*) FROM agent_review_decisions WHERE root_run_id=?1)",
        [&root_run_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap();
    assert_eq!((reviewers, requests, decisions), (0, 0, 0));
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn insufficient_finding_remains_reviewable_only_after_a_new_candidate_revision() {
    let (root, db_path, root_run_id, lease) =
        multi_agent_test_root("finding-gap-revision", 80_000, 30);
    let connection = db::open(&db_path).unwrap();
    let mut context = test_context(&db_path, &lease.target_key, vec![AgentIdentity::anonymous()]);
    context.scan_id = lease.scan_id.clone();
    context.run = Some(AgentRunLedger {
        db_path: db_path.clone(),
        run_id: root_run_id.clone(),
    });
    stage_agent_finding(
        &context, "web", "evidence", "gap", "candidate", "info",
        &serde_json::json!({"fact":"frozen candidate"}),
    ).unwrap();
    let mut session = multi_agent_prepare(&mut context).unwrap();
    context.evidence["forceReviewerInsufficientForTest"] = serde_json::json!(true);
    let outcome = AgentTargetOutcome::incomplete("same executor result");
    multi_agent_finish_execution(&context, &mut session, &outcome).unwrap();
    assert!(matches!(multi_agent_review(&context, &mut session, outcome.clone()),
        AgentTargetOutcome::Incomplete(_)));
    let first: (String, i64) = connection.query_row(
        "SELECT status,candidate_revision FROM agent_finding_candidates WHERE root_run_id=?1",
        [&root_run_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(first, ("insufficient_evidence".into(), 1));
    assert_eq!(pending_agent_finding_candidates(&connection, &root_run_id).unwrap().len(), 1);
    let published: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sentinel_findings WHERE scan_id=?1", [&lease.scan_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(published, 0);

    let replayed = multi_agent_review(&context, &mut session, outcome.clone());
    assert!(matches!(replayed, AgentTargetOutcome::Incomplete(ref reason)
        if reason.reason.starts_with("review_gate_insufficient_evidence:")), "{replayed:?}");
    let reviewers: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_runs WHERE root_run_id=?1 AND role='evidence_reviewer'",
        [&root_run_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(reviewers, 1, "a frozen revision must not spend another Reviewer call");
    connection.execute(
        "UPDATE agent_finding_candidates SET status='pending' WHERE root_run_id=?1",
        [&root_run_id],
    ).unwrap();
    let broken = multi_agent_review(&context, &mut session, outcome.clone());
    assert!(matches!(broken, AgentTargetOutcome::Incomplete(ref reason)
        if reason.reason.contains("review_replay_publication_incomplete_requires_recovery")), "{broken:?}");
    connection.execute(
        "UPDATE agent_finding_candidates SET status='insufficient_evidence' WHERE root_run_id=?1",
        [&root_run_id],
    ).unwrap();

    // Changing only model-visible prose cannot manufacture a new fact or spend
    // another Reviewer call after an insufficient decision.
    context.evidence.as_object_mut().unwrap().remove("forceReviewerInsufficientForTest");
    context.evidence["newVerifiedEvidence"] = serde_json::json!({"factRef":"fixture:2"});
    let unchanged = multi_agent_review(&context, &mut session, outcome.clone());
    assert!(matches!(unchanged, AgentTargetOutcome::Incomplete(ref reason)
        if reason.reason.starts_with("review_gate_insufficient_evidence:")), "{unchanged:?}");
    let reviewers: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_runs WHERE root_run_id=?1 AND role='evidence_reviewer'",
        [&root_run_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(reviewers, 1);

    use crate::agent_runtime::evidence_graph::{
        contract::{EvidenceNode, EvidenceNodeKind, EvidenceProvenance},
        store::insert_evidence_node,
    };
    let mut fact = EvidenceNode {
        id: "fixture-gap-fact".into(),
        root_run_id: root_run_id.clone(),
        revision: 2,
        kind: EvidenceNodeKind::RequestRecord,
        provenance: EvidenceProvenance::Inferred,
        natural_key_hash: crate::agent_runtime::store::stable_hash("fixture-gap-fact"),
        payload: serde_json::json!({"request":"fixture:2"}),
        artifact_refs: vec!["fixture-artifact:2".into()],
        created_by_run_id: session.executor.run_id.clone(),
        supersedes_id: String::new(),
        created_at: String::new(),
    };
    insert_evidence_node(&connection, &fact).unwrap();
    let inferred = multi_agent_review(&context, &mut session, outcome.clone());
    assert!(matches!(inferred, AgentTargetOutcome::Incomplete(ref reason)
        if reason.reason.starts_with("review_gate_insufficient_evidence:")), "{inferred:?}");
    fact.id = "fixture-gap-unlinked-observation".into();
    fact.natural_key_hash = crate::agent_runtime::store::stable_hash(&fact.id);
    fact.provenance = EvidenceProvenance::Observed;
    fact.artifact_refs.clear();
    insert_evidence_node(&connection, &fact).unwrap();
    let unlinked = multi_agent_review(&context, &mut session, outcome.clone());
    assert!(matches!(unlinked, AgentTargetOutcome::Incomplete(ref reason)
        if reason.reason.starts_with("review_gate_insufficient_evidence:")), "{unlinked:?}");
    fact.id = "fixture-gap-observation".into();
    fact.natural_key_hash = crate::agent_runtime::store::stable_hash(&fact.id);
    fact.artifact_refs = vec!["fixture-artifact:2".into()];
    insert_evidence_node(&connection, &fact).unwrap();
    // The Mapper can store source-derived graph nodes, but a nonempty string
    // is not proof that a source artifact exists or belongs to this attempt.
    // It must not reopen an insufficient Review without a source verifier.
    fact.id = "fixture-gap-mapper-unverified".into();
    fact.natural_key_hash = crate::agent_runtime::store::stable_hash(&fact.id);
    fact.kind = EvidenceNodeKind::Endpoint;
    fact.provenance = EvidenceProvenance::SourceDerived;
    fact.created_by_run_id = session.mapper.run_id.clone();
    insert_evidence_node(&connection, &fact).unwrap();
    // An arbitrary nonempty artifact reference is not a Broker observation.
    // The former test incorrectly let this fixture open a second Reviewer
    // revision and even publish a finding without new verified HTTP evidence.
    assert!(matches!(multi_agent_review(&context, &mut session, outcome),
        AgentTargetOutcome::Incomplete(_)));
    let second: (String, i64) = connection.query_row(
        "SELECT status,candidate_revision FROM agent_finding_candidates WHERE root_run_id=?1",
        [&root_run_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(second, ("insufficient_evidence".into(), 1));
    let decisions: Vec<(i64, String)> = connection.prepare(
        "SELECT candidate_revision,verdict FROM agent_review_decisions WHERE root_run_id=?1 ORDER BY candidate_revision",
    ).unwrap().query_map([&root_run_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap().map(Result::unwrap).collect();
    assert_eq!(decisions, vec![(1, "insufficient_evidence".into())]);
    let published: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sentinel_findings WHERE scan_id=?1", [&lease.scan_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(published, 0);
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn reviewer_model_decision_requires_a_bounded_consistent_schema() {
    let valid = r#"{"verdict":"confirmed","reasonCodes":["verified_fact"],"missingEvidence":[],"confidence":0.9,"summary":"Evidence checked"}"#;
    let decision = validated_review_decision(valid).unwrap();
    assert_eq!(decision.verdict, "confirmed");
    let invalid = [
        ("not json", "review_decision_invalid_json"),
        (r#"{"verdict":"confirmed"}"#, "review_decision_invalid_shape"),
        (r#"{"verdict":"maybe","reasonCodes":[],"missingEvidence":[],"confidence":0.9,"summary":"x"}"#, "review_decision_invalid_verdict"),
        (r#"{"verdict":"confirmed","reasonCodes":[{"bad":1}],"missingEvidence":[],"confidence":0.9,"summary":"x"}"#, "review_decision_invalid_reasons"),
        (r#"{"verdict":"confirmed","reasonCodes":[],"missingEvidence":["missing control"],"confidence":0.9,"summary":"x"}"#, "review_decision_confirmed_with_missing_evidence"),
        (r#"{"verdict":"rejected","reasonCodes":[],"missingEvidence":[],"confidence":1.5,"summary":"x"}"#, "review_decision_invalid_confidence"),
        (r#"{"verdict":"rejected","reasonCodes":[],"missingEvidence":[],"confidence":0.5,"summary":""}"#, "review_decision_invalid_summary"),
        (r#"{"verdict":"rejected","reasonCodes":[],"missingEvidence":[],"confidence":0.5,"summary":"x","extra":true}"#, "review_decision_invalid_shape"),
    ];
    for (text, expected) in invalid {
        assert_eq!(validated_review_decision(text).err().as_deref(), Some(expected), "{text}");
    }
}

