fn workbench_start_fixture() -> (PathBuf,rusqlite::Connection,WorkbenchStartRecord,ScanBackendPlan,String) {
    let root = std::env::temp_dir().canonicalize().unwrap().join(format!("oviraptor-workbench-start-{}",Uuid::new_v4()));
    let db_path = db::initialize(&root).unwrap();
    let connection = db::open(&db_path).unwrap();
    connection.execute("UPDATE config_profiles SET settings_json=json_set(settings_json,'$.modelProfiles',json(?1),'$.activeModelProfileId','source-fixture') WHERE id=(SELECT id FROM config_profiles ORDER BY is_default DESC,id LIMIT 1)",
        [json!([{"id":"source-fixture","llm":"fixture-source-model","apiKey":"fixture-source-key","apiBase":"http://127.0.0.1:9/v1","deployment":"cloud"}]).to_string()]).unwrap();
    let scope = Uuid::new_v4().to_string();
    connection.execute("INSERT INTO projects(id,name) VALUES(1,'Workbench')",[]).unwrap();
    connection.execute("INSERT INTO browser_auth_sessions(id,project_id,draft_scope_id,name,entry_url) VALUES('wb-identity',1,?1,'Test','https://workbench.example.test')",[&scope]).unwrap();
    let expiry=(chrono::Utc::now()+chrono::Duration::hours(1)).to_rfc3339();
    let document=json!({"schemaVersion":1,"id":"wb-identity","projectId":1,
        "scopeHosts":["workbench.example.test"],"expiresAt":expiry,
        "cookies":[{"name":"session_id","value":"fixture-account-a","domain":"workbench.example.test","path":"/"}],
        "headers":{},"localStorage":{},"sessionStorage":{}});
    connection.execute("UPDATE browser_auth_sessions SET status='valid',expires_at=?1,session_json=?2 WHERE id='wb-identity'",
        params![expiry,document.to_string()]).unwrap();
    let prepared=workbench_current_browser_auth(&connection,&["wb-identity".into()],1,None).unwrap();
    fs::create_dir(root.join("attempt-0001")).unwrap();
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(root.join("attempt-0001"),fs::Permissions::from_mode(0o700)).unwrap();
    }
    crate::auth_session::write_session_document(&root.join("attempt-0001/auth-session.json"),&prepared).unwrap();
    let mut policy = build_web_investigation_policy(Some("deep"),None,vec!["wb-identity".into()],&[],"","workbench",None).unwrap();
    policy["maxCritical"] = json!(0);
    policy["maxHigh"] = json!(5);
    policy["blockRelease"] = json!(false);
    let record: WorkbenchStartRecord = serde_json::from_value(json!({
        "scanId":"wb-start", "projectId":1, "projectName":"Workbench", "taskName":"Combined",
        "scanType":"greybox", "attempt":1, "urls":["https://workbench.example.test"],
        "sourcePath":"", "scopeMode":"diff", "diffBase":"fixture-base", "skills":"", "environment":"internal", "authProfileName":"Test",
        "authType":"browser_session", "authSessionIds":["wb-identity"], "authenticated":true,
        "ciProvider":"", "repositoryUrl":"", "branch":"", "commitSha":"", "buildId":"",
        "policy":policy,"llmPolicy":source_model_policy(&model_runtime_env(&sentinel_settings(&connection)).unwrap())
    })).unwrap();
    let plan = ScanBackendPlan {scan_id:record.scan_id.clone(),attempt_number:1,
        targets: vec![ScanTargetBackend {url:record.urls[0].clone(),backend:AgentBackendKind::Native,selection_reason:"fixture".into()}],requires_node:true,requires_browser:true};
    (root,connection,record,plan,scope)
}

