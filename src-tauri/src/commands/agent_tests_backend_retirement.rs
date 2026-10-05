fn retirement_matrix(attempt: i64) -> JsonValue {
    serde_json::json!({"schemaVersion":1,"scanId":"agent-scan","attemptNumber":attempt,
        "targets":[{"url":"https://app.example.invalid","backend":"native"}],
        "requiresNode":true,"requiresBrowser":true})
}

#[test]
fn backend_retirement_matrix_never_silently_discards_invalid_targets() {
    for backend in [
        serde_json::json!("strix"),
        serde_json::json!("docker"),
        serde_json::json!("unknown"),
        JsonValue::Null,
    ] {
        let mut value = retirement_matrix(1);
        value["targets"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"url":"https://other.example.invalid","backend":backend}));
        assert!(ScanBackendPlan::from_json(&value).is_none(), "{backend}");
    }
    assert!(ScanBackendPlan::from_json(&retirement_matrix(1)).is_some());
}

#[test]
fn backend_retirement_invalid_attempt_matrix_never_falls_back_to_native_projection() {
    let (_root, db_path) = temp_database("backend-invalid-matrix");
    seed_scan(&db_path, "agent-scan", "scanning");
    seed_attempt_row(&db_path, 1, "initial");
    write_agent_checkpoint(
        &db_path,
        "agent-scan",
        "",
        "scan_backend_plan",
        &retirement_matrix(1),
    )
    .unwrap();
    let connection = db::open(&db_path).unwrap();
    let mut retired = retirement_matrix(1);
    retired["targets"][0]["backend"] = serde_json::json!("strix");
    for raw in ["{".to_string(), retired.to_string(), "null".into()] {
        connection
            .execute(
                "UPDATE sentinel_scan_attempts SET backend_plan_json=?1 WHERE scan_id='agent-scan'",
                [&raw],
            )
            .unwrap();
        assert_ne!(
            agent_select_backend(
                &db_path,
                "agent-scan",
                1,
                "https://app.example.invalid",
                &JsonValue::Null,
                true
            ),
            AgentBackendKind::Native,
            "{raw}"
        );
    }
}

#[test]
fn backend_retirement_stored_target_plan_and_backend_must_both_be_current() {
    let (_root, db_path) = temp_database("backend-invalid-plan");
    seed_scan(&db_path, "agent-scan", "scanning");
    let plan = test_plan("standard");
    persist_agent_execution_plan(&db_path, "agent-scan", 1, &plan.target_url, &plan).unwrap();
    let connection = db::open(&db_path).unwrap();
    let native = plan.as_json();
    let mut retired = native.clone();
    retired["backend"] = serde_json::json!("strix");
    for (backend, raw) in [
        ("native", retired.to_string()),
        ("strix", native.to_string()),
        ("native", "{".into()),
    ] {
        connection
            .execute(
                "UPDATE agent_runs SET backend=?1,plan_json=?2 WHERE scan_id='agent-scan'",
                params![backend, raw],
            )
            .unwrap();
        assert_ne!(
            agent_select_backend(
                &db_path,
                "agent-scan",
                1,
                &plan.target_url,
                &JsonValue::Null,
                true
            ),
            AgentBackendKind::Native
        );
    }
}

#[test]
fn backend_retirement_runtime_report_never_defaults_missing_or_invalid_plan_to_native() {
    let (_root, db_path) = temp_database("backend-invalid-report");
    seed_scan(&db_path, "agent-scan", "scanning");
    let route = test_route("standard", "framework_application");
    for value in [
        JsonValue::Null,
        serde_json::json!({"attemptNumber":1,"backend":"strix"}),
        serde_json::json!({"attemptNumber":1,"backend":"unknown"}),
    ] {
        write_agent_checkpoint(
            &db_path,
            "agent-scan",
            &route.url,
            "agent_execution_plan",
            &value,
        )
        .unwrap();
        assert_ne!(
            runtime_report(&db_path, "agent-scan", &route).backend,
            AgentBackendKind::Native
        );
    }
}

