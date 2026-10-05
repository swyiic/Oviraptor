fn closure_handoff_fixture() -> (PathBuf,PathBuf,rusqlite::Connection,ClosureHandoffInput) {
    let (root,path,connection) = administrative_closure_fixture();
    administrative_close_test(&path);
    let preview = closure_handoff_preview_in(&connection,"start-test").unwrap();
    let input = ClosureHandoffInput {
        request_id:Uuid::new_v4().to_string(),source_scan_id:"start-test".into(),
        closure_id:preview["closureId"].as_str().unwrap().into(),source_hash:preview["sourceHash"].as_str().unwrap().into(),
        task_name:"Independent follow-up".into(),urls:serde_json::from_value(preview["targetUrls"].clone()).unwrap(),
        scan_mode:"standard".into(),max_budget_usd:2.0,auth_session_ids:vec![],
        auth_session_scope_id:Uuid::new_v4().to_string(),skill_ids:vec![],instruction:"Fresh review only".into(),
        closure:"breadth".into(),operator_confirmed:true,
    };
    (root,path,connection,input)
}

#[test]
fn closure_handoff_is_independent_draft_and_recovers_after_restart_without_replay() {
    let (root,path,connection,input) = closure_handoff_fixture();
    let before = administrative_closure_evidence(&connection,"start-test").unwrap();
    let receipt = create_closure_handoff_in(&connection,&input).unwrap();
    let scan = receipt["scanId"].as_str().unwrap();
    assert_ne!(scan,"start-test");
    assert_eq!(receipt["scan"]["status"],"draft");
    assert_eq!(receipt["scan"]["attemptCount"],0);
    assert_eq!(receipt["executionGranted"],false);
    assert_eq!(receipt["sourceExecutionSettled"],false);
    assert_eq!(administrative_closure_evidence_except_event(&connection,"start-test",Some(&input.request_id)).unwrap(),before);
    for table in ["agent_runs","sentinel_scan_attempts","native_branch_dispatches","agent_coordinator_leases"] {
        assert_eq!(connection.query_row(&format!("SELECT COUNT(*) FROM {table} WHERE scan_id=?1"),[scan],|r|r.get::<_,i64>(0)).unwrap(),0);
    }
    let snapshot = deletion_snapshot(&connection);
    assert_eq!(create_closure_handoff_in(&connection,&input).unwrap(),receipt);
    assert_eq!(deletion_snapshot(&connection),snapshot);
    assert_eq!(db::initialize(&root).unwrap(),path);
    let reopened=db::open(&path).unwrap();
    assert_eq!(closure_handoff_preview_in(&reopened,"start-test").unwrap()["savedHandoff"],receipt);
    assert_eq!(native_scan_status(&reopened,"start-test").unwrap()["closureHandoff"]["successor"],receipt);
    assert_eq!(native_scan_status(&reopened,scan).unwrap()["closureHandoff"]["source"],receipt);
    let status=native_scan_status(&reopened,"start-test").unwrap();
    let event=status["timeline"].as_array().unwrap().iter().find(|e|e["eventType"]=="closure_handoff").unwrap();
    let sequence=event["sequence"].as_i64().unwrap();assert!(sequence>0);
    assert!(!native_scan_status_after(&reopened,"start-test",Some(sequence)).unwrap()["timeline"].as_array().unwrap().iter().any(|e|e["eventType"]=="closure_handoff"));
    assert!(native_scan_status_after(&reopened,"start-test",Some(sequence-1)).unwrap()["timeline"].as_array().unwrap().iter().any(|e|e["eventType"]=="closure_handoff"));
    connection.execute("UPDATE sentinel_scans SET status='paused' WHERE id=?1",[scan]).unwrap();
    assert_eq!(create_closure_handoff_in(&reopened,&input).unwrap()["scan"]["status"],"paused");
    assert!(delete_sentinel_scan_inner(&path,scan).unwrap_err().contains("交接"));
    assert!(connection.execute("INSERT OR REPLACE INTO native_web_closure_handoffs SELECT * FROM native_web_closure_handoffs",[]).is_err());
    assert!(connection.execute("DELETE FROM native_web_closure_handoffs",[]).is_err());
    drop(reopened);drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn closure_handoff_requires_fresh_explicit_configuration_and_exact_source() {
    let (root,_path,connection,input) = closure_handoff_fixture();
    let before=deletion_snapshot(&connection);
    let mut variants=Vec::new();
    let mut wrong=input.clone();wrong.operator_confirmed=false;variants.push(wrong);
    let mut wrong=input.clone();wrong.closure_id=Uuid::new_v4().to_string();variants.push(wrong);
    let mut wrong=input.clone();wrong.source_hash="stale".into();variants.push(wrong);
    let mut wrong=input.clone();wrong.urls=vec!["https://other.example.test/".into()];variants.push(wrong);
    let mut wrong=input.clone();wrong.max_budget_usd=0.0;variants.push(wrong);
    let mut wrong=input.clone();wrong.max_budget_usd=f64::INFINITY;variants.push(wrong);
    let mut wrong=input.clone();wrong.auth_session_ids=vec!["old-task-identity".into()];variants.push(wrong);
    let mut wrong=input.clone();wrong.auth_session_scope_id.clear();variants.push(wrong);
    let mut wrong=input.clone();wrong.scan_mode="auto-host".into();variants.push(wrong);
    for wrong in variants {
        assert!(create_closure_handoff_in(&connection,&wrong).is_err(),"{wrong:?}");
        assert_eq!(deletion_snapshot(&connection),before);
    }
    connection.execute("UPDATE sentinel_targets SET url=url||'/changed' WHERE scan_id='start-test'",[]).unwrap();
    let changed=deletion_snapshot(&connection);
    assert!(create_closure_handoff_in(&connection,&input).unwrap_err().contains("source_changed"));
    assert_eq!(deletion_snapshot(&connection),changed);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn closure_handoff_rolls_back_partial_writes_and_preserves_source_obligations() {
    for trigger in [
        "CREATE TRIGGER reject_handoff BEFORE INSERT ON native_web_closure_handoffs BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER reject_handoff BEFORE INSERT ON native_web_closure_handoffs BEGIN SELECT RAISE(ABORT,'disk failure'); END;",
        "CREATE TRIGGER corrupt_source AFTER INSERT ON native_web_closure_handoffs BEGIN UPDATE agent_budget_ledger SET reserved_requests=0; END;",
        "CREATE TRIGGER corrupt_draft AFTER INSERT ON native_web_closure_handoffs BEGIN UPDATE sentinel_scans SET status='scanning' WHERE id=NEW.scan_id; END;",
        "CREATE TRIGGER corrupt_target AFTER INSERT ON sentinel_targets WHEN NEW.scan_id<>'start-test' BEGIN UPDATE sentinel_targets SET url='https://wrong.example.test/' WHERE id=NEW.id; END;",
        "CREATE TRIGGER lose_event BEFORE INSERT ON agent_collaboration_events WHEN NEW.event_type='closure_handoff' BEGIN SELECT RAISE(IGNORE); END;",
    ] {
        let (root,_path,connection,input)=closure_handoff_fixture();
        connection.execute_batch(trigger).unwrap();
        let before=deletion_snapshot(&connection);
        assert!(create_closure_handoff_in(&connection,&input).is_err(),"{trigger}");
        assert_eq!(deletion_snapshot(&connection),before,"{trigger}");
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn closure_handoff_concurrent_clients_create_one_canonical_draft() {
    let (root,path,connection,input)=closure_handoff_fixture();
    let barrier=Arc::new(std::sync::Barrier::new(2));
    let handles=(0..2).map(|_|{
        let path=path.clone();let barrier=barrier.clone();let mut input=input.clone();input.request_id=Uuid::new_v4().to_string();
        std::thread::spawn(move ||{let db=db::open(&path).unwrap();barrier.wait();create_closure_handoff_in(&db,&input)})
    }).collect::<Vec<_>>();
    let results=handles.into_iter().map(|h|h.join().unwrap()).collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|r|r.is_ok()).count(),1);
    assert!(results.iter().any(|r|matches!(r,Err(e) if e.contains("exists_refresh"))));
    assert_eq!(connection.query_row("SELECT COUNT(*) FROM native_web_closure_handoffs",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(connection.query_row("SELECT COUNT(*) FROM sentinel_scans",[],|r|r.get::<_,i64>(0)).unwrap(),2);
    assert!(closure_handoff_preview_in(&connection,"start-test").unwrap()["savedHandoff"].is_object());
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn closure_handoff_rejects_changed_request_and_preserves_exact_trailing_slash() {
    let (root,_path,connection,mut input)=closure_handoff_fixture();
    connection.execute("UPDATE sentinel_targets SET url=rtrim(url,'/')||'/' WHERE scan_id='start-test'",[]).unwrap();
    let preview=closure_handoff_preview_in(&connection,"start-test").unwrap();
    input.source_hash=preview["sourceHash"].as_str().unwrap().into();
    input.urls=serde_json::from_value(preview["targetUrls"].clone()).unwrap();
    let receipt=create_closure_handoff_in(&connection,&input).unwrap();
    let target:String=connection.query_row("SELECT url FROM sentinel_targets WHERE scan_id=?1",[receipt["scanId"].as_str().unwrap()],|r|r.get(0)).unwrap();
    assert_eq!(target,input.urls[0]);
    let before=deletion_snapshot(&connection);
    input.max_budget_usd+=1.0;
    assert!(create_closure_handoff_in(&connection,&input).unwrap_err().contains("exists_refresh"));
    assert_eq!(deletion_snapshot(&connection),before);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn closure_handoff_retained_receipt_detects_corrupt_links_and_fails_status_read() {
    for mutation in ["UPDATE native_web_closure_handoffs SET input_hash=printf('%064d',0)",
        "UPDATE native_web_closure_handoffs SET preview_json=json_set(preview_json,'$.snapshotHash','wrong')",
        "UPDATE native_web_closure_handoffs SET request_id='not-a-uuid'"] {
        let (root,_path,connection,input)=closure_handoff_fixture();
        let saved=create_closure_handoff_in(&connection,&input).unwrap();
        connection.execute_batch("DROP TRIGGER closure_handoff_immutable;").unwrap();
        connection.execute(mutation,[]).unwrap();
        let before=deletion_snapshot(&connection);
        assert!(create_closure_handoff_in(&connection,&input).is_err());
        assert!(closure_handoff_preview_in(&connection,"start-test").is_err());
        assert!(native_scan_status(&connection,saved["scanId"].as_str().unwrap()).is_err());
        assert_eq!(deletion_snapshot(&connection),before);
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn closure_handoff_does_not_bypass_archived_workspace_or_fused_targets() {
    for mutation in ["UPDATE projects SET status='archived' WHERE id=1",
        "INSERT INTO sentinel_fuse_zone(project_id,url,normalized_url) SELECT project_id,url,lower(rtrim(trim(url),'/')) FROM sentinel_targets WHERE scan_id='start-test'"] {
        let (root,_path,connection,input)=closure_handoff_fixture();
        connection.execute(mutation,[]).unwrap();
        let before=deletion_snapshot(&connection);
        assert!(create_closure_handoff_in(&connection,&input).is_err());
        assert_eq!(deletion_snapshot(&connection),before);
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn closure_handoff_rejects_missing_duplicate_and_conflicting_events_even_behind_cursor() {
    for mutation in ["DELETE FROM agent_collaboration_events WHERE event_type='closure_handoff'",
        "UPDATE agent_collaboration_events SET payload_json=json_set(payload_json,'$.executionGranted',1) WHERE event_type='closure_handoff'",
        "UPDATE agent_collaboration_events SET scan_id='foreign' WHERE event_type='closure_handoff'",
        "INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json) SELECT scan_id,attempt_number,entity_type,entity_id,event_type,payload_json FROM agent_collaboration_events WHERE event_type='closure_handoff'"] {
        let (root,_path,connection,input)=closure_handoff_fixture();
        create_closure_handoff_in(&connection,&input).unwrap();
        connection.execute(mutation,[]).unwrap();
        let before=deletion_snapshot(&connection);
        assert!(create_closure_handoff_in(&connection,&input).is_err());
        assert!(native_scan_status_after(&connection,"start-test",Some(i64::MAX)).is_err());
        assert_eq!(deletion_snapshot(&connection),before);
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}
