// Actual live SDK regressions use new original Root/parent fixtures.
#[test]
fn directive_proposal_completed_result_is_revalidated_before_projection_and_context() {
    for valid in [true,false] {
    for damage in [
        "DELETE FROM agent_messages WHERE kind='human_assessment_result'",
        "UPDATE agent_messages SET acknowledged_at='' WHERE kind='human_assessment_result'",
        "UPDATE agent_messages SET payload_json=json_set(payload_json,'$.assessment.summary','changed') WHERE kind='human_assessment_result'",
        "UPDATE agent_assignments SET budget_settled_at='' WHERE id IN (SELECT assignment_id FROM agent_directive_proposals)",
        "UPDATE agent_user_directives SET text_redacted='changed after completion'",
        "DELETE FROM agent_directive_proposals",
        "DELETE FROM agent_events WHERE event_type='model_round_completed'",
        "UPDATE agent_messages SET from_agent='coordinator' WHERE kind='human_assessment_result'",
        "UPDATE agent_messages SET payload_json=json_set(payload_json,'$.request','changed') WHERE kind='human_assessment_request'",
        "UPDATE agent_user_directives SET payload_json=json_set(payload_json,'$.estimatedBudget.tokens',1)",
        "DELETE FROM agent_collaboration_events WHERE event_type='user_directive' AND json_extract(payload_json,'$.status') IN ('completed','failed')",
    ] {
        let (root,mut context,id,_parent)=live_proposal_fixture("proposal-history-proof","@mapper 请分析已有证据");
        let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(move |_|(200,"application/json",proposal_model_response(if valid { valid_proposal_text() } else { "invalid model response" }))));
        context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
        let mut inbox=take_human_directives(&context).unwrap();
        assert_eq!(apply_human_proposal_actions(&context,&mut inbox).unwrap().len(),usize::from(valid));
        let connection=db::open(&context.db_path).unwrap();
        connection.execute_batch(damage).unwrap();
        let before=connection.total_changes();
        let timeline=native_scan_status(&connection,&context.scan_id).unwrap();
        let event=timeline["timeline"].as_array().unwrap().iter().find(|event|event["id"]==id).unwrap();
        assert!(event["proposalAction"].is_null(),"{damage}: {event}");
        assert_eq!(event["deliveryState"],"receipt_unverified","{damage}");
        assert!(event["reasonCodes"].as_array().unwrap().contains(&serde_json::json!("proposal_receipt_unverified")));
        for message in timeline["timeline"].as_array().unwrap().iter().filter(|event|event["messageKind"]=="human_assessment_result") {
            assert_eq!(message["deliveryState"],"receipt_unverified","{damage}");
            assert!(message["summary"].as_str().unwrap().contains("未通过交付校验"));
        }
        assert_eq!(connection.total_changes(),before,"history reads cannot repair execution");
        assert!(apply_human_proposal_actions(&context,&mut inbox).is_err(),"{damage}");
        assert_eq!(seen.lock().unwrap().len(),1,"invalid proof must not reissue model calls");
        let _=fs::remove_dir_all(root);
    }
    }
}

#[test]
fn directive_proposal_history_survives_terminal_root_and_replaced_current_fence() {
    use crate::agent_runtime::multi_agent::directive::proposals;
    let (root,mut context,id,_parent)=live_proposal_fixture("proposal-terminal-history","@mapper 请分析已有证据");
    let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(|_|(200,"application/json",proposal_model_response(valid_proposal_text()))));
    context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
    let mut inbox=take_human_directives(&context).unwrap();
    assert_eq!(apply_human_proposal_actions(&context,&mut inbox).unwrap().len(),1);
    let lease=inbox.lease.as_ref().unwrap();
    let connection=db::open(&context.db_path).unwrap();
    finish_coordinator_run(&connection,lease,&AgentTargetOutcome::Cancelled).unwrap();
    connection.execute_batch("UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1,fencing_token='replacement',lease_expires_at='2000-01-01 00:00:00';").unwrap();
    let before=receipt_database_snapshot(&connection);
    let timeline=native_scan_status(&connection,&context.scan_id).unwrap();
    let event=timeline["timeline"].as_array().unwrap().iter().find(|event|event["id"]==id).unwrap();
    assert_eq!(event["proposalAction"]["state"],"completed");
    assert_ne!(event["deliveryState"],"receipt_unverified");
    assert_eq!(proposals::verified_context(&connection,lease).unwrap().len(),1);
    assert_eq!(before,receipt_database_snapshot(&connection));
    assert!(apply_human_proposal_actions(&context,&mut inbox).is_err());
    assert_eq!(seen.lock().unwrap().len(),1);
    let _=fs::remove_dir_all(root);
}

