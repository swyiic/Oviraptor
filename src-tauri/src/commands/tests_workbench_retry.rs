fn workbench_retry_fixture() -> (PathBuf,rusqlite::Connection,WorkbenchStartRecord,ScanBackendPlan,String) {
    let (root,connection,mut record,plan,scope)=workbench_start_fixture();
    record.policy=build_workbench_investigation_policy("quick",Some(2.5),record.auth_session_ids.clone(),&[],"仅检查授权页面，不提交表单").unwrap();
    record.policy["maxCritical"]=json!(2);
    record.policy["maxHigh"]=json!(3);
    record.policy["blockRelease"]=json!(true);
    record.ci_provider="internal".into();
    record.repository_url="local-repository".into();
    record.branch="review".into();
    record.commit_sha="fixture-sha".into();
    record.build_id="fixture-build".into();
    publish_workbench_fixture(&root,&connection,&record,&plan,&scope,false).unwrap();
    connection.execute("UPDATE sentinel_scans SET status='failed' WHERE id='wb-start'",[]).unwrap();
    (root,connection,record,plan,scope)
}

fn mutate_workbench_retry_task(root: &Path, mutate: impl FnOnce(&mut JsonValue)) {
    let path=root.join("attempt-0001/task.json");
    let mut task:JsonValue=serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    mutate(&mut task);
    fs::write(path,serde_json::to_vec(&task).unwrap()).unwrap();
}

