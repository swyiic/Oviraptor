use super::*;

#[test]
fn backend_retirement_rejects_old_aliases_in_text_and_serde() {
    for word in ["strix", " Strix ", "docker", "unknown", ""] {
        assert_eq!(AgentBackendKind::parse(word), None, "{word}");
        assert!(serde_json::from_value::<AgentBackendKind>(json!(word)).is_err());
    }
    for backend in [AgentBackendKind::Native, AgentBackendKind::LegacyRemoved] {
        assert_eq!(AgentBackendKind::parse(backend.as_str()), Some(backend));
        assert_eq!(
            serde_json::from_value::<AgentBackendKind>(serde_json::to_value(backend).unwrap())
                .unwrap(),
            backend
        );
    }
}

#[test]
fn backend_retirement_unknown_stored_backend_is_not_a_native_run() {
    let (_root, path) = temp_db("backend-rejection");
    let connection = crate::db::open(&path).unwrap();
    seeded(&connection, "backend-rejection");
    let row = run_row("backend-rejection", "https://app.example.invalid");
    store::create_run(&connection, &row).unwrap();
    for word in ["strix", "docker", "unknown", "", "Bearer private-fixture"] {
        connection
            .execute(
                "UPDATE agent_runs SET backend=?1 WHERE id=?2",
                rusqlite::params![word, row.id],
            )
            .unwrap();
        let error = store::load_run(&connection, &row.id).unwrap_err();
        assert!(error.contains("backend"), "{error}");
        assert!(!error.contains("private-fixture"));
        assert!(store::find_run(
            &connection,
            "backend-rejection",
            1,
            &row.target_url,
            AgentRole::Coordinator
        )
        .is_err());
    }
    connection
        .execute(
            "UPDATE agent_runs SET backend='native' WHERE id=?1",
            [&row.id],
        )
        .unwrap();
    assert_eq!(
        store::load_run(&connection, &row.id)
            .unwrap()
            .unwrap()
            .backend,
        AgentBackendKind::Native
    );
    assert!(store::load_run(&connection, "missing").unwrap().is_none());
}

