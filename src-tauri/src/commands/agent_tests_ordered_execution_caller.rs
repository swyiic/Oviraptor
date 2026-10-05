// Local public entry must preserve an existing caller's SQLite authorizer.
#[test]
fn ordered_execution_schedule_preserves_caller_hook_and_rejects_caller_transaction() {
    use crate::agent_runtime::multi_agent::directive::ordered_execution as ordered;
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    let (f, id) = ordered_exec_fixture("ordered-caller-boundary", "http://127.0.0.1:9/v1");
    let _inbox = take_human_directives(&f.context).unwrap();
    let connection = db::open(&f.context.db_path).unwrap();
    connection.authorizer(Some(|ctx: AuthContext<'_>| match ctx.action {
        AuthAction::Update { table_name: "projects", .. } => Authorization::Deny,
        _ => Authorization::Allow,
    })).unwrap();
    let job = ordered::prepare_next(&connection, &f.actor, &f.context.evidence, |_| Ok(()))
        .unwrap().unwrap();
    assert_eq!(job.state, "prepared");
    assert!(connection.execute("UPDATE projects SET name='caller_hook_lost'", []).is_err(),
        "ordered scheduling must not remove the caller's original authorizer");
    assert_eq!(ordered_exec_count(&connection, "agent_specialist_calls"), 0);
    assert_eq!(ordered_exec_receipts(&connection, &id).len(), 0);
    connection.authorizer(None::<fn(AuthContext<'_>) -> Authorization>).unwrap();
    let before = receipt_database_snapshot(&connection);
    let tx = rusqlite::Transaction::new_unchecked(&connection, rusqlite::TransactionBehavior::Immediate).unwrap();
    tx.execute("UPDATE projects SET name='uncommitted_caller'", []).unwrap();
    assert!(ordered::prepare_next(&tx, &f.actor, &f.context.evidence, |_| Ok(())).is_err());
    assert!(!connection.is_autocommit());
    tx.rollback().unwrap();
    assert_eq!(receipt_database_snapshot(&connection), before);
}

#[test]
fn ordered_execution_budget_deferral_rejects_ignore_business_and_forged_event_effects() {
    for fault in [
        "CREATE TRIGGER ordered_defer_fault BEFORE UPDATE OF status ON agent_user_directives WHEN NEW.status='deferred' BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER ordered_defer_fault AFTER UPDATE OF status ON agent_user_directives WHEN NEW.status='deferred' BEGIN UPDATE projects SET name='unauthorized-deferral'; END;",
        "DROP TRIGGER agent_collaboration_directive_update; CREATE TRIGGER agent_collaboration_directive_update AFTER UPDATE OF status ON agent_user_directives WHEN NEW.status='deferred' BEGIN INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json) VALUES(NEW.scan_id,NEW.attempt_number,'user_directive',NEW.id,'user_directive','{}'); END;",
    ] {
        let f = root_tick_fixture_protocol_limits("ordered-deferral-fault", "http://127.0.0.1:9/v1", true, (100,20));
        let db = db::open(&f.context.db_path).unwrap();
        let id = confirm_queue_directive(&db, &f.actor, "@investigator 然后 @mapper 评估原证据");
        let mut inbox = take_human_directives(&f.context).unwrap();
        db.execute_batch(fault).unwrap();
        let before = receipt_database_snapshot(&db);
        let error = apply_human_proposal_actions(&f.context, &mut inbox).unwrap_err();
        assert!(error.contains("ordered_defer_not_persisted") || error.contains("not authorized")
            || error.contains("ordered_defer_events_changed"), "{error}");
        assert_eq!(receipt_database_snapshot(&db), before, "{fault}");
        assert_eq!(ordered_exec_count(&db, "agent_specialist_calls"), 0);
        assert_eq!(ordered_exec_count(&db, "agent_directive_ordered_actions"), 0);
        assert!(ordered_exec_receipts(&db, &id).is_empty());
    }
}
