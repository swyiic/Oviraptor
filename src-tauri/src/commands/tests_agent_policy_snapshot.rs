fn save_test_agent_policy(
    connection:&rusqlite::Connection,settings:JsonValue,
) -> crate::configuration_policies::PolicyReference {
    let tx=rusqlite::Transaction::new_unchecked(connection,rusqlite::TransactionBehavior::Immediate).unwrap();
    let profile:i64=tx.query_row("SELECT id FROM config_profiles ORDER BY is_default DESC,id LIMIT 1",[],|r|r.get(0)).unwrap();
    let current=crate::configuration_policies::read_profile(&tx,profile).unwrap();
    let reference=crate::configuration_policies::reference(&current,"agent").unwrap();
    let saved=crate::configuration_policies::save(&tx,profile,"agent",&settings,
        &crate::configuration_policies::project(&current,"agent").unwrap(),reference.as_ref(),reference.is_none()).unwrap();
    tx.commit().unwrap();
    saved
}

// Follow the real Web startup's publication/consumption/sealing order, without
// launching a worker, browser, target request or model. The final test uses the
// actual branch claim and live private-binding preflight, not a fixture grant.
fn publish_test_frozen_agent_web_start(
    connection:&mut rusqlite::Connection,root:&Path,worker:&Path,
) -> (PathBuf,JsonValue) {
    publish_test_agent_web_start(connection,root,worker,true)
}

fn publish_test_agent_web_start(
    connection:&mut rusqlite::Connection,root:&Path,worker:&Path,freeze:bool,
) -> (PathBuf,JsonValue) {
    let tx=connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
    let mut startup=prepare_web_startup_in(&tx,root,"start-test",WebStartMode::Confirm).unwrap();
    if freeze { freeze_agent_policy_in(&tx,&startup.scan_id,startup.attempt.into(),None).unwrap(); }
    let settings=agent_policy_settings_for_attempt(&tx,&startup.scan_id,startup.attempt.into()).unwrap();
    let model=model_runtime_env(&settings).unwrap();
    let runtime=resolve_agent_web_pipeline_runtime_for_attempt(&tx,&startup.scan_id,startup.attempt.into(),&model.deployment,worker.into()).unwrap();
    tx.execute("UPDATE sentinel_scan_contexts SET policy_json=?1 WHERE scan_id=?2",params![runtime.web_policy.to_string(),startup.scan_id]).unwrap();
    let targets=startup.targets.iter().map(|(company,url)|json!({"company":company,"url":url})).collect::<Vec<_>>();
    fs::write(startup.files.path.join("task.json"),json!({"scanId":startup.scan_id,"targets":targets,
        "effectiveWebPolicy":runtime.web_policy,"agentPolicySnapshot":runtime.agent_policy_snapshot}).to_string()).unwrap();
    fs::write(startup.files.path.join("targets.json"),json!(targets).to_string()).unwrap();
    fs::write(startup.files.path.join("targets.txt"),startup.targets.iter().map(|(_,url)|url.as_str()).collect::<Vec<_>>().join("\n")).unwrap();
    fs::write(startup.files.path.join("agent-instruction.md"),"owned frozen-policy startup").unwrap();
    persist_web_startup_in(&tx,&startup,"initializing web","",&runtime.web_policy).unwrap();
    let descriptor=web_dispatch_runtime_binding(&tx,&runtime,&model,&sentinel_runtime_path(root.parent().unwrap())).unwrap();
    register_web_dispatch_binding_in(&tx,&startup,&descriptor).unwrap();
    register_private_web_mode_on(&tx,&startup,&descriptor).unwrap();
    startup.files.preserve=true;
    let work=startup.files.path.clone();
    tx.commit().unwrap();
    (work,descriptor)
}

