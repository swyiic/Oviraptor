// Actual original creator/SDK/paid exit, then explicit backed-up schema upgrade.
#[test]
fn scan_deletion_paid_multi_original_run_identity_upgrade_retains_bills_then_deletes() {
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
        db.execute_batch(
            "DROP TABLE agent_root_tick_timeline_receipts;DROP TABLE agent_root_tick_receipts;",
        )
        .unwrap();
        db.execute_batch(
            &include_str!("../agent_runtime/multi_agent/budget/root/model/tick/schema.sql")
                .replace(
                    "REFERENCES agent_root_budget_attempts(root_run_id)",
                    "REFERENCES agent_runs(id)",
                ),
        )
        .unwrap();
    });
    let db = db::open(&h.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let file = fs::read(h.context.target_dir.join("frontend-evidence.json")).unwrap();
    let calls = h.model_seen.lock().unwrap().len();
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../tools/migrate_native_tick_identity.py");
    let inventory = std::process::Command::new("python3")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .arg(&script)
        .arg("--database")
        .arg(&h.db_path)
        .arg("--backup-directory")
        .arg(h.root.join("financial-backups"))
        .output()
        .unwrap();
    assert!(
        inventory.status.success(),
        "{}",
        String::from_utf8_lossy(&inventory.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&inventory.stdout).unwrap();
    assert_eq!(report["state"], "original_run_identity");
    assert!(super::tests::application_table_snapshot(&db) == before);
    let upgraded = std::process::Command::new("python3")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .arg(&script)
        .arg("--apply")
        .arg("--inventory")
        .arg(report["inventory"].as_str().unwrap())
        .arg("--expected-sha256")
        .arg(report["inventory_sha256"].as_str().unwrap())
        .output()
        .unwrap();
    assert!(
        upgraded.status.success(),
        "{}",
        String::from_utf8_lossy(&upgraded.stderr)
    );
    let reply: serde_json::Value = serde_json::from_slice(&upgraded.stdout).unwrap();
    assert_eq!(reply["changed"], true);
    assert!(super::tests::application_table_snapshot(&db) == before);
    multi_exit_receipt_read(&db, &root).unwrap();
    delete_sentinel_scan_inner(&h.db_path, &h.context.scan_id).unwrap();
    crate::agent_runtime::deleted_scan_audit::verify_deleted(&db, &h.context.scan_id).unwrap();
    let deleted = super::tests::application_table_snapshot(&db);
    delete_sentinel_scan_inner(&h.db_path, &h.context.scan_id).unwrap();
    assert!(super::tests::application_table_snapshot(&db) == deleted);
    assert_eq!(
        fs::read(h.context.target_dir.join("frontend-evidence.json")).unwrap(),
        file
    );
    assert_eq!(h.model_seen.lock().unwrap().len(), calls);
    assert_eq!(h.site_seen.lock().unwrap().len(), 2);
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}