fn publish_workbench_fixture(root: &Path, connection: &rusqlite::Connection, record: &WorkbenchStartRecord, plan: &ScanBackendPlan, scope: &str, reuse: bool) -> Result<SentinelScan,String> {
    let mut record = record.clone();
    if reuse && record.retry_basis.is_none() {
        record.retry_basis = Some(capture_workbench_retry_basis(connection,&record.scan_id)?);
    }
    let work=root.join(format!("attempt-{:04}",record.attempt));
    if !work.exists() {
        fs::create_dir(&work).unwrap();
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&work,fs::Permissions::from_mode(0o700)).unwrap();
        }
        if !record.auth_session_ids.is_empty() {
            let document=workbench_current_browser_auth(connection,&record.auth_session_ids,record.project_id,reuse.then_some(record.scan_id.as_str()))?;
            crate::auth_session::write_session_document(&work.join("auth-session.json"),&document)?;
        }
    }
    if !work.join("task.json").exists() {
        let task = json!({"scanId":record.scan_id,"projectId":record.project_id,"taskName":record.task_name,
            "scanType":record.scan_type,"attempt":record.attempt,"urls":record.urls,"sourcePath":record.source_path,
            "scanMode":record.policy["webModeCeiling"],"scopeMode":record.scope_mode,"diffBase":record.diff_base,
            "maxBudgetUsd":record.policy["maxBudgetUsd"],"environment":record.environment,
            "authSessionId":record.policy["authSessionId"],"authSessionIds":record.auth_session_ids,
            "ciProvider":record.ci_provider,"repositoryUrl":record.repository_url,"branch":record.branch,
            "commitSha":record.commit_sha,"buildId":record.build_id,"runtimePolicy":{"backend":"native-agent"},"policy":record.policy,
            "llmPolicy":record.llm_policy});
        fs::write(work.join("task.json"),serde_json::to_vec(&task).unwrap()).unwrap();
    }
    publish_workbench_start(connection,&record,reuse,scope,&root.join(format!("attempt-{:04}",record.attempt)),plan,root.join("fixture-worker.cjs"),&model_runtime_env(&sentinel_settings(connection))?)
        .map(|(scan,_)| scan)
}

#[test]
fn workbench_start_budget_is_finite_and_preserves_explicit_ceiling() {
    for invalid in [f64::NAN,f64::INFINITY,f64::NEG_INFINITY,0.0,-1.0,10_000.01] {
        assert!(workbench_budget(Some(invalid)).is_err());
    }
    assert_eq!(workbench_budget(Some(12.5)).unwrap(),Some(12.5));
    assert_eq!(workbench_budget(Some(10_000.0)).unwrap(),Some(10_000.0));
    assert_eq!(workbench_budget(None).unwrap(),None);
}

