/// §7.9 — nodes and edges round-trip, a replay dedups, and a dangling edge fails.
#[test]
fn evidence_graph_round_trips_and_rejects_dangling_edges() {
    use super::evidence_graph::contract::{EvidenceEdge, EvidenceEdgeKind, EvidenceNode, EvidenceNodeKind, EvidenceProvenance};
    use super::evidence_graph::store::{EvidenceInsert, evidence_natural_key, insert_evidence_edge, insert_evidence_node, list_evidence_edges_at_revision, list_evidence_nodes_at_revision, load_evidence_node};
    let (root, connection) = stage1a_connection("stage1a-evidence");
    let run = run_row("stage1a-scan", "https://app.example.invalid");
    store::create_run(&connection, &run).unwrap();

    let node = |id: &str, kind: EvidenceNodeKind, identity: &str| EvidenceNode {
        id: id.to_string(),
        root_run_id: run.id.clone(),
        revision: 1,
        kind,
        provenance: EvidenceProvenance::Observed,
        natural_key_hash: evidence_natural_key(&run.id, kind, identity),
        payload: json!({"url": format!("https://app.example.invalid/{identity}")}),
        artifact_refs: vec!["artifact-1".to_string()],
        created_by_run_id: run.id.clone(),
        supersedes_id: String::new(),
        created_at: String::new(),
    };
    assert_eq!(
        insert_evidence_node(&connection, &node("node-a", EvidenceNodeKind::Endpoint, "/api/orders")),
        Ok(EvidenceInsert::Inserted("node-a".into()))
    );
    assert_eq!(
        insert_evidence_node(&connection, &node("node-a2", EvidenceNodeKind::Endpoint, "/api/orders")),
        Ok(EvidenceInsert::Existing("node-a".into())),
        "the same observation at the same revision is one node"
    );
    insert_evidence_node(&connection, &node("node-b", EvidenceNodeKind::RequestRecord, "req-1")).unwrap();
    let edge = EvidenceEdge {
        id: 0,
        root_run_id: run.id.clone(),
        revision: 1,
        from_node_id: "node-b".into(),
        to_node_id: "node-a".into(),
        kind: EvidenceEdgeKind::Targets,
        payload: json!({"method": "GET"}),
        created_by_run_id: run.id.clone(),
        created_at: String::new(),
    };
    insert_evidence_edge(&connection, &edge).unwrap();
    let edges = list_evidence_edges_at_revision(&connection, &run.id, 1).unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].kind, EvidenceEdgeKind::Targets);
    assert_eq!(edges[0].payload["method"], json!("GET"));
    let dangling = EvidenceEdge {
        id: 0,
        from_node_id: "ghost".into(),
        ..edge.clone()
    };
    assert!(insert_evidence_edge(&connection, &dangling).is_err(), "a dangling edge is refused");
    let replay = insert_evidence_edge(&connection, &edge).unwrap();
    assert!(matches!(replay, EvidenceInsert::Existing(_)));
    let nodes = list_evidence_nodes_at_revision(&connection, &run.id, 1).unwrap();
    assert_eq!(nodes.len(), 2);
    let loaded = load_evidence_node(&connection, "node-a").unwrap().unwrap();
    assert_eq!(loaded.kind, EvidenceNodeKind::Endpoint);
    assert_eq!(loaded.provenance, EvidenceProvenance::Observed);
    assert_eq!(loaded.artifact_refs, vec!["artifact-1".to_string()]);
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

