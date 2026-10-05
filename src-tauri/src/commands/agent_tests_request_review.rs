fn review_input(context: &AgentRunContext, item: &JsonValue) -> AgentRequestReviewInput {
    AgentRequestReviewInput {
        scan_id:context.scan_id.clone(),attempt_number:context.attempt_number,target_url:context.target_url.clone(),
        request_key:item["requestKey"].as_str().unwrap().into(),snapshot_hash:item["snapshotHash"].as_str().unwrap().into(),
        previous_review_id:String::new(),operation_id:Uuid::new_v4().to_string(),disposition:"still_unknown".into(),
        note:"人工检查本地记录，尚不能确认目标效果；保留未决。".into(),operator_confirmed:true,
    }
}

#[test]
fn request_review_real_claim_is_audited_without_refund_or_replay() {
    let (_root,context,runtime,request)=http_journal_fixture("http://127.0.0.1:9/",3);
    claim_agent_http_request(&context,&runtime,&request,&request.url).unwrap().unwrap();
    let mut connection=db::open(&context.db_path).unwrap();
    let before=agent_request_accounting(&connection,&context.scan_id,1,&context.target_url).unwrap().as_json();
    let view=request_review_view_in(&connection,&context.scan_id,1,&context.target_url).unwrap();
    assert_eq!(view["unitemizedExecutorRequests"],3);
    assert_eq!(view["items"].as_array().unwrap().len(),1);
    assert_eq!(view["items"][0]["receiptState"],"no_headers_recorded");
    let mut input=review_input(&context,&view["items"][0]);
    input.disposition="not_sent_attested".into();
    let receipt=record_request_review_in(&mut connection,&input).unwrap();
    assert_eq!(record_request_review_in(&mut connection,&input).unwrap(),receipt);
    assert_eq!(receipt["actor"],"local_operator");
    assert_eq!(agent_request_accounting(&connection,&context.scan_id,1,&context.target_url).unwrap().as_json(),before);
    assert_eq!(claim_agent_http_request(&context,&runtime,&request,&request.url).unwrap_err(),"http_journal_checkpoint_requires_reconciliation");
    assert!(connection.execute("UPDATE agent_request_reviews SET disposition='effect_observed'",[]).is_err());
    let view=request_review_view_in(&connection,&context.scan_id,1,&context.target_url).unwrap();
    assert_eq!(view["items"][0]["reviews"].as_array().unwrap().len(),1);
    assert_eq!(view["executionUnlocked"],false);
    let status=native_scan_status(&connection,&context.scan_id).unwrap();
    let events:Vec<_>=status["timeline"].as_array().unwrap().iter().filter(|i|i["eventType"]=="request_review").collect();
    assert_eq!(events.len(),1);
    assert_eq!(events[0]["fromRole"],"operator");
    assert_eq!(events[0]["status"],"not_sent_attested");
}

#[test]
fn request_review_late_headers_reject_stale_decision_but_keep_retry_and_revision_history() {
    let (_root,context,runtime,request)=http_journal_fixture("http://127.0.0.1:9/",0);
    let claim=claim_agent_http_request(&context,&runtime,&request,&request.url).unwrap().unwrap();
    let mut connection=db::open(&context.db_path).unwrap();
    let view=request_review_view_in(&connection,&context.scan_id,1,&context.target_url).unwrap();
    let input=review_input(&context,&view["items"][0]);
    let first=record_request_review_in(&mut connection,&input).unwrap();
    receive_agent_http_headers(&context,&claim,200).unwrap();
    assert_eq!(record_request_review_in(&mut connection,&input).unwrap(),first);
    let mut stale=review_input(&context,&view["items"][0]);
    stale.previous_review_id=input.operation_id.clone();
    assert_eq!(record_request_review_in(&mut connection,&stale).unwrap_err(),"request_review_source_changed");
    let updated=request_review_view_in(&connection,&context.scan_id,1,&context.target_url).unwrap();
    assert_eq!(updated["items"][0]["sourceChangedSinceReview"],true);
    assert_eq!(updated["items"][0]["receiptState"],"headers_recorded_body_not_asserted");
    let mut next=review_input(&context,&updated["items"][0]);
    assert_eq!(record_request_review_in(&mut connection,&next).unwrap_err(),"request_review_revision_changed");
    next.previous_review_id=input.operation_id;
    next.disposition="effect_observed".into();
    let second=record_request_review_in(&mut connection,&next).unwrap();
    let final_view=request_review_view_in(&connection,&context.scan_id,1,&context.target_url).unwrap();
    assert_eq!(final_view["items"][0]["reviews"],json!([first,second]));
    assert_eq!(final_view["items"][0]["sourceChangedSinceReview"],false);
}

