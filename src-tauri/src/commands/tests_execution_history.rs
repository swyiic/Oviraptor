fn assert_production_source_execution_history(connection:&rusqlite::Connection,scan_id:&str) {
    let page=native_attempt_execution_history(connection,scan_id,1,None,100).unwrap();
    assert_eq!(page["schemaVersion"],2);
    let items=page["invocations"].as_array().unwrap();
    assert_eq!(items.len(),4);
    assert!(items.iter().all(|item| item["origin"]=="source_receipt" && item["status"]=="completed"
        && item["receiptIntegrity"]=="matched" && item["startedAt"]=="" && !item["finishedAt"].as_str().unwrap().is_empty()));
    assert_eq!(items.iter().filter(|item|item["resultKind"]=="control").count(),2);
    assert_eq!(items.iter().map(|item|item["id"].as_str().unwrap()).collect::<HashSet<_>>().len(),4);
    for mutation in [
        "DELETE FROM agent_events WHERE event_type='tool_invocation_completed'",
        "UPDATE agent_assignments SET child_run_id='other-child'",
        "DROP TRIGGER agent_source_tool_receipt_immutable; UPDATE agent_source_tool_receipts SET output_json='{\"secret\":\"private-output-value\"}'",
        "DROP TRIGGER agent_source_round_immutable; UPDATE agent_source_model_rounds SET response_json='not-json'",
    ] {
        connection.execute_batch("SAVEPOINT history_damage").unwrap();
        connection.execute_batch(mutation).unwrap();
        let damaged=native_attempt_execution_history(connection,scan_id,1,None,100).unwrap();
        assert_eq!(damaged["invocations"].as_array().unwrap().len(),4);
        assert!(damaged["invocations"].as_array().unwrap().iter().all(|item| item["status"]=="unverified"
            && item["finishedAt"]==""),"{mutation}: {damaged}");
        assert!(!damaged.to_string().contains("private-output-value"));
        connection.execute_batch("ROLLBACK TO history_damage; RELEASE history_damage").unwrap();
    }
    let mut cursor=None;
    let mut paged=Vec::new();
    for _ in 0..4 {
        let page=native_attempt_execution_history(connection,scan_id,1,cursor.as_deref(),1).unwrap();
        let item=&page["invocations"][0];
        assert!(item["id"].is_string());
        paged.push(item["id"].as_str().unwrap().to_string());
        cursor=page["olderCursor"].as_str().map(str::to_string);
        if page["hasOlder"]==false {break;}
    }
    paged.reverse();
    assert_eq!(paged,items.iter().map(|item|item["id"].as_str().unwrap().to_string()).collect::<Vec<_>>());
}

fn assert_history_rejects_unknown_source_tool(connection:&rusqlite::Connection,scan_id:&str) {
    // Deliberately corrupt all matching records together: a matching event and
    // hash must not make an unknown tool look supported or executed.
    connection.execute_batch("SAVEPOINT unknown_history_tool;
        DROP TRIGGER agent_source_tool_receipt_immutable;
        DROP TRIGGER agent_source_round_immutable;
        UPDATE agent_source_tool_receipts SET tool_name='unsupported.secret-tool';
        UPDATE agent_source_model_rounds SET response_json=json_set(response_json,'$.toolCalls[0].name','unsupported.secret-tool');
        UPDATE agent_events SET payload_json=json_set(payload_json,'$.call.name','unsupported.secret-tool')
          WHERE event_type='tool_invocation_completed';").unwrap();
    let events=connection.prepare("SELECT run_id,sequence,payload_json FROM agent_events WHERE event_type='tool_invocation_completed'")
        .unwrap().query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?)))
        .unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    for (run,sequence,payload) in events {
        let payload:JsonValue=serde_json::from_str(&payload).unwrap();
        let hash=crate::agent_runtime::store::stable_hash(&payload.to_string());
        connection.execute("UPDATE agent_source_tool_receipts SET receipt_hash=?1 WHERE event_sequence=?2
            AND assignment_id IN (SELECT assignment_id FROM agent_source_model_rounds WHERE child_run_id=?3)",
            params![hash,sequence,run]).unwrap();
    }
    let page=native_attempt_execution_history(connection,scan_id,1,None,50).unwrap();
    let items=page["invocations"].as_array().unwrap();
    assert_eq!(items.len(),1);
    assert_eq!(items[0]["status"],"unverified");
    assert_eq!(items[0]["receiptIntegrity"],"unverified");
    assert_eq!(items[0]["toolName"],"unknown_source_tool");
    assert_eq!(items[0]["finishedAt"],"");
    assert!(!page.to_string().contains("unsupported.secret-tool"));
    connection.execute_batch("ROLLBACK TO unknown_history_tool; RELEASE unknown_history_tool").unwrap();
}

