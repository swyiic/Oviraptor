fn insert_assignment_rows(connection: &Connection) {
    use super::evidence_graph::store::insert_evidence_node;
    use super::multi_agent::assignment::insert_assignment;
    use super::evidence_graph::contract::{EvidenceNodeKind, EvidenceProvenance};
    insert_assignment(connection, &assignment("a-1", "root-a", "dedup-1")).unwrap();
    insert_evidence_node(
        connection,
        &evidence_node(
            "n-1",
            "root-a",
            1,
            EvidenceNodeKind::Endpoint,
            EvidenceProvenance::Observed,
            json!({"method": "GET"}),
        ),
    )
    .unwrap();
}

/// §4.11 — every persisted JSON column the repositories parse must fail loudly
/// rather than surface as an empty list or object.
#[test]
fn corrupted_json_columns_are_reported_with_table_and_column() {
    use super::evidence_graph::contract::{EvidenceEdgeKind, EvidenceNodeKind, EvidenceProvenance};
    use super::evidence_graph::store::{
        insert_evidence_edge, insert_evidence_node, list_evidence_edges_at_revision,
        load_evidence_node,
    };
    use super::multi_agent::assignment::{insert_assignment, load_assignment};
    use super::review::{insert_review_decision, list_review_decisions_for_candidate, ReviewDecision, ReviewVerdict};
    let (root, connection) = stage1a_connection("stage1a-corrupt-json");
    let mut run = run_row("stage1a-scan", "https://app.example.invalid");
    run.id = "json-run".into();
    store::create_run(&connection, &run).unwrap();
    let reviewer = reviewer_row(&connection, &run, "json-reviewer");
    insert_assignment(&connection, &assignment("json-assignment", "json-run", "dedup-json")).unwrap();
    insert_evidence_node(
        &connection,
        &evidence_node("json-node", "json-run", 1, EvidenceNodeKind::Endpoint, EvidenceProvenance::Observed, json!({"method": "GET"})),
    )
    .unwrap();
    insert_evidence_node(
        &connection,
        &evidence_node("json-node-2", "json-run", 1, EvidenceNodeKind::Hypothesis, EvidenceProvenance::Inferred, json!({})),
    )
    .unwrap();
    insert_evidence_edge(
        &connection,
        &super::evidence_graph::contract::EvidenceEdge {
            id: 0,
            root_run_id: "json-run".into(),
            revision: 1,
            from_node_id: "json-node".into(),
            to_node_id: "json-node-2".into(),
            kind: EvidenceEdgeKind::Supports,
            payload: json!({"why": "pair"}),
            created_by_run_id: "json-reviewer".into(),
            created_at: String::new(),
        },
    )
    .unwrap();
    insert_review_decision(
        &connection,
        &ReviewDecision {
            id: 0,
            root_run_id: run.id.clone(),
            candidate_id: "json-candidate".into(),
            candidate_revision: 1,
            reviewer_run_id: reviewer.id.clone(),
            verdict: ReviewVerdict::Rejected,
            reason_codes: vec!["no_pair".into()],
            evidence_refs: vec!["req-1".into()],
            counter_evidence_refs: vec![],
            missing_evidence: vec![],
            confidence: 0.2,
            severity: "low".into(),
            created_at: String::new(),
        },
    )
    .unwrap();

    type CorruptRead = fn(&Connection) -> Result<(), String>;
    type CorruptCase = (&'static str, &'static str, &'static str, &'static str, CorruptRead);
    let cases: Vec<CorruptCase> = vec![
        ("agent_runs", "capability_lease_json", "[]", "json-run", |c| {
            store::load_run(c, "json-run").map(|_| ())
        }),
        ("agent_assignments", "task_slice_json", "{}", "json-assignment", |c| {
            load_assignment(c, "json-assignment").map(|_| ())
        }),
        ("agent_assignments", "contract_keys_json", "[]", "json-assignment", |c| {
            load_assignment(c, "json-assignment").map(|_| ())
        }),
        ("agent_assignments", "identity_handles_json", "[]", "json-assignment", |c| {
            load_assignment(c, "json-assignment").map(|_| ())
        }),
        ("agent_assignments", "capability_lease_json", "[]", "json-assignment", |c| {
            load_assignment(c, "json-assignment").map(|_| ())
        }),
        ("agent_evidence_nodes", "payload_json", "{}", "json-node", |c| {
            load_evidence_node(c, "json-node").map(|_| ())
        }),
        ("agent_evidence_nodes", "artifact_refs_json", "[]", "json-node", |c| {
            load_evidence_node(c, "json-node").map(|_| ())
        }),
        ("agent_evidence_edges", "payload_json", "{}", "json-edge", |c| {
            list_evidence_edges_at_revision(c, "json-run", 1).map(|_| ())
        }),
        ("agent_review_decisions", "reason_codes_json", "[]", "json-review", |c| {
            list_review_decisions_for_candidate(c, "json-candidate").map(|_| ())
        }),
        ("agent_review_decisions", "evidence_refs_json", "[]", "json-review", |c| {
            list_review_decisions_for_candidate(c, "json-candidate").map(|_| ())
        }),
        (
            "agent_review_decisions",
            "counter_evidence_refs_json",
            "[]",
            "json-review",
            |c| list_review_decisions_for_candidate(c, "json-candidate").map(|_| ()),
        ),
        ("agent_review_decisions", "missing_evidence_json", "[]", "json-review", |c| {
            list_review_decisions_for_candidate(c, "json-candidate").map(|_| ())
        }),
    ];
    let broken = "{\"unterminated";
    let edge_id: i64 = connection
        .query_row("SELECT id FROM agent_evidence_edges LIMIT 1", [], |row| row.get(0))
        .unwrap();
    let review_id: i64 = connection
        .query_row("SELECT id FROM agent_review_decisions LIMIT 1", [], |row| row.get(0))
        .unwrap();
    for (table, column, valid, key, read) in cases {
        let id = match key {
            "json-edge" => edge_id.to_string(),
            "json-review" => review_id.to_string(),
            other => other.to_string(),
        };
        connection
            .execute(
                &format!("UPDATE {table} SET {column}=?1 WHERE id=?2"),
                rusqlite::params![broken, id],
            )
            .unwrap();
        let error = read(&connection).unwrap_err();
        assert!(
            error.contains(column) && error.contains(table),
            "{table}.{column} 损坏时应报出表与列：{error}"
        );
        connection
            .execute(
                &format!("UPDATE {table} SET {column}=?1 WHERE id=?2"),
                rusqlite::params![valid, id],
            )
            .unwrap();
        read(&connection).unwrap();
    }
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

/// §4.12 — the Stage 1A columns round-trip, and a run written before them still
/// reads as single / no lane / empty lease.
#[test]
fn agent_run_row_round_trips_the_stage1a_columns() {
    let (root, connection) = stage1a_connection("stage1a-run-columns");
    let legacy = run_row("stage1a-scan", "https://app.example.invalid");
    store::create_run(&connection, &legacy).unwrap();
    let read = store::load_run(&connection, &legacy.id).unwrap().unwrap();
    assert_eq!(read, legacy);
    assert_eq!(read.root_run_id, "");
    assert_eq!(read.assignment_id, "");
    assert_eq!(read.lane, None);
    assert_eq!(
        read.orchestration_policy,
        super::contract::MultiAgentPolicy::Single
    );
    assert_eq!(read.capability_lease, Vec::<String>::new());
    assert_eq!(read.reserved_tokens, 0);
    assert_eq!(read.reserved_requests, 0);
    assert_eq!(read.heartbeat_at, "");
    assert_eq!(read.cancel_requested_at, "");

    let mut child = run_row("stage1a-scan", "https://app.example.invalid");
    child.id = "run-child".into();
    child.role = AgentRole::Authorization;
    child.parent_run_id = Some(legacy.id.clone());
    child.root_run_id = legacy.id.clone();
    child.assignment_id = "a-1".into();
    child.lane = Some(AgentLane::ReadOnlyAnalysis);
    child.orchestration_policy = super::contract::MultiAgentPolicy::Shadow;
    child.capability_lease = vec!["http_replay".into(), "read_artifact".into()];
    child.reserved_tokens = 4_000;
    child.reserved_requests = 3;
    child.heartbeat_at = "2026-09-21 10:00:00".into();
    child.cancel_requested_at = "2026-09-21 10:05:00".into();
    store::create_run(&connection, &child).unwrap();
    let read = store::load_run(&connection, "run-child").unwrap().unwrap();
    assert_eq!(read, child);
    connection
        .execute(
            "UPDATE agent_runs SET lane='smuggled' WHERE id='run-child'",
            [],
        )
        .unwrap();
    assert!(store::load_run(&connection, "run-child")
        .unwrap_err()
        .contains("lane"));
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

/// §4.13–§4.14 — a review answer needs a real reviewer inside the root, and a
/// confidence outside 0.0..=1.0 is refused before it can be stored.
#[test]
fn review_decisions_require_a_real_reviewer_and_a_bounded_confidence() {
    use super::review::{insert_review_decision, list_review_decisions_for_candidate, ReviewDecision, ReviewVerdict};
    let (root, connection) = stage1a_connection("stage1a-review-guard");
    let run = run_row("stage1a-scan", "https://app.example.invalid");
    store::create_run(&connection, &run).unwrap();
    let reviewer = reviewer_row(&connection, &run, "reviewer-1");
    let foreign = {
        let mut row = run_row("stage1a-scan", "https://other.example.invalid");
        row.id = "reviewer-foreign".into();
        row.role = AgentRole::EvidenceReviewer;
        row.root_run_id = "root-elsewhere".into();
        store::create_run(&connection, &row).unwrap();
        row
    };
    let decision = |who: &str, confidence: f64| ReviewDecision {
        id: 0,
        root_run_id: run.id.clone(),
        candidate_id: "candidate-guard".into(),
        candidate_revision: 1,
        reviewer_run_id: who.into(),
        verdict: ReviewVerdict::Rejected,
        reason_codes: vec![],
        evidence_refs: vec![],
        counter_evidence_refs: vec![],
        missing_evidence: vec![],
        confidence,
        severity: String::new(),
        created_at: String::new(),
    };
    let error = insert_review_decision(&connection, &decision(&run.id, 0.5)).unwrap_err();
    assert!(error.contains("evidence_reviewer"), "{error}");
    let error = insert_review_decision(&connection, &decision("missing-run", 0.5)).unwrap_err();
    assert!(error.contains("不存在"), "{error}");
    let error = insert_review_decision(&connection, &decision(&foreign.id, 0.5)).unwrap_err();
    assert!(error.contains("不属于 root"), "{error}");
    for confidence in [-0.1f64, 1.1, f64::NAN, f64::INFINITY] {
        let error = insert_review_decision(&connection, &decision(&reviewer.id, confidence))
            .unwrap_err();
        assert!(error.contains("confidence"), "{confidence} 应被拒绝：{error}");
    }
    assert!(list_review_decisions_for_candidate(&connection, "candidate-guard")
        .unwrap()
        .is_empty());
    let empty_root = ReviewDecision {
        root_run_id: String::new(),
        ..decision(&reviewer.id, 0.5)
    };
    assert!(insert_review_decision(&connection, &empty_root).unwrap_err().contains("root_run_id"));
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

/// §3.3 收尾：共享图上的每条事实都要署名，署名必须是真的 run，被取代的节点必须同 root；
/// 否则一个新 revision 可以悄悄退役另一个调查的证据。
#[test]
fn evidence_facts_require_an_author_and_a_same_root_supersession() {
    use super::evidence_graph::contract::{EvidenceNodeKind, EvidenceProvenance};
    use super::evidence_graph::store::{insert_evidence_edge, insert_evidence_node};
    let (root, connection) = stage1a_connection("stage1a-author");
    create_runs(&connection, &["author-a", "author-b"]);
    let node = evidence_node(
        "n-author",
        "author-a",
        1,
        EvidenceNodeKind::Endpoint,
        EvidenceProvenance::Observed,
        json!({"method": "GET"}),
    );
    insert_evidence_node(&connection, &node).unwrap();

    let anonymous = super::evidence_graph::contract::EvidenceNode {
        id: "n-anon".into(),
        created_by_run_id: String::new(),
        ..node.clone()
    };
    assert!(insert_evidence_node(&connection, &anonymous)
        .unwrap_err()
        .contains("署名"));
    let stranger = super::evidence_graph::contract::EvidenceNode {
        id: "n-stranger".into(),
        created_by_run_id: "ghost-run".into(),
        ..node.clone()
    };
    assert!(insert_evidence_node(&connection, &stranger)
        .unwrap_err()
        .contains("不存在的 run"));
    let retargeted = |id: &str, supersedes: &str| super::evidence_graph::contract::EvidenceNode {
        id: id.into(),
        natural_key_hash: super::evidence_graph::store::evidence_natural_key(
            "author-a",
            EvidenceNodeKind::Endpoint,
            id,
        ),
        supersedes_id: supersedes.into(),
        ..node.clone()
    };
    let missing_target = retargeted("n-supersedes-missing", "n-gone");
    let error = insert_evidence_node(&connection, &missing_target).unwrap_err();
    assert!(error.contains("不存在的节点"), "{error}");
    assert!(error.contains("n-gone"), "{error}");

    let other_root = evidence_node(
        "n-b1",
        "author-b",
        1,
        EvidenceNodeKind::Endpoint,
        EvidenceProvenance::Observed,
        json!({"method": "GET"}),
    );
    insert_evidence_node(&connection, &other_root).unwrap();
    let cross_root = retargeted("n-supersedes-other", "n-b1");
    assert!(insert_evidence_node(&connection, &cross_root)
        .unwrap_err()
        .contains("其它 root"));
    let same_revision = retargeted("n-supersedes-same-revision", "n-author");
    assert!(insert_evidence_node(&connection, &same_revision)
        .unwrap_err()
        .contains("更高 revision"));
    let different_kind = super::evidence_graph::contract::EvidenceNode {
        revision: 2,
        kind: EvidenceNodeKind::Target,
        ..retargeted("n-supersedes-different-kind", "n-author")
    };
    assert!(insert_evidence_node(&connection, &different_kind)
        .unwrap_err()
        .contains("不同 kind"));
    let same_root = super::evidence_graph::contract::EvidenceNode {
        revision: 2,
        ..retargeted("n-supersedes-same", "n-author")
    };
    insert_evidence_node(&connection, &same_root).unwrap();
    // Edge author's isolated check still needs two endpoints at its revision;
    // the successor above is correctly at revision 2 now.
    insert_evidence_node(&connection, &retargeted("n-peer", "")).unwrap();

    let orphan_edge_author = super::evidence_graph::contract::EvidenceEdge {
        id: 0,
        root_run_id: "author-a".into(),
        revision: 1,
        from_node_id: "n-author".into(),
        to_node_id: "n-peer".into(),
        kind: super::evidence_graph::contract::EvidenceEdgeKind::Supersedes,
        payload: json!({}),
        created_by_run_id: String::new(),
        created_at: String::new(),
    };
    assert!(insert_evidence_edge(&connection, &orphan_edge_author)
        .unwrap_err()
        .contains("署名"));
    let rows: i64 = connection
        .query_row("SELECT COUNT(*) FROM agent_evidence_edges", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 0, "被拒绝的边不得留下行");
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}