#[test]
fn request_review_external_claim_and_received_capture_are_both_visible() {
    let (port,seen,stop)=spawn_endpoint(std::sync::Arc::new(|_|(200,"text/plain","entry".into())));
    let (_root,mut context,lease)=public_surface_fixture(&format!("http://127.0.0.1:{port}/"));
    let child=public_surface_child(&mut context,&lease);
    claim_public_surface_capture(&context,&lease,&child).unwrap();
    let mut connection=db::open(&context.db_path).unwrap();
    let view=request_review_view_in(&connection,&context.scan_id,1,&context.target_url).unwrap();
    assert_eq!(view["items"][0]["snapshot"]["source"],"external_surface");
    let input=review_input(&context,&view["items"][0]);
    record_request_review_in(&mut connection,&input).unwrap();
    assert!(seen.lock().unwrap().is_empty());
    assert_eq!(agent_request_accounting(&connection,&context.scan_id,1,&context.target_url).unwrap().external.unresolved,1);
    stop.store(true,std::sync::atomic::Ordering::SeqCst);

    let (port,seen,stop)=spawn_endpoint(std::sync::Arc::new(|_|(200,"text/plain","entry".into())));
    let (_root,mut context,lease)=public_surface_fixture(&format!("http://127.0.0.1:{port}/"));
    let child=public_surface_child(&mut context,&lease);
    capture_public_surface(&context,&lease,&child).unwrap();
    let connection=db::open(&context.db_path).unwrap();
    let view=request_review_view_in(&connection,&context.scan_id,1,&context.target_url).unwrap();
    assert_eq!(view["items"][0]["receiptState"],"response_recorded");
    assert_eq!(seen.lock().unwrap().len(),1);
    stop.store(true,std::sync::atomic::Ordering::SeqCst);
}

#[test]
fn request_review_invalid_confirmation_and_cross_target_do_not_write() {
    let (_root,context,runtime,request)=http_journal_fixture("http://127.0.0.1:9/",0);
    claim_agent_http_request(&context,&runtime,&request,&request.url).unwrap();
    let mut connection=db::open(&context.db_path).unwrap();
    let view=request_review_view_in(&connection,&context.scan_id,1,&context.target_url).unwrap();
    for variant in 0..7 {
        let mut input=review_input(&context,&view["items"][0]);
        match variant {
            0=>input.operator_confirmed=false,
            1=>input.operation_id="invalid".into(),
            2=>input.disposition="approved_to_replay".into(),
            3=>input.note=" ".into(),
            4=>input.note="x".repeat(4001),
            5=>input.target_url="http://127.0.0.1:8/".into(),
            _=>input.snapshot_hash="stale".into(),
        }
        assert!(record_request_review_in(&mut connection,&input).is_err());
    }
    assert_eq!(connection.query_row("SELECT COUNT(*) FROM agent_request_reviews",[],|r|r.get::<_,i64>(0)).unwrap(),0);
}

