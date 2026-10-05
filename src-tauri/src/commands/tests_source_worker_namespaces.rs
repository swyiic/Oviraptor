#[test]
fn source_worker_namespace_same_assignment_round_has_independent_children() {
    use crate::agent_runtime::multi_agent::source_rounds as rounds;
    let (root, db, context, lease, child, request) = source_round_fixture();
    let check = |db: &rusqlite::Connection| {
        agent_native_source_tool_authority(db, &context, "assignment.finish")
            .map(|_| ())
            .map_err(str::to_string)
    };
    rounds::start_authorized(&db, &lease, &child, 1, &request, 8_000, check).unwrap();
    let second = super::agent_tests::namespace_copy_child(&db, &child.run_id);
    db.execute("INSERT INTO agent_source_model_rounds(assignment_id,round_number,child_run_id,root_run_id,role,lease_epoch,fencing_token,request_json,request_hash,reserved_tokens,state)
        SELECT assignment_id,round_number,?2,root_run_id,role,lease_epoch,fencing_token,request_json,request_hash,reserved_tokens,state
        FROM agent_source_model_rounds WHERE child_run_id=?1",params![child.run_id,second]).expect("round one belongs to its worker, not a previous worker of the logical assignment");
    assert_eq!(db.query_row("SELECT count(*) FROM agent_source_model_rounds WHERE assignment_id=?1 AND round_number=1",[&child.assignment_id],|r|r.get::<_,i64>(0)).unwrap(),2);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_worker_namespace_tool_receipt_binds_child_in_its_foreign_key() {
    use crate::agent_runtime::multi_agent::source_rounds as rounds;
    let (root, db, context, lease, child, request) = source_round_fixture();
    let check = |db: &rusqlite::Connection| {
        agent_native_source_tool_authority(db, &context, "assignment.finish")
            .map(|_| ())
            .map_err(str::to_string)
    };
    let rounds::Start::Dispatch(call) =
        rounds::start_authorized(&db, &lease, &child, 1, &request, 8_000, check).unwrap()
    else {
        panic!()
    };
    rounds::record_received(
        &db,
        &call,
        &source_round_result("original-tool", "repo.inventory", json!({})),
    )
    .unwrap();
    let owner:String=db.query_row("SELECT child_run_id FROM agent_source_tool_receipts WHERE assignment_id=?1 AND round_number=1 AND call_index=0",[&child.assignment_id],|r|r.get(0)).expect("durable tool receipt must bind its original worker");
    assert_eq!(owner, child.run_id);
    let other = super::agent_tests::namespace_copy_child(&db, &child.run_id);
    db.execute("INSERT INTO agent_source_model_rounds(assignment_id,round_number,child_run_id,root_run_id,role,lease_epoch,fencing_token,request_json,request_hash,reserved_tokens,state)
        SELECT assignment_id,round_number,?2,root_run_id,role,lease_epoch,fencing_token,request_json,request_hash,reserved_tokens,state
        FROM agent_source_model_rounds WHERE child_run_id=?1",params![child.run_id,other]).unwrap();
    let before = application_table_snapshot(&db);
    // The state transition and new parent FK are otherwise valid. Require the
    // identity guard itself to reject moving this original planned receipt.
    assert!(db.execute("UPDATE agent_source_tool_receipts SET state='completed',child_run_id=?1 WHERE child_run_id=?2",params![other,child.run_id])
        .unwrap_err().to_string().contains("source_tool_receipt_immutable"));
    assert_eq!(application_table_snapshot(&db), before);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_worker_namespace_original_callbacks_preserve_copies_but_unowned_dispatch_blocks_tools() {
    use crate::agent_runtime::multi_agent::source_rounds as rounds;
    for phase in ["received", "uncertain"] {
        let (root, db, context, lease, child, request) = source_round_fixture();
        let check = |db: &rusqlite::Connection| {
            agent_native_source_tool_authority(db, &context, "assignment.finish")
                .map(|_| ())
                .map_err(str::to_string)
        };
        let rounds::Start::Dispatch(call) =
            rounds::start_authorized(&db, &lease, &child, 1, &request, 8_000, check).unwrap()
        else {
            panic!()
        };
        let other = super::agent_tests::namespace_copy_child(&db, &child.run_id);
        db.execute("INSERT INTO agent_source_model_rounds(assignment_id,round_number,child_run_id,root_run_id,role,lease_epoch,fencing_token,request_json,request_hash,reserved_tokens,state)
            SELECT assignment_id,round_number,?2,root_run_id,role,lease_epoch,fencing_token,request_json,request_hash,reserved_tokens,state
            FROM agent_source_model_rounds WHERE child_run_id=?1",params![child.run_id,other]).unwrap();
        let read_other = || {
            db.query_row("SELECT json_array(assignment_id,round_number,child_run_id,root_run_id,role,lease_epoch,fencing_token,request_json,request_hash,reserved_tokens,state,response_json,usage_json,response_hash,event_sequence,failure_code,created_at,finished_at)
            FROM agent_source_model_rounds WHERE child_run_id=?1",[&other],|r|r.get::<_,String>(0)).unwrap()
        };
        let before = read_other();
        if phase == "uncertain" {
            rounds::record_uncertain(&db, &call, "original provider outcome unknown").unwrap();
        } else {
            rounds::record_received(
                &db,
                &call,
                &source_round_result("shared-call-id", "repo.inventory", json!({})),
            )
            .unwrap();
            db.execute("INSERT INTO agent_source_tool_receipts(assignment_id,child_run_id,round_number,call_index,call_id,tool_name,arguments_json,state)
                SELECT assignment_id,?2,round_number,call_index,call_id,tool_name,arguments_json,state FROM agent_source_tool_receipts WHERE child_run_id=?1",params![child.run_id,other]).unwrap();
            // The copied executing worker has no original attempt or paid
            // dispatch. It must block new work, while original callbacks still
            // settle only their own namespace. Never fabricate its allowance.
            let before_tool=application_table_snapshot(&db);
            let error=rounds::execute_tool(&db,&call,0,check,|_,_,_| {
                panic!("unowned copied dispatch granted a Source tool")
            }).unwrap_err();
            assert_eq!(error,"source_tool_original_finance_unavailable");
            assert_eq!(application_table_snapshot(&db),before_tool);
            for worker in [&child.run_id,&other] {
                assert_eq!(db.query_row("SELECT state FROM agent_source_tool_receipts WHERE child_run_id=?1",[worker],|r|r.get::<_,String>(0)).unwrap(),"planned");
            }
            assert!(rounds::continuation(&db,&call).is_err(),"unexecuted tool cannot become a continuation");
            assert!(db.execute("INSERT INTO agent_source_tool_receipts(assignment_id,child_run_id,round_number,call_index,call_id,tool_name,arguments_json,state)
                SELECT assignment_id,child_run_id,round_number,99,call_id,tool_name,arguments_json,'planned' FROM agent_source_tool_receipts WHERE child_run_id=?1",[&other]).is_err());
        }
        assert_eq!(read_other(), before, "{phase}");
        assert_eq!(db.query_row("SELECT count(*) FROM agent_events WHERE run_id=?1 AND event_type IN ('model_round_completed','tool_invocation_completed')",[&other],|r|r.get::<_,i64>(0)).unwrap(),0);
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}