#[test]
fn directive_proposal_silent_completion_write_loss_rolls_back_local_commit() {
    for trigger in [
        "CREATE TRIGGER ignore_proposal_completion BEFORE UPDATE OF state ON agent_directive_proposals WHEN NEW.state='completed' BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER ignore_proposal_completion BEFORE INSERT ON agent_collaboration_events WHEN NEW.event_type='user_directive' AND json_extract(NEW.payload_json,'$.status')='completed' BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER ignore_proposal_completion BEFORE INSERT ON agent_collaboration_events WHEN NEW.event_type='user_directive' AND json_extract(NEW.payload_json,'$.status')='applied' BEGIN SELECT RAISE(IGNORE); END;",
    ] {
        let (root,mut context,id,_parent)=live_proposal_fixture("proposal-ignored-completion","@mapper 请分析已有证据");
        let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(|_|(200,"application/json",proposal_model_response(valid_proposal_text()))));
        context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
        let connection=db::open(&context.db_path).unwrap();
        connection.execute_batch(trigger).unwrap();
        let mut inbox=take_human_directives(&context).unwrap();
        assert!(apply_human_proposal_actions(&context,&mut inbox).is_err(),"{trigger}");
        let state:(String,String,String,i64)=connection.query_row(
            "SELECT p.state,d.status,a.budget_settled_at,(SELECT COUNT(*) FROM agent_messages WHERE kind='human_assessment_result') FROM agent_directive_proposals p JOIN agent_user_directives d ON d.id=p.directive_id JOIN agent_assignments a ON a.id=p.assignment_id WHERE d.id=?1",
            [&id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
        ).unwrap();
        assert_eq!(state,("received".into(),"assigned".into(),String::new(),0));
        connection.execute_batch("DROP TRIGGER ignore_proposal_completion").unwrap();
        assert_eq!(apply_human_proposal_actions(&context,&mut inbox).unwrap().len(),1);
        assert_eq!(seen.lock().unwrap().len(),1,"local commit retry must reuse the saved response");
        let _=fs::remove_dir_all(root);
    }
}

#[test]
fn directive_proposal_pre_dispatch_rechecks_frozen_payload_and_assessment_request() {
    use crate::agent_runtime::multi_agent::directive::proposals;
    for damage in [
        "UPDATE agent_user_directives SET payload_json=json_set(payload_json,'$.estimatedBudget.tokens',1)",
        "UPDATE agent_assignments SET task_slice_json=json_set(task_slice_json,'$.request','changed'); UPDATE agent_messages SET payload_json=json_set(payload_json,'$.request','changed') WHERE kind='human_assessment_request'",
    ] {
        let (root,mut context,_id,_parent)=live_proposal_fixture("proposal-dispatch-proof","@mapper 请分析已有证据");
        let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(|_|(200,"application/json",proposal_model_response(valid_proposal_text()))));
        context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
        let mut inbox=take_human_directives(&context).unwrap();
        let connection=db::open(&context.db_path).unwrap();
        proposals::prepare_next(&connection,inbox.lease.as_ref().unwrap(),&context.evidence).unwrap().unwrap();
        connection.execute_batch(damage).unwrap();
        assert!(apply_human_proposal_actions(&context,&mut inbox).is_err());
        assert_eq!(seen.lock().unwrap().len(),0,"damaged frozen request must be refused before transport: {damage}");
        let _=fs::remove_dir_all(root);
    }
}

