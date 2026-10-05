fn source_coordinator_fixture() -> (PathBuf,rusqlite::Connection,WorkbenchStartRecord) {
    let (root,connection,record)=analysis_view_fixture("full",true);
    analysis_view_run(&root,&record,|_,engine,_,_,scratch,_|
        Ok(source_result_outcome(engine,scratch,"app.py","coordinator-registration"))).unwrap();
    (root,connection,record)
}

fn source_coordinator_row(connection:&rusqlite::Connection,id:&str)->Vec<rusqlite::types::Value> {
    let mut stmt=connection.prepare("SELECT * FROM agent_runs WHERE id=?1").unwrap();
    let columns=stmt.column_count();
    stmt.query_row([id],|r|(0..columns).map(|i|r.get(i)).collect()).unwrap()
}

#[test]
fn source_coordinator_rejects_unimplemented_phase_version_before_publication() {
    let (root,connection,record)=source_coordinator_fixture();
    for schema in [0,5,99] {
        assert_eq!(prepare_native_source_coordinator_schema(&connection,&record.scan_id,1,
            &root.join("attempt-0001"),schema).unwrap_err(),"source_surface_frozen_plan_invalid");
        for table in ["agent_runs","agent_assignments","agent_coordinator_leases","agent_budget_ledger"] {
            assert_eq!(connection.query_row(&format!("SELECT count(*) FROM {table}"),[],|r|r.get::<_,i64>(0)).unwrap(),0,
                "unsupported version {schema} must not publish {table}");
        }
    }
    assert!(prepare_native_source_coordinator(&connection,&record.scan_id,1,&root.join("attempt-0001")).is_ok());
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_coordinator_registers_real_material_and_preserves_recovery_state() {
    let (root,connection,record)=source_coordinator_fixture();
    let work=root.join("attempt-0001");
    let registered=prepare_native_source_coordinator(&connection,&record.scan_id,1,&work).unwrap();
    let (_,_,runtime)=verify_source_runtime_contract(&connection,&record.scan_id,1,&work).unwrap();
    let (frozen,tokens,requests):(String,i64,i64)=connection.query_row(
        "SELECT plan_json,hard_token_budget,hard_request_budget FROM agent_runs WHERE id=?1",
        [&registered.run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    let frozen:JsonValue=serde_json::from_str(&frozen).unwrap();
    assert_eq!(frozen["runtime"],runtime);
    assert_eq!(frozen["targetRequestsGranted"],0);
    assert_eq!(frozen["hostActionsGranted"],0);
    assert_eq!(runtime["budget"]["tokenLimit"],tokens);
    assert_eq!(frozen["modelRequestLimit"],requests);
    assert_eq!(frozen["schemaVersion"],4,"new source tasks must include real coverage review");
    assert_eq!(frozen["sourceCoveragePhaseVersion"],1);
    assert_eq!(frozen["sourceCoverageDecisionPhaseVersion"],1);
    assert_eq!(requests,10);
    assert_eq!(registered.target_key,crate::agent_runtime::multi_agent::source::target_key(&source_result_view(&connection,&record)));
    connection.execute("UPDATE agent_runs SET status='running',used_tokens=17,used_requests=1,
        reserved_tokens=21,reserved_requests=1,heartbeat_at='preserve-me' WHERE id=?1",[&registered.run_id]).unwrap();
    let before=source_coordinator_row(&connection,&registered.run_id);
    assert_eq!(prepare_native_source_coordinator(&connection,&record.scan_id,1,&work).unwrap(),registered);
    assert_eq!(source_coordinator_row(&connection,&registered.run_id),before);
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_runs",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_coordinator_leases",[],|r|r.get::<_,i64>(0)).unwrap(),1,
        "true fresh Source registration freezes the first original C with its financial control");
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_coordinator_never_repairs_terminal_cancelled_or_changed_roots() {
    for mutation in ["status='terminal'","status='legacy_backend_removed'","status='paused'",
        "cancel_requested_at='cancelled'","backend='unknown'","role='source_analyst'",
        "plan_hash='changed'","evidence_hash='changed'","plan_json='{}'",
        "target_url='https://unrelated.invalid'","hard_token_budget=1","hard_request_budget=999",
        "terminal_code='already_finished'","capability_lease_json='[\"shell\"]'","lane='target_touching'"] {
        let (root,connection,record)=source_coordinator_fixture();
        let work=root.join("attempt-0001");
        let registered=prepare_native_source_coordinator(&connection,&record.scan_id,1,&work).unwrap();
        connection.execute(&format!("UPDATE agent_runs SET {mutation} WHERE id=?1"),[&registered.run_id]).unwrap();
        let before=source_coordinator_row(&connection,&registered.run_id);
        assert_eq!(prepare_native_source_coordinator(&connection,&record.scan_id,1,&work).unwrap_err(),
            "source_coordinator_binding_or_state_conflict","{mutation}");
        assert_eq!(source_coordinator_row(&connection,&registered.run_id),before,"{mutation}");
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_coordinator_registration_preserves_versions_and_freezes_review_contracts() {
    for schema in [1,2,3,4] {
        let review=schema>=2;
        let (root,connection,record)=source_coordinator_fixture();
        let work=root.join("attempt-0001");
        let registered=prepare_native_source_coordinator_schema(&connection,&record.scan_id,1,&work,schema).unwrap();
        let before=source_coordinator_row(&connection,&registered.run_id);
        assert_eq!(prepare_native_source_coordinator(&connection,&record.scan_id,1,&work).unwrap(),registered);
        assert_eq!(source_coordinator_row(&connection,&registered.run_id),before);
        let (text,limit):(String,i64)=connection.query_row("SELECT plan_json,hard_request_budget FROM agent_runs WHERE id=?1",[&registered.run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        let plan:JsonValue=serde_json::from_str(&text).unwrap();
        assert_eq!(plan["schemaVersion"],schema);
        assert_eq!(limit,if schema==4 {10} else if review {9} else {8});
        assert_eq!(crate::agent_runtime::multi_agent::source::model_request_limit(&plan).unwrap(),limit);
        assert_eq!(plan.get("sourceReviewPhaseVersion").is_some(),review);
        assert_eq!(plan.get("sourceDecisionPhaseVersion").is_some(),schema>=3);
        assert_eq!(plan.get("sourceCoveragePhaseVersion").is_some(),schema==4);
        assert_eq!(plan.get("sourceCoverageDecisionPhaseVersion").is_some(),schema==4);
        let mut malformed=plan.clone();
        malformed["sourceDecisionPhaseVersion"]=json!(if schema>=3 {2} else {1});
        assert!(crate::agent_runtime::multi_agent::source::model_request_limit(&malformed).is_err());
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_coordinator_requires_published_runtime_and_current_attempt() {
    for mutation in ["missing_key","paused","deleted","replaced_attempt","missing_results"] {
        let (root,connection,record)=source_coordinator_fixture();
        let work=root.join("attempt-0001");
        match mutation {
            "missing_key"=>fs::remove_file(work.join(SOURCE_RUNTIME_KEY_FILE)).unwrap(),
            "paused"=>{connection.execute("UPDATE sentinel_scans SET status='paused' WHERE id=?1",[&record.scan_id]).unwrap();},
            "deleted"=>{connection.execute("INSERT INTO sentinel_deleted_scans(scan_id) VALUES(?1)",[&record.scan_id]).unwrap();},
            "replaced_attempt"=>{connection.execute("UPDATE sentinel_scans SET attempt_count=2 WHERE id=?1",[&record.scan_id]).unwrap();},
            "missing_results"=>{
                // Simulate a damaged/legacy database, not a supported delete.
                connection.execute_batch("DROP TRIGGER analysis_results_no_delete;").unwrap();
                connection.execute("DELETE FROM source_analysis_results WHERE scan_id=?1",[&record.scan_id]).unwrap();
            },
            _=>unreachable!(),
        }
        assert!(prepare_native_source_coordinator(&connection,&record.scan_id,1,&work).is_err(),"{mutation}");
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_runs",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_coordinator_insert_failures_and_post_write_revocation_roll_back() {
    for action in ["SELECT RAISE(IGNORE);","SELECT RAISE(ABORT,'injected');",
        "UPDATE sentinel_scans SET status='paused' WHERE id=NEW.scan_id;",
        "UPDATE agent_runs SET hard_token_budget=1 WHERE id=NEW.id;",
        "UPDATE agent_runs SET used_tokens=1 WHERE id=NEW.id;",
        "UPDATE config_profiles SET settings_json=json_set(settings_json,'$.agentDeepTokenLimit',123456);"] {
        let (root,connection,record)=source_coordinator_fixture();
        let timing=if action.starts_with("SELECT RAISE") {"BEFORE"} else {"AFTER"};
        connection.execute_batch(&format!("CREATE TRIGGER source_root_fault {timing} INSERT ON agent_runs
            WHEN NEW.role='coordinator' BEGIN {action} END;")).unwrap();
        assert!(prepare_native_source_coordinator(&connection,&record.scan_id,1,&root.join("attempt-0001")).is_err(),"{action}");
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_runs",[],|r|r.get::<_,i64>(0)).unwrap(),0,"{action}");
        assert_eq!(connection.query_row("SELECT status FROM sentinel_scans WHERE id=?1",[&record.scan_id],|r|r.get::<_,String>(0)).unwrap(),"scanning");
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_coordinator_changed_valid_results_do_not_mint_a_second_root() {
    let (root,connection,record)=source_coordinator_fixture();
    let work=root.join("attempt-0001");
    let registered=prepare_native_source_coordinator(&connection,&record.scan_id,1,&work).unwrap();
    let before=source_coordinator_row(&connection,&registered.run_id);
    let view=source_result_view(&connection,&record);
    let mut results=crate::native_pipeline::results::AnalysisResults::load(&connection,&view).unwrap();
    results.gaps.push("changed-after-registration".into());
    connection.execute_batch("DROP TRIGGER analysis_results_no_update;").unwrap();
    connection.execute("UPDATE source_analysis_results SET receipt_json=?1,receipt_digest=?2 WHERE scan_id=?3",
        params![serde_json::to_string(&results).unwrap(),results.digest(),record.scan_id]).unwrap();
    // A self-consistent receipt is still not the one registered by the root.
    crate::native_pipeline::results::AnalysisResults::load(&connection,&view).unwrap();
    assert_eq!(prepare_native_source_coordinator(&connection,&record.scan_id,1,&work).unwrap_err(),
        "source_coordinator_binding_or_state_conflict");
    assert_eq!(source_coordinator_row(&connection,&registered.run_id),before);
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_runs",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_coordinator_does_not_adopt_an_existing_differently_named_source_root() {
    use crate::agent_runtime::{contract::{AgentRole,AgentBackendKind},store::{self,AgentRunRow}};
    for target in ["actual","source:old-material"] {
        let (root,connection,record)=source_coordinator_fixture();
        let actual=crate::agent_runtime::multi_agent::source::target_key(&source_result_view(&connection,&record));
        let target=if target=="actual" {actual.as_str()} else {target};
        let row=AgentRunRow::new("legacy-source-root",&record.scan_id,1,target,AgentBackendKind::Native,
            AgentRole::Coordinator,"legacy-plan","legacy-results");
        store::create_run(&connection,&row).unwrap();
        let before=source_coordinator_row(&connection,"legacy-source-root");
        assert_eq!(prepare_native_source_coordinator(&connection,&record.scan_id,1,&root.join("attempt-0001")).unwrap_err(),
            "source_coordinator_conflicting_root");
        assert_eq!(source_coordinator_row(&connection,"legacy-source-root"),before);
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_runs",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_coordinator_concurrent_registration_has_one_identity() {
    let (root,connection,record)=source_coordinator_fixture();
    let barrier=Arc::new(std::sync::Barrier::new(2));
    let workers=(0..2).map(|_| {
        let root=root.clone();let scan=record.scan_id.clone();let barrier=barrier.clone();
        thread::spawn(move || {
            let connection=db::open(&root.join("oviraptor.sqlite3")).unwrap();
            barrier.wait();
            prepare_native_source_coordinator(&connection,&scan,1,&root.join("attempt-0001"))
        })
    }).collect::<Vec<_>>();
    let roots=workers.into_iter().map(|worker|worker.join().unwrap().unwrap()).collect::<Vec<_>>();
    assert_eq!(roots[0],roots[1]);
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_runs",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_coordinator_recovery_does_not_reset_unknown_model_dispatch() {
    use crate::agent_runtime::{contract::AgentRole,multi_agent::{source,scheduler,specialist}};
    let (root,connection,record,lease)=source_specialist_fixture();
    let slice=source::task_slice(&connection,&lease,AgentRole::RepoMapper).unwrap();
    let child=scheduler::prepare_readonly_child(&connection,&lease,AgentRole::RepoMapper,"source_results_ready",&slice,8_000).unwrap();
    let request=source_specialist_request(&connection,&lease,AgentRole::RepoMapper);
    assert!(matches!(specialist::start(&connection,&lease,&child,&request).unwrap(),specialist::Start::Dispatch(_)));
    let before=source_coordinator_row(&connection,&lease.root_run_id);
    prepare_native_source_coordinator(&connection,&record.scan_id,1,&root.join("attempt-0001")).unwrap();
    assert_eq!(source_coordinator_row(&connection,&lease.root_run_id),before);
    assert!(matches!(specialist::start(&connection,&lease,&child,&request),Err(error)
        if error=="specialist_call_outcome_unknown_requires_reconciliation"));
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_specialist_calls",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(connection.query_row("SELECT reserved_tokens FROM agent_assignments WHERE id=?1",[&child.assignment_id],|r|r.get::<_,i64>(0)).unwrap(),8_000);
    drop(connection);fs::remove_dir_all(root).unwrap();
}
