// Actual born Source + two paid SDK calls; no historical owner/clock backfill.
fn source_heartbeat_root_deadline(db: &rusqlite::Connection, root: &str) -> String {
    db.query_row(
        "SELECT datetime(o.started_at,printf('+%f seconds',l.hard_limit/1000.0))
        FROM agent_budget_clock_origins o JOIN agent_budget_limits l ON l.root_run_id=o.root_run_id
        WHERE o.root_run_id=?1 AND l.dimension='wall_time_ms'",
        [root],
        |r| r.get(0),
    )
    .unwrap()
}
fn source_heartbeat_all_grants_bounded(
    db: &rusqlite::Connection,
    root: &str,
    deadline: &str,
) -> bool {
    db.query_row(
        "SELECT NOT EXISTS(SELECT 1 FROM (
        SELECT lease_expires_at AS expiry FROM agent_coordinator_leases WHERE root_run_id=?1
        UNION ALL SELECT lease_expires_at FROM agent_runs WHERE root_run_id=?1
        UNION ALL SELECT lease_expires_at FROM agent_assignments WHERE coordinator_run_id=?1
        UNION ALL SELECT lease_expires_at FROM agent_capability_leases WHERE root_run_id=?1
        UNION ALL SELECT expires_at FROM agent_assignment_attempts WHERE root_run_id=?1
        ) WHERE expiry>?2)",
        params![root, deadline],
        |r| r.get(0),
    )
    .unwrap()
}
#[test]
fn source_heartbeat_actual_paid_initial_calls_never_extend_born_30_second_grants() {
    let (root, db, _, actor, _, seen, stop) = source_timer_fixture(4);
    let deadline = source_heartbeat_root_deadline(&db, &actor.root_run_id);
    let bounded = source_heartbeat_all_grants_bounded(&db, &actor.root_run_id, &deadline);
    let requests = seen.lock().unwrap().len();
    let fees = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &actor.root_run_id,
        None,
        "model_requests",
    )
    .unwrap()
    .consumed;
    let owner = crate::agent_runtime::multi_agent::budget::root::RootOwner::load_original(
        &db,
        &actor.root_run_id,
    )
    .unwrap();
    owner.require_original_coordinator(&db, &actor).unwrap();
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    drop(db);
    fs::remove_dir_all(root).unwrap();
    assert_eq!(
        (requests, fees),
        (2, 2),
        "physical SDK invoices stay original"
    );
    assert!(
        bounded,
        "actual initial Source cancellation heartbeat extended a born grant beyond {deadline}"
    );
}
#[test]
fn source_heartbeat_short_live_tool_call_keeps_grants_at_deadline_without_redundant_writes() {
    use crate::agent_runtime::multi_agent::{attempts, source, source_rounds};
    for fault in ["live", "expired_worker", "revoked", "after_wrong_c_expiry"] {
        let (root, db, record, actor, child, seen, stop) = source_timer_fixture(4);
        let path = root.join("oviraptor.sqlite3");
        let work = root.join("attempt-0001");
        let (environment, runtime, _) =
            verify_source_runtime_contract(&db, &record.scan_id, 1, &work).unwrap();
        let proxy = source_runtime_proxy(&runtime).unwrap();
        let context = SpecialistTransportContext {
            supervision: None,
            db_path: &path,
            scan_id: &record.scan_id,
            attempt_number: 1,
            target_key: &actor.target_key,
            run_id: &actor.root_run_id,
            environment: &environment,
            proxy,
            usage_dir: &work,
            deadline: None,
        };
        let revision: i64 = db
            .query_row(
                "SELECT evidence_revision FROM agent_assignments WHERE id=?1",
                [&child.assignment_id],
                |r| r.get(0),
            )
            .unwrap();
        let slice = source::tool_task_slice(&db, &actor, child.role, revision).unwrap();
        let caps = source::tool_capabilities(child.role).unwrap();
        let request = json!({"messages":[{"role":"user","content":json!({"sourceTask":slice}).to_string()}],
            "tools":caps.iter().map(|name|json!({"type":"function","function":{"name":name}})).collect::<Vec<_>>()});
        assert!(matches!(
            source_rounds::start_authorized(&db, &actor, &child, 1, &request, 800, |tx| {
                authorize_source_tool_phase(tx, &context, &actor, &child)
            })
            .unwrap(),
            source_rounds::Start::Dispatch(_)
        ));
        match fault {
            "expired_worker" => {
                db.execute("UPDATE agent_assignment_attempts SET expires_at='2000-01-01' WHERE child_run_id=?1",[&child.run_id]).unwrap();
            }
            "revoked" => {
                db.execute("UPDATE agent_capability_leases SET revoked_at='revoked' WHERE child_run_id=?1 AND capability='repo.inventory'",[&child.run_id]).unwrap();
            }
            "after_wrong_c_expiry" => {
                // Force one genuinely earlier grant; a bounded renewal must
                // write it, then reject collateral C extension atomically.
                db.execute("UPDATE agent_assignments SET lease_expires_at=datetime('now','+5 seconds','localtime') WHERE id=?1",[&child.assignment_id]).unwrap();
                db.execute_batch("CREATE TRIGGER heartbeat_bad_c AFTER UPDATE ON agent_assignment_attempts BEGIN
                    UPDATE agent_coordinator_leases SET lease_expires_at=datetime('now','+700 seconds','localtime'); END;").unwrap();
            }
            _ => {}
        }
        let before = application_table_snapshot(&db);
        let original = attempts::current(&db, &actor, &child.assignment_id).unwrap();
        let tx =
            rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
                .unwrap();
        let result = heartbeat_source_tool_phase(&tx, &context, &actor, &child);
        let success = result.is_ok();
        if fault == "live" && success {
            tx.commit().unwrap();
        } else {
            drop(tx);
        }
        let unchanged = application_table_snapshot(&db) == before;
        let bounded = source_heartbeat_all_grants_bounded(
            &db,
            &actor.root_run_id,
            &source_heartbeat_root_deadline(&db, &actor.root_run_id),
        );
        if fault == "live" && success {
            assert_eq!(
                attempts::current(&db, &actor, &child.assignment_id).unwrap(),
                original
            );
        }
        let calls = seen.lock().unwrap().len();
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        drop(db);
        fs::remove_dir_all(root).unwrap();
        assert_eq!(success, fault == "live", "{fault}: {result:?}");
        assert!(unchanged,"{fault}: no-benefit renewal / rejected renewal must leave every application row unchanged");
        assert_eq!(calls, 2, "heartbeat never sends this reserved tool round");
        if fault == "live" {
            assert!(
                bounded,
                "short live work must keep the original absolute deadline"
            );
        }
    }
}
