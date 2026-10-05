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
