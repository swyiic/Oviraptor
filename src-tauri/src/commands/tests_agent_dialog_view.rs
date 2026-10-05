fn dialog_view_fixture() -> (PathBuf, PathBuf, rusqlite::Connection, i64) {
    let (root, _, path) = source_regression_fixture();
    let connection = db::open(&path).unwrap();
    crate::agent_runtime::multi_agent::directive::create_draft(
        &connection, "source-regression", 1, "", "", "coordinator",
        "重点检查登录模块", 0, "",
    ).unwrap();
    let status = native_scan_status(&connection, "source-regression").unwrap();
    assert!(status["timeline"].as_array().unwrap().iter().any(|item| dialog_timeline_thread(item) == "team"));
    let sequence = status["latestSequence"].as_i64().unwrap();
    assert!(sequence > 0);
    (root, path, connection, sequence)
}

fn dialog_view_input(revision: i64, thread: &str, cursor: Option<i64>) -> AgentDialogViewInput {
    AgentDialogViewInput { scan_id: "source-regression".into(), attempt_number: 1,
        expected_revision: revision, selected_thread: thread.into(), mark_read_through: cursor,
        selected_thread_sequence: None }
}

#[test]
fn dialog_view_accepts_only_a_scoped_historical_thread_witness() {
    let (root, _, connection, _) = dialog_view_fixture();
    connection.execute(
        "INSERT INTO agent_runs(id,scan_id,attempt_number,target_url,backend,role,status,assignment_id) \
         VALUES('historic-thread-run','source-regression',1,'https://authorized.example.test','native','spa_api_mapper','prepared','historic-assignment')",
        [],
    ).unwrap();
    let sequence: i64 = connection.query_row(
        "SELECT MAX(sequence) FROM agent_collaboration_events WHERE entity_id='historic-thread-run'",
        [], |row| row.get(0),
    ).unwrap();
    for index in 0..110 {
        connection.execute(
            "INSERT INTO agent_runs(id,scan_id,attempt_number,target_url,backend,role,status) \
             VALUES(?1,'source-regression',1,'https://authorized.example.test','native','spa_api_mapper','prepared')",
            [format!("newer-run-{index:03}")],
        ).unwrap();
    }
    assert!(!native_scan_status(&connection, "source-regression").unwrap()["timeline"]
        .as_array().unwrap().iter().any(|item| dialog_timeline_thread(item) == "historic-assignment"));
    let mut witness = dialog_view_input(0, "historic-assignment", None);
    witness.selected_thread_sequence = Some(sequence);
    let before = bundle_native_snapshot(&connection);
    for (wrong_sequence, wrong_thread) in [(Some(sequence + 1), "historic-assignment"),
        (Some(sequence), "fabricated"), (None, "historic-assignment")] {
        let mut rejected = dialog_view_input(0, wrong_thread, None);
        rejected.selected_thread_sequence = wrong_sequence;
        assert_eq!(persist_dialog_view(&connection, &rejected).unwrap_err(), "dialog_view_thread_unavailable");
        assert_eq!(bundle_native_snapshot(&connection), before);
    }
    let saved = persist_dialog_view(&connection, &witness).unwrap();
    assert_eq!(saved.selected_thread, "historic-assignment");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn dialog_view_read_is_pure_and_save_only_changes_ui_preferences() {
    let (root, path, connection, sequence) = dialog_view_fixture();
    let before = bundle_native_snapshot(&connection);
    let empty = load_dialog_view(&connection, "source-regression", 1).unwrap();
    assert_eq!(empty.revision, 0);
    assert_eq!(bundle_native_snapshot(&connection), before);
    let saved = persist_dialog_view(&connection, &dialog_view_input(0, "team", Some(sequence))).unwrap();
    assert_eq!(saved.thread_read_sequences["team"], sequence);
    assert_eq!(saved.all_read_sequence, 0);
    let after = bundle_native_snapshot(&connection);
    let without_view = |snapshot: Vec<(String, Vec<String>)>| snapshot.into_iter()
        .filter(|(name, _)| name != "agent_dialog_views").collect::<Vec<_>>();
    assert_eq!(without_view(after), without_view(before));
    drop(connection);
    assert_eq!(load_dialog_view(&db::open(&path).unwrap(), "source-regression", 1).unwrap(), saved);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn dialog_view_conflicts_invalid_threads_and_future_cursors_never_write() {
    let (root, _, connection, sequence) = dialog_view_fixture();
    persist_dialog_view(&connection, &dialog_view_input(0, "team", None)).unwrap();
    for (input, reason) in [
        (dialog_view_input(0, "team", None), "dialog_view_revision_conflict"),
        (dialog_view_input(1, "fabricated", None), "dialog_view_thread_unavailable"),
        (dialog_view_input(1, "", Some(sequence + 1)), "dialog_view_future_cursor"),
        (dialog_view_input(1, "", Some(-1)), "dialog_view_invalid_input"),
        (dialog_view_input(DIALOG_VIEW_MAX_INTEGER, "", None), "dialog_view_invalid_input"),
    ] {
        let before = bundle_native_snapshot(&connection);
        assert_eq!(persist_dialog_view(&connection, &input).unwrap_err(), reason);
        assert_eq!(bundle_native_snapshot(&connection), before);
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn dialog_view_cursors_are_monotonic_and_all_read_compacts_threads() {
    let (root, _, connection, sequence) = dialog_view_fixture();
    persist_dialog_view(&connection, &dialog_view_input(0, "team", Some(sequence))).unwrap();
    let older = persist_dialog_view(&connection, &dialog_view_input(1, "team", Some(0))).unwrap();
    assert_eq!(older.thread_read_sequences["team"], sequence);
    let all = persist_dialog_view(&connection, &dialog_view_input(2, "", Some(sequence))).unwrap();
    assert_eq!(all.all_read_sequence, sequence);
    assert!(all.thread_read_sequences.is_empty());
    let older = persist_dialog_view(&connection, &dialog_view_input(3, "", Some(0))).unwrap();
    assert_eq!(older.all_read_sequence, sequence);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn dialog_view_attempt_and_tombstone_boundaries_are_fail_closed() {
    let (root, _, connection, _) = dialog_view_fixture();
    persist_dialog_view(&connection, &dialog_view_input(0, "team", None)).unwrap();
    connection.execute("UPDATE sentinel_scans SET attempt_count=2 WHERE id='source-regression'", []).unwrap();
    assert_eq!(load_dialog_view(&connection, "source-regression", 1).unwrap_err(), "dialog_view_attempt_changed_or_deleted");
    assert_eq!(persist_dialog_view(&connection, &dialog_view_input(1, "", None)).unwrap_err(), "dialog_view_attempt_changed_or_deleted");
    assert_eq!(load_dialog_view(&connection, "source-regression", 2).unwrap().revision, 0);
    connection.execute("INSERT INTO sentinel_deleted_scans(scan_id) VALUES('source-regression')", []).unwrap();
    let before = bundle_native_snapshot(&connection);
    assert_eq!(load_dialog_view(&connection, "source-regression", 2).unwrap_err(), "dialog_view_attempt_changed_or_deleted");
    let mut input = dialog_view_input(0, "", None); input.attempt_number = 2;
    assert_eq!(persist_dialog_view(&connection, &input).unwrap_err(), "dialog_view_attempt_changed_or_deleted");
    assert_eq!(bundle_native_snapshot(&connection), before);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn dialog_view_postwrite_damage_rolls_back_and_corrupt_reads_do_not_repair() {
    let (root, _, connection, sequence) = dialog_view_fixture();
    connection.execute_batch("CREATE TRIGGER damage_dialog_view AFTER INSERT ON agent_dialog_views BEGIN UPDATE agent_dialog_views SET selected_thread='tampered' WHERE scan_id=NEW.scan_id; END;").unwrap();
    let before = bundle_native_snapshot(&connection);
    assert_eq!(persist_dialog_view(&connection, &dialog_view_input(0, "team", None)).unwrap_err(), "dialog_view_write_not_persisted");
    assert_eq!(bundle_native_snapshot(&connection), before);
    connection.execute_batch("DROP TRIGGER damage_dialog_view;").unwrap();
    persist_dialog_view(&connection, &dialog_view_input(0, "team", None)).unwrap();
    for cursors in [json!({"team":sequence+1}), json!({"team":-1}), json!({"team":"bad"}), json!({"":0})] {
        connection.execute("UPDATE agent_dialog_views SET thread_read_sequences_json=?1", [cursors.to_string()]).unwrap();
        let before = bundle_native_snapshot(&connection);
        assert!(load_dialog_view(&connection, "source-regression", 1).is_err());
        assert_eq!(bundle_native_snapshot(&connection), before);
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn dialog_view_concurrent_windows_have_one_winner_and_never_mutate_execution() {
    let (root, path, connection, sequence) = dialog_view_fixture();
    let before = bundle_native_snapshot(&connection);
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let workers: Vec<_> = (0..2).map(|_| {
        let path = path.clone(); let barrier = barrier.clone();
        std::thread::spawn(move || {
            let db = db::open(&path).unwrap();
            assert_eq!(load_dialog_view(&db, "source-regression", 1).unwrap().revision, 0);
            barrier.wait();
            persist_dialog_view(&db, &dialog_view_input(0, "team", Some(sequence)))
        })
    }).collect();
    let results: Vec<_> = workers.into_iter().map(|worker| worker.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(results.iter().filter_map(|result| result.as_ref().err()).collect::<Vec<_>>(),
        vec![&"dialog_view_revision_conflict".to_string()]);
    assert_eq!(load_dialog_view(&connection, "source-regression", 1).unwrap().revision, 1);
    let after = bundle_native_snapshot(&connection);
    assert_eq!(before.into_iter().filter(|(name,_)| name != "agent_dialog_views").collect::<Vec<_>>(),
        after.into_iter().filter(|(name,_)| name != "agent_dialog_views").collect::<Vec<_>>());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn dialog_view_schema_upgrade_is_idempotent_and_scan_deletion_cascades() {
    let (root, path, connection, sequence) = dialog_view_fixture();
    let before = persist_dialog_view(&connection, &dialog_view_input(0, "team", Some(sequence))).unwrap();
    drop(connection);
    assert_eq!(db::initialize(&root.join("app")).unwrap(), path);
    assert_eq!(db::initialize(&root.join("app")).unwrap(), path);
    let connection = db::open(&path).unwrap();
    assert_eq!(load_dialog_view(&connection, "source-regression", 1).unwrap(), before);
    connection.execute("DELETE FROM sentinel_scans WHERE id='source-regression'", []).unwrap();
    assert_eq!(connection.query_row("SELECT COUNT(*) FROM agent_dialog_views", [], |row| row.get::<_,i64>(0)).unwrap(), 0);
    assert_eq!(persist_dialog_view(&connection, &dialog_view_input(0, "", None)).unwrap_err(), "dialog_view_attempt_changed_or_deleted");
    fs::remove_dir_all(root).unwrap();
}
