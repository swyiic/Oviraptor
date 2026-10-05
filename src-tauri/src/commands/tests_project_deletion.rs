fn project_deletion_fixture() -> (PathBuf, PathBuf, rusqlite::Connection) {
    let root = std::env::temp_dir().join(format!("oviraptor-project-delete-{}", Uuid::new_v4()));
    let path = db::initialize(&root).unwrap();
    let connection = db::open(&path).unwrap();
    connection.execute("INSERT INTO projects(id,name) VALUES(91,'Empty workspace')", []).unwrap();
    (root, path, connection)
}

#[test]
fn project_deletion_holds_writer_admission_until_commit() {
    let (root, path, mut connection) = project_deletion_fixture();
    let writer = db::open(&path).unwrap();
    writer.busy_timeout(std::time::Duration::ZERO).unwrap();
    let mut insertion = None;
    delete_project_for_connection(&mut connection, 91, || {
        insertion = Some(writer.execute(
            "INSERT INTO exposure_runs(project_id) VALUES(91)", [],
        ));
    }).unwrap();
    assert!(matches!(insertion.unwrap(), Err(rusqlite::Error::SqliteFailure(error, _))
        if error.code == rusqlite::ErrorCode::DatabaseBusy),
        "another writer must not create records between admission and deletion");
    assert!(writer.execute("INSERT INTO exposure_runs(project_id) VALUES(91)", []).is_err());
    drop(writer);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn project_deletion_preserves_previously_uncounted_records() {
    let fixtures = [
        "INSERT INTO exposure_runs(project_id) VALUES(91)",
        "INSERT INTO exposure_findings(project_id,category,title,fingerprint) VALUES(91,'test','Keep','keep')",
        "INSERT INTO asset_ownership_profiles(project_id,legal_name) VALUES(91,'Keep policy')",
        "INSERT INTO asset_ownership_rules(project_id,rule_type,pattern,action) VALUES(91,'domain','example.test','allow')",
        "INSERT INTO knowledge_strategies(project_id,strategy_key,title) VALUES(91,'keep','Keep knowledge')",
    ];
    for sql in fixtures {
        let (root, _, mut connection) = project_deletion_fixture();
        connection.execute(sql, []).unwrap();
        let before = deletion_snapshot(&connection);
        assert!(project_impact_for_connection(&connection, 91).unwrap().total_records > 0, "{sql}");
        assert!(delete_project_for_connection(&mut connection, 91, || {}).is_err(), "{sql}");
        assert_eq!(deletion_snapshot(&connection), before, "{sql}");
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn project_deletion_counts_new_and_legacy_ownership_without_double_counting() {
    let (root, _, mut connection) = project_deletion_fixture();
    connection.execute_batch(
        "CREATE TABLE test_project_legacy(project_id INTEGER, payload TEXT);
         CREATE TABLE test_project_future(owner INTEGER REFERENCES projects(id) ON DELETE CASCADE,
             reviewer INTEGER REFERENCES projects(id) ON DELETE SET NULL);
         CREATE TABLE \"test_project_quoted\"\"name\"(project_id INTEGER);
         INSERT INTO test_project_legacy VALUES(91,'keep');
         INSERT INTO test_project_future VALUES(91,91);
         INSERT INTO \"test_project_quoted\"\"name\" VALUES(91);
         INSERT INTO saved_views(project_id,name) VALUES(91,'one categorized view');"
    ).unwrap();
    let impact = project_impact_for_connection(&connection, 91).unwrap();
    assert_eq!(impact.saved_view_count, 1);
    assert_eq!(impact.other_record_count, 3);
    assert_eq!(impact.total_records, 4);
    let before = deletion_snapshot(&connection);
    assert!(delete_project_for_connection(&mut connection, 91, || {}).is_err());
    assert_eq!(deletion_snapshot(&connection), before);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn project_deletion_rechecks_stale_preview_and_never_detaches_scans() {
    for status in ["draft", "scanning", "paused", "completed", "failed", "cancelled"] {
        let (root, path, mut connection) = project_deletion_fixture();
        assert_eq!(project_impact_for_connection(&connection, 91).unwrap().total_records, 0);
        let writer = db::open(&path).unwrap();
        writer.execute("INSERT INTO sentinel_scans(id,project_id,status) VALUES('keep',91,?1)", [status]).unwrap();
        let before = deletion_snapshot(&connection);
        assert!(delete_project_for_connection(&mut connection, 91, || {}).unwrap_err().contains("请归档"));
        assert_eq!(deletion_snapshot(&connection), before, "{status}");
        drop(writer);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn project_deletion_busy_writer_then_committed_record_remains_protected() {
    let (root, path, mut connection) = project_deletion_fixture();
    connection.busy_timeout(std::time::Duration::ZERO).unwrap();
    let mut writer = db::open(&path).unwrap();
    let transaction = writer.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
    transaction.execute("INSERT INTO exposure_runs(project_id) VALUES(91)", []).unwrap();
    let before = deletion_snapshot(&connection);
    assert!(delete_project_for_connection(&mut connection, 91, || panic!("must not admit")).is_err());
    assert_eq!(deletion_snapshot(&connection), before);
    transaction.commit().unwrap();
    let before = deletion_snapshot(&connection);
    assert!(delete_project_for_connection(&mut connection, 91, || panic!("must not admit")).is_err());
    assert_eq!(deletion_snapshot(&connection), before);
    drop(writer);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn project_deletion_empty_success_is_scoped_and_missing_ids_preserve_database() {
    let (root, _, mut connection) = project_deletion_fixture();
    connection.execute_batch(
        "INSERT INTO projects(id,name) VALUES(92,'Keep other project');
         INSERT INTO exposure_runs(project_id) VALUES(92);
         INSERT INTO assets(id,asset_key) VALUES(991,'keep-orphan');"
    ).unwrap();
    let before = deletion_snapshot(&connection);
    for id in [0, -1, i64::MAX] {
        assert!(delete_project_for_connection(&mut connection, id, || panic!("must not admit")).is_err());
        assert_eq!(deletion_snapshot(&connection), before);
    }
    let other_rows = application_table_snapshot(&connection).into_iter()
        .filter(|(name, _)| name != "projects").collect::<Vec<_>>();
    delete_project_for_connection(&mut connection, 91, || {}).unwrap();
    assert_eq!(connection.query_row("SELECT COUNT(*) FROM projects WHERE id=91", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    assert_eq!(application_table_snapshot(&connection).into_iter()
        .filter(|(name, _)| name != "projects").collect::<Vec<_>>(), other_rows);
    let snapshot_after = application_table_snapshot(&connection);
    assert!(delete_project_for_connection(&mut connection, 91, || {}).is_err());
    assert_application_tables_unchanged(&connection, &snapshot_after);
    assert_eq!(connection.query_row("SELECT COUNT(*) FROM exposure_runs WHERE project_id=92", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    assert_eq!(connection.query_row("SELECT COUNT(*) FROM assets WHERE id=991", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn project_deletion_faults_rollback_ignored_recreated_and_commit_failed_deletes() {
    let faults = [
        "CREATE TRIGGER test_delete BEFORE DELETE ON projects WHEN OLD.id=91 BEGIN
            SELECT RAISE(ABORT,'injected delete failure'); END;",
        "CREATE TRIGGER test_delete BEFORE DELETE ON projects WHEN OLD.id=91 BEGIN
            UPDATE projects SET name='must roll back' WHERE id=92; SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER test_delete AFTER DELETE ON projects WHEN OLD.id=91 BEGIN
            INSERT INTO projects(id,name) VALUES(91,'recreated'); END;",
        "CREATE TABLE test_commit(project_id INTEGER REFERENCES projects(id) DEFERRABLE INITIALLY DEFERRED);
         CREATE TRIGGER test_delete AFTER DELETE ON projects WHEN OLD.id=91 BEGIN
            INSERT INTO test_commit VALUES(91); END;",
    ];
    for sql in faults {
        let (root, _, mut connection) = project_deletion_fixture();
        connection.execute("INSERT INTO projects(id,name) VALUES(92,'Keep')", []).unwrap();
        connection.execute_batch(sql).unwrap();
        let before = deletion_snapshot(&connection);
        assert!(delete_project_for_connection(&mut connection, 91, || {}).is_err(), "{sql}");
        assert_eq!(deletion_snapshot(&connection), before, "{sql}");
        assert!(connection.is_autocommit());
        connection.execute_batch("DROP TRIGGER test_delete").unwrap();
        delete_project_for_connection(&mut connection, 91, || {}).unwrap();
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}
