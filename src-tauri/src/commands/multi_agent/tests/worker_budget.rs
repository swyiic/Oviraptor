// Restore two distinct financial contexts in a temporary database. Execution
// reassignment and Supervisor require separate production and process tests.
fn worker_budget_restored_second(
    db: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    original: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
) -> (
    crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    String,
    String,
) {
    use crate::agent_runtime::multi_agent::attempts;
    let first = attempts::current(db, lease, &original.assignment_id)
        .unwrap()
        .id;
    expire_worker_deadline(db, &original.run_id);
    let tx =
        rusqlite::Transaction::new_unchecked(db, rusqlite::TransactionBehavior::Immediate).unwrap();
    assert!(attempts::try_expire_original_in_transaction(&tx, lease, original).unwrap());
    tx.commit().unwrap();
    let run = namespace_copy_child(db, &original.run_id);
    let id = uuid::Uuid::new_v4().to_string();
    db.execute("UPDATE agent_runs SET status='running',used_tokens=0,used_cached_tokens=0,used_requests=0,lease_expires_at=?2 WHERE id=?1",params![run,lease.lease_expires_at]).unwrap();
    db.execute("INSERT INTO agent_assignment_attempts(id,root_run_id,assignment_id,child_run_id,coordinator_epoch,coordinator_fencing_token,lease_epoch,fencing_token,worker_id,state,expires_at)
        VALUES(?1,?2,?3,?4,?5,?6,2,?7,?8,'running',?9)",params![id,lease.root_run_id,original.assignment_id,run,lease.lease_epoch,lease.fencing_token,uuid::Uuid::new_v4().to_string(),uuid::Uuid::new_v4().to_string(),lease.lease_expires_at]).unwrap();
    db.execute(
        "UPDATE agent_assignments SET child_run_id=?2,state='running',failure_class='' WHERE id=?1",
        params![original.assignment_id, run],
    )
    .unwrap();
    db.execute("UPDATE agent_budget_ledger SET reserved_tokens=reserved_tokens+8000,reserved_requests=reserved_requests+1 WHERE root_run_id=?1",[&lease.root_run_id]).unwrap();
    let child = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
        assignment_id: original.assignment_id.clone(),
        run_id: run,
        role: original.role,
    };
    assert_eq!(
        attempts::require_live_for_run(db, &child.run_id)
            .unwrap()
            .id,
        id
    );
    (child, first, id)
}

