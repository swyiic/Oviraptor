#[test]
fn ordered_execution_local_ignore_or_business_trigger_rolls_back_without_losing_original_fee() {
    for trigger in [
      "CREATE TRIGGER ordered_fault BEFORE UPDATE OF state ON agent_directive_ordered_actions WHEN NEW.state='received' BEGIN SELECT RAISE(IGNORE); END;",
      "CREATE TRIGGER ordered_fault BEFORE INSERT ON agent_directive_ordered_receipts BEGIN SELECT RAISE(IGNORE); END;",
      "CREATE TRIGGER ordered_fault BEFORE INSERT ON agent_collaboration_events WHEN json_extract(NEW.payload_json,'$.orderedReceiptId') IS NOT NULL BEGIN SELECT RAISE(IGNORE); END;",
      "CREATE TRIGGER ordered_fault AFTER INSERT ON agent_directive_ordered_receipts BEGIN UPDATE projects SET name='unauthorized'; END;",
      "CREATE TRIGGER ordered_fault AFTER INSERT ON agent_directive_ordered_receipts BEGIN UPDATE agent_runs SET plan_json='{}' WHERE id='ordered-other-root'; END;",
      "CREATE TRIGGER ordered_fault AFTER INSERT ON agent_directive_ordered_receipts BEGIN UPDATE agent_directive_drafts SET safe_execution_text='replaced' WHERE id='ordered-old-draft'; END;",
    ] {
      let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(|_|(200,"application/json",proposal_model_response(valid_proposal_text()))));
      let (f,id)=ordered_exec_fixture("ordered-local-fault",&format!("http://127.0.0.1:{port}/v1"));let db=db::open(&f.context.db_path).unwrap();
      db.execute("INSERT INTO agent_runs(id,scan_id,attempt_number,plan_json,status) VALUES('ordered-other-root',?1,1,?2,'completed')",params![f.actor.scan_id,f.context.execution_plan.as_json().to_string()]).unwrap();
      let old=ordered_human_draft(&db,&f.actor,"@mapper 请评估已有证据");
      db.execute("UPDATE agent_directive_drafts SET id='ordered-old-draft' WHERE id=?1",[old.id]).unwrap();
      let before=super::tests::application_table_snapshot(&db);let native=ordered_exec_native(&db,&f.actor.root_run_id);
      db.execute_batch(trigger).unwrap();assert!(ordered_exec_apply(&f).is_err(),"{trigger}");
      let after=super::tests::application_table_snapshot(&db);
      for table in ["projects","agent_directive_drafts"] {assert_eq!(before.iter().find(|(t,_)|t==table),after.iter().find(|(t,_)|t==table),"{trigger}: {table}");}
      let other:String=db.query_row("SELECT plan_json FROM agent_runs WHERE id='ordered-other-root'",[],|r|r.get(0)).unwrap();assert_eq!(other,f.context.execution_plan.as_json().to_string());
      assert_eq!(ordered_exec_receipts(&db,&id).len(),0);assert_eq!(seen.lock().unwrap().len(),1);
      assert_eq!(ordered_exec_native(&db,&f.actor.root_run_id),native);
      let paid:i64=db.query_row("SELECT COUNT(*) FROM agent_specialist_calls c JOIN agent_directive_ordered_actions a ON a.child_run_id=c.child_run_id WHERE a.directive_id=?1 AND c.state='received' AND c.event_sequence>0",[&id],|r|r.get(0)).unwrap();assert_eq!(paid,1,"physical original paid receipt survives local rollback");
      db.execute_batch("DROP TRIGGER ordered_fault").unwrap();ordered_exec_apply(&f).unwrap();
      assert_eq!(ordered_exec_receipts(&db,&id).len(),1);assert_eq!(seen.lock().unwrap().len(),1,"local repair must not send a second HTTP request");
    }
}

