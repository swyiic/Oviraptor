// Original new creator and actual paid Root SDK. Corruption is temporary only.
fn multi_terminal_original_fixture(
    tag: &str,
) -> (
    RootTickFixture,
    Seen,
    OriginalAgentTerminalIdentity,
    AgentTargetOutcome,
) {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            sdk_exact_model_body(&root_tick_valid_text("original terminal consumer")),
        )
    }));
    let f = root_tick_fixture(tag, &format!("http://127.0.0.1:{port}/v1"));
    let original = OriginalAgentTerminalIdentity::capture(&f.context);
    native_coordinator_tick(&f.context, &f.actor).unwrap();
    let outcome = AgentTargetOutcome::incomplete("original paid terminal reason");
    let db = db::open(&f.context.db_path).unwrap();
    finish_coordinator_run(&db, &f.actor, &outcome).unwrap();
    (f, seen, original, outcome)
}

#[test]
fn multi_terminal_original_changed_reason_cannot_publish_from_mutable_root() {
    let (f, seen, original, outcome) = multi_terminal_original_fixture("multi-terminal-reason");
    let db = db::open(&f.context.db_path).unwrap();
    db.execute(
        "UPDATE agent_runs SET terminal_reason='changed after original paid exit' WHERE id=?1",
        [&f.actor.root_run_id],
    )
    .unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert!(
        record_runtime_terminal_facts_original(
            &f.context.db_path,
            &f.actor.scan_id,
            &f.context.route,
            &original,
            &outcome
        )
        .is_err(),
        "terminal consumer trusted the mutable reason instead of the original financial exit"
    );
    assert!(super::tests::application_table_snapshot(&db) == before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn multi_terminal_original_bad_proof_cannot_project_failure_or_count_result() {
    let (f, seen, original, outcome) =
        multi_terminal_original_fixture("multi-terminal-no-failed-projection");
    let db = db::open(&f.context.db_path).unwrap();
    db.execute(
        "UPDATE agent_runs SET terminal_reason='corrupt original closure' WHERE id=?1",
        [&f.actor.root_run_id],
    )
    .unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let mut tally = AgentPipelineTally::default();
    assert!(
        record_agent_target_outcome_original(
            &f.context.db_path,
            &f.actor.scan_id,
            &f.context.route,
            &original,
            outcome,
            &mut tally
        )
        .is_err(),
        "invalid original proof must stop before derived target/checkpoint/fuse projection"
    );
    assert_eq!(tally.counted(), 0);
    assert!(super::tests::application_table_snapshot(&db) == before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn multi_terminal_original_valid_paid_replay_preserves_invoice_and_counts_once() {
    let (f, seen, original, outcome) = multi_terminal_original_fixture("multi-terminal-replay");
    let db = db::open(&f.context.db_path).unwrap();
    let mut tally = AgentPipelineTally::default();
    assert!(record_agent_target_outcome_original(
        &f.context.db_path,
        &f.actor.scan_id,
        &f.context.route,
        &original,
        outcome.clone(),
        &mut tally
    )
    .unwrap());
    assert_eq!((tally.counted(), tally.partial), (1, 1));
    let before = super::tests::application_table_snapshot(&db);
    assert!(record_agent_target_outcome_original(
        &f.context.db_path,
        &f.actor.scan_id,
        &f.context.route,
        &original,
        outcome,
        &mut tally
    )
    .unwrap());
    assert_eq!((tally.counted(), tally.partial), (1, 1));
    assert!(super::tests::application_table_snapshot(&db) == before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn multi_terminal_original_missing_receipt_is_not_backfilled_or_projected() {
    let (f, seen, original, outcome) = multi_terminal_original_fixture("multi-terminal-missing");
    let db = db::open(&f.context.db_path).unwrap();
    db.execute_batch("DROP TRIGGER multi_exit_no_delete")
        .unwrap();
    db.execute(
        "DELETE FROM agent_multi_exit_receipts WHERE root_run_id=?1",
        [&f.actor.root_run_id],
    )
    .unwrap();
    db.execute_batch(MULTI_EXIT_RECEIPT_SCHEMA).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let mut tally = AgentPipelineTally::default();
    assert_eq!(
        record_agent_target_outcome_original(
            &f.context.db_path,
            &f.actor.scan_id,
            &f.context.route,
            &original,
            outcome,
            &mut tally
        )
        .unwrap_err(),
        "budget_multi_exit_original_receipt_missing"
    );
    assert_eq!(tally.counted(), 0);
    assert!(super::tests::application_table_snapshot(&db) == before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn multi_terminal_original_live_paid_root_cannot_be_closed_by_consumer() {
    let (f, seen) = multi_exit_original_paid_fixture("multi-terminal-unclosed");
    let original = OriginalAgentTerminalIdentity::capture(&f.context);
    let db = db::open(&f.context.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let mut tally = AgentPipelineTally::default();
    assert!(record_agent_target_outcome_original(
        &f.context.db_path,
        &f.actor.scan_id,
        &f.context.route,
        &original,
        AgentTargetOutcome::incomplete("unclosed original producer"),
        &mut tally
    )
    .is_err());
    assert_eq!(tally.counted(), 0);
    assert!(super::tests::application_table_snapshot(&db) == before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn multi_terminal_original_changed_callback_cannot_reinterpret_paid_exit() {
    let (f, seen, original, _) =
        multi_terminal_original_fixture("multi-terminal-callback-conflict");
    let db = db::open(&f.context.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let mut tally = AgentPipelineTally::default();
    for altered in [
        AgentTargetOutcome::incomplete("different callback"),
        AgentTargetOutcome::failed("original paid terminal reason"),
        AgentTargetOutcome::Cancelled,
    ] {
        assert_eq!(
            record_agent_target_outcome_original(
                &f.context.db_path,
                &f.actor.scan_id,
                &f.context.route,
                &original,
                altered,
                &mut tally
            )
            .unwrap_err(),
            "runtime_terminal_original_outcome_conflict"
        );
        assert_eq!(tally.counted(), 0);
        assert!(super::tests::application_table_snapshot(&db) == before);
    }
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn multi_terminal_original_change_after_verified_read_is_refused_in_projection_transaction() {
    let (f, seen, original, outcome) = multi_terminal_original_fixture("multi-terminal-race");
    let path = f.context.db_path.clone();
    let root = f.actor.root_run_id.clone();
    let rows = std::sync::Arc::new(std::sync::Mutex::new(None));
    let saved = rows.clone();
    ORIGINAL_TERMINAL_LEGACY_PAUSE.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move || {
            let db = db::open(&path).unwrap();
            db.execute("UPDATE agent_runs SET terminal_reason='changed between original read and projection' WHERE id=?1", [&root]).unwrap();
            *saved.lock().unwrap() = Some(super::tests::application_table_snapshot(&db));
        }));
    });
    let mut tally = AgentPipelineTally::default();
    assert!(record_agent_target_outcome_original(
        &f.context.db_path,
        &f.actor.scan_id,
        &f.context.route,
        &original,
        outcome,
        &mut tally
    )
    .is_err());
    assert_eq!(tally.counted(), 0);
    let db = db::open(&f.context.db_path).unwrap();
    assert!(
        super::tests::application_table_snapshot(&db) == *rows.lock().unwrap().as_ref().unwrap()
    );
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn multi_terminal_original_single_relabel_cannot_escape_immutable_contract() {
    let (f, seen, original, outcome) = multi_terminal_original_fixture("multi-terminal-policy");
    let db = db::open(&f.context.db_path).unwrap();
    db.execute(
        "UPDATE agent_runs SET orchestration_policy='single' WHERE id=?1",
        [&f.actor.root_run_id],
    )
    .unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let mut tally = AgentPipelineTally::default();
    assert!(record_agent_target_outcome_original(
        &f.context.db_path,
        &f.actor.scan_id,
        &f.context.route,
        &original,
        outcome,
        &mut tally
    )
    .is_err());
    assert_eq!(tally.counted(), 0);
    assert!(super::tests::application_table_snapshot(&db) == before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn multi_terminal_original_unknown_bill_stays_unsettled_and_cannot_become_completion() {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            503,
            "application/json",
            r#"{"error":{"message":"original provider cost unknown"}}"#.into(),
        )
    }));
    let f = root_tick_fixture(
        "multi-terminal-unknown",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let original = OriginalAgentTerminalIdentity::capture(&f.context);
    assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
    let db = db::open(&f.context.db_path).unwrap();
    let outcome = AgentTargetOutcome::incomplete("original bill unresolved");
    finish_coordinator_run(&db, &f.actor, &outcome).unwrap();
    let costs = multi_terminal_nonprojection_rows(&db);
    let mut tally = AgentPipelineTally::default();
    assert!(record_agent_target_outcome_original(
        &f.context.db_path,
        &f.actor.scan_id,
        &f.context.route,
        &original,
        outcome,
        &mut tally
    )
    .unwrap());
    assert_eq!((tally.counted(), tally.partial), (1, 1));
    assert_eq!(multi_terminal_nonprojection_rows(&db), costs);
    assert!(
        crate::agent_runtime::multi_agent::budget::admission::require_settled_for_completion(
            &db,
            &f.actor.root_run_id
        )
        .is_err()
    );
    let before = super::tests::application_table_snapshot(&db);
    let claimed = AgentTargetOutcome::Completed(AgentCompletion::without_ledger(
        "original bill unresolved",
        AGENT_STOP_FINISH,
    ));
    assert_eq!(
        record_agent_target_outcome_original(
            &f.context.db_path,
            &f.actor.scan_id,
            &f.context.route,
            &original,
            claimed,
            &mut tally
        )
        .unwrap_err(),
        "runtime_terminal_original_outcome_conflict"
    );
    assert_eq!((tally.counted(), tally.partial), (1, 1));
    assert!(super::tests::application_table_snapshot(&db) == before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn multi_terminal_original_naturally_expired_c_can_project_only_original_exit() {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            sdk_exact_model_body(&root_tick_valid_text("original before C expiry")),
        )
    }));
    let mut f = root_tick_fixture_protocol_limits_timeout(
        "multi-terminal-expiry",
        &format!("http://127.0.0.1:{port}/v1"),
        true,
        (60000, 20),
        Some(4),
    );
    let original = OriginalAgentTerminalIdentity::capture(&f.context);
    native_coordinator_tick(&f.context, &f.actor).unwrap();
    let db = db::open(&f.context.db_path).unwrap();
    let outcome = AgentTargetOutcome::incomplete("original paid exit before natural C expiry");
    finish_coordinator_run(&db, &f.actor, &outcome).unwrap();
    drop(f.parent.take());
    let deadline = std::time::Instant::now() + Duration::from_secs(6);
    while db.query_row("SELECT lease_expires_at>datetime('now','localtime') FROM agent_coordinator_leases WHERE root_run_id=?1",[&f.actor.root_run_id],|r|r.get::<_,bool>(0)).unwrap() {
        assert!(std::time::Instant::now()<deadline,"original C did not naturally expire within fixture ceiling");
        std::thread::sleep(Duration::from_millis(50));
    }
    let financial = multi_terminal_nonprojection_rows(&db);
    let mut tally = AgentPipelineTally::default();
    assert!(record_agent_target_outcome_original(
        &f.context.db_path,
        &f.actor.scan_id,
        &f.context.route,
        &original,
        outcome,
        &mut tally
    )
    .unwrap());
    assert_eq!((tally.counted(), tally.partial), (1, 1));
    assert_eq!(multi_terminal_nonprojection_rows(&db), financial);
    let before = super::tests::application_table_snapshot(&db);
    assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
    assert!(super::tests::application_table_snapshot(&db) == before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

// Successful publication intentionally changes its target and two derived checkpoints.
// Preserve every other application's original physical row, including all finance/C.
fn multi_terminal_nonprojection_rows(
    db: &rusqlite::Connection,
) -> Vec<(String, Vec<Vec<rusqlite::types::Value>>)> {
    single_finally_physical(db)
        .into_iter()
        .filter(|(name, _)| !matches!(name.as_str(), "sentinel_targets" | "sentinel_checkpoints"))
        .collect()
}
