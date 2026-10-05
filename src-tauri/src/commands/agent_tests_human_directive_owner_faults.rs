#[test]
fn human_directive_original_owner_revoked_during_acceptance_rolls_back_whole_inbox() {
    for damage in [
        "UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1,fencing_token='mid-claim-owner';",
        "UPDATE agent_runs SET cancel_requested_at=datetime('now','localtime') WHERE role='web_executor';",
        "UPDATE agent_runs SET cancel_requested_at=datetime('now','localtime') WHERE role='coordinator';",
    ] {
        let f = web_financial_producer_fixture();
        let db = db::open(&f.context().db_path).unwrap();
        confirm_queue_directive(&db, &f.f.session.as_ref().unwrap().lease, "prioritize authorization");
        db.execute_batch(&format!("CREATE TRIGGER withdraw_inbox_owner AFTER UPDATE OF status ON agent_user_directives WHEN NEW.status='accepted' BEGIN {damage} END;")).unwrap();
        let before = (web_mode_test_rows(&db), single_finally_physical(&db));
        assert!(take_human_directives(f.context()).is_err(), "{damage}");
        web_mode_assert_rows(&db, &before.0);
    assert_eq!(single_finally_physical(&db), before.1);
        f.assert_calls(0);
    }
}

#[test]
fn human_directive_original_owner_acceptance_write_fault_keeps_original_pending() {
    for fault in [
        "ABORT,'inbox_acceptance_fault'",
        "FAIL,'inbox_acceptance_fault'",
        "IGNORE",
    ] {
        let f = web_financial_producer_fixture();
        let db = db::open(&f.context().db_path).unwrap();
        confirm_queue_directive(
            &db,
            &f.f.session.as_ref().unwrap().lease,
            "prioritize authorization",
        );
        db.execute_batch(&format!("CREATE TRIGGER refuse_inbox_acceptance BEFORE UPDATE OF status ON agent_user_directives WHEN NEW.status='accepted' BEGIN SELECT RAISE({fault}); END;")).unwrap();
        let before = (web_mode_test_rows(&db), single_finally_physical(&db));
        assert!(take_human_directives(f.context()).is_err(), "{fault}");
        web_mode_assert_rows(&db, &before.0);
        assert_eq!(single_finally_physical(&db), before.1);
        f.assert_calls(0);
    }
}

// Wait for the real original parent to finish its separate expiry transaction.
// The following lookup is then compared for zero writes, without mistaking
// authorized background withdrawal for a write by the caller under test.
fn human_directive_wait_original_worker_expiry(
    db: &rusqlite::Connection,
    parent: &crate::agent_runtime::multi_agent::supervision_ticket::SupervisionTicket,
    child: &str,
) {
    let identity = |db: &rusqlite::Connection| {
        db.query_row(
        "SELECT id,worker_id,fencing_token,lease_epoch,coordinator_epoch,coordinator_fencing_token FROM agent_assignment_attempts WHERE child_run_id=?1", [child],
        |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,i64>(3)?,r.get::<_,i64>(4)?,r.get::<_,String>(5)?))).unwrap()
    };
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
    let original_identity = identity(db);
    let original_finance = finance(db);
    let until = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        parent.check().unwrap();
        let withdrawn: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignment_attempts x JOIN agent_assignments a ON a.id=x.assignment_id JOIN agent_runs r ON r.id=x.child_run_id
            WHERE x.child_run_id=?1 AND x.state='expired' AND x.failure_class='worker_lease_expired'
              AND a.state='paused' AND a.failure_class='worker_lease_expired' AND r.status='paused'
              AND NOT EXISTS(SELECT 1 FROM agent_capability_leases c WHERE c.child_run_id=x.child_run_id AND c.revoked_at=''))",[child],|r|r.get(0)).unwrap();
        if withdrawn {
            assert_eq!(identity(db), original_identity);
            assert_eq!(finance(db), original_finance);
            return;
        }
        assert!(
            std::time::Instant::now() < until,
            "original parent did not withdraw expired worker"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