#[test]
fn ordered_execution_dispatch_or_schedule_trigger_cannot_mint_business_or_root_authority() {
    for (table, body) in [
        (
            "agent_directive_ordered_actions",
            "UPDATE projects SET name='hidden-side-effect'",
        ),
        (
            "agent_directive_ordered_actions",
            "UPDATE agent_runs SET plan_json='{}' WHERE role='coordinator'",
        ),
        (
            "agent_specialist_calls",
            "UPDATE projects SET name='hidden-side-effect'",
        ),
        (
            "agent_specialist_calls",
            "UPDATE agent_runs SET plan_json='{}' WHERE role='coordinator'",
        ),
    ] {
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
            (
                200,
                "application/json",
                proposal_model_response(valid_proposal_text()),
            )
        }));
        let (f, _id) = ordered_exec_fixture(
            "ordered-dispatch-intent",
            &format!("http://127.0.0.1:{port}/v1"),
        );
        let db = db::open(&f.context.db_path).unwrap();
        let native = ordered_exec_native(&db, &f.actor.root_run_id);
        let before = super::tests::application_table_snapshot(&db);
        db.execute_batch(&format!(
            "CREATE TRIGGER ordered_fault AFTER INSERT ON {table} BEGIN {body}; END;"
        ))
        .unwrap();
        assert!(ordered_exec_apply(&f).is_err(), "{table}: {body}");
        assert_eq!(seen.lock().unwrap().len(), 0);
        assert_eq!(ordered_exec_count(&db, "agent_specialist_calls"), 0);
        assert_eq!(ordered_exec_native(&db, &f.actor.root_run_id), native);
        let after = super::tests::application_table_snapshot(&db);
        assert_eq!(
            before.iter().find(|(t, _)| t == "projects"),
            after.iter().find(|(t, _)| t == "projects")
        );
    }
}

#[test]
fn ordered_execution_all_receipt_unique_keys_deny_replace_even_with_recursive_triggers_off() {
    use crate::agent_runtime::multi_agent::directive::ordered_execution as ordered;
    let (port, _seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            proposal_model_response(valid_proposal_text()),
        )
    }));
    let (f, id) =
        ordered_exec_fixture("ordered-no-replace", &format!("http://127.0.0.1:{port}/v1"));
    let db = db::open(&f.context.db_path).unwrap();
    ordered_exec_apply(&f).unwrap();
    ordered_exec_apply(&f).unwrap();
    let before = ordered_exec_receipts(&db, &id);
    let next = confirm_queue_directive(&db, &f.actor, "@mapper 然后 @investigator 评估已有证据");
    let inbox = take_human_directives(&f.context).unwrap();
    let actor = inbox.lease.as_ref().unwrap();
    let job = ordered::prepare_next(&db, actor, &f.context.evidence, |_| Ok(()))
        .unwrap()
        .unwrap();
    assert_eq!(job.directive_id, next);
    db.execute_batch("PRAGMA recursive_triggers=OFF").unwrap();
    let sequence:i64=db.query_row("SELECT sequence FROM agent_directive_ordered_receipts WHERE directive_id=?1 AND action_order=1",[&id],|r|r.get(0)).unwrap();
    for key in [
        "sequence",
        "receipt_id",
        "action_id",
        "child_run_id",
        "directive_order",
    ] {
        let seq = if key == "sequence" { sequence } else { 9000 };
        let receipt = if key == "receipt_id" {
            before[0]["receiptId"].as_str().unwrap().into()
        } else {
            uuid::Uuid::new_v4().to_string()
        };
        let action = if key == "action_id" {
            before[0]["actionId"].as_str().unwrap()
        } else {
            &job.action.action_id
        };
        let child = if key == "child_run_id" {
            before[0]["childRunId"].as_str().unwrap()
        } else {
            &job.child.run_id
        };
        let directive = if key == "directive_order" { &id } else { &next };
        let error=db.execute("INSERT OR REPLACE INTO agent_directive_ordered_receipts(sequence,receipt_id,action_id,directive_id,action_order,child_run_id,receipt_json) VALUES(?1,?2,?3,?4,1,?5,'{}')",
        params![seq,receipt,action,directive,child]).unwrap_err().to_string();
        assert!(
            error.contains("ordered_receipt_no_replace"),
            "{key}: {error}"
        );
        assert_eq!(ordered_exec_receipts(&db, &id), before);
    }
    for statement in [
        "UPDATE agent_directive_ordered_receipts SET receipt_json='{}'",
        "DELETE FROM agent_directive_ordered_receipts",
    ] {
        assert!(db.execute(statement, []).is_err());
        assert_eq!(ordered_exec_receipts(&db, &id), before);
    }
}

