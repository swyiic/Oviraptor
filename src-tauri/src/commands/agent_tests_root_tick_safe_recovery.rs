// Actual Root SDK, first redaction, immutable paid receipt and same-API replay.
fn root_tick_secret_summary_text() -> String {
    json!({"schemaVersion":1,"observed":[
        "Cookie: sessionid=ROOT_COOKIE_DO_NOT_PERSIST",
        "Authorization: Bearer ROOT_AUTHORIZATION_DO_NOT_PERSIST",
        "password=ROOT_PASSWORD_DO_NOT_PERSIST",
        "https://ROOT_USER_DO_NOT_PERSIST:ROOT_URL_PASSWORD_DO_NOT_PERSIST@example.test/"
    ],"missing":[],"suggestions":[],"costNotes":[],"risks":[]})
    .to_string()
}
fn root_tick_assert_no_raw_semantic_secrets(db: &rusqlite::Connection) {
    let saved = format!("{:?}", web_mode_test_rows(db));
    for raw in [
        "ROOT_COOKIE_DO_NOT_PERSIST",
        "ROOT_AUTHORIZATION_DO_NOT_PERSIST",
        "ROOT_PASSWORD_DO_NOT_PERSIST",
        "ROOT_USER_DO_NOT_PERSIST",
        "ROOT_URL_PASSWORD_DO_NOT_PERSIST",
    ] {
        assert!(
            !saved.contains(raw),
            "raw model semantic credential was persisted: {raw}"
        );
    }
}
#[test]
fn coordinator_tick_safe_secret_semantics_normal_save_and_replay_use_one_actual_sdk() {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            proposal_model_response(&root_tick_secret_summary_text()),
        )
    }));
    let f = root_tick_fixture(
        "tick-safe-secret-normal",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let db = db::open(&f.context.db_path).unwrap();
    let initial = native_coordinator_tick(&f.context, &f.actor)
        .expect("first real SDK semantic save must succeed");
    assert!(!initial.replayed);
    assert_eq!(seen.lock().unwrap().len(), 1);
    root_tick_assert_no_raw_semantic_secrets(&db);
    assert!(initial.summary.as_json()["observed"]
        .to_string()
        .contains("<redacted:auth:"));
    let before = web_mode_test_rows(&db);
    let restored = native_coordinator_tick(&f.context, &f.actor).expect(
        "already-sanitized original semantics must survive recovery without rehash rejection",
    );
    assert!(restored.replayed);
    assert_eq!(restored.summary, initial.summary);
    assert_eq!(restored.event_sequence, initial.event_sequence);
    assert_eq!(seen.lock().unwrap().len(), 1);
    web_mode_assert_rows(&db, &before);
    root_tick_assert_no_raw_semantic_secrets(&db);
    for phase in ["request", "decision", "publication"] {
        assert_eq!(root_tick_count(&db, &f.actor.root_run_id, phase), 1);
    }
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &f.actor.root_run_id,
            None,
            "model_requests"
        )
        .unwrap()
        .consumed,
        1
    );
}
#[test]
fn coordinator_tick_safe_secret_paid_publication_recovery_preserves_original_bill_and_summary() {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            proposal_model_response(&root_tick_secret_summary_text()),
        )
    }));
    let f = root_tick_fixture(
        "tick-safe-secret-paid",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let db = db::open(&f.context.db_path).unwrap();
    db.execute_batch("CREATE TRIGGER safe_secret_publication_fault BEFORE INSERT ON agent_events
        WHEN NEW.event_type='model_round_completed' AND json_extract(NEW.payload_json,'$.rootTickVersion')=1
        BEGIN SELECT RAISE(ABORT,'safe_secret_paid_before_publication'); END;").unwrap();
    assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "decision"), 1);
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 0);
    root_tick_assert_no_raw_semantic_secrets(&db);
    let before = web_mode_test_rows(&db);
    let paid_summary:String=db.query_row("SELECT json_extract(fact_json,'$.summary') FROM agent_root_tick_receipts WHERE phase='decision'",[],|r|r.get(0)).unwrap();
    db.execute_batch("DROP TRIGGER safe_secret_publication_fault;")
        .unwrap();
    let restored=native_coordinator_tick(&f.context,&f.actor)
        .expect("real paid sanitized receipt must publish locally without another SDK or losing its original charge");
    assert!(restored.replayed);
    assert_eq!(restored.summary.as_json().to_string(), paid_summary);
    assert_eq!(seen.lock().unwrap().len(), 1);
    root_tick_assert_no_raw_semantic_secrets(&db);
    let after = web_mode_test_rows(&db);
    for (table, rows) in &before {
        if matches!(
            table.as_str(),
            "agent_root_budget_attempts"
                | "agent_budget_limits"
                | "agent_budget_clock_origins"
                | "agent_budget_entries"
                | "agent_root_model_journal"
        ) {
            assert_eq!(
                after.iter().find(|(name, _)| name == table).map(|(_, r)| r),
                Some(rows),
                "original financial evidence changed: {table}"
            );
        }
    }
    for phase in ["request", "decision", "publication"] {
        assert_eq!(root_tick_count(&db, &f.actor.root_run_id, phase), 1);
    }
    let final_rows = web_mode_test_rows(&db);
    let again = native_coordinator_tick(&f.context, &f.actor).unwrap();
    assert_eq!(again.summary, restored.summary);
    assert_eq!(again.event_sequence, restored.event_sequence);
    assert_eq!(seen.lock().unwrap().len(), 1);
    web_mode_assert_rows(&db, &final_rows);
}
#[test]
fn coordinator_tick_safe_secret_validation_still_rejects_raw_or_marker_adjacent_credentials() {
    use crate::agent_runtime::multi_agent::budget::root::model::tick::decision::DecisionSummary;
    for raw in [
        "Cookie: ROOT_RAW_COOKIE",
        "Authorization: Bearer ROOT_RAW_AUTH",
        "password=ROOT_RAW_PASSWORD",
        "Cookie: <redacted:auth:012345abcdef> ROOT_RAW_TAIL",
        "password=<redacted:auth:012345abcdef>ROOT_RAW_TAIL",
        "Authorization: <redacted:opaque:012345abcdef>",
        "password=<redacted:auth:012345ABCDEf>",
        "https://ROOT_RAW_USER:ROOT_RAW_PASSWORD@example.test/",
    ] {
        let value = json!({"schemaVersion":1,"observed":[raw],"missing":[],"suggestions":[],"costNotes":[],"risks":[]});
        assert!(
            DecisionSummary::from_json(&value).is_err(),
            "unsafe saved semantic value accepted: {raw}"
        );
    }
}
