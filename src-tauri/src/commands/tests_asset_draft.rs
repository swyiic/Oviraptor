fn asset_draft_fixture() -> (PathBuf, rusqlite::Connection) {
    let (root, connection, _) = web_draft_fixture();
    connection.execute("INSERT INTO projects(id,name) VALUES(2,'Other workspace')", []).unwrap();
    for (id, url, project, deleted) in [
        (1, "https://asset.example.test/first", 1, 0),
        (2, "https://asset.example.test/second", 1, 0),
        (3, "https://asset.example.test/first", 1, 0),
        (4, "https://asset.example.test/fused", 1, 0),
        (5, "https://asset.example.test/foreign", 2, 0),
        (6, "https://asset.example.test/deleted", 1, 1),
    ] {
        connection.execute("INSERT INTO assets(id,asset_key,company,link) VALUES(?1,?2,?2,?3)", params![id, format!("asset-{id}"), url]).unwrap();
        connection.execute("INSERT INTO project_assets(project_id,asset_id,is_deleted) VALUES(?1,?2,?3)", params![project,id,deleted]).unwrap();
    }
    connection.execute("INSERT INTO sentinel_fuse_zone(project_id,url,normalized_url) VALUES(1,'https://asset.example.test/fused','https://asset.example.test/fused')", []).unwrap();
    // Preexisting user data must survive every failing attempt unchanged.
    connection.execute("INSERT INTO sentinel_scans(id,project_id,status,task_name) VALUES('preserved',1,'completed','Keep existing task')", []).unwrap();
    (root, connection)
}

fn submit_asset_draft(connection: &rusqlite::Connection) -> Result<SentinelScan, String> {
    create_sentinel_asset_scan_in(connection, 1, vec![2,1], Some("deep".into()))
}

