fn administrative_closure_fixture() -> (PathBuf,PathBuf,rusqlite::Connection) {
    let (root,path,connection) = pause_fixture();
    connection.execute_batch("UPDATE sentinel_scans SET status='paused';
        INSERT INTO native_scan_branches(scan_id,attempt_number,branch) VALUES('start-test',1,'web');
        INSERT INTO native_branch_dispatches(scan_id,attempt_number,branch,claim_id,claimed_at)
        VALUES('start-test',1,'web','old-claim','2026-01-01');
        INSERT INTO agent_runs(id,scan_id,attempt_number,status,terminal_code)
        VALUES('closure-root','start-test',1,'paused','request_reconciliation_required');
        INSERT INTO agent_budget_ledger(root_run_id,total_tokens,reserved_tokens,total_requests,reserved_requests)
        VALUES('closure-root',100,30,10,2);
        INSERT INTO agent_assignments(id,coordinator_run_id,dedup_key,state,reserved_tokens,reserved_requests)
        VALUES('closure-assignment','closure-root','original','running',30,2);
        INSERT INTO agent_lane_leases(scan_id,attempt_number,target_key,lane,assignment_id)
        VALUES('start-test',1,'target','target_touching','closure-assignment');
        INSERT INTO tool_invocations(run_id,invocation_id,tool_name,status)
        VALUES('closure-root','unknown-request','replay_http','interrupted');").unwrap();
    (root,path,connection)
}

fn administrative_close_test(path: &Path) -> JsonValue {
    let preview = preview_administrative_closure(path,"start-test",1).unwrap();
    close_administrative_web_task(path,"start-test",1,&Uuid::new_v4().to_string(),preview["snapshotHash"].as_str().unwrap(),true).unwrap()
}

#[test]
fn administrative_closure_retains_unknown_obligations_and_seals_old_task() {
    let (root,path,mut connection) = administrative_closure_fixture();
    let before = administrative_closure_evidence(&connection,"start-test").unwrap();
    let receipt = administrative_close_test(&path);
    assert_eq!(receipt["executionState"],"administratively_closed_unsettled");
    assert_eq!(receipt["executionSettled"],false);
    assert_eq!(administrative_closure_evidence(&connection,"start-test").unwrap(),before);
    assert_eq!(pause_status(&connection),"cancelled");
    assert!(sentinel_scan_by_id(&connection,"start-test").unwrap().administrative_closure_recorded);
    assert!(crate::agent_runtime::multi_agent::lease::require_active_attempt(&connection,"start-test",1).is_err());
    assert!(crate::agent_runtime::multi_agent::lease::acquire_coordinator_lease(&connection,"start-test",1,"target","closure-root",60).is_err());
    assert!(delete_sentinel_scan_inner(&path,"start-test").is_err());
    for mode in [WebStartMode::Retry,WebStartMode::Resume,WebStartMode::Confirm] {
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
        assert!(matches!(prepare_web_startup_in(&tx,&root,"start-test",mode),Err(e) if e=="administrative_closure_requires_independent_task"));
    }
    for sql in ["UPDATE sentinel_scans SET status='scanning' WHERE id='start-test'",
        "UPDATE sentinel_scans SET attempt_count=2 WHERE id='start-test'",
        "INSERT INTO sentinel_scan_attempts(scan_id,attempt_number) VALUES('start-test',2)",
        "UPDATE sentinel_scan_attempts SET status='completed' WHERE scan_id='start-test'",
        "DELETE FROM sentinel_scan_attempts WHERE scan_id='start-test'",
        "INSERT INTO agent_runs(id,scan_id) VALUES('resurrect','start-test')",
        "INSERT OR REPLACE INTO native_web_administrative_closures SELECT * FROM native_web_administrative_closures",
        "DELETE FROM native_web_administrative_closures", "DELETE FROM sentinel_scans WHERE id='start-test'"] {
        assert!(connection.execute(sql,[]).is_err(),"{sql}");
    }
    let after = deletion_snapshot(&connection);
    assert_eq!(close_administrative_web_task(&path,"start-test",1,receipt["closureId"].as_str().unwrap(),receipt["snapshotHash"].as_str().unwrap(),true).unwrap(),receipt);
    assert_eq!(deletion_snapshot(&connection),after);
    assert_eq!(preview_administrative_closure(&path,"start-test",1).unwrap()["receipt"],receipt);
    assert!(close_administrative_web_task(&path,"start-test",1,&Uuid::new_v4().to_string(),receipt["snapshotHash"].as_str().unwrap(),true).is_err());
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn administrative_closure_rejects_duplicate_cross_scope_and_contradictory_events() {
    for sql in [
        "UPDATE agent_collaboration_events SET payload_json='{\"executionSettled\":0,\"automaticReplayAllowed\":false}' WHERE event_type='administrative_closure'",
        "UPDATE agent_collaboration_events SET payload_json='{\"executionSettled\":false,\"automaticReplayAllowed\":false,\"executionUnlocked\":true}' WHERE event_type='administrative_closure'",
        "UPDATE agent_collaboration_events SET scan_id='other' WHERE event_type='administrative_closure'",
        "INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json) SELECT scan_id,attempt_number,entity_type,entity_id,event_type,payload_json FROM agent_collaboration_events WHERE event_type='administrative_closure'",
    ] {
        let (root,path,connection) = administrative_closure_fixture();
        let receipt = administrative_close_test(&path);
        connection.execute(sql,[]).unwrap();
        let before = deletion_snapshot(&connection);
        assert!(verified_administrative_closure(&connection,"start-test").is_err(),"{sql}");
        assert!(preview_administrative_closure(&path,"start-test",1).is_err(),"{sql}");
        assert!(close_administrative_web_task(&path,"start-test",1,receipt["closureId"].as_str().unwrap(),receipt["snapshotHash"].as_str().unwrap(),true).is_err(),"{sql}");
        assert_eq!(deletion_snapshot(&connection),before);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn administrative_closure_requires_confirmation_fresh_snapshot_and_quiescence() {
    let (root,path,connection) = administrative_closure_fixture();
    let preview = preview_administrative_closure(&path,"start-test",1).unwrap();
    let hash = preview["snapshotHash"].as_str().unwrap();
    let operation = Uuid::new_v4().to_string();
    let before = deletion_snapshot(&connection);
    assert!(close_administrative_web_task(&path,"start-test",1,&operation,hash,false).is_err());
    assert!(close_administrative_web_task(&path,"start-test",2,&operation,hash,true).is_err());
    for (kind,target) in [("branch","web"),("branch","source"),("frontend_producer","web"),
        ("target","https://start.example.test/one"),("frontend_recon","https://start.example.test/two")] {
        let owner = claim_native_invocation(&path,"start-test",1,kind,target).unwrap();
        assert!(preview_administrative_closure(&path,"start-test",1).is_err());
        assert!(close_administrative_web_task(&path,"start-test",1,&operation,hash,true).is_err());
        drop(owner);
    }
    assert_eq!(deletion_snapshot(&connection),before);
    connection.execute("UPDATE agent_budget_ledger SET reserved_requests=3 WHERE root_run_id='closure-root'",[]).unwrap();
    let changed = deletion_snapshot(&connection);
    assert_eq!(close_administrative_web_task(&path,"start-test",1,&operation,hash,true).unwrap_err(),"administrative_closure_preview_stale");
    assert_eq!(deletion_snapshot(&connection),changed);
    administrative_close_test(&path);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn administrative_closure_rolls_back_partial_or_corrupt_writes() {
    for sql in [
        "CREATE TRIGGER closure_fault BEFORE INSERT ON native_web_administrative_closures BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER closure_fault BEFORE UPDATE ON sentinel_scans BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER closure_fault AFTER INSERT ON native_web_administrative_closures BEGIN UPDATE agent_budget_ledger SET reserved_requests=0; END;",
        "CREATE TRIGGER closure_fault AFTER INSERT ON native_web_administrative_closures BEGIN DELETE FROM agent_lane_leases; END;",
        "CREATE TRIGGER closure_fault BEFORE INSERT ON agent_collaboration_events WHEN NEW.event_type='administrative_closure' BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER closure_fault AFTER INSERT ON agent_collaboration_events WHEN NEW.event_type='administrative_closure' BEGIN UPDATE agent_collaboration_events SET payload_json='{\"executionSettled\":true}' WHERE sequence=NEW.sequence; END;",
    ] {
        let (root,path,connection) = administrative_closure_fixture();
        connection.execute_batch(sql).unwrap();
        let preview = preview_administrative_closure(&path,"start-test",1).unwrap();
        let before = deletion_snapshot(&connection);
        assert!(close_administrative_web_task(&path,"start-test",1,&Uuid::new_v4().to_string(),preview["snapshotHash"].as_str().unwrap(),true).is_err(),"{sql}");
        assert_eq!(deletion_snapshot(&connection),before,"{sql}");
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn administrative_closure_event_is_real_incremental_and_rechecked_after_restart() {
    let (root,path,connection) = administrative_closure_fixture();
    let receipt = administrative_close_test(&path);
    let status = native_scan_status_after(&connection,"start-test",None).unwrap();
    assert_eq!(status["administrativeClosure"],receipt);
    assert_eq!(status["manualAdministrativeClosureAvailable"],false);
    let event = status["timeline"].as_array().unwrap().iter().find(|e|e["eventType"]=="administrative_closure").unwrap();
    let sequence = event["sequence"].as_i64().unwrap();
    assert!(sequence>0);
    let delta = native_scan_status_after(&connection,"start-test",Some(sequence)).unwrap();
    assert!(!delta["timeline"].as_array().unwrap().iter().any(|e|e["eventType"]=="administrative_closure"));
    assert!(native_scan_status_after(&connection,"start-test",Some(sequence-1)).unwrap()["timeline"].as_array().unwrap().iter().any(|e|e["eventType"]=="administrative_closure"));
    drop(connection);
    assert_eq!(db::initialize(&root).unwrap(),path);
    let connection = db::open(&path).unwrap();
    assert_eq!(verified_administrative_closure(&connection,"start-test").unwrap(),Some(receipt.clone()));
    connection.execute("DELETE FROM agent_collaboration_events WHERE event_type='administrative_closure'",[]).unwrap();
    let before = deletion_snapshot(&connection);
    for cursor in [None,Some(0),Some(sequence),Some(sequence+100)] {
        assert!(native_scan_status_after(&connection,"start-test",cursor).is_err());
    }
    assert!(close_administrative_web_task(&path,"start-test",1,receipt["closureId"].as_str().unwrap(),receipt["snapshotHash"].as_str().unwrap(),true).is_err());
    assert_eq!(deletion_snapshot(&connection),before);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn administrative_closure_never_ignores_unconfirmed_process_or_container_cleanup() {
    for sql in ["INSERT INTO sentinel_processes(scan_id,process_id) VALUES('start-test',2147483647)",
        "INSERT INTO analyzer_container_receipts(receipt_id,invocation_key,scan_id,attempt_number,purpose,container_name,owner_token,create_args_json,cleanup_status)
         VALUES('closure-container','closure-invocation','start-test',1,'test','test','test','[]','unconfirmed')"] {
        let (root,path,connection) = administrative_closure_fixture();
        let preview = preview_administrative_closure(&path,"start-test",1).unwrap();
        connection.execute(sql,[]).unwrap();
        let before = deletion_snapshot(&connection);
        assert!(preview_administrative_closure(&path,"start-test",1).is_err());
        assert!(close_administrative_web_task(&path,"start-test",1,&Uuid::new_v4().to_string(),preview["snapshotHash"].as_str().unwrap(),true).is_err());
        assert_eq!(deletion_snapshot(&connection),before);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn administrative_closure_other_scan_is_not_sealed_and_new_ownership_rows_invalidate_preview() {
    let (root,path,connection) = administrative_closure_fixture();
    connection.execute_batch("INSERT INTO sentinel_scans(id,project_id,status) VALUES('separate-task',1,'draft');
        CREATE TABLE future_native_record(run_id TEXT, payload TEXT);
        INSERT INTO future_native_record VALUES('closure-root','original');").unwrap();
    let preview = preview_administrative_closure(&path,"start-test",1).unwrap();
    connection.execute("UPDATE future_native_record SET payload='changed'",[]).unwrap();
    let before = deletion_snapshot(&connection);
    assert_eq!(close_administrative_web_task(&path,"start-test",1,&Uuid::new_v4().to_string(),preview["snapshotHash"].as_str().unwrap(),true).unwrap_err(),"administrative_closure_preview_stale");
    assert_eq!(deletion_snapshot(&connection),before);
    administrative_close_test(&path);
    connection.execute("UPDATE sentinel_scans SET status='scanning',attempt_count=1 WHERE id='separate-task'",[]).unwrap();
    connection.execute("INSERT INTO sentinel_scan_attempts(scan_id,attempt_number) VALUES('separate-task',1)",[]).unwrap();
    assert!(crate::agent_runtime::multi_agent::lease::require_active_attempt(&connection,"separate-task",1).is_ok());
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn administrative_closure_rejects_live_unclaimed_and_wrong_surface_tasks() {
    for sql in ["UPDATE sentinel_scans SET status='scanning'", "UPDATE sentinel_scans SET status='pausing'",
        "UPDATE sentinel_scans SET scan_type='source'", "UPDATE native_branch_dispatches SET claim_id='',claimed_at=''",
        "UPDATE sentinel_scans SET source_path='/tmp/source'"] {
        let (root,path,connection) = administrative_closure_fixture();
        connection.execute(sql,[]).unwrap();
        let before = deletion_snapshot(&connection);
        assert!(preview_administrative_closure(&path,"start-test",1).is_err(),"{sql}");
        assert_eq!(deletion_snapshot(&connection),before);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}