/// §7.10 — credentials never reach storage unchanged.
#[test]
fn evidence_payload_is_redacted_before_storage() {
    use super::evidence_graph::contract::{EvidenceNode, EvidenceNodeKind, EvidenceProvenance};
    use super::evidence_graph::store::{evidence_natural_key};
    let (root, connection) = stage1a_connection("stage1a-redaction");
    let run = run_row("stage1a-scan", "https://app.example.invalid");
    store::create_run(&connection, &run).unwrap();
    let node = EvidenceNode {
        id: "secret-node".into(),
        root_run_id: run.id.clone(),
        revision: 3,
        kind: EvidenceNodeKind::RequestRecord,
        provenance: EvidenceProvenance::Observed,
        natural_key_hash: evidence_natural_key(&run.id, EvidenceNodeKind::RequestRecord, "req-secret"),
        payload: json!({
            "headers": {
                "Authorization": "Bearer super-secret-token-value",
                "Cookie": "session=abcdefghij0123456789"
            },
            "password": "hunter2-password",
            "api_key": "sk-live-abcdef1234567890",
            "path": "/api/orders"
        }),
        // References are ids, never credential text; the payload above is what the
        // shared redactor has to scrub on the way in.
        artifact_refs: vec!["artifact:9f3c0a71d2".to_string()],
        created_by_run_id: run.id.clone(),
        supersedes_id: String::new(),
        created_at: String::new(),
    };
    super::evidence_graph::store::insert_evidence_node(&connection, &node).unwrap();
    let stored: String = connection
        .query_row(
            "SELECT payload_json || '|' || artifact_refs_json FROM agent_evidence_nodes WHERE id='secret-node'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    for secret in [
        "super-secret-token-value",
        "abcdefghij0123456789",
        "hunter2-password",
        "sk-live-abcdef1234567890",
    ] {
        assert!(
        !stored.contains(secret),
        "{secret} landed in the database unchanged: {stored}"
    );
    }
    assert!(
        !stored.contains("Bearer "),
        "a bearer credential must not survive in the stored row: {stored}"
    );
    assert!(
        stored.contains("<redacted"),
        "the value must leave a marker: {stored}"
    );
    assert!(stored.contains("/api/orders"), "the non-secret part survives");
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

/// §7.11 — reviews are idempotent per unique key, conflicts are refused, and a new
/// revision is a different decision.
#[test]
fn review_decisions_are_idempotent_and_conflicts_are_refused() {
    use super::review::{
        insert_review_decision, list_review_decisions_for_candidate, ReviewDecision, ReviewInsert,
        ReviewVerdict,
    };
    let (root, connection) = stage1a_connection("stage1a-review");
    let run = run_row("stage1a-scan", "https://app.example.invalid");
    store::create_run(&connection, &run).unwrap();
    let reviewer = reviewer_row(&connection, &run, "stage1a-reviewer");
    let decision = || ReviewDecision {
        id: 0,
        root_run_id: run.id.clone(),
        candidate_id: "candidate-1".into(),
        candidate_revision: 1,
        reviewer_run_id: reviewer.id.clone(),
        verdict: ReviewVerdict::InsufficientEvidence,
        reason_codes: vec!["missing_control_request".into()],
        evidence_refs: vec!["req-1".into()],
        counter_evidence_refs: vec![],
        missing_evidence: vec!["req-2".into()],
        confidence: 0.4,
        severity: "medium".into(),
        created_at: String::new(),
    };
    let first = insert_review_decision(&connection, &decision()).unwrap();
    assert!(matches!(first, ReviewInsert::Inserted(_)));
    let replay = insert_review_decision(&connection, &decision()).unwrap();
    assert!(
        matches!(replay, ReviewInsert::Existing(_)),
        "the same decision replays as the row that exists: {replay:?}"
    );
    assert_eq!(
        list_review_decisions_for_candidate(&connection, "candidate-1")
            .unwrap()
            .len(),
        1,
        "a replay never adds a second decision"
    );
    let changed = ReviewDecision {
        verdict: ReviewVerdict::Confirmed,
        ..decision()
    };
    assert!(
        insert_review_decision(&connection, &changed).is_err(),
        "a different answer for the same candidate/revision/reviewer may not overwrite"
    );
    let next_revision = ReviewDecision {
        candidate_revision: 2,
        ..changed
    };
    assert!(matches!(
        insert_review_decision(&connection, &next_revision).unwrap(),
        ReviewInsert::Inserted(_)
    ));
    let rows = list_review_decisions_for_candidate(&connection, "candidate-1").unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].verdict, ReviewVerdict::Confirmed);
    assert_eq!(rows[1].candidate_revision, 2);
    assert_eq!(rows[0].reason_codes, vec!["missing_control_request".to_string()]);
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

