fn prepare_recovery_test_files(startup: &WebStartup) {
    let targets = serde_json::json!(startup.targets.iter().map(|(company,url)|
        serde_json::json!({"company":company,"url":url})).collect::<Vec<_>>());
    fs::write(startup.files.path.join("targets.json"),targets.to_string()).unwrap();
    let task_path = startup.files.path.join("task.json");
    let mut task: JsonValue = serde_json::from_slice(&fs::read(&task_path).unwrap()).unwrap();
    task["runtimePolicy"] = json!({"backend":"native-agent"});
    fs::write(task_path,task.to_string()).unwrap();
}

fn recovery_test_fixture() -> (PathBuf,PathBuf,PathBuf,rusqlite::Connection) {
    recovery_test_fixture_with(|_| {})
}

fn recovery_test_fixture_with(setup: impl FnOnce(&rusqlite::Connection)) -> (PathBuf,PathBuf,PathBuf,rusqlite::Connection) {
    let (root,db_path,mut connection) = web_start_fixture();
    setup(&connection);
    let transaction = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
    let mut startup = prepare_binding_test_startup(&transaction,&root);
    prepare_recovery_test_files(&startup);
    register_web_dispatch_binding_in(&transaction,&startup,&binding_test_runtime()).unwrap();
    register_private_web_mode_on(&transaction,&startup,&binding_test_runtime()).unwrap();
    let work_dir = startup.files.path.clone();
    startup.files.preserve = true;
    transaction.commit().unwrap();
    (root,db_path,work_dir,connection)
}

fn recovery_test_reconstruct(connection: &rusqlite::Connection, root: &Path) -> Result<PathBuf,String> {
    let (path,targets) = web_recovery_plan_in(connection,root,"start-test",1)?;
    assert_eq!(targets.len(),2);
    verify_web_dispatch_binding_in(connection,"start-test",1,&WebBindingDirectory::open(&path)?,&binding_test_runtime())?;
    Ok(path)
}

fn recovery_test_unclaimed(connection: &rusqlite::Connection) {
    let row: (String,String,String,i64) = connection.query_row(
        "SELECT d.claim_id,d.claimed_at,s.status,s.attempt_count FROM native_branch_dispatches d
         JOIN sentinel_scans s ON s.id=d.scan_id WHERE d.scan_id='start-test' AND d.attempt_number=1",
        [],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
    ).unwrap();
    assert_eq!(row,(String::new(),String::new(),"scanning".into(),1));
}