#[test]
fn backend_retirement_non_native_report_cannot_create_an_active_run() {
    let (_root, path) = temp_db("backend-report-rejection");
    let connection = crate::db::open(&path).unwrap();
    seeded(&connection, "backend-rejection");
    let report = BackendReport::new(
        "backend-rejection",
        1,
        "https://app.example.invalid",
        AgentBackendKind::LegacyRemoved,
    );
    assert!(runtime_adapter::open_run(&connection, &report).is_err());
    for table in ["agent_runs", "agent_events"] {
        assert_eq!(
            connection
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn backend_retirement_native_report_cannot_reactivate_non_native_row() {
    let (_root, path) = temp_db("backend-existing-rejection");
    let connection = crate::db::open(&path).unwrap();
    seeded(&connection, "backend-rejection");
    let mut row = run_row("backend-rejection", "https://app.example.invalid");
    row.backend = AgentBackendKind::LegacyRemoved;
    assert_eq!(
        store::create_run(&connection, &row).unwrap_err(),
        "agent_runs_backend_unsupported"
    );
    row.backend = AgentBackendKind::Native;
    store::create_run(&connection, &row).unwrap();
    connection
        .execute(
            "UPDATE agent_runs SET backend='legacy_backend_removed' WHERE id=?1",
            [&row.id],
        )
        .unwrap();
    let before = connection
        .query_row(
            "SELECT status,plan_hash,hard_token_budget FROM agent_runs WHERE id=?1",
            [&row.id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .unwrap();
    let report = BackendReport::new(
        "backend-rejection",
        1,
        &row.target_url,
        AgentBackendKind::Native,
    );
    assert!(runtime_adapter::open_run(&connection, &report).is_err());
    let after = connection
        .query_row(
            "SELECT status,plan_hash,hard_token_budget FROM agent_runs WHERE id=?1",
            [&row.id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(before, after);
}

#[test]
fn run_store_cannot_relabel_or_advance_an_unsealed_retired_row() {
    let (_root, path) = temp_db("backend-store-fence");
    let connection = crate::db::open(&path).unwrap();
    seeded(&connection, "backend-store-fence");
    let row = run_row("backend-store-fence", "https://app.example.invalid");
    store::create_run(&connection, &row).unwrap();
    connection
        .execute(
            "UPDATE agent_runs SET backend='strix',status='running' WHERE id=?1",
            [&row.id],
        )
        .unwrap();

    assert_eq!(
        store::create_run(&connection, &row).unwrap_err(),
        "agent_runs_identity_or_backend_conflict"
    );
    let error = store::set_run_status(&connection, &row.id, AgentRunStatus::Terminal).unwrap_err();
    assert!(
        error.contains("非 Native"),
        "旧行不能通过状态写端继续执行：{error}"
    );
    let actual: (String, String) = connection
        .query_row(
            "SELECT backend,status FROM agent_runs WHERE id=?1",
            [&row.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        actual,
        ("strix".into(), "running".into()),
        "被拒操作不得改变历史行"
    );

    connection
        .execute(
            "UPDATE agent_runs SET backend='native' WHERE id=?1",
            [&row.id],
        )
        .unwrap();
    let mut different_target = row.clone();
    different_target.target_url = "https://other.example.invalid".into();
    assert_eq!(
        store::create_run(&connection, &different_target).unwrap_err(),
        "agent_runs_identity_or_backend_conflict"
    );
    assert_eq!(
        store::set_run_status(&connection, "missing-run", AgentRunStatus::Running).unwrap_err(),
        "agent_run_not_found"
    );
}

#[test]
fn retired_backend_cannot_record_an_attempt_plan_or_change_its_projection() {
    let (_root, path) = temp_db("retired-plan-writer");
    let connection = crate::db::open(&path).unwrap();
    seeded(&connection, "retired-plan-writer");
    let target = "https://app.example.invalid";
    let plan = json!({"attemptNumber": 1, "backend": "native"});

    assert_eq!(
        store::record_attempt_plan(
            &connection,
            "retired-plan-writer",
            1,
            target,
            AgentBackendKind::LegacyRemoved,
            "plan-hash",
            &plan,
        )
        .unwrap_err(),
        "agent_runs_backend_unsupported"
    );
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM agent_runs", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );

    let row = run_row("retired-plan-writer", target);
    store::create_run(&connection, &row).unwrap();
    connection
        .execute(
            "UPDATE agent_runs SET backend='strix',status='running' WHERE id=?1",
            [&row.id],
        )
        .unwrap();
    assert_eq!(
        store::record_attempt_plan(
            &connection,
            "retired-plan-writer",
            1,
            target,
            AgentBackendKind::Native,
            "plan-hash",
            &plan,
        )
        .unwrap_err(),
        "agent_runs_backend_unsupported"
    );
    let (hash, json): (String, String) = connection
        .query_row(
            "SELECT plan_hash,plan_json FROM agent_runs WHERE id=?1",
            [&row.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(hash, "plan-hash");
    assert_eq!(json, "{}");
    let projected: i64 = connection
        .query_row("SELECT count(*) FROM sentinel_checkpoints", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(projected, 0);
}

#[test]
fn retired_or_missing_run_cannot_receive_plan_budget_usage_or_lease() {
    let (_root, path) = temp_db("retired-run-writers");
    let connection = crate::db::open(&path).unwrap();
    seeded(&connection, "retired-run-writers");
    let row = run_row("retired-run-writers", "https://app.example.invalid");
    store::create_run(&connection, &row).unwrap();
    let usage = store::UsageDelta {
        total_tokens: 12,
        cached_input_tokens: 2,
        model_requests: 1,
        ..Default::default()
    };
    for (backend, status) in [("strix", "running"), ("native", "legacy_backend_removed")] {
        connection
            .execute(
                "UPDATE agent_runs SET backend=?1,status=?2 WHERE id=?3",
                rusqlite::params![backend, status, row.id],
            )
            .unwrap();
        assert!(store::apply_run_plan(&connection, &row.id, "changed", 1, 2, 3, 4).is_err());
        assert!(store::settle_usage(&connection, &row.id, &usage).is_err());
        assert!(store::renew_lease(&connection, &row.id, 3).is_err());
    }
    for id in ["missing", &row.id] {
        assert!(store::apply_run_plan(&connection, id, "changed", 1, 2, 3, 4).is_err());
        assert!(store::settle_usage(&connection, id, &usage).is_err());
        assert!(store::renew_lease(&connection, id, 3).is_err());
    }
    let (hash, tokens, requests, expires): (String, i64, i64, String) = connection
        .query_row(
            "SELECT plan_hash,used_tokens,used_requests,lease_expires_at FROM agent_runs WHERE id=?1",
            [&row.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_ne!(hash, "changed");
    assert_eq!((tokens, requests, expires.as_str()), (0, 0, ""));
}