#[test]
fn request_review_fault_injection_rolls_back_and_tamper_fails_closed() {
    for trigger in [
        "CREATE TRIGGER fail_review BEFORE INSERT ON agent_request_reviews BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER fail_review AFTER INSERT ON agent_request_reviews BEGIN DELETE FROM agent_request_reviews WHERE id=NEW.id; END;",
        "CREATE TRIGGER fail_review AFTER INSERT ON agent_request_reviews BEGIN UPDATE agent_http_request_claims SET response_status=200,received_at='late'; END;",
        "CREATE TRIGGER fail_review BEFORE INSERT ON agent_collaboration_events WHEN NEW.event_type='request_review' BEGIN SELECT RAISE(IGNORE); END;",
    ] {
        let (_root,context,runtime,request)=http_journal_fixture("http://127.0.0.1:9/",0);
        claim_agent_http_request(&context,&runtime,&request,&request.url).unwrap();
        let mut connection=db::open(&context.db_path).unwrap();
        let view=request_review_view_in(&connection,&context.scan_id,1,&context.target_url).unwrap();
        let input=review_input(&context,&view["items"][0]);
        connection.execute_batch(trigger).unwrap();
        assert!(record_request_review_in(&mut connection,&input).is_err());
        assert_eq!(request_review_view_in(&connection,&context.scan_id,1,&context.target_url).unwrap(),view);
        connection.execute("DROP TRIGGER fail_review",[]).unwrap();
        record_request_review_in(&mut connection,&input).unwrap();
        connection.execute("DROP TRIGGER immutable_agent_request_review",[]).unwrap();
        connection.execute("UPDATE agent_request_reviews SET note='tampered'",[]).unwrap();
        assert_eq!(request_review_view_in(&connection,&context.scan_id,1,&context.target_url).unwrap_err(),"request_review_history_integrity");
    }
}

#[test]
fn request_review_authorization_claims_remain_charged_after_attestation() {
    use crate::agent_runtime::{contract::{AgentRole,AgentLane,AgentRunStatus,MultiAgentPolicy},
        multi_agent::{lease,scheduler},store::{self,AgentRunRow}};
    let (_root,path,app_dir,control)=authorization_fixture();
    let mut connection=db::open(&path).unwrap();
    save_authorization_control_in(&mut connection,&app_dir,&control).unwrap();
    let mut context=test_context(&path,&control.target_url,vec![AgentIdentity::scoped("session-a"),AgentIdentity::scoped("session-b")]);
    context.attempt_number=2;
    context.execution_plan=context.execution_plan.with_attempt(2);
    connection.execute("UPDATE sentinel_scans SET status='scanning',attempt_count=2 WHERE id=?1",[&control.scan_id]).unwrap();
    let mut root=AgentRunRow::new("review-authorization-root",&control.scan_id,2,&control.target_url,
        AgentBackendKind::Native,AgentRole::Coordinator,context.execution_plan.hash(),"evidence").with_budget(100,1000,2,10);
    root.status=AgentRunStatus::Running; root.root_run_id=root.id.clone();
    root.orchestration_policy=MultiAgentPolicy::Multi; root.lane=Some(AgentLane::ReadOnlyAnalysis);
    store::create_run(&connection,&root).unwrap();
    freeze_authorization_test_plan(&context);
    let lease=lease::acquire_coordinator_lease(&connection,&control.scan_id,2,&control.target_url,&root.id,600).unwrap();
    let child=scheduler::schedule_child(&connection,&lease,AgentRole::Authorization,AgentLane::TargetTouching,
        "test",&json!({}),1,&["authorization_probe".into()],0,0).unwrap();
    scheduler::mark_child_running(&connection,&lease,&child).unwrap();
    context.run=Some(AgentRunLedger{db_path:path,run_id:child.run_id.clone()});
    accounting_native(&context,0);
    for (side,identity,url) in [("owner",&control.owner_identity,&control.owner_object_url),
        ("cross",&control.tester_identity,&control.owner_object_url),("tester",&control.tester_identity,&control.tester_control_url)] {
        if side=="owner" {
            claim_authorization_probe(&context,&child,&control,side,identity,url).unwrap();
        } else {
            assert_eq!(claim_authorization_probe(&context,&child,&control,side,identity,url).unwrap_err(),
                "budget_indeterminate_requires_reconciliation");
            // Retain the three-source historical review contract. Older Native
            // releases could leave several unknown claims; seed those existing
            // facts directly, never grant new I/O to create the fixture.
            connection.execute("INSERT INTO agent_authorization_probe_claims(scan_id,attempt_number,target_url,contract_key,side,child_run_id,assignment_id)
                VALUES(?1,?2,?3,?4,?5,?6,?7)",params![context.scan_id,context.attempt_number,context.target_url,
                    control.contract_key,side,child.run_id,child.assignment_id]).unwrap();
        }
    }
    let before=request_review_view_in(&connection,&context.scan_id,2,&context.target_url).unwrap();
    assert_eq!(before["items"].as_array().unwrap().len(),3);
    for item in before["items"].as_array().unwrap() {
        assert_eq!(item["snapshot"]["source"],"authorization_probe");
        record_request_review_in(&mut connection,&review_input(&context,item)).unwrap();
    }
    let after=request_review_view_in(&connection,&context.scan_id,2,&context.target_url).unwrap();
    assert_eq!(before["accounting"],after["accounting"]);
    assert_eq!(after["accounting"]["authorizationUnresolvedClaims"],3);
    assert_eq!(after["accounting"]["budgetCommittedRequests"],3);
}

