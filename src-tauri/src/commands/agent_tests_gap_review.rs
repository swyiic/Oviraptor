struct GapReviewFixture {
    source: FollowupFixture,
    context: AgentRunContext,
    session: MultiAgentSession,
}

#[test]
fn gap_review_history_does_not_recursively_embed_execution_guidance() {
    let original = json!({"target":"https://fixture.invalid", "evidence":{"observations":["sealed"]},
        "findingCandidates":[{"id":"original-hypothesis"}]});
    let mut previous = original.clone();
    for index in 0..32 {
        let next = json!({"target":original["target"],"findingCandidates":[],
            "evidence":{"followupObjective":{"historicalCandidate":previous},"observations":["fresh"]},
            "gapFollowup":{"historicalCandidate":previous}});
        let sealed = next.clone();
        previous = gap_review_historical_hypothesis(&next);
        assert_eq!(previous,original,"coverage-only chain {index}");
        assert_eq!(next,sealed,"projection must not mutate sealed source");
    }
    let mut fresh = json!({"target":original["target"],"findingCandidates":[{"id":"fresh-hypothesis"}],
        "evidence":{"observations":["fresh"],"followupObjective":{"historicalCandidate":original}},
        "gapFollowup":{"historicalCandidate":original}});
    let projected = gap_review_historical_hypothesis(&fresh);
    assert_eq!(projected["findingCandidates"],fresh["findingCandidates"]);
    assert_eq!(projected["evidence"],json!({"observations":["fresh"]}));
    // Also sanitize guidance inherited from a previously frozen bundle.
    fresh["findingCandidates"] = json!([]);
    fresh["gapFollowup"]["historicalCandidate"]["evidence"]["followupObjective"] = json!({"nested":"old guidance"});
    assert_eq!(gap_review_historical_hypothesis(&fresh),original);
}

fn gap_review_fixture(with_fact: bool) -> GapReviewFixture {
    use crate::agent_runtime::{contract::{AgentBackendKind,AgentRole,AgentRunStatus,AgentLane,MultiAgentPolicy},store::{self,AgentRunRow}};
    let source = followup_fixture();
    let connection = db::open(&source.context.db_path).unwrap();
    let draft = create_agent_gap_followup_in(&connection,&followup_input(&source)).unwrap();
    connection.execute("UPDATE sentinel_scans SET status='scanning',attempt_count=1 WHERE id=?1",[&draft.id]).unwrap();
    let mut context = test_context(&source.context.db_path,&source.context.target_url,vec![AgentIdentity::anonymous()]);
    context.scan_id = draft.id.clone();
    let root = format!("root-{}",draft.id);
    context.run = Some(AgentRunLedger {db_path:context.db_path.clone(),run_id:root.clone()});
    let attempt = source.root.join("followup-work/attempt-00001");
    context.target_dir = attempt.join("url-pipeline/target-00000");
    fs::create_dir_all(&context.target_dir).unwrap();
    fs::write(context.target_dir.join(".oviraptor-scan-id"),&context.scan_id).unwrap();
    connection.execute("INSERT INTO sentinel_scan_attempts(scan_id,attempt_number,work_dir) VALUES(?1,1,?2)",params![context.scan_id,attempt.to_string_lossy()]).unwrap();
    let plan = test_plan_for("standard",&context.target_url).with_attempt(1);
    let mut run = AgentRunRow::new(&root,&context.scan_id,1,&context.target_url,AgentBackendKind::Native,
        AgentRole::Coordinator,plan.hash(),"evidence").with_budget(30_000,60_000,10,20);
    run.status=AgentRunStatus::Running; run.root_run_id=root;
    run.orchestration_policy=MultiAgentPolicy::Multi; run.lane=Some(AgentLane::ReadOnlyAnalysis);
    store::create_run(&connection,&run).unwrap();
    connection.execute("UPDATE agent_runs SET plan_json=?1 WHERE id=?2",params![plan.as_json().to_string(),run.id]).unwrap();
    bind_agent_evidence_location(&context).unwrap();
    let mut session=multi_agent_prepare(&mut context).unwrap();
    if with_fact {
        let body=b"control denied access";
        let view=json!({"requestId":"req-new-control","status":403,"preview":"control denied access",
            "bodySha256":format!("{:x}",Sha256::digest(body)),"redaction":{"applied":true}});
        let artifact=agent_write_http_record(&context,1,&json!({"method":"GET","url":context.target_url,"identity":"anonymous"}),&view,body).unwrap();
        agent_record_http_observation(&context,"replay_http",&artifact,&view,"req-new-control").unwrap();
    }
    multi_agent_finish_execution(&context,&mut session,&AgentTargetOutcome::incomplete("fixture coverage")).unwrap();
    connection.execute("UPDATE agent_runs SET status='terminal',terminal_state='incomplete' WHERE id=?1",[&source.root_run_id]).unwrap();
    connection.execute("UPDATE sentinel_scans SET status='completed_with_gaps' WHERE id=?1",[&source.context.scan_id]).unwrap();
    connection.execute("UPDATE sentinel_scan_attempts SET status='completed_with_gaps' WHERE scan_id=?1",[&source.context.scan_id]).unwrap();
    GapReviewFixture {source,context,session}
}

