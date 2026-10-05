#[test]
fn a_host_candidate_write_failure_rolls_back_the_chat_draft() {
    use crate::agent_runtime::multi_agent::directive;

    let (root, db_path, root_run_id, lease) = multi_agent_test_root("directive-host-atomic", 200, 4);
    let connection = db::open(&db_path).unwrap();
    connection.execute_batch(
        "CREATE TRIGGER test_refuse_host_candidate BEFORE INSERT ON agent_host_boundary_candidates \
         BEGIN SELECT RAISE(ABORT,'test_refusal'); END;",
    ).unwrap();
    assert!(directive::create_draft(
        &connection, "scan-directive-host-atomic", 1, &root_run_id,
        "https://authorized.example.test", "coordinator", "请用 SSH 登录主机验证",
        lease.lease_epoch, &lease.fencing_token,
    ).unwrap_err().contains("test_refusal"));
    for table in ["agent_directive_drafts", "agent_host_boundary_candidates"] {
        let count: i64 = connection.query_row(
            &format!("SELECT COUNT(*) FROM {table} WHERE scan_id='scan-directive-host-atomic'"),
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 0, "{table} must roll back");
    }
    let events: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_collaboration_events \
         WHERE scan_id='scan-directive-host-atomic' AND event_type IN ('directive_draft','host_boundary_candidate')",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(events, 0, "the draft and boundary notifications must roll back together");
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn directive_confirmation_is_bound_to_scan_root_and_coordinator_fencing() {
    use crate::agent_runtime::{
        contract::{AgentBackendKind, AgentLane, AgentRole, AgentRunStatus, MultiAgentPolicy},
        multi_agent::{directive, lease},
        store::{self, AgentRunRow},
    };

    let (root, db_path, first_root, old_lease) = multi_agent_test_root("directive-fencing", 200, 4);
    let connection = db::open(&db_path).unwrap();
    let draft = directive::create_draft(
        &connection,
        "scan-directive-fencing",
        1,
        &first_root,
        "https://authorized.example.test",
        "coordinator",
        "优先验证登录接口",
        old_lease.lease_epoch,
        &old_lease.fencing_token,
    )
    .unwrap();
    assert_eq!(
        directive::confirm_draft(
            &connection,
            "another-scan",
            1,
            &first_root,
            "https://authorized.example.test",
            &draft.id,
            draft.revision,
            &draft.draft_hash,
        )
        .unwrap_err(),
        "directive_draft_scope_mismatch"
    );

    let second_root = "root-directive-fencing-second";
    let mut second_run = AgentRunRow::new(
        second_root,
        "scan-directive-fencing",
        1,
        "https://authorized.example.test",
        AgentBackendKind::Native,
        AgentRole::Coordinator,
        "plan-2",
        "evidence-2",
    )
    .with_budget(100, 200, 2, 4);
    second_run.status = AgentRunStatus::Running;
    second_run.root_run_id = second_root.into();
    second_run.orchestration_policy = MultiAgentPolicy::Multi;
    second_run.lane = Some(AgentLane::ReadOnlyAnalysis);
    store::create_run(&connection, &second_run).unwrap();
    connection
        .execute(
            "UPDATE agent_coordinator_leases SET lease_expires_at=datetime('now','-1 second','localtime') WHERE root_run_id=?1",
            [&first_root],
        )
        .unwrap();
    lease::acquire_coordinator_lease(
        &connection,
        "scan-directive-fencing",
        1,
        "https://authorized.example.test",
        second_root,
        600,
    )
    .unwrap();
    assert_eq!(
        directive::confirm_draft(
            &connection,
            "scan-directive-fencing",
            1,
            second_root,
            "https://authorized.example.test",
            &draft.id,
            draft.revision,
            &draft.draft_hash,
        )
        .unwrap_err(),
        "directive_draft_root_binding_changed"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM agent_user_directives WHERE source_draft_id=?1",
                [&draft.id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn coordinator_claim_rejects_legacy_or_tampered_pending_directives() {
    use crate::agent_runtime::multi_agent::directive;

    let (root, db_path, root_run_id, lease) =
        multi_agent_test_root("directive-claim-guard", 200, 4);
    let connection = db::open(&db_path).unwrap();
    connection
        .execute(
            "INSERT INTO agent_user_directives(id,scan_id,attempt_number,text_redacted,status) \
             VALUES('legacy-unconfirmed','scan-directive-claim-guard',1,'直接执行旧指令','pending')",
            [],
        )
        .unwrap();

    let draft = directive::create_draft(
        &connection,
        "scan-directive-claim-guard",
        1,
        &root_run_id,
        "https://authorized.example.test",
        "coordinator",
        "优先核对 G-19 的证据",
        lease.lease_epoch,
        &lease.fencing_token,
    )
    .unwrap();
    let confirmed = directive::confirm_draft(
        &connection,
        "scan-directive-claim-guard",
        1,
        &root_run_id,
        "https://authorized.example.test",
        &draft.id,
        draft.revision,
        &draft.draft_hash,
    )
    .unwrap();

    let claimed = directive::claim_pending_directives(&connection, &lease, 20).unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].id, confirmed.id);
    let legacy: (String, String) = connection
        .query_row(
            "SELECT status,rejection_code FROM agent_user_directives WHERE id='legacy-unconfirmed'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(legacy, ("rejected".into(), "unconfirmed_directive_blocked".into()));

    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}
