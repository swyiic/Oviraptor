fn source_failed_deletion_original_fixture(
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
            drop(busy.take());
            let path = root.join("oviraptor.sqlite3");
            let result = run_native_source_assessments(
                &path,
                &record.scan_id,
                1,
                &root.join("attempt-0001"),
            );
            assert_eq!(
                result.as_ref().unwrap_err(),
                "source_tool_phase_round_budget_exhausted_without_finish"
            );
            assert!(finish_native_source_branch(
                &path,
                &record.scan_id,
                1,
                &json!({"error":result.as_ref().unwrap_err()})
            )
            .unwrap());
            result
        });
    assert!(result.is_err());
    assert_eq!(
        db.query_row(
            "SELECT status FROM sentinel_scans WHERE id=?1",
            [&actor.scan_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "partial"
    );
    (root, db, actor)
}
fn source_failed_deletion_cas_files(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut pending = vec![root.join("artifact-import-cas")];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(dir).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                pending.push(p);
            } else {
                out.push((p.clone(), fs::read(p).unwrap()));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}
#[test]
fn source_failed_deletion_original_known_exhaustion_preserves_finance_materials_and_cold_audit() {
    use crate::agent_runtime::{deleted_scan_audit, multi_agent::budget};
    for (mode, checkpoint, calls) in [
        ("tool_exhausted", "first_tool_received", 5),
        ("analyst_exhausted", "first_tool_finish_received", 7),
    ] {
        let (root, db, actor) = source_failed_deletion_original_fixture(mode, checkpoint, calls);
        let path = root.join("oviraptor.sqlite3");
        let before = source_exit_snapshot(&db);
        let files = source_failed_deletion_cas_files(&root);
        assert!(!files.is_empty());
        for name in [
            "source_scope_contracts",
            "source_runtime_contracts",
            "source_analysis_views",
            "source_analysis_results",
            "source_ci_policies",
            "source_snapshots",
            "analyzer_runs",
            "analyzer_container_receipts",
            "import_record_revisions",
            "agent_source_model_rounds",
            "agent_source_tool_receipts",
            "agent_multi_exit_receipts",
        ] {
            eprintln!(
                "Disposable failed Source inventory {name}: {} rows",
                db.query_row(&format!("SELECT count(*) FROM {name}"), [], |r| r
                    .get::<_, i64>(0))
                    .unwrap()
            );
        }
        let tx = db.unchecked_transaction().unwrap();
        budget::clock::FinalClock::verify_original_exit(&tx, &actor.root_run_id).unwrap();
        let prepared = deleted_scan_audit::prepare(&tx, &actor.scan_id).expect(
            "original known failed Source requires a separate audit, not fabricated completion",
        );
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
            "agent_root_model_journal",
            "agent_single_exit_receipts",
            "agent_single_projection_receipts",
            "agent_root_budget_definitions",
            "agent_root_mode_definitions",
            "native_web_mode_receipts",
            "native_web_mode_drafts",
            "native_web_mode_actions",
            "native_sdk_log_owners",
            "native_sdk_log_rows",
            "native_sdk_log_gaps",
            "agent_root_tick_receipts",
            "agent_root_tick_timeline_receipts",
            "agent_web_model_journal",
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
        assert!(
            budget::root::RootOwner::load_original(&db, &actor.root_run_id).is_err(),
            "cold audit cannot restore live execution"
        );
        for (dimension, n) in [
            ("model_requests", calls as i64),
            ("model_input_tokens", calls as i64 * 10),
            ("model_output_tokens", calls as i64 * 10),
        ] {
            let b = budget::balance(&db, &actor.root_run_id, None, dimension).unwrap();
            assert_eq!((b.consumed, b.reserved, b.indeterminate), (n, 0, 0));
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