#[test]
fn scan_agent_policy_real_web_runtime_and_claim_keep_revision_after_owner_edit() {
    let (root,db_path,mut connection)=web_start_fixture();
    connection.execute("UPDATE config_profiles SET settings_json=?1",[json!({
        "modelName":"fixture-policy-model","modelApiKey":"synthetic-policy-key",
        "modelApiBase":"http://127.0.0.1:9/v1","modelDeployment":"cloud",
        "agentStandardTokenLimit":999999,"agentStandardTimeout":999,
    }).to_string()]).unwrap();
    let legacy:String=connection.query_row("SELECT settings_json FROM config_profiles ORDER BY is_default DESC,id LIMIT 1",[],|r|r.get(0)).unwrap();
    let first=save_test_agent_policy(&connection,json!({"agentStandardTokenLimit":123456,"agentStandardTimeout":321,
        "agentStandardRequestLimit":9,"agentFrontendPacketMode":"custom","agentFrontendPacketBudgetKb":13}));
    let worker=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/workers/9_frontend_runtime_probe.cjs");
    let (work,descriptor)=publish_test_frozen_agent_web_start(&mut connection,&root,&worker);
    assert_eq!(descriptor["runtime"]["adaptive"]["standard_tokens"],123456);
    assert_eq!(descriptor["runtime"]["adaptive"]["standard_timeout"],321);
    assert_eq!(descriptor["runtime"]["packet_budget"],13*1024);
    assert_eq!(descriptor["runtime"]["agent_policy_snapshot"]["policyReference"],json!(first));
    let frozen:String=connection.query_row("SELECT snapshot_json FROM scan_agent_policy_snapshots WHERE scan_id='start-test' AND attempt_number=1",[],|r|r.get(0)).unwrap();
    let second=save_test_agent_policy(&connection,json!({"agentStandardTokenLimit":654321,"agentStandardTimeout":654,
        "agentStandardRequestLimit":18,"agentFrontendPacketMode":"compact","agentFrontendPacketBudgetKb":4,
        "agentPromptAuditMode":"full"}));
    assert_eq!(second.revision,first.revision+1);
    let restored=resolve_agent_web_pipeline_runtime(&connection,"start-test","cloud",worker.clone()).unwrap();
    assert_eq!(restored.adaptive.standard_tokens,123456);
    assert_eq!(restored.adaptive.standard_timeout,321);
    assert_eq!(restored.packet_budget,13*1024);
    let model=agent_model_runtime_for_scan(&connection,"start-test").unwrap();
    assert_eq!(model.prompt_audit_mode,"off");
    let restored_descriptor=web_dispatch_runtime_binding(&connection,&restored,&model,&sentinel_runtime_path(root.parent().unwrap())).unwrap();
    assert!(binding_test_changed_fields(&descriptor,&restored_descriptor).is_empty());
    let mut guard=NativeBranchGuard::claim_with_preflight(&db_path,"start-test",1,"web",|db|
        verify_live_web_dispatch_binding_in(db,"start-test",1,&work,&worker,root.parent().unwrap())).unwrap();
    guard.disarm();drop(guard);
    assert!(NativeBranchGuard::claim_with_preflight(&db_path,"start-test",1,"web",|db|
        verify_live_web_dispatch_binding_in(db,"start-test",1,&work,&worker,root.parent().unwrap())).is_err(),
        "a stable policy must not reset the durable dispatch claim");
    assert_eq!(connection.query_row("SELECT snapshot_json FROM scan_agent_policy_snapshots WHERE scan_id='start-test' AND attempt_number=1",[],|r|r.get::<_,String>(0)).unwrap(),frozen);
    assert_eq!(connection.query_row("SELECT settings_json FROM config_profiles ORDER BY is_default DESC,id LIMIT 1",[],|r|r.get::<_,String>(0)).unwrap(),legacy);
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_runs",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn scan_agent_policy_web_resume_inherits_parent_and_does_not_borrow_current_revision() {
    let (root,_,mut connection)=web_start_fixture();
    connection.execute("UPDATE config_profiles SET settings_json=?1",[json!({"modelName":"fixture-policy-model",
        "modelApiKey":"synthetic-policy-key","modelApiBase":"http://127.0.0.1:9/v1","modelDeployment":"cloud"}).to_string()]).unwrap();
    let first=save_test_agent_policy(&connection,json!({"agentStandardTokenLimit":123456}));
    let worker=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/workers/9_frontend_runtime_probe.cjs");
    publish_test_frozen_agent_web_start(&mut connection,&root,&worker);
    save_test_agent_policy(&connection,json!({"agentStandardTokenLimit":654321}));
    connection.execute("UPDATE sentinel_scans SET status='paused' WHERE id='start-test'",[]).unwrap();
    connection.execute("UPDATE sentinel_scan_attempts SET status='paused' WHERE scan_id='start-test'",[]).unwrap();
    connection.execute("UPDATE sentinel_targets SET status='queued' WHERE scan_id='start-test'",[]).unwrap();
    {
        let tx=connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
        let startup=prepare_web_startup_in(&tx,&root,"start-test",WebStartMode::Resume).unwrap();
        assert_eq!(startup.execution_mode,"resume");
        let snapshot=freeze_agent_policy_in(&tx,&startup.scan_id,startup.attempt.into(),Some(startup.previous_attempt)).unwrap();
        assert_eq!(snapshot.policy_reference,Some(first));
        let runtime=resolve_agent_web_pipeline_runtime_for_attempt(&tx,"start-test",startup.attempt.into(),"cloud",worker).unwrap();
        assert_eq!(runtime.adaptive.standard_tokens,123456);
        // Preparation is not execution/publication; rollback cannot leave an
        // authorized new branch or a replacement of the parent's policy.
        drop(startup);
        tx.rollback().unwrap();
    }
    assert_eq!(connection.query_row("SELECT count(*) FROM scan_agent_policy_snapshots WHERE scan_id='start-test'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_runs",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn scan_agent_policy_real_source_runtime_freezes_budget_and_fresh_attempt_takes_new_revision() {
    let (root,connection,mut record,mut plan,scope)=source_ci_publication_fixture();
    let legacy:String=connection.query_row("SELECT settings_json FROM config_profiles ORDER BY is_default DESC,id LIMIT 1",[],|r|r.get(0)).unwrap();
    let first=save_test_agent_policy(&connection,json!({"agentDeepTokenLimit":1234567,"agentDeepTimeout":1234,
        "agentDeepRequestLimit":31,"agentFrontendPacketMode":"custom","agentFrontendPacketBudgetKb":14}));
    publish_workbench_fixture(&root,&connection,&record,&plan,&scope,false).unwrap();
    let (_,runtime,contract)=verify_source_runtime_contract(&connection,&record.scan_id,1,&root.join("attempt-0001")).unwrap();
    assert_eq!(contract["budget"]["tokenLimit"],1234567);
    assert_eq!(contract["budget"]["timeoutSeconds"],1234);
    assert_eq!(contract["agentPolicySnapshot"]["policyReference"],json!(first));
    assert_eq!(runtime.packet_budget,14*1024);
    let stored:String=connection.query_row("SELECT snapshot_json FROM scan_agent_policy_snapshots WHERE scan_id=?1 AND attempt_number=1",[&record.scan_id],|r|r.get(0)).unwrap();
    let second=save_test_agent_policy(&connection,json!({"agentDeepTokenLimit":7654321,"agentDeepTimeout":4321,
        "agentDeepRequestLimit":45,"agentFrontendPacketMode":"compact","agentPromptAuditMode":"full"}));
    let (model,runtime,restored)=verify_source_runtime_contract(&connection,&record.scan_id,1,&root.join("attempt-0001")).unwrap();
    assert_eq!(restored,contract);
    assert_eq!(runtime.adaptive.deep_tokens,1234567);
    assert_eq!(runtime.adaptive.deep_timeout,1234);
    assert_eq!(model.prompt_audit_mode,"off");
    for sql in ["UPDATE scan_agent_policy_snapshots SET snapshot_json='{}'","DELETE FROM scan_agent_policy_snapshots",
        "INSERT OR REPLACE INTO scan_agent_policy_snapshots SELECT * FROM scan_agent_policy_snapshots"] {
        assert!(connection.execute(sql,[]).is_err());
    }
    connection.execute("UPDATE sentinel_scans SET status='failed' WHERE id=?1",[&record.scan_id]).unwrap();
    record.attempt=2;plan.attempt_number=2;
    record.llm_policy=source_model_policy(&model_runtime_env(&sentinel_settings(&connection)).unwrap());
    publish_workbench_fixture(&root,&connection,&record,&plan,&scope,true).unwrap();
    let (model,runtime,next)=verify_source_runtime_contract(&connection,&record.scan_id,2,&root.join("attempt-0002")).unwrap();
    assert_eq!(next["agentPolicySnapshot"]["policyReference"],json!(second));
    assert_eq!(next["budget"]["tokenLimit"],7654321);
    assert_eq!(runtime.adaptive.deep_timeout,4321);
    assert_eq!(runtime.packet_budget,6*1024);
    assert_eq!(model.prompt_audit_mode,"full");
    assert_eq!(connection.query_row("SELECT snapshot_json FROM scan_agent_policy_snapshots WHERE scan_id=?1 AND attempt_number=1",[&record.scan_id],|r|r.get::<_,String>(0)).unwrap(),stored);
    assert_eq!(connection.query_row("SELECT settings_json FROM config_profiles ORDER BY is_default DESC,id LIMIT 1",[],|r|r.get::<_,String>(0)).unwrap(),legacy);
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_runs",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    // Credentials remain a live binding gate even with a valid policy snapshot.
    connection.execute("UPDATE config_profiles SET settings_json=json_set(settings_json,'$.modelProfiles[0].apiKey','rotated-fixture-key')",[]).unwrap();
    assert!(verify_source_runtime_contract(&connection,&record.scan_id,2,&root.join("attempt-0002")).is_err());
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn scan_agent_policy_unknown_history_is_readonly_and_never_backfilled_for_resume() {
    let (root,_,mut connection)=web_start_fixture();
    connection.execute("UPDATE config_profiles SET settings_json=?1",[json!({"modelName":"fixture-history-model","modelApiKey":"synthetic-history-key","modelApiBase":"http://127.0.0.1:9/v1","modelDeployment":"cloud"}).to_string()]).unwrap();
    let worker=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/workers/9_frontend_runtime_probe.cjs");
    let (work,_)=publish_test_agent_web_start(&mut connection,&root,&worker,false);
    assert!(read_agent_policy_snapshot_in(&connection,"start-test",1).unwrap().is_none());
    let before=web_start_snapshot(&connection);
    {
        let tx=connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
        let error=freeze_agent_policy_in(&tx,"start-test",2,Some(1)).err().unwrap();
        assert_eq!(error,"scan_agent_policy_history_unknown_requires_fresh_attempt");
        tx.rollback().unwrap();
    }
    assert!(!agent_policy_snapshot_storage_exists(&connection).unwrap());
    assert_eq!(web_start_snapshot(&connection),before);
    connection.execute("UPDATE config_profiles SET settings_json=json_set(settings_json,'$.agentStandardTokenLimit',123456)",[]).unwrap();
    let tx=connection.unchecked_transaction().unwrap();
    assert_eq!(verify_live_web_dispatch_binding_in(&tx,"start-test",1,&work,&worker,root.parent().unwrap()).unwrap_err(),"web_binding_inputs_changed",
        "unknown history keeps the original real runtime binding refusal instead of adopting today's policy");
    tx.rollback().unwrap();
    assert!(!agent_policy_snapshot_storage_exists(&connection).unwrap());
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_runs",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

fn save_test_web_owner(connection:&rusqlite::Connection,section:&str,settings:JsonValue) {
    let tx=rusqlite::Transaction::new_unchecked(connection,rusqlite::TransactionBehavior::Immediate).unwrap();
    let profile:i64=tx.query_row("SELECT id FROM config_profiles ORDER BY is_default DESC,id LIMIT 1",[],|r|r.get(0)).unwrap();
    let current=crate::configuration_policies::read_profile(&tx,profile).unwrap();
    let reference=crate::configuration_policies::reference(&current,section).unwrap();
    if matches!(section,"models"|"assetApis") {
        crate::configuration_connections::save(&tx,profile,section,&settings,
            &crate::configuration_connections::project(&current,section).unwrap(),reference.as_ref(),reference.is_none()).unwrap();
    } else {
        crate::configuration_policies::save(&tx,profile,section,&settings,
            &crate::configuration_policies::project(&current,section).unwrap(),reference.as_ref(),reference.is_none()).unwrap();
    }
    tx.commit().unwrap();
}

fn configure_test_web_model(connection:&rusqlite::Connection) {
    connection.execute("UPDATE config_profiles SET settings_json=?1",[json!({
        "modelProfiles":[{"id":"web-fixture","name":"Display name before","llm":"fixture-model",
            "apiBase":"http://127.0.0.1:9/v1","apiKey":"synthetic-model-key","deployment":"cloud"}],
        "activeModelProfileId":"web-fixture","agentProxyEnabled":true,
        "authorizedProxyPool":["ALL|http://127.0.0.1:9"],"noProxy":"127.0.0.1,localhost"
    }).to_string()]).unwrap();
    save_test_web_owner(connection,"models",crate::configuration_connections::project(&sentinel_settings(connection),"models").unwrap());
}

#[test]
fn web_binding_unrelated_adopted_owners_and_model_display_name_do_not_invalidate_real_recovery() {
    let (root,db_path,mut connection)=web_start_fixture();
    configure_test_web_model(&connection);
    for (section,settings) in [("assetApis",json!({"fofaKey":"synthetic-fofa-before"})),
        ("collection",json!({"maxPages":1})),("probe",json!({"probeRetries":0})),
        ("rules",json!({"negativeKeywords":["before"]}))] { save_test_web_owner(&connection,section,settings); }
    let legacy:String=connection.query_row("SELECT settings_json FROM config_profiles ORDER BY is_default DESC,id LIMIT 1",[],|r|r.get(0)).unwrap();
    let worker=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/workers/9_frontend_runtime_probe.cjs");
    let (work,original)=publish_test_frozen_agent_web_start(&mut connection,&root,&worker);
    for (section,settings) in [("assetApis",json!({"fofaKey":"synthetic-fofa-after"})),
        ("collection",json!({"maxPages":7})),("probe",json!({"probeRetries":3,"workers":17})),
        ("rules",json!({"negativeKeywords":["after"],"contentThreshold":0.7}))] {
        save_test_web_owner(&connection,section,settings);
        let tx=connection.unchecked_transaction().unwrap();
        verify_live_web_dispatch_binding_in(&tx,"start-test",1,&work,&worker,root.parent().unwrap()).unwrap();
        tx.rollback().unwrap();
    }
    let mut models=crate::configuration_connections::project(&sentinel_settings(&connection),"models").unwrap();
    models["modelProfiles"][0]["name"]=json!("Display name after");
    save_test_web_owner(&connection,"models",models);
    let runtime=resolve_agent_web_pipeline_runtime(&connection,"start-test","cloud",worker.clone()).unwrap();
    let model=agent_model_runtime_for_scan(&connection,"start-test").unwrap();
    let descriptor=web_dispatch_runtime_binding(&connection,&runtime,&model,&sentinel_runtime_path(root.parent().unwrap())).unwrap();
    assert!(binding_test_changed_fields(&original,&descriptor).is_empty(),"only the consumed execution inputs belong in the descriptor");
    let mut guard=NativeBranchGuard::claim_with_preflight(&db_path,"start-test",1,"web",|db|
        verify_live_web_dispatch_binding_in(db,"start-test",1,&work,&worker,root.parent().unwrap())).unwrap();
    guard.disarm();drop(guard);
    assert_eq!(connection.query_row("SELECT settings_json FROM config_profiles ORDER BY is_default DESC,id LIMIT 1",[],|r|r.get::<_,String>(0)).unwrap(),legacy);
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_runs",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn web_binding_projection_still_rejects_real_transport_worker_and_authority_changes_without_claim() {
    for change in ["endpoint","key","actual-model","proxy","worker","project-authority","target-scope"] {
        let (root,db_path,mut connection)=web_start_fixture();
        configure_test_web_model(&connection);
        let worker=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/workers/9_frontend_runtime_probe.cjs");
        let (work,_)=publish_test_frozen_agent_web_start(&mut connection,&root,&worker);
        let mut candidate=worker.clone();
        match change {
            "endpoint"|"key"|"actual-model"=>{
                let mut models=crate::configuration_connections::project(&sentinel_settings(&connection),"models").unwrap();
                let (field,value)=match change {"endpoint"=>("apiBase","http://127.0.0.1:8/v1"),"key"=>("apiKey","rotated-synthetic-key"),_=>("llm","different-fixture-model")};
                models["modelProfiles"][0][field]=json!(value);
                save_test_web_owner(&connection,"models",models);
            },
            "proxy"=>{connection.execute("UPDATE config_profiles SET settings_json=json_set(settings_json,'$.authorizedProxyPool',json('[\"ALL|http://127.0.0.1:8\"]'))",[]).unwrap();},
            "worker"=>{candidate=root.join("9_frontend_runtime_probe.cjs");let mut bytes=fs::read(&worker).unwrap();bytes.extend_from_slice(b"\n// changed fixture worker\n");fs::write(&candidate,bytes).unwrap();},
            "project-authority"=>{connection.execute("UPDATE projects SET status='archived' WHERE id=1",[]).unwrap();},
            _=>{connection.execute("UPDATE sentinel_targets SET url='http://127.0.0.1:9/replaced-scope' WHERE scan_id='start-test' AND company='one'",[]).unwrap();},
        }
        let result=NativeBranchGuard::claim_with_preflight(&db_path,"start-test",1,"web",|db|
            verify_live_web_dispatch_binding_in(db,"start-test",1,&work,&candidate,root.parent().unwrap()));
        assert!(result.is_err(),"{change}");
        assert_eq!(connection.query_row("SELECT claim_id FROM native_branch_dispatches WHERE scan_id='start-test' AND attempt_number=1 AND branch='web'",[],|r|r.get::<_,String>(0)).unwrap(),"");
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_runs",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}
