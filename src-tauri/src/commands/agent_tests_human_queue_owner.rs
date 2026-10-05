// Queue effects require the same original parent and worker as inbox collection.
#[test]
fn human_queue_original_owner_parent_loss_after_collection_denies_zero_write() {
    for missing in [false, true] {
        let mut f = web_financial_producer_fixture();
        let db = db::open(&f.context().db_path).unwrap();
        confirm_queue_directive(
            &db,
            &f.f.session.as_ref().unwrap().lease,
            "prioritize authorization",
        );
        let mut inbox = take_human_directives(f.context()).unwrap();
        if missing {
            f.f.h.context.supervision = None;
        } else {
            drop(f.f.session.take());
        }
        let mut queue = priority_test_queue();
        let original = queue.clone();
        let original_items = inbox.items.clone();
        let before = (web_mode_test_rows(&db), single_finally_physical(&db));
        assert!(
            apply_human_queue_actions(f.context(), &mut inbox, &mut queue).is_err(),
            "original parent lost after collection, missing={missing}"
        );
        assert_eq!(queue, original);
        assert_eq!(inbox.items, original_items);
        web_mode_assert_rows(&db, &before.0);
        assert_eq!(single_finally_physical(&db), before.1);
        assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 4);
        assert!(f.web_seen.lock().unwrap().is_empty());
    }
}

#[test]
fn human_queue_original_owner_child_withdrawn_after_collection_denies_zero_write() {
    for damage in [
        "UPDATE agent_runs SET cancel_requested_at=datetime('now','localtime') WHERE role='web_executor'",
        "UPDATE agent_capability_leases SET revoked_at=datetime('now','localtime') WHERE assignment_id IN (SELECT id FROM agent_assignments WHERE role='web_executor')",
        "UPDATE agent_assignment_attempts SET expires_at=datetime('now','-1 second','localtime') WHERE assignment_id IN (SELECT id FROM agent_assignments WHERE role='web_executor')",
    ] {
        let f = web_financial_producer_fixture();
        let db = db::open(&f.context().db_path).unwrap();
        confirm_queue_directive(&db, &f.f.session.as_ref().unwrap().lease, "prioritize authorization");
        let mut inbox = take_human_directives(f.context()).unwrap();
        db.execute_batch(damage).unwrap();
        if damage.contains("UPDATE agent_assignment_attempts") {
            human_directive_wait_original_worker_expiry(&db, f.context().supervision.as_ref().unwrap(), &f.context().run.as_ref().unwrap().run_id);
        }
        let mut queue = priority_test_queue();
        let original_items = inbox.items.clone();
        let before = (web_mode_test_rows(&db), single_finally_physical(&db));
        assert!(apply_human_queue_actions(f.context(), &mut inbox, &mut queue).is_err(), "{damage}");
        assert_eq!(queue, priority_test_queue());
        assert_eq!(inbox.items, original_items);
        web_mode_assert_rows(&db, &before.0);
        assert_eq!(single_finally_physical(&db), before.1);
        f.assert_calls(0);
    }
}

#[test]
fn human_queue_original_owner_borrowed_scope_and_fence_cannot_change() {
    for field in ["scan", "attempt", "target", "root", "epoch", "fence"] {
        let f = web_financial_producer_fixture();
        let db = db::open(&f.context().db_path).unwrap();
        confirm_queue_directive(
            &db,
            &f.f.session.as_ref().unwrap().lease,
            "prioritize authorization",
        );
        let mut inbox = take_human_directives(f.context()).unwrap();
        let borrowed = inbox.lease.as_mut().unwrap();
        match field {
            "scan" => borrowed.scan_id.push_str("-foreign"),
            "attempt" => borrowed.attempt_number += 1,
            "target" => borrowed.target_key.push_str("/foreign"),
            "root" => borrowed.root_run_id.push_str("-foreign"),
            "epoch" => borrowed.lease_epoch += 1,
            _ => borrowed.fencing_token.push_str("-foreign"),
        }
        let mut queue = priority_test_queue();
        let original_items = inbox.items.clone();
        let before = (web_mode_test_rows(&db), single_finally_physical(&db));
        assert!(
            apply_human_queue_actions(f.context(), &mut inbox, &mut queue).is_err(),
            "{field}"
        );
        assert_eq!(queue, priority_test_queue());
        assert_eq!(inbox.items, original_items);
        web_mode_assert_rows(&db, &before.0);
        assert_eq!(single_finally_physical(&db), before.1);
        f.assert_calls(0);
    }
}

#[test]
fn human_queue_original_owner_withdrawn_during_receipt_rolls_back_effects() {
    for damage in [
        "UPDATE agent_runs SET cancel_requested_at=datetime('now','localtime') WHERE role='web_executor';",
        "UPDATE agent_capability_leases SET revoked_at=datetime('now','localtime') WHERE assignment_id IN (SELECT id FROM agent_assignments WHERE role='web_executor');",
        "UPDATE agent_runs SET cancel_requested_at=datetime('now','localtime') WHERE role='coordinator';",
        "UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1,fencing_token='queue-foreign-owner';",
    ] {
        let f = web_financial_producer_fixture();
        let db = db::open(&f.context().db_path).unwrap();
        confirm_queue_directive(&db, &f.f.session.as_ref().unwrap().lease, "prioritize authorization");
        let mut inbox = take_human_directives(f.context()).unwrap();
        db.execute_batch(&format!("CREATE TRIGGER withdraw_queue_owner AFTER INSERT ON agent_directive_queue_actions BEGIN {damage} END;")).unwrap();
        let mut queue = priority_test_queue();
        let original_items = inbox.items.clone();
        let before = (web_mode_test_rows(&db), single_finally_physical(&db));
        assert!(apply_human_queue_actions(f.context(), &mut inbox, &mut queue).is_err(), "{damage}");
        assert_eq!(queue, priority_test_queue());
        assert_eq!(inbox.items, original_items);
        web_mode_assert_rows(&db, &before.0);
        assert_eq!(single_finally_physical(&db), before.1);
        f.assert_calls(0);
    }
}

