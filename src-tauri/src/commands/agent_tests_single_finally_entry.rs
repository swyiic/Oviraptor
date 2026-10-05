#[test]
fn single_finally_actual_backend_early_model_setup_failure_saves_original_elapsed() {
    let harness = single_finally_fixture("single-finally-actual-setup");
    let db = db::open(&harness.db_path).unwrap();
    let sources =
        single_finally_original_sources(&db, &harness.context.run.as_ref().unwrap().run_id);
    drop(db);
    let lower = single_finally_lower_bound(&harness);
    let outcome = NativeAgentBackend.execute(&harness.context);
    assert!(
        matches!(outcome, AgentTargetOutcome::Failed(_)),
        "{outcome:?}"
    );
    assert!(
        outcome.detail().contains("缺少模型名"),
        "original failure must survive: {outcome:?}"
    );
    single_finally_verify_cost(&harness, lower, &sources);
    single_target_cleanup(harness);
}

#[test]
fn single_finally_actual_old_attempt_exit_records_only_original_financial_elapsed() {
    let harness = single_finally_fixture("single-finally-old-callback");
    let db = db::open(&harness.db_path).unwrap();
    db.execute(
        "UPDATE sentinel_scans SET attempt_count=2 WHERE id=?1",
        [&harness.context.scan_id],
    )
    .unwrap();
    let sources =
        single_finally_original_sources(&db, &harness.context.run.as_ref().unwrap().run_id);
    drop(db);
    let lower = single_finally_lower_bound(&harness);
    let outcome = NativeAgentBackend.execute(&harness.context);
    assert_eq!(outcome, AgentTargetOutcome::Cancelled);
    single_finally_verify_cost(&harness, lower, &sources);
    let db = db::open(&harness.db_path).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT attempt_count FROM sentinel_scans WHERE id=?1",
            [&harness.context.scan_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_runs WHERE attempt_number=2",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0,
        "financial closure must not create current-attempt work"
    );
    drop(db);
    single_target_cleanup(harness);
}

#[test]
fn single_finally_actual_backend_failed_write_returns_persistence_and_rolls_back_full_scope() {
    for fault in ["ignore", "abort", "fail", "business", "other_root"] {
        let harness = single_finally_fixture(&format!("single-finally-fault-{fault}"));
        let db = db::open(&harness.db_path).unwrap();
        let root = &harness.context.run.as_ref().unwrap().run_id;
        let sibling = crate::agent_runtime::store::AgentRunRow::new(
            Uuid::new_v4().to_string(),
            harness.context.scan_id.clone(),
            1,
            format!("{}/sibling", harness.context.target_url),
            AgentBackendKind::Native,
            crate::agent_runtime::contract::AgentRole::Coordinator,
            "sibling-plan",
            "sibling-evidence",
        );
        crate::agent_runtime::store::create_run(&db, &sibling).unwrap();
        let body = match fault {
            "ignore" => "SELECT RAISE(IGNORE);".to_string(),
            "abort" => "SELECT RAISE(ABORT,'single_finally_fault');".to_string(),
            "fail" => "SELECT RAISE(FAIL,'single_finally_fault');".to_string(),
            "business" => {
                "INSERT INTO projects(name) VALUES('forbidden-single-finally-effect');".to_string()
            }
            _ => format!(
                "UPDATE agent_runs SET used_tokens=used_tokens+999 WHERE id='{}';",
                sibling.id
            ),
        };
        let timing = if matches!(fault, "business" | "other_root") {
            "AFTER"
        } else {
            "BEFORE"
        };
        db.execute_batch(&format!("CREATE TRIGGER single_finally_fault {timing} INSERT ON agent_budget_entries WHEN NEW.root_run_id='{root}' AND NEW.dimension='wall_time_ms' AND NEW.kind='reserve' BEGIN {body} END;")).unwrap();
        let before = single_finally_physical(&db);
        drop(db);
        let outcome = NativeAgentBackend.execute(&harness.context);
        assert_eq!(
            outcome.terminal_code(),
            AGENT_STOP_PERSISTENCE,
            "fault {fault} cannot disappear because baseline never sampled elapsed: {outcome:?}"
        );
        assert!(
            outcome.detail().contains("缺少模型名"),
            "original exit reason must survive: {outcome:?}"
        );
        let db = db::open(&harness.db_path).unwrap();
        assert_eq!(
            single_finally_physical(&db),
            before,
            "full financial/application/rowid rollback: {fault}"
        );
        assert!(harness.model_seen.lock().unwrap().is_empty());
        assert!(harness.site_seen.lock().unwrap().is_empty());
        drop(db);
        single_target_cleanup(harness);
    }
}
