fn multi_agent_test_root(
    name: &str,
    hard_tokens: i64,
    hard_requests: i64,
) -> (
    std::path::PathBuf,
    std::path::PathBuf,
    String,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) {
    multi_agent_test_root_for_target(name,hard_tokens,hard_requests,"https://authorized.example.test")
}

fn multi_agent_test_root_for_target(
    name: &str, hard_tokens: i64, hard_requests: i64, target: &str,
) -> (
    std::path::PathBuf, std::path::PathBuf, String,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) {
    use crate::agent_runtime::{
        contract::{
            AgentBackendKind, AgentLane, AgentRole, AgentRunStatus, MultiAgentPolicy,
        },
        multi_agent::lease,
        store::{self, AgentRunRow},
    };

    let root = std::env::temp_dir().join(format!("oviraptor-{name}-{}", uuid::Uuid::new_v4()));
    let db_path = db::initialize(&root).unwrap();
    let connection = db::open(&db_path).unwrap();
    connection
        .execute(
            "INSERT INTO projects(name) VALUES(?1)",
            [format!("Multi-agent {name}")],
        )
        .unwrap();
    let project_id = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO sentinel_scans(id,project_id,project_name,status,scan_type,attempt_count) VALUES(?1,?2,?3,'scanning','web',1)",
            rusqlite::params![
                format!("scan-{name}"),
                project_id,
                format!("Multi-agent {name}")
            ],
        )
        .unwrap();
    let root_run_id = format!("root-{name}");
    let plan = test_plan_for("standard", target).with_attempt(1);
    let mut run = AgentRunRow::new(
        &root_run_id,
        format!("scan-{name}"),
        1,
        target,
        AgentBackendKind::Native,
        AgentRole::Coordinator,
        plan.hash(),
        "evidence",
    )
    .with_budget(
        hard_tokens.saturating_div(2),
        hard_tokens,
        hard_requests.saturating_div(2),
        hard_requests,
    );
    run.status = AgentRunStatus::Running;
    run.root_run_id = root_run_id.clone();
    run.orchestration_policy = MultiAgentPolicy::Multi;
    run.lane = Some(AgentLane::ReadOnlyAnalysis);
    store::create_run(&connection, &run).unwrap();
    connection.execute(
        "UPDATE agent_runs SET plan_json=?1 WHERE id=?2",
        rusqlite::params![plan.as_json().to_string(), &root_run_id],
    ).unwrap();
    let coordinator_lease = lease::acquire_coordinator_lease(
        &connection,
        &format!("scan-{name}"),
        1,
        target,
        &root_run_id,
        600,
    )
    .unwrap();
    (root, db_path, root_run_id, coordinator_lease)
}

fn seal_gap_review_fixture(
    connection: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    context: &AgentRunContext,
    candidate_id: &str,
    revision: i64,
    missing: &serde_json::Value,
) {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{mailbox, scheduler},
    };
    let reviewer = scheduler::schedule_child(
        connection, lease, AgentRole::EvidenceReviewer, AgentLane::Review,
        "candidate_ready", &serde_json::json!({"candidateId":candidate_id,"candidateRevision":revision}),
        revision, &["evidence.read".into(), "review.write".into()], 8_000, 1,
    ).unwrap();
    scheduler::mark_child_running(connection, lease, &reviewer).unwrap();
    let request_id = format!("review-{candidate_id}-{revision}");
    let candidate = serde_json::json!({
        "evidence": context.evidence,
        "persistedFactRefs": review_fact_refs(connection, &lease.root_run_id, &context.target_dir).unwrap(),
        "findingCandidates": [],
    });
    open_review_request(connection, lease, &reviewer, &request_id, candidate_id, revision,
        &candidate.to_string(), &context.target_dir).unwrap();
    let message_id = persist_review_decision(
        connection, lease, &reviewer, &request_id, candidate_id, revision,
        "insufficient_evidence", &serde_json::json!(["missing_control"]), missing,
        0.2, "reviewer needs control", &context.target_dir,
    ).unwrap();
    mailbox::deliver_expected(connection, lease, &lease.root_run_id, &message_id).unwrap();
    mailbox::acknowledge(connection, lease, &lease.root_run_id, &message_id).unwrap();
    scheduler::finish_child(connection, lease, &reviewer, true, "review completed").unwrap();
}

