// Stage 1A acceptance tests (§7). They run against a temporary SQLite file only; no
// website, no model call and no target request is involved.

/// §3.7 — a review is graded by an evidence reviewer that belongs to the root. The
/// tests create one instead of letting the coordinator mark its own candidate.
fn reviewer_row(connection: &Connection, root: &store::AgentRunRow, id: &str) -> store::AgentRunRow {
    let mut row = run_row("stage1a-scan", "https://app.example.invalid");
    row.id = id.to_string();
    row.role = AgentRole::EvidenceReviewer;
    row.root_run_id = root.id.clone();
    store::create_run(connection, &row).unwrap();
    row
}

/// The repositories hold foreign keys to `agent_runs`, so a test root has to be a
/// run that exists.
fn create_runs(connection: &Connection, ids: &[&str]) {
    for id in ids {
        let mut row = run_row("stage1a-scan", "https://app.example.invalid");
        row.id = (*id).to_string();
        store::create_run(connection, &row).unwrap();
    }
}

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
        AgentRole::WebExecutor,
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
    assert_eq!(AgentRole::try_parse("who_knows"), None);

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

#[test]
fn unknown_persisted_roles_cannot_become_coordinators() {
    let (root, connection) = stage1a_connection("unknown-role");
    let run = run_row("stage1a-scan", "https://app.example.invalid");
    store::create_run(&connection, &run).unwrap();
    connection
        .execute("UPDATE agent_runs SET role='unknown_role' WHERE id=?1", [&run.id])
        .unwrap();
    assert!(store::load_run(&connection, &run.id)
        .unwrap_err()
        .contains("agent_runs 的 role"));
    connection
        .execute("UPDATE agent_runs SET role='coordinator' WHERE id=?1", [&run.id])
        .unwrap();
    connection
        .execute(
            "INSERT INTO agent_assignments(id,coordinator_run_id,role,dedup_key,state) \
             VALUES('unknown-assignment',?1,'unknown_role','unknown-role','prepared')",
            [&run.id],
        )
        .unwrap();
    assert!(super::multi_agent::assignment::load_assignment(&connection, "unknown-assignment")
        .unwrap_err()
        .contains("agent_assignments 的 role"));
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