#[test]
fn native_draft_status_expects_no_completion_receipts_and_never_grants_execution() {
    let (root, connection)=asset_draft_fixture();
    let scan=submit_asset_draft(&connection).unwrap();
    let before=web_draft_snapshot(&connection);
    let value=native_scan_status(&connection,&scan.id).unwrap();
    assert_eq!(value["stopDiagnostic"]["code"],"draft_waiting_confirmation");
    assert_eq!(value["stopDiagnostic"]["category"],"draft");
    assert_eq!(value["stopDiagnostic"]["nextAction"],"confirm_scope_and_start");
    assert_eq!(value["stopDiagnostic"]["automaticResumeAllowed"],false);
    assert_eq!(value["stopDiagnostic"]["obligations"],json!([]));
    assert_eq!(value["attemptNumber"],0);
    assert_eq!(web_draft_snapshot(&connection),before,"status read must not manufacture a receipt or execution attempt");
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_draft_status_does_not_hide_execution_history_usage_or_physical_owners() {
    for fault in [
        "UPDATE sentinel_scans SET attempt_count=1 WHERE id=?1",
        "UPDATE sentinel_scans SET task_path='/retained/attempt' WHERE id=?1",
        "UPDATE sentinel_scans SET llm_requests=1 WHERE id=?1",
        "UPDATE sentinel_scans SET total_tokens=4 WHERE id=?1",
        "INSERT INTO sentinel_scan_attempts(scan_id,attempt_number) VALUES(?1,1)",
        "INSERT INTO sentinel_processes(scan_id,process_id) VALUES(?1,123)",
        "INSERT INTO agent_runs(id,scan_id,status) VALUES('draft-history-run',?1,'completed')",
        "UPDATE sentinel_targets SET last_attempt_number=1 WHERE scan_id=?1",
        "UPDATE sentinel_targets SET status='failed' WHERE scan_id=?1",
        "INSERT INTO analyzer_container_receipts(receipt_id,invocation_key,scan_id,attempt_number,purpose,container_name,owner_token,create_args_json,cleanup_status) VALUES('draft-cleanup','draft-cleanup',?1,1,'fixture','fixture-only','fixture','[]','confirmed')",
    ] {
        let (root,connection)=asset_draft_fixture();let scan=submit_asset_draft(&connection).unwrap();
        connection.execute(fault,[&scan.id]).unwrap();
        let value=native_scan_status(&connection,&scan.id).unwrap();
        assert_ne!(value["stopDiagnostic"]["category"],"draft","{fault}");
        assert_eq!(value["stopDiagnostic"]["automaticResumeAllowed"],false);
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn asset_draft_commits_exact_scoped_deduplicated_targets_without_starting() {
    let (root, connection) = asset_draft_fixture();
    let before = web_draft_snapshot(&connection);
    let scan = create_sentinel_asset_scan_in(&connection, 1, vec![6,5,4,3,2,1,1,999], Some("deep".into())).unwrap();
    assert_eq!(scan.status, "draft");
    assert_eq!(scan.scan_type, "web");
    assert!(scan.current_checkpoint.contains("已加入 2 个 URL"));
    assert!(scan.current_checkpoint.contains("4 个所选资产"));
    assert!(scan.current_checkpoint.contains("合并 1 个重复 URL"));
    let mut statement = connection.prepare("SELECT asset_id,company,url FROM sentinel_targets WHERE scan_id=?1 ORDER BY asset_id").unwrap();
    let actual = statement.query_map([&scan.id], |row| Ok((row.get::<_,i64>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?)))
        .unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    assert_eq!(actual, vec![
        (1,"asset-1".into(),"https://asset.example.test/first".into()),
        (2,"asset-2".into(),"https://asset.example.test/second".into()),
    ]);
    drop(statement);
    let policy: String = connection.query_row("SELECT policy_json FROM sentinel_scan_contexts WHERE scan_id=?1", [&scan.id], |row| row.get(0)).unwrap();
    let mut expected = build_web_investigation_policy(Some("deep"),None,Vec::new(),&[],"","asset-workspace",Some("breadth")).unwrap();
    // Every new ordinary Web draft freezes its orchestration mode in the same
    // creation transaction (web_mode::draft::register_new); asset drafts have no
    // mode input and take the new-draft default, Multi. Nothing else is added.
    expected.as_object_mut().unwrap().insert("orchestration".into(), serde_json::json!({"version":1,"mode":"multi"}));
    assert_eq!(serde_json::from_str::<JsonValue>(&policy).unwrap(), expected);
    assert_eq!(
        crate::agent_runtime::web_mode::draft::read(&connection, &scan.id).unwrap().1,
        crate::agent_runtime::web_mode::WebMode::Multi,
        "the declared mode and its draft receipt agree"
    );
    let after = web_draft_snapshot(&connection);
    assert_eq!(after["sentinel_scans"], 2);
    assert_eq!(after["sentinel_scan_contexts"], 1);
    assert_eq!(after["sentinel_targets"], 2);
    for key in ["sentinel_scan_attempts","agent_runs","agent_authorization_controls","identities"] {
        assert_eq!(after[key], before[key], "creating a draft must not execute or consume identities: {key}");
    }
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn asset_draft_rolls_back_aborted_and_silently_ignored_writes() {
    for (table, condition) in [
        ("sentinel_scans", "1"),
        ("sentinel_scan_contexts", "1"),
        ("sentinel_targets", "NEW.url LIKE '%/second'"),
    ] {
        for fault in ["ABORT,'injected'", "IGNORE"] {
            let (root, connection) = asset_draft_fixture();
            let before = web_draft_snapshot(&connection);
            connection.execute_batch(&format!("CREATE TRIGGER asset_draft_fault BEFORE INSERT ON {table} WHEN {condition} BEGIN SELECT RAISE({fault}); END;")).unwrap();
            assert!(submit_asset_draft(&connection).is_err(), "{table}/{fault}");
            assert_eq!(before, web_draft_snapshot(&connection), "{table}/{fault}");
            connection.execute_batch("DROP TRIGGER asset_draft_fault").unwrap();
            assert_eq!(submit_asset_draft(&connection).unwrap().status, "draft");
            drop(connection);
            fs::remove_dir_all(root).unwrap();
        }
    }
}

#[test]
fn asset_draft_detects_after_insert_deletion_or_changed_persisted_content() {
    for (table, body) in [
        ("sentinel_scan_contexts", "DELETE FROM sentinel_scan_contexts WHERE scan_id=NEW.scan_id"),
        ("sentinel_scan_contexts", "UPDATE sentinel_scan_contexts SET policy_json='{}' WHERE scan_id=NEW.scan_id"),
        ("sentinel_targets", "DELETE FROM sentinel_targets WHERE id=NEW.id"),
        ("sentinel_targets", "UPDATE sentinel_targets SET project_id=2 WHERE id=NEW.id"),
        ("sentinel_targets", "UPDATE sentinel_targets SET status='completed' WHERE id=NEW.id"),
        ("sentinel_targets", "UPDATE sentinel_scans SET status='scanning' WHERE id=NEW.scan_id"),
    ] {
        let (root, connection) = asset_draft_fixture();
        let before = web_draft_snapshot(&connection);
        connection.execute_batch(&format!("CREATE TRIGGER asset_draft_fault AFTER INSERT ON {table} BEGIN {body}; END;")).unwrap();
        assert!(submit_asset_draft(&connection).is_err(), "{body}");
        assert_eq!(before, web_draft_snapshot(&connection), "{body}");
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn asset_draft_preflight_errors_leave_existing_tasks_and_identities_unchanged() {
    let (root, connection) = asset_draft_fixture();
    let before = web_draft_snapshot(&connection);
    for ids in [vec![], vec![4,5,6,999]] {
        assert!(create_sentinel_asset_scan_in(&connection,1,ids,None).is_err());
        assert_eq!(before,web_draft_snapshot(&connection));
    }
    connection.execute("UPDATE projects SET status='archived' WHERE id=1", []).unwrap();
    assert!(submit_asset_draft(&connection).is_err());
    assert_eq!(before,web_draft_snapshot(&connection));
    connection.execute("UPDATE projects SET status='active' WHERE id=1", []).unwrap();
    connection.execute_batch("DROP TABLE sentinel_fuse_zone").unwrap();
    assert!(submit_asset_draft(&connection).is_err(), "fuse read failures must not default to unblocked");
    assert_eq!(before,web_draft_snapshot(&connection));
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn asset_draft_rechecks_project_after_a_concurrent_archive_commits() {
    let (root, connection) = asset_draft_fixture();
    let before = web_draft_snapshot(&connection);
    let path = PathBuf::from(connection.path().unwrap());
    let other = db::open(&path).unwrap();
    let transaction = rusqlite::Transaction::new_unchecked(&connection,rusqlite::TransactionBehavior::Immediate).unwrap();
    transaction.execute("UPDATE projects SET status='archived' WHERE id=1", []).unwrap();
    let (sender,receiver) = std::sync::mpsc::channel();
    let creator = std::thread::spawn(move || {
        sender.send(()).unwrap();
        submit_asset_draft(&other)
    });
    receiver.recv().unwrap();
    transaction.commit().unwrap();
    assert!(creator.join().unwrap().unwrap_err().contains("已归档"));
    assert_eq!(before,web_draft_snapshot(&connection));
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}
