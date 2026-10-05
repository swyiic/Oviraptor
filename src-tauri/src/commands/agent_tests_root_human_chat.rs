// Real creator and paid Human Root chain, not a manufactured UI receipt.
fn human_chat_confirm(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    thread: &str,
) -> String {
    use crate::agent_runtime::multi_agent::directive;
    let d = directive::create_draft_in_thread(
        db,
        &actor.scan_id,
        actor.attempt_number,
        &actor.root_run_id,
        &actor.target_key,
        "coordinator",
        "prioritize authorization",
        thread,
        actor.lease_epoch,
        &actor.fencing_token,
    )
    .unwrap();
    directive::confirm_draft(
        db,
        &actor.scan_id,
        actor.attempt_number,
        &actor.root_run_id,
        &actor.target_key,
        &d.id,
        d.revision,
        &d.draft_hash,
    )
    .unwrap()
    .id
}
fn human_chat_row(status: &JsonValue, event: i64) -> JsonValue {
    status["timeline"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| {
            r["eventType"] == "root_decision" && r["decisionRecord"]["modelEventSequence"] == event
        })
        .unwrap()
        .clone()
}
#[test]
fn root_human_paid_chat_routes_original_team_target_worker_and_root_threads_without_new_cursor() {
    for kind in ["team", "target", "worker", "root"] {
        let f = human_root_producer(0);
        let db = db::open(&f.f.h.db_path).unwrap();
        let actor = &f.f.session.as_ref().unwrap().lease;
        let thread = match kind {
            "team" => "team".into(),
            "target" => actor.target_key.clone(),
            "worker" => f.f.session.as_ref().unwrap().executor.assignment_id.clone(),
            _ => format!("coordinator:{}", actor.root_run_id),
        };
        let id = human_chat_confirm(&db, actor, &thread);
        let _inbox = take_human_directives(&f.f.h.context).unwrap();
        let cursor =
            native_scan_status_for_attempt(&db, &actor.scan_id, None, Some(actor.attempt_number))
                .unwrap()["latestSequence"]
                .as_i64()
                .unwrap();
        let paid = native_coordinator_human_assessments(&f.f.h.context, actor).unwrap();
        assert_eq!(paid.len(), 1);
        let event = paid[0].paid.event_sequence;
        let fact = &paid[0].frame.semantic;
        let before = (web_mode_test_rows(&db), single_finally_physical(&db));
        let delta = native_scan_status_for_attempt(
            &db,
            &actor.scan_id,
            Some(cursor),
            Some(actor.attempt_number),
        )
        .unwrap();
        let row = human_chat_row(&delta, event);
        assert_eq!(
            row["threadKey"], thread,
            "paid original thread missing for {kind}"
        );
        assert_eq!(
            row["decisionRecord"]["humanDirective"],
            json!({"schemaVersion":1,"directiveId":id,"draftId":fact["draftId"],"revision":fact["revision"],"draftHash":fact["draftHash"],"confirmationReceiptId":fact["confirmationReceiptId"],"threadKey":thread,"targetKey":actor.target_key})
        );
        assert_eq!(row["fromRunId"], actor.root_run_id);
        assert_eq!(row["decisionRecord"]["usage"]["modelRequests"], 1);
        assert_eq!(row["decisionRecord"]["advisoryOnly"], true);
        let sequence = row["sequence"].as_i64().unwrap();
        assert!(sequence > cursor);
        let replay = native_scan_status_for_attempt(
            &db,
            &actor.scan_id,
            Some(sequence),
            Some(actor.attempt_number),
        )
        .unwrap();
        assert!(!replay["timeline"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["eventType"] == "root_decision"));
        let history =
            native_scan_timeline_page(&db, &actor.scan_id, actor.attempt_number, sequence + 1)
                .unwrap();
        assert!(history["timeline"].as_array().unwrap().contains(&row));
        web_mode_assert_rows(&db, &before.0);
        assert_eq!(single_finally_physical(&db), before.1);
        assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 5);
        assert!(f.web.lock().unwrap().is_empty());
        assert!(f.f.h.site_seen.lock().unwrap().is_empty());
    }
}
#[test]
fn root_human_two_paid_confirmation_threads_keep_distinct_original_bindings_on_replay() {
    let f = human_root_producer(0);
    let db = db::open(&f.f.h.db_path).unwrap();
    let actor = &f.f.session.as_ref().unwrap().lease;
    let threads = [
        "team".to_owned(),
        f.f.session.as_ref().unwrap().executor.assignment_id.clone(),
    ];
    let ids = threads
        .iter()
        .map(|t| human_chat_confirm(&db, actor, t))
        .collect::<Vec<_>>();
    let _inbox = take_human_directives(&f.f.h.context).unwrap();
    let paid = native_coordinator_human_assessments(&f.f.h.context, actor).unwrap();
    assert_eq!(paid.len(), 2);
    assert_eq!(f.human.lock().unwrap().len(), 2);
    assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 6);
    let before = (web_mode_test_rows(&db), single_finally_physical(&db));
    let status =
        native_scan_status_for_attempt(&db, &actor.scan_id, None, Some(actor.attempt_number))
            .unwrap();
    let mut cursors = std::collections::BTreeSet::new();
    for (i, p) in paid.iter().enumerate() {
        let row = human_chat_row(&status, p.paid.event_sequence);
        assert_eq!(row["threadKey"], threads[i]);
        assert_eq!(
            row["decisionRecord"]["humanDirective"]["directiveId"],
            ids[i]
        );
        assert!(cursors.insert(row["sequence"].as_i64().unwrap()));
    }
    let replay = native_coordinator_human_assessments(&f.f.h.context, actor).unwrap();
    assert!(replay.iter().all(|p| p.paid.replayed));
    web_mode_assert_rows(&db, &before.0);
    assert_eq!(single_finally_physical(&db), before.1);
    assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 6);
    assert!(f.web.lock().unwrap().is_empty());
    assert!(f.f.h.site_seen.lock().unwrap().is_empty());
}
#[test]
fn root_human_chat_uses_paid_thread_after_current_directive_damage_and_rejects_paid_request_tampering(
) {
    let f = human_root_producer(0);
    let db = db::open(&f.f.h.db_path).unwrap();
    let actor = &f.f.session.as_ref().unwrap().lease;
    let id = human_chat_confirm(&db, actor, "team");
    let _inbox = take_human_directives(&f.f.h.context).unwrap();
    let paid = native_coordinator_human_assessments(&f.f.h.context, actor).unwrap();
    let event = paid[0].paid.event_sequence;
    db.execute(
        "UPDATE agent_user_directives SET thread_key='foreign-thread' WHERE id=?1",
        [&id],
    )
    .unwrap();
    let before = (web_mode_test_rows(&db), single_finally_physical(&db));
    let status =
        native_scan_status_for_attempt(&db, &actor.scan_id, None, Some(actor.attempt_number))
            .unwrap();
    let row = human_chat_row(&status, event);
    assert_eq!(row["threadKey"], "team");
    assert_eq!(row["decisionRecord"]["humanDirective"]["threadKey"], "team");
    web_mode_assert_rows(&db, &before.0);
    assert_eq!(single_finally_physical(&db), before.1);
    let call = row["decisionRecord"]["callId"].as_str().unwrap();
    let text: String = db
        .query_row(
            "SELECT fact_json FROM agent_root_tick_receipts WHERE call_id=?1 AND phase='request'",
            [call],
            |r| r.get(0),
        )
        .unwrap();
    let mut fact: JsonValue = serde_json::from_str(&text).unwrap();
    fact["request"]["basis"]["changedFact"]["semantic"]["threadKey"] = "foreign-thread".into();
    // Corruption is confined to this fresh temporary fixture, after a real bill.
    db.execute_batch("DROP TRIGGER root_tick_no_update;")
        .unwrap();
    db.execute(
        "UPDATE agent_root_tick_receipts SET fact_json=?1 WHERE call_id=?2 AND phase='request'",
        params![fact.to_string(), call],
    )
    .unwrap();
    let before = (web_mode_test_rows(&db), single_finally_physical(&db));
    let error =
        native_scan_status_for_attempt(&db, &actor.scan_id, None, Some(actor.attempt_number))
            .unwrap_err();
    assert!(
        error.contains("root_tick_original_input_conflict"),
        "{error}"
    );
    web_mode_assert_rows(&db, &before.0);
    assert_eq!(single_finally_physical(&db), before.1);
    assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 5);
    assert!(f.web.lock().unwrap().is_empty());
}
#[test]
fn root_human_paid_chat_after_original_parent_exit_reads_without_resuming_or_new_cost() {
    let mut f = human_root_producer(0);
    let db = db::open(&f.f.h.db_path).unwrap();
    let actor = f.f.session.as_ref().unwrap().lease.clone();
    human_chat_confirm(&db, &actor, "team");
    let _inbox = take_human_directives(&f.f.h.context).unwrap();
    let paid = native_coordinator_human_assessments(&f.f.h.context, &actor).unwrap();
    let event = paid[0].paid.event_sequence;
    drop(f.f.session.take());
    assert!(
        check_native_human_directive_actor_on(&db, &f.f.h.context, &actor).is_err(),
        "actual original parent must have exited"
    );
    let before = (web_mode_test_rows(&db), single_finally_physical(&db));
    let status =
        native_scan_status_for_attempt(&db, &actor.scan_id, None, Some(actor.attempt_number))
            .unwrap();
    let row = human_chat_row(&status, event);
    assert_eq!(row["threadKey"], "team");
    assert_eq!(row["decisionRecord"]["humanDirective"]["threadKey"], "team");
    assert_eq!(row["decisionRecord"]["advisoryOnly"], true);
    web_mode_assert_rows(&db, &before.0);
    assert_eq!(single_finally_physical(&db), before.1);
    assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 5);
    assert!(f.web.lock().unwrap().is_empty());
    assert!(f.f.h.site_seen.lock().unwrap().is_empty());
}
