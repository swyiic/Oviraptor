#[test]
fn assignment_attempt_reassignment_issues_new_identity_without_resurrecting_original() {
    use crate::agent_runtime::multi_agent::{attempts, budget, scheduler};
    let (root, context, lease, original) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    expire_worker_deadline(&db, &original.run_id);
    stop_failed_child_preserving_usage(&db, &lease, &original, "expired").unwrap();
    let old = expired_saved_worker_row(&db, &original.run_id);
    let origin: String = db
        .query_row(
            "SELECT started_at FROM agent_budget_clock_origins WHERE root_run_id=?1",
            [&lease.root_run_id],
            |r| r.get(0),
        )
        .unwrap();
    let balance: (i64,i64)=db.query_row("SELECT reserved_tokens,reserved_requests FROM agent_budget_ledger WHERE root_run_id=?1",[&lease.root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let child = scheduler::reassign_undispatched_expired(&db, &lease, &original)
        .expect("an audited expired worker with no dispatch may be explicitly replaced");
    assert_eq!(child.assignment_id, original.assignment_id);
    assert_ne!(child.run_id, original.run_id);
    let replacement = attempts::current(&db, &lease, &child.assignment_id).unwrap();
    assert_eq!(replacement.lease_epoch, 2);
    assert_eq!(replacement.state, "leased");
    assert_eq!(expired_saved_worker_row(&db, &original.run_id), old);
    assert!(attempts::require_live_for_run(&db, &original.run_id).is_err());
    assert_eq!(
        attempts::require_live_for_run(&db, &child.run_id)
            .unwrap()
            .id,
        replacement.id
    );
    assert_eq!(
        budget::balance(&db, &lease.root_run_id, None, "concurrency_batches")
            .unwrap()
            .reserved,
        1
    );
    assert_eq!(db.query_row("SELECT reserved_tokens,reserved_requests FROM agent_budget_ledger WHERE root_run_id=?1",[&lease.root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap(),balance);
    assert_eq!(
        db.query_row(
            "SELECT started_at FROM agent_budget_clock_origins WHERE root_run_id=?1",
            [&lease.root_run_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        origin
    );
    let snapshot = super::tests::application_table_snapshot(&db);
    let replay = scheduler::schedule_child(
        &db,
        &lease,
        original.role,
        crate::agent_runtime::contract::AgentLane::ReadOnlyAnalysis,
        "journal-test",
        &json!({"fixture":true}),
        1,
        &["evidence.read".into(), "mailbox.write".into()],
        8000,
        1,
    )
    .unwrap();
    assert_eq!(replay, child);
    assert_eq!(super::tests::application_table_snapshot(&db), snapshot);
    assert_eq!(
        scheduler::reassign_undispatched_expired(&db, &lease, &original).unwrap(),
        child
    );
    assert_eq!(super::tests::application_table_snapshot(&db), snapshot);
    scheduler::start_child_or_release(&db, &lease, &child).unwrap();
    assert_eq!(
        attempts::require_live_for_run(&db, &child.run_id)
            .unwrap()
            .state,
        "running"
    );
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_reassignment_rejects_live_or_claimed_original_without_writes() {
    use crate::agent_runtime::multi_agent::{scheduler, specialist};
    for claimed in [false, true] {
        let (root, context, lease, child) = specialist_journal_fixture();
        let db = db::open(&context.db_path).unwrap();
        if claimed {
            assert!(matches!(
                specialist::start(&db, &lease, &child, &json!({"messages":[]})).unwrap(),
                specialist::Start::Dispatch(_)
            ));
            expire_worker_deadline(&db, &child.run_id);
            stop_failed_child_preserving_usage(&db, &lease, &child, "expired").unwrap();
        }
        let snapshot = super::tests::application_table_snapshot(&db);
        assert!(scheduler::reassign_undispatched_expired(&db, &lease, &child).is_err());
        assert_eq!(super::tests::application_table_snapshot(&db), snapshot);
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn assignment_attempt_reassignment_final_write_cannot_change_new_usage_or_heartbeat() {
    use crate::agent_runtime::multi_agent::scheduler;
    for change in [
        "UPDATE agent_runs SET used_tokens=1 WHERE id=(SELECT child_run_id FROM agent_assignment_attempts WHERE id=NEW.replacement_attempt_id)",
        "UPDATE agent_assignment_attempts SET heartbeat_at='1999-01-01' WHERE id=NEW.replacement_attempt_id",
    ] {
        let (root,context,lease,child)=specialist_journal_fixture();
        let db=db::open(&context.db_path).unwrap();
        expire_worker_deadline(&db,&child.run_id);
        stop_failed_child_preserving_usage(&db,&lease,&child,"expired").unwrap();
        db.execute_batch(&format!("CREATE TRIGGER replacement_final_fault AFTER INSERT ON agent_assignment_replacements BEGIN {change}; END;")).unwrap();
        let snapshot=super::tests::application_table_snapshot(&db);
        assert!(scheduler::reassign_undispatched_expired(&db,&lease,&child).is_err(),"new worker fields must match the actual fresh grant: {change}");
        assert!(super::tests::application_table_snapshot(&db)==snapshot,"all replacement writes must roll back");
        drop(db);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn assignment_attempt_reassignment_twice_advances_original_audit_chain() {
    use crate::agent_runtime::multi_agent::{attempts, scheduler};
    let (root, context, lease, first) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    expire_worker_deadline(&db, &first.run_id);
    let second = scheduler::reassign_undispatched_expired(&db, &lease, &first).unwrap();
    scheduler::start_child_or_release(&db, &lease, &second).unwrap();
    let first_saved = expired_saved_worker_row(&db, &first.run_id);
    expire_worker_deadline(&db, &second.run_id);
    let third = scheduler::reassign_undispatched_expired(&db, &lease, &second).unwrap();
    assert_eq!(
        attempts::current(&db, &lease, &first.assignment_id)
            .unwrap()
            .lease_epoch,
        3
    );
    assert_eq!(first.assignment_id, third.assignment_id);
    assert_ne!(second.run_id, third.run_id);
    assert_eq!(expired_saved_worker_row(&db, &first.run_id), first_saved);
    let stable = super::tests::application_table_snapshot(&db);
    assert!(scheduler::reassign_undispatched_expired(&db, &lease, &first).is_err());
    assert!(super::tests::application_table_snapshot(&db) == stable);
    assert_eq!(
        scheduler::reassign_undispatched_expired(&db, &lease, &second).unwrap(),
        third
    );
    assert!(super::tests::application_table_snapshot(&db) == stable);
    scheduler::start_child_or_release(&db, &lease, &third).unwrap();
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_reassignment_real_new_transport_and_business_settlement() {
    use crate::agent_runtime::multi_agent::{attempts, budget, scheduler};
    let (root, mut context, lease, original) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    expire_worker_deadline(&db, &original.run_id);
    let child = scheduler::prepare_supervised_readonly_child(
        &db,
        &lease,
        original.role,
        "journal-test",
        &json!({"fixture":true}),
        8000,
    )
    .expect("the real Web bootstrap recovery path must issue and start the replacement");
    let old = expired_saved_worker_row(&db, &original.run_id);
    assert_eq!(
        attempts::current(&db, &lease, &child.assignment_id)
            .unwrap()
            .state,
        "running"
    );
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            proposal_model_response("replacement summary"),
        )
    }));
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let (response, usage) = multi_agent_child_round_transport(
        &context,
        &lease,
        &child,
        "readonly",
        json!({"evidence":"fixed"}),
    )
    .unwrap();
    complete_readonly_assessment(&db, &lease, &child, &usage, &json!({"summary":response}))
        .unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(
        attempts::current(&db, &lease, &child.assignment_id)
            .unwrap()
            .state,
        "completed"
    );
    assert_eq!(expired_saved_worker_row(&db, &original.run_id), old);
    assert_eq!(
        budget::balance(&db, &lease.root_run_id, None, "model_requests")
            .unwrap()
            .consumed,
        1
    );
    assert_eq!(
        budget::balance(&db, &lease.root_run_id, None, "model_requests")
            .unwrap()
            .reserved,
        0
    );
    assert_eq!(
        budget::balance(&db, &lease.root_run_id, None, "concurrency_batches")
            .unwrap()
            .reserved,
        0
    );
    let stable = super::tests::application_table_snapshot(&db);
    complete_readonly_assessment(&db, &lease, &child, &usage, &json!({"summary":response}))
        .unwrap();
    assert!(super::tests::application_table_snapshot(&db) == stable);
    assert!(multi_agent_child_round_transport(
        &context,
        &lease,
        &original,
        "readonly",
        json!({"evidence":"fixed"})
    )
    .is_err());
    assert_eq!(seen.lock().unwrap().len(), 1);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