#[test]
fn directive_proposal_real_child_transport_mailbox_and_truthful_completion() {
    for role in ["@mapper","@investigator"] {
        let (root,mut context,id,_parent)=live_proposal_fixture("proposal-transport",&format!("{role} 请分析已有证据"));
        let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(|_|(200,"application/json",proposal_model_response(valid_proposal_text()))));
        context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
        let mut inbox=take_human_directives(&context).unwrap();
        let messages=apply_human_proposal_actions(&context,&mut inbox).unwrap();
        assert_eq!(messages.len(),1);
        assert!(inbox.items.is_empty());
        assert!(messages[0]["content"].as_str().unwrap().contains("独立评估已有路由"));
        assert_eq!(apply_human_proposal_actions(&context,&mut inbox).unwrap(),messages);
        let seen=seen.lock().unwrap();
        assert_eq!(seen.len(),1,"recovery cannot make a second provider call");
        let request:JsonValue=serde_json::from_str(seen[0].split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(request["max_tokens"],512);
        assert!(request.get("tools").is_none_or(|tools|tools.as_array().is_some_and(|a|a.is_empty())));
        assert!(request["messages"][0]["content"].as_str().unwrap().contains("independent"));
        let input:JsonValue=serde_json::from_str(request["messages"][1]["content"].as_str().unwrap()).unwrap();
        assert_eq!(input["directiveId"],id);
        assert_eq!(input["frozenEvidence"],context.evidence);
        let connection=db::open(&context.db_path).unwrap();
        let (status,child,assignment):(String,String,String)=connection.query_row("SELECT d.status,p.child_run_id,p.assignment_id FROM agent_user_directives d JOIN agent_directive_proposals p ON p.directive_id=d.id WHERE d.id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(status,"completed");
        assert_ne!(child,context.run.as_ref().unwrap().run_id);
        let (count,acked):(i64,i64)=connection.query_row("SELECT COUNT(*),SUM(acknowledged_at<>'') FROM agent_messages WHERE assignment_id=?1",[assignment],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!((count,acked),(2,2));
        let usage:(i64,i64)=connection.query_row("SELECT used_tokens,used_requests FROM agent_runs WHERE id=?1",[child],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(usage,(20,1));
        let events:i64=connection.query_row("SELECT COUNT(*) FROM agent_events WHERE event_type='model_round_completed'",[],|r|r.get(0)).unwrap();
        assert_eq!(events,1);
        let timeline=native_scan_status(&connection,&context.scan_id).unwrap();
        let event=timeline["timeline"].as_array().unwrap().iter().find(|event|event["id"]==id).unwrap();
        assert_eq!(event["proposalAction"]["state"],"completed");
        assert_eq!(event["proposalAction"]["coverageVerified"],false);
        let _=fs::remove_dir_all(root);
    }
}

#[test]
fn directive_proposal_invalid_response_is_failed_not_completed() {
    let (root,mut context,id,_parent)=live_proposal_fixture("proposal-invalid","@mapper 请分析已有证据");
    let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(|_|(200,"application/json",proposal_model_response("not a proposal"))));
    context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
    let mut inbox=take_human_directives(&context).unwrap();
    assert!(apply_human_proposal_actions(&context,&mut inbox).unwrap().is_empty());
    assert!(apply_human_proposal_actions(&context,&mut inbox).unwrap().is_empty());
    assert_eq!(seen.lock().unwrap().len(),1);
    let connection=db::open(&context.db_path).unwrap();
    let status:String=connection.query_row("SELECT status FROM agent_user_directives WHERE id=?1",[id],|r|r.get(0)).unwrap();
    assert_eq!(status,"failed");
    let _=fs::remove_dir_all(root);
}

#[test]
fn directive_proposal_transport_uncertain_retains_reservation_without_retry() {
    for status_code in [401,503] {
    let (root,mut context,id,_parent)=live_proposal_fixture("proposal-uncertain","@mapper 请分析已有证据");
    let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(move |_|(status_code,"application/json",r#"{"error":{"message":"fixture rejected","type":"fixture_error"}}"#.into())));
    context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
    let mut inbox=take_human_directives(&context).unwrap();
    let error = apply_human_proposal_actions(&context,&mut inbox).unwrap_err();
    assert!(error.contains("子智能体模型调用失败"), "{error}");
    let original_rows=receipt_database_snapshot(&db::open(&context.db_path).unwrap());
    assert_eq!(apply_human_proposal_actions(&context,&mut inbox).unwrap_err(),"budget_indeterminate_requires_reconciliation");
    assert_eq!(receipt_database_snapshot(&db::open(&context.db_path).unwrap()),original_rows);
    assert_eq!(seen.lock().unwrap().len(),1);
    let connection=db::open(&context.db_path).unwrap();
    let (state,reserved):(String,i64)=connection.query_row("SELECT p.state,a.reserved_requests FROM agent_directive_proposals p JOIN agent_assignments a ON a.id=p.assignment_id WHERE directive_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!((state.as_str(),reserved),("executing",1));
    let fees=crate::agent_runtime::multi_agent::budget::balance(&connection,&context.run.as_ref().unwrap().run_id,None,"model_requests").unwrap();
    assert_eq!(fees.indeterminate,1,"actual unknown transport retains the original fee");
    assert_eq!(human_owned_calls(&connection),1);
    assert_eq!(human_owned_acked(&connection),0);
    let _=fs::remove_dir_all(root);
    }
}

#[test]
fn directive_proposal_capacity_and_reviewer_constraints_do_not_abort_web_inbox() {
    use crate::agent_runtime::multi_agent::directive::proposals;
    for (text,reason,small_budget) in [
        ("@reviewer 请复核已有证据","proposal_reviewer_requires_frozen_candidate",false),
        ("@mapper @investigator 请联合分析","ordered_budget_unavailable",true),
        ("@mapper 请分析已有证据","proposal_budget_unavailable",true),
    ] {
        let (root,context,id,_parent)=live_proposal_fixture_limits("proposal-deferral",text, (if small_budget {100} else {60_000}, 20));
        let mut inbox=take_human_directives(&context).unwrap();
        let connection=db::open(&context.db_path).unwrap();
        assert!(proposals::prepare_next(&connection,inbox.lease.as_ref().unwrap(),&context.evidence).unwrap().is_none());
        assert!(apply_human_proposal_actions(&context,&mut inbox).unwrap().is_empty());
        let (status,code):(String,String)=connection.query_row("SELECT status,rejection_code FROM agent_user_directives WHERE id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!((status.as_str(),code.as_str()),("deferred",reason));
        let count:i64=connection.query_row("SELECT COUNT(*) FROM agent_assignments",[],|r|r.get(0)).unwrap();
        assert_eq!(count,0);
        let _=fs::remove_dir_all(root);
    }
}
