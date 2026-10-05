struct FollowupFixture {
    root: PathBuf,
    context: AgentRunContext,
    root_run_id: String,
    assessment_id: String,
    _fixture: WebModeFixture,
}

fn followup_fixture() -> FollowupFixture {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|request| {
        let response = if request.contains("独立 Evidence Reviewer") {
            proposal_model_response(&json!({"verdict":"insufficient_evidence",
                "reasonCodes":["missing_control"],"missingEvidence":["missing control"],
                "confidence":0.2,"summary":"actual independent Reviewer needs control"}).to_string())
        } else { paid_proposal_response(&request, false) };
        (200, "application/json", response)
    }));
    let mut f = root_tick_fixture("gap-followup", &format!("http://127.0.0.1:{port}/v1"));
    drop(f.parent.take());
    let _transport = RealSpecialistTransport::enter();
    let mut session = multi_agent_prepare(&mut f.context).unwrap();
    let context = &f.context;
    let connection = db::open(&context.db_path).unwrap();
    let view = json!({"requestId":"req-source", "status":200,
        "bodySha256":format!("{:x}", Sha256::digest(b"safe body")),"redaction":{"applied":true}});
    // A local persisted artifact exercises followup provenance; no target HTTP is claimed.
    let artifact = agent_write_http_record(context,1,&json!({"method":"GET","url":context.target_url,"identity":"anonymous"}),&view,b"safe body").unwrap();
    agent_record_http_observation(context,"replay_http",&artifact,&view,"req-source").unwrap();
    assert_eq!(review_fact_refs(&connection,&session.lease.root_run_id,&context.target_dir).unwrap().len(),1);
    stage_agent_finding(context, AGENT_VULNERABILITY_STAGE, "vulnerability", "followup-hypothesis",
        "independently authorized control absent", "info", &json!({"claim":"missing control"})).unwrap();
    let outcome = AgentTargetOutcome::incomplete("fixture has no verified target control");
    multi_agent_finish_execution(context,&mut session,&outcome).unwrap();
    let reviewed = multi_agent_review(context,&mut session,outcome);
    assert!(!matches!(reviewed,AgentTargetOutcome::Completed(_)), "{reviewed:?}");
    assert_eq!(seen.lock().unwrap().len(),7,"four Root and independent Mapper/Reviewer/Investigator SDK");
    let (candidate,revision,missing) = paid_proposal_candidate(&connection);
    assert_eq!(missing,json!(["missing control"]));
    let before = web_mode_test_rows(&connection);
    multi_agent_investigate_review_gap(context,&session,&candidate,revision,&missing).unwrap();
    assert_eq!(seen.lock().unwrap().len(),7,"saved assessment replays without SDK or target permission");
    web_mode_assert_rows(&connection,&before);
    let assessment_id = connection.query_row("SELECT id FROM agent_messages WHERE kind='proposal_assessed'", [], |r|r.get(0)).unwrap();
    let root_run_id = session.lease.root_run_id.clone();
    drop(session);
    FollowupFixture {root:f.f.root.clone(),context:f.context,root_run_id,assessment_id,_fixture:f.f}
}

fn followup_preview(f: &FollowupFixture) -> Result<GapFollowupPreview,String> {
    let connection = db::open(&f.context.db_path)?;
    let transaction = connection.unchecked_transaction().unwrap();
    gap_followup_preview_in(&transaction, &f.context.scan_id, &f.assessment_id)
}

fn followup_input(f: &FollowupFixture) -> GapFollowupDraftInput {
    GapFollowupDraftInput {request_id:Uuid::new_v4().to_string(),source_scan_id:f.context.scan_id.clone(),
        assessment_message_id:f.assessment_id.clone(),source_hash:followup_preview(f).unwrap().source_hash,
        task_name:"Fresh evidence".into(), scan_mode:Some("standard".into()),max_budget_usd:Some(5.0),
        auth_session_ids:vec![],auth_session_scope_id:Uuid::new_v4().to_string(),skill_ids:vec![],instruction:String::new(),closure:None}
}

