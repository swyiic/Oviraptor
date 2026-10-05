#[test]
fn source_branch_hidden_paid_original_cannot_be_consumed_as_unstarted_failure() {
    use crate::agent_runtime::multi_agent::budget;
    for unknown in [false, true] {
        let (root, db, record, report, guard, calls) = source_branch_production_report(unknown);
        let path = root.join("oviraptor.sqlite3");
        let actor: String = db
            .query_row(
                "SELECT id FROM agent_runs WHERE scan_id=?1 AND role='coordinator'",
                [&record.scan_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(calls, if unknown { 1 } else { 7 });
        let original = source_exit_snapshot(&db);
        let original_control: String = db
            .query_row(
                "SELECT contract_json FROM agent_root_budget_attempts WHERE root_run_id=?1",
                [&actor],
                |r| r.get(0),
            )
            .unwrap();
        let frozen: JsonValue = serde_json::from_str(&original_control).unwrap();
        assert_eq!(frozen["root"]["scan"], record.scan_id);
        assert_eq!(frozen["root"]["attempt"], 1);
        assert!(frozen["root"]["target"]
            .as_str()
            .unwrap()
            .starts_with("source:"));
        let (target, plan): (String, String) = db
            .query_row(
                "SELECT target_url,plan_json FROM agent_runs WHERE id=?1",
                [&actor],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        let ctarget: String = db
            .query_row(
                "SELECT target_key FROM agent_coordinator_leases WHERE root_run_id=?1",
                [&actor],
                |r| r.get(0),
            )
            .unwrap();
        let balances = [
            "model_requests",
            "model_input_tokens",
            "model_output_tokens",
        ]
        .map(|dimension| {
            let b = budget::balance(&db, &actor, None, dimension).unwrap();
            (b.consumed, b.indeterminate)
        });
        assert_eq!(
            balances,
            if unknown {
                [(0, 1), (0, 4406), (0, 4406)]
            } else {
                [(7, 0), (70, 0), (70, 0)]
            }
        );
        let tx = db.unchecked_transaction().unwrap();
        budget::clock::FinalClock::verify_original_exit(&tx, &actor).unwrap();
        tx.rollback().unwrap();
        // Fault only in disposable DB: the mutable row no longer looks Source,
        // while the original financial scope and all paid receipts still exist.
        db.execute("UPDATE agent_runs SET target_url='https://different.example.test',plan_json=json_set(plan_json,'$.surface','web') WHERE id=?1",[&actor]).unwrap();
        db.execute("UPDATE agent_coordinator_leases SET target_key='https://different.example.test' WHERE root_run_id=?1",[&actor]).unwrap();
        let damaged = source_exit_snapshot(&db);
        let roots = source_branch_original_roots(&db, &record.scan_id, 1).unwrap();
        eprintln!("Source original discovery after mutable scope loss: unknown={unknown}, SDK={calls}, roots={roots:?}");
        let error = finish_native_source_branch(
            &path,
            &record.scan_id,
            1,
            &json!({"error":"runtime_scope_lost"}),
        )
        .unwrap_err();
        eprintln!("Source hidden original consumer rejection: {error}");
        assert_eq!(roots, vec![actor.clone()]);
        assert_eq!(error, "budget_root_original_owner_conflict");
        assert_eq!(
            source_exit_snapshot(&db),
            damaged,
            "cannot turn original paid work into unstarted failure"
        );
        // Guard drop uses the same verifier; no raw failed fallback or new SDK.
        drop(guard);
        assert_eq!(source_exit_snapshot(&db), damaged);
        db.execute(
            "UPDATE agent_runs SET target_url=?2,plan_json=?3 WHERE id=?1",
            params![actor, target, plan],
        )
        .unwrap();
        db.execute(
            "UPDATE agent_coordinator_leases SET target_key=?2 WHERE root_run_id=?1",
            params![actor, ctarget],
        )
        .unwrap();
        assert_eq!(
            source_exit_snapshot(&db),
            original,
            "restoring the exact original fields must preserve every rowid"
        );
        assert!(finish_native_source_branch(&path, &record.scan_id, 1, &report).unwrap());
        let status: String = db
            .query_row(
                "SELECT status FROM native_scan_branches WHERE scan_id=?1 AND branch='source'",
                [&record.scan_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            status,
            if unknown {
                "partial"
            } else {
                "completed_with_gaps"
            }
        );
        for (dimension, expected) in [
            "model_requests",
            "model_input_tokens",
            "model_output_tokens",
        ]
        .into_iter()
        .zip(balances)
        {
            let b = budget::balance(&db, &actor, None, dimension).unwrap();
            assert_eq!((b.consumed, b.indeterminate), expected);
        }
        let closed = source_exit_snapshot(&db);
        assert!(
            !finish_native_source_branch(&path, &record.scan_id, 1, &json!({"error":"late"}))
                .unwrap()
        );
        assert_eq!(source_exit_snapshot(&db), closed);
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}
