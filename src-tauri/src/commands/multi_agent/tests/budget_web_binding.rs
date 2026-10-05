#[test]
fn budget_web_model_mismatched_terminal_receipt_never_clears_original_obligation() {
    use crate::agent_runtime::multi_agent::budget;
    for phase in ["received", "unsent"] {
        for column in [
            "root_run_id",
            "assignment_id",
            "child_run_id",
            "lease_epoch",
            "fencing_token",
            "round",
            "request_hash",
        ] {
            let (root, context, _, _) = http_journal_fixture("http://127.0.0.1:9/", 0);
            let admission =
                native_model_budget_admission(&context, 1, "original-request-hash".into())
                    .unwrap()
                    .unwrap();
            let db = db::open(&context.db_path).unwrap();
            let replacement = if matches!(column, "lease_epoch" | "round") {
                "999"
            } else {
                "'different-binding'"
            };
            let fields = [
                "root_run_id",
                "assignment_id",
                "child_run_id",
                "lease_epoch",
                "fencing_token",
                "round",
                "request_hash",
            ]
            .map(|name| if name == column { replacement } else { name })
            .join(",");
            // Restore/import corruption cannot turn another worker's receipt
            // into the terminal fact for the original immutable dispatch.
            db.execute(
                &format!(
                    "INSERT INTO agent_web_model_journal
                SELECT call_id,{fields}, ?1, '{{\"usageReported\":true}}',created_at
                FROM agent_web_model_journal WHERE phase='dispatch'",
                ),
                [phase],
            )
            .unwrap();
            let before = receipt_database_snapshot(&db);
            assert!(
                budget::admission::require_determinate(&db, &admission.lease.root_run_id).is_err(),
                "{column}/{phase}"
            );
            let tx = db.unchecked_transaction().unwrap();
            assert!(
                budget::model::settle(
                    &tx,
                    &admission.lease,
                    &admission.assignment,
                    &crate::agent_runtime::store::UsageDelta::default()
                )
                .is_err(),
                "{column}/{phase}"
            );
            assert!(
                budget::model::release_unsent(&tx, &admission.lease, &admission.assignment)
                    .is_err(),
                "{column}/{phase}"
            );
            tx.rollback().unwrap();
            assert_eq!(receipt_database_snapshot(&db), before);
            drop(db);
            fs::remove_dir_all(root).unwrap();
        }
    }
}

#[test]
fn budget_web_model_deletion_guard_requires_terminal_receipt_binding() {
    for phase in ["received", "unsent"] {
        for matched in [true, false] {
            let (root, path, run, lease) = multi_agent_test_root("web-binding-delete", 1000, 10);
            let db = db::open(&path).unwrap();
            db.execute(
                "UPDATE agent_runs SET status='completed' WHERE id=?1",
                [&run],
            )
            .unwrap();
            for row_phase in ["dispatch", phase] {
                db.execute("INSERT INTO agent_web_model_journal
                    (call_id,root_run_id,assignment_id,child_run_id,lease_epoch,fencing_token,round,request_hash,phase,receipt_json)
                    VALUES('original-call',?3,'original-asg','original-child',1,?1,1,'original-hash',?2,'{}')",
                    params![if row_phase=="dispatch" || matched {"original-fence"} else {"foreign-fence"},row_phase,run]).unwrap();
            }
            let before = db.total_changes();
            assert_eq!(
                require_scan_deletion_settled_in(&db, &lease.scan_id).is_ok(),
                matched,
                "{phase}/{matched}"
            );
            assert_eq!(db.total_changes(), before);
            drop(db);
            fs::remove_dir_all(root).unwrap();
        }
    }
}
