#[test]
fn root_human_actual_executor_entry_pays_original_assessment_before_queue_and_web() {
    let f = human_root_producer(0);
    let db = db::open(&f.f.h.db_path).unwrap();
    let actor = &f.f.session.as_ref().unwrap().lease;
    let id = confirm_queue_directive(&db, actor, "prioritize authorization");
    let outcome = NativeAgentBackend.execute(&f.f.h.context);
    assert_eq!(
        f.human.lock().unwrap().len(),
        1,
        "actual Root HumanDirective SDK missing: {outcome:?}"
    );
    let input = human_root_wire_input(&f.human.lock().unwrap()[0]).unwrap();
    assert_eq!(input["basis"]["changedFact"]["semantic"]["directiveId"], id);
    assert_eq!(
        input["basis"]["changedFact"]["semantic"]["confirmedText"],
        "prioritize authorization"
    );
    assert_eq!(input["permittedSuggestion"], "assess:human_directive");
    assert_eq!(
        f.web.lock().unwrap().len(),
        1,
        "original one-request Web grant retained"
    );
    assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 6);
    assert_eq!(root_tick_count(&db, &actor.root_run_id, "publication"), 4);
    let (status,actions):(String,i64)=db.query_row("SELECT status,(SELECT COUNT(*) FROM agent_directive_queue_actions) FROM agent_user_directives WHERE id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!((status, actions), ("completed".into(), 1));
    assert!(f.f.h.site_seen.lock().unwrap().is_empty());
    let wire = f.f.h.model_seen.lock().unwrap();
    let human = wire
        .iter()
        .position(|w| human_root_wire_input(w).is_some())
        .unwrap();
    let web = wire
        .iter()
        .position(|w| !w.contains("You are the Root Coordinator") && !w.contains("SPA/API Mapper"))
        .unwrap();
    assert!(human < web);
}

#[test]
fn root_human_actual_unknown_response_never_applies_or_starts_web() {
    let f = human_root_producer(3);
    let db = db::open(&f.f.h.db_path).unwrap();
    let actor = &f.f.session.as_ref().unwrap().lease;
    let id = confirm_queue_directive(&db, actor, "prioritize authorization");
    let outcome = NativeAgentBackend.execute(&f.f.h.context);
    assert_eq!(
        f.human.lock().unwrap().len(),
        1,
        "actual unknown Root Human SDK: {outcome:?}"
    );
    assert!(f.web.lock().unwrap().is_empty());
    assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 5);
    let (status,actions):(String,i64)=db.query_row("SELECT status,(SELECT COUNT(*) FROM agent_directive_queue_actions) FROM agent_user_directives WHERE id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!((status, actions), ("accepted".into(), 0));
    let root = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &actor.root_run_id,
        Some(""),
        "model_requests",
    )
    .unwrap();
    assert_eq!((root.consumed, root.indeterminate), (3, 1));
    assert_eq!(root_tick_count(&db, &actor.root_run_id, "publication"), 3);
    assert!(f.f.h.site_seen.lock().unwrap().is_empty());
}