#[test]
fn workbench_start_commits_identity_attempt_targets_and_branches_together() {
    let (root,connection,mut record,plan,scope) = workbench_start_fixture();
    let repo = root.join("repository");
    fs::create_dir(&repo).unwrap();
    fs::write(repo.join("example.js"),"const local = true;\n").unwrap();
    record.source_path = repo.to_string_lossy().into_owned();
    let scan = publish_workbench_fixture(&root,&connection,&record,&plan,&scope,false).unwrap();
    assert_eq!(scan.status,"scanning");
    let counts: (i64,i64,i64,i64) = connection.query_row("SELECT (SELECT count(*) FROM sentinel_scan_attempts),(SELECT count(*) FROM sentinel_targets),(SELECT count(*) FROM native_scan_branches),(SELECT count(*) FROM native_branch_dispatches)",[],|r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(counts,(1,2,2,2));
    let bound: String = connection.query_row("SELECT owner_scan_id FROM browser_auth_sessions WHERE id='wb-identity'",[],|r| r.get(0)).unwrap();
    assert_eq!(bound,"wb-start");
    let before = web_start_snapshot(&connection);
    assert!(publish_workbench_fixture(&root,&connection,&record,&plan,&scope,false).is_err());
    assert_eq!(before,web_start_snapshot(&connection));
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn workbench_start_write_failures_and_ignored_writes_roll_back_all_surfaces() {
    for table in ["sentinel_scans","sentinel_scan_attempts","sentinel_scan_contexts","sentinel_targets","sentinel_checkpoints","native_scan_branches","native_branch_dispatches"] {
        for action in ["RAISE(ABORT,'injected')","RAISE(IGNORE)"] {
            let (root,connection,record,plan,scope) = workbench_start_fixture();
            connection.execute_batch(&format!("CREATE TRIGGER wb_fault BEFORE INSERT ON {table} BEGIN SELECT {action}; END;")).unwrap();
            let before = web_start_snapshot(&connection);
            let result = publish_workbench_fixture(&root,&connection,&record,&plan,&scope,false);
            assert!(result.is_err(),"{table}/{action} must reject publication");
            assert_eq!(before,web_start_snapshot(&connection),"{table}/{action}");
            let dispatches: i64 = connection.query_row("SELECT count(*) FROM native_branch_dispatches",[],|r| r.get(0)).unwrap();
            assert_eq!(dispatches,0);
            connection.execute_batch("DROP TRIGGER wb_fault").unwrap();
            publish_workbench_fixture(&root,&connection,&record,&plan,&scope,false).unwrap();
            drop(connection);
            fs::remove_dir_all(root).unwrap();
        }
    }
}

#[test]
fn workbench_start_rechecks_project_fuse_and_identity_before_publication() {
    for change in [
        "UPDATE projects SET status='archived' WHERE id=1;",
        "UPDATE browser_auth_sessions SET owner_scan_id='another-task',draft_scope_id='' WHERE id='wb-identity';",
        "INSERT INTO sentinel_fuse_zone(project_id,normalized_url,url,company) VALUES(1,'https://workbench.example.test','https://workbench.example.test','Workbench');",
    ] {
        let (root,connection,record,plan,scope) = workbench_start_fixture();
        connection.execute_batch(change).unwrap();
        let before = web_start_snapshot(&connection);
        assert!(publish_workbench_fixture(&root,&connection,&record,&plan,&scope,false).is_err());
        assert_eq!(before,web_start_snapshot(&connection));
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn workbench_start_late_tampering_is_detected_before_commit() {
    for change in [
        "DELETE FROM sentinel_scan_contexts;",
        "UPDATE sentinel_scan_contexts SET authenticated=0;",
        "UPDATE sentinel_scan_attempts SET backend_plan_json='{}';",
        "DELETE FROM sentinel_targets;",
        "UPDATE sentinel_scans SET task_path='old.json';",
        "UPDATE browser_auth_sessions SET owner_scan_id='other';",
        "UPDATE sentinel_targets SET last_attempt_number=0;",
    ] {
        let (root,connection,record,plan,scope) = workbench_start_fixture();
        connection.execute_batch(&format!("CREATE TRIGGER wb_tamper AFTER INSERT ON native_branch_dispatches BEGIN {change} END;")).unwrap();
        let before = web_start_snapshot(&connection);
        assert!(publish_workbench_fixture(&root,&connection,&record,&plan,&scope,false).is_err(),"{change}");
        assert_eq!(before,web_start_snapshot(&connection));
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn workbench_start_retry_failure_preserves_prior_attempt_and_rejects_scope_change() {
    let (root,connection,mut record,mut plan,scope) = workbench_start_fixture();
    publish_workbench_fixture(&root,&connection,&record,&plan,&scope,false).unwrap();
    connection.execute("UPDATE sentinel_scans SET status='failed' WHERE id='wb-start'",[]).unwrap();
    record.attempt=2;
    plan.attempt_number=2;
    let before=web_start_snapshot(&connection);
    connection.execute_batch("CREATE TRIGGER wb_retry_fault BEFORE INSERT ON native_scan_branches WHEN NEW.attempt_number=2 BEGIN SELECT RAISE(ABORT,'retry fail'); END;").unwrap();
    assert!(publish_workbench_fixture(&root,&connection,&record,&plan,&scope,true).is_err());
    assert_eq!(before,web_start_snapshot(&connection));
    connection.execute_batch("DROP TRIGGER wb_retry_fault").unwrap();
    record.scan_type="code".into();
    assert!(publish_workbench_fixture(&root,&connection,&record,&plan,&scope,true).is_err());
    assert_eq!(before,web_start_snapshot(&connection));
    record.scan_type="greybox".into();
    publish_workbench_fixture(&root,&connection,&record,&plan,&scope,true).unwrap();
    let old_path: String = connection.query_row("SELECT work_dir FROM sentinel_scan_attempts WHERE scan_id='wb-start' AND attempt_number=1",[],|r| r.get(0)).unwrap();
    assert!(old_path.ends_with("attempt-0001"));
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn workbench_start_effective_runtime_keeps_budget_mode_and_ci_policy() {
    let (root,connection,mut record,plan,scope) = workbench_start_fixture();
    record.policy = build_web_investigation_policy(Some("quick"),workbench_budget(Some(2.5)).unwrap(),record.auth_session_ids.clone(),&[],"operator instruction","workbench",None).unwrap();
    record.policy["maxCritical"]=json!(0);
    record.policy["maxHigh"]=json!(1);
    record.policy["blockRelease"]=json!(true);
    let mut model=model_runtime_env(&sentinel_settings(&connection)).unwrap();
    model.deployment="local".into();
    let (_,runtime)=publish_workbench_start(&connection,&record,false,&scope,&root.join("attempt-0001"),&plan,PathBuf::new(),&model).unwrap();
    assert_eq!(runtime.adaptive.max_budget_usd,Some(2.5));
    assert_eq!(runtime.adaptive.max_mode,"quick");
    assert_eq!(runtime.web_policy["blockRelease"],true);
    assert_eq!(runtime.web_policy["maxHigh"],1);
    assert_eq!(runtime.web_policy["additionalInstruction"],"operator instruction");
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn workbench_start_owned_file_cleanup_preserves_previous_attempt_credentials() {
    let root=std::env::temp_dir().join(format!("oviraptor-wb-files-{}",Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    let mut previous=WebStartFiles::allocate(&root,"wb-files",1).unwrap();
    previous.preserve=true;
    fs::write(previous.path.join("auth-session.json"),"previous credential fixture").unwrap();
    fs::write(previous.path.join("task.json"),"previous plan").unwrap();
    {
        let failed=WebStartFiles::allocate(&root,"wb-files",2).unwrap();
        fs::write(failed.path.join("auth-session.json"),"new credential fixture").unwrap();
        fs::write(failed.path.join("task.json"),"unpublished plan").unwrap();
        assert!(WebStartFiles::allocate(&root,"wb-files",2).is_err());
    }
    assert!(!root.join("attempt-0002").exists());
    assert_eq!(fs::read_to_string(previous.path.join("auth-session.json")).unwrap(),"previous credential fixture");
    assert_eq!(fs::read_to_string(previous.path.join("task.json")).unwrap(),"previous plan");
    drop(previous);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn workbench_start_spawn_failure_is_local_to_branch_and_does_not_fake_dispatch() {
    let (root,db_path,connection)=workbench_admission_fixture();
    record_workbench_spawn_result(&db_path,"wb-start",1,"source",Err("injected spawn failure".into())).unwrap();
    let status: String=connection.query_row("SELECT status FROM sentinel_scans WHERE id='wb-start'",[],|r| r.get(0)).unwrap();
    assert_eq!(status,"scanning","the independent Web branch is still pending");
    let report: String=connection.query_row("SELECT report_json FROM native_scan_branches WHERE scan_id='wb-start' AND branch='source'",[],|r| r.get(0)).unwrap();
    assert_eq!(json(report)["executionStarted"],false);
    let claims: i64=connection.query_row("SELECT count(*) FROM native_branch_dispatches WHERE claim_id<>''",[],|r| r.get(0)).unwrap();
    assert_eq!(claims,0);
    record_workbench_spawn_result(&db_path,"wb-start",1,"web",Err("second spawn failure".into())).unwrap();
    let status: String=connection.query_row("SELECT status FROM sentinel_scans WHERE id='wb-start'",[],|r| r.get(0)).unwrap();
    assert_eq!(status,"failed");
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn workbench_start_concurrent_publications_have_one_winner_and_one_identity_owner() {
    let (root,connection,record,plan,scope)=workbench_start_fixture();
    let db_path=PathBuf::from(connection.path().unwrap());
    let barrier=Arc::new(std::sync::Barrier::new(2));
    let callers=(0..2).map(|_| {
        let (root,db_path,record,plan,scope,barrier)=(root.clone(),db_path.clone(),record.clone(),plan.clone(),scope.clone(),Arc::clone(&barrier));
        std::thread::spawn(move || {
            let connection=db::open(&db_path).unwrap();
            barrier.wait();
            publish_workbench_fixture(&root,&connection,&record,&plan,&scope,false).is_ok()
        })
    }).collect::<Vec<_>>();
    assert_eq!(callers.into_iter().filter_map(|c| c.join().unwrap().then_some(())).count(),1);
    let counts: (i64,i64,i64)=connection.query_row("SELECT (SELECT count(*) FROM sentinel_scan_attempts),(SELECT count(*) FROM native_branch_dispatches),(SELECT count(*) FROM browser_auth_sessions WHERE owner_scan_id='wb-start')",[],|r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(counts,(1,1,1));
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn workbench_start_source_only_publishes_without_a_web_worker_or_web_branch() {
    let (root,connection,mut record,mut plan,scope)=workbench_start_fixture();
    let repo=root.join("source-only");
    fs::create_dir(&repo).unwrap();
    fs::write(repo.join("main.rs"),"fn main() {}\n").unwrap();
    record.source_path=repo.to_string_lossy().into_owned();
    record.scan_type="code".into();
    record.urls.clear();
    record.auth_session_ids.clear();
    record.authenticated=false;
    record.auth_type="none".into();
    record.policy=json!({"webModeCeiling":"quick"});
    plan.targets.clear();
    plan.requires_browser=false;
    plan.requires_node=false;
    fs::write(root.join("attempt-0001/task.json"),json!({"scanId":record.scan_id,"attempt":record.attempt,
        "sourcePath":record.source_path,"scanType":record.scan_type,"scopeMode":record.scope_mode,"diffBase":record.diff_base,
        "scanMode":"quick","llmPolicy":record.llm_policy,"policy":record.policy}).to_string()).unwrap();
    publish_workbench_start(&connection,&record,false,&scope,&root.join("attempt-0001"),&plan,PathBuf::new(),&model_runtime_env(&sentinel_settings(&connection)).unwrap()).unwrap();
    let branches: String=connection.query_row("SELECT group_concat(branch) FROM native_scan_branches",[],|r| r.get(0)).unwrap();
    assert_eq!(branches,"source");
    let inventory=sentinel_findings_in(&connection,&record.scan_id,Some("source_inventory")).unwrap();
    assert_eq!(inventory.len(),1,"admitted startup, not the findings reader, owns inventory collection");
    let saved:JsonValue=serde_json::from_str(&inventory[0].record_json).unwrap();
    assert_eq!(saved["lineStats"]["physical"],1);
    let before=bundle_native_snapshot(&connection);
    fs::write(repo.join("main.rs"),"// changed after startup\nfn main() {}\n").unwrap();
    assert_eq!(sentinel_findings_in(&connection,&record.scan_id,Some("source_inventory")).unwrap()[0].record_json,inventory[0].record_json);
    assert_eq!(bundle_native_snapshot(&connection),before);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}
