// Actual fresh Single producer with received SDK response and absent usage.
// Incoming completion reports must replay the original debt, never authorize work.
#[test]
fn fresh_single_unknown_usage_canonical_replay_retains_original_debt() {
    use crate::agent_runtime::multi_agent::budget;
    let mut h = fresh_single_production_harness("unknown-usage-canonical-replay", mock_site);
    freeze_fresh_single_production_harness(&mut h);
    let mut reply: JsonValue = serde_json::from_str(&model_round(
        &[(
            "finish_target",
            json!({"coverage":closing_ledger(&[]),"stopReason":"unknown usage"}),
        )],
        100,
    ))
    .unwrap();
    reply.as_object_mut().unwrap().remove("usage");
    retarget_model(&mut h, vec![reply.to_string()]);
    let owned = execute_fresh_single_production_harness(&mut h);
    assert!(
        matches!(owned.outcome, AgentTargetOutcome::Incomplete(_)),
        "{:?}",
        owned.outcome
    );
    assert_eq!(
        owned.outcome.terminal_code(),
        terminal_code::REQUEST_RECONCILIATION_REQUIRED
    );
    assert_eq!(h.model_seen.lock().unwrap().len(), 1);
    assert!(h.site_seen.lock().unwrap().is_empty());
    let db = db::open(&h.db_path).unwrap();
    let root = owned.original_terminal.root_run_id.as_ref().unwrap();
    budget::root::RootOwner::load_single(&db, root)
        .unwrap()
        .read_single_exit(&db)
        .unwrap();
    let balance = |dimension| budget::balance(&db, root, None, dimension).unwrap();
    let request = balance("model_requests");
    assert_eq!(
        (request.consumed, request.reserved, request.indeterminate),
        (1, 0, 0)
    );
    for dimension in budget::DIMENSIONS[..3].iter().copied() {
        assert!(balance(dimension).indeterminate > 0, "{dimension}");
    }
    assert_eq!(balance("target_requests").consumed, 0);
    assert!(
        NativeAgentState::load(&h.db_path, &h.context.scan_id, &h.context.target_url)
            .unwrap()
            .is_none()
    );
    assert!(findings_for(&h.db_path, AGENT_COVERAGE_STAGE).is_empty());
    assert!(findings_for(&h.db_path, AGENT_VULNERABILITY_STAGE).is_empty());
    let mut tally = AgentPipelineTally::default();
    assert!(record_owned_agent_target_outcome(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        &owned,
        &mut tally
    ));
    assert_eq!(tally.counted(), 1);
    assert_eq!(tally.failed, 0);
    let terminal = read_agent_checkpoint(
        &h.db_path,
        &h.context.scan_id,
        &h.context.target_url,
        "agent_terminal",
    );
    assert_eq!(terminal["status"], "paused");
    assert_eq!(
        terminal["stop"]["code"],
        terminal_code::REQUEST_RECONCILIATION_REQUIRED
    );
    let original = single_finally_physical(&db);
    for _ in 0..2 {
        assert!(record_owned_agent_target_outcome(
            &h.db_path,
            &h.context.scan_id,
            &h.context.route,
            &owned,
            &mut tally
        ));
        assert!(record_agent_target_outcome(
            &h.db_path,
            &h.context.scan_id,
            &h.context.route,
            AgentTargetOutcome::Completed(AgentCompletion::without_ledger(
                "forged debt closure",
                AGENT_STOP_FINISH
            )),
            &mut tally
        ));
        let after = read_agent_checkpoint(
            &h.db_path,
            &h.context.scan_id,
            &h.context.target_url,
            "agent_terminal",
        );
        assert_eq!(
            after, terminal,
            "incoming completion cannot rewrite the original stop or completion"
        );
        assert_eq!(tally.counted(), 1);
        assert!(
            single_finally_physical(&db) == original,
            "all original typed rows/rowids/fees remain unchanged"
        );
    }
    assert_eq!(h.model_seen.lock().unwrap().len(), 1);
    assert!(h.site_seen.lock().unwrap().is_empty());
    drop(owned);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}
