#[test]
fn root_human_paid_overrun_reports_original_financial_review_without_persistence_failure() {
    let f = human_root_producer(5);
    let db = db::open(&f.f.h.db_path).unwrap();
    let actor = &f.f.session.as_ref().unwrap().lease;
    confirm_queue_directive(&db, actor, "prioritize authorization");
    let tokens = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &actor.root_run_id,
        Some(""),
        "model_input_tokens",
    )
    .unwrap()
    .consumed;
    let outcome = NativeAgentBackend.execute(&f.f.h.context);
    assert_eq!(
        outcome.terminal_code(),
        terminal_code::REQUEST_RECONCILIATION_REQUIRED,
        "{outcome:?}"
    );
    assert!(!outcome.detail().contains("本地记录失败"));
    assert_eq!(f.human.lock().unwrap().len(), 1);
    assert!(f.web.lock().unwrap().is_empty());
    assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 5);
    let after = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &actor.root_run_id,
        Some(""),
        "model_input_tokens",
    )
    .unwrap();
    assert_eq!(
        after.consumed, tokens,
        "over-reservation usage is retained for reconciliation, not fabricated as settled"
    );
    assert!(after.indeterminate > 0);
    let reported:(i64,bool)=db.query_row("SELECT json_extract(receipt_json,'$.usage.inputTokens'),json_extract(receipt_json,'$.usageReported') FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='received' ORDER BY round DESC LIMIT 1",[&actor.root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(reported, (100_000, true));
    let requests = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &actor.root_run_id,
        Some(""),
        "model_requests",
    )
    .unwrap();
    assert_eq!((requests.consumed, requests.indeterminate), (4, 0));
    assert_eq!(root_tick_count(&db, &actor.root_run_id, "publication"), 3);
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM agent_directive_queue_actions",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert!(f.f.h.site_seen.lock().unwrap().is_empty());
}
#[test]
fn root_human_deferral_writer_cannot_spoof_a_financial_code_and_keeps_original_rows() {
    for text in [
        "budget_hard_limit_exceeded",
        "budget_indeterminate_requires_reconciliation",
    ] {
        let f = human_root_producer_with_limits(0, 6, 8);
        let db = db::open(&f.f.h.db_path).unwrap();
        let actor = &f.f.session.as_ref().unwrap().lease;
        confirm_queue_directive(&db, actor, "prioritize authorization");
        let _inbox = take_human_directives(&f.f.h.context).unwrap();
        db.execute_batch(&format!("CREATE TRIGGER spoof_budget_error BEFORE UPDATE OF status ON agent_user_directives WHEN NEW.status='deferred' BEGIN SELECT RAISE(ABORT,'{text}'); END;")).unwrap();
        let finance = fresh_multi_model_financial_rows(&db);
        let outcome = NativeAgentBackend.execute(&f.f.h.context);
        assert_eq!(
            outcome.terminal_code(),
            AGENT_STOP_PERSISTENCE,
            "{outcome:?}"
        );
        assert_eq!(fresh_multi_model_financial_rows(&db), finance);
        // The actual Native entry legitimately refreshes its original worker.
        // The deferral transaction itself must still be entirely zero-write.
        let mut inbox = take_human_directives(&f.f.h.context).unwrap();
        let before = (web_mode_test_rows(&db), single_finally_physical(&db));
        let items = inbox.items.clone();
        let error = defer_human_directives_for_capacity(&f.f.h.context, &mut inbox).unwrap_err();
        assert!(error.contains(text), "{error}");
        assert_eq!(inbox.items, items);
        web_mode_assert_rows(&db, &before.0);
        assert_eq!(single_finally_physical(&db), before.1);
        assert!(f.human.lock().unwrap().is_empty());
        assert!(f.web.lock().unwrap().is_empty());
        assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 4);
        assert!(f.f.h.site_seen.lock().unwrap().is_empty());
    }
}
