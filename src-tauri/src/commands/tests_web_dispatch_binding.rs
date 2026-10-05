fn binding_test_runtime() -> JsonValue {
    serde_json::json!({"model":{"api_key":"synthetic-only-api-key","api_base":"https://model.example.test"},
        "runtime":{"proxies":[["test","http://test:synthetic-proxy-secret@proxy.example.test"]],"packet_budget":4096},
        "settings":{"roleModels":{"reviewer":"independent"}},"build":"test-build","runtimePath":"/test-only"})
}

// Report field paths only: runtime descriptors can carry credentials even in
// an assertion failure. Keep diagnosis useful without printing their values.
fn binding_test_changed_fields(before: &JsonValue, after: &JsonValue) -> Vec<String> {
    fn walk(before: &JsonValue, after: &JsonValue, path: &str, fields: &mut Vec<String>) {
        if before == after { return; }
        match (before,after) {
            (JsonValue::Object(a),JsonValue::Object(b)) => {
                let keys = a.keys().chain(b.keys()).collect::<std::collections::BTreeSet<_>>();
                for key in keys { walk(&before[key],&after[key],&format!("{path}/{key}"),fields); }
            },
            (JsonValue::Array(a),JsonValue::Array(b)) if a.len()==b.len() => {
                for (index,(a,b)) in a.iter().zip(b).enumerate() { walk(a,b,&format!("{path}/{index}"),fields); }
            },
            _ => fields.push(path.into()),
        }
    }
    let mut fields = Vec::new();
    walk(before,after,"",&mut fields);
    fields
}

// Same production preparation/publication/sealing order as the Web command;
// no AppHandle, model, worker, browser, target or network access is required.
fn prepare_binding_test_startup(transaction: &rusqlite::Connection, root: &Path) -> WebStartup {
    let startup = prepare_web_startup_in(transaction,root,"start-test",WebStartMode::Confirm).unwrap();
    let policy: String = transaction.query_row("SELECT policy_json FROM sentinel_scan_contexts WHERE scan_id='start-test'",[],|r|r.get(0)).unwrap();
    let policy: JsonValue = serde_json::from_str(&policy).unwrap();
    for name in ["task.json","targets.json","targets.txt","agent-instruction.md"] {
        fs::write(startup.files.path.join(name),"test-owned startup artifact").unwrap();
    }
    fs::write(startup.files.path.join("task.json"),json!({"scanId":"start-test","effectiveWebPolicy":policy,
        "targets":startup.targets.iter().map(|(company,url)|json!({"company":company,"url":url})).collect::<Vec<_>>()}).to_string()).unwrap();
    if let Some(auth) = web_dispatch_auth_document(transaction,1,&policy).unwrap() {
        crate::auth_session::write_session_document(&startup.files.path.join("auth-sessions.json"),&auth).unwrap();
    }
    persist_web_startup_in(transaction,&startup,"initializing web","test-skill",&policy).unwrap();
    startup
}

fn commit_binding_test_startup(connection: &mut rusqlite::Connection, root: &Path) -> PathBuf {
    let transaction = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
    let mut startup = prepare_binding_test_startup(&transaction,root);
    register_web_dispatch_binding_in(&transaction,&startup,&binding_test_runtime()).unwrap();
    register_private_web_mode_on(&transaction,&startup,&binding_test_runtime()).unwrap();
    let directory = startup.files.path.clone();
    startup.files.preserve = true;
    transaction.commit().unwrap();
    directory
}

fn binding_test_verify(connection: &mut rusqlite::Connection, path: &Path, runtime: &JsonValue) -> Result<(),String> {
    let transaction = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
    let directory = WebBindingDirectory::open(path)?;
    verify_web_dispatch_binding_in(&transaction,"start-test",1,&directory,runtime)
}

