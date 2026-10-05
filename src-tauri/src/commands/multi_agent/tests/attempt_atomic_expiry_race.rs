fn assert_atomic_expiry_writer_busy(db: &rusqlite::Connection) {
    db.busy_timeout(std::time::Duration::ZERO).unwrap();
    let err =
        match rusqlite::Transaction::new_unchecked(db, rusqlite::TransactionBehavior::Immediate) {
            Ok(_) => panic!("expected real SQLite writer contention"),
            Err(err) => err,
        };
    assert_eq!(
        err.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseBusy)
    );
    db.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
}

#[test]
fn assignment_attempt_atomic_expiry_and_production_renewal_obey_writer_order() {
    use crate::agent_runtime::{
        contract::AgentRole,
        multi_agent::{attempts, budget, scheduler},
    };
    for renewal_first in [true, false] {
        let (root, context, _runtime, _request) = http_journal_fixture("http://127.0.0.1:9/", 0);
        let db = db::open(&context.db_path).unwrap();
        let contender = rusqlite::Connection::open(&context.db_path).unwrap();
        contender
            .busy_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let run = context.run.as_ref().unwrap().run_id.clone();
        let tx = db.unchecked_transaction().unwrap();
        let (lease, assignment_id) = budget::target::child_owner(&tx, &run).unwrap().unwrap();
        tx.rollback().unwrap();
        let child = scheduler::ScheduledChild {
            assignment_id,
            run_id: run.clone(),
            role: AgentRole::WebExecutor,
        };
        if renewal_first {
            // Production renewal owns the lock, not a fixture UPDATE or sleep.
            let (locked_tx, locked_rx) = std::sync::mpsc::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel();
            let renewing_run = run.clone();
            let renew = std::thread::spawn(move || {
                let first = std::cell::Cell::new(true);
                scheduler::refresh_running_executor_leases_authorized(
                    &contender,
                    &renewing_run,
                    |_| {
                        if first.replace(false) {
                            locked_tx.send(()).unwrap();
                            release_rx
                                .recv_timeout(std::time::Duration::from_secs(5))
                                .unwrap();
                        }
                        Ok(())
                    },
                )
            });
            locked_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            assert_atomic_expiry_writer_busy(&db);
            release_tx.send(()).unwrap();
            let tx =
                rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
                    .unwrap();
            assert!(!attempts::try_expire_original_in_transaction(&tx, &lease, &child).unwrap());
            tx.commit().unwrap();
            renew.join().unwrap().unwrap();
            attempts::require_live_for_run(&db, &run).unwrap();
        } else {
            expire_worker_deadline(&db, &run);
            let before = super::tests::application_table_snapshot(&db);
            let tx =
                rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
                    .unwrap();
            assert!(attempts::try_expire_original_in_transaction(&tx, &lease, &child).unwrap());
            let (busy_tx, busy_rx) = std::sync::mpsc::channel();
            let renewing_run = run.clone();
            let renew = std::thread::spawn(move || {
                assert_atomic_expiry_writer_busy(&contender);
                busy_tx.send(()).unwrap();
                scheduler::refresh_running_executor_leases(&contender, &renewing_run)
            });
            busy_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            tx.commit().unwrap();
            assert!(
                renew.join().unwrap().is_err(),
                "expiry must never be renewed away"
            );
            assert_expiry_preserved_resources(&db, &before);
            assert_eq!(
                attempts::current(&db, &lease, &child.assignment_id)
                    .unwrap()
                    .state,
                "expired"
            );
            let closed = super::tests::application_table_snapshot(&db);
            assert!(scheduler::refresh_running_executor_leases(&db, &run).is_err());
            assert_eq!(super::tests::application_table_snapshot(&db), closed);
        }
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}