#[test]
fn execution_history_mixes_real_source_receipts_and_web_without_fake_start_or_cursor_collisions() {
    use crate::agent_runtime::multi_agent::source_rounds::{self as rounds,Start};
    let (root,connection,context,lease,child,request)=source_round_fixture();
    let check=|db:&rusqlite::Connection|agent_native_source_tool_authority(db,&context,"assignment.finish").map(|_|()).map_err(str::to_string);
    let Start::Dispatch(call)=rounds::start_authorized(&connection,&lease,&child,1,&request,8_000,check).unwrap() else {panic!()};
    rounds::record_received(&connection,&call,&source_round_result("read-1","repo.read_slice",json!({"path":"app.py"}))).unwrap();
    let pending=native_attempt_execution_history(&connection,&context.scan_id,1,None,50).unwrap();
    let planned=&pending["invocations"][0];
    assert_eq!(planned["status"],"planned");
    assert_eq!(planned["receiptIntegrity"],"pending");
    assert_eq!(planned["startedAt"],"");
    assert_eq!(planned["finishedAt"],"");
    assert_eq!(planned["timeBasis"],"model_response_received");
    assert!(!pending.to_string().contains("app.py"));
    assert_history_rejects_unknown_source_tool(&connection,&context.scan_id);
    let (_,mut broker,_)=agent_source_broker(&context).unwrap();
    rounds::execute_tool(&connection,&call,0,check,|db,name,args|broker.call(db,name,args).map_err(|e|e.code.to_string())).unwrap();
    let complete=native_attempt_execution_history(&connection,&context.scan_id,1,None,50).unwrap();
    assert_eq!(complete["invocations"][0]["status"],"completed");
    assert_eq!(complete["invocations"][0]["id"],planned["id"]);
    assert_eq!(complete["invocations"][0]["recordedAt"],planned["recordedAt"]);
    assert!(!complete.to_string().contains("print('changed')"));
    assert!(!complete.to_string().contains("app.py"));
    assert_history_rejects_unknown_source_tool(&connection,&context.scan_id);
    // Deliberately same timestamp; disjoint identity namespaces avoid SQLite
    // row-number collisions and receipt completion never moves the source row.
    connection.execute("INSERT INTO tool_invocations(run_id,invocation_id,tool_name,status,started_at,input_summary_json,error_class)
        VALUES(?1,'web-call','http_request','confirmed',?2,'{\"secret\":\"private-input\"}','secret=private-input')",
        params![lease.root_run_id,planned["recordedAt"].as_str().unwrap()]).unwrap();
    let newest=native_attempt_execution_history(&connection,&context.scan_id,1,None,1).unwrap();
    assert_eq!(newest["hasOlder"],true);
    let older=native_attempt_execution_history(&connection,&context.scan_id,1,newest["olderCursor"].as_str(),1).unwrap();
    assert_eq!(older["hasOlder"],false);
    assert_ne!(newest["invocations"][0]["id"],older["invocations"][0]["id"]);
    let all=native_attempt_execution_history(&connection,&context.scan_id,1,None,1000).unwrap();
    assert_eq!(all["invocations"].as_array().unwrap().len(),2);
    assert!(!all.to_string().contains("private-input"));
    assert_eq!(all["invocations"][0],older["invocations"][0]);
    assert_eq!(all["invocations"][1],newest["invocations"][0]);
    let cursor:JsonValue=serde_json::from_str(newest["olderCursor"].as_str().unwrap()).unwrap();
    for (key,value) in [("schemaVersion",json!(1)),("scanId",json!("another-scan")),("attemptNumber",json!(2)),
        ("recordedAt",json!("")),("recordKey",json!("unsupported")),("extra",json!(true))] {
        let mut invalid=cursor.clone();invalid[key]=value;
        assert!(native_attempt_execution_history(&connection,&context.scan_id,1,Some(&invalid.to_string()),1).is_err(),"{invalid}");
    }
    for invalid in ["not-json","{}",&"x".repeat(4097)] {
        assert!(native_attempt_execution_history(&connection,&context.scan_id,1,Some(invalid),1).is_err());
    }
    connection.execute("INSERT INTO sentinel_scan_attempts(scan_id,attempt_number) VALUES(?1,2)",[&context.scan_id]).unwrap();
    assert_eq!(native_attempt_execution_history(&connection,&context.scan_id,2,None,50).unwrap()["invocations"],json!([]));
    assert!(native_attempt_execution_history(&connection,&context.scan_id,2,newest["olderCursor"].as_str(),50).is_err());
    assert!(native_attempt_execution_history(&connection,&context.scan_id,3,None,50).is_err());
    assert!(native_attempt_execution_history(&connection,&context.scan_id,0,None,50).is_err());
    connection.execute("INSERT INTO sentinel_deleted_scans(scan_id) VALUES(?1)",[&context.scan_id]).unwrap();
    assert!(native_attempt_execution_history(&connection,&context.scan_id,1,None,50).is_err());
    drop(connection);fs::remove_dir_all(root).unwrap();
}