fn worker_budget_entries(db: &rusqlite::Connection, attempt: &str) -> Vec<String> {
    let mut query=db.prepare("SELECT json_array(entry_id,root_run_id,assignment_id,lease_attempt_id,dimension,kind,amount,idempotency_key,source_id,created_at)
        FROM agent_budget_entries WHERE lease_attempt_id=?1 ORDER BY rowid").unwrap();
    let rows = query
        .query_map([attempt], |r| r.get(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    rows
}

fn worker_budget_seed_second(
    db: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    assignment: &str,
) {
    use crate::agent_runtime::multi_agent::budget;
    let tx =
        rusqlite::Transaction::new_unchecked(db, rusqlite::TransactionBehavior::Immediate).unwrap();
    for (dimension, amount) in budget::DIMENSIONS[..4].iter().zip([8000, 8000, 8000, 1]) {
        budget::append(
            &tx,
            lease,
            assignment,
            dimension,
            budget::Kind::Reserve,
            amount,
            &format!("fixture-worker-second:{dimension}"),
            &format!("assignment:{assignment}"),
        )
        .unwrap();
    }
    tx.commit().unwrap();
}

#[test]
fn assignment_attempt_worker_budget_second_reservation_uses_independent_idempotency() {
    use crate::agent_runtime::multi_agent::budget;
    let (root, context, lease, original) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    let (child, first, second) = worker_budget_restored_second(&db, &lease, &original);
    let old = worker_budget_entries(&db, &first);
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    budget::model::reserve(&tx, &lease, &child.assignment_id, 8000, 1).expect(
        "a second worker reservation must not collide with the original worker's immutable keys",
    );
    tx.commit().unwrap();
    assert_eq!(worker_budget_entries(&db, &first), old);
    assert_eq!(worker_budget_entries(&db, &second).len(), 4);
    let stable = application_table_snapshot_for_worker_budget(&db);
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    budget::model::reserve(&tx, &lease, &child.assignment_id, 8000, 1).unwrap();
    tx.commit().unwrap();
    assert_eq!(application_table_snapshot_for_worker_budget(&db), stable);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

fn application_table_snapshot_for_worker_budget(
    db: &rusqlite::Connection,
) -> Vec<(String, Vec<String>)> {
    super::tests::application_table_snapshot(db)
}

#[test]
fn assignment_attempt_worker_budget_unsent_release_keeps_original_reservation() {
    worker_budget_release_or_forfeit(true);
}

#[test]
fn assignment_attempt_worker_budget_unknown_forfeit_keeps_original_reservation() {
    worker_budget_release_or_forfeit(false);
}

fn worker_budget_release_or_forfeit(unsent: bool) {
    use crate::agent_runtime::multi_agent::budget;
    {
        let (root, context, lease, original) = specialist_journal_fixture();
        let db = db::open(&context.db_path).unwrap();
        let (child, first, second) = worker_budget_restored_second(&db, &lease, &original);
        worker_budget_seed_second(&db, &lease, &child.assignment_id);
        let old = worker_budget_entries(&db, &first);
        let tx =
            rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
                .unwrap();
        if unsent {
            budget::model::release_unsent(&tx, &lease, &child.assignment_id)
                .expect("only the new worker's unused reservation may be released");
        } else {
            budget::model::forfeit_call(
                &tx,
                &lease,
                &child.assignment_id,
                "second-worker-unknown",
                None,
            )
            .expect("unknown outcome holds only the new worker's estimate");
        }
        tx.commit().unwrap();
        assert_eq!(worker_budget_entries(&db, &first), old);
        assert_eq!(worker_budget_entries(&db, &second).len(), 8);
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn assignment_attempt_worker_budget_slot_release_ignores_an_original_held_slot() {
    use crate::agent_runtime::multi_agent::budget;
    let (root, context, lease, original) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    let (child, first, second) = worker_budget_restored_second(&db, &lease, &original);
    let old = worker_budget_entries(&db, &first);
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    budget::append(
        &tx,
        &lease,
        &child.assignment_id,
        "concurrency_batches",
        budget::Kind::Reserve,
        1,
        "fixture-second-slot",
        "fixture-second-slot",
    )
    .unwrap();
    tx.commit().unwrap();
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    budget::limits::release_slot(&tx, &lease, &child.assignment_id)
        .expect("current slot release must leave an earlier worker's held slot alone");
    tx.commit().unwrap();
    assert_eq!(worker_budget_entries(&db, &first), old);
    assert_eq!(worker_budget_entries(&db, &second).len(), 2);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_worker_budget_settlement_excludes_original_known_model_usage() {
    use crate::agent_runtime::{
        multi_agent::{budget, specialist},
        store::UsageDelta,
    };
    let (root, context, lease, original) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    let specialist::Start::Dispatch(call) =
        specialist::start(&db, &lease, &original, &json!({"messages":[]})).unwrap()
    else {
        panic!()
    };
    specialist::record_received(
        &db,
        &call,
        "original known bill",
        false,
        &late_web_model_response().usage,
    )
    .unwrap();
    let (child, first, second) = worker_budget_restored_second(&db, &lease, &original);
    worker_budget_seed_second(&db, &lease, &child.assignment_id);
    let old = worker_budget_entries(&db, &first);
    let used = UsageDelta {
        input_tokens: 1,
        output_tokens: 2,
        total_tokens: 3,
        model_requests: 1,
        ..Default::default()
    };
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    budget::model::settle(&tx,&lease,&child.assignment_id,&used).expect("current settlement must not subtract the prior worker's known bill from this worker's usage");
    tx.commit().unwrap();
    assert_eq!(worker_budget_entries(&db, &first), old);
    assert!(worker_budget_entries(&db, &second).len() > 4);
    assert_eq!(
        budget::balance(
            &db,
            &lease.root_run_id,
            Some(&child.assignment_id),
            "model_requests"
        )
        .unwrap()
        .consumed,
        2
    );
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