fn gap_review_response(candidate: &JsonValue, verdict: &str) -> JsonValue {
    let addressed=verdict!="insufficient_evidence";
    json!({"verdict":verdict,"reasonCodes":["fresh_control"],"missingEvidence":if addressed {json!([])} else {json!(["missing control"])},
        "confidence":0.9,"summary":"Fresh control reviewed; no new finding is fabricated",
        "gapAssessment":{"sourceHash":candidate["gapFollowup"]["source"]["sourceHash"],"hypothesisVerdict":verdict,"items":[{
            "index":0,"missingEvidence":"missing control","status":if addressed {"addressed"} else {"insufficient"},
            "factRefs":if addressed {candidate["persistedFactRefs"].clone()} else {json!([])},
            "reason":"The fresh control supplies the previously missing observation"}]}})
}

fn run_gap_review(f: &mut GapReviewFixture, mutation: &'static str) -> (AgentTargetOutcome,usize) {
    let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(move |request| {
        let body: JsonValue=serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        let candidate: JsonValue=serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
        if candidate.get("gapFollowup").is_none() {
            return (200,"application/json",proposal_model_response(&gap_receipt_response()));
        }
        let mut response=gap_review_response(&candidate,if mutation=="insufficient" {"insufficient_evidence"} else {"rejected"});
        match mutation {
            "old_fact"=>response["gapAssessment"]["items"][0]["factRefs"]=json!(["ev-from-old-root"]),
            "no_assessment"=>{response.as_object_mut().unwrap().remove("gapAssessment");},
            "wrong_source"=>response["gapAssessment"]["sourceHash"]=json!("wrong-source"),
            _=>{},
        }
        (200,"application/json",proposal_model_response(&response.to_string()))
    }));
    f.context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
    let _transport=RealSpecialistTransport::enter();
    let outcome=multi_agent_review(&f.context,&mut f.session,AgentTargetOutcome::incomplete("fixture coverage"));
    let count=seen.lock().unwrap().len();
    (outcome,count)
}

