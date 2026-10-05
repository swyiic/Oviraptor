#[test]
fn assignment_attempt_supervisor_parent_stop_cancels_live_worker_without_database_rewrite() {
    use crate::agent_runtime::multi_agent::supervisor::WorkerSupervisor;
    let (root, context, lease, child) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    let guard = WorkerSupervisor::start(&context.db_path, &lease).unwrap();
    let cancel =
        specialist_model_cancel_token(&context.db_path, &lease, &child, Some(guard.ticket()));
    assert!(!cancel.is_cancelled());
    let before = super::tests::application_table_snapshot(&db);
    drop(guard);
    assert!(
        cancel.is_cancelled(),
        "the original parent stopped while persisted leases remain live"
    );
    assert!(super::tests::application_table_snapshot(&db) == before);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_supervisor_old_ticket_never_revives_under_same_parent_lease() {
    use crate::agent_runtime::multi_agent::supervisor::WorkerSupervisor;
    let (root, context, lease, child) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    let first = WorkerSupervisor::start(&context.db_path, &lease).unwrap();
    let old = first.ticket();
    let cancel = specialist_model_cancel_token(&context.db_path, &lease, &child, Some(old.clone()));
    drop(first);
    let before = super::tests::application_table_snapshot(&db);
    let successor = WorkerSupervisor::start(&context.db_path, &lease).unwrap();
    assert!(old.check_actor(&context.db_path, &lease).is_err());
    assert!(cancel.is_cancelled());
    assert!(!specialist_model_cancel_token(
        &context.db_path,
        &lease,
        &child,
        Some(successor.ticket())
    )
    .is_cancelled());
    assert!(super::tests::application_table_snapshot(&db) == before);
    drop(successor);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_supervisor_ticket_rejects_other_database_actor_and_run_scope() {
    use crate::agent_runtime::multi_agent::supervisor::WorkerSupervisor;
    let (root, context, lease, child) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    let guard = WorkerSupervisor::start(&context.db_path, &lease).unwrap();
    let ticket = guard.ticket();
    let other = root.join("other.sqlite3");
    fs::write(&other, []).unwrap();
    assert!(ticket.check_actor(&other, &lease).is_err());
    let before = super::tests::application_table_snapshot(&db);
    for field in ["scan", "attempt", "target", "root", "epoch", "fence"] {
        let mut wrong = lease.clone();
        match field {
            "scan" => wrong.scan_id.push_str("-other"),
            "attempt" => wrong.attempt_number += 1,
            "target" => wrong.target_key.push_str("/other"),
            "root" => wrong.root_run_id = uuid::Uuid::new_v4().to_string(),
            "epoch" => wrong.lease_epoch += 1,
            _ => wrong.fencing_token = uuid::Uuid::new_v4().to_string(),
        }
        assert!(
            ticket.check_actor(&context.db_path, &wrong).is_err(),
            "{field}"
        );
    }
    let mut renewed = lease.clone();
    renewed.lease_expires_at = "renewed deadline does not replace identity".into();
    ticket.check_actor(&context.db_path, &renewed).unwrap();
    ticket
        .check_run(
            &db,
            &context.db_path,
            &lease.scan_id,
            lease.attempt_number,
            &lease.target_key,
            &child.run_id,
        )
        .unwrap();
    assert!(ticket
        .check_run(
            &db,
            &context.db_path,
            &lease.scan_id,
            lease.attempt_number,
            &lease.target_key,
            "foreign-run"
        )
        .is_err());
    assert!(super::tests::application_table_snapshot(&db) == before);
    drop(guard);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_supervisor_async_parent_failure_revokes_ticket_without_rewrite() {
    use crate::agent_runtime::multi_agent::supervisor::WorkerSupervisor;
    let (root, context, lease, _) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    let guard = WorkerSupervisor::start(&context.db_path, &lease).unwrap();
    let ticket = guard.ticket();
    db.execute(
        "UPDATE agent_runs SET cancel_requested_at='operator' WHERE id=?1",
        [&lease.root_run_id],
    )
    .unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while ticket.check().is_ok() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(guard.check().is_err());
    assert!(super::tests::application_table_snapshot(&db) == before);
    drop(guard);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_supervisor_stop_denies_web_tools_but_preserves_original_provider_bill() {
    use crate::agent_runtime::multi_agent::{budget, supervisor::WorkerSupervisor};
    let (root, mut context, _, _) = http_journal_fixture("http://127.0.0.1:9/", 0);
    let db = db::open(&context.db_path).unwrap();
    let run = &context.run.as_ref().unwrap().run_id;
    let (lease, assignment) = {
        let tx = db.unchecked_transaction().unwrap();
        budget::target::child_owner(&tx, run).unwrap().unwrap()
    };
    let guard = WorkerSupervisor::start(&context.db_path, &lease).unwrap();
    context.supervision = Some(guard.ticket());
    agent_authorize_tool_on(&db, &context, "replay_http").unwrap();
    let admission = native_model_budget_admission(&context, 1, "original-supervised-model".into())
        .unwrap()
        .unwrap();
    drop(guard);
    let before = super::tests::application_table_snapshot(&db);
    assert_eq!(
        agent_authorize_tool_on(&db, &context, "replay_http"),
        Err("worker_supervision_denied")
    );
    assert!(native_model_budget_admission(&context, 2, "must-not-send".into()).is_err());
    assert!(super::tests::application_table_snapshot(&db) == before);
    let response = late_web_model_response();
    assert!(
        !native_model_terminal_receipt(&context, &admission, "received", Some(&response), "")
            .unwrap()
    );
    let cost =
        budget::balance(&db, &lease.root_run_id, Some(&assignment), "model_requests").unwrap();
    assert_eq!((cost.consumed, cost.indeterminate), (1, 0));
    let paid = super::tests::application_table_snapshot(&db);
    assert!(
        native_model_terminal_receipt(&context, &admission, "received", Some(&response), "")
            .is_err()
    );
    assert!(super::tests::application_table_snapshot(&db) == paid);
    assert_eq!(
        agent_authorize_tool_on(&db, &context, "replay_http"),
        Err("worker_supervision_denied")
    );
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
