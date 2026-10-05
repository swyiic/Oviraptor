#[test]
fn single_finally_actual_pipeline_counts_reduced_obligations_not_completed_label() {
    let h = single_projection_frozen_finance("single-pipeline-gap");
    let completion = AgentCompletion {
        summary: "declared remaining obligation".into(),
        terminal_code: AGENT_STOP_FINISH,
        ledger_reported: true,
        model_requests: 0,
        total_tokens: 0,
        verified_tool_results: 0,
        covered_families: vec![],
        uncovered_families: vec!["authorization".into()],
        confirmed_findings: 0,
    };
    let mut tally = AgentPipelineTally::default();
    record_agent_target_outcome(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        AgentTargetOutcome::Completed(completion),
        &mut tally,
    );
    assert_eq!(
        (tally.completed, tally.partial),
        (0, 1),
        "the reducer still has an evidence obligation"
    );
    let db = db::open(&h.db_path).unwrap();
    let text: String = db
        .query_row(
            "SELECT raw_json FROM sentinel_checkpoints WHERE scan_id=?1 AND stage='agent_terminal'",
            [&h.context.scan_id],
            |r| r.get(0),
        )
        .unwrap();
    let terminal: JsonValue = serde_json::from_str(&text).unwrap();
    assert_eq!(terminal["status"], "paused");
    assert_eq!(terminal["code"], AGENT_STOP_DERIVED);
    drop(db);
    single_target_cleanup(h);
}
#[test]
fn single_finally_actual_pipeline_pause_projects_pause_without_terminal_checkpoint() {
    let h = single_projection_frozen_finance("single-pipeline-pause");
    NativeAgentState::fresh(
        h.context.attempt_number,
        AgentBackendKind::Native,
        &agent_stable_hash(&h.context.evidence),
        &h.context.execution_plan.hash(),
        vec![],
    )
    .persist(&h.db_path, &h.context.scan_id, &h.context.target_url)
    .unwrap();
    let db = db::open(&h.db_path).unwrap();
    db.execute(
        "UPDATE sentinel_scans SET status='pausing' WHERE id=?1",
        [&h.context.scan_id],
    )
    .unwrap();
    drop(db);
    let mut tally = AgentPipelineTally::default();
    assert!(!record_agent_target_outcome(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        AgentTargetOutcome::Cancelled,
        &mut tally
    ));
    assert_eq!(tally.counted(), 0);
    let db = db::open(&h.db_path).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM sentinel_checkpoints WHERE scan_id=?1 AND stage='agent_terminal'",
            [&h.context.scan_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0,
        "pause cannot create a cancelled terminal checkpoint"
    );
    let (root,target):(String,String)=db.query_row("SELECT r.status,t.status FROM agent_runs r JOIN sentinel_targets t ON t.scan_id=r.scan_id AND t.url=r.target_url WHERE r.id=?1",[&h.context.run.as_ref().unwrap().run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!((root, target), ("paused".into(), "paused".into()));
    drop(db);
    single_target_cleanup(h);
}
