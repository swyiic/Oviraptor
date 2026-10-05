#[test]
fn source_runtime_freezes_operator_budget_skills_and_model_without_persisting_credentials() {
    let (root,connection,mut record,plan,scope)=source_ci_publication_fixture();
    record.policy["maxBudgetUsd"]=json!(2.5);
    record.policy["additionalInstruction"]=json!("inspect authorization boundaries in source only");
    connection.execute("INSERT INTO agent_skills(name,instructions,enabled) VALUES('source-fixture-skill','Read the frozen source evidence, not the live repository.',1)",[]).unwrap();
    record.policy["selectedSkillIds"]=json!([connection.last_insert_rowid()]);
    publish_workbench_fixture(&root,&connection,&record,&plan,&scope,false).unwrap();
    let (model,_,contract)=verify_source_runtime_contract(&connection,&record.scan_id,1,&root.join("attempt-0001")).unwrap();
    assert_eq!(contract["budget"]["maxBudgetUsd"],2.5);
    assert_eq!(contract["budget"]["mode"],"deep");
    assert_eq!(contract["operatorPolicy"]["additionalInstruction"],record.policy["additionalInstruction"]);
    assert!(contract["skillInstructions"].as_str().unwrap().contains("frozen source evidence"));
    assert_eq!(contract["targetRequestsGranted"],0);
    assert_eq!(contract["hostActionsGranted"],0);
    for public in [contract.to_string(),fs::read_to_string(root.join("attempt-0001/task.json")).unwrap()] {
        assert!(!public.contains(&model.api_key));
        assert!(!public.contains(&model.api_base));
    }
    let before=web_start_snapshot(&connection);
    verify_source_runtime_contract(&connection,&record.scan_id,1,&root.join("attempt-0001")).unwrap();
    assert_eq!(before,web_start_snapshot(&connection),"verification is read-only");
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_runs",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_runtime_rejects_changed_model_credentials_budget_policy_and_skills() {
    for change in ["model","endpoint","credential","tokens","timeout","proxy","instruction","skills"] {
        let (root,connection,mut record,plan,scope)=source_ci_publication_fixture();
        connection.execute("INSERT INTO agent_skills(name,instructions,enabled) VALUES('bound-skill','frozen skill',1)",[]).unwrap();
        let skill=connection.last_insert_rowid();record.policy["selectedSkillIds"]=json!([skill]);
        publish_workbench_fixture(&root,&connection,&record,&plan,&scope,false).unwrap();
        let path=match change {
            "model"=>Some(("$.modelProfiles[0].llm",json!("different-model"))),
            "endpoint"=>Some(("$.modelProfiles[0].apiBase",json!("http://127.0.0.1:8/v1"))),
            "credential"=>Some(("$.modelProfiles[0].apiKey",json!("rotated-fixture-key"))),
            "tokens"=>Some(("$.agentDeepTokenLimit",json!(123456))),
            "timeout"=>Some(("$.agentDeepTimeout",json!(1234))),
            "proxy"=>Some(("$.noProxy",json!("different.invalid"))),
            _=>None,
        };
        if let Some((path,value))=path {
            connection.execute("UPDATE config_profiles SET settings_json=json_set(settings_json,?1,json(?2))",params![path,value.to_string()]).unwrap();
        } else if change=="skills" {
            connection.execute("UPDATE agent_skills SET instructions='replacement skill' WHERE id=?1",[skill]).unwrap();
        } else {
            connection.execute("UPDATE sentinel_scan_contexts SET policy_json=json_set(policy_json,'$.additionalInstruction','replacement instruction')",[]).unwrap();
        }
        assert!(verify_source_runtime_contract(&connection,&record.scan_id,1,&root.join("attempt-0001")).is_err(),"{change}");
        // The production entry also refuses before snapshot/analyzer execution.
        assert!(run_native_source_scan(&root.join("oviraptor.sqlite3"),&root,&record.scan_id,1,&root.join("attempt-0001"),&record.source_path,"cicd","").is_err(),"{change}");
        assert_eq!(connection.query_row("SELECT count(*) FROM source_snapshots",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_runtime_receipt_failures_roll_back_every_published_surface() {
    for action in ["RAISE(ABORT,'injected')","RAISE(IGNORE)"] {
        let (root,connection,record,plan,scope)=source_ci_publication_fixture();
        connection.execute_batch(&format!("CREATE TRIGGER runtime_fault BEFORE INSERT ON source_runtime_contracts BEGIN SELECT {action}; END;")).unwrap();
        let before=web_start_snapshot(&connection);
        assert!(publish_workbench_fixture(&root,&connection,&record,&plan,&scope,false).is_err());
        assert_eq!(before,web_start_snapshot(&connection));
        for table in ["source_scope_contracts","source_runtime_contracts","native_branch_dispatches","source_ci_policies"] {
            assert_eq!(connection.query_row(&format!("SELECT count(*) FROM {table}"),[],|r|r.get::<_,i64>(0)).unwrap(),0,"{table}");
        }
        // A leftover private key cannot repair a missing publication receipt.
        assert!(verify_source_runtime_contract(&connection,&record.scan_id,1,&root.join("attempt-0001")).is_err());
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_runtime_final_insert_cannot_tamper_prior_publication_postconditions() {
    for sql in [
        "UPDATE sentinel_scan_contexts SET policy_json=json_set(policy_json,'$.additionalInstruction','changed');",
        "UPDATE config_profiles SET settings_json=json_set(settings_json,'$.agentDeepTokenLimit',123456);",
        "UPDATE config_profiles SET settings_json=json_set(settings_json,'$.modelProfiles[0].apiKey','replacement-key');",
        "UPDATE sentinel_scans SET task_path='replacement.json';",
        "DELETE FROM sentinel_targets;",
        "UPDATE native_branch_dispatches SET claim_id='stolen',claimed_at='now' WHERE branch='source';",
    ] {
        let (root,connection,record,plan,scope)=source_ci_publication_fixture();
        connection.execute_batch(&format!("CREATE TRIGGER runtime_tamper AFTER INSERT ON source_runtime_contracts BEGIN {sql} END;")).unwrap();
        let before=web_start_snapshot(&connection);
        assert!(publish_workbench_fixture(&root,&connection,&record,&plan,&scope,false).is_err(),"{sql}");
        assert_eq!(before,web_start_snapshot(&connection));
        assert_eq!(connection.query_row("SELECT count(*) FROM source_runtime_contracts",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_runtime_private_key_and_task_are_required_and_never_repaired_on_read() {
    for change in ["missing_key","changed_key","changed_task","forged_receipt"] {
        let (root,connection,record,plan,scope)=source_ci_publication_fixture();
        publish_workbench_fixture(&root,&connection,&record,&plan,&scope,false).unwrap();
        let work=root.join("attempt-0001");let key=work.join(SOURCE_RUNTIME_KEY_FILE);
        match change {
            "missing_key"=>fs::remove_file(&key).unwrap(),
            "changed_key"=>fs::write(&key,[0u8;32]).unwrap(),
            "changed_task"=>{
                let mut task:JsonValue=serde_json::from_slice(&fs::read(work.join("task.json")).unwrap()).unwrap();
                task["maxBudgetUsd"]=json!(0.01);task["policy"]["maxBudgetUsd"]=json!(0.01);
                fs::write(work.join("task.json"),task.to_string()).unwrap();
                connection.execute("UPDATE sentinel_scan_contexts SET policy_json=json_set(policy_json,'$.maxBudgetUsd',0.01)",[]).unwrap();
            },
            _=>connection.execute_batch("DROP TRIGGER source_runtime_no_update; UPDATE source_runtime_contracts SET binding_tag=zeroblob(32);").unwrap(),
        }
        assert!(verify_source_runtime_contract(&connection,&record.scan_id,1,&work).is_err(),"{change}");
        if change=="missing_key" { assert!(!key.exists()); }
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn source_runtime_rejects_unsafe_keys_and_replaced_directories() {
    use std::os::unix::fs::{symlink,PermissionsExt};
    for change in ["public_key","symlink_key","hardlink_key","replaced_directory"] {
        let (root,connection,record,plan,scope)=source_ci_publication_fixture();
        publish_workbench_fixture(&root,&connection,&record,&plan,&scope,false).unwrap();
        let work=root.join("attempt-0001");let key=work.join(SOURCE_RUNTIME_KEY_FILE);
        match change {
            "public_key"=>fs::set_permissions(&key,fs::Permissions::from_mode(0o644)).unwrap(),
            "symlink_key"=>{let other=root.join("copied-key");fs::rename(&key,&other).unwrap();symlink(&other,&key).unwrap();},
            "hardlink_key"=>fs::hard_link(&key,root.join("extra-key-link")).unwrap(),
            _=>{
                let old=root.join("original-directory");fs::rename(&work,&old).unwrap();
                fs::create_dir(&work).unwrap();fs::set_permissions(&work,fs::Permissions::from_mode(0o700)).unwrap();
                for name in ["task.json",SOURCE_RUNTIME_KEY_FILE] {fs::copy(old.join(name),work.join(name)).unwrap();}
            },
        }
        assert!(verify_source_runtime_contract(&connection,&record.scan_id,1,&work).is_err(),"{change}");
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_runtime_contract_is_immutable_and_new_attempt_has_a_distinct_key() {
    let (root,connection,mut record,mut plan,scope)=source_ci_publication_fixture();
    publish_workbench_fixture(&root,&connection,&record,&plan,&scope,false).unwrap();
    let key=fs::read(root.join("attempt-0001").join(SOURCE_RUNTIME_KEY_FILE)).unwrap();
    for sql in ["UPDATE source_runtime_contracts SET binding_tag=zeroblob(32)","DELETE FROM source_runtime_contracts",
        "INSERT OR REPLACE INTO source_runtime_contracts SELECT * FROM source_runtime_contracts"] {assert!(connection.execute(sql,[]).is_err());}
    connection.execute("UPDATE sentinel_scans SET status='failed'",[]).unwrap();
    record.attempt=2;plan.attempt_number=2;record.policy["maxBudgetUsd"]=json!(1.5);
    publish_workbench_fixture(&root,&connection,&record,&plan,&scope,true).unwrap();
    assert_ne!(key,fs::read(root.join("attempt-0002").join(SOURCE_RUNTIME_KEY_FILE)).unwrap());
    assert!(verify_source_runtime_contract(&connection,&record.scan_id,1,&root.join("attempt-0001")).is_err());
    let (_,_,contract)=verify_source_runtime_contract(&connection,&record.scan_id,2,&root.join("attempt-0002")).unwrap();
    assert_eq!(contract["budget"]["maxBudgetUsd"],1.5);
    connection.execute("DELETE FROM sentinel_scans WHERE id=?1",[&record.scan_id]).unwrap();
    assert_eq!(connection.query_row("SELECT count(*) FROM source_runtime_contracts",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_runtime_migration_never_backfills_execution_from_old_task_json() {
    let (root,connection,record,plan,scope)=source_ci_publication_fixture();
    publish_workbench_fixture(&root,&connection,&record,&plan,&scope,false).unwrap();
    connection.execute_batch("DROP TRIGGER source_runtime_no_delete; DROP TABLE source_runtime_contracts;").unwrap();
    drop(connection);
    let db_path=db::initialize(&root).unwrap();let connection=db::open(&db_path).unwrap();
    assert_eq!(connection.query_row("SELECT count(*) FROM source_runtime_contracts",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert!(root.join("attempt-0001/task.json").exists());
    assert!(verify_source_runtime_contract(&connection,&record.scan_id,1,&root.join("attempt-0001")).is_err());
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_runtime_invalid_budget_or_changed_prepared_selection_never_publishes() {
    for invalid in [json!(0),json!(-1),json!(10000.01),json!("unlimited"),json!({"model":"substituted"})] {
        let (root,connection,mut record,plan,scope)=source_ci_publication_fixture();
        if invalid.is_object() {record.llm_policy=invalid.clone();}
        else {record.policy["maxBudgetUsd"]=invalid.clone();}
        let before=web_start_snapshot(&connection);
        assert!(publish_workbench_fixture(&root,&connection,&record,&plan,&scope,false).is_err(),"{invalid}");
        assert_eq!(before,web_start_snapshot(&connection));
        assert_eq!(connection.query_row("SELECT count(*) FROM source_runtime_contracts",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_runtime_cosmetic_settings_change_does_not_invalidate_frozen_execution_inputs() {
    let (root,connection,record,plan,scope)=source_ci_publication_fixture();
    publish_workbench_fixture(&root,&connection,&record,&plan,&scope,false).unwrap();
    let (_,_,before)=verify_source_runtime_contract(&connection,&record.scan_id,1,&root.join("attempt-0001")).unwrap();
    connection.execute("UPDATE config_profiles SET settings_json=json_set(settings_json,'$.uiTheme','different-theme')",[]).unwrap();
    let (_,_,after)=verify_source_runtime_contract(&connection,&record.scan_id,1,&root.join("attempt-0001")).unwrap();
    assert_eq!(before,after);
    drop(connection);fs::remove_dir_all(root).unwrap();
}
