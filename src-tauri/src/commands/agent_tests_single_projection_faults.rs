// Existing public pipeline/reporting entries, not a new test-only finalizer.
#[test]
fn single_finally_existing_failed_report_must_not_reduce_to_completed() {
    let h = single_finally_fixture("single-failed-truth");
    record_runtime_terminal_facts(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        &AgentTargetOutcome::failed("original failed execution"),
    );
    let db = db::open(&h.db_path).unwrap();
    let root = &h.context.run.as_ref().unwrap().run_id;
    let state: String = db
        .query_row(
            "SELECT terminal_state FROM agent_runs WHERE id=?1",
            [root],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        state, "failed",
        "a generic backend failure cannot fall through to completed"
    );
    drop(db);
    single_target_cleanup(h);
}
#[test]
fn single_finally_existing_pipeline_terminal_ignore_must_not_count_success() {
    let h = single_finally_fixture("single-terminal-ignore");
    let db = db::open(&h.db_path).unwrap();
    db.execute_batch("CREATE TRIGGER single_terminal_ignore BEFORE UPDATE OF status ON agent_runs WHEN NEW.status='terminal' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    drop(db);
    let mut tally = AgentPipelineTally::default();
    record_agent_target_outcome(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        AgentTargetOutcome::Completed(AgentCompletion::without_ledger(
            "projection only",
            AGENT_STOP_DERIVED,
        )),
        &mut tally,
    );
    assert_eq!(tally.completed, 0, "silent terminal discard is not success");
    assert_eq!(
        tally.failed, 1,
        "publication failure must reach the actual pipeline"
    );
    let db = db::open(&h.db_path).unwrap();
    let text: String = db
        .query_row(
            "SELECT raw_json FROM sentinel_checkpoints WHERE scan_id=?1 AND stage='agent_terminal'",
            [&h.context.scan_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<JsonValue>(&text).unwrap()["code"],
        AGENT_STOP_PERSISTENCE
    );
    drop(db);
    single_target_cleanup(h);
}