#[test]
fn ordered_execution_public_history_is_read_only_and_damage_withholds_actual_timeline_receipt() {
    for damage in [
      "UPDATE agent_messages SET acknowledged_at='' WHERE kind='human_ordered_assessment_result'",
      "DROP TRIGGER agent_specialist_call_immutable; UPDATE agent_specialist_calls SET request_hash='wrong-hash'",
      "DELETE FROM agent_collaboration_events WHERE json_extract(payload_json,'$.orderedReceiptId') IS NOT NULL",
      "DROP TRIGGER ordered_receipt_frozen; UPDATE agent_directive_ordered_receipts SET receipt_json=json_set(receipt_json,'$.independentReviewApproved',json('true'))",
    ] {
      let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(|_|(200,"application/json",proposal_model_response(valid_proposal_text()))));
      let (f,id)=ordered_exec_fixture("ordered-reader-proof",&format!("http://127.0.0.1:{port}/v1"));let db=db::open(&f.context.db_path).unwrap();ordered_exec_apply(&f).unwrap();
      db.execute_batch(damage).unwrap();let changes=db.total_changes();
      let timeline=native_scan_status_after(&db,&f.actor.scan_id,None).unwrap();
      let item=timeline["timeline"].as_array().unwrap().iter().find(|r|r["id"]==id&&r["eventType"]=="user_directive").unwrap();
      assert!(item["orderedAssessmentExecution"].is_null());assert_eq!(item["deliveryState"],"receipt_unverified");
      assert_eq!(db.total_changes(),changes);assert!(ordered_exec_apply(&f).is_err());assert_eq!(seen.lock().unwrap().len(),1);
    }
}
#[test]
fn ordered_execution_checkpoint_unique_keys_and_frozen_fields_deny_replacement() {
    let (port, _seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            proposal_model_response(valid_proposal_text()),
        )
    }));
    let (f, id) = ordered_exec_fixture(
        "ordered-checkpoint-frozen",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let db = db::open(&f.context.db_path).unwrap();
    ordered_exec_apply(&f).unwrap();
    let next = confirm_queue_directive(&db, &f.actor, "@mapper 然后 @investigator 评估已有证据");
    let original:(String,String,String,String)=db.query_row("SELECT action_id,assignment_id,child_run_id,request_message_id FROM agent_directive_ordered_actions WHERE directive_id=?1 AND action_order=1",[&id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    let before = ordered_exec_receipts(&db, &id);
    db.execute_batch("PRAGMA recursive_triggers=OFF").unwrap();
    for key in [
        "action_id",
        "assignment_id",
        "child_run_id",
        "request_message_id",
        "directive_order",
    ] {
        let action = if key == "action_id" {
            original.0.clone()
        } else {
            "e".repeat(64)
        };
        let assignment = if key == "assignment_id" {
            original.1.clone()
        } else {
            uuid::Uuid::new_v4().to_string()
        };
        let child = if key == "child_run_id" {
            original.2.clone()
        } else {
            uuid::Uuid::new_v4().to_string()
        };
        let message = if key == "request_message_id" {
            original.3.clone()
        } else {
            uuid::Uuid::new_v4().to_string()
        };
        let directive = if key == "directive_order" { &id } else { &next };
        let order = if key == "directive_order" { 1 } else { 2 };
        let error=db.execute("INSERT OR REPLACE INTO agent_directive_ordered_actions(action_id,directive_id,action_order,plan_hash,assignment_id,child_run_id,request_message_id,input_json)
        VALUES(?1,?2,?3,'test',?4,?5,?6,'{}')",params![action,directive,order,assignment,child,message]).unwrap_err().to_string();
        assert!(
            error.contains("ordered_action_no_replace"),
            "{key}: {error}"
        );
        assert_eq!(ordered_exec_receipts(&db, &id), before);
    }
    for column in [
        "plan_hash",
        "previous_action_id",
        "assignment_id",
        "child_run_id",
        "request_message_id",
        "input_json",
    ] {
        let error=db.execute(&format!("UPDATE agent_directive_ordered_actions SET {column}='changed' WHERE directive_id=?1"),[&id]).unwrap_err().to_string();
        assert!(error.contains("ordered_action_frozen"), "{column}: {error}");
        assert_eq!(ordered_exec_receipts(&db, &id), before);
    }
}
