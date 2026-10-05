fn single_finally_projection_contract(tag: &str, outcome: AgentTargetOutcome) {
    let harness = single_finally_fixture(tag);
    let db = db::open(&harness.db_path).unwrap();
    let sources =
        single_finally_original_sources(&db, &harness.context.run.as_ref().unwrap().run_id);
    drop(db);
    let lower = single_finally_lower_bound(&harness);
    // This is the actual production reporter, not a new finalizer shape API.
    // Typed outcome injection proves all financial branches, not actual role
    // model/HTTP completion or reviewer acceptance.
    record_runtime_terminal_facts(
        &harness.db_path,
        &harness.context.scan_id,
        &harness.context.route,
        &outcome,
    );
    single_finally_verify_cost(&harness, lower, &sources);
    let db = db::open(&harness.db_path).unwrap();
    let root = &harness.context.run.as_ref().unwrap().run_id;
    let finished: String = db
        .query_row(
            "SELECT finished_at FROM agent_runs WHERE id=?1",
            [root],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!finished.is_empty());
    let wall = crate::agent_runtime::multi_agent::budget::balance(&db, root, None, "wall_time_ms")
        .unwrap();
    let before = single_finally_physical(&db);
    drop(db);
    // Fixed finished_at replay: no second cost, projection or snapshot/event.
    record_runtime_terminal_facts(
        &harness.db_path,
        &harness.context.scan_id,
        &harness.context.route,
        &outcome,
    );
    let db = db::open(&harness.db_path).unwrap();
    assert_eq!(
        single_finally_physical(&db),
        before,
        "terminal replay must be purely original receipt read"
    );
    assert_eq!(
        db.query_row(
            "SELECT finished_at FROM agent_runs WHERE id=?1",
            [root],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        finished
    );
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(&db, root, None, "wall_time_ms")
            .unwrap()
            .consumed,
        wall.consumed
    );
    drop(db);
    single_target_cleanup(harness);
}

macro_rules! single_finally_projection_case {
    ($name:ident,$outcome:expr) => {
        #[test]
        fn $name() {
            single_finally_projection_contract(stringify!($name), $outcome);
        }
    };
}
single_finally_projection_case!(
    single_finally_completed_projection,
    AgentTargetOutcome::Completed(AgentCompletion::without_ledger(
        "financial branch only",
        AGENT_STOP_DERIVED
    ))
);
single_finally_projection_case!(
    single_finally_bounded_projection,
    AgentTargetOutcome::BoundedCompleted(AgentCompletion::bounded("financial branch only"))
);
single_finally_projection_case!(
    single_finally_incomplete_projection,
    AgentTargetOutcome::incomplete("financial branch only")
);
single_finally_projection_case!(
    single_finally_limited_projection,
    AgentTargetOutcome::limited("financial branch only")
);
single_finally_projection_case!(
    single_finally_failed_projection,
    AgentTargetOutcome::failed("financial branch only")
);
single_finally_projection_case!(
    single_finally_resume_incompatible_projection,
    AgentTargetOutcome::resume_incompatible("financial branch only")
);
single_finally_projection_case!(
    single_finally_cancelled_projection,
    AgentTargetOutcome::Cancelled
);
single_finally_projection_case!(
    single_finally_unknown_projection,
    AgentTargetOutcome::Incomplete(AgentStop::new(
        terminal_code::REQUEST_RECONCILIATION_REQUIRED,
        "unknown original work cannot resume"
    ))
);

#[test]
fn single_finally_actual_pause_projection_saves_elapsed_without_terminal_or_new_control() {
    let harness = single_finally_fixture("single-finally-pausing-projection");
    NativeAgentState::fresh(
        harness.context.attempt_number,
        AgentBackendKind::Native,
        &agent_stable_hash(&harness.context.evidence),
        &harness.context.execution_plan.hash(),
        vec![],
    )
    .persist(
        &harness.db_path,
        &harness.context.scan_id,
        &harness.context.target_url,
    )
    .unwrap();
    let db = db::open(&harness.db_path).unwrap();
    db.execute(
        "UPDATE sentinel_scans SET status='pausing' WHERE id=?1",
        [&harness.context.scan_id],
    )
    .unwrap();
    let root = &harness.context.run.as_ref().unwrap().run_id;
    let sources = single_finally_original_sources(&db, root);
    let native: String = db
        .query_row(
            "SELECT raw_json FROM sentinel_checkpoints WHERE scan_id=?1 AND url=?2 AND stage=?3",
            params![
                harness.context.scan_id,
                harness.context.target_url,
                NATIVE_AGENT_STATE_STAGE
            ],
            |r| r.get(0),
        )
        .unwrap();
    drop(db);
    let lower = single_finally_lower_bound(&harness);
    // Existing pause vocabulary is Cancelled; finance must save yield elapsed.
    // This test does not claim the cancelled spelling is a completed pause API.
    record_runtime_terminal_facts(
        &harness.db_path,
        &harness.context.scan_id,
        &harness.context.route,
        &AgentTargetOutcome::Cancelled,
    );
    single_finally_verify_cost(&harness, lower, &sources);
    let db = db::open(&harness.db_path).unwrap();
    let state: (String, String, String) = db
        .query_row(
            "SELECT status,terminal_state,finished_at FROM agent_runs WHERE id=?1",
            [root],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(state, ("paused".into(), String::new(), String::new()));
    assert_eq!(
        db.query_row(
            "SELECT raw_json FROM sentinel_checkpoints WHERE scan_id=?1 AND url=?2 AND stage=?3",
            params![
                harness.context.scan_id,
                harness.context.target_url,
                NATIVE_AGENT_STATE_STAGE
            ],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        native
    );
    assert!(
        crate::agent_runtime::multi_agent::budget::root::RootOwner::load_single(&db, root)
            .unwrap()
            .require_live(&db)
            .is_err(),
        "pause financial closure cannot issue next work"
    );
    drop(db);
    single_target_cleanup(harness);
}
