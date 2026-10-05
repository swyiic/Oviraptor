// Actual creator/SDK chain on an original pre-migration receipt schema.
#[test]
fn scan_deletion_paid_multi_existing_run_foreign_keys_require_audited_migration_without_writes() {
    let (h, root, stop) = paid_multi_archive_fixture_with_setup(|path| {
        let db = db::open(path).unwrap();
        for table in [
            "agent_root_tick_receipts",
            "agent_root_tick_timeline_receipts",
        ] {
            assert_eq!(
                db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                0
            );
        }
        // Disposable empty schema fixture only, before any original dispatch.
        db.execute_batch(
            "DROP TABLE agent_root_tick_timeline_receipts;DROP TABLE agent_root_tick_receipts;",
        )
        .unwrap();
        let original =
            include_str!("../agent_runtime/multi_agent/budget/root/model/tick/schema.sql").replace(
                "REFERENCES agent_root_budget_attempts(root_run_id)",
                "REFERENCES agent_runs(id)",
            );
        db.execute_batch(&original).unwrap();
    });
    let db = db::open(&h.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    let error = crate::agent_runtime::deleted_scan_audit::prepare(&tx, &h.context.scan_id)
        .err()
        .expect("existing financial schema must not silently migrate");
    assert_eq!(error, "deleted_audit_financial_schema_migration_required");
    tx.rollback().unwrap();
    let error = delete_sentinel_scan_inner(&h.db_path, &h.context.scan_id).unwrap_err();
    assert!(
        error.contains("deleted_audit_financial_schema_migration_required"),
        "{error}"
    );
    assert!(super::tests::application_table_snapshot(&db) == before);
    multi_exit_receipt_read(&db, &root).unwrap();
    assert_eq!(h.site_seen.lock().unwrap().len(), 2);
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}
