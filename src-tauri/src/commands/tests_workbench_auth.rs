fn workbench_auth_fixture() -> (PathBuf,rusqlite::Connection,JsonValue) {
    let (root,connection,record,plan,scope)=workbench_start_fixture();
    publish_workbench_fixture(&root,&connection,&record,&plan,&scope,false).unwrap();
    let previous=serde_json::from_slice(&fs::read(root.join("attempt-0001/auth-session.json")).unwrap()).unwrap();
    (root,connection,previous)
}

#[test]
fn workbench_auth_single_retry_restores_policy_handle_and_refreshes_live_material() {
    let (root,connection,previous)=workbench_auth_fixture();
    let old=fs::read(root.join("attempt-0001/auth-session.json")).unwrap();
    connection.execute("UPDATE browser_auth_sessions SET name='Current account',session_json=json_set(session_json,'$.cookies[0].value','fresh-fixture-token') WHERE id='wb-identity'",[]).unwrap();
    let (ids,document)=restore_workbench_browser_auth(&connection,&previous,1,"wb-start").unwrap().unwrap();
    assert_eq!(ids,vec!["wb-identity"]);
    assert_eq!(document["cookies"][0]["value"],"fresh-fixture-token");
    assert_eq!(document["name"],"Current account");
    assert_ne!(document,previous);
    let policy=build_web_investigation_policy(Some("deep"),Some(2.0),ids,&[],"","workbench",None).unwrap();
    assert_eq!(crate::auth_session::auth_session_ids_from_policy(&policy),vec!["wb-identity"]);
    assert_eq!(old,fs::read(root.join("attempt-0001/auth-session.json")).unwrap(),"historical credentials are not overwritten");
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn workbench_auth_retry_rejects_invalid_expired_missing_and_foreign_identity() {
    for change in [
        "UPDATE browser_auth_sessions SET status='invalid'",
        "UPDATE browser_auth_sessions SET status='capturing'",
        "UPDATE browser_auth_sessions SET status='needs_check'",
        "UPDATE browser_auth_sessions SET expires_at='2000-01-01T00:00:00Z'",
        "UPDATE browser_auth_sessions SET expires_at='invalid-date'",
        "UPDATE browser_auth_sessions SET owner_scan_id='another-task'",
        "UPDATE browser_auth_sessions SET owner_scan_id=''",
        "UPDATE browser_auth_sessions SET draft_scope_id='another-draft'",
        "INSERT INTO projects(id,name) VALUES(2,'Other'); UPDATE browser_auth_sessions SET project_id=2",
        "DELETE FROM browser_auth_sessions",
    ] {
        let (root,connection,previous)=workbench_auth_fixture();
        connection.execute_batch(change).unwrap();
        let before=web_start_snapshot(&connection);
        assert!(restore_workbench_browser_auth(&connection,&previous,1,"wb-start").is_err(),"{change}");
        assert_eq!(before,web_start_snapshot(&connection));
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn workbench_auth_explicit_retry_uses_the_same_owner_and_document_validation() {
    let (root,connection,_)=workbench_auth_fixture();
    let ids=vec!["wb-identity".into()];
    assert!(workbench_current_browser_auth(&connection,&ids,1,Some("wb-start")).is_ok());
    assert!(workbench_current_browser_auth(&connection,&ids,1,Some("other-task")).is_err());
    assert!(workbench_current_browser_auth(&connection,&ids,2,Some("wb-start")).is_err());
    for replacement in [json!(null),json!({}),json!([]),json!({"schemaVersion":1,"id":"wb-identity"})] {
        connection.execute("UPDATE browser_auth_sessions SET session_json=?1",[replacement.to_string()]).unwrap();
        assert!(workbench_current_browser_auth(&connection,&ids,1,Some("wb-start")).is_err());
    }
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn workbench_auth_history_cannot_downgrade_malformed_browser_identity_to_raw_credentials() {
    let (root,connection,previous)=workbench_auth_fixture();
    for document in [
        json!(null),json!([]),json!({"schemaVersion":1}),
        json!({"schemaVersion":1,"id":""}),json!({"schemaVersion":1,"id":" wb-identity"}),
        json!({"schemaVersion":1,"id":42}),
        json!({"schemaVersion":1,"id":"wb-identity","sessions":[]}),
        json!({"schemaVersion":2,"kind":"identity-matrix","sessions":[]}),
        json!({"schemaVersion":2,"kind":"identity-matrix","sessions":[previous.clone(),previous.clone()]}),
        json!({"schemaVersion":2,"kind":"identity-matrix","sessions":[{"id":"missing"}]}),
        json!({"schemaVersion":2,"kind":"identity-matrix","sessions":{}}),
        json!({"schemaVersion":2,"kind":"identity-matrix","sessions":[{}]}),
        json!({"schemaVersion":2,"sessions":[previous.clone()]}),
        json!({"schemaVersion":3,"id":"wb-identity","type":"bearer","value":"do-not-replay"}),
        json!({"sessions":[],"type":"cookie","value":"do-not-replay"}),
    ] {
        assert!(restore_workbench_browser_auth(&connection,&document,1,"wb-start").is_err(),"{document}");
    }
    let raw=json!({"type":"bearer","value":"legacy-fixture","profile":"Legacy"});
    assert!(restore_workbench_browser_auth(&connection,&raw,1,"wb-start").unwrap().is_none(),"raw legacy auth remains with its existing type/value validation");
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn workbench_auth_matrix_refreshes_all_handles_and_requires_distinct_current_material() {
    let (root,connection,previous)=workbench_auth_fixture();
    let mut second=previous.clone();
    second["id"]=json!("wb-second");
    second["cookies"][0]["value"]=json!("fixture-account-b");
    connection.execute("INSERT INTO browser_auth_sessions(id,project_id,owner_scan_id,name,entry_url,status,expires_at,session_json) SELECT 'wb-second',project_id,owner_scan_id,'Second',entry_url,status,expires_at,?1 FROM browser_auth_sessions WHERE id='wb-identity'",[second.to_string()]).unwrap();
    let history=json!({"schemaVersion":2,"kind":"identity-matrix","sessions":[second,previous]});
    let (ids,current)=restore_workbench_browser_auth(&connection,&history,1,"wb-start").unwrap().unwrap();
    assert_eq!(ids,vec!["wb-identity","wb-second"]);
    assert_eq!(current["sessions"][0]["id"],"wb-identity");
    assert_eq!(current["sessions"][1]["id"],"wb-second");
    assert_eq!(current["comparisonPolicy"],"same-target-same-action-plan");
    connection.execute("UPDATE browser_auth_sessions SET session_json=json_set(session_json,'$.cookies[0].value','fixture-account-a') WHERE id='wb-second'",[]).unwrap();
    assert!(restore_workbench_browser_auth(&connection,&history,1,"wb-start").is_err());
    connection.execute("UPDATE browser_auth_sessions SET session_json=json_set(session_json,'$.cookies[0].value','fixture-account-b','$.scopeHosts',json('[\"other.example.test\"]')) WHERE id='wb-second'",[]).unwrap();
    assert!(restore_workbench_browser_auth(&connection,&history,1,"wb-start").is_err());
    let too_many=json!({"schemaVersion":2,"kind":"identity-matrix","sessions":(0..6).map(|i|json!({"id":format!("id-{i}")})).collect::<Vec<_>>()});
    assert_eq!(restore_workbench_browser_auth(&connection,&too_many,1,"wb-start").unwrap_err(),"workbench_auth_identity_set_invalid");
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn workbench_auth_publication_rejects_changed_live_auth_and_rolls_back_every_write() {
    for change in [
        "UPDATE browser_auth_sessions SET status='invalid';",
        "UPDATE browser_auth_sessions SET expires_at='2000-01-01T00:00:00Z';",
        "UPDATE browser_auth_sessions SET session_json=json_set(session_json,'$.cookies[0].value','changed-fixture-secret');",
        "UPDATE browser_auth_sessions SET session_json=json_set(session_json,'$.id','another-id');",
        "UPDATE browser_auth_sessions SET session_json=json_set(session_json,'$.projectId',2);",
        "UPDATE browser_auth_sessions SET session_json=json_set(session_json,'$.scopeHosts',json('[]'));",
        "UPDATE browser_auth_sessions SET session_json=json_set(session_json,'$.expiresAt','2000-01-01T00:00:00Z');",
    ] {
        let (root,connection,record,plan,scope)=workbench_start_fixture();
        connection.execute_batch(&format!("CREATE TRIGGER wb_auth_changed AFTER INSERT ON native_branch_dispatches BEGIN {change} END;")).unwrap();
        let before=web_start_snapshot(&connection);
        let old_auth: String=connection.query_row("SELECT session_json FROM browser_auth_sessions",[],|row|row.get(0)).unwrap();
        let error=publish_workbench_fixture(&root,&connection,&record,&plan,&scope,false).err().unwrap();
        assert!(!error.contains("changed-fixture-secret"));
        assert_eq!(before,web_start_snapshot(&connection),"{change}");
        assert_eq!(old_auth,connection.query_row("SELECT session_json FROM browser_auth_sessions",[],|row|row.get::<_,String>(0)).unwrap());
        assert_eq!(connection.query_row("SELECT count(*) FROM native_branch_dispatches",[],|row|row.get::<_,i64>(0)).unwrap(),0);
        connection.execute_batch("DROP TRIGGER wb_auth_changed").unwrap();
        publish_workbench_fixture(&root,&connection,&record,&plan,&scope,false).unwrap();
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn workbench_auth_publication_requires_matching_file_policy_and_identity_list() {
    for case in ["missing","invalid-json","stale-file","empty-ids","policy-mismatch","singular-policy-mismatch","not-authenticated","raw-type"] {
        let (root,connection,mut record,plan,scope)=workbench_start_fixture();
        let path=root.join("attempt-0001/auth-session.json");
        match case {
            "missing" => fs::remove_file(&path).unwrap(),
            "invalid-json" => fs::write(&path,"invalid-json").unwrap(),
            "stale-file" => fs::write(&path,"{}").unwrap(),
            "empty-ids" => record.auth_session_ids.clear(),
            "policy-mismatch" => record.policy["authSessionIds"]=json!([]),
            "singular-policy-mismatch" => record.policy["authSessionId"]=json!("other"),
            "not-authenticated" => record.authenticated=false,
            "raw-type" => record.auth_type="bearer".into(),
            _ => unreachable!(),
        }
        let before=web_start_snapshot(&connection);
        assert!(publish_workbench_fixture(&root,&connection,&record,&plan,&scope,false).is_err(),"{case}");
        assert_eq!(before,web_start_snapshot(&connection),"{case}");
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn workbench_auth_retry_publication_preserves_old_attempt_when_auth_changes() {
    let (root,connection,mut record,mut plan,scope)=workbench_start_fixture();
    publish_workbench_fixture(&root,&connection,&record,&plan,&scope,false).unwrap();
    connection.execute("UPDATE sentinel_scans SET status='failed' WHERE id='wb-start'",[]).unwrap();
    let previous=fs::read(root.join("attempt-0001/auth-session.json")).unwrap();
    record.attempt=2;
    plan.attempt_number=2;
    fs::create_dir(root.join("attempt-0002")).unwrap();
    fs::write(root.join("attempt-0002/auth-session.json"),&previous).unwrap();
    connection.execute("UPDATE browser_auth_sessions SET session_json=json_set(session_json,'$.cookies[0].value','renewed-fixture-token')",[]).unwrap();
    let before=web_start_snapshot(&connection);
    assert_eq!(publish_workbench_fixture(&root,&connection,&record,&plan,&scope,true).err().unwrap(),"workbench_auth_inputs_changed");
    assert_eq!(before,web_start_snapshot(&connection));
    assert_eq!(previous,fs::read(root.join("attempt-0001/auth-session.json")).unwrap());
    let (_,fresh)=restore_workbench_browser_auth(&connection,&serde_json::from_slice(&previous).unwrap(),1,"wb-start").unwrap().unwrap();
    crate::auth_session::write_session_document(&root.join("attempt-0002/auth-session.json"),&fresh).unwrap();
    publish_workbench_fixture(&root,&connection,&record,&plan,&scope,true).unwrap();
    assert_eq!(connection.query_row("SELECT attempt_count FROM sentinel_scans WHERE id='wb-start'",[],|row|row.get::<_,i64>(0)).unwrap(),2);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}