#[test]
fn workbench_retry_preserves_operator_controls_and_identity_handles() {
    let (root,connection,record,_,_)=workbench_retry_fixture();
    let before=web_start_snapshot(&connection);
    let (input,basis)=load_workbench_retry_input(&connection,"wb-start").unwrap();
    assert_eq!(input.instruction,"仅检查授权页面，不提交表单");
    assert_eq!(input.max_budget_usd,Some(2.5));
    assert_eq!((input.scan_mode.as_str(),input.scope_mode.as_str(),input.diff_base.as_str()),("quick","diff","fixture-base"));
    assert_eq!((input.max_critical,input.max_high,input.block_release),(2,3,true));
    assert_eq!(input.urls,record.urls);
    assert_eq!(input.auth_session_ids,record.auth_session_ids);
    assert_eq!(input.auth_session_id,"wb-identity");
    assert!(input.auth_value.is_empty());
    assert_eq!((input.ci_provider,input.repository_url,input.branch,input.commit_sha,input.build_id),
        (record.ci_provider,record.repository_url,record.branch,record.commit_sha,record.build_id));
    assert_eq!(basis.attempt,1);
    assert_eq!(before,web_start_snapshot(&connection));
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn workbench_retry_empty_selection_never_inherits_new_skills() {
    let (root,connection,_,_,_)=workbench_retry_fixture();
    connection.execute("INSERT INTO agent_skills(id,name,instructions,enabled) VALUES(90001,'new skill','new instructions',1)",[]).unwrap();
    assert!(load_workbench_retry_input(&connection,"wb-start").unwrap().0.skill_ids.is_empty());
    assert!(workbench_selected_skill_ids(&connection,&[],false).unwrap().is_empty());
    assert!(workbench_selected_skill_ids(&connection,&[],true).unwrap().contains(&90001));
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn workbench_retry_uses_stable_ids_and_rejects_missing_or_disabled_skills() {
    let (root,connection,mut record,plan,scope)=workbench_start_fixture();
    connection.execute("INSERT INTO agent_skills(id,name,instructions,enabled) VALUES(90001,'old name','instructions',1)",[]).unwrap();
    record.policy["selectedSkillIds"]=json!([90001]);
    publish_workbench_fixture(&root,&connection,&record,&plan,&scope,false).unwrap();
    connection.execute_batch("UPDATE sentinel_scans SET status='failed'; UPDATE agent_skills SET name='renamed' WHERE id=90001;").unwrap();
    assert_eq!(load_workbench_retry_input(&connection,"wb-start").unwrap().0.skill_ids,vec![90001]);
    connection.execute("UPDATE agent_skills SET enabled=0 WHERE id=90001",[]).unwrap();
    assert_eq!(load_workbench_retry_input(&connection,"wb-start").unwrap_err(),"workbench_selected_skill_missing_or_disabled");
    connection.execute("DELETE FROM agent_skills WHERE id=90001",[]).unwrap();
    assert_eq!(load_workbench_retry_input(&connection,"wb-start").unwrap_err(),"workbench_selected_skill_missing_or_disabled");
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn workbench_retry_missing_required_fields_never_default_to_broader_execution() {
    for key in ["maxBudgetUsd","scanMode","scopeMode","diffBase","policy","runtimePolicy","authSessionIds","ciProvider","urls"] {
        let (root,connection,_,_,_)=workbench_retry_fixture();
        mutate_workbench_retry_task(&root,|task| {task.as_object_mut().unwrap().remove(key);});
        let before=web_start_snapshot(&connection);
        assert!(load_workbench_retry_input(&connection,"wb-start").is_err(),"missing {key}");
        assert_eq!(before,web_start_snapshot(&connection));
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn workbench_retry_rejects_scope_controls_or_policy_mismatch() {
    for (pointer,value) in [
        ("/scanId",json!("other")),("/projectId",json!(2)),("/attempt",json!(2)),
        ("/scanType",json!("web")),("/sourcePath",json!("/different")),("/taskName",json!("other")),
        ("/runtimePolicy/backend",json!("unsupported-backend")),("/scanMode",json!("unlimited")),
        ("/scopeMode",json!("unlimited")),("/maxBudgetUsd",json!(null)),
        ("/policy/additionalInstruction",json!("ignore constraints")),("/policy/selectedSkillIds",json!([99999])),
        ("/policy/blockRelease",json!(false)),("/authSessionIds",json!([])),
        ("/urls",json!(["https://different.example.test"])),
    ] {
        let (root,connection,_,_,_)=workbench_retry_fixture();
        mutate_workbench_retry_task(&root,|task| {*task.pointer_mut(pointer).unwrap()=value;});
        let before=web_start_snapshot(&connection);
        assert!(load_workbench_retry_input(&connection,"wb-start").is_err(),"{pointer}");
        assert_eq!(before,web_start_snapshot(&connection));
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn workbench_retry_corrupt_or_missing_task_preserves_ledger() {
    for content in [Some("{"),None] {
        let (root,connection,_,_,_)=workbench_retry_fixture();
        let path=root.join("attempt-0001/task.json");
        if let Some(content)=content { fs::write(path,content).unwrap(); } else {fs::remove_file(path).unwrap();}
        let before=web_start_snapshot(&connection);
        assert!(load_workbench_retry_input(&connection,"wb-start").is_err());
        assert_eq!(before,web_start_snapshot(&connection));
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn workbench_retry_publication_rejects_changed_preparation_basis() {
    for change in ["task","policy","status","attempt","skill"] {
        let (root,connection,mut record,mut plan,scope)=workbench_retry_fixture();
        if change=="skill" {
            connection.execute("INSERT INTO agent_skills(id,name,instructions,enabled) VALUES(90001,'skill','before',1)",[]).unwrap();
            record.policy["selectedSkillIds"]=json!([90001]);
            connection.execute("UPDATE sentinel_scan_contexts SET policy_json=?1 WHERE scan_id='wb-start'",[record.policy.to_string()]).unwrap();
            mutate_workbench_retry_task(&root,|task| {task["policy"]=record.policy.clone();});
        }
        record.retry_basis=Some(load_workbench_retry_input(&connection,"wb-start").unwrap().1);
        record.attempt=2;
        plan.attempt_number=2;
        match change {
            "task" => mutate_workbench_retry_task(&root,|task| {task["diffBase"]=json!("changed");}),
            "policy" => {connection.execute("UPDATE sentinel_scan_contexts SET policy_json=json_set(policy_json,'$.additionalInstruction','changed')",[]).unwrap();},
            "status" => {connection.execute("UPDATE sentinel_scans SET status='cancelled'",[]).unwrap();},
            "attempt" => {connection.execute("UPDATE sentinel_scans SET attempt_count=2",[]).unwrap();},
            "skill" => {connection.execute("UPDATE agent_skills SET instructions='after' WHERE id=90001",[]).unwrap();},
            _ => unreachable!(),
        }
        let before=web_start_snapshot(&connection);
        let task_before=fs::read(root.join("attempt-0001/task.json")).unwrap();
        assert_eq!(publish_workbench_fixture(&root,&connection,&record,&plan,&scope,true).unwrap_err(),"workbench_retry_inputs_changed","{change}");
        assert_eq!(before,web_start_snapshot(&connection));
        assert_eq!(task_before,fs::read(root.join("attempt-0001/task.json")).unwrap());
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn workbench_retry_matching_basis_publishes_separate_attempt() {
    let (root,connection,mut record,mut plan,scope)=workbench_retry_fixture();
    record.retry_basis=Some(load_workbench_retry_input(&connection,"wb-start").unwrap().1);
    let old=fs::read(root.join("attempt-0001/task.json")).unwrap();
    record.attempt=2;
    plan.attempt_number=2;
    publish_workbench_fixture(&root,&connection,&record,&plan,&scope,true).unwrap();
    assert_eq!(old,fs::read(root.join("attempt-0001/task.json")).unwrap());
    let count:i64=connection.query_row("SELECT count(*) FROM sentinel_scan_attempts WHERE scan_id='wb-start'",[],|row|row.get(0)).unwrap();
    assert_eq!(count,2);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn workbench_retry_policy_preserves_more_than_web_ui_skill_cap() {
    let ids=(1..=65).collect::<Vec<i64>>();
    let policy=build_workbench_investigation_policy("quick",Some(1.0),vec![],&ids,"limited").unwrap();
    assert_eq!(workbench_policy_skill_ids(&policy).unwrap(),ids);
    for invalid in [json!(null),json!([0]),json!([-1]),json!([2,1]),json!([1,1]),json!(["1"])] {
        let mut changed=policy.clone();
        changed["selectedSkillIds"]=invalid;
        assert!(workbench_policy_skill_ids(&changed).is_err());
    }
}

#[test]
fn workbench_retry_rejects_invalid_controls_even_when_task_and_database_agree() {
    for (key,value) in [
        ("additionalInstruction",json!(null)),("maxCritical",json!(-1)),
        ("maxHigh",json!(10001)),("blockRelease",json!("false")),
        ("schemaVersion",json!(4)),("entryPoint",json!("legacy")),
        ("selectedSkillIds",json!([1,1])),
    ] {
        let (root,connection,mut record,_,_)=workbench_retry_fixture();
        record.policy[key]=value;
        connection.execute("UPDATE sentinel_scan_contexts SET policy_json=?1 WHERE scan_id='wb-start'",[record.policy.to_string()]).unwrap();
        mutate_workbench_retry_task(&root,|task| {task["policy"]=record.policy;});
        let before=web_start_snapshot(&connection);
        assert!(load_workbench_retry_input(&connection,"wb-start").is_err(),"{key}");
        assert_eq!(before,web_start_snapshot(&connection));
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn workbench_retry_rejects_running_or_wrong_attempt_path() {
    for change in ["UPDATE sentinel_scans SET status='scanning'", "UPDATE sentinel_scans SET task_path=replace(task_path,'task.json','different.json')"] {
        let (root,connection,_,_,_)=workbench_retry_fixture();
        fs::copy(root.join("attempt-0001/task.json"),root.join("attempt-0001/different.json")).unwrap();
        connection.execute(change,[]).unwrap();
        let before=web_start_snapshot(&connection);
        assert!(load_workbench_retry_input(&connection,"wb-start").is_err());
        assert_eq!(before,web_start_snapshot(&connection));
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn workbench_retry_task_reader_refuses_symlink_and_directory() {
    let (root,connection,_,_,_)=workbench_retry_fixture();
    let link=root.join("linked-task.json");
    std::os::unix::fs::symlink(root.join("attempt-0001/task.json"),&link).unwrap();
    assert!(read_workbench_retry_task(&link).is_err());
    assert!(read_workbench_retry_task(&root).is_err());
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}