/// §7.12 — storing a `confirmed` review publishes nothing.
#[test]
fn confirmed_review_does_not_create_a_sentinel_finding() {
    use super::review::{insert_review_decision, ReviewDecision, ReviewVerdict};
    let (root, connection) = stage1a_connection("stage1a-publish");
    let run = run_row("stage1a-scan", "https://app.example.invalid");
    store::create_run(&connection, &run).unwrap();
    let reviewer = reviewer_row(&connection, &run, "stage1a-reviewer-confirmed");
    let before: i64 = connection
        .query_row("SELECT COUNT(*) FROM sentinel_findings", [], |row| row.get(0))
        .unwrap();
    insert_review_decision(
        &connection,
        &ReviewDecision {
            id: 0,
            root_run_id: run.id.clone(),
            candidate_id: "candidate-confirmed".into(),
            candidate_revision: 1,
            reviewer_run_id: reviewer.id.clone(),
            verdict: ReviewVerdict::Confirmed,
            reason_codes: vec!["paired_requests".into()],
            evidence_refs: vec!["req-1".into(), "req-2".into()],
            counter_evidence_refs: vec![],
            missing_evidence: vec![],
            confidence: 0.95,
            severity: "high".into(),
            created_at: String::new(),
        },
    )
    .unwrap();
    let after: i64 = connection
        .query_row("SELECT COUNT(*) FROM sentinel_findings", [], |row| row.get(0))
        .unwrap();
    assert_eq!(before, after, "a review row is not a published finding");
    let nodes: i64 = connection
        .query_row("SELECT COUNT(*) FROM agent_evidence_nodes", [], |row| row.get(0))
        .unwrap();
    assert_eq!(nodes, 0);
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

/// §7.13 — one Coordinator run stays one Coordinator run at the repository level:
/// nothing in Stage 1A creates a child run or an assignment row. This is a storage
/// check only; the acceptance that a real native scan spends exactly four model
/// rounds, one target request and one coordinator row lives in
/// `commands::agent_tests::e2e_anonymous_spa_completes_without_strix` (§3.8).
#[test]
fn a_single_coordinator_run_stays_alone() {
    let (root, connection) = stage1a_connection("stage1a-single");
    let run = run_row("stage1a-scan", "https://app.example.invalid");
    store::create_run(&connection, &run).unwrap();
    store::set_run_status(&connection, &run.id, super::contract::AgentRunStatus::Running).unwrap();
    store::append_event(&connection, &run.id, AgentEventKind::RunCreated, &json!({}), &[]).unwrap();
    store::settle_usage(
        &connection,
        &run.id,
        &store::UsageDelta {
            total_tokens: 1_200,
            model_requests: 1,
            ..Default::default()
        },
    )
    .unwrap();
    let report = super::runtime_adapter::BackendReport::new(
        "stage1a-scan",
        1,
        &run.target_url,
        AgentBackendKind::Native,
    );
    super::runtime_adapter::close_run(&connection, &run.id, &report).unwrap();

    let runs: i64 = connection
        .query_row("SELECT COUNT(*) FROM agent_runs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(runs, 1, "Stage 1A must not create a child run");
    let coordinators: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM agent_runs WHERE role='coordinator'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(coordinators, 1);
    for table in ["agent_assignments", "agent_evidence_nodes", "agent_evidence_edges", "agent_review_decisions"] {
        let rows: i64 = connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 0, "{table} must stay empty until a scheduler exists");
    }
    let policy: String = connection
        .query_row("SELECT orchestration_policy FROM agent_runs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(policy, "single");
    let tokens: i64 = connection
        .query_row("SELECT used_tokens FROM agent_runs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(tokens, 1_200, "the single loop keeps its own accounting");
    let assignment_rows: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM agent_runs WHERE parent_run_id<>''",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(assignment_rows, 0, "Stage 1A adds no child run of any kind");
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

/// §4.1–§4.3 — an assignment id that belongs to another plan is refused, a dedup
/// replay with different first-plan content is a conflict, and an exact replay
/// returns the stored row instead of a second task.
#[test]
fn assignment_insert_distinguishes_replay_from_conflict() {
    use super::multi_agent::assignment::{insert_assignment, AssignmentInsert};
    let (root, connection) = stage1a_connection("stage1a-assign-conflict");
    create_runs(&connection, &["coord-1", "coord-2"]);
    let first = insert_assignment(&connection, &assignment("a-keep", "coord-1", "dedup-1")).unwrap();
    assert_eq!(first, AssignmentInsert::Inserted("a-keep".into()));
    let replay = insert_assignment(&connection, &assignment("a-other-id", "coord-1", "dedup-1"))
        .unwrap();
    assert_eq!(replay, AssignmentInsert::Existing("a-keep".into()));

    let drift = {
        let mut item = assignment("a-drift", "coord-1", "dedup-1");
        item.reserved_tokens = 9_000;
        item
    };
    let error = insert_assignment(&connection, &drift).unwrap_err();
    assert!(
        error.contains("assignment_dedup_conflict"),
        "内容不同的 dedup 重放必须是冲突：{error}"
    );
    let taken = {
        let mut item = assignment("a-keep", "coord-2", "dedup-2");
        item.lane = AgentLane::TargetTouching;
        item
    };
    let error = insert_assignment(&connection, &taken).unwrap_err();
    assert!(
        error.contains("已被其它计划占用"),
        "复用他人 id 不能当成重放：{error}"
    );
    let count: i64 = connection
        .query_row("SELECT COUNT(*) FROM agent_assignments", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1, "被拒绝的写入不得留下行");
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}
