// Real publisher and full temporary application DB. No SDK or success shim.
fn original_terminal_second_root(h: &AgentHarness) -> String {
    let db = db::open(&h.db_path).unwrap();
    db.execute(
        "UPDATE sentinel_scans SET attempt_count=2 WHERE id=?1",
        [&h.context.scan_id],
    )
    .unwrap();
    let plan = h.context.execution_plan.clone().with_attempt(2);
    persist_agent_execution_plan(
        &h.db_path,
        &h.context.scan_id,
        2,
        &h.context.target_url,
        &plan,
    )
    .unwrap();
    let current = runtime_open_run(&h.db_path, &h.context.scan_id, &h.context.route).unwrap();
    // Deliberate lower-level accounting issuer only, before any current SDK or
    // Native state. This is not proof of new-task creation or live SDK execution.
    let tx = db.unchecked_transaction().unwrap();
    crate::agent_runtime::multi_agent::budget::root::RootOwner::initialize_financial_fixture_for_test(&tx,&current.run_id).unwrap();
    tx.commit().unwrap();
    current.run_id
}
#[test]
fn original_terminal_identity_late_setup_exit_cannot_publish_current_root_without_native_state() {
    let h = single_finally_fixture("original-terminal-late");
    let original = OriginalAgentTerminalIdentity::capture(&h.context);
    let outcome = NativeAgentBackend.execute(&h.context);
    assert!(matches!(outcome, AgentTargetOutcome::Failed(_)));
    assert!(
        NativeAgentState::read(&h.db_path, &h.context.scan_id, &h.context.target_url).is_none()
    );
    let current = original_terminal_second_root(&h);
    assert_ne!(Some(&current), original.root_run_id.as_ref());
    let db = db::open(&h.db_path).unwrap();
    let before = single_finally_physical(&db);
    let result = record_runtime_terminal_facts_original(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        &original,
        &outcome,
    );
    assert!(
        result.is_err(),
        "a late original setup result must not adopt active attempt: {result:?}"
    );
    assert_eq!(
        single_finally_physical(&db),
        before,
        "no current Root finance, publication, legacy projection or business write"
    );
    drop(db);
    single_target_cleanup(h);
}
#[test]
fn original_terminal_identity_missing_captured_root_cannot_adopt_later_root() {
    let h = single_finally_fixture("original-terminal-no-root");
    let mut early = h.context.clone();
    early.run = None;
    let original = OriginalAgentTerminalIdentity::capture(&early);
    assert!(original.root_run_id.is_none());
    let outcome = NativeAgentBackend.execute(&h.context);
    let db = db::open(&h.db_path).unwrap();
    let before = single_finally_physical(&db);
    let result = record_runtime_terminal_facts_original(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        &original,
        &outcome,
    );
    assert!(
        result.is_err(),
        "an early scope without a Root cannot adopt a later original Root: {result:?}"
    );
    assert_eq!(single_finally_physical(&db), before);
    drop(db);
    single_target_cleanup(h);
}
#[test]
fn original_terminal_identity_foreign_captured_root_cannot_select_by_active_scope() {
    let h = single_finally_fixture("original-terminal-foreign-root");
    let mut original = OriginalAgentTerminalIdentity::capture(&h.context);
    original.root_run_id = Some(Uuid::new_v4().to_string());
    let outcome = NativeAgentBackend.execute(&h.context);
    let db = db::open(&h.db_path).unwrap();
    let before = single_finally_physical(&db);
    let result = record_runtime_terminal_facts_original(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        &original,
        &outcome,
    );
    assert!(
        result.is_err(),
        "a supplied missing original ID cannot select the current valid Root: {result:?}"
    );
    assert_eq!(single_finally_physical(&db), before);
    drop(db);
    single_target_cleanup(h);
}
#[test]
fn original_terminal_identity_current_original_publishes_and_saved_replay_does_not_write() {
    let h = single_finally_fixture("original-terminal-current");
    let original = OriginalAgentTerminalIdentity::capture(&h.context);
    let outcome = NativeAgentBackend.execute(&h.context);
    let first = record_runtime_terminal_facts_original(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        &original,
        &outcome,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        first.state,
        crate::agent_runtime::contract::TerminalState::Failed
    );
    let db = db::open(&h.db_path).unwrap();
    let before = single_finally_physical(&db);
    let replay = record_runtime_terminal_facts_original(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        &original,
        &outcome,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        (replay.state, replay.code, replay.reason),
        (first.state, first.code, first.reason)
    );
    assert_eq!(single_finally_physical(&db), before);
    drop(db);
    single_target_cleanup(h);
}
