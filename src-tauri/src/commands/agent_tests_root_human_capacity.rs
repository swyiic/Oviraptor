#[test]
fn root_human_actual_capacity_defers_request_and_keeps_original_web_progress() {
    let f = human_root_producer_with_limits(0, 6, 8);
    let db = db::open(&f.f.h.db_path).unwrap();
    let actor = &f.f.session.as_ref().unwrap().lease;
    let id = confirm_queue_directive(&db, actor, "prioritize authorization");
    let outcome = NativeAgentBackend.execute(&f.f.h.context);
    let state:(String,String,i64)=db.query_row("SELECT status,rejection_code,(SELECT COUNT(*) FROM agent_directive_queue_actions) FROM agent_user_directives WHERE id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(
        state,
        (
            "deferred".into(),
            "root_human_assessment_budget_unavailable".into(),
            0
        ),
        "{outcome:?}"
    );
    assert!(f.human.lock().unwrap().is_empty());
    assert_eq!(f.web.lock().unwrap().len(), 1);
    assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 5);
    assert_eq!(root_tick_count(&db, &actor.root_run_id, "publication"), 3);
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &actor.root_run_id,
            Some(""),
            "model_requests"
        )
        .unwrap()
        .consumed,
        3
    );
    assert!(f.f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn root_human_capacity_deferral_writer_fault_is_atomic_and_cannot_touch_finance() {
    for body in [
        "SELECT RAISE(IGNORE);",
        "UPDATE agent_budget_ledger SET spent_tokens=spent_tokens+1;",
        "INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json) VALUES(NEW.scan_id,NEW.attempt_number,'user_directive',NEW.id,'user_directive','{}');",
    ] {
        let f = human_root_producer_with_limits(0, 6, 8);
        let db = db::open(&f.f.h.db_path).unwrap();
        let actor = &f.f.session.as_ref().unwrap().lease;
        confirm_queue_directive(&db, actor, "prioritize authorization");
        let mut inbox = take_human_directives(&f.f.h.context).unwrap();
        db.execute_batch(&format!("CREATE TRIGGER refuse_human_deferral BEFORE UPDATE OF status ON agent_user_directives WHEN NEW.status='deferred' BEGIN {body} END;")).unwrap();
        let before = (web_mode_test_rows(&db), single_finally_physical(&db));
        let items = inbox.items.clone();
        assert!(defer_human_directives_for_capacity(&f.f.h.context, &mut inbox).is_err());
        assert_eq!(inbox.items, items);
        web_mode_assert_rows(&db, &before.0);
        assert_eq!(single_finally_physical(&db), before.1);
        assert!(f.human.lock().unwrap().is_empty());
        assert!(f.web.lock().unwrap().is_empty());
        assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 4);
    }
    let f = human_root_producer_with_limits(0, 6, 8);
    let db = db::open(&f.f.h.db_path).unwrap();
    let actor = &f.f.session.as_ref().unwrap().lease;
    confirm_queue_directive(&db, actor, "prioritize authorization");
    let mut inbox = take_human_directives(&f.f.h.context).unwrap();
    db.execute_batch("CREATE TRIGGER ignore_human_deferred_event BEFORE INSERT ON agent_collaboration_events WHEN json_extract(NEW.payload_json,'$.status')='deferred' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    let before = (web_mode_test_rows(&db), single_finally_physical(&db));
    let items = inbox.items.clone();
    let error = defer_human_directives_for_capacity(&f.f.h.context, &mut inbox).unwrap_err();
    assert!(
        error.contains("root_human_deferral_events_changed"),
        "{error}"
    );
    assert_eq!(inbox.items, items);
    web_mode_assert_rows(&db, &before.0);
    assert_eq!(single_finally_physical(&db), before.1);
    assert!(f.human.lock().unwrap().is_empty());
    assert!(f.web.lock().unwrap().is_empty());
    assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 4);
}
#[test]
fn root_human_actual_paid_overrun_never_defers_into_another_web_request() {
    let f = human_root_producer(5);
    let db = db::open(&f.f.h.db_path).unwrap();
    let actor = &f.f.session.as_ref().unwrap().lease;
    let id = confirm_queue_directive(&db, actor, "prioritize authorization");
    let outcome = NativeAgentBackend.execute(&f.f.h.context);
    assert_eq!(f.human.lock().unwrap().len(), 1, "{outcome:?}");
    assert!(f.web.lock().unwrap().is_empty());
    let state:(String,i64)=db.query_row("SELECT status,(SELECT COUNT(*) FROM agent_directive_queue_actions) FROM agent_user_directives WHERE id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(
        state,
        ("accepted".into(), 0),
        "a paid overrun must stop all work: {outcome:?}"
    );
    let requests = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &actor.root_run_id,
        Some(""),
        "model_requests",
    )
    .unwrap();
    assert_eq!(requests.consumed + requests.indeterminate, 4);
    let tokens = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &actor.root_run_id,
        Some(""),
        "model_input_tokens",
    )
    .unwrap();
    assert!(tokens.consumed >= 100_000 || tokens.indeterminate > 0);
    assert_eq!(root_tick_count(&db, &actor.root_run_id, "publication"), 3);
    assert!(f.f.h.site_seen.lock().unwrap().is_empty());
}
