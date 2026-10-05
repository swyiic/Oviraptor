// Use the original actual paid producer and actual pause/branch Drop. No
// branch/target terminal, Root result, fee or exit row is inserted by this test.
fn source_pause_branch_original_fixture(
    mode: &'static str,
    checkpoint: &str,
    calls: usize,
) -> (
    PathBuf,
    rusqlite::Connection,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) {
    let (root, db, actor, result) =
        source_exhausted_pending_root_using(mode, checkpoint, calls, |root, _, record, _, busy| {
            request_sentinel_pause(&root.join("oviraptor.sqlite3"), &record.scan_id).unwrap();
            drop(busy.take());
            Ok(json!({}))
        });
    result.unwrap();
    assert_eq!(
        db.query_row(
            "SELECT status FROM sentinel_scans WHERE id=?1",
            [&actor.scan_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "paused"
    );
    assert_eq!(
        db.query_row(
            "SELECT status||':'||terminal_state FROM agent_runs WHERE id=?1",
            [&actor.root_run_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "terminal:paused"
    );
    (root, db, actor)
}

#[test]
fn source_pause_branch_actual_known_failure_consumes_original_partial_and_retains_cold_audit() {
    use crate::agent_runtime::{deleted_scan_audit, multi_agent::budget};
    for (mode, checkpoint, calls) in [
        ("tool_exhausted", "first_tool_received", 5),
        ("analyst_exhausted", "first_tool_finish_received", 7),
    ] {
        let (root, db, actor) = source_pause_branch_original_fixture(mode, checkpoint, calls);
        let path = root.join("oviraptor.sqlite3");
        let (state, checkpoint, raw): (String, String, String) = db
            .query_row(
                "SELECT status,checkpoint,report_json FROM native_scan_branches
            WHERE scan_id=?1 AND attempt_number=1 AND branch='source'",
                [&actor.scan_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        if state == "pending" {
            let before = source_exit_snapshot(&db);
            let tx = db.unchecked_transaction().unwrap();
            let error = deleted_scan_audit::prepare(&tx, &actor.scan_id)
                .err()
                .expect("pending branch must refuse paid deletion");
            eprintln!("Actual paused Source pending branch audit refusal: {error}");
            tx.rollback().unwrap();
            assert_eq!(source_exit_snapshot(&db), before);
            assert!(delete_sentinel_scan_inner(&path, &actor.scan_id).is_err());
            assert_eq!(source_exit_snapshot(&db), before);
        }
        assert_eq!(
            state, "partial",
            "actual exhausted Source cannot leave a pending branch after known local exit"
        );
        let report: JsonValue = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            report,
            json!({"error":"source_tool_phase_round_budget_exhausted_without_finish"})
        );
        let reason: String = db
            .query_row(
                "SELECT terminal_reason FROM agent_runs WHERE id=?1",
                [&actor.root_run_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(checkpoint, reason);
        let target:(String,i64)=db.query_row("SELECT t.status,t.last_attempt_number FROM sentinel_targets t JOIN sentinel_scans s
            ON s.id=t.scan_id WHERE s.id=?1 AND t.url=s.source_path",[&actor.scan_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(target, ("partial".into(), 1));
        let files = source_failed_deletion_cas_files(&root);
        assert!(!files.is_empty());
        let before = source_exit_snapshot(&db);
        let tx = db.unchecked_transaction().unwrap();
        let prepared = deleted_scan_audit::prepare(&tx, &actor.scan_id).unwrap();
        prepared.verify_scope(&tx, &actor.scan_id).unwrap();
        drop(prepared);
        tx.rollback().unwrap();
        assert_eq!(source_exit_snapshot(&db), before);
        delete_sentinel_scan_inner(&path, &actor.scan_id).unwrap();
        assert!(deleted_scan_audit::verify_deleted(&db, &actor.scan_id).unwrap());
        let cold = source_exit_snapshot(&db);
        for name in [
            "agent_budget_entries",
            "agent_root_budget_attempts",
            "agent_budget_limits",
            "agent_budget_clock_origins",
            "agent_multi_exit_receipts",
            "agent_model_cost_facts",
            "agent_assignment_attempts",
            "agent_assignment_replacements",
            "agent_root_elapsed_facts",
            "native_sdk_log_owners",
            "native_sdk_log_rows",
            "native_sdk_log_gaps",
            "source_snapshots",
            "analyzer_runs",
            "analyzer_container_receipts",
            "import_record_revisions",
            "assets",
            "project_assets",
        ] {
            assert_eq!(
                before.iter().find(|(n, _)| n == name),
                cold.iter().find(|(n, _)| n == name),
                "retained original {name}"
            );
        }
        assert_eq!(source_failed_deletion_cas_files(&root), files);
        assert!(budget::root::RootOwner::load_original(&db, &actor.root_run_id).is_err());
        for (dimension, amount) in [
            ("model_requests", calls as i64),
            ("model_input_tokens", calls as i64 * 10),
            ("model_output_tokens", calls as i64 * 10),
        ] {
            let b = budget::balance(&db, &actor.root_run_id, None, dimension).unwrap();
            assert_eq!((b.consumed, b.reserved, b.indeterminate), (amount, 0, 0));
        }
        delete_sentinel_scan_inner(&path, &actor.scan_id).unwrap();
        assert_eq!(source_exit_snapshot(&db), cold);
        drop(db);
        let reopened = db::open(&path).unwrap();
        assert!(deleted_scan_audit::verify_deleted(&reopened, &actor.scan_id).unwrap());
        assert_eq!(source_exit_snapshot(&reopened), cold);
        assert_eq!(source_failed_deletion_cas_files(&root), files);
        drop(reopened);
        fs::remove_dir_all(root).unwrap();
    }
}

// Compare every physical projection cell; only the original Source row and
// finite documented result columns may change. Unowned rows remain exact.
fn source_pause_branch_assert_projection_delta(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    before: &SourceExitSnapshot,
) {
    use rusqlite::types::Value;
    let after = source_exit_snapshot(db);
    let reason: String = db
        .query_row(
            "SELECT terminal_reason FROM agent_runs WHERE id=?1",
            [&actor.root_run_id],
            |r| r.get(0),
        )
        .unwrap();
    for table in ["native_scan_branches", "sentinel_targets"] {
        let owned: i64 = if table == "native_scan_branches" {
            db.query_row("SELECT rowid FROM native_scan_branches WHERE scan_id=?1 AND attempt_number=?2 AND branch='source'",params![actor.scan_id,actor.attempt_number],|r|r.get(0)).unwrap()
        } else {
            db.query_row("SELECT rowid FROM sentinel_targets WHERE scan_id=?1 AND url=(SELECT source_path FROM sentinel_scans WHERE id=?1)",[&actor.scan_id],|r|r.get(0)).unwrap()
        };
        let columns = db
            .prepare(&format!("SELECT rowid,* FROM {table} LIMIT 0"))
            .unwrap()
            .column_names()
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        let mut expected = before.iter().find(|(n, _)| n == table).unwrap().1.clone();
        let actual = &after.iter().find(|(n, _)| n == table).unwrap().1;
        let new = actual
            .iter()
            .find(|row| row[0] == Value::Integer(owned))
            .unwrap();
        let row = expected
            .iter_mut()
            .find(|row| row[0] == Value::Integer(owned))
            .unwrap();
        for (index, column) in columns.iter().enumerate() {
            match column.as_str() {
                "status" => row[index] = Value::Text("partial".into()),
                "last_attempt_number" => row[index] = Value::Integer(actor.attempt_number),
                "checkpoint" => row[index] = Value::Text(reason.clone()),
                "report_json" => {
                    row[index] = Value::Text(
                        json!({"error":"source_tool_phase_round_budget_exhausted_without_finish"})
                            .to_string(),
                    )
                }
                "updated_at" if table == "native_scan_branches" => row[index] = new[index].clone(),
                _ => {}
            }
        }
        assert_eq!(
            *actual, expected,
            "only original Source projection columns may change: {table}"
        );
    }
}