#[test]
fn web_recovery_requires_explicit_exact_attempt_and_does_not_claim_on_reads() {
    let (root,path,_,connection) = recovery_test_fixture();
    let before = web_start_snapshot(&connection);
    for _ in 0..3 {
        let status = native_scan_status(&connection,"start-test").unwrap();
        assert_eq!(status["branches"][0]["dispatch"]["manualRecoveryAvailable"],true);
        assert_eq!(status["branches"][0]["dispatch"]["automaticReplayAllowed"],false);
    }
    for (attempt,confirmed) in [(1,false),(0,true),(-1,true),(2,true),(i64::MAX,true)] {
        assert!(claim_web_recovery(&path,"start-test",attempt,confirmed,|_| -> Result<(),String> {
            panic!("invalid input must not reach reconstruction");
        }).is_err());
    }
    assert_eq!(web_start_snapshot(&connection),before);
    recovery_test_unclaimed(&connection);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn web_recovery_transfers_one_durable_claim_to_worker_without_reclaim_or_scope_change() {
    let (root,path,work_dir,connection) = recovery_test_fixture();
    let before = web_start_snapshot(&connection);
    let mut calls = 0;
    let OwnedWebRecovery { _lifecycle, guard, inputs } = claim_web_recovery(&path,"start-test",1,true,|conn| {
        calls += 1;
        recovery_test_reconstruct(conn,&root)
    }).unwrap();
    assert_eq!(calls,2);
    assert_eq!(inputs,work_dir);
    assert!(claim_scan_control(&path,"start-test").is_err(),"lifecycle owner survives admission");
    let receipt: String = connection.query_row("SELECT claim_id FROM native_branch_dispatches",[],|r|r.get(0)).unwrap();
    assert!(!receipt.is_empty());
    let worker = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/workers/9_frontend_runtime_probe.cjs");
    let mut admission = Some(WebDispatchAdmission { home:root.clone(),settings:serde_json::json!({}),
        recon_config:frontend_recon_config(&worker),claimed_guard:Some(guard) });
    let mut owned = claim_web_pipeline_branch(&path,"start-test",1,&work_dir,&worker,&mut admission).unwrap();
    assert!(admission.as_ref().unwrap().claimed_guard.is_none());
    assert!(claim_web_pipeline_branch(&path,"start-test",1,&work_dir,&worker,&mut admission).is_err());
    assert_eq!(connection.query_row("SELECT claim_id FROM native_branch_dispatches",[],|r|r.get::<_,String>(0)).unwrap(),receipt);
    assert_eq!(web_start_snapshot(&connection),before,"recovery must not create attempts, widen scope or reset budgets");
    owned.disarm();
    drop(owned);
    drop(_lifecycle);
    assert!(claim_web_recovery(&path,"start-test",1,true,|conn|recovery_test_reconstruct(conn,&root)).is_err(),"release does not permit replay");
    assert!(!web_recovery_available_in(&connection,"start-test",1));
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn web_recovery_rejects_unsupported_or_changed_scope_without_side_effects() {
    for change in [
        "UPDATE sentinel_scans SET scan_type='greybox'",
        "UPDATE sentinel_scans SET source_path='/not-a-web-source'",
        "UPDATE sentinel_scans SET llm_requests=1",
        "UPDATE sentinel_scans SET total_tokens=1",
        "UPDATE sentinel_scan_attempts SET status='failed'",
        "UPDATE projects SET status='archived'",
        "UPDATE sentinel_scans SET status='paused'",
        "INSERT INTO native_scan_branches(scan_id,attempt_number,branch) VALUES('start-test',1,'source')",
        "INSERT INTO sentinel_processes(scan_id,process_id) VALUES('start-test',999999)",
        "INSERT INTO agent_gap_followups(scan_id,request_id,request_hash,source_scan_id,assessment_message_id,source_hash,source_preview_json) VALUES('start-test','r','h','s','a','h','{}')",
        "INSERT INTO agent_authorization_controls(scan_id,attempt_number,target_url,contract_key,method,owner_object_url,tester_control_url,object_query_key,owner_object_value,tester_object_value,response_object_pointer,owner_identity,tester_identity) VALUES('start-test',1,'https://start.example.test/one','x','GET','x','y','id','x','y','/id','a','b')",
        "DELETE FROM native_web_dispatch_bindings",
    ] {
        let (root,path,_,connection) = recovery_test_fixture();
        connection.execute_batch(change).unwrap();
        let before = web_start_snapshot(&connection);
        assert!(!web_recovery_available_in(&connection,"start-test",1),"{change}");
        assert!(claim_web_recovery(&path,"start-test",1,true,|_| -> Result<(),String> {
            panic!("unsupported scope reached reconstruction: {change}");
        }).is_err(),"{change}");
        assert_eq!(web_start_snapshot(&connection),before,"{change}");
        assert!(connection.query_row("SELECT claim_id FROM native_branch_dispatches WHERE branch='web'",[],|r|r.get::<_,String>(0)).unwrap().is_empty());
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn web_recovery_faults_before_after_and_at_commit_never_publish_a_guard() {
    for fault in [
        "CREATE TRIGGER recovery_fault BEFORE UPDATE OF claim_id ON native_branch_dispatches BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER recovery_fault BEFORE UPDATE OF claim_id ON native_branch_dispatches BEGIN SELECT RAISE(ABORT,'injected'); END;",
        "CREATE TRIGGER recovery_fault AFTER UPDATE OF claim_id ON native_branch_dispatches BEGIN UPDATE sentinel_targets SET url='https://outside.example.test'; END;",
        "CREATE TRIGGER recovery_fault AFTER UPDATE OF claim_id ON native_branch_dispatches BEGIN UPDATE sentinel_scans SET llm_requests=1; END;",
        "CREATE TRIGGER recovery_fault AFTER UPDATE OF claim_id ON native_branch_dispatches BEGIN INSERT INTO sentinel_fuse_zone(project_id,company,url,normalized_url,reason) VALUES(1,'one','https://start.example.test/one','https://start.example.test/one','test'); END;",
        "CREATE TABLE recovery_parent(id INTEGER PRIMARY KEY); CREATE TABLE recovery_child(id INTEGER REFERENCES recovery_parent(id) DEFERRABLE INITIALLY DEFERRED); CREATE TRIGGER recovery_fault AFTER UPDATE OF claim_id ON native_branch_dispatches BEGIN INSERT INTO recovery_child VALUES(1); END;",
    ] {
        let (root,path,_,connection) = recovery_test_fixture();
        connection.execute_batch(fault).unwrap();
        let before = web_start_snapshot(&connection);
        assert!(claim_web_recovery(&path,"start-test",1,true,|conn|recovery_test_reconstruct(conn,&root)).is_err(),"{fault}");
        assert_eq!(web_start_snapshot(&connection),before,"{fault}");
        recovery_test_unclaimed(&connection);
        assert!(claim_scan_control(&path,"start-test").is_ok());
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn web_recovery_and_original_dispatch_race_has_one_winner() {
    let (root,path,work_dir,connection) = recovery_test_fixture();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let start = barrier.clone();
    let (original_path,original_dir) = (path.clone(),work_dir.clone());
    let original = thread::spawn(move || {
        start.wait();
        NativeBranchGuard::claim_with_preflight(&original_path,"start-test",1,"web",|conn|
            verify_web_dispatch_binding_in(conn,"start-test",1,&WebBindingDirectory::open(&original_dir)?,&binding_test_runtime()))
    });
    let (recovery_path,recovery_root) = (path.clone(),root.clone());
    let recovery = thread::spawn(move || {
        barrier.wait();
        claim_web_recovery(&recovery_path,"start-test",1,true,|conn|recovery_test_reconstruct(conn,&recovery_root))
    });
    let mut original = original.join().unwrap();
    let mut recovery = recovery.join().unwrap();
    assert_ne!(original.is_ok(),recovery.is_ok(),"only one branch owner may exist");
    if let Ok(guard) = &mut original { guard.disarm(); }
    if let Ok(owned) = &mut recovery { owned.guard.disarm(); }
    drop(original);
    drop(recovery);
    assert!(claim_web_recovery(&path,"start-test",1,true,|conn|recovery_test_reconstruct(conn,&root)).is_err());
    assert_eq!(connection.query_row("SELECT status FROM sentinel_scans",[],|r|r.get::<_,String>(0)).unwrap(),"scanning");
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn web_recovery_rejects_altered_artifacts_and_incomplete_native_backend_matrix() {
    for alteration in ["task","targets","extra","plan-row","plan-backend","plan-attempt","path","key"] {
        let (root,path,work_dir,connection) = recovery_test_fixture();
        match alteration {
            "task" => fs::write(work_dir.join("task.json"),"{}").unwrap(),
            "targets" => fs::write(work_dir.join("targets.json"),"[]").unwrap(),
            "extra" => fs::write(work_dir.join("worker-output.json"),"{}").unwrap(),
            "key" => fs::remove_file(work_dir.join(WEB_DISPATCH_KEY_FILE)).unwrap(),
            "path" => { connection.execute("UPDATE sentinel_scan_attempts SET work_dir='/tmp/unowned'",[]).unwrap(); },
            "plan-row" => { connection.execute("UPDATE sentinel_scan_attempts SET backend_plan_json=json_insert(backend_plan_json,'$.targets[#]',json('{}'))",[]).unwrap(); },
            "plan-backend" => { connection.execute("UPDATE sentinel_scan_attempts SET backend_plan_json=json_set(backend_plan_json,'$.targets[0].backend','strix')",[]).unwrap(); },
            "plan-attempt" => { connection.execute("UPDATE sentinel_scan_attempts SET backend_plan_json=json_set(backend_plan_json,'$.attemptNumber',2)",[]).unwrap(); },
            _ => unreachable!(),
        }
        let before = web_start_snapshot(&connection);
        assert!(claim_web_recovery(&path,"start-test",1,true,|conn|recovery_test_reconstruct(conn,&root)).is_err(),"{alteration}");
        assert_eq!(web_start_snapshot(&connection),before);
        recovery_test_unclaimed(&connection);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn web_recovery_tool_admission_never_substitutes_missing_or_nonexecutable_candidates() {
    let executable = serde_json::json!({"state":"present","stamp":{"unix":{"mode":0o100755}}});
    let mut descriptor = serde_json::json!({"toolchain":{"nodeCandidates":[executable.clone()],"browserCandidates":[executable]}});
    web_recovery_tools_present(&descriptor).unwrap();
    descriptor["toolchain"]["nodeCandidates"] = serde_json::json!([{"state":"missing"}]);
    assert_eq!(web_recovery_tools_present(&descriptor).unwrap_err(),"web_recovery_tool_missing:nodeCandidates");
    #[cfg(unix)] {
        descriptor["toolchain"]["nodeCandidates"] = serde_json::json!([{"state":"present","stamp":{"unix":{"mode":0o100644}}}]);
        assert!(web_recovery_tools_present(&descriptor).is_err());
    }
    assert!(web_recovery_tools_present(&serde_json::json!({})).is_err());
}

#[cfg(unix)]
#[test]
fn web_recovery_diagnostic_log_cannot_redirect_worker_writes() {
    let (root,path,work_dir,connection) = recovery_test_fixture();
    let outside = root.join("not-a-task-log");
    fs::write(&outside,"keep me unchanged").unwrap();
    std::os::unix::fs::symlink(&outside,work_dir.join("oviraptor-runner.log")).unwrap();
    assert!(claim_web_recovery(&path,"start-test",1,true,|conn|recovery_test_reconstruct(conn,&root)).is_err());
    recovery_test_unclaimed(&connection);
    assert_eq!(fs::read_to_string(outside).unwrap(),"keep me unchanged");
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn web_recovery_revalidates_current_identity_and_denies_changed_auth_without_rewriting_it() {
    for change in [
        "UPDATE browser_auth_sessions SET owner_scan_id='another-task'",
        "UPDATE browser_auth_sessions SET status='invalid'",
        "UPDATE browser_auth_sessions SET expires_at='2000-01-01T00:00:00Z'",
        "UPDATE browser_auth_sessions SET session_json=json_set(session_json,'$.scopeHosts',json('[\"other.example.test\"]'))",
        "UPDATE browser_auth_sessions SET session_json=json_set(session_json,'$.headers.Authorization','Bearer replaced')",
    ] {
        let (root,path,work_dir,mut connection) = recovery_test_fixture_with(seed_binding_auth);
        let transaction = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
        recovery_test_reconstruct(&transaction,&root).unwrap();
        transaction.rollback().unwrap();
        let original = fs::read(work_dir.join("auth-sessions.json")).unwrap();
        connection.execute_batch(change).unwrap();
        let before = web_start_snapshot(&connection);
        assert!(claim_web_recovery(&path,"start-test",1,true,|conn|recovery_test_reconstruct(conn,&root)).is_err(),"{change}");
        recovery_test_unclaimed(&connection);
        assert_eq!(web_start_snapshot(&connection),before);
        assert_eq!(fs::read(work_dir.join("auth-sessions.json")).unwrap(),original);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn web_recovery_real_reconstruction_is_readonly_and_preserves_native_web_inputs() {
    let (root,_,mut connection) = web_start_fixture();
    connection.execute("UPDATE config_profiles SET settings_json=?1",[serde_json::json!({
        "modelName":"fixture","modelApiKey":"synthetic-recovery-key","modelApiBase":"https://model.example.test/v1",
        "modelDeployment":"cloud","agentPromptAuditMode":"off",
    }).to_string()]).unwrap();
    let worker = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/workers/9_frontend_runtime_probe.cjs");
    let transaction = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
    let model = model_runtime_env(&sentinel_settings(&transaction)).unwrap();
    let runtime = resolve_agent_web_pipeline_runtime(&transaction,"start-test",&model.deployment,worker.clone()).unwrap();
    transaction.execute("UPDATE sentinel_scan_contexts SET policy_json=?1",[runtime.web_policy.to_string()]).unwrap();
    let mut startup = prepare_binding_test_startup(&transaction,&root);
    prepare_recovery_test_files(&startup);
    let descriptor = web_dispatch_runtime_binding(&transaction,&runtime,&model,&sentinel_runtime_path(root.parent().unwrap())).unwrap();
    let expected_tool_result = web_recovery_tools_present(&descriptor);
    register_web_dispatch_binding_in(&transaction,&startup,&descriptor).unwrap();
    register_private_web_mode_on(&transaction,&startup,&descriptor).unwrap();
    let work_dir = startup.files.path.clone();
    startup.files.preserve = true;
    transaction.commit().unwrap();
    let before = web_start_snapshot(&connection);
    let transaction = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
    let result = reconstruct_web_recovery_in(&transaction,&root,"start-test",1,&worker);
    match expected_tool_result {
        Ok(()) => {
            let inputs = result.unwrap();
            assert_eq!(inputs.work_dir,work_dir);
            assert_eq!(inputs.runtime.worker,worker);
            assert_eq!(inputs.targets.len(),2);
            assert!(inputs.auth_session_path.is_none());
            assert!(inputs.admission.claimed_guard.is_none());
        },
        Err(expected) => assert!(matches!(result,Err(error) if error==expected)),
    }
    transaction.rollback().unwrap();
    assert_eq!(web_start_snapshot(&connection),before);
    recovery_test_unclaimed(&connection);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}