#[test]
fn gap_review_real_independent_receipt_closes_link_without_rewriting_old_root_or_fabricating_findings() {
    let mut f=gap_review_fixture(true);
    assert_eq!(f.context.evidence["followupObjective"]["targetRequestsGranted"],0);
    assert_eq!(f.context.evidence["followupObjective"]["source"]["missingEvidence"],json!(["missing control"]));
    let connection=db::open(&f.context.db_path).unwrap();
    let source_before:String=connection.query_row("SELECT candidate_json FROM agent_review_requests WHERE root_run_id=?1",[&f.source.root_run_id],|r|r.get(0)).unwrap();
    let (outcome,calls)=run_gap_review(&mut f,"valid");
    assert!(outcome.detail().contains("review_gate_rejected"),"{outcome:?}");
    assert_eq!(calls,1);
    let tx=connection.unchecked_transaction().unwrap();
    let status=gap_review_status(&tx,&f.context.scan_id).unwrap();
    assert_eq!(status["status"],"resolved","{status}");
    assert_eq!(status["verdict"],"rejected");
    assert_eq!(status["candidateRevision"],1,"new root has its own revision sequence, not the source's permissions");
    assert_eq!(f.context.evidence["followupObjective"]["source"]["candidateRevision"],2);
    let source=gap_followup_relations(&tx,&f.source.context.scan_id).unwrap();
    assert_eq!(source["tasks"][0]["review"]["gapResolved"],true);
    assert_eq!(source["gapResolved"],false,"one child is not proof for the whole source scan");
    let count:i64=tx.query_row("SELECT COUNT(*) FROM sentinel_findings",[],|r|r.get(0)).unwrap();
    assert_eq!(count,0);
    let source_after:String=tx.query_row("SELECT candidate_json FROM agent_review_requests WHERE root_run_id=?1",[&f.source.root_run_id],|r|r.get(0)).unwrap();
    assert_eq!(source_before,source_after);
    drop(tx);
    let (_,calls)=run_gap_review(&mut f,"valid");
    assert_eq!(calls,0,"frozen review must replay locally");
    connection.execute("UPDATE agent_runs SET status='terminal',terminal_state='incomplete' WHERE id=?1",[&f.session.lease.root_run_id]).unwrap();
    connection.execute("UPDATE sentinel_scans SET status='completed_with_gaps' WHERE id=?1",[&f.context.scan_id]).unwrap();
    let tx=connection.unchecked_transaction().unwrap();
    assert_eq!(gap_review_status(&tx,&f.context.scan_id).unwrap()["gapResolved"],true,"history does not require live permissions");
}

#[test]
fn gap_review_rejects_unbacked_model_claims_and_keeps_original_gap_open() {
    for (with_fact,mutation) in [(true,"old_fact"),(true,"no_assessment"),(true,"wrong_source"),(false,"valid")] {
        let mut f=gap_review_fixture(with_fact);
        let (outcome,calls)=run_gap_review(&mut f,mutation);
        assert_eq!(calls,1,"{mutation}");
        assert!(matches!(outcome,AgentTargetOutcome::Failed(_) | AgentTargetOutcome::Incomplete(_)),"{outcome:?}");
        let connection=db::open(&f.context.db_path).unwrap();
        let count:i64=connection.query_row("SELECT COUNT(*) FROM agent_gap_review_receipts",[],|r|r.get(0)).unwrap();
        assert_eq!(count,0,"{mutation}: {outcome:?}");
        let tx=connection.unchecked_transaction().unwrap();
        assert_eq!(gap_review_status(&tx,&f.context.scan_id).unwrap()["gapResolved"],false);
    }
}

#[test]
fn gap_review_receipt_insert_failure_rolls_back_entire_delivery_and_recovers_without_model_retry() {
    for (timing,failure) in [("BEFORE","SELECT RAISE(ABORT,'receipt fault');"),("BEFORE","SELECT RAISE(IGNORE);"),
        ("AFTER","DELETE FROM agent_gap_review_receipts WHERE request_id=NEW.request_id;")] {
        let mut f=gap_review_fixture(true);
        let connection=db::open(&f.context.db_path).unwrap();
        connection.execute_batch(&format!("CREATE TRIGGER gap_receipt_fault {timing} INSERT ON agent_gap_review_receipts BEGIN {failure} END;")).unwrap();
        let (outcome,calls)=run_gap_review(&mut f,"valid");
        assert_eq!(calls,1); assert!(matches!(outcome,AgentTargetOutcome::Failed(_)),"{outcome:?}");
        let count:i64=connection.query_row("SELECT COUNT(*) FROM agent_review_decisions WHERE root_run_id=?1",[&f.session.lease.root_run_id],|r|r.get(0)).unwrap();
        assert_eq!(count,0);
        connection.execute_batch("DROP TRIGGER gap_receipt_fault").unwrap();
        let (outcome,calls)=run_gap_review(&mut f,"valid");
        assert_eq!(calls,0,"{outcome:?}");
        let tx=connection.unchecked_transaction().unwrap();
        let status=gap_review_status(&tx,&f.context.scan_id).unwrap();
        assert_eq!(status["gapResolved"],true,"{outcome:?}; {status}");
    }
}

