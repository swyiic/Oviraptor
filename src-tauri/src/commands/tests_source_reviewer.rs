#[test]
fn source_reviewer_real_execution_delivers_independent_candidate_review_without_claiming_ci() {
    use crate::agent_runtime::multi_agent::source_reviewer;
    let (root,connection,lease,result)=source_reviewer_execution_fixture("valid",None);
    let result=result.unwrap();
    assert_eq!(result["status"],"source_candidates_reviewed_coverage_open");
    assert_eq!(result["independentCandidateReviewCompleted"],true);
    assert_eq!(result["independentReviewCompleted"],false);
    assert_eq!(result["modelRequests"],7);
    assert_eq!(result["totalTokens"],140);
    assert_eq!(result["sourceDecisionProjection"]["counts"]["insufficient"],2);
    assert_eq!(result["sourceDecisionProjection"]["findings"],json!([]));
    assert_eq!(result["confirmedFindings"],0);
    let tx=rusqlite::Transaction::new_unchecked(&connection,rusqlite::TransactionBehavior::Immediate).unwrap();
    let audit=source_reviewer::audit_delivery(&tx,&lease).unwrap();
    assert_ne!(audit.child.run_id,lease.root_run_id);
    assert_eq!(audit.payload,result["candidateReview"]);
    assert!(audit.payload["result"]["decisions"].as_array().unwrap().iter().all(|d|d["verdict"]=="insufficient_evidence"));
    tx.rollback().unwrap();
    let budget:(i64,i64,i64,i64,i64)=connection.query_row("SELECT total_requests,spent_requests,spent_tokens,reserved_requests,reserved_tokens FROM agent_budget_ledger WHERE root_run_id=?1",
        [&lease.root_run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
    assert_eq!(budget,(9,7,140,0,0));
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_review_decisions WHERE root_run_id=?1",[&lease.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),0,
        "source identities must not masquerade as numeric Web revisions");
    let (trace,events)=collect_native_agent_trace(&connection,&lease.scan_id,true,true).unwrap();
    assert_eq!((trace.llm_requests,trace.total_tokens),(7,140));
    let message=events.iter().find(|event|event.event_type=="source_review_result").unwrap();
    assert_eq!(message.session_id,audit.child.run_id);
    assert_eq!(message.role,"evidence_reviewer");
    assert_eq!(message.status,"acknowledged");
    for cursor in [None,Some(0)] {
        let status=native_scan_status_after(&connection,&lease.scan_id,cursor).unwrap();
        let chat=status["timeline"].as_array().unwrap().iter().find(|event|event["id"]==audit.message_id).unwrap();
        assert_eq!(chat["fromRunId"],audit.child.run_id);
        assert_eq!(chat["fromRole"],"evidence_reviewer");
        assert_eq!(chat["messageKind"],"source_review_result");
        assert_eq!(chat["ackState"],"acknowledged");
        assert!(chat["sequence"].as_i64().is_some_and(|n|n>0));
        assert_eq!(chat["summary"],audit.payload["summary"]);
    }
    let text:String=connection.query_row("SELECT payload_json FROM agent_events WHERE run_id=?1 AND event_type='terminal_reduced'",[&lease.root_run_id],|r|r.get(0)).unwrap();
    let event:JsonValue=serde_json::from_str(&text).unwrap();
    assert_eq!(event["candidateReview"],result["candidateReview"]);
    assert_eq!(event["independentCandidateReviewCompleted"],true);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_reviewer_consumer_projects_only_confirmed_with_typed_identity_and_exact_attempt() {
    use crate::agent_runtime::multi_agent::source_decisions;
    for mode in ["confirmed","rejected","graph_confirmed"] {
        let (root,connection,lease,result)=source_reviewer_execution_fixture(mode,None);
        let result=result.unwrap();
        assert!(source_decisions::read_audited(&connection,&lease).is_err(),"consumer requires a stable transaction");
        let tx=connection.unchecked_transaction().unwrap();
        let changes=tx.total_changes();
        let set=source_decisions::read_audited(&tx,&lease).unwrap();
        let projection=set.as_json();
        let count=usize::from(mode!="rejected");
        assert_eq!(set.confirmed_count(),count);
        assert_eq!(projection["scanId"],lease.scan_id);
        assert_eq!(projection["attemptNumber"],lease.attempt_number);
        assert_eq!(projection["rootRunId"],lease.root_run_id);
        assert_eq!(projection["counts"]["decisions"],2);
        assert_eq!(projection["counts"]["insufficient"],1);
        assert_eq!(projection["counts"]["rejected"],usize::from(mode=="rejected"));
        assert_eq!(projection,result["sourceDecisionProjection"]);
        assert_eq!(result["confirmedFindings"],count);
        assert_eq!(projection["independentReviewCompleted"],false,"candidate review cannot certify coverage");
        assert_eq!(projection["findings"].as_array().unwrap().len(),count);
        if count==1 {
            let row=&projection["findings"][0];
            if mode=="graph_confirmed" {
                assert_eq!(row["identity"]["kind"],"graph_candidate");
                assert!(row["identity"]["nodeId"].as_str().unwrap().starts_with("cand-"));
                assert_eq!(row["identity"]["revision"],1);
            } else {
                assert_eq!(row["identity"]["kind"],"analyzer_revision");
                assert!(row["identity"]["revisionHash"].as_str().unwrap().starts_with("sha256:"));
            }
            assert!(row.get("decisionRevision").is_none(),"source hashes must not become Web revisions");
            assert_eq!(row["path"],"app.py");
            assert_eq!(row["severity"],"high");
            assert_eq!(row["reviewState"],"confirmed");
            assert_ne!(row["reviewerRunId"],lease.root_run_id);
            assert!(projection["decisions"].as_array().unwrap().iter().any(|d|d["id"]==row["sourceDecisionId"]));
        }
        let event:String=tx.query_row("SELECT payload_json FROM agent_events WHERE run_id=?1 AND event_type='terminal_reduced'",[&lease.root_run_id],|r|r.get(0)).unwrap();
        let event:JsonValue=serde_json::from_str(&event).unwrap();
        assert_eq!(event["sourceDecisionProjection"],projection);
        let mut foreign=lease.clone();foreign.attempt_number+=1;
        assert!(source_decisions::read_audited(&tx,&foreign).is_err());
        assert_eq!(tx.total_changes(),changes,"projection never repairs or backfills");
        tx.rollback().unwrap();
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_reviewer_ci_projection_enforces_task_threshold_without_hiding_coverage() {
    for (mode,block,status,exit) in [("confirmed",true,"blocked",2),("confirmed",false,"coverage_incomplete",3),
        ("rejected",true,"coverage_incomplete",3),("valid",true,"coverage_incomplete",3)] {
        let (root,connection,lease,result)=source_reviewer_execution_fixture_policy(mode,None,3,
            Some(json!({"maxCritical":0,"maxHigh":0,"blockRelease":block})));
        let result=result.unwrap();
        let gate=&result["gate"];
        assert_eq!(gate["status"],status,"{gate}");
        assert_eq!(gate["exitCode"],exit);
        assert_eq!(gate["scanId"],lease.scan_id);
        assert_eq!(gate["attemptNumber"],1);
        assert_eq!(gate["rootRunId"],lease.root_run_id);
        assert_eq!(gate["counts"]["blocking"],usize::from(mode=="confirmed"));
        assert_eq!(gate["sourceDecisionProjection"],result["sourceDecisionProjection"]);
        assert_eq!(gate["findings"],result["sourceDecisionProjection"]["findings"]);
        assert_eq!(gate["independentReviewCompleted"],false);
        let gaps=gate["gaps"].as_array().unwrap();
        assert!(gaps.contains(&json!("source_coverage_review")));
        assert!(gaps.contains(&json!("source_candidate_evidence_incomplete")));
        assert!(!gaps.contains(&json!("source_review_not_completed")));
        let saved:(String,String)=connection.query_row("SELECT gate_status,gate_reason FROM sentinel_scan_contexts WHERE scan_id=?1",
            [&lease.scan_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(saved.0,status);
        assert_eq!(saved.1,gate["reasons"].as_array().unwrap().iter().map(|r|r.as_str().unwrap()).collect::<Vec<_>>().join("; "));
        let text:String=connection.query_row("SELECT payload_json FROM agent_events WHERE run_id=?1 AND event_type='terminal_reduced'",[&lease.root_run_id],|r|r.get(0)).unwrap();
        assert_eq!(serde_json::from_str::<JsonValue>(&text).unwrap()["gate"],*gate);
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_reviewer_ci_projection_write_faults_roll_back_gate_without_replaying_model() {
    for fault in [
        "CREATE TRIGGER gate_fault BEFORE UPDATE OF gate_status ON sentinel_scan_contexts WHEN NEW.gate_status='blocked' BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER gate_fault AFTER UPDATE OF gate_status ON sentinel_scan_contexts WHEN NEW.gate_status='blocked' BEGIN UPDATE agent_messages SET acknowledged_at='' WHERE kind='source_review_result'; END;",
        "CREATE TRIGGER gate_fault AFTER UPDATE OF gate_status ON sentinel_scan_contexts WHEN NEW.gate_status='blocked' BEGIN UPDATE sentinel_scans SET status='paused' WHERE id=NEW.scan_id; END;",
        "CREATE TRIGGER gate_fault AFTER INSERT ON agent_events WHEN NEW.event_type='terminal_reduced' AND json_extract(NEW.payload_json,'$.gate.status')='blocked' BEGIN UPDATE sentinel_scan_contexts SET gate_status='passed'; END;",
        "DROP TRIGGER source_ci_policy_no_update; CREATE TRIGGER gate_fault AFTER UPDATE OF gate_status ON sentinel_scan_contexts WHEN NEW.gate_status='blocked' BEGIN UPDATE source_ci_policies SET max_high=9999; END;",
    ] {
        // Install terminal-only faults after the actual seven SDK deliveries.
        // SQLite authorizers inspect trigger writes even when WHEN is false;
        // an event trigger installed earlier would fail the first paid receipt.
        let (root,connection,lease,result)=source_reviewer_execution_fixture_using_calls("confirmed",None,3,
            Some(json!({"maxCritical":0,"maxHigh":0,"blockRelease":true})),7,|root,connection,record,lease| {
                source_reviewer_checkpoint_fixture(root,connection,record,lease,"delivered");
                connection.execute_batch(fault).unwrap();
                run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001"))
            });
        assert!(result.is_err(),"accepted gate fault: {fault}: {result:?}");
        assert_eq!(connection.query_row("SELECT gate_status FROM sentinel_scan_contexts WHERE scan_id=?1",[&lease.scan_id],|r|r.get::<_,String>(0)).unwrap(),"coverage_incomplete");
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_events WHERE run_id=?1 AND event_type='terminal_reduced'",[&lease.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),0);
        let tx=connection.unchecked_transaction().unwrap();
        let set=crate::agent_runtime::multi_agent::source_decisions::read_audited(&tx,&lease).expect("failed gate must retain delivered review");
        assert_eq!(set.confirmed_count(),1);
        assert_eq!(tx.query_row("SELECT spent_requests,reserved_requests FROM agent_budget_ledger WHERE root_run_id=?1",[&lease.root_run_id],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?))).unwrap(),(7,0));
        tx.rollback().unwrap();
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_reviewer_actual_bad_responses_preserve_receipt_and_never_publish_or_refund() {
    for mode in ["missing","foreign","fabricated"] {
        let (root,connection,lease,result)=source_reviewer_execution_fixture(mode,None);
        assert!(result.is_err(),"{mode}: {result:?}");
        let state:(String,String,i64,i64)=connection.query_row("SELECT c.state,a.state,a.reserved_requests,r.used_requests FROM agent_specialist_calls c JOIN agent_assignments a ON a.id=c.assignment_id JOIN agent_runs r ON r.id=a.child_run_id WHERE a.role='evidence_reviewer'",
            [],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(state,("received".into(),"paused".into(),1,0),"{mode}");
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_messages WHERE kind='source_review_result'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_capability_leases WHERE root_run_id=?1 AND revoked_at=''",[&lease.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),0);
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_reviewer_delivery_faults_roll_back_mailbox_settlement_and_completion() {
    for fault in [
        "CREATE TRIGGER review_fault BEFORE INSERT ON agent_messages WHEN NEW.kind='source_review_result' BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER review_fault AFTER UPDATE OF acknowledged_at ON agent_messages WHEN NEW.kind='source_review_result' BEGIN UPDATE agent_messages SET payload_json='{}' WHERE id=NEW.id; END;",
        "CREATE TRIGGER review_fault AFTER UPDATE ON agent_runs WHEN NEW.role='evidence_reviewer' AND NEW.status='terminal' BEGIN UPDATE agent_runs SET used_requests=999 WHERE id=NEW.id; END;",
        "CREATE TRIGGER review_fault BEFORE INSERT ON agent_collaboration_events WHEN NEW.event_type='mailbox_message' AND json_extract(NEW.payload_json,'$.kind')='source_review_result' BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER review_fault BEFORE INSERT ON agent_collaboration_events WHEN NEW.event_type='assignment' AND json_extract(NEW.payload_json,'$.role')='evidence_reviewer' AND json_extract(NEW.payload_json,'$.state')='completed' BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER review_fault BEFORE INSERT ON agent_collaboration_events WHEN NEW.event_type='agent_run' AND json_extract(NEW.payload_json,'$.role')='evidence_reviewer' AND json_extract(NEW.payload_json,'$.status')='terminal' BEGIN SELECT RAISE(IGNORE); END;",
    ] {
        let (root,connection,lease,result)=source_reviewer_execution_fixture("valid",Some(fault));
        assert!(result.is_err(),"{fault}: {result:?}");
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_messages WHERE kind='source_review_result'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        let budget:(i64,i64,i64)=connection.query_row("SELECT spent_requests,reserved_requests,spent_tokens FROM agent_budget_ledger WHERE root_run_id=?1",
            [&lease.root_run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(budget,(6,1,120));
        let state:String=connection.query_row("SELECT state FROM agent_specialist_calls WHERE role='evidence_reviewer'",[],|r|r.get(0)).unwrap();
        assert_eq!(state,"received");
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_reviewer_terminal_proof_rejects_changed_receipt_material_and_ack() {
    use crate::agent_runtime::multi_agent::source_reviewer;
    let (root,connection,lease,result)=source_reviewer_execution_fixture("valid",None);
    assert!(result.is_ok(),"{result:?}");
    for mutation in [
        "UPDATE agent_messages SET acknowledged_at='' WHERE kind='source_review_result';",
        "UPDATE agent_messages SET from_run_id=to_run_id WHERE kind='source_review_result';",
        "UPDATE agent_specialist_calls SET response_json='{}' WHERE role='evidence_reviewer';",
        "UPDATE agent_assignments SET task_slice_json=json_set(task_slice_json,'$.reviewMaterial.digest','changed') WHERE role='evidence_reviewer';",
        "UPDATE agent_runs SET used_requests=9 WHERE role='evidence_reviewer';",
        "UPDATE agent_assignments SET state='paused' WHERE role='repo_mapper' AND json_extract(task_slice_json,'$.phase') IS NULL;",
    ] {
        let tx=rusqlite::Transaction::new_unchecked(&connection,rusqlite::TransactionBehavior::Immediate).unwrap();
        if mutation.starts_with("UPDATE agent_specialist_calls") {
            // First prove the production guard; then simulate storage damage
            // inside this rolled-back transaction to test the independent audit.
            assert!(tx.execute_batch(mutation).is_err());
            tx.execute_batch("DROP TRIGGER agent_specialist_call_immutable;").unwrap();
        }
        tx.execute_batch(mutation).unwrap();
        assert!(source_reviewer::audit_delivery(&tx,&lease).is_err(),"accepted {mutation}");
        tx.rollback().unwrap();
    }
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_reviewer_local_delivery_recovers_saved_response_without_model_replay() {
    let fault="CREATE TRIGGER review_fault BEFORE INSERT ON agent_messages
        WHEN NEW.kind='source_review_result' AND EXISTS(SELECT 1 FROM agent_assignments WHERE id=NEW.assignment_id AND state='running')
        BEGIN SELECT RAISE(IGNORE); END;";
    let (root,connection,lease,result)=source_reviewer_execution_fixture("valid",Some(fault));
    let result=result.unwrap();
    assert_eq!(result["modelRequests"],7);
    let state:(String,String,i64)=connection.query_row("SELECT a.state,r.terminal_code,r.used_requests FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE a.role='evidence_reviewer'",
        [],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(state,("completed".into(),"source_review_receipt_reconciled".into(),1));
    let tx=connection.unchecked_transaction().unwrap();
    let audit=crate::agent_runtime::multi_agent::source_reviewer::audit_delivery(&tx,&lease).unwrap();
    assert_eq!(audit.payload,result["candidateReview"]);
    tx.rollback().unwrap();
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_messages WHERE kind='source_review_result' AND acknowledged_at<>''",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_reviewer_dispatch_rechecks_both_permissions_after_persisting_claim() {
    let (root,connection,_,result)=source_reviewer_execution_fixture_using_calls("confirmed",None,3,None,6,|root,connection,record,lease| {
        source_reviewer_checkpoint_fixture(root,connection,record,lease,"dispatch_revoked");
        Err("expected_dispatch_refusal".into())
    });
    assert_eq!(result.unwrap_err(),"expected_dispatch_refusal");
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_reviewer_root_final_write_cannot_rewrite_the_delivered_review() {
    let fault="CREATE TRIGGER review_fault AFTER UPDATE ON agent_runs
        WHEN NEW.role='coordinator' AND NEW.terminal_state='completed_with_gaps' AND OLD.status<>'terminal'
        BEGIN UPDATE agent_messages SET payload_json='{}' WHERE kind='source_review_result'; END;";
    let (root,connection,lease,result)=source_reviewer_execution_fixture("valid",Some(fault));
    assert!(result.is_err(),"{result:?}");
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_events WHERE run_id=?1 AND event_type='terminal_reduced'",[&lease.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),0);
    let tx=connection.unchecked_transaction().unwrap();
    crate::agent_runtime::multi_agent::source_reviewer::audit_delivery(&tx,&lease).expect("failed root transaction must roll back review damage");
    tx.rollback().unwrap();
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_reviewer_v3_cannot_skip_initial_assessments_or_grant_web_tools() {
    use crate::agent_runtime::{contract::{AgentRole,AgentLane},multi_agent::{scheduler,lease}};
    let (root,connection,record)=source_coordinator_fixture();
    let coordinator=prepare_native_source_coordinator_schema(&connection,&record.scan_id,1,&root.join("attempt-0001"),3).unwrap();
    let lease=lease::acquire_coordinator_lease(&connection,&record.scan_id,1,&coordinator.target_key,&coordinator.run_id,600).unwrap();
    for caps in [vec!["evidence.read".into(),"review.write".into()],vec!["http.request".into()]] {
        assert!(scheduler::schedule_child(&connection,&lease,AgentRole::EvidenceReviewer,AgentLane::Review,
            "source_candidates_ready",&json!({"surface":"source","phase":"source_review"}),1,&caps,100,1).is_err());
    }
    for table in ["agent_assignments","agent_specialist_calls","agent_lane_leases","agent_capability_leases"] {
        assert_eq!(connection.query_row(&format!("SELECT count(*) FROM {table}"),[],|r|r.get::<_,i64>(0)).unwrap(),0);
    }
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_reviewer_prerequisite_proof_rejects_phase_mailbox_usage_and_binding_damage() {
    use crate::agent_runtime::multi_agent::source_reviewer;
    let (root,connection,lease,result)=source_reviewer_execution_fixture("valid",None);
    assert!(result.is_ok(),"{result:?}");
    let mut mutations=vec![
        "UPDATE agent_messages SET acknowledged_at='' WHERE kind='evidence_summary';",
        "UPDATE agent_messages SET acknowledged_at='' WHERE kind='source_tool_result';",
        "UPDATE agent_messages SET delivery_attempts=2 WHERE kind='evidence_summary';",
        "UPDATE agent_messages SET from_run_id=to_run_id WHERE kind='source_tool_result';",
        "UPDATE agent_runs SET used_tokens=used_tokens+1 WHERE role='repo_mapper';",
        "UPDATE agent_runs SET used_cached_tokens=used_cached_tokens+1 WHERE role='source_analyst';",
        "UPDATE agent_runs SET attempt_number=attempt_number+1 WHERE role='repo_mapper';",
        "UPDATE agent_assignments SET fencing_token=fencing_token+1 WHERE role='source_analyst';",
        "UPDATE agent_assignments SET task_slice_json=json_set(task_slice_json,'$.targetRequestsGranted',1) WHERE role='repo_mapper';",
        "UPDATE agent_capability_leases SET revoked_at='' WHERE child_run_id IN (SELECT id FROM agent_runs WHERE role='repo_mapper');",
    ].into_iter().map(str::to_string).collect::<Vec<_>>();
    for phase in ["json_extract(task_slice_json,'$.phase') IS NULL","json_extract(task_slice_json,'$.phase')='source_tools'"] {
        for role in ["repo_mapper","source_analyst"] {
            for change in ["used_requests=used_requests+1","used_tokens=used_tokens+1","parent_run_id=id","lane='review'","cancel_requested_at='cancelled'"] {
                mutations.push(format!("UPDATE agent_runs SET {change} WHERE id IN (SELECT child_run_id FROM agent_assignments WHERE role='{role}' AND {phase});"));
            }
        }
    }
    let mut admitted=Vec::new();
    for mutation in mutations {
        let tx=rusqlite::Transaction::new_unchecked(&connection,rusqlite::TransactionBehavior::Immediate).unwrap();
        tx.execute_batch(&mutation).unwrap();
        let changes:i64=tx.query_row("SELECT total_changes()",[],|r|r.get(0)).unwrap();
        for (entry,rejected) in [
            ("task_slice",source_reviewer::task_slice(&tx,&lease).is_err()),
            ("audit_delivery",source_reviewer::audit_delivery(&tx,&lease).is_err()),
            ("decision_consumer",crate::agent_runtime::multi_agent::source_decisions::read_audited(&tx,&lease).is_err()),
        ] {
            if !rejected {admitted.push(format!("{entry}: {mutation}"));}
        }
        assert_eq!(tx.query_row("SELECT total_changes()",[],|r|r.get::<_,i64>(0)).unwrap(),changes,"audits must never repair damage");
        tx.rollback().unwrap();
    }
    drop(connection);fs::remove_dir_all(root).unwrap();
    assert!(admitted.is_empty(),"prerequisite damage accepted: {admitted:#?}");
}

#[test]
fn source_reviewer_historical_proof_survives_scan_completion_without_execution_grants() {
    use crate::agent_runtime::{contract::{AgentRole,AgentLane},multi_agent::{source,source_reviewer,scheduler}};
    let (root,connection,lease,result)=source_reviewer_execution_fixture("valid",None);
    let result=result.unwrap();
    let expected={
        let tx=rusqlite::Transaction::new_unchecked(&connection,rusqlite::TransactionBehavior::Immediate).unwrap();
        source_reviewer::audit_delivery(&tx,&lease).unwrap()
    };
    let original:PathBuf=connection.query_row("SELECT source_path FROM source_scope_contracts WHERE scan_id=?1 AND attempt_number=1",
        [&lease.scan_id],|r|r.get::<_,String>(0)).unwrap().into();
    assert!(original.starts_with(&root) && original!=root);
    fs::rename(&original,root.join("moved-original-repository")).unwrap();
    for previous_attempt in [false,true] {
        let tx=rusqlite::Transaction::new_unchecked(&connection,rusqlite::TransactionBehavior::Immediate).unwrap();
        tx.execute("UPDATE sentinel_scan_attempts SET status='completed',finished_at='completed' WHERE scan_id=?1 AND attempt_number=1",[&lease.scan_id]).unwrap();
        tx.execute("UPDATE sentinel_scans SET status='completed' WHERE id=?1",[&lease.scan_id]).unwrap();
        if previous_attempt {
            tx.execute("UPDATE sentinel_scans SET status='scanning',attempt_count=2,source_path='/unrelated/new-attempt' WHERE id=?1",[&lease.scan_id]).unwrap();
            tx.execute("INSERT INTO sentinel_scan_attempts(scan_id,attempt_number,status) VALUES(?1,2,'scanning')",[&lease.scan_id]).unwrap();
        }
        tx.commit().unwrap();
        let tx=rusqlite::Transaction::new_unchecked(&connection,rusqlite::TransactionBehavior::Immediate).unwrap();
        let changes:i64=tx.query_row("SELECT total_changes()",[],|r|r.get(0)).unwrap();
        assert_eq!(source_reviewer::audit_delivery(&tx,&lease).unwrap(),expected);
        let set=crate::agent_runtime::multi_agent::source_decisions::read_audited(&tx,&lease).unwrap();
        assert_eq!(set.as_json(),result["sourceDecisionProjection"]);
        let bases=source::completion_task_slices(&tx,&lease).unwrap();
        let proof=source_assessment_completion(&tx,&lease,&bases).unwrap();
        assert!(source_completion_gate(&tx,&lease,&proof).is_err(),"historical decision reads must not update a current gate");
        assert!(source::task_slice(&tx,&lease,AgentRole::RepoMapper).is_err());
        assert_eq!(tx.query_row("SELECT total_changes()",[],|r|r.get::<_,i64>(0)).unwrap(),changes);
        tx.rollback().unwrap();
        assert!(scheduler::schedule_child(&connection,&lease,AgentRole::EvidenceReviewer,AgentLane::Review,
            "source_candidates_ready",&expected.payload["sourceTask"],1,&["evidence.read".into(),"review.write".into()],100,1).is_err());
        assert!(source_reviewer::prepare(&connection,&lease,&expected.payload["sourceTask"],100).is_err());
        assert_eq!(connection.query_row("SELECT total_changes()",[],|r|r.get::<_,i64>(0)).unwrap(),changes);
    }
    let (view,_,_)=source::historical_materials(&connection,&lease.scan_id,1).unwrap();
    let frozen_file=view.repository().join("app.py");
    let retained=root.join("retained-frozen-file");
    fs::rename(&frozen_file,&retained).unwrap();
    {
        let tx=rusqlite::Transaction::new_unchecked(&connection,rusqlite::TransactionBehavior::Immediate).unwrap();
        assert!(source_reviewer::audit_delivery(&tx,&lease).is_err(),"historical read must still verify frozen bytes");
        assert!(crate::agent_runtime::multi_agent::source_decisions::read_audited(&tx,&lease).is_err());
    }
    fs::rename(&retained,&frozen_file).unwrap();
    {
        let tx=rusqlite::Transaction::new_unchecked(&connection,rusqlite::TransactionBehavior::Immediate).unwrap();
        assert_eq!(source_reviewer::audit_delivery(&tx,&lease).unwrap(),expected);
        let mut foreign=lease.clone();foreign.attempt_number=2;
        assert!(source_reviewer::audit_delivery(&tx,&foreign).is_err(),"old evidence must not become new-attempt evidence");
    }
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_reviewer_decisions_bind_exact_receipt_and_replay_without_writes() {
    use crate::agent_runtime::multi_agent::{source_reviewer,source_decisions};
    let (root,connection,lease,result)=source_reviewer_execution_fixture("valid",None);
    let result=result.unwrap();
    let tx=connection.unchecked_transaction().unwrap();
    let audit=source_reviewer::audit_delivery(&tx,&lease).unwrap();
    assert_eq!(audit.decision_ids.len(),audit.payload["result"]["decisions"].as_array().unwrap().len());
    assert!(audit.decision_ids.len()>=2,"exercise both analyzer and graph identities");
    let rows=tx.prepare("SELECT record_json,record_digest FROM agent_source_review_decisions WHERE root_run_id=?1 ORDER BY id").unwrap()
        .query_map([&lease.root_run_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    let mut kinds=std::collections::BTreeSet::new();
    for (text,digest) in &rows {
        assert_eq!(crate::artifact_import::canonical::sha256_hex(text.as_bytes()),*digest);
        let record:JsonValue=serde_json::from_str(text).unwrap();
        assert_eq!(record["rootRunId"],lease.root_run_id);
        assert_eq!(record["scanId"],lease.scan_id);
        assert_eq!(record["attemptNumber"],lease.attempt_number);
        assert_eq!(record["reviewerRunId"],audit.child.run_id);
        assert_eq!(record["assignmentId"],audit.child.assignment_id);
        assert_eq!(record["messageId"],audit.message_id);
        assert_eq!(record["materialDigest"],audit.payload["result"]["materialDigest"]);
        assert!(audit.payload["result"]["decisions"].as_array().unwrap().contains(&record["decision"]));
        kinds.insert(record["decision"]["identity"]["kind"].as_str().unwrap().to_string());
        let hashes:(String,String)=tx.query_row("SELECT request_hash,response_hash FROM agent_specialist_calls WHERE assignment_id=?1",[&audit.child.assignment_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(record["modelRequestHash"],hashes.0);
        assert_eq!(record["modelResponseHash"],hashes.1);
    }
    assert_eq!(kinds,std::collections::BTreeSet::from(["analyzer_revision".into(),"graph_candidate".into()]));
    let changes=tx.total_changes();
    source_decisions::publish(&tx,&lease).unwrap();
    assert_eq!(tx.total_changes(),changes);
    assert_eq!(source_reviewer::audit_delivery(&tx,&lease).unwrap(),audit);
    let event:String=tx.query_row("SELECT payload_json FROM agent_events WHERE run_id=?1 AND event_type='terminal_reduced'",[&lease.root_run_id],|r|r.get(0)).unwrap();
    let event:JsonValue=serde_json::from_str(&event).unwrap();
    assert_eq!(event["assignments"].as_array().unwrap().last().unwrap()["sourceDecisionIds"],json!(audit.decision_ids));
    assert_eq!(result["independentReviewCompleted"],false,"canonical candidate decisions alone do not close coverage or CI");
    tx.rollback().unwrap();
    assert!(source_decisions::publish(&connection,&lease).is_err(),"publication requires a transaction");
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_reviewer_decisions_immutable_and_damage_never_silently_repaired() {
    use crate::agent_runtime::multi_agent::{source_reviewer,source_decisions};
    let (root,connection,lease,result)=source_reviewer_execution_fixture("valid",None);
    result.unwrap();
    for mutation in [
        "UPDATE agent_source_review_decisions SET record_json='{}';",
        "UPDATE agent_source_review_decisions SET scan_id='foreign';",
        "UPDATE agent_source_review_decisions SET attempt_number=2;",
        "UPDATE agent_source_review_decisions SET candidate_identity_json='{}' WHERE id=(SELECT min(id) FROM agent_source_review_decisions);",
        "UPDATE agent_source_review_decisions SET material_digest=printf('%064d',1);",
        "UPDATE agent_source_review_decisions SET record_digest=printf('%064d',1);",
        "UPDATE agent_source_review_decisions SET record_json=json_set(record_json,'$.rootRunId','foreign');",
        "UPDATE agent_source_review_decisions SET record_json=json_set(record_json,'$.reviewerRunId',root_run_id);",
        "UPDATE agent_source_review_decisions SET record_json=json_set(record_json,'$.assignmentId','foreign');",
        "UPDATE agent_source_review_decisions SET record_json=json_set(record_json,'$.messageId','foreign');",
        "UPDATE agent_source_review_decisions SET record_json=json_set(record_json,'$.modelRequestHash','foreign');",
        "UPDATE agent_source_review_decisions SET record_json=json_set(record_json,'$.modelResponseHash','foreign');",
        "UPDATE agent_source_review_decisions SET record_json=json_set(record_json,'$.decision.verdict','confirmed');",
        "DELETE FROM agent_source_review_decisions WHERE id=(SELECT min(id) FROM agent_source_review_decisions);",
        "DELETE FROM agent_source_review_decisions;",
        "INSERT OR REPLACE INTO agent_source_review_decisions SELECT * FROM agent_source_review_decisions;",
    ] {
        let tx=connection.unchecked_transaction().unwrap();
        let previously_audited=crate::agent_runtime::multi_agent::source_review_projection::read_audited(&tx,&lease).unwrap();
        assert!(tx.execute_batch(mutation).is_err(),"guard accepted {mutation}");
        tx.execute_batch("DROP TRIGGER source_decision_no_update; DROP TRIGGER source_decision_no_delete; DROP TRIGGER source_decision_no_replace;").unwrap();
        tx.execute_batch(mutation).unwrap();
        if !mutation.starts_with("INSERT OR REPLACE") {
            assert!(source_reviewer::audit_delivery(&tx,&lease).is_err(),"audit accepted {mutation}");
            let changes=tx.total_changes();
            assert!(source_decisions::read_audited(&tx,&lease).is_err(),"consumer accepted {mutation}");
            assert!(crate::native_pipeline::source_ci::evaluate(&tx,&lease,&previously_audited,
                crate::native_pipeline::ci::GatePolicy::default()).is_err(),"CI trusted a stale set after {mutation}");
            assert!(source_decisions::publish(&tx,&lease).is_err(),"publisher repaired {mutation}");
            assert_eq!(tx.total_changes(),changes);
        }
        tx.rollback().unwrap();
    }
    // A later extra row must invalidate both fresh reads and previously audited
    // opaque values. No trigger needs disabling to simulate an extra INSERT.
    let tx=connection.unchecked_transaction().unwrap();
    let previously_audited=crate::agent_runtime::multi_agent::source_review_projection::read_audited(&tx,&lease).unwrap();
    assert!(crate::native_pipeline::source_ci::evaluate(&tx,&lease,&previously_audited,
        crate::native_pipeline::ci::GatePolicy::default()).is_ok());
    tx.execute_batch("INSERT INTO agent_source_review_decisions(id,root_run_id,scan_id,attempt_number,candidate_identity_json,material_digest,record_json,record_digest)
        SELECT 'extra-after-publication',root_run_id,scan_id,attempt_number,'{}',material_digest,'{}',record_digest
        FROM agent_source_review_decisions LIMIT 1;").unwrap();
    let changes=tx.total_changes();
    assert!(source_decisions::read_audited(&tx,&lease).is_err());
    assert!(crate::native_pipeline::source_ci::evaluate(&tx,&lease,&previously_audited,
        crate::native_pipeline::ci::GatePolicy::default()).is_err());
    assert!(source_decisions::publish(&tx,&lease).is_err());
    assert_eq!(tx.total_changes(),changes,"consumers cannot repair an extra canonical row");
    tx.rollback().unwrap();
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_reviewer_decision_write_faults_preserve_usage_and_rollback_complete_set() {
    for fault in [
        "CREATE TRIGGER decision_fault BEFORE INSERT ON agent_source_review_decisions BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER decision_fault BEFORE INSERT ON agent_source_review_decisions WHEN EXISTS(SELECT 1 FROM agent_source_review_decisions) BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER decision_fault AFTER INSERT ON agent_source_review_decisions BEGIN UPDATE agent_messages SET acknowledged_at='' WHERE kind='source_review_result'; END;",
        "CREATE TRIGGER decision_fault AFTER INSERT ON agent_source_review_decisions BEGIN UPDATE agent_runs SET used_requests=999 WHERE role='evidence_reviewer'; END;",
        "CREATE TRIGGER decision_fault AFTER INSERT ON agent_source_review_decisions BEGIN UPDATE sentinel_scans SET status='paused' WHERE id=NEW.scan_id; END;",
        "CREATE TRIGGER decision_fault AFTER INSERT ON agent_source_review_decisions WHEN (SELECT count(*) FROM agent_source_review_decisions)=1 BEGIN
            INSERT INTO agent_source_review_decisions(id,root_run_id,scan_id,attempt_number,candidate_identity_json,material_digest,record_json,record_digest)
            VALUES('extra',NEW.root_run_id,NEW.scan_id,NEW.attempt_number,'{}',NEW.material_digest,'{}',NEW.record_digest); END;",
    ] {
        let (root,connection,lease,result)=source_reviewer_execution_fixture("valid",Some(fault));
        assert!(result.is_err(),"{fault}: {result:?}");
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_source_review_decisions",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_messages WHERE kind='source_review_result'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        let budget:(i64,i64,i64)=connection.query_row("SELECT spent_requests,reserved_requests,spent_tokens FROM agent_budget_ledger WHERE root_run_id=?1",
            [&lease.root_run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(budget,(6,1,120));
        assert_eq!(connection.query_row("SELECT state FROM agent_specialist_calls WHERE role='evidence_reviewer'",[],|r|r.get::<_,String>(0)).unwrap(),"received");
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_reviewer_historical_v2_has_no_canonical_backfill() {
    use crate::agent_runtime::multi_agent::{source_reviewer,source_decisions};
    let (root,connection,lease,result)=source_reviewer_execution_fixture_schema("valid",None,2);
    let result=result.unwrap();
    assert!(result["sourceDecisionProjection"].is_null());
    assert!(result["gate"].is_null());
    let tx=connection.unchecked_transaction().unwrap();
    let audit=source_reviewer::audit_delivery(&tx,&lease).unwrap();
    assert!(audit.decision_ids.is_empty());
    assert!(source_decisions::read_audited(&tx,&lease).is_err(),"v2 receipts must not be upgraded by a reader");
    let changes=tx.total_changes();
    source_decisions::publish(&tx,&lease).unwrap();
    assert_eq!(changes,tx.total_changes());
    assert_eq!(tx.query_row("SELECT count(*) FROM agent_source_review_decisions",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    let plan:String=tx.query_row("SELECT plan_json FROM agent_runs WHERE id=?1",[&lease.root_run_id],|r|r.get(0)).unwrap();
    assert_eq!(serde_json::from_str::<JsonValue>(&plan).unwrap()["schemaVersion"],2);
    tx.rollback().unwrap();
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_reviewer_decisions_survive_reinitialize_and_allow_scan_cascade() {
    use crate::agent_runtime::multi_agent::source_reviewer;
    let (root,connection,lease,result)=source_reviewer_execution_fixture("valid",None);
    result.unwrap();
    let tx=connection.unchecked_transaction().unwrap();
    let before=source_reviewer::audit_delivery(&tx,&lease).unwrap();
    tx.rollback().unwrap();
    drop(connection);
    let path=crate::db::initialize(&root).unwrap();
    let connection=crate::db::open(&path).unwrap();
    let tx=connection.unchecked_transaction().unwrap();
    assert_eq!(source_reviewer::audit_delivery(&tx,&lease).unwrap(),before);
    tx.execute("DELETE FROM sentinel_scans WHERE id=?1",[&lease.scan_id]).unwrap();
    assert_eq!(tx.query_row("SELECT count(*) FROM agent_source_review_decisions",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(tx.query_row("SELECT count(*) FROM agent_runs WHERE id=?1",[&lease.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),0);
    tx.rollback().unwrap();
    drop(connection);fs::remove_dir_all(root).unwrap();
}
