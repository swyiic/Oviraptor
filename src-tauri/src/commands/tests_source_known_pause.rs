// The actual paid producer exits before the branch Drop finalizes a user pause.
// No failed child, receipt, fee, Root terminal or SDK exit is inserted by this test.
#[test]
fn source_known_pause_actual_exit_closes_paid_root_without_resending_or_refunding() {
    use crate::agent_runtime::multi_agent::budget;
    for (mode, checkpoint, calls) in [
        ("tool_exhausted", "first_tool_received", 5),
        ("analyst_exhausted", "first_tool_finish_received", 7),
    ] {
        let (root, db, actor, result) = source_exhausted_pending_root_using(
            mode,
            checkpoint,
            calls,
            |root, db, record, actor, busy| {
                let path = root.join("oviraptor.sqlite3");
                assert_eq!(request_sentinel_pause(&path, &record.scan_id).unwrap(), 1);
                drop(busy.take());
                let before = source_exit_snapshot(db);
                assert_eq!(
                    finish_sentinel_pause(&path, &record.scan_id, 1).unwrap_err(),
                    "scan_quiescence_worker_active_or_unverifiable"
                );
                assert_eq!(
                    source_exit_snapshot(db),
                    before,
                    "still-owned branch cannot close"
                );
                assert_eq!(
                    db.query_row(
                        "SELECT status FROM agent_runs WHERE id=?1",
                        [&actor.root_run_id],
                        |r| r.get::<_, String>(0)
                    )
                    .unwrap(),
                    "running"
                );
                Ok(json!({"paid":source_exit_paid_rows(db,&actor.root_run_id)}))
            },
        );
        let payload = result.unwrap();
        let path = root.join("oviraptor.sqlite3");
        assert_eq!(
            db.query_row(
                "SELECT status FROM sentinel_scans WHERE id=?1",
                [&actor.scan_id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "paused"
        );
        let state: (String, String) = db
            .query_row(
                "SELECT status,terminal_state FROM agent_runs WHERE id=?1",
                [&actor.root_run_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            state,
            ("terminal".into(), "paused".into()),
            "known paid failure must close only after the actual last owner exits"
        );
        assert_eq!(
            json!(source_exit_paid_rows(&db, &actor.root_run_id)),
            payload["paid"]
        );
        let tx = db.unchecked_transaction().unwrap();
        budget::clock::FinalClock::verify_original_exit(&tx, &actor.root_run_id).unwrap();
        tx.rollback().unwrap();
        for (dimension, amount) in [
            ("model_requests", calls as i64),
            ("model_input_tokens", calls as i64 * 10),
            ("model_output_tokens", calls as i64 * 10),
        ] {
            let cost = budget::balance(&db, &actor.root_run_id, None, dimension).unwrap();
            assert_eq!(
                (cost.consumed, cost.reserved, cost.indeterminate),
                (amount, 0, 0)
            );
        }
        let closed = source_exit_snapshot(&db);
        assert!(!finish_sentinel_pause(&path, &actor.scan_id, 1).unwrap());
        assert!(run_native_source_assessments(
            &path,
            &actor.scan_id,
            1,
            &root.join("attempt-0001")
        )
        .is_err());
        assert_eq!(
            source_exit_snapshot(&db),
            closed,
            "closed pause cannot dispatch or backfill"
        );
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_known_pause_private_writer_rechecks_original_scope_after_preflight() {
    for sql in [
        "UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1",
        "UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00'",
        "UPDATE sentinel_scans SET attempt_count=2",
        "DROP TRIGGER agent_source_round_immutable; UPDATE agent_source_model_rounds SET state='uncertain' WHERE round_number=3",
        "DROP TRIGGER source_runtime_no_update; UPDATE source_runtime_contracts SET contract_json=json_set(contract_json,'$.budget.tokenLimit',1)",
    ] {
        let (root,db,actor,busy)=source_known_pause_pending_fixture();drop(busy);
        let path=root.join("oviraptor.sqlite3");
        let tx=db.unchecked_transaction().unwrap();
        let owners=claim_scan_quiescence_in(&tx,&path,&actor.scan_id).unwrap();
        let original=known_source_pause_root_in(&tx,&actor.scan_id,1).unwrap().unwrap();
        tx.rollback().unwrap();
        // A changed original between transactions must be rechecked by the
        // private writer while the actual aggregate exit owners remain held.
        db.execute_batch(sql).unwrap();assert!(db.changes()>0);
        let damaged=source_exit_snapshot(&db);
        assert!(finish_known_source_pause(&db,&original).is_err(),"{sql}");
        assert_eq!(source_exit_snapshot(&db),damaged,"private writer reused stale preflight: {sql}");
        drop(owners);drop(db);fs::remove_dir_all(root).unwrap();
    }
}