#[test]
fn human_queue_original_owner_completed_rule_replay_still_requires_parent() {
    let mut f = web_financial_producer_fixture();
    let db = db::open(&f.context().db_path).unwrap();
    confirm_queue_directive(
        &db,
        &f.f.session.as_ref().unwrap().lease,
        "prioritize authorization",
    );
    let mut inbox = take_human_directives(f.context()).unwrap();
    let mut queue = priority_test_queue();
    apply_human_queue_actions(f.context(), &mut inbox, &mut queue).unwrap();
    assert_eq!(queue[0], "family:authorization");
    drop(f.f.session.take());
    let before = (web_mode_test_rows(&db), single_finally_physical(&db));
    let mut next_queue = priority_test_queue();
    assert!(apply_human_queue_actions(f.context(), &mut inbox, &mut next_queue).is_err());
    assert_eq!(next_queue, priority_test_queue());
    web_mode_assert_rows(&db, &before.0);
    assert_eq!(single_finally_physical(&db), before.1);
    assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 4);
    assert!(f.web_seen.lock().unwrap().is_empty());
}

#[test]
fn human_queue_original_owner_writer_fault_does_not_publish_queue_or_receipt() {
    for fault in [
        "ABORT,'queue_write_fault'",
        "FAIL,'queue_write_fault'",
        "IGNORE",
    ] {
        let f = web_financial_producer_fixture();
        let db = db::open(&f.context().db_path).unwrap();
        confirm_queue_directive(
            &db,
            &f.f.session.as_ref().unwrap().lease,
            "prioritize authorization",
        );
        let mut inbox = take_human_directives(f.context()).unwrap();
        db.execute_batch(&format!("CREATE TRIGGER refuse_queue_receipt BEFORE INSERT ON agent_directive_queue_actions BEGIN SELECT RAISE({fault}); END;")).unwrap();
        let mut queue = priority_test_queue();
        let original_items = inbox.items.clone();
        let before = (web_mode_test_rows(&db), single_finally_physical(&db));
        assert!(
            apply_human_queue_actions(f.context(), &mut inbox, &mut queue).is_err(),
            "{fault}"
        );
        assert_eq!(queue, priority_test_queue());
        assert_eq!(inbox.items, original_items);
        web_mode_assert_rows(&db, &before.0);
        assert_eq!(single_finally_physical(&db), before.1);
        f.assert_calls(0);
    }
}

#[test]
fn human_queue_original_owner_valid_rule_replay_preserves_original_finance() {
    let f = web_financial_producer_fixture();
    let db = db::open(&f.context().db_path).unwrap();
    let id = confirm_queue_directive(
        &db,
        &f.f.session.as_ref().unwrap().lease,
        "prioritize authorization",
    );
    let mut inbox = take_human_directives(f.context()).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_queue_c_write BEFORE UPDATE ON agent_coordinator_leases BEGIN SELECT RAISE(ABORT,'queue_must_not_renew_c'); END;").unwrap();
    let finance = |db: &rusqlite::Connection| {
        single_finally_physical(db)
            .into_iter()
            .filter(|(name, _)| {
                name.starts_with("agent_budget_")
                    || name.starts_with("agent_root_budget_")
                    || matches!(
                        name.as_str(),
                        "agent_root_model_journal"
                            | "agent_web_model_journal"
                            | "agent_model_cost_facts"
                            | "agent_root_mode_definitions"
                            | "agent_coordinator_leases"
                    )
            })
            .collect::<Vec<_>>()
    };
    let original_finance = finance(&db);
    let mut queue = priority_test_queue();
    apply_human_queue_actions(f.context(), &mut inbox, &mut queue).unwrap();
    assert!(inbox.items.is_empty());
    assert_eq!(queue[0], "family:authorization");
    assert_eq!(queue.len(), priority_test_queue().len());
    let (state,count): (String,i64) = db.query_row("SELECT status,(SELECT COUNT(*) FROM agent_directive_queue_actions) FROM agent_user_directives WHERE id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!((state, count), ("completed".into(), 1));
    assert_eq!(finance(&db), original_finance);
    let before = (web_mode_test_rows(&db), single_finally_physical(&db));
    let ordered = queue.clone();
    queue = priority_test_queue();
    apply_human_queue_actions(f.context(), &mut inbox, &mut queue).unwrap();
    assert_eq!(queue, ordered);
    web_mode_assert_rows(&db, &before.0);
    assert_eq!(single_finally_physical(&db), before.1);
    f.assert_calls(0); // Only a local queue preference; HumanDirective Root SDK remains pending.
}
