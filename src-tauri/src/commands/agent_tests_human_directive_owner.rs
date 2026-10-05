// Original creator, paid Root/Mapper/budget event, and one real parent instance.
#[test]
fn human_directive_original_owner_empty_inbox_never_writes_coordinator() {
    let f = web_financial_producer_fixture();
    let db = db::open(&f.context().db_path).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_inbox_coordinator_write BEFORE UPDATE ON agent_coordinator_leases BEGIN SELECT RAISE(ABORT,'inbox_must_not_renew_coordinator'); END;").unwrap();
    let before = (web_mode_test_rows(&db), single_finally_physical(&db));
    let inbox = take_human_directives(f.context()).expect("read inbox does not acquire or renew C");
    assert!(inbox.items.is_empty());
    let original = &f.f.session.as_ref().unwrap().lease;
    let actor = inbox.lease.unwrap();
    assert_eq!(actor.root_run_id, original.root_run_id);
    assert_eq!(actor.lease_epoch, original.lease_epoch);
    assert_eq!(actor.fencing_token, original.fencing_token);
    web_mode_assert_rows(&db, &before.0);
    assert_eq!(single_finally_physical(&db), before.1);
    f.assert_calls(0);
}

#[test]
fn human_directive_original_owner_missing_parent_cannot_accept_confirmed_text() {
    let mut f = web_financial_producer_fixture();
    let db = db::open(&f.context().db_path).unwrap();
    confirm_queue_directive(
        &db,
        &f.f.session.as_ref().unwrap().lease,
        "prioritize authorization",
    );
    f.f.h.context.supervision = None;
    let before = (web_mode_test_rows(&db), single_finally_physical(&db));
    assert!(
        take_human_directives(f.context()).is_err(),
        "live C alone is not the original parent instance"
    );
    web_mode_assert_rows(&db, &before.0);
    assert_eq!(single_finally_physical(&db), before.1);
    f.assert_calls(0);
}

#[test]
fn human_directive_original_owner_dead_parent_cannot_accept_or_reacquire() {
    let mut f = web_financial_producer_fixture();
    let db = db::open(&f.context().db_path).unwrap();
    let actor = f.f.session.as_ref().unwrap().lease.clone();
    confirm_queue_directive(&db, &actor, "prioritize authorization");
    drop(f.f.session.take());
    let before = (web_mode_test_rows(&db), single_finally_physical(&db));
    assert!(
        take_human_directives(f.context()).is_err(),
        "weak ticket must not keep the parent alive"
    );
    web_mode_assert_rows(&db, &before.0);
    assert_eq!(single_finally_physical(&db), before.1);
    assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 4);
    assert!(f.web_seen.lock().unwrap().is_empty());
}

#[test]
fn human_directive_original_owner_expired_or_replaced_coordinator_never_adopts() {
    for damage in [
        "UPDATE agent_coordinator_leases SET lease_expires_at=datetime('now','-1 second','localtime')",
        "UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1,fencing_token='foreign-inbox-owner'",
    ] {
        let f = web_financial_producer_fixture();
        let db = db::open(&f.context().db_path).unwrap();
        confirm_queue_directive(&db, &f.f.session.as_ref().unwrap().lease, "prioritize authorization");
        // Explicit negative fault on the temporary C, never a success producer.
        db.execute_batch(damage).unwrap();
        let before = (web_mode_test_rows(&db), single_finally_physical(&db));
        assert!(take_human_directives(f.context()).is_err(), "{damage}");
        web_mode_assert_rows(&db, &before.0);
    assert_eq!(single_finally_physical(&db), before.1);
        f.assert_calls(0);
    }
}

#[test]
fn human_directive_original_owner_cancelled_or_expired_child_cannot_claim() {
    for damage in [
        "UPDATE agent_runs SET cancel_requested_at=datetime('now','localtime') WHERE role='web_executor'",
        "UPDATE agent_capability_leases SET revoked_at=datetime('now','localtime') WHERE assignment_id IN (SELECT id FROM agent_assignments WHERE role='web_executor')",
        "UPDATE agent_assignment_attempts SET expires_at=datetime('now','-1 second','localtime') WHERE assignment_id IN (SELECT id FROM agent_assignments WHERE role='web_executor')",
    ] {
        let f = web_financial_producer_fixture();
        let db = db::open(&f.context().db_path).unwrap();
        confirm_queue_directive(&db, &f.f.session.as_ref().unwrap().lease, "prioritize authorization");
        // Temporary cancellation/revocation; stop the live parent first only
        // after the entry returns, so a missing-parent shortcut cannot pass.
        db.execute_batch(damage).unwrap();
        if damage.contains("UPDATE agent_assignment_attempts") {
            human_directive_wait_original_worker_expiry(&db,
                f.context().supervision.as_ref().unwrap(),
                &f.context().run.as_ref().unwrap().run_id);
        }
        let before = (web_mode_test_rows(&db), single_finally_physical(&db));
        assert!(take_human_directives(f.context()).is_err(), "{damage}");
        web_mode_assert_rows(&db, &before.0);
    assert_eq!(single_finally_physical(&db), before.1);
        f.assert_calls(0);
    }
}

#[test]
fn human_directive_original_owner_exact_scope_rejects_without_writes() {
    for field in ["scan", "attempt", "target"] {
        let mut f = web_financial_producer_fixture();
        let db = db::open(&f.context().db_path).unwrap();
        confirm_queue_directive(
            &db,
            &f.f.session.as_ref().unwrap().lease,
            "prioritize authorization",
        );
        match field {
            "scan" => f.f.h.context.scan_id.push_str("-foreign"),
            "attempt" => f.f.h.context.attempt_number += 1,
            _ => f.f.h.context.target_url.push_str("/foreign"),
        }
        let before = (web_mode_test_rows(&db), single_finally_physical(&db));
        assert!(take_human_directives(f.context()).is_err(), "{field}");
        web_mode_assert_rows(&db, &before.0);
        assert_eq!(single_finally_physical(&db), before.1);
        f.assert_calls(0);
    }
}

#[test]
fn human_directive_original_owner_valid_confirmation_accepts_and_replays_same_text() {
    let f = web_financial_producer_fixture();
    let db = db::open(&f.context().db_path).unwrap();
    let id = confirm_queue_directive(
        &db,
        &f.f.session.as_ref().unwrap().lease,
        "prioritize authorization",
    );
    let inbox = take_human_directives(f.context()).unwrap();
    assert_eq!(inbox.items.len(), 1);
    assert_eq!(inbox.items[0].id, id);
    assert_eq!(inbox.items[0].status, "accepted");
    let before = (web_mode_test_rows(&db), single_finally_physical(&db));
    let replay = take_human_directives(f.context()).unwrap();
    assert_eq!(replay.items, inbox.items);
    web_mode_assert_rows(&db, &before.0);
    assert_eq!(single_finally_physical(&db), before.1);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_directive_queue_actions",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    f.assert_calls(0); // Inbox acceptance is not an action or a Root Human SDK.
}
