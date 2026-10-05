#[test]
fn findings_read_boundary_never_refreshes_saved_inventory_from_live_source() {
    let (root,state,connection)=bundle_import_fixture();
    let repository=root.join("repository");fs::create_dir(&repository).unwrap();
    fs::write(repository.join("new.rs"),"fn now() {}\n").unwrap();
    connection.execute("UPDATE sentinel_scans SET scan_type='code',source_path=?1 WHERE id='live'",[repository.to_str().unwrap()]).unwrap();
    connection.execute("INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title,record_json) VALUES('live',?1,'local-inventory','source_inventory','repository','Historical inventory','{\"languages\":{\"OldLanguage\":7},\"futureField\":true}')",[repository.to_str().unwrap()]).unwrap();
    let before=bundle_native_snapshot(&connection);
    let rows=sentinel_findings_in(&connection,"live",None).unwrap();
    assert_eq!(rows.len(),1);
    assert_eq!(rows[0].record_json,"{\"languages\":{\"OldLanguage\":7},\"futureField\":true}","viewing history must not replace saved evidence with the current repository");
    assert_eq!(bundle_native_snapshot(&connection),before);
    assert_eq!(fs::read(repository.join("new.rs")).unwrap(),b"fn now() {}\n");
    fs::rename(&repository, root.join("moved-repository")).unwrap();
    assert_eq!(sentinel_findings_in(&connection,"live",None).unwrap()[0].record_json,rows[0].record_json);
    assert_eq!(bundle_native_snapshot(&connection),before,"missing original source directory must not alter saved inventory");
    drop(state);drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn findings_read_boundary_supports_readonly_database_without_synthesizing_inventory() {
    let (root,state,connection)=bundle_import_fixture();
    let repository=root.join("repository");fs::create_dir(&repository).unwrap();
    fs::write(repository.join("app.js"),"const value = 1;\n").unwrap();
    connection.execute("UPDATE sentinel_scans SET scan_type='code',source_path=?1 WHERE id='live'",[repository.to_str().unwrap()]).unwrap();
    let before=bundle_native_snapshot(&connection);
    connection.execute_batch("PRAGMA query_only=ON").unwrap();
    assert!(sentinel_findings_in(&connection,"live",Some("vulnerability")).unwrap().is_empty());
    assert!(sentinel_findings_in(&connection,"live",None).unwrap().is_empty());
    assert_eq!(bundle_native_snapshot(&connection),before);
    drop(state);drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn findings_read_boundary_tombstone_hides_rows_and_task_index_without_deleting_bytes() {
    let (root,state,connection)=bundle_import_fixture();
    connection.execute_batch("INSERT INTO sentinel_findings(scan_id,stage,kind,record_key,title) VALUES('live','native','vulnerability','keep','Keep bytes');
        INSERT INTO sentinel_deleted_scans(scan_id) VALUES('live');").unwrap();
    let before=bundle_native_snapshot(&connection);
    assert!(sentinel_findings_in(&connection,"live",None).unwrap().is_empty());
    assert!(sentinel_vulnerability_scan_ids_in(&connection,None).unwrap().is_empty());
    assert_eq!(bundle_native_snapshot(&connection),before);
    drop(state);drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn findings_read_boundary_proof_index_matches_detail_for_typed_and_trimmed_fields() {
    let (root,state,connection)=bundle_import_fixture();
    connection.execute_batch("INSERT INTO sentinel_scan_contexts(scan_id,policy_json) VALUES('live','{\"closure\":\"proof\"}');
        INSERT INTO sentinel_findings(scan_id,stage,kind,record_key,title) VALUES('live','native','vulnerability','candidate','Candidate');").unwrap();
    for (record,visible) in [
        (json!({"controlRequestId":1,"testRequestId":2,"impact":"number IDs"}).to_string(),false),
        (json!({"controlRequestId":"same","testRequestId":" same ","impact":"identical IDs"}).to_string(),false),
        (json!({"controlRequestId":"\t","testRequestId":"test","impact":"empty ID"}).to_string(),false),
        (json!({"controlRequestId":"control","testRequestId":"test","impact":true}).to_string(),false),
        ("broken-json".to_string(),false),
        (json!({"controlRequestId":"control","testRequestId":"test","impact":"Observed effect"}).to_string(),true),
    ] {
        connection.execute("UPDATE sentinel_findings SET record_json=?1 WHERE scan_id='live'",[&record]).unwrap();
        let detail=sentinel_findings_in(&connection,"live",None).unwrap();
        let index=sentinel_vulnerability_scan_ids_in(&connection,Some(1)).unwrap();
        assert_eq!(!detail.is_empty(),visible,"detail: {record}");
        assert_eq!(!index.is_empty(),visible,"index: {record}");
    }
    drop(state);drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn findings_read_boundary_rejects_invalid_policy_without_broadening_visibility() {
    let (root,state,connection)=bundle_import_fixture();
    connection.execute_batch("INSERT INTO sentinel_scan_contexts(scan_id,policy_json) VALUES('live','{}');
        INSERT INTO sentinel_findings(scan_id,stage,kind,record_key,title) VALUES('live','native','vulnerability','candidate','Candidate');").unwrap();
    for policy in ["broken-json", "null", "[]", "true", r#"{"closure":null}"#, r#"{"closure":true}"#, r#"{"closure":"unknown"}"#, r#"{"closure":" proof "}"#] {
        connection.execute("UPDATE sentinel_scan_contexts SET policy_json=?1 WHERE scan_id='live'",[policy]).unwrap();
        let before=bundle_native_snapshot(&connection);
        assert_eq!(sentinel_findings_in(&connection,"live",None).unwrap_err(),"findings_policy_invalid","{policy}");
        assert_eq!(sentinel_vulnerability_scan_ids_in(&connection,None).unwrap_err(),"findings_policy_invalid","{policy}");
        assert_eq!(bundle_native_snapshot(&connection),before);
    }
    drop(state);drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn findings_read_boundary_legacy_policy_and_non_vulnerability_records_stay_readable() {
    let (root,state,connection)=bundle_import_fixture();
    connection.execute_batch("INSERT INTO sentinel_findings(scan_id,stage,kind,record_key,title) VALUES
        ('live','native','vulnerability','candidate','Candidate'),
        ('live','local-inventory','source_inventory','inventory','Saved inventory');").unwrap();
    for policy in [None,Some("{}"),Some(r#"{"closure":"breadth"}"#),Some(r#"{"futureField":true}"#)] {
        connection.execute("DELETE FROM sentinel_scan_contexts WHERE scan_id='live'",[]).unwrap();
        if let Some(policy)=policy {
            connection.execute("INSERT INTO sentinel_scan_contexts(scan_id,policy_json) VALUES('live',?1)",[policy]).unwrap();
        }
        assert_eq!(sentinel_findings_in(&connection,"live",None).unwrap().len(),2);
        assert_eq!(sentinel_vulnerability_scan_ids_in(&connection,None).unwrap(),vec!["live"]);
    }
    connection.execute("UPDATE sentinel_scan_contexts SET policy_json=?1 WHERE scan_id='live'",[r#"{"closure":"proof"}"#]).unwrap();
    let before=bundle_native_snapshot(&connection);
    let rows=sentinel_findings_in(&connection,"live",None).unwrap();
    assert_eq!(rows.len(),1);assert_eq!(rows[0].kind,"source_inventory");
    assert!(sentinel_findings_in(&connection,"live",Some("vulnerability")).unwrap().is_empty());
    assert!(sentinel_findings_in(&connection,"unknown",None).unwrap().is_empty());
    assert_eq!(bundle_native_snapshot(&connection),before);
    drop(state);drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn findings_read_boundary_index_is_scoped_deduplicated_ordered_and_readonly() {
    let (root,state,connection)=bundle_import_fixture();
    connection.execute_batch("INSERT INTO projects(id,name) VALUES(2,'Other project');
        UPDATE sentinel_scans SET updated_at='2026-09-26' WHERE id='live';
        INSERT INTO sentinel_scans(id,project_id,updated_at) VALUES('a',1,'2026-09-27'),('b',1,'2026-09-27'),('other',2,'2026-09-28');
        INSERT INTO sentinel_findings(scan_id,stage,kind,record_key,title) VALUES
        ('live','native','vulnerability','one','One'),('live','native','vulnerability','two','Two'),
        ('a','native','vulnerability','one','One'),('b','native','vulnerability','one','One'),
        ('other','native','vulnerability','one','One');").unwrap();
    let before=bundle_native_snapshot(&connection);
    let reader=rusqlite::Connection::open_with_flags(&state.db_path,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert_eq!(sentinel_vulnerability_scan_ids_in(&reader,None).unwrap(),vec!["other","b","a","live"]);
    assert_eq!(sentinel_vulnerability_scan_ids_in(&reader,Some(1)).unwrap(),vec!["b","a","live"]);
    assert_eq!(sentinel_vulnerability_scan_ids_in(&reader,Some(2)).unwrap(),vec!["other"]);
    assert!(sentinel_vulnerability_scan_ids_in(&reader,Some(3)).unwrap().is_empty());
    assert_eq!(sentinel_findings_in(&reader,"live",Some("vulnerability")).unwrap().len(),2);
    reader.execute_batch("BEGIN").unwrap();
    assert_eq!(sentinel_findings_in(&reader,"live",None).unwrap().len(),2,"reader shares caller snapshot");
    assert!(!reader.is_autocommit());reader.execute_batch("ROLLBACK").unwrap();
    assert_eq!(bundle_native_snapshot(&connection),before);
    drop(reader);drop(state);drop(connection);fs::remove_dir_all(root).unwrap();
}
