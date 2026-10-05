fn dialog_selection_fixture() -> (PathBuf, PathBuf, rusqlite::Connection, i64, i64) {
    let (root, path, connection, _) = dialog_view_fixture();
    connection.execute("INSERT INTO projects(name) VALUES('dialog-project-one')", []).unwrap();
    let one = connection.last_insert_rowid();
    connection.execute("INSERT INTO projects(name) VALUES('dialog-project-two')", []).unwrap();
    let two = connection.last_insert_rowid();
    connection.execute("UPDATE sentinel_scans SET project_id=?1,updated_at='2000-01-01' WHERE id='source-regression'", [one]).unwrap();
    connection.execute("INSERT INTO sentinel_scans(id,project_id,project_name,status,attempt_count) VALUES('other-task',?1,'other','completed',1)", [two]).unwrap();
    (root, path, connection, one, two)
}

fn dialog_selection_input(project: Option<i64>, revision: i64, scan: &str) -> AgentDialogSelectionInput {
    AgentDialogSelectionInput { project_id: project, expected_revision: revision, scan_id: scan.into() }
}

#[test]
fn dialog_selection_restores_older_than_first_page_without_mutation_or_pagination() {
    let (root, path, connection, project, _) = dialog_selection_fixture();
    connection.execute("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<305)
        INSERT INTO sentinel_scans(id,project_id,project_name,status,updated_at)
        SELECT 'recent-'||x,?1,'recent','completed','2001-01-01' FROM n", [project]).unwrap();
    let before = bundle_native_snapshot(&connection);
    assert_eq!(load_dialog_selection(&connection, Some(project)).unwrap().revision, 0);
    assert_eq!(bundle_native_snapshot(&connection), before);
    let page = list_sentinel_scans_inner(&path, Some(project), Some(300), None, None).unwrap();
    assert_eq!(page.len(), 300);
    assert!(!page.iter().any(|scan| scan.id == "source-regression"));
    persist_dialog_selection(&connection, &dialog_selection_input(Some(project), 0, "source-regression")).unwrap();
    let after = bundle_native_snapshot(&connection);
    assert_eq!(before.into_iter().filter(|(name,_)| name != "agent_dialog_selections").collect::<Vec<_>>(),
        after.iter().filter(|(name,_)| name != "agent_dialog_selections").cloned().collect::<Vec<_>>());
    drop(connection);
    let connection = db::open(&path).unwrap();
    let restored = load_dialog_selection(&connection, Some(project)).unwrap();
    assert_eq!(restored.selected_scan.unwrap().id, "source-regression");
    assert_eq!(restored.revision, 1);
    assert_eq!(bundle_native_snapshot(&connection), after);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn dialog_selection_project_scope_global_scope_and_tombstones_never_cross() {
    let (root, _, connection, one, two) = dialog_selection_fixture();
    assert!(dialog_task_in(&connection, Some(two), "source-regression").unwrap().is_none());
    let before = bundle_native_snapshot(&connection);
    assert!(persist_dialog_selection(&connection, &dialog_selection_input(Some(two), 0, "source-regression")).is_err());
    assert!(load_dialog_selection(&connection, Some(999_999)).is_err());
    assert!(load_dialog_selection(&connection, Some(0)).is_err());
    assert_eq!(bundle_native_snapshot(&connection), before);
    persist_dialog_selection(&connection, &dialog_selection_input(Some(one), 0, "source-regression")).unwrap();
    persist_dialog_selection(&connection, &dialog_selection_input(None, 0, "other-task")).unwrap();
    assert_eq!(load_dialog_selection(&connection, Some(two)).unwrap().revision, 0);
    assert_eq!(load_dialog_selection(&connection, None).unwrap().selected_scan.unwrap().id, "other-task");
    connection.execute("INSERT INTO sentinel_deleted_scans(scan_id) VALUES('source-regression')", []).unwrap();
    let before = bundle_native_snapshot(&connection);
    let unavailable = load_dialog_selection(&connection, Some(one)).unwrap();
    assert!(unavailable.selection_unavailable && unavailable.selected_scan.is_none());
    assert_eq!(unavailable.revision, 1);
    assert!(persist_dialog_selection(&connection, &dialog_selection_input(Some(one), 1, "source-regression")).is_err());
    assert_eq!(bundle_native_snapshot(&connection), before);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn dialog_selection_cas_and_postwrite_scope_drift_roll_back() {
    let (root, _, connection, one, two) = dialog_selection_fixture();
    persist_dialog_selection(&connection, &dialog_selection_input(Some(one), 0, "source-regression")).unwrap();
    let before = bundle_native_snapshot(&connection);
    assert_eq!(persist_dialog_selection(&connection, &dialog_selection_input(Some(one), 0, "source-regression")).err().unwrap(), "dialog_selection_revision_conflict");
    assert_eq!(bundle_native_snapshot(&connection), before);
    connection.execute_batch(&format!("CREATE TRIGGER move_dialog_task AFTER UPDATE ON agent_dialog_selections BEGIN UPDATE sentinel_scans SET project_id={two} WHERE id=NEW.scan_id; END;")).unwrap();
    assert_eq!(persist_dialog_selection(&connection, &dialog_selection_input(Some(one), 1, "source-regression")).err().unwrap(), "dialog_selection_write_not_persisted");
    assert_eq!(bundle_native_snapshot(&connection), before);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn dialog_selection_upgrade_and_deletion_preserve_conflict_history_without_reviving_tasks() {
    let (root, path, connection, one, _) = dialog_selection_fixture();
    persist_dialog_selection(&connection, &dialog_selection_input(Some(one), 0, "source-regression")).unwrap();
    drop(connection);
    db::initialize(&root.join("app")).unwrap();
    db::initialize(&root.join("app")).unwrap();
    let connection = db::open(&path).unwrap();
    assert_eq!(load_dialog_selection(&connection, Some(one)).unwrap().revision, 1);
    connection.execute("DELETE FROM sentinel_scans WHERE id='source-regression'", []).unwrap();
    let saved = load_dialog_selection(&connection, Some(one)).unwrap();
    assert_eq!(saved.revision, 1);
    assert!(saved.selected_scan_id.is_none() && saved.selection_unavailable);
    connection.execute("INSERT INTO sentinel_scans(id,project_id,project_name,status) VALUES('replacement',?1,'replacement','completed')", [one]).unwrap();
    assert_eq!(persist_dialog_selection(&connection, &dialog_selection_input(Some(one), 0, "replacement")).err().unwrap(), "dialog_selection_revision_conflict");
    assert_eq!(persist_dialog_selection(&connection, &dialog_selection_input(Some(one), 1, "replacement")).unwrap().revision, 2);
    connection.execute("DELETE FROM projects WHERE id=?1", [one]).unwrap();
    assert_eq!(connection.query_row("SELECT COUNT(*) FROM agent_dialog_selections", [], |r|r.get::<_,i64>(0)).unwrap(), 0);
    assert!(load_dialog_selection(&connection, Some(one)).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn dialog_selection_concurrent_windows_have_one_winner_without_execution_changes() {
    let (root, path, connection, _, _) = dialog_selection_fixture();
    let before = bundle_native_snapshot(&connection);
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let workers: Vec<_> = ["source-regression", "other-task"].into_iter().map(|scan| {
        let path = path.clone(); let barrier = barrier.clone();
        std::thread::spawn(move || {
            let db = db::open(&path).unwrap();
            assert_eq!(load_dialog_selection(&db, None).unwrap().revision, 0);
            barrier.wait();
            persist_dialog_selection(&db, &dialog_selection_input(None, 0, scan))
                .map(|saved| saved.selected_scan_id.unwrap())
        })
    }).collect();
    let results: Vec<_> = workers.into_iter().map(|worker| worker.join().unwrap()).collect();
    let winners: Vec<_> = results.iter().filter_map(|result| result.as_ref().ok()).collect();
    assert_eq!(winners.len(), 1);
    assert_eq!(results.iter().filter_map(|result| result.as_ref().err()).collect::<Vec<_>>(),
        vec![&"dialog_selection_revision_conflict".to_string()]);
    let restored = load_dialog_selection(&connection, None).unwrap();
    assert_eq!(restored.revision, 1);
    assert_eq!(restored.selected_scan_id.as_ref(), Some(winners[0]));
    let after = bundle_native_snapshot(&connection);
    assert_eq!(before.into_iter().filter(|(name,_)| name != "agent_dialog_selections").collect::<Vec<_>>(),
        after.into_iter().filter(|(name,_)| name != "agent_dialog_selections").collect::<Vec<_>>());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn dialog_selection_invalid_inputs_and_damaged_records_are_read_only_errors() {
    let (root, _, connection, project, _) = dialog_selection_fixture();
    let before = bundle_native_snapshot(&connection);
    for revision in [-1, DIALOG_VIEW_MAX_INTEGER, i64::MAX] {
        assert!(persist_dialog_selection(&connection,
            &dialog_selection_input(Some(project), revision, "source-regression")).is_err());
    }
    for id in [String::new(), "x".repeat(257)] {
        assert!(dialog_task_in(&connection, Some(project), &id).is_err());
        assert!(persist_dialog_selection(&connection,
            &dialog_selection_input(Some(project), 0, &id)).is_err());
    }
    // SQL CHECK must reject NULL's three-valued-logic escape, not just reads.
    assert!(connection.execute("INSERT INTO agent_dialog_selections(scope_key,revision) VALUES('project:999',1)", []).is_err());
    assert!(connection.execute("INSERT INTO agent_dialog_selections(scope_key,revision) VALUES(NULL,1)", []).is_err());
    assert_eq!(bundle_native_snapshot(&connection), before);
    persist_dialog_selection(&connection, &dialog_selection_input(Some(project), 0, "source-regression")).unwrap();
    connection.execute("UPDATE agent_dialog_selections SET revision=?1", [DIALOG_VIEW_MAX_INTEGER + 1]).unwrap();
    let damaged = bundle_native_snapshot(&connection);
    assert_eq!(load_dialog_selection(&connection, Some(project)).err().unwrap(), "dialog_selection_invalid_record");
    assert!(persist_dialog_selection(&connection, &dialog_selection_input(Some(project), 1, "source-regression")).is_err());
    assert_eq!(bundle_native_snapshot(&connection), damaged);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn dialog_selection_insert_damage_rolls_back_the_entire_transaction() {
    let (root, _, connection, project, _) = dialog_selection_fixture();
    connection.execute_batch("CREATE TRIGGER damage_dialog_selection AFTER INSERT ON agent_dialog_selections BEGIN
        UPDATE sentinel_scans SET status='scanning' WHERE id=NEW.scan_id;
        UPDATE agent_dialog_selections SET revision=NEW.revision+1 WHERE scope_key=NEW.scope_key;
        END;").unwrap();
    let before = bundle_native_snapshot(&connection);
    assert_eq!(persist_dialog_selection(&connection, &dialog_selection_input(Some(project), 0, "source-regression")).err().unwrap(), "dialog_selection_write_not_persisted");
    assert_eq!(bundle_native_snapshot(&connection), before);
    fs::remove_dir_all(root).unwrap();
}
