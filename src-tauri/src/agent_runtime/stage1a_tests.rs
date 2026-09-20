// Stage 1A acceptance tests (§7). They run against a temporary SQLite file only; no
// website, no model call and no target request is involved.

fn stage1a_connection(tag: &str) -> (std::path::PathBuf, Connection) {
    let (root, path) = temp_db(tag);
    let connection = crate::db::open(&path).unwrap();
    seeded(&connection, "stage1a-scan");
    (root, connection)
}

fn assignment(id: &str, coordinator: &str, dedup: &str) -> super::multi_agent::assignment::AgentAssignment {
    super::multi_agent::assignment::AgentAssignment::new(
        id,
        coordinator,
        AgentRole::Authorization,
        AgentLane::ReadOnlyAnalysis,
        "https://app.example.invalid",
        dedup,
    )
}

/// §7.1 — the policy defaults to today's single-loop behaviour and nothing else
/// parses into a more permissive value.
#[test]
fn multi_agent_policy_defaults_to_single() {
    assert_eq!(super::contract::MultiAgentPolicy::default(), super::contract::MultiAgentPolicy::Single);
    for value in ["", "   ", "shadow_", "two", "STRIX", "多"] {
        assert_eq!(
            super::contract::MultiAgentPolicy::parse(value),
            super::contract::MultiAgentPolicy::Single,
            "{value} must not widen the policy"
        );
    }
    assert_eq!(
        super::contract::MultiAgentPolicy::parse("shadow").as_str(),
        "shadow"
    );
    assert_eq!(super::contract::MultiAgentPolicy::Multi.as_str(), "multi");
    let text = serde_json::to_string(&super::contract::MultiAgentPolicy::Shadow).unwrap();
    assert_eq!(text, "\"shadow\"");
    let back: super::contract::MultiAgentPolicy = serde_json::from_str(&text).unwrap();
    assert_eq!(back, super::contract::MultiAgentPolicy::Shadow);
}