#[test]
fn gap_review_status_revalidates_bytes_mailbox_and_receipts_instead_of_trusting_completed_task() {
    for mutation in ["fresh_bytes","source_bytes","mailbox","response","new_attempt","receipt"] {
        let mut f=gap_review_fixture(true);
        run_gap_review(&mut f,"valid");
        let connection=db::open(&f.context.db_path).unwrap();
        match mutation {
            "fresh_bytes"=>fs::write(f.context.target_dir.join("agent-http/0001.body"),"changed").unwrap(),
            "source_bytes"=>fs::write(f.source.context.target_dir.join("agent-http/0001.body"),"changed").unwrap(),
            "mailbox"=>{connection.execute("UPDATE agent_messages SET acknowledged_at='' WHERE root_run_id=?1 AND kind='review_decision'",[&f.session.lease.root_run_id]).unwrap();},
            "response"=>{
                assert!(connection.execute("UPDATE agent_specialist_calls SET response_json='{}' WHERE root_run_id=?1 AND role='evidence_reviewer'",[&f.session.lease.root_run_id]).is_err());
                // Simulate on-disk corruption after proving the normal write is blocked.
                connection.execute_batch("DROP TRIGGER agent_specialist_call_immutable").unwrap();
                connection.execute("UPDATE agent_specialist_calls SET response_json='{}' WHERE root_run_id=?1 AND role='evidence_reviewer'",[&f.session.lease.root_run_id]).unwrap();
            },
            "new_attempt"=>{connection.execute("UPDATE sentinel_scans SET attempt_count=2 WHERE id=?1",[&f.context.scan_id]).unwrap();},
            _=>{connection.execute("DELETE FROM agent_gap_review_receipts",[]).unwrap();},
        }
        let tx=connection.unchecked_transaction().unwrap();
        assert_eq!(gap_review_status(&tx,&f.context.scan_id).unwrap()["gapResolved"],false,"{mutation}");
    }
}

#[test]
fn gap_review_assessment_requires_exact_items_and_fresh_visible_refs() {
    let f=gap_review_fixture(true);
    let connection=db::open(&f.context.db_path).unwrap();
    let tx=connection.unchecked_transaction().unwrap();
    let candidate=json!({"gapFollowup":gap_followup_review_context(&tx,&f.session.lease.root_run_id,&f.context.target_dir).unwrap(),
        "persistedFactRefs":review_fact_refs(&tx,&f.session.lease.root_run_id,&f.context.target_dir).unwrap()});
    let valid=gap_review_response(&candidate,"rejected");
    assert!(validate_gap_assessment(&candidate,&validated_review_decision(&valid.to_string()).unwrap()).unwrap());
    for mutation in ["index","text","extra","empty","duplicate","reason","verdict"] {
        let mut response=valid.clone();
        match mutation {
            "index"=>response["gapAssessment"]["items"][0]["index"]=json!(1),
            "text"=>response["gapAssessment"]["items"][0]["missingEvidence"]=json!("different gap"),
            "extra"=>response["gapAssessment"]["items"][0]["allow"]=json!(true),
            "empty"=>response["gapAssessment"]["items"][0]["factRefs"]=json!([]),
            "duplicate"=>{let id=response["gapAssessment"]["items"][0]["factRefs"][0].clone();response["gapAssessment"]["items"][0]["factRefs"]=json!([id,id]);},
            "reason"=>response["gapAssessment"]["items"][0]["reason"]=json!(""),
            _=>response["gapAssessment"]["hypothesisVerdict"]=json!("insufficient_evidence"),
        }
        assert!(validate_gap_assessment(&candidate,&validated_review_decision(&response.to_string()).unwrap()).is_err(),"{mutation}");
    }
    assert!(!validate_gap_assessment(&candidate,&validated_review_decision(&gap_review_response(&candidate,"insufficient_evidence").to_string()).unwrap()).unwrap());
    let mut distinct=valid.clone();
    distinct["verdict"]=json!("insufficient_evidence");
    distinct["missingEvidence"]=json!(["new unrelated finding lacks evidence"]);
    assert!(validate_gap_assessment(&candidate,&validated_review_decision(&distinct.to_string()).unwrap()).unwrap(),
        "new finding verdict must not overwrite the original hypothesis verdict");
}

