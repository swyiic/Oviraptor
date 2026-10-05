// Actual ordinary creator/HMAC and original Root/C/control/limits at birth.
// This fixture adds genuine Root SDK calls; child counts exclude them explicitly.
fn specialist_original_context(
    tag: &str,
    endpoint: &str,
) -> (
    PathBuf,
    AgentRunContext,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) {
    let (root, path, run_id, actor) = multi_agent_new_task_root(tag, 60_000, 20);
    let db = db::open(&path).unwrap();
    let mut context = test_context(&path, &actor.target_key, vec![AgentIdentity::anonymous()]);
    context.scan_id = actor.scan_id.clone();
    context.execution_plan =
        web_mode_positive_plan_or(&path, &actor.target_key, context.execution_plan);
    let work: String = db
        .query_row(
            "SELECT work_dir FROM sentinel_scan_attempts WHERE scan_id=?1 AND attempt_number=1",
            [&actor.scan_id],
            |r| r.get(0),
        )
        .unwrap();
    context.target_dir = PathBuf::from(work).join("url-pipeline/target-00001");
    fs::create_dir_all(&context.target_dir).unwrap();
    fs::write(
        context.target_dir.join(".oviraptor-scan-id"),
        &actor.scan_id,
    )
    .unwrap();
    fs::write(
        context.target_dir.join("frontend-evidence.json"),
        context.evidence.to_string(),
    )
    .unwrap();
    context.environment.api_base = endpoint.into();
    context.run = Some(AgentRunLedger {
        db_path: path,
        run_id,
    });
    bind_agent_evidence_location(&context).unwrap();
    native_frozen_web_root_mode(&context).unwrap();
    (root, context, actor)
}
type SpecialistTestHandler =
    std::sync::Arc<dyn Fn(String) -> (u16, &'static str, String) + Send + Sync>;
type SpecialistTestSeen = std::sync::Arc<std::sync::Mutex<Vec<String>>>;
fn specialist_original_endpoint(
    handler: SpecialistTestHandler,
) -> (
    u16,
    SpecialistTestSeen,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    spawn_endpoint(std::sync::Arc::new(move |request| {
        if request.contains("You are the Root Coordinator") {
            (
                200,
                "application/json",
                changed_fact_reviewer_response(&request, true),
            )
        } else if request.contains("SPA/API Mapper") {
            (
                200,
                "application/json",
                proposal_model_response(
                    r#"{"summary":"actual paid Mapper receipt","priorityContracts":[],"risks":[]}"#,
                ),
            )
        } else if request.contains("Original fixture Evidence Reviewer") {
            (
                200,
                "application/json",
                proposal_model_response(
                    r#"{"verdict":"insufficient_evidence","reasonCodes":["missing_control"],"missingEvidence":["missing control"],"confidence":0.2,"summary":"reviewer needs control"}"#,
                ),
            )
        } else {
            handler(request)
        }
    }))
}
fn specialist_child_call_count(seen: &SpecialistTestSeen) -> usize {
    seen.lock()
        .unwrap()
        .iter()
        .filter(|request| !request.contains("You are the Root Coordinator"))
        .count()
}

// Retained unchanged for existing historical-scope tests outside this batch.
// Those callers are not evidence of a newly authorized production pipeline.
fn specialist_gap_fixture() -> (std::path::PathBuf, AgentRunContext, MultiAgentSession) {
    let (root, db_path, root_run_id, lease) = multi_agent_test_root("specialist-http", 60_000, 20);
    let mut context = test_context(
        &db_path,
        &lease.target_key,
        vec![AgentIdentity::anonymous()],
    );
    context.scan_id = lease.scan_id;
    context.run = Some(AgentRunLedger {
        db_path: db_path.clone(),
        run_id: root_run_id,
    });
    let mut session = multi_agent_prepare(&mut context).unwrap();
    multi_agent_finish_execution(
        &context,
        &mut session,
        &AgentTargetOutcome::incomplete("fixture"),
    )
    .unwrap();
    let connection = db::open(&db_path).unwrap();
    seal_gap_review_fixture(
        &connection,
        &session.lease,
        &context,
        "candidate-gap",
        2,
        &serde_json::json!(["missing control"]),
    );
    (root, context, session)
}

fn specialist_original_mapper_failure_endpoint(
    handler: SpecialistTestHandler,
) -> (
    u16,
    SpecialistTestSeen,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    spawn_endpoint(std::sync::Arc::new(move |request| {
        if request.contains("You are the Root Coordinator") {
            (
                200,
                "application/json",
                changed_fact_reviewer_response(&request, true),
            )
        } else {
            handler(request)
        }
    }))
}
fn seal_specialist_original_gap_review(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    context: &AgentRunContext,
) {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{mailbox, scheduler},
    };
    let child = scheduler::schedule_child(
        db,
        actor,
        AgentRole::EvidenceReviewer,
        AgentLane::Review,
        "candidate_ready",
        &json!({"candidateId":"candidate-gap","candidateRevision":2}),
        2,
        &["evidence.read".into(), "review.write".into()],
        8000,
        1,
    )
    .unwrap();
    scheduler::start_child_or_release(db, actor, &child).unwrap();
    let candidate = json!({"evidence":context.evidence,"persistedFactRefs":review_fact_refs(db,&actor.root_run_id,&context.target_dir).unwrap(),"findingCandidates":[]});
    open_review_request(
        db,
        actor,
        &child,
        "review-candidate-gap-2",
        "candidate-gap",
        2,
        &candidate.to_string(),
        &context.target_dir,
    )
    .unwrap();
    let (text, usage) = multi_agent_child_round_transport(
        context,
        actor,
        &child,
        "Original fixture Evidence Reviewer: inspect frozen evidence only",
        candidate,
    )
    .unwrap();
    let decision = validated_review_decision(&text).unwrap();
    assert_eq!(decision.verdict, "insufficient_evidence");
    assert_eq!(decision.missing_evidence, json!(["missing control"]));
    settle_child_usage(db, actor, &child, &usage).unwrap();
    let message = persist_review_decision(
        db,
        actor,
        &child,
        "review-candidate-gap-2",
        "candidate-gap",
        2,
        "insufficient_evidence",
        &json!(["missing_control"]),
        &decision.missing_evidence,
        0.2,
        &decision.summary,
        &context.target_dir,
    )
    .unwrap();
    mailbox::deliver_expected(db, actor, &actor.root_run_id, &message).unwrap();
    mailbox::acknowledge(db, actor, &actor.root_run_id, &message).unwrap();
    scheduler::finish_child(db, actor, &child, true, "review completed").unwrap();
}