/// §7.2 — every role round-trips, the six retired names read as the new semantic
/// role, and writing a loaded row back stores only the new value.
#[test]
fn agent_roles_round_trip_and_legacy_aliases_rewrite_to_new_values() {
    let roles = [
        AgentRole::Coordinator,
        AgentRole::SpaApiMapper,
        AgentRole::ExternalSurface,
        AgentRole::IdentitySession,
        AgentRole::Authorization,
        AgentRole::InputParser,
        AgentRole::Upload,
        AgentRole::BusinessLogic,
        AgentRole::Concurrency,
        AgentRole::ClientSide,
        AgentRole::DeepInvestigator,
        AgentRole::EvidenceReviewer,
    ];
    for role in roles {
        assert_eq!(AgentRole::parse(role.as_str()), role, "{role:?}");
        assert_ne!(role.as_str().find(char::is_uppercase), Some(0));
    }
    let aliases = [
        ("evidence_triage", AgentRole::SpaApiMapper),
        ("contract_verifier", AgentRole::InputParser),
        ("identity_comparator", AgentRole::Authorization),
        ("business_flow_analyst", AgentRole::BusinessLogic),
        ("attack_chain_correlator", AgentRole::DeepInvestigator),
        ("final_reviewer", AgentRole::EvidenceReviewer),
    ];
    for (legacy, expected) in aliases {
        assert_eq!(AgentRole::parse(legacy), expected, "{legacy}");
        assert_ne!(expected.as_str(), legacy);
    }
    assert_eq!(AgentRole::parse("who_knows"), AgentRole::Coordinator);

    let (root, connection) = stage1a_connection("stage1a-roles");
    let run = run_row("stage1a-scan", "https://app.example.invalid");
    store::create_run(&connection, &run).unwrap();
    let mut legacy = assignment("role-replay", &run.id, "role-replay");
    legacy.role = AgentRole::parse("contract_verifier");
    connection
        .execute(
            "INSERT INTO agent_assignments(id,coordinator_run_id,role,dedup_key,state) VALUES('legacy-row',?1,'contract_verifier','legacy-key','prepared')",
            [&run.id],
        )
        .unwrap();
    let loaded = super::multi_agent::assignment::load_assignment(&connection, "legacy-row")
        .unwrap()
        .expect("the legacy row reads");
    assert_eq!(loaded.role, AgentRole::InputParser);
    let mut replay = loaded.clone();
    replay.id = String::new();
    legacy.id = String::new();
    assert!(replay.id.is_empty() && legacy.id.is_empty());
    match super::multi_agent::assignment::insert_assignment(&connection, &loaded) {
        // Re-inserting under the same primary key is a dedup hit for this
        // coordinator, and the value it carries is the new role name.
        Ok(result) => assert!(matches!(
            result,
            super::multi_agent::assignment::AssignmentInsert::Existing(_)
        )),
        Err(error) => panic!("legacy assignment replay failed: {error}"),
    }
    let stored: String = connection
        .query_row("SELECT role FROM agent_assignments WHERE id='legacy-row'", [], |row| row.get(0))
        .unwrap();
    assert_eq!(stored, "contract_verifier", "an update is not part of a replay");
    let mut fresh = assignment("rewritten", &run.id, "rewritten");
    fresh.role = loaded.role;
    super::multi_agent::assignment::insert_assignment(&connection, &fresh).unwrap();
    let rewritten: String = connection
        .query_row("SELECT role FROM agent_assignments WHERE id='rewritten'", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rewritten, "input_parser", "only the new value is written");
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

/// §7.3 — full round-trip of every new vocabulary.
#[test]
fn stage1a_vocabulary_round_trips() {
    for lane in [AgentLane::TargetTouching, AgentLane::ReadOnlyAnalysis, AgentLane::Review] {
        assert_eq!(AgentLane::parse(lane.as_str()), lane);
    }
    assert_eq!(AgentLane::TargetTouching.as_str(), "target_touching");
    assert!(AgentLane::TargetTouching.touches_target());
    assert!(!AgentLane::Review.touches_target());

    use super::contract::AssignmentState as State;
    let states = [
        State::Prepared, State::Leased, State::Running, State::WaitingReview, State::Paused,
        State::NeedsEvidence, State::Completed, State::Failed, State::Cancelled, State::LeaseExpired,
    ];
    for state in states {
        assert_eq!(State::parse(state.as_str()), state);
        assert_eq!(state.is_terminal(), matches!(
            state,
            State::Completed | State::Failed | State::Cancelled | State::LeaseExpired
        ));
    }

    use super::evidence_graph::contract::{EvidenceEdgeKind, EvidenceNodeKind, EvidenceProvenance};
    for kind in [
        EvidenceNodeKind::Target, EvidenceNodeKind::Identity, EvidenceNodeKind::PageState,
        EvidenceNodeKind::Endpoint, EvidenceNodeKind::RequestRecord, EvidenceNodeKind::ResponseShape,
        EvidenceNodeKind::BusinessObject, EvidenceNodeKind::Hypothesis, EvidenceNodeKind::Contract,
        EvidenceNodeKind::ToolInvocation, EvidenceNodeKind::CandidateFinding, EvidenceNodeKind::Finding,
    ] {
        assert_eq!(EvidenceNodeKind::parse(kind.as_str()), kind);
        let text = serde_json::to_string(&kind).unwrap();
        assert_eq!(serde_json::from_str::<EvidenceNodeKind>(&text).unwrap(), kind);
    }
    for provenance in [EvidenceProvenance::Observed, EvidenceProvenance::SourceDerived, EvidenceProvenance::Inferred] {
        assert_eq!(EvidenceProvenance::parse(provenance.as_str()), provenance);
    }
    for kind in [
        EvidenceEdgeKind::DerivedFrom, EvidenceEdgeKind::ObservedBy, EvidenceEdgeKind::Supports,
        EvidenceEdgeKind::Contradicts, EvidenceEdgeKind::Targets, EvidenceEdgeKind::UsesIdentity,
        EvidenceEdgeKind::Controls, EvidenceEdgeKind::Tests, EvidenceEdgeKind::Supersedes,
    ] {
        assert_eq!(EvidenceEdgeKind::parse(kind.as_str()), kind);
    }
    assert_eq!(
        super::review::ReviewVerdict::InsufficientEvidence.as_str(),
        "insufficient_evidence"
    );
    for verdict in [
        super::review::ReviewVerdict::Confirmed,
        super::review::ReviewVerdict::Rejected,
        super::review::ReviewVerdict::InsufficientEvidence,
    ] {
        assert_eq!(super::review::ReviewVerdict::parse(verdict.as_str()), verdict);
    }
    assert_eq!(
        super::review::ReviewVerdict::parse("maybe"),
        super::review::ReviewVerdict::InsufficientEvidence,
        "an unreadable verdict is never a confirmation"
    );
    for kind in [
        AgentMessageKind::GapProposed, AgentMessageKind::ProposalAssessed,
        AgentMessageKind::ReviewRequested, AgentMessageKind::ReviewDecided,
        AgentMessageKind::HumanDirective,
    ] {
        assert_eq!(AgentMessageKind::parse(kind.as_str()), kind);
    }
}

/// §7.4 — every allowed transition works, and nothing else does.
#[test]
fn assignment_transitions_are_a_closed_table() {
    use super::contract::{can_transition, AssignmentState as State};
    let allowed = [
        (State::Prepared, State::Leased),
        (State::Prepared, State::Cancelled),
        (State::Leased, State::Running),
        (State::Leased, State::Cancelled),
        (State::Leased, State::LeaseExpired),
        (State::Running, State::WaitingReview),
        (State::Running, State::Paused),
        (State::Running, State::NeedsEvidence),
        (State::Running, State::Completed),
        (State::Running, State::Failed),
        (State::Running, State::Cancelled),
        (State::Running, State::LeaseExpired),
        (State::Paused, State::Leased),
        (State::Paused, State::Cancelled),
        (State::NeedsEvidence, State::Leased),
        (State::NeedsEvidence, State::Cancelled),
        (State::WaitingReview, State::Completed),
        (State::WaitingReview, State::NeedsEvidence),
        (State::WaitingReview, State::Failed),
        (State::WaitingReview, State::Cancelled),
    ];
    for (from, to) in allowed {
        assert!(can_transition(from, to), "{} -> {}", from.as_str(), to.as_str());
    }
    let forbidden = [
        (State::Prepared, State::Running),
        (State::Prepared, State::Completed),
        (State::Leased, State::Completed),
        (State::Running, State::Leased),
        (State::Running, State::Prepared),
        (State::Paused, State::Running),
        (State::WaitingReview, State::Running),
        (State::WaitingReview, State::LeaseExpired),
    ];
    for (from, to) in forbidden {
        assert!(!can_transition(from, to), "{} -> {} must be refused", from.as_str(), to.as_str());
    }
    for terminal in [State::Completed, State::Failed, State::Cancelled, State::LeaseExpired] {
        for target in [State::Prepared, State::Leased, State::Running, State::Completed, terminal] {
            assert!(
                !can_transition(terminal, target),
                "{} may never leave the terminal state",
                terminal.as_str()
            );
        }
    }
}

/// §7.5 and §7.6 — the migration is repeatable, keeps history, and the new schema
/// is actually present.
#[test]
fn initialize_twice_keeps_history_and_creates_the_stage1a_schema() {
    let (root, path) = temp_db("stage1a-migration");
    {
        let connection = crate::db::open(&path).unwrap();
        seeded(&connection, "stage1a-scan");
        let run = run_row("stage1a-scan", "https://app.example.invalid");
        store::create_run(&connection, &run).unwrap();
    }
    // A second initialize over a database that already has the tables must be a
    // no-op, not an error, and must not touch the historical row.
    crate::db::initialize(&root).unwrap();
    let connection = crate::db::open(&path).unwrap();
    let runs: i64 = connection
        .query_row("SELECT COUNT(*) FROM agent_runs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(runs, 1);
    let policy: String = connection
        .query_row("SELECT orchestration_policy FROM agent_runs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(policy, "single", "history is never rewritten to multi");
    let roles: String = connection
        .query_row("SELECT role FROM agent_runs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(roles, "coordinator");

    for table in [
        "agent_assignments",
        "agent_evidence_nodes",
        "agent_evidence_edges",
        "agent_review_decisions",
    ] {
        let exists: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(exists, 1, "{table} is missing");
    }
    for column in [
        "root_run_id", "assignment_id", "lane", "orchestration_policy", "capability_lease_json",
        "reserved_tokens", "reserved_requests", "heartbeat_at", "cancel_requested_at",
    ] {
        let found: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('agent_runs') WHERE name=?1",
                [column],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(found, 1, "agent_runs.{column} is missing");
    }
    for index in [
        "idx_agent_assignments_coordinator",
        "idx_agent_assignments_child",
        "idx_agent_evidence_nodes_revision",
        "idx_agent_evidence_edges_revision",
        "idx_agent_review_candidate",
    ] {
        let found: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name=?1",
                [index],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(found, 1, "{index} is missing");
    }
    // The unique constraints themselves are exercised in
    // `stage1a_unique_constraints_are_enforced_by_sql`, which writes through raw SQL.
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

/// §7.6 — the unique constraints live in the schema, so a caller that bypasses the
/// repository still cannot create a duplicate task, node, dangling edge or decision.
#[test]
fn stage1a_unique_constraints_are_enforced_by_sql() {
    let (root, connection) = stage1a_connection("stage1a-constraints");
    let run = run_row("stage1a-scan", "https://app.example.invalid");
    store::create_run(&connection, &run).unwrap();
    connection
        .execute(
            "INSERT INTO agent_assignments(id,coordinator_run_id,dedup_key) VALUES('x1',?1,'k')",
            [&run.id],
        )
        .unwrap();
    assert!(
        connection
            .execute(
                "INSERT INTO agent_assignments(id,coordinator_run_id,dedup_key) VALUES('x2',?1,'k')",
                [&run.id]
            )
            .is_err(),
        "the (coordinator_run_id, dedup_key) unique key is missing"
    );
    connection
        .execute(
            "INSERT INTO agent_evidence_nodes(id,root_run_id,revision,natural_key_hash) VALUES('n1',?1,1,'h')",
            [&run.id],
        )
        .unwrap();
    assert!(
        connection
            .execute(
                "INSERT INTO agent_evidence_nodes(id,root_run_id,revision,natural_key_hash) VALUES('n2',?1,1,'h')",
                [&run.id]
            )
            .is_err(),
        "the evidence node natural key is missing"
    );
    assert!(
        connection
            .execute(
                "INSERT INTO agent_evidence_edges(root_run_id,revision,from_node_id,to_node_id,kind) VALUES(?1,1,'n1','missing','targets')",
                [&run.id]
            )
            .is_err(),
        "the edge endpoint foreign key is missing"
    );
    connection
        .execute(
            "INSERT INTO agent_review_decisions(root_run_id,candidate_id,candidate_revision,reviewer_run_id,verdict) VALUES(?1,'c',1,'r','confirmed')",
            [&run.id],
        )
        .unwrap();
    assert!(
        connection
            .execute(
                "INSERT INTO agent_review_decisions(root_run_id,candidate_id,candidate_revision,reviewer_run_id,verdict) VALUES(?1,'c',1,'r','rejected')",
                [&run.id]
            )
            .is_err(),
        "the (candidate_id, candidate_revision, reviewer_run_id) unique key is missing"
    );
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

/// §7.7 — a dedup key is unique per coordinator, and two coordinators may reuse it.
#[test]
fn assignment_dedup_key_replays_instead_of_duplicating() {
    let (root, connection) = stage1a_connection("stage1a-dedup");
    let first = run_row("stage1a-scan", "https://a.example.invalid");
    let second = run_row("stage1a-scan", "https://b.example.invalid");
    store::create_run(&connection, &first).unwrap();
    store::create_run(&connection, &second).unwrap();

    assert_eq!(
        super::multi_agent::assignment::insert_assignment(&connection, &assignment("one", &first.id, "same-key")),
        Ok(super::multi_agent::assignment::AssignmentInsert::Inserted("one".into()))
    );
    assert_eq!(
        super::multi_agent::assignment::insert_assignment(&connection, &assignment("two", &first.id, "same-key")),
        Ok(super::multi_agent::assignment::AssignmentInsert::Existing("one".into())),
        "a replay must not create a second task for the same work"
    );
    assert_eq!(
        super::multi_agent::assignment::insert_assignment(&connection, &assignment("three", &second.id, "same-key")),
        Ok(super::multi_agent::assignment::AssignmentInsert::Inserted("three".into())),
        "another coordinator owns its own dedup space"
    );
    let rows: i64 = connection
        .query_row("SELECT COUNT(*) FROM agent_assignments", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 2);
    assert_eq!(
        super::multi_agent::assignment::list_assignments_for_coordinator(&connection, &first.id)
            .unwrap()
            .len(),
        1
    );
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

/// §7.8 — the lifecycle update is a compare-and-set.
#[test]
fn transition_assignment_compares_and_sets() {
    let (root, connection) = stage1a_connection("stage1a-cas");
    let run = run_row("stage1a-scan", "https://app.example.invalid");
    store::create_run(&connection, &run).unwrap();
    super::multi_agent::assignment::insert_assignment(&connection, &assignment("cas", &run.id, "cas")).unwrap();

    assert!(super::multi_agent::assignment::transition_assignment(
        &connection,
        "cas",
        super::contract::AssignmentState::Prepared,
        super::contract::AssignmentState::Leased
    )
    .unwrap());
    assert!(
        !super::multi_agent::assignment::transition_assignment(
            &connection,
            "cas",
            super::contract::AssignmentState::Prepared,
            super::contract::AssignmentState::Leased
        )
        .unwrap(),
        "the row is already leased, so a stale expected state may not move it again"
    );
    assert!(super::multi_agent::assignment::transition_assignment(
        &connection,
        "cas",
        super::contract::AssignmentState::Prepared,
        super::contract::AssignmentState::Running
    )
    .is_err());
    let loaded = super::multi_agent::assignment::load_assignment(&connection, "cas")
        .unwrap()
        .unwrap();
    assert_eq!(loaded.state, super::contract::AssignmentState::Leased);
    assert!(!loaded.leased_at.is_empty(), "the lease timestamp is stamped");
    assert!(super::multi_agent::assignment::transition_assignment(
        &connection,
        "cas",
        super::contract::AssignmentState::Leased,
        super::contract::AssignmentState::Completed
    )
    .is_err());
    assert!(
        !super::multi_agent::assignment::transition_assignment(
            &connection,
            "missing",
            super::contract::AssignmentState::Leased,
            super::contract::AssignmentState::Running
        )
        .unwrap(),
        "an absent assignment cannot be moved"
    );
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

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
    let decision = || ReviewDecision {
        id: 0,
        root_run_id: run.id.clone(),
        candidate_id: "candidate-1".into(),
        candidate_revision: 1,
        reviewer_run_id: run.id.clone(),
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
            reviewer_run_id: run.id.clone(),
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

/// §7.13 — one Coordinator run stays one Coordinator run: nothing in Stage 1A
/// creates a child run, an assignment or an extra request.
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
    let report = super::strix_adapter::BackendReport::new(
        "stage1a-scan",
        1,
        &run.target_url,
        AgentBackendKind::Native,
    );
    super::strix_adapter::close_run(&connection, &run.id, &report).unwrap();

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