#[test]
fn gap_review_concurrent_receipt_recovery_commits_exactly_once() {
    let mut f=gap_review_fixture(true);
    let connection=db::open(&f.context.db_path).unwrap();
    connection.execute_batch("CREATE TRIGGER gap_receipt_fault BEFORE INSERT ON agent_gap_review_receipts BEGIN SELECT RAISE(ABORT,'fault'); END;").unwrap();
    run_gap_review(&mut f,"valid");
    connection.execute_batch("DROP TRIGGER gap_receipt_fault").unwrap();
    let (candidate,revision,text):(String,i64,String)=connection.query_row(
        "SELECT candidate_id,candidate_revision,candidate_json FROM agent_review_requests WHERE root_run_id=?1",
        [&f.session.lease.root_run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    let barrier=std::sync::Arc::new(std::sync::Barrier::new(2));
    let results=std::thread::scope(|scope| {
        let handles:Vec<_>=(0..2).map(|_|{
            let gate=barrier.clone();let path=&f.context.db_path;let lease=&f.session.lease;
            let dir=&f.context.target_dir;let candidate=&candidate;let text=&text;
            scope.spawn(move || {let db=db::open(path).unwrap();gate.wait();recover_received_review(&db,lease,candidate,revision,text,dir).is_ok()})
        }).collect();
        handles.into_iter().map(|h|h.join().unwrap()).collect::<Vec<_>>()
    });
    assert_eq!(results.iter().filter(|v|**v).count(),1);
    let count:i64=connection.query_row("SELECT COUNT(*) FROM agent_gap_review_receipts",[],|r|r.get(0)).unwrap();
    assert_eq!(count,1);
    let tx=connection.unchecked_transaction().unwrap();
    assert_eq!(gap_review_status(&tx,&f.context.scan_id).unwrap()["gapResolved"],true);
}

#[test]
fn gap_review_source_is_revalidated_before_any_new_specialist_or_target_capability() {
    let mut f=gap_review_fixture(false);
    let connection=db::open(&f.context.db_path).unwrap();
    connection.execute("UPDATE agent_messages SET acknowledged_at='' WHERE root_run_id=?1 AND kind='proposal_assessed'",[&f.source.root_run_id]).unwrap();
    f.context.run.as_mut().unwrap().run_id=f.session.lease.root_run_id.clone();
    let before=review_delivery_snapshot(&connection);
    assert!(multi_agent_prepare(&mut f.context).err().unwrap().contains("followup_acknowledged_assessment_unavailable"));
    assert_eq!(review_delivery_snapshot(&connection),before);
}

#[test]
fn gap_review_no_new_facts_preserves_insufficient_receipt_and_does_not_repeat_model_rounds() {
    let mut f=gap_review_fixture(false);
    let (outcome,calls)=run_gap_review(&mut f,"insufficient");
    assert!(matches!(outcome,AgentTargetOutcome::Incomplete(_)),"{outcome:?}");
    assert_eq!(calls,2,"one Reviewer and one bounded Investigator");
    let connection=db::open(&f.context.db_path).unwrap();
    let tx=connection.unchecked_transaction().unwrap();
    let status=gap_review_status(&tx,&f.context.scan_id).unwrap();
    assert_eq!(status["status"],"insufficient_evidence","{status}");
    assert_eq!(status["gapResolved"],false);
    drop(tx);
    let (_,calls)=run_gap_review(&mut f,"insufficient");
    assert_eq!(calls,0);
}
