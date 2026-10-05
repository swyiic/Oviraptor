fn source_ci_publication_fixture() -> (PathBuf, rusqlite::Connection, WorkbenchStartRecord, ScanBackendPlan, String) {
    let (root, connection, mut record, mut plan, scope) = workbench_start_fixture();
    let repo = root.join("repository");
    fs::create_dir(&repo).unwrap();
    fs::write(repo.join("app.py"), "print('ci fixture')\n").unwrap();
    record.scan_type = "cicd".into();
    record.scope_mode = "full".into();
    record.diff_base.clear();
    record.auth_session_ids.clear();
    record.authenticated = false;
    record.auth_type = "none".into();
    record.auth_profile_name.clear();
    record.policy["authSessionId"] = json!("");
    record.policy["authSessionIds"] = json!([]);
    record.source_path = repo.to_string_lossy().into_owned();
    record.urls.clear();
    plan.targets.clear();
    (root, connection, record, plan, scope)
}

#[test]
fn source_ci_policy_publication_freezes_per_attempt_release_controls() {
    let (root, connection, record, plan, scope) = source_ci_publication_fixture();
    publish_workbench_fixture(&root, &connection, &record, &plan, &scope, false).unwrap();
    let frozen: (i64, i64, bool) = connection.query_row(
        "SELECT max_critical,max_high,block_release FROM source_ci_policies WHERE scan_id='wb-start' AND attempt_number=1",
        [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap();
    assert_eq!(frozen, (0, 5, false));
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_ci_policy_real_pipeline_uses_the_published_task_not_global_defaults() {
    let (root, connection, record, plan, scope) = source_ci_publication_fixture();
    publish_workbench_fixture(&root, &connection, &record, &plan, &scope, false).unwrap();
    connection.execute("INSERT INTO config_profiles(name,is_default,settings_json) VALUES('ci-policy-fixture',1,?1)",
        [json!({"ciGate":{"allowedBlocking":0,"blockSeverities":["critical","high"]}}).to_string()]).unwrap();
    let report = run_native_source_scan_using(
        &root.join("oviraptor.sqlite3"), &root, &record.scan_id, 1, &root.join("attempt-0001"),
        &record.source_path, "cicd", "", |connection, engine, _, _, scratch, _| {
            // Mutable settings and context cannot replace the policy of a running attempt.
            assert_eq!(connection.execute("UPDATE config_profiles SET settings_json=json_set(settings_json,'$.ciGate.allowedBlocking',9999) WHERE name='ci-policy-fixture'",[]).unwrap(),1);
            connection.execute("UPDATE sentinel_scan_contexts SET policy_json=json_set(policy_json,'$.maxHigh',9999,'$.blockRelease',1) WHERE scan_id='wb-start'",[]).unwrap();
            Ok(source_regression_outcome(engine, scratch))
        },
    ).unwrap();
    assert_eq!(report["gate"]["policy"]["releaseLimits"], json!({"maxCritical":0,"maxHigh":5,"blockRelease":false}));
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_ci_policy_failed_or_ignored_publication_rolls_back_the_task() {
    for action in ["RAISE(ABORT,'injected')", "RAISE(IGNORE)"] {
        let (root, connection, record, plan, scope) = source_ci_publication_fixture();
        connection.execute_batch(&format!("CREATE TRIGGER ci_policy_fault BEFORE INSERT ON source_ci_policies BEGIN SELECT {action}; END;")).unwrap();
        let before = web_start_snapshot(&connection);
        assert!(publish_workbench_fixture(&root,&connection,&record,&plan,&scope,false).is_err());
        assert_eq!(web_start_snapshot(&connection),before);
        assert_eq!(connection.query_row("SELECT count(*) FROM source_ci_policies",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        connection.execute_batch("DROP TRIGGER ci_policy_fault").unwrap();
        publish_workbench_fixture(&root,&connection,&record,&plan,&scope,false).unwrap();
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_ci_policy_is_immutable_and_new_attempt_gets_its_own_controls() {
    let (root, connection, mut record, mut plan, scope) = source_ci_publication_fixture();
    publish_workbench_fixture(&root,&connection,&record,&plan,&scope,false).unwrap();
    for sql in ["UPDATE source_ci_policies SET max_high=9999", "UPDATE source_ci_policies SET block_release=1",
        "DELETE FROM source_ci_policies", "INSERT OR REPLACE INTO source_ci_policies(scan_id,attempt_number,max_critical,max_high,block_release) VALUES('wb-start',1,9999,9999,0)"] {
        assert!(connection.execute(sql,[]).is_err());
    }
    let old = load_workbench_ci_policy(&connection,&record.scan_id,1).unwrap();
    connection.execute("UPDATE sentinel_scans SET status='failed' WHERE id='wb-start'",[]).unwrap();
    record.attempt = 2;
    plan.attempt_number = 2;
    record.policy["maxHigh"] = json!(2);
    record.policy["blockRelease"] = json!(true);
    publish_workbench_fixture(&root,&connection,&record,&plan,&scope,true).unwrap();
    let new = load_workbench_ci_policy(&connection,&record.scan_id,2).unwrap();
    assert_ne!(old,new);
    assert!(load_workbench_ci_policy(&connection,&record.scan_id,1).is_err());
    let history: (i64,bool) = connection.query_row("SELECT max_high,block_release FROM source_ci_policies WHERE attempt_number=1",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(history,(5,false));
    connection.execute("DELETE FROM sentinel_scans WHERE id='wb-start'",[]).unwrap();
    assert_eq!(connection.query_row("SELECT count(*) FROM source_ci_policies",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_ci_policy_missing_or_changed_receipt_never_falls_back_to_defaults() {
    for remove_before_execution in [true,false] {
        let (root, connection, record, plan, scope) = source_ci_publication_fixture();
        publish_workbench_fixture(&root,&connection,&record,&plan,&scope,false).unwrap();
        // Simulate damaged storage / a pre-upgrade task, not an available app operation.
        connection.execute_batch("DROP TRIGGER source_ci_policy_no_delete").unwrap();
        if remove_before_execution { connection.execute("DELETE FROM source_ci_policies",[]).unwrap(); }
        let mut calls = 0;
        let outcome = run_native_source_scan_using(
            &root.join("oviraptor.sqlite3"),&root,&record.scan_id,1,&root.join("attempt-0001"),&record.source_path,"cicd","",
            |connection,engine,_,_,scratch,_| {
                calls += 1;
                connection.execute("DELETE FROM source_ci_policies",[]).unwrap();
                Ok(source_regression_outcome(engine,scratch))
            },
        );
        assert!(outcome.unwrap_err().contains("workbench_ci_policy_unavailable"));
        assert_eq!(calls,if remove_before_execution { 0 } else { 1 });
        assert!(!root.join("artifact-import-cas").exists());
        assert_eq!(connection.query_row("SELECT gate_status FROM sentinel_scan_contexts WHERE scan_id='wb-start'",[],|r|r.get::<_,String>(0)).unwrap(),"not_evaluated");
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_ci_policy_rejects_incomplete_or_malformed_task_controls() {
    for (key,value) in [("maxCritical",json!(-1)),("maxHigh",json!(10001)),("maxHigh",json!(1.5)),
        ("maxHigh",json!("5")),("blockRelease",json!(1)),("blockRelease",JsonValue::Null)] {
        let (root,connection,mut record,plan,scope)=source_ci_publication_fixture();
        record.policy[key]=value;
        let before=web_start_snapshot(&connection);
        assert!(publish_workbench_fixture(&root,&connection,&record,&plan,&scope,false).is_err());
        assert_eq!(web_start_snapshot(&connection),before);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
    for key in ["maxCritical","maxHigh","blockRelease"] {
        let mut policy=json!({"maxCritical":0,"maxHigh":5,"blockRelease":true});
        policy.as_object_mut().unwrap().remove(key);
        assert!(GatePolicy::from_workbench(&policy).is_err());
    }
}