#[test]
fn web_binding_commits_private_receipt_and_readonly_verification_survives_reopen() {
    let (root,db_path,mut connection) = web_start_fixture();
    let path = commit_binding_test_startup(&mut connection,&root);
    let (schema,tag): (i64,Vec<u8>) = connection.query_row("SELECT schema_version,binding_tag FROM native_web_dispatch_bindings",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(schema,1);
    assert_eq!(tag.len(),32);
    assert!(!String::from_utf8_lossy(&tag).contains("synthetic"));
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(path.join(WEB_DISPATCH_KEY_FILE)).unwrap().permissions().mode() & 0o777,0o600);
    }
    let before = web_start_snapshot(&connection);
    drop(connection);
    let mut reopened = db::open(&db_path).unwrap();
    binding_test_verify(&mut reopened,&path,&binding_test_runtime()).unwrap();
    binding_test_verify(&mut reopened,&path,&binding_test_runtime()).unwrap();
    assert_eq!(web_start_snapshot(&reopened),before);
    reopened.execute("UPDATE projects SET description='UI-only change',updated_at='changed'",[]).unwrap();
    reopened.execute("UPDATE sentinel_scans SET current_checkpoint='UI-only summary',updated_at='changed'",[]).unwrap();
    binding_test_verify(&mut reopened,&path,&binding_test_runtime()).unwrap();
    let claim: String = reopened.query_row("SELECT claim_id FROM native_branch_dispatches WHERE branch='web'",[],|r|r.get(0)).unwrap();
    assert!(claim.is_empty(),"verification is not a dispatch claim");
    drop(reopened);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn web_binding_detects_each_runtime_input_change_without_exposing_values() {
    let (root,_,mut connection) = web_start_fixture();
    let path = commit_binding_test_startup(&mut connection,&root);
    for pointer in ["/model/api_key","/model/api_base","/settings/roleModels/reviewer","/build","/runtimePath","/runtime/packet_budget","/runtime/proxies"] {
        let mut changed = binding_test_runtime();
        *changed.pointer_mut(pointer).unwrap() = serde_json::json!("changed-private-value");
        assert_eq!(binding_test_verify(&mut connection,&path,&changed).unwrap_err(),"web_binding_inputs_changed","{pointer}");
    }
    binding_test_verify(&mut connection,&path,&binding_test_runtime()).unwrap();
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn web_binding_registration_faults_restore_entire_startup_and_owned_files() {
    for trigger in [
        "BEFORE INSERT ON native_web_dispatch_bindings BEGIN SELECT RAISE(ABORT,'fixture failure'); END;",
        "BEFORE INSERT ON native_web_dispatch_bindings BEGIN SELECT RAISE(IGNORE); END;",
        "AFTER INSERT ON native_web_dispatch_bindings BEGIN DELETE FROM native_web_dispatch_bindings; END;",
        "AFTER INSERT ON native_web_dispatch_bindings BEGIN UPDATE native_web_dispatch_bindings SET binding_tag=zeroblob(32); END;",
        "AFTER INSERT ON native_web_dispatch_bindings BEGIN UPDATE sentinel_targets SET url='https://changed.example.test'; END;",
        "AFTER INSERT ON native_web_dispatch_bindings BEGIN UPDATE sentinel_scan_contexts SET policy_json='{}',environment='changed'; END;",
        "AFTER INSERT ON native_web_dispatch_bindings BEGIN UPDATE sentinel_scans SET task_path='changed'; END;",
        "AFTER INSERT ON native_web_dispatch_bindings BEGIN UPDATE config_profiles SET settings_json='{\"changed\":true}'; END;",
        "AFTER INSERT ON native_web_dispatch_bindings BEGIN UPDATE native_branch_dispatches SET claim_id='other',claimed_at='other'; END;",
    ] {
        let (root,_,mut connection) = web_start_fixture();
        let before = web_start_snapshot(&connection);
        connection.execute_batch(&format!("CREATE TRIGGER binding_fault {trigger}")).unwrap();
        {
            let transaction = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
            let startup = prepare_binding_test_startup(&transaction,&root);
            assert!(register_web_dispatch_binding_in(&transaction,&startup,&binding_test_runtime()).is_err(),"{trigger}");
        }
        assert_eq!(web_start_snapshot(&connection),before,"{trigger}");
        assert!(!root.join("agent-jobs/start-test/attempt-0001").exists(),"{trigger}");
        let receipts: i64 = connection.query_row("SELECT count(*) FROM native_web_dispatch_bindings",[],|r|r.get(0)).unwrap();
        assert_eq!(receipts,0);
        connection.execute_batch("DROP TRIGGER binding_fault").unwrap();
        commit_binding_test_startup(&mut connection,&root);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn web_binding_old_tasks_are_not_backfilled_and_lost_keys_are_not_regenerated() {
    let (root,db_path,mut connection) = web_mode_legacy_start_fixture();
    let before = web_mode_test_rows(&connection);
    assert!(run_web_start_fixture(&mut connection,&root,WebStartMode::Confirm).is_err());
    web_mode_assert_rows(&connection,&before);
    drop(connection);
    db::initialize(&root).unwrap();
    let connection = db::open(&db_path).unwrap();
    // Schema startup re-seeds built-ins with INSERT OR IGNORE, which advances
    // SQLite's AUTOINCREMENT counters. Every application row must stay exact.
    let after_initialize = web_mode_test_rows(&connection);
    assert_eq!(after_initialize.len(),before.len());
    for ((name,rows),(old_name,old_rows)) in after_initialize.iter().zip(&before) {
        assert_eq!(name,old_name);
        if name == "sqlite_sequence" {
            eprintln!("schema reseed sequence before={old_rows:?} after={rows:?}");
        } else {
            assert_eq!(rows,old_rows,"schema startup changed old application data: {name}");
        }
    }
    for table in ["native_web_dispatch_bindings","native_web_mode_receipts"] {
        assert_eq!(connection.query_row(&format!("SELECT count(*) FROM {table}"),[],|r|r.get::<_,i64>(0)).unwrap(),0);
    }
    assert!(!root.join("agent-jobs/start-test/attempt-0001").exists());
    drop(connection);
    fs::remove_dir_all(root).unwrap();

    let (root,_,mut connection) = web_start_fixture();
    let path = commit_binding_test_startup(&mut connection,&root);
    fs::remove_file(path.join(WEB_DISPATCH_KEY_FILE)).unwrap();
    assert!(binding_test_verify(&mut connection,&path,&binding_test_runtime()).is_err());
    assert!(!path.join(WEB_DISPATCH_KEY_FILE).exists());
    fs::write(path.join(WEB_DISPATCH_KEY_FILE),b"wrong key length").unwrap();
    assert!(binding_test_verify(&mut connection,&path,&binding_test_runtime()).is_err());
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn web_binding_rejects_artifact_drift_missing_files_and_unexpected_auth() {
    for name in [".oviraptor-scan-id","task.json","targets.json","targets.txt","agent-instruction.md","model-prompt-audit.json","auth-sessions.json"] {
        let (root,_,mut connection) = web_start_fixture();
        let path = commit_binding_test_startup(&mut connection,&root);
        let artifact = path.join(name);
        let previous = fs::read(&artifact).ok();
        fs::write(&artifact,b"changed").unwrap();
        assert!(binding_test_verify(&mut connection,&path,&binding_test_runtime()).is_err(),"{name}");
        fs::remove_file(&artifact).unwrap();
        if let Some(bytes) = previous {
            assert!(binding_test_verify(&mut connection,&path,&binding_test_runtime()).is_err(),"{name}");
            fs::write(&artifact,bytes).unwrap();
        }
        binding_test_verify(&mut connection,&path,&binding_test_runtime()).unwrap();
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn web_binding_rejects_lifecycle_database_and_dispatch_drift() {
    for sql in [
        "UPDATE sentinel_scans SET status='paused'",
        "UPDATE sentinel_scans SET attempt_count=2",
        "UPDATE sentinel_scans SET scan_type='code'",
        "UPDATE sentinel_scan_contexts SET policy_json='bad json'",
        "UPDATE sentinel_scan_contexts SET policy_json='{}',environment='other'",
        "UPDATE sentinel_targets SET company='other'",
        "UPDATE sentinel_targets SET url='https://other.example.test' WHERE company='one'",
        "UPDATE config_profiles SET settings_json='{\"changed\":true}'",
        "UPDATE sentinel_scan_attempts SET backend_plan_json='{}'",
        "UPDATE projects SET status='archived'",
        "INSERT INTO environment_preparation_lease(singleton,owner) VALUES(1,'fixture')",
        "INSERT INTO sentinel_deleted_scans(scan_id) VALUES('start-test')",
        "DELETE FROM native_branch_dispatches",
    ] {
        let (root,_,mut connection) = web_start_fixture();
        let path = commit_binding_test_startup(&mut connection,&root);
        connection.execute_batch(sql).unwrap();
        assert!(binding_test_verify(&mut connection,&path,&binding_test_runtime()).is_err(),"{sql}");
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn web_binding_rechecks_target_fuses_before_and_after_dispatch_claim() {
    let (root,db_path,mut connection) = web_start_fixture();
    let path = commit_binding_test_startup(&mut connection,&root);
    // An archived fuse and an unrelated URL do not block this attempt.
    connection.execute_batch("INSERT INTO sentinel_fuse_zone(project_id,url,normalized_url,archived)
        VALUES(1,'https://start.example.test/one','https://start.example.test/one',1),
        (1,'https://unrelated.example.test','https://unrelated.example.test',0)").unwrap();
    binding_test_verify(&mut connection,&path,&binding_test_runtime()).unwrap();
    connection.execute("UPDATE sentinel_fuse_zone SET archived=0 WHERE normalized_url='https://start.example.test/one'",[]).unwrap();
    let claim = || NativeBranchGuard::claim_with_preflight(&db_path,"start-test",1,"web",|transaction| {
        verify_web_dispatch_binding_in(transaction,"start-test",1,&WebBindingDirectory::open(&path)?,&binding_test_runtime())
    });
    assert!(matches!(claim(),Err(error) if error=="web_binding_target_fused"));
    let state = || connection.query_row(
        "SELECT s.status,b.status,d.claim_id FROM sentinel_scans s JOIN native_scan_branches b ON b.scan_id=s.id
         JOIN native_branch_dispatches d ON d.scan_id=s.id WHERE s.id='start-test'",[],
        |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?))).unwrap();
    assert_eq!(state(),("scanning".into(),"pending".into(),String::new()));
    connection.execute("DELETE FROM sentinel_fuse_zone WHERE normalized_url='https://start.example.test/one'",[]).unwrap();
    connection.execute_batch("CREATE TRIGGER fuse_during_claim AFTER UPDATE ON native_branch_dispatches BEGIN
        INSERT INTO sentinel_fuse_zone(project_id,url,normalized_url) VALUES(1,'https://start.example.test/one','https://start.example.test/one'); END;").unwrap();
    assert!(matches!(claim(),Err(error) if error=="web_binding_target_fused"));
    assert_eq!(state(),("scanning".into(),"pending".into(),String::new()));
    assert_eq!(connection.query_row("SELECT count(*) FROM sentinel_fuse_zone WHERE normalized_url='https://start.example.test/one'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    connection.execute_batch("DROP TRIGGER fuse_during_claim").unwrap();
    let mut guard = claim().unwrap();
    guard.disarm();
    drop(guard);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

fn seed_binding_auth(connection: &rusqlite::Connection) {
    let document = serde_json::json!({"id":"binding-auth","scopeHosts":["start.example.test"],"headers":{"Authorization":"Bearer synthetic-session-only"}});
    let expiry = (chrono::Utc::now()+chrono::Duration::hours(1)).to_rfc3339();
    connection.execute("INSERT INTO browser_auth_sessions(id,project_id,owner_scan_id,name,entry_url,status,session_json,expires_at) VALUES('binding-auth',1,'start-test','Test identity','https://start.example.test','valid',?1,?2)",params![document.to_string(),expiry]).unwrap();
    connection.execute("UPDATE sentinel_scan_contexts SET policy_json=json_set(policy_json,'$.authSessionId','binding-auth','$.authSessionIds',json('[\"binding-auth\"]')) WHERE scan_id='start-test'",[]).unwrap();
}

#[test]
fn web_binding_revalidates_auth_expiry_ownership_status_and_actual_material() {
    for sql in [
        "UPDATE browser_auth_sessions SET status='invalid'",
        "UPDATE browser_auth_sessions SET expires_at='2000-01-01T00:00:00Z'",
        "UPDATE browser_auth_sessions SET expires_at='invalid'",
        "UPDATE browser_auth_sessions SET owner_scan_id='another-task'",
        "UPDATE browser_auth_sessions SET session_json=json_set(session_json,'$.scopeHosts',json('[\"other.example.test\"]'))",
        "INSERT INTO projects(id,name) VALUES(2,'Other'); UPDATE browser_auth_sessions SET project_id=2",
        "UPDATE browser_auth_sessions SET session_json='{}'",
        "UPDATE browser_auth_sessions SET session_json=json_set(session_json,'$.headers.Authorization','Bearer changed')",
        "DELETE FROM browser_auth_sessions",
    ] {
        let (root,_,mut connection) = web_start_fixture();
        seed_binding_auth(&connection);
        let path = commit_binding_test_startup(&mut connection,&root);
        binding_test_verify(&mut connection,&path,&binding_test_runtime()).unwrap();
        connection.execute_batch(sql).unwrap();
        assert!(binding_test_verify(&mut connection,&path,&binding_test_runtime()).is_err(),"{sql}");
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn web_binding_refuses_symlink_hardlink_fifo_oversized_and_swapped_directory() {
    use std::os::unix::fs::{symlink,PermissionsExt};
    for replacement in ["symlink","hardlink","fifo","oversized","key_permissions"] {
        let (root,_,mut connection) = web_start_fixture();
        let path = commit_binding_test_startup(&mut connection,&root);
        let name = if replacement == "key_permissions" { WEB_DISPATCH_KEY_FILE } else { "task.json" };
        let artifact = path.join(name);
        let saved = root.join("saved-original");
        fs::rename(&artifact,&saved).unwrap();
        match replacement {
            "symlink" => symlink(&saved,&artifact).unwrap(),
            "hardlink" => fs::hard_link(&saved,&artifact).unwrap(),
            "fifo" => {
                let raw = std::ffi::CString::new(artifact.to_str().unwrap()).unwrap();
                // SAFETY: test-owned path, valid NUL-terminated string.
                assert_eq!(unsafe {libc::mkfifo(raw.as_ptr(),0o600)},0);
            },
            "oversized" => File::create(&artifact).unwrap().set_len(WEB_DISPATCH_BINDING_LIMIT+1).unwrap(),
            "key_permissions" => {
                fs::copy(&saved,&artifact).unwrap();
                fs::set_permissions(&artifact,fs::Permissions::from_mode(0o644)).unwrap();
            },
            _ => unreachable!(),
        }
        assert!(binding_test_verify(&mut connection,&path,&binding_test_runtime()).is_err(),"{replacement}");
        assert!(saved.exists());
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
    let (root,_,mut connection) = web_start_fixture();
    let path = commit_binding_test_startup(&mut connection,&root);
    let directory = WebBindingDirectory::open(&path).unwrap();
    let moved = root.join("moved-attempt");
    fs::rename(&path,&moved).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(directory.check_identity().is_err());
    fs::set_permissions(&path,fs::Permissions::from_mode(0o700)).unwrap();
    for entry in fs::read_dir(&moved).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(),path.join(entry.file_name())).unwrap();
    }
    assert_eq!(binding_test_verify(&mut connection,&path,&binding_test_runtime()).unwrap_err(),"web_binding_inputs_changed",
        "copying the key and all artifact bytes cannot replace the owned directory");
    drop(directory);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn web_binding_commit_failure_preserves_only_owned_artifacts_and_never_claims() {
    let (root,_,mut connection) = web_start_fixture();
    let before = web_start_snapshot(&connection);
    connection.execute_batch("CREATE TABLE binding_parent(id INTEGER PRIMARY KEY);
        CREATE TABLE binding_child(id INTEGER REFERENCES binding_parent(id) DEFERRABLE INITIALLY DEFERRED);
        CREATE TRIGGER binding_commit_fault AFTER INSERT ON native_web_dispatch_bindings BEGIN INSERT INTO binding_child VALUES(1); END;").unwrap();
    let transaction = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
    let mut startup = prepare_binding_test_startup(&transaction,&root);
    register_web_dispatch_binding_in(&transaction,&startup,&binding_test_runtime()).unwrap();
    startup.files.preserve = true;
    let path = startup.files.path.clone();
    assert!(transaction.commit().is_err());
    drop(startup);
    assert_eq!(web_start_snapshot(&connection),before);
    assert!(path.join(WEB_DISPATCH_KEY_FILE).exists());
    let count: i64 = connection.query_row("SELECT count(*) FROM native_branch_dispatches",[],|r|r.get(0)).unwrap();
    assert_eq!(count,0);
    let count: i64 = connection.query_row("SELECT count(*) FROM native_web_dispatch_bindings",[],|r|r.get(0)).unwrap();
    assert_eq!(count,0);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn web_binding_runtime_resolution_is_readonly_and_binds_real_model_and_worker_config() {
    let (root,_,connection) = web_start_fixture();
    let worker = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/workers/9_frontend_runtime_probe.cjs");
    let before = web_start_snapshot(&connection);
    let runtime = resolve_agent_web_pipeline_runtime(&connection,"start-test","cloud",worker.clone()).unwrap();
    assert_eq!(web_start_snapshot(&connection),before);
    let model = ModelRuntimeEnv { llm:"fixture".into(), api_key:"test-secret".into(), api_base:"https://model.example.test".into(),deployment:"cloud".into(),full_power:false,prompt_audit_mode:"off".into() };
    let runtime_path = root.join("missing-tools");
    let descriptor = web_dispatch_runtime_binding(&connection,&runtime,&model,runtime_path.as_os_str()).unwrap();
    assert_eq!(descriptor["model"]["api_key"],"test-secret");
    assert_eq!(descriptor["runtimePath"],runtime_path.to_str().unwrap());
    assert_eq!(descriptor["buildSha256"].as_str().unwrap().len(),64);
    assert!(descriptor["frontendConfig"]["hardTimeoutSeconds"].is_number());
    assert!(descriptor["runtime"]["adaptive"].is_object());
    assert!(descriptor["environment"].get("OVIRAPTOR_NODE_EXECUTABLE").is_some());
    assert_eq!(descriptor["toolchain"]["schemaVersion"],1);
    assert!(descriptor["toolchain"]["nodeCandidates"].as_array().unwrap().iter()
        .any(|entry| entry["path"] == runtime_path.join(if cfg!(windows) { "node.exe" } else { "node" }).to_str().unwrap()));
    connection.execute("UPDATE sentinel_scan_contexts SET policy_json='invalid-json'",[]).unwrap();
    let before = web_start_snapshot(&connection);
    assert!(resolve_agent_web_pipeline_runtime(&connection,"start-test","cloud",worker).is_err());
    assert_eq!(web_start_snapshot(&connection),before);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn web_binding_live_dispatch_rechecks_real_settings_and_owns_exactly_one_claim() {
    let (root,db_path,mut connection) = web_start_fixture();
    let settings = serde_json::json!({"modelName":"fixture","modelApiKey":"synthetic-binding-key",
        "modelApiBase":"https://model.example.test/v1","modelDeployment":"cloud","agentPromptAuditMode":"off"});
    connection.execute("UPDATE config_profiles SET settings_json=?1",[settings.to_string()]).unwrap();
    let worker = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/workers/9_frontend_runtime_probe.cjs");
    let home = root.parent().unwrap();
    let transaction = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
    let model = model_runtime_env(&sentinel_settings(&transaction)).unwrap();
    let runtime = resolve_agent_web_pipeline_runtime(&transaction,"start-test",&model.deployment,worker.clone()).unwrap();
    transaction.execute("UPDATE sentinel_scan_contexts SET policy_json=?1",[runtime.web_policy.to_string()]).unwrap();
    let mut startup = prepare_binding_test_startup(&transaction,&root);
    let descriptor = web_dispatch_runtime_binding(&transaction,&runtime,&model,&sentinel_runtime_path(home)).unwrap();
    register_web_dispatch_binding_in(&transaction,&startup,&descriptor).unwrap();
    let frozen_database = web_binding_database_snapshot(&transaction,"start-test").unwrap();
    let path = startup.files.path.clone();
    startup.files.preserve = true;
    transaction.commit().unwrap();

    assert!(matches!(NativeBranchGuard::claim(&db_path,"start-test",1,"web"),Err(error) if error=="web_binding_preflight_required"));
    let claim = || NativeBranchGuard::claim_with_preflight(&db_path,"start-test",1,"web",|connection| {
        verify_live_web_dispatch_binding_in(connection,"start-test",1,&path,&worker,home)
    });
    connection.execute("UPDATE config_profiles SET settings_json=json_set(settings_json,'$.modelApiKey','different-private-value')",[]).unwrap();
    assert!(matches!(claim(),Err(error) if error=="web_binding_inputs_changed"));
    let state: (String,String,String) = connection.query_row(
        "SELECT s.status,b.status,d.claim_id FROM sentinel_scans s JOIN native_scan_branches b ON b.scan_id=s.id JOIN native_branch_dispatches d ON d.scan_id=s.id WHERE s.id='start-test'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(state,("scanning".into(),"pending".into(),String::new()));
    connection.execute("UPDATE config_profiles SET settings_json=?1",[settings.to_string()]).unwrap();

    // A trigger changing configuration AFTER the admission check must roll back
    // the claim and its own write, without arming the branch failure guard.
    connection.execute_batch("CREATE TRIGGER binding_claim_fault AFTER UPDATE ON native_branch_dispatches BEGIN UPDATE config_profiles SET settings_json=json_set(settings_json,'$.modelApiKey','trigger-changed'); END;").unwrap();
    assert!(matches!(claim(),Err(error) if error=="web_binding_inputs_changed"));
    assert_eq!(sentinel_settings(&connection),settings);
    let claim_id: String = connection.query_row("SELECT claim_id FROM native_branch_dispatches",[],|r|r.get(0)).unwrap();
    assert!(claim_id.is_empty());
    connection.execute_batch("DROP TRIGGER binding_claim_fault").unwrap();

    let restored_model = model_runtime_env(&sentinel_settings(&connection)).unwrap();
    let restored_runtime = resolve_agent_web_pipeline_runtime(&connection,"start-test",&restored_model.deployment,worker.clone()).unwrap();
    let restored = web_dispatch_runtime_binding(&connection,&restored_runtime,&restored_model,&sentinel_runtime_path(home)).unwrap();
    assert_eq!(binding_test_changed_fields(&descriptor,&restored),Vec::<String>::new(),"restored runtime binding differs (paths only)");
    assert_eq!(binding_test_changed_fields(&frozen_database,&web_binding_database_snapshot(&connection,"start-test").unwrap()),Vec::<String>::new(),"restored database binding differs (paths only)");

    let mut guard = claim().unwrap();
    assert!(claim().is_err(),"live owner must not admit a second worker");
    guard.disarm();
    drop(guard);
    assert!(claim().is_err(),"released OS lock must not reset durable claim");
    let terminal: String = connection.query_row("SELECT status FROM native_scan_branches",[],|r|r.get(0)).unwrap();
    assert_eq!(terminal,"pending","failed admissions must never own failure cleanup");
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn web_binding_identical_inputs_use_distinct_keys_and_cannot_move_between_tasks() {
    let (root_a,_,mut connection_a) = web_start_fixture();
    let (root_b,_,mut connection_b) = web_start_fixture();
    let path_a = commit_binding_test_startup(&mut connection_a,&root_a);
    let path_b = commit_binding_test_startup(&mut connection_b,&root_b);
    let key_a = fs::read(path_a.join(WEB_DISPATCH_KEY_FILE)).unwrap();
    let key_b = fs::read(path_b.join(WEB_DISPATCH_KEY_FILE)).unwrap();
    assert_ne!(key_a,key_b);
    let tag_a: Vec<u8> = connection_a.query_row("SELECT binding_tag FROM native_web_dispatch_bindings",[],|r|r.get(0)).unwrap();
    let tag_b: Vec<u8> = connection_b.query_row("SELECT binding_tag FROM native_web_dispatch_bindings",[],|r|r.get(0)).unwrap();
    assert_ne!(tag_a,tag_b);
    fs::write(path_b.join(WEB_DISPATCH_KEY_FILE),key_a).unwrap();
    connection_b.execute("UPDATE native_web_dispatch_bindings SET binding_tag=?1",[tag_a]).unwrap();
    assert_eq!(binding_test_verify(&mut connection_b,&path_b,&binding_test_runtime()).unwrap_err(),"web_binding_inputs_changed");
    binding_test_verify(&mut connection_a,&path_a,&binding_test_runtime()).unwrap();
    drop(connection_a);
    drop(connection_b);
    fs::remove_dir_all(root_a).unwrap();
    fs::remove_dir_all(root_b).unwrap();
}

#[test]
fn web_binding_repeat_registration_never_rotates_key_or_overwrites_receipt() {
    let (root,_,mut connection) = web_start_fixture();
    let transaction = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
    let mut startup = prepare_binding_test_startup(&transaction,&root);
    register_web_dispatch_binding_in(&transaction,&startup,&binding_test_runtime()).unwrap();
    let path = startup.files.path.clone();
    let key = fs::read(path.join(WEB_DISPATCH_KEY_FILE)).unwrap();
    let tag: Vec<u8> = transaction.query_row("SELECT binding_tag FROM native_web_dispatch_bindings",[],|r|r.get(0)).unwrap();
    assert_eq!(register_web_dispatch_binding_in(&transaction,&startup,&binding_test_runtime()).unwrap_err(),"web_binding_key_create_failed");
    assert_eq!(fs::read(path.join(WEB_DISPATCH_KEY_FILE)).unwrap(),key);
    let still_tag: Vec<u8> = transaction.query_row("SELECT binding_tag FROM native_web_dispatch_bindings",[],|r|r.get(0)).unwrap();
    assert_eq!(still_tag,tag);
    startup.files.preserve = true;
    transaction.commit().unwrap();
    binding_test_verify(&mut connection,&path,&binding_test_runtime()).unwrap();
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}