#[test]
fn backend_retirement_missing_storage_is_not_permission_to_select_native() {
    let (_root, db_path) = temp_database("backend-storage-rejection");
    seed_scan(&db_path, "agent-scan", "scanning");
    let connection = db::open(&db_path).unwrap();
    connection
        .execute_batch("DROP TABLE sentinel_checkpoints")
        .unwrap();
    assert_ne!(
        agent_select_backend(
            &db_path,
            "agent-scan",
            1,
            "https://app.example.invalid",
            &JsonValue::Null,
            true
        ),
        AgentBackendKind::Native
    );
}

#[test]
fn backend_retirement_current_native_json_roundtrips_without_mutation() {
    let (_root, db_path) = temp_database("backend-current-native");
    seed_scan(&db_path, "agent-scan", "scanning");
    seed_attempt_row(&db_path, 1, "initial");
    let plan = test_plan("standard");
    let original = plan.as_json();
    persist_agent_execution_plan(&db_path, "agent-scan", 1, &plan.target_url, &plan).unwrap();
    let matrix = retirement_matrix(1);
    write_scan_backend_plan(&db_path, &ScanBackendPlan::from_json(&matrix).unwrap()).unwrap();
    assert_eq!(
        persisted_attempt_backend(&db_path, "agent-scan", 1, &plan.target_url).unwrap(),
        Some(AgentBackendKind::Native)
    );
    assert_eq!(
        agent_select_backend(
            &db_path,
            "agent-scan",
            1,
            &plan.target_url,
            &JsonValue::Null,
            true
        ),
        AgentBackendKind::Native
    );
    let report = runtime_report(
        &db_path,
        "agent-scan",
        &test_route("standard", "framework_application"),
    );
    assert_eq!(report.backend, AgentBackendKind::Native);
    assert_eq!(report.plan_json, original);
    assert_eq!(
        frozen_plan_of(&db_path, "agent-scan", 1, &plan.target_url),
        original
    );
}

#[test]
fn backend_retirement_matrix_scope_and_unique_targets_are_required() {
    let mut wrong_scan = retirement_matrix(1);
    wrong_scan["scanId"] = serde_json::json!("another-scan");
    let mut duplicate = retirement_matrix(1);
    let target = duplicate["targets"][0].clone();
    duplicate["targets"].as_array_mut().unwrap().push(target);
    let mut empty = retirement_matrix(1);
    empty["targets"][0]["url"] = serde_json::json!(" ");
    for value in [wrong_scan, retirement_matrix(2), duplicate, empty] {
        assert!(checked_backend_matrix(&value, "agent-scan", 1).is_err());
    }
    assert!(checked_backend_matrix(&retirement_matrix(1), "agent-scan", 1).is_ok());
}

#[test]
fn backend_retirement_native_matrix_cannot_override_corrupt_target_authority() {
    let (_root, db_path) = temp_database("backend-target-authority");
    seed_scan(&db_path, "agent-scan", "scanning");
    seed_attempt_row(&db_path, 1, "initial");
    let plan = test_plan("standard");
    persist_agent_execution_plan(&db_path, "agent-scan", 1, &plan.target_url, &plan).unwrap();
    write_scan_backend_plan(
        &db_path,
        &ScanBackendPlan::from_json(&retirement_matrix(1)).unwrap(),
    )
    .unwrap();
    db::open(&db_path)
        .unwrap()
        .execute(
            "UPDATE agent_runs SET plan_json='{' WHERE scan_id='agent-scan'",
            [],
        )
        .unwrap();
    assert_ne!(
        agent_select_backend(
            &db_path,
            "agent-scan",
            1,
            &plan.target_url,
            &JsonValue::Null,
            true
        ),
        AgentBackendKind::Native
    );
}
