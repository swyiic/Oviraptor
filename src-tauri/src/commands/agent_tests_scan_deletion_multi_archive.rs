// Actual original creator/SDK/owned result/branch, no positive SQL labels.
fn paid_multi_archive_fixture() -> (
    AgentHarness,
    String,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    paid_multi_archive_fixture_with_setup(|_| {})
}
fn paid_multi_archive_fixture_with_setup(
    setup: impl FnOnce(&Path),
) -> (
    AgentHarness,
    String,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    let (h, owned, stop) = multi_terminal_owned_dispatch_with_setup(setup);
    let root = owned
        .original_terminal
        .root_run_id
        .as_ref()
        .unwrap()
        .clone();
    let mut tally = AgentPipelineTally::default();
    assert!(record_owned_agent_target_outcome(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        &owned,
        &mut tally
    ));
    let report = json!({"targets":tally.counted(),"originalRoot":root,"stopCode":owned.outcome.terminal_code(),"detail":owned.outcome.detail()});
    drop(owned);
    assert!(finish_native_branch(
        &h.db_path,
        &h.context.scan_id,
        1,
        "web",
        "partial",
        "original closed Multi target",
        &report
    )
    .unwrap());
    let db = db::open(&h.db_path).unwrap();
    multi_exit_receipt_read(&db, &root).unwrap();
    for d in crate::agent_runtime::multi_agent::budget::DIMENSIONS {
        let b = crate::agent_runtime::multi_agent::budget::balance(&db, &root, None, d).unwrap();
        assert_eq!((b.reserved, b.indeterminate), (0, 0));
    }
    (h, root, stop)
}

#[test]
fn scan_deletion_paid_multi_original_full_sources_can_be_verified_without_live_authority() {
    let (h, root, stop) = paid_multi_archive_fixture();
    let db = db::open(&h.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    let proof = crate::agent_runtime::deleted_scan_audit::prepare(&tx, &h.context.scan_id).expect(
        "closed original Multi needs an independent full-source audit, not a Single-only refusal",
    );
    proof.verify_scope(&tx, &h.context.scan_id).unwrap();
    drop(proof);
    tx.rollback().unwrap();
    assert!(super::tests::application_table_snapshot(&db) == before);
    assert!(
        crate::agent_runtime::multi_agent::budget::root::RootOwner::load_original(&db, &root)
            .unwrap()
            .require_executable(&db)
            .is_err()
    );
    assert_eq!(h.site_seen.lock().unwrap().len(), 2);
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}

#[test]
fn scan_deletion_paid_multi_closed_branch_physically_deletes_task_and_keeps_original_finance() {
    let (h, root, stop) = paid_multi_archive_fixture();
    let db = db::open(&h.db_path).unwrap();
    let calls = h.model_seen.lock().unwrap().len();
    let evidence = fs::read(h.context.target_dir.join("frontend-evidence.json")).unwrap();
    let tables = [
        "agent_budget_entries",
        "agent_root_budget_attempts",
        "agent_root_model_journal",
        "agent_root_tick_receipts",
        "agent_root_tick_timeline_receipts",
        "agent_multi_exit_receipts",
        "agent_web_model_journal",
        "agent_model_cost_facts",
        "agent_assignment_attempts",
        "agent_assignment_replacements",
    ];
    let original = tables.map(|t| {
        crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(
            &db,
            &format!("SELECT rowid,* FROM {t} ORDER BY rowid"),
            [],
        )
        .unwrap()
    });
    delete_sentinel_scan_inner(&h.db_path, &h.context.scan_id)
        .expect("original closed Multi must support task deletion with lossless independent audit");
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM sentinel_scans WHERE id=?1",
            [&h.context.scan_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_runs WHERE scan_id=?1",
            [&h.context.scan_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert!(
        crate::agent_runtime::deleted_scan_audit::verify_deleted(&db, &h.context.scan_id).unwrap()
    );
    for (t, rows) in tables.into_iter().zip(original) {
        assert!(
            crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(
                &db,
                &format!("SELECT rowid,* FROM {t} ORDER BY rowid"),
                []
            )
            .unwrap()
                == rows,
            "{t}"
        );
    }
    let before = super::tests::application_table_snapshot(&db);
    delete_sentinel_scan_inner(&h.db_path, &h.context.scan_id).unwrap();
    assert!(super::tests::application_table_snapshot(&db) == before);
    assert!(
        crate::agent_runtime::multi_agent::budget::root::RootOwner::load_original(&db, &root)
            .is_err()
    );
    assert_eq!(h.model_seen.lock().unwrap().len(), calls);
    assert_eq!(h.site_seen.lock().unwrap().len(), 2);
    assert_eq!(
        fs::read(h.context.target_dir.join("frontend-evidence.json")).unwrap(),
        evidence
    );
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}