#[test]
fn followup_preview_and_atomic_draft_are_readonly_and_durably_linked() {
    let f = followup_fixture();
    let connection = db::open(&f.context.db_path).unwrap();
    // Preparing a new task from a closed source is evidence reading, not
    // resuming its execution. No live source capability may be required.
    connection.execute("UPDATE agent_runs SET status='terminal',terminal_state='incomplete' WHERE id=?1",[&f.root_run_id]).unwrap();
    connection.execute("UPDATE sentinel_scans SET status='completed_with_gaps' WHERE id=?1",[&f.context.scan_id]).unwrap();
    connection.execute("UPDATE sentinel_scan_attempts SET status='completed_with_gaps' WHERE scan_id=?1",[&f.context.scan_id]).unwrap();
    let counts = |connection:&rusqlite::Connection| -> (i64,i64,i64,i64) {
        connection.query_row("SELECT (SELECT COUNT(*) FROM agent_runs),(SELECT COUNT(*) FROM agent_messages),\
            (SELECT COUNT(*) FROM agent_capability_leases),(SELECT COUNT(*) FROM sentinel_scan_attempts)",[],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap()
    };
    let before = counts(&connection);
    let input = followup_input(&f);
    let draft = create_agent_gap_followup_in(&connection, &input).unwrap();
    assert_eq!(draft.status,"draft");
    assert_eq!(counts(&connection), before);
    let targets: (i64,String) = connection.query_row("SELECT COUNT(*),MIN(url) FROM sentinel_targets WHERE scan_id=?1",[&draft.id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(targets,(1,f.context.target_url.clone()));
    let tx = connection.unchecked_transaction().unwrap();
    let outgoing = gap_followup_relations(&tx,&f.context.scan_id).unwrap();
    assert_eq!(outgoing["tasks"][0]["scanId"],draft.id);
    assert_eq!(outgoing["gapResolved"],false);
    let incoming = gap_followup_relations(&tx,&draft.id).unwrap();
    assert_eq!(incoming["source"]["sourceHash"],input.source_hash);
    drop(tx);
    assert!(gap_followup_control_preflight(&connection,&f.root,&draft.id).unwrap_err().contains("two_identities"));
    // Lost IPC response: replay returns the same real record, including its
    // current status. Replaying never grants permission or issues a start.
    connection.execute("UPDATE sentinel_scans SET status='completed' WHERE id=?1",[&draft.id]).unwrap();
    let replay = create_agent_gap_followup_in(&connection,&input).unwrap();
    assert_eq!(replay.id,draft.id);
    assert_eq!(replay.status,"completed");
    let mut conflict=input.clone(); conflict.task_name.push('!');
    assert_eq!(create_agent_gap_followup_in(&connection,&conflict).unwrap_err(),"followup_request_id_conflict");
    assert_eq!(counts(&connection),before);
}

#[test]
fn followup_source_and_creation_reject_mutated_or_unacknowledged_evidence() {
    for mutation in [
        "UPDATE agent_messages SET acknowledged_at='' WHERE kind='proposal_assessed'",
        "UPDATE agent_messages SET acknowledged_at='' WHERE kind='gap_proposed'",
        "UPDATE agent_messages SET payload_json=json_set(payload_json,'$.schemaVersion',2) WHERE kind='proposal_assessed'",
        "UPDATE agent_messages SET payload_json=json_set(payload_json,'$.targetRequestsGranted',3) WHERE kind='proposal_assessed'",
        "UPDATE agent_review_requests SET candidate_json='{}'",
        "UPDATE agent_review_requests SET status='superseded'",
        "UPDATE agent_assignments SET state='failed' WHERE role='deep_investigator'",
        "UPDATE projects SET status='archived'",
        "DELETE FROM agent_evidence_locations",
    ] {
        let f = followup_fixture();
        let input = followup_input(&f);
        let connection = db::open(&f.context.db_path).unwrap();
        connection.execute(mutation,[]).unwrap();
        assert!(followup_preview(&f).is_err(),"{mutation}");
        assert!(create_agent_gap_followup_in(&connection,&input).is_err(),"{mutation}");
        assert_eq!(connection.query_row("SELECT COUNT(*) FROM sentinel_scans",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    }
}

#[test]
fn followup_creation_rolls_back_provenance_failure_and_retries_same_key() {
    for action in ["ABORT", "IGNORE"] {
        let f=followup_fixture();
        let input=followup_input(&f);
        let connection=db::open(&f.context.db_path).unwrap();
        let raise = if action=="ABORT" { "RAISE(ABORT,'fault')" } else { "RAISE(IGNORE)" };
        connection.execute_batch(&format!("CREATE TRIGGER reject_followup BEFORE INSERT ON agent_gap_followups BEGIN SELECT {raise}; END;")).unwrap();
        let before=web_mode_test_rows(&connection);
        assert!(create_agent_gap_followup_in(&connection,&input).is_err());
        let counts:(i64,i64,i64)=connection.query_row("SELECT (SELECT COUNT(*) FROM sentinel_scans),\
            (SELECT COUNT(*) FROM sentinel_targets),(SELECT COUNT(*) FROM agent_gap_followups)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(counts,(1,1,0)); // The genuine source creator has one original target.
        web_mode_assert_rows(&connection,&before);
        connection.execute_batch("DROP TRIGGER reject_followup").unwrap();
        assert_eq!(create_agent_gap_followup_in(&connection,&input).unwrap().status,"draft");
    }
}

#[test]
fn followup_historical_binding_rejects_retargeting_missing_and_symlinked_directories() {
    let f=followup_fixture();
    let connection=db::open(&f.context.db_path).unwrap();
    assert!(connection.execute("UPDATE agent_evidence_locations SET target_url='https://other.invalid'",[]).is_err());
    // Mutating the current URL list cannot move the persisted evidence binding.
    connection.execute("INSERT INTO sentinel_targets(project_id,scan_id,company,url) SELECT project_id,id,'fixture','https://other.invalid' FROM sentinel_scans",[]).unwrap();
    assert_eq!(followup_preview(&f).unwrap().target_url,f.context.target_url);
    let body=f.context.target_dir.join("agent-http/0001.body");
    fs::write(&body,b"fake body").unwrap();
    assert!(followup_preview(&f).is_err(),"same-length artifact replacement must invalidate a sealed source");
    fs::write(&body,b"safe body").unwrap();
    assert!(followup_preview(&f).is_ok());
    let moved=f.context.target_dir.with_file_name("target-00002");
    assert_ne!(moved,f.context.target_dir);
    fs::rename(&f.context.target_dir,&moved).unwrap();
    assert!(followup_preview(&f).is_err());
    #[cfg(unix)] {
        std::os::unix::fs::symlink(&moved,&f.context.target_dir).unwrap();
        assert_eq!(followup_preview(&f).unwrap_err(),"followup_evidence_directory_mismatch");
        fs::remove_file(&f.context.target_dir).unwrap();
    }
    fs::rename(&moved,&f.context.target_dir).unwrap();
    fs::write(f.context.target_dir.join(".oviraptor-scan-id"),"other-scan").unwrap();
    assert_eq!(followup_preview(&f).unwrap_err(),"followup_evidence_marker_mismatch");
    fs::write(f.context.target_dir.join(".oviraptor-scan-id"),&f.context.scan_id).unwrap();
    connection.execute("UPDATE agent_runs SET target_url='https://other.invalid' WHERE id=?1",[&f.root_run_id]).unwrap();
    assert!(followup_preview(&f).is_err());
}

#[test]
fn followup_concurrent_idempotency_creates_one_task() {
    let f=followup_fixture();
    let input=followup_input(&f);
    let barrier=std::sync::Arc::new(std::sync::Barrier::new(2));
    let threads:Vec<_>=(0..2).map(|_| {
        let input=input.clone(); let path=f.context.db_path.clone(); let barrier=barrier.clone();
        std::thread::spawn(move || { let connection=db::open(&path).unwrap(); barrier.wait(); create_agent_gap_followup_in(&connection,&input).unwrap().id })
    }).collect();
    let ids:Vec<_>=threads.into_iter().map(|t|t.join().unwrap()).collect();
    assert_eq!(ids[0],ids[1]);
    let connection=db::open(&f.context.db_path).unwrap();
    assert_eq!(connection.query_row("SELECT COUNT(*) FROM agent_gap_followups",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    let mut stale=input.clone(); stale.request_id=Uuid::new_v4().to_string(); stale.source_hash="stale".into();
    assert_eq!(create_agent_gap_followup_in(&connection,&stale).unwrap_err(),"followup_source_changed_preview_again");
}

fn add_followup_identities(connection: &rusqlite::Connection, input: &mut GapFollowupDraftInput) {
    input.auth_session_ids = vec!["fresh-owner".into(), "fresh-tester".into()];
    let project:i64=connection.query_row("SELECT project_id FROM sentinel_scans WHERE id=?1",[&input.source_scan_id],|r|r.get(0)).unwrap();
    for id in &input.auth_session_ids {
        connection.execute("INSERT INTO browser_auth_sessions(id,project_id,draft_scope_id,name,entry_url,status,session_json,expires_at) \
            VALUES(?1,?5,?2,?1,'https://authorized.example.test','valid',?3,?4)",
            params![id,input.auth_session_scope_id,json!({"scopeHosts":["authorized.example.test"],"cookies":[{"name":"session","value":id}]}).to_string(),
                (chrono::Utc::now()+chrono::Duration::hours(1)).to_rfc3339(),project]).unwrap();
    }
}

#[test]
fn followup_fresh_identities_and_explicit_controls_are_required_before_start() {
    let f=followup_fixture();
    let mut connection=db::open(&f.context.db_path).unwrap();
    let mut input=followup_input(&f);
    add_followup_identities(&connection,&mut input);
    connection.execute("UPDATE browser_auth_sessions SET owner_scan_id=?1,draft_scope_id='' WHERE id='fresh-owner'",[&f.context.scan_id]).unwrap();
    assert!(create_agent_gap_followup_in(&connection,&input).is_err(),"old task identity cannot be inherited");
    connection.execute("UPDATE browser_auth_sessions SET owner_scan_id='',draft_scope_id=?1 WHERE id='fresh-owner'",[&input.auth_session_scope_id]).unwrap();
    let draft=create_agent_gap_followup_in(&connection,&input).unwrap();
    assert_eq!(gap_followup_control_preflight(&connection,&f.root,&draft.id).unwrap_err(),"followup_requires_explicit_authorization_controls");
    let setup=authorization_control_setup_in(&connection,&f.root,&draft.id,&f.context.target_url).unwrap();
    let control=SaveAuthorizationControlInput {
        scan_id:draft.id.clone(),attempt_number:setup.attempt_number,target_url:f.context.target_url.clone(),
        contract_key:"idor|/api/orders".into(),owner_object_url:format!("{}/api/orders?id=owner",f.context.target_url),
        tester_control_url:format!("{}/api/orders?id=tester",f.context.target_url),object_query_key:"id".into(),
        owner_object_value:"owner".into(),tester_object_value:"tester".into(),response_object_pointer:"/order/id".into(),
        owner_identity:"fresh-owner".into(),tester_identity:"fresh-tester".into(),
    };
    save_authorization_control_in(&mut connection,&f.root,&control).unwrap();
    assert_eq!(gap_followup_control_preflight(&connection,&f.root,&draft.id).unwrap(),Some(setup.attempt_number));
    connection.execute("INSERT INTO sentinel_targets(project_id,scan_id,company,url) SELECT project_id,id,'extra','https://extra.invalid' FROM sentinel_scans WHERE id=?1",[&draft.id]).unwrap();
    assert_eq!(gap_followup_control_preflight(&connection,&f.root,&draft.id).unwrap_err(),"followup_source_scope_changed");
    connection.execute("DELETE FROM sentinel_targets WHERE scan_id=?1 AND url='https://extra.invalid'",[&draft.id]).unwrap();
    connection.execute("INSERT INTO projects(id,name) SELECT project_id+100,'other-followup-project' FROM sentinel_scans WHERE id=?1",[&draft.id]).unwrap();
    connection.execute("UPDATE sentinel_targets SET project_id=project_id+100 WHERE scan_id=?1",[&draft.id]).unwrap();
    assert_eq!(gap_followup_control_preflight(&connection,&f.root,&draft.id).unwrap_err(),"followup_source_scope_changed");
    connection.execute("UPDATE sentinel_targets SET project_id=project_id-100 WHERE scan_id=?1",[&draft.id]).unwrap();
    connection.execute("UPDATE sentinel_scans SET project_id=project_id+100 WHERE id=?1",[&draft.id]).unwrap();
    assert_eq!(gap_followup_control_preflight(&connection,&f.root,&draft.id).unwrap_err(),"followup_source_scope_changed");
    connection.execute("UPDATE sentinel_scans SET project_id=project_id-100 WHERE id=?1",[&draft.id]).unwrap();
    assert_eq!(gap_followup_control_preflight(&connection,&f.root,&draft.id).unwrap(),Some(setup.attempt_number));
    let ids=crate::auth_session::validated_scan_identities(&connection,&draft.id,&f.context.target_url).unwrap().1;
    assert_eq!(ids.len(),2);
    assert_eq!(create_agent_gap_followup_in(&connection,&input).unwrap().id,draft.id,"receipt replay must not rebind consumed identities");
    connection.execute("UPDATE browser_auth_sessions SET status='expired' WHERE id='fresh-owner'",[]).unwrap();
    assert!(gap_followup_control_preflight(&connection,&f.root,&draft.id).is_err());
    connection.execute("UPDATE browser_auth_sessions SET status='valid' WHERE id='fresh-owner'",[]).unwrap();
    fs::create_dir_all(neutral_scan_work_root(&f.root).join(&draft.id).join("attempt-00001")).unwrap();
    assert!(gap_followup_control_preflight(&connection,&f.root,&draft.id).is_err(),"a skipped attempt cannot inherit the old control group");
}

#[test]
fn followup_provenance_failure_rolls_back_identity_ownership_and_all_draft_rows() {
    let f=followup_fixture();
    let connection=db::open(&f.context.db_path).unwrap();
    let mut input=followup_input(&f);
    add_followup_identities(&connection,&mut input);
    connection.execute_batch("CREATE TRIGGER reject_followup_identity BEFORE INSERT ON agent_gap_followups BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    assert!(create_agent_gap_followup_in(&connection,&input).is_err());
    let owners:i64=connection.query_row("SELECT COUNT(*) FROM browser_auth_sessions WHERE owner_scan_id='' AND draft_scope_id=?1",[&input.auth_session_scope_id],|r|r.get(0)).unwrap();
    assert_eq!(owners,2);
    let drafts:i64=connection.query_row("SELECT COUNT(*) FROM sentinel_scans WHERE status='draft'",[],|r|r.get(0)).unwrap();
    assert_eq!(drafts,0);
    connection.execute_batch("DROP TRIGGER reject_followup_identity").unwrap();
    assert_eq!(create_agent_gap_followup_in(&connection,&input).unwrap().status,"draft");
}

#[test]
fn followup_exact_target_does_not_strip_the_sources_resource_slash() {
    let f=followup_fixture();
    let connection=db::open(&f.context.db_path).unwrap();
    let transaction=connection.unchecked_transaction().unwrap();
    let target="https://authorized.example.test/path/";
    let project:i64=transaction.query_row("SELECT project_id FROM sentinel_scans WHERE id=?1",[&f.context.scan_id],|r|r.get(0)).unwrap();
    let draft=create_sentinel_url_draft_rows(&transaction,project,"exact".into(),vec![target.into()],None,None,None,None,None,None,None,None,Some(target)).unwrap();
    let saved:String=transaction.query_row("SELECT url FROM sentinel_targets WHERE scan_id=?1",[draft.id],|r|r.get(0)).unwrap();
    assert_eq!(saved,target);
}