#[test]
fn request_review_concurrent_operator_decisions_have_one_winner() {
    let (_root,context,runtime,request)=http_journal_fixture("http://127.0.0.1:9/",0);
    claim_agent_http_request(&context,&runtime,&request,&request.url).unwrap();
    let connection=db::open(&context.db_path).unwrap();
    let view=request_review_view_in(&connection,&context.scan_id,1,&context.target_url).unwrap();
    let barrier=std::sync::Arc::new(std::sync::Barrier::new(2));
    let workers:Vec<_>=(0..2).map(|_| {
        let barrier=barrier.clone(); let path=context.db_path.clone();
        let input=review_input(&context,&view["items"][0]);
        std::thread::spawn(move || {
            let mut connection=db::open(&path).unwrap(); barrier.wait();
            record_request_review_in(&mut connection,&input)
        })
    }).collect();
    let results:Vec<_>=workers.into_iter().map(|h|h.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r|r.is_ok()).count(),1);
    assert!(results.iter().any(|r|r.as_ref().err().is_some_and(|e|e=="request_review_revision_changed")));
    let view=request_review_view_in(&connection,&context.scan_id,1,&context.target_url).unwrap();
    assert_eq!(view["items"][0]["reviews"].as_array().unwrap().len(),1);
}

#[test]
fn request_review_resume_keeps_prior_attestations_and_fresh_attempt_does_not_inherit_claims() {
    use crate::agent_runtime::{contract::AgentRole,store::{self,AgentRunRow}};
    let (_root,context,runtime,request)=http_journal_fixture("http://127.0.0.1:9/",0);
    claim_agent_http_request(&context,&runtime,&request,&request.url).unwrap();
    let mut connection=db::open(&context.db_path).unwrap();
    let view=request_review_view_in(&connection,&context.scan_id,1,&context.target_url).unwrap();
    let receipt=record_request_review_in(&mut connection,&review_input(&context,&view["items"][0])).unwrap();
    for (attempt,mode) in [(1,"initial"),(2,"resume"),(3,"fresh")] {
        connection.execute("INSERT INTO sentinel_scan_attempts(scan_id,attempt_number,execution_mode) VALUES(?1,?2,?3)",params![context.scan_id,attempt,mode]).unwrap();
        if attempt>1 {
            let run=AgentRunRow::new(format!("review-attempt-{attempt}"),&context.scan_id,attempt,&context.target_url,
                AgentBackendKind::Native,AgentRole::Coordinator,"plan","evidence");
            store::create_run(&connection,&run).unwrap();
        }
    }
    let resumed=request_review_view_in(&connection,&context.scan_id,2,&context.target_url).unwrap();
    assert_eq!(resumed["items"][0]["reviews"],json!([receipt]));
    let fresh=request_review_view_in(&connection,&context.scan_id,3,&context.target_url).unwrap();
    assert!(fresh["items"].as_array().unwrap().is_empty());
}

