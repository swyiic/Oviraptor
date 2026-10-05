#[test]
fn root_human_original_six_request_floor_refuses_without_any_sdk_or_financial_write() {
    let f = human_root_producer_with_limits(0, 6, 8);
    let db = db::open(&f.f.h.db_path).unwrap();
    let actor = &f.f.session.as_ref().unwrap().lease;
    confirm_queue_directive(&db, actor, "prioritize authorization");
    let mut inbox = take_human_directives(&f.f.h.context).unwrap();
    let mut queue = priority_test_queue();
    let items = inbox.items.clone();
    let before = (web_mode_test_rows(&db), single_finally_physical(&db));
    assert_eq!(
        apply_assessed_human_queue_actions(&f.f.h.context, &mut inbox, &mut queue).unwrap_err(),
        "child_budget_reservation_exceeded_or_stale"
    );
    assert_eq!(queue, priority_test_queue());
    assert_eq!(inbox.items, items);
    web_mode_assert_rows(&db, &before.0);
    assert_eq!(single_finally_physical(&db), before.1);
    assert!(f.human.lock().unwrap().is_empty());
    assert!(f.web.lock().unwrap().is_empty());
    assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 4);
}

#[test]
fn root_human_actual_defer_or_unbounded_advice_preserves_bill_without_action_or_retry() {
    for (reply, expected) in [
        (1, "root_human_directive_deferred"),
        (2, "root_human_step_not_bounded"),
    ] {
        let f = human_root_producer(reply);
        let db = db::open(&f.f.h.db_path).unwrap();
        let actor = &f.f.session.as_ref().unwrap().lease;
        confirm_queue_directive(&db, actor, "prioritize authorization");
        let mut inbox = take_human_directives(&f.f.h.context).unwrap();
        let mut queue = priority_test_queue();
        let original = inbox.items.clone();
        assert_eq!(
            apply_assessed_human_queue_actions(&f.f.h.context, &mut inbox, &mut queue).unwrap_err(),
            expected
        );
        assert_eq!(queue, priority_test_queue());
        assert_eq!(inbox.items, original);
        assert_eq!(f.human.lock().unwrap().len(), 1);
        assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 5);
        let root = crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &actor.root_run_id,
            Some(""),
            "model_requests",
        )
        .unwrap();
        assert_eq!((root.consumed, root.indeterminate), (4, 0));
        assert_eq!(root_tick_count(&db, &actor.root_run_id, "publication"), 4);
        let before = (web_mode_test_rows(&db), single_finally_physical(&db));
        assert_eq!(
            apply_assessed_human_queue_actions(&f.f.h.context, &mut inbox, &mut queue).unwrap_err(),
            expected
        );
        web_mode_assert_rows(&db, &before.0);
        assert_eq!(single_finally_physical(&db), before.1);
        assert_eq!(f.human.lock().unwrap().len(), 1);
        assert!(f.web.lock().unwrap().is_empty());
    }
}

#[test]
fn root_human_actual_completed_queue_replay_uses_only_same_paid_assessment() {
    let f = human_root_producer(0);
    let db = db::open(&f.f.h.db_path).unwrap();
    let actor = &f.f.session.as_ref().unwrap().lease;
    confirm_queue_directive(&db, actor, "prioritize authorization");
    let mut inbox = take_human_directives(&f.f.h.context).unwrap();
    let mut queue = priority_test_queue();
    apply_assessed_human_queue_actions(&f.f.h.context, &mut inbox, &mut queue).unwrap();
    assert_eq!(queue[0], "family:authorization");
    assert!(inbox.items.is_empty());
    assert_eq!(f.human.lock().unwrap().len(), 1);
    let before = (web_mode_test_rows(&db), single_finally_physical(&db));
    queue = priority_test_queue();
    apply_assessed_human_queue_actions(&f.f.h.context, &mut inbox, &mut queue).unwrap();
    assert_eq!(queue[0], "family:authorization");
    web_mode_assert_rows(&db, &before.0);
    assert_eq!(single_finally_physical(&db), before.1);
    assert_eq!(f.human.lock().unwrap().len(), 1);
    assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 5);
    assert!(f.web.lock().unwrap().is_empty());
}

#[test]
fn root_human_completed_unassessed_history_cannot_mint_paid_authority() {
    let f = human_root_producer(0);
    let db = db::open(&f.f.h.db_path).unwrap();
    let actor = &f.f.session.as_ref().unwrap().lease;
    confirm_queue_directive(&db, actor, "prioritize authorization");
    let mut inbox = take_human_directives(&f.f.h.context).unwrap();
    let mut queue = priority_test_queue();
    // The test-only storage wrapper creates an old local receipt, never a paid Root fact.
    apply_human_queue_actions(&f.f.h.context, &mut inbox, &mut queue).unwrap();
    let before = (web_mode_test_rows(&db), single_finally_physical(&db));
    queue = priority_test_queue();
    assert_eq!(
        apply_assessed_human_queue_actions(&f.f.h.context, &mut inbox, &mut queue).unwrap_err(),
        "root_human_original_assessment_missing"
    );
    assert_eq!(queue, priority_test_queue());
    web_mode_assert_rows(&db, &before.0);
    assert_eq!(single_finally_physical(&db), before.1);
    assert!(f.human.lock().unwrap().is_empty());
    assert!(f.web.lock().unwrap().is_empty());
    assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 4);
}
