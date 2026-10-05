#[test]
fn source_round_unsent_restored_mismatched_fact_does_not_clear_stopped_dispatch() {
    use crate::agent_runtime::multi_agent::{attempts, budget, source_rounds as rounds};
    for mismatch in ["assignment", "code"] {
        let (root, db, context, lease, child, request) = source_round_fixture();
        let check = |db: &rusqlite::Connection| {
            agent_native_source_tool_authority(db, &context, "assignment.finish")
                .map(|_| ())
                .map_err(str::to_string)
        };
        rounds::start_authorized(&db, &lease, &child, 1, &request, 8_000, check).unwrap();
        let worker = attempts::current(&db, &lease, &child.assignment_id).unwrap();
        super::agent_tests::expire_worker_deadline(&db, &child.run_id);
        // Restored invalid data is not a typed gateway observation.
        db.execute("INSERT INTO agent_model_cost_facts(family,root_run_id,assignment_id,lease_attempt_id,child_run_id,lease_epoch,fencing_token,round_number,request_hash,phase,fact_json)
            SELECT 'source-round',root_run_id,?2,?3,child_run_id,lease_epoch,fencing_token,round_number,request_hash,'unsent',?4
            FROM agent_source_model_rounds WHERE child_run_id=?1",
            params![child.run_id,if mismatch=="assignment" {"another-assignment"} else {&child.assignment_id},worker.id,
                json!({"code":if mismatch=="code" {"unproven-restored-label"} else {"model_cancelled_before_transport"}}).to_string()]).unwrap();
        let before = application_table_snapshot(&db);
        assert!(
            budget::admission::require_determinate(&db, &lease.root_run_id).is_err(),
            "{mismatch}: an unbound unsent label cannot clear the stopped original call"
        );
        let tx = db.unchecked_transaction().unwrap();
        assert!(budget::model::release_unsent(&tx, &lease, &child.assignment_id).is_err());
        tx.rollback().unwrap();
        assert_eq!(application_table_snapshot(&db), before);
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_round_unsent_is_readonly_replay_blocks_response_and_preserves_original_budget() {
    use crate::agent_runtime::multi_agent::{budget, source_rounds as rounds};
    for expired in [false, true] {
        let (root, db, context, lease, child, request) = source_round_fixture();
        let check = |db: &rusqlite::Connection| {
            agent_native_source_tool_authority(db, &context, "assignment.finish")
                .map(|_| ())
                .map_err(str::to_string)
        };
        let rounds::Start::Dispatch(call) =
            rounds::start_authorized(&db, &lease, &child, 1, &request, 8_000, check).unwrap()
        else {
            panic!()
        };
        if expired {
            super::agent_tests::expire_worker_deadline(&db, &child.run_id);
            let tx = db.unchecked_transaction().unwrap();
            assert!(
                crate::agent_runtime::multi_agent::attempts::try_expire_original_in_transaction(
                    &tx, &lease, &child
                )
                .unwrap()
            );
            tx.commit().unwrap();
        }
        let before = application_table_snapshot(&db);
        rounds::record_not_sent(&db, &call, "user_cancelled").unwrap();
        let saved = application_table_snapshot(&db);
        for (name, rows) in &before {
            if name != "agent_model_cost_facts" {
                assert_eq!(
                    &saved.iter().find(|(n, _)| n == name).unwrap().1,
                    rows,
                    "{name}/{expired}"
                );
            }
        }
        assert!(budget::admission::require_determinate(&db, &lease.root_run_id).is_ok());
        let count = db.total_changes();
        rounds::record_not_sent(&db, &call, "user_cancelled").unwrap();
        assert_eq!(db.total_changes(), count);
        assert!(rounds::record_not_sent(&db, &call, "model_network").is_err());
        assert!(rounds::record_uncertain(&db, &call, "user_cancelled").is_err());
        assert!(rounds::record_received(
            &db,
            &call,
            &source_round_result("impossible-received", "repo.inventory", json!({}))
        )
        .is_err());
        assert!(rounds::start_authorized(&db, &lease, &child, 1, &request, 8_000, check).is_err());
        assert!(db
            .execute(
                "UPDATE agent_source_model_rounds SET state='received' WHERE child_run_id=?1",
                [&child.run_id]
            )
            .unwrap_err()
            .to_string()
            .contains("source_round_unsent_immutable"));
        assert!(db
            .execute(
                "UPDATE agent_model_cost_facts SET phase='received' WHERE child_run_id=?1",
                [&child.run_id]
            )
            .is_err());
        assert_eq!(application_table_snapshot(&db), saved);
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_round_unsent_write_faults_and_collateral_trigger_roll_back_without_refund() {
    use crate::agent_runtime::multi_agent::{budget, source_rounds as rounds};
    for fault in ["ignore", "abort", "collateral"] {
        let (root, db, context, lease, child, request) = source_round_fixture();
        let check = |db: &rusqlite::Connection| {
            agent_native_source_tool_authority(db, &context, "assignment.finish")
                .map(|_| ())
                .map_err(str::to_string)
        };
        let rounds::Start::Dispatch(call) =
            rounds::start_authorized(&db, &lease, &child, 1, &request, 8_000, check).unwrap()
        else {
            panic!()
        };
        let sql=match fault {
            "ignore"=>"CREATE TRIGGER source_no_send_fault BEFORE INSERT ON agent_model_cost_facts BEGIN SELECT RAISE(IGNORE); END",
            "abort"=>"CREATE TRIGGER source_no_send_fault BEFORE INSERT ON agent_model_cost_facts BEGIN SELECT RAISE(ABORT,'fact unavailable'); END",
            _=>"CREATE TRIGGER source_no_send_fault AFTER INSERT ON agent_model_cost_facts BEGIN UPDATE agent_runs SET status='paused' WHERE id=NEW.root_run_id; END",
        };
        db.execute_batch(sql).unwrap();
        let before = application_table_snapshot(&db);
        assert!(
            rounds::record_not_sent(&db, &call, "user_cancelled").is_err(),
            "{fault}"
        );
        assert_eq!(application_table_snapshot(&db), before, "{fault}");
        let tx = db.unchecked_transaction().unwrap();
        assert!(budget::model::release_unsent(&tx, &lease, &child.assignment_id).is_err());
        tx.rollback().unwrap();
        assert_eq!(application_table_snapshot(&db), before);
        db.execute_batch("DROP TRIGGER source_no_send_fault")
            .unwrap();
        rounds::record_not_sent(&db, &call, "user_cancelled").unwrap();
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}