#[test]
fn request_review_database_reinitialization_adds_schema_and_preserves_receipts() {
    let (_root,context,runtime,request)=http_journal_fixture("http://127.0.0.1:9/",0);
    let parent=context.db_path.parent().unwrap();
    claim_agent_http_request(&context,&runtime,&request,&request.url).unwrap();
    let connection=db::open(&context.db_path).unwrap();
    connection.execute_batch("DROP TABLE agent_request_reviews").unwrap();
    drop(connection);
    assert_eq!(db::initialize(parent).unwrap(),context.db_path);
    let mut connection=db::open(&context.db_path).unwrap();
    let view=request_review_view_in(&connection,&context.scan_id,1,&context.target_url).unwrap();
    let input=review_input(&context,&view["items"][0]);
    let receipt=record_request_review_in(&mut connection,&input).unwrap();
    drop(connection);
    db::initialize(parent).unwrap();
    let mut connection=db::open(&context.db_path).unwrap();
    assert_eq!(record_request_review_in(&mut connection,&input).unwrap(),receipt);
    assert_eq!(connection.query_row("SELECT COUNT(*) FROM agent_collaboration_events WHERE event_type='request_review'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
}

#[test]
fn request_review_historical_journal_survives_a_newer_checkpoint_without_false_zero() {
    let (_root,mut context,runtime,request)=http_journal_fixture("http://127.0.0.1:9/",5);
    claim_agent_http_request(&context,&runtime,&request,&request.url).unwrap();
    let mut connection=db::open(&context.db_path).unwrap();
    let before=request_review_view_in(&connection,&context.scan_id,1,&context.target_url).unwrap();
    let input=review_input(&context,&before["items"][0]);
    record_request_review_in(&mut connection,&input).unwrap();
    context.attempt_number=2;
    connection.execute("UPDATE sentinel_scans SET attempt_count=2 WHERE id=?1",[&context.scan_id]).unwrap();
    accounting_native(&context,99);
    let history=request_review_view_in(&connection,&context.scan_id,1,&context.target_url).unwrap();
    assert_eq!(history["accounting"],before["accounting"]);
    assert_eq!(history["accounting"]["budgetCommittedRequests"],6);
    assert_eq!(history["unitemizedExecutorRequests"],5);
    assert_eq!(history["items"][0]["reviews"].as_array().unwrap().len(),1);
}

#[test]
fn request_review_event_integrity_is_required_for_history_retry_and_correction() {
    for damage in [
        "DELETE FROM agent_collaboration_events WHERE entity_id=?1",
        "UPDATE agent_collaboration_events SET scan_id='other-scan' WHERE entity_id=?1",
        "UPDATE agent_collaboration_events SET attempt_number=99 WHERE entity_id=?1",
        "UPDATE agent_collaboration_events SET entity_type='directive' WHERE entity_id=?1",
        "UPDATE agent_collaboration_events SET event_type='directive' WHERE entity_id=?1",
        "UPDATE agent_collaboration_events SET payload_json='{}' WHERE entity_id=?1",
        "UPDATE agent_collaboration_events SET payload_json='not json' WHERE entity_id=?1",
        "UPDATE agent_collaboration_events SET payload_json=json_object('disposition','effect_observed','executionUnlocked',json('false')) WHERE entity_id=?1",
        "UPDATE agent_collaboration_events SET payload_json=json_object('disposition','still_unknown','executionUnlocked',0) WHERE entity_id=?1",
        "UPDATE agent_collaboration_events SET payload_json=json_object('disposition','still_unknown','executionUnlocked',json('false'),'automaticReplayAllowed',json('true')) WHERE entity_id=?1",
        "INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json) SELECT scan_id,attempt_number,entity_type,entity_id,event_type,payload_json FROM agent_collaboration_events WHERE entity_id=?1",
    ] {
        let (_root,context,runtime,request)=http_journal_fixture("http://127.0.0.1:9/",0);
        claim_agent_http_request(&context,&runtime,&request,&request.url).unwrap();
        let mut connection=db::open(&context.db_path).unwrap();
        let view=request_review_view_in(&connection,&context.scan_id,1,&context.target_url).unwrap();
        let input=review_input(&context,&view["items"][0]);
        record_request_review_in(&mut connection,&input).unwrap();
        connection.execute(damage,[&input.operation_id]).unwrap();
        let before=receipt_database_snapshot(&connection);
        let execution_before=invocation_test_snapshot(&connection);
        let accounting_before=agent_request_accounting(&connection,&context.scan_id,1,&context.target_url).unwrap().as_json();
        assert!(request_review_view_in(&connection,&context.scan_id,1,&context.target_url).is_err(),"history accepted {damage}");
        assert!(record_request_review_in(&mut connection,&input).is_err(),"retry accepted {damage}");
        let mut correction=review_input(&context,&view["items"][0]);
        correction.previous_review_id=input.operation_id;
        assert!(record_request_review_in(&mut connection,&correction).is_err(),"correction accepted {damage}");
        assert_eq!(receipt_database_snapshot(&connection),before);
        assert_eq!(invocation_test_snapshot(&connection),execution_before);
        assert_eq!(agent_request_accounting(&connection,&context.scan_id,1,&context.target_url).unwrap().as_json(),accounting_before);
        assert_eq!(connection.query_row("SELECT COUNT(*) FROM agent_request_reviews",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    }
}

#[test]
fn request_review_event_integrity_rejects_partial_match_at_initial_commit() {
    for action in [
        "UPDATE agent_collaboration_events SET payload_json=json_object('disposition','still_unknown','executionUnlocked',0) WHERE entity_id=NEW.entity_id;",
        "UPDATE agent_collaboration_events SET payload_json=json_object('disposition','still_unknown','executionUnlocked',json('false'),'automaticReplayAllowed',json('true')) WHERE entity_id=NEW.entity_id;",
        "INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json) VALUES('other',1,'request_review',NEW.entity_id,'request_review','{}');",
    ] {
        let (_root,context,runtime,request)=http_journal_fixture("http://127.0.0.1:9/",0);
        claim_agent_http_request(&context,&runtime,&request,&request.url).unwrap();
        let mut connection=db::open(&context.db_path).unwrap();
        let view=request_review_view_in(&connection,&context.scan_id,1,&context.target_url).unwrap();
        let input=review_input(&context,&view["items"][0]);
        connection.execute_batch(&format!("CREATE TRIGGER malformed_review_event AFTER INSERT ON agent_collaboration_events WHEN NEW.event_type='request_review' AND NEW.scan_id<>'other' BEGIN {action} END;")).unwrap();
        let before=receipt_database_snapshot(&connection);
        let execution_before=invocation_test_snapshot(&connection);
        assert!(record_request_review_in(&mut connection,&input).is_err(),"accepted {action}");
        assert_eq!(receipt_database_snapshot(&connection),before);
        assert_eq!(invocation_test_snapshot(&connection),execution_before);
        assert_eq!(request_review_view_in(&connection,&context.scan_id,1,&context.target_url).unwrap(),view);
        connection.execute_batch("DROP TRIGGER malformed_review_event").unwrap();
        record_request_review_in(&mut connection,&input).unwrap();
    }
}

#[test]
fn request_review_timeline_checks_receipts_even_behind_the_incremental_cursor() {
    for damage in [
        "DELETE FROM agent_collaboration_events WHERE entity_id=?1",
        "UPDATE agent_collaboration_events SET payload_json='{}' WHERE entity_id=?1",
        "UPDATE agent_request_reviews SET disposition='effect_observed' WHERE id=?1",
        "UPDATE agent_request_reviews SET note='changed after persistence' WHERE id=?1",
        "DELETE FROM agent_request_reviews WHERE id=?1",
    ] {
        let (_root,context,runtime,request)=http_journal_fixture("http://127.0.0.1:9/",0);
        claim_agent_http_request(&context,&runtime,&request,&request.url).unwrap();
        let mut connection=db::open(&context.db_path).unwrap();
        let view=request_review_view_in(&connection,&context.scan_id,1,&context.target_url).unwrap();
        let input=review_input(&context,&view["items"][0]);
        record_request_review_in(&mut connection,&input).unwrap();
        let status=native_scan_status_after(&connection,&context.scan_id,None).unwrap();
        let sequence=connection.query_row("SELECT sequence FROM agent_collaboration_events WHERE entity_id=?1",[&input.operation_id],|r|r.get::<_,i64>(0)).unwrap();
        assert!(status["timeline"].as_array().unwrap().iter().any(|e|e["id"]==input.operation_id && e["sequence"]==sequence));
        assert!(!native_scan_status_after(&connection,&context.scan_id,Some(sequence)).unwrap()["timeline"].as_array().unwrap().iter().any(|e|e["id"]==input.operation_id));
        connection.execute_batch("DROP TRIGGER immutable_agent_request_review").unwrap();
        connection.execute(damage,[&input.operation_id]).unwrap();
        let before=receipt_database_snapshot(&connection);
        for cursor in [None,Some(0),Some(sequence),Some(sequence+1)] {
            assert!(native_scan_status_after(&connection,&context.scan_id,cursor).is_err(),"timeline accepted {damage} at {cursor:?}");
        }
        assert_eq!(receipt_database_snapshot(&connection),before);
    }
}
