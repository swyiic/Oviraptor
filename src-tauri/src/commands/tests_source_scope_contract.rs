#[test]
fn source_scope_contract_publication_persists_a_bound_attempt_receipt() {
    let (root, connection, record, plan, draft) = source_ci_publication_fixture();
    publish_workbench_fixture(&root,&connection,&record,&plan,&draft,false).unwrap();
    let row: (String,String,String) = connection.query_row(
        "SELECT source_path,canonical_root,scan_type FROM source_scope_contracts WHERE scan_id='wb-start' AND attempt_number=1",
        [],|r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).unwrap();
    assert_eq!(row,(record.source_path.clone(),Path::new(&record.source_path).canonicalize().unwrap().to_string_lossy().into_owned(),"cicd".into()));
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_scope_contract_rejects_substituted_source_or_scan_kind_before_any_analyzer() {
    for change_root in [true,false] {
        let (root,connection,record,plan,draft)=source_ci_publication_fixture();
        publish_workbench_fixture(&root,&connection,&record,&plan,&draft,false).unwrap();
        let other=root.join("not-authorized");
        fs::create_dir(&other).unwrap();
        fs::write(other.join("private.py"),"private = True\n").unwrap();
        let source=if change_root {other.to_str().unwrap()} else {&record.source_path};
        let kind=if change_root {"cicd"} else {"code"};
        let mut calls=0;
        let result=run_native_source_scan_using(
            &root.join("oviraptor.sqlite3"),&root,&record.scan_id,1,&root.join("attempt-0001"),source,kind,"",
            |_,engine,_,_,scratch,_| {calls+=1;Ok(source_regression_outcome(engine,scratch))},
        );
        assert!(result.is_err(),"worker arguments cannot replace the published source/kind");
        assert_eq!(calls,0);
        assert_eq!(connection.query_row("SELECT count(*) FROM source_snapshots",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_scope_contract_preserves_modes_and_normalizes_only_the_effective_base() {
    for (mode,base,expected) in [("full","origin/main",""),("diff"," main ","main"),("auto"," main ","main"),("auto","","")] {
        let (root,connection,mut record,plan,draft)=source_ci_publication_fixture();
        record.scope_mode=mode.into();
        record.diff_base=base.into();
        publish_workbench_fixture(&root,&connection,&record,&plan,&draft,false).unwrap();
        let scope=load_workbench_source_scope(&connection,&record.scan_id,1).unwrap();
        assert_eq!(scope.scope_mode,mode);
        assert_eq!(scope.diff_base,expected);
        assert!(scope.verify_worker(&record.source_path,"cicd",base).is_ok());
        if mode!="full" { assert!(scope.verify_worker(&record.source_path,"cicd","other-base").is_err()); }
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_scope_contract_invalid_or_mismatched_task_never_publishes() {
    for (field,value) in [("scopeMode",json!("unlimited")),("scopeMode",JsonValue::Null),
        ("diffBase",JsonValue::Null),("attempt",json!(2)),("scanId",json!("other")),
        ("sourcePath",json!("/definitely-not-an-authorized-source")),("scanType",json!("code"))] {
        let (root,connection,record,plan,draft)=source_ci_publication_fixture();
        let mut task=json!({"scanId":record.scan_id,"attempt":1,"sourcePath":record.source_path,
            "scopeMode":record.scope_mode,"diffBase":record.diff_base,"scanType":record.scan_type});
        task[field]=value;
        fs::write(root.join("attempt-0001/task.json"),task.to_string()).unwrap();
        let before=web_start_snapshot(&connection);
        assert!(publish_workbench_fixture(&root,&connection,&record,&plan,&draft,false).is_err(),"{field}");
        assert_eq!(web_start_snapshot(&connection),before);
        assert_eq!(connection.query_row("SELECT count(*) FROM source_scope_contracts",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
    let (root,connection,mut record,plan,draft)=source_ci_publication_fixture();
    for (mode,base) in [("diff",""),("diff","   "),("unsupported","main")] {
        record.scope_mode=mode.into(); record.diff_base=base.into();
        let before=web_start_snapshot(&connection);
        assert!(publish_workbench_fixture(&root,&connection,&record,&plan,&draft,false).is_err());
        assert_eq!(web_start_snapshot(&connection),before);
    }
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_scope_contract_failed_or_ignored_insert_rolls_back_publication() {
    for action in ["RAISE(ABORT,'injected')","RAISE(IGNORE)"] {
        let (root,connection,record,plan,draft)=source_ci_publication_fixture();
        connection.execute_batch(&format!("CREATE TRIGGER source_scope_fault BEFORE INSERT ON source_scope_contracts BEGIN SELECT {action}; END;")).unwrap();
        let before=web_start_snapshot(&connection);
        assert!(publish_workbench_fixture(&root,&connection,&record,&plan,&draft,false).is_err());
        assert_eq!(web_start_snapshot(&connection),before);
        assert_eq!(connection.query_row("SELECT count(*) FROM source_scope_contracts",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        connection.execute_batch("DROP TRIGGER source_scope_fault").unwrap();
        publish_workbench_fixture(&root,&connection,&record,&plan,&draft,false).unwrap();
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_scope_contract_publication_rechecks_binding_after_later_writes() {
    for change in ["UPDATE sentinel_scans SET source_path='other-root' WHERE id=NEW.scan_id;",
        "UPDATE sentinel_scan_attempts SET status='paused' WHERE scan_id=NEW.scan_id;"] {
        let (root,connection,record,plan,draft)=source_ci_publication_fixture();
        connection.execute_batch(&format!("CREATE TRIGGER source_scope_late_fault AFTER INSERT ON native_branch_dispatches BEGIN {change} END;")).unwrap();
        let before=web_start_snapshot(&connection);
        assert!(publish_workbench_fixture(&root,&connection,&record,&plan,&draft,false).is_err());
        assert_eq!(web_start_snapshot(&connection),before);
        assert_eq!(connection.query_row("SELECT count(*) FROM source_scope_contracts",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_scope_contract_is_immutable_and_each_attempt_owns_its_selection() {
    let (root,connection,mut record,mut plan,draft)=source_ci_publication_fixture();
    publish_workbench_fixture(&root,&connection,&record,&plan,&draft,false).unwrap();
    let first=load_workbench_source_scope(&connection,&record.scan_id,1).unwrap();
    for sql in ["UPDATE source_scope_contracts SET scope_mode='auto'","DELETE FROM source_scope_contracts",
        "INSERT OR REPLACE INTO source_scope_contracts SELECT * FROM source_scope_contracts"] {
        assert!(connection.execute(sql,[]).is_err());
        assert_eq!(load_workbench_source_scope(&connection,&record.scan_id,1).unwrap(),first);
    }
    connection.execute("UPDATE sentinel_scans SET status='failed' WHERE id='wb-start'",[]).unwrap();
    record.attempt=2; plan.attempt_number=2;
    record.scope_mode="diff".into(); record.diff_base="approved-new-base".into();
    publish_workbench_fixture(&root,&connection,&record,&plan,&draft,true).unwrap();
    assert!(load_workbench_source_scope(&connection,&record.scan_id,1).is_err());
    let next=load_workbench_source_scope(&connection,&record.scan_id,2).unwrap();
    assert_eq!((next.scope_mode.as_str(),next.diff_base.as_str()),("diff","approved-new-base"));
    assert_eq!(connection.query_row("SELECT scope_mode FROM source_scope_contracts WHERE attempt_number=1",[],|r|r.get::<_,String>(0)).unwrap(),"full");
    connection.execute("DELETE FROM sentinel_scans WHERE id='wb-start'",[]).unwrap();
    assert_eq!(connection.query_row("SELECT count(*) FROM source_scope_contracts",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_scope_contract_missing_or_changed_in_flight_cannot_import_or_fallback() {
    for before in [true,false] {
        for change_scan in [true,false] {
            let (root,connection,record,plan,draft)=source_ci_publication_fixture();
            publish_workbench_fixture(&root,&connection,&record,&plan,&draft,false).unwrap();
            let alter=|connection: &rusqlite::Connection| {
                if change_scan {
                    connection.execute("UPDATE sentinel_scans SET source_path='other-root' WHERE id='wb-start'",[]).unwrap();
                } else {
                    // Storage corruption/pre-upgrade absence, not an app capability.
                    connection.execute_batch("DROP TRIGGER source_scope_no_delete; DELETE FROM source_scope_contracts;").unwrap();
                }
            };
            if before { alter(&connection); }
            let mut calls=0;
            let result=run_native_source_scan_using(
                &root.join("oviraptor.sqlite3"),&root,&record.scan_id,1,&root.join("attempt-0001"),&record.source_path,"cicd","",
                |connection,engine,_,_,scratch,cancelled| {
                    calls+=1;
                    assert!(!cancelled());
                    alter(connection);
                    assert!(cancelled(),"in-flight analyzer must see withdrawn source binding");
                    Ok(source_regression_outcome(engine,scratch))
                },
            );
            assert!(result.is_err());
            assert_eq!(calls,if before {0} else {1});
            assert!(!root.join("artifact-import-cas").exists());
            assert!(NativeSourcePlan::load(&connection,&record.scan_id,1).unwrap().is_none());
            assert_eq!(connection.query_row("SELECT gate_status FROM sentinel_scan_contexts WHERE scan_id='wb-start'",[],|r|r.get::<_,String>(0)).unwrap(),"not_evaluated");
            drop(connection);
            fs::remove_dir_all(root).unwrap();
        }
    }
}

#[test]
fn source_scope_contract_runtime_ignores_mutable_task_json_but_not_the_receipt() {
    let (root,connection,record,plan,draft)=source_ci_publication_fixture();
    publish_workbench_fixture(&root,&connection,&record,&plan,&draft,false).unwrap();
    fs::write(root.join("attempt-0001/task.json"),"{broken after publication").unwrap();
    let mut calls=0;
    let report=run_native_source_scan_using(
        &root.join("oviraptor.sqlite3"),&root,&record.scan_id,1,&root.join("attempt-0001"),&record.source_path,"cicd","stale-hidden-base",
        |_,engine,snapshot,_,scratch,_| {
            calls+=1;
            assert!(snapshot.base_sha.is_empty(),"full must not resolve a stale hidden base");
            Ok(source_regression_outcome(engine,scratch))
        },
    ).unwrap();
    assert_eq!(calls,2);
    assert_eq!(report["requestedSourceScope"]["scopeMode"],"full");
    assert_eq!(report["requestedSourceScope"]["diffBase"],"");
    assert_eq!(report["requestedSourceScope"]["sourcePath"],record.source_path);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn source_scope_contract_symlink_retarget_cannot_change_published_root() {
    for before in [true,false] {
        let (root,connection,mut record,plan,draft)=source_ci_publication_fixture();
        let link=root.join("selected-repository");
        std::os::unix::fs::symlink(&record.source_path,&link).unwrap();
        record.source_path=link.to_str().unwrap().into();
        publish_workbench_fixture(&root,&connection,&record,&plan,&draft,false).unwrap();
        let other=root.join("other-repository");
        fs::create_dir(&other).unwrap();
        fs::write(other.join("app.py"),"private = True\n").unwrap();
        let retarget=|| {fs::remove_file(&link).unwrap();std::os::unix::fs::symlink(&other,&link).unwrap();};
        if before {retarget();}
        let mut calls=0;
        let result=run_native_source_scan_using(
            &root.join("oviraptor.sqlite3"),&root,&record.scan_id,1,&root.join("attempt-0001"),&record.source_path,"cicd","",
            |_,engine,_,_,scratch,cancelled| {
                calls+=1;retarget();assert!(cancelled());Ok(source_regression_outcome(engine,scratch))
            },
        );
        assert!(result.is_err());
        assert_eq!(calls,if before {0} else {1});
        assert!(!root.join("artifact-import-cas").exists());
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_scope_contract_task_reader_rejects_directories_and_oversized_files() {
    for directory in [true,false] {
        let (root,connection,record,plan,draft)=source_ci_publication_fixture();
        let task=root.join("attempt-0001/task.json");
        if directory { fs::create_dir(&task).unwrap(); } else {
            // Sparse oversized file: reject by descriptor metadata before allocation.
            fs::File::create(&task).unwrap().set_len(16*1024*1024+1).unwrap();
        }
        let before=web_start_snapshot(&connection);
        assert!(publish_workbench_fixture(&root,&connection,&record,&plan,&draft,false).is_err());
        assert_eq!(web_start_snapshot(&connection),before);
        assert_eq!(connection.query_row("SELECT count(*) FROM source_scope_contracts",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn source_scope_contract_task_reader_refuses_links_and_does_not_block_on_fifo() {
    for fifo in [true,false] {
        let (root,connection,record,plan,draft)=source_ci_publication_fixture();
        let task=root.join("attempt-0001/task.json");
        if fifo {
            assert!(std::process::Command::new("mkfifo").arg(&task).status().unwrap().success());
        } else {
            let other=root.join("other-task.json");
            fs::write(&other,json!({"scanId":record.scan_id,"attempt":1,"sourcePath":record.source_path,
                "scopeMode":record.scope_mode,"diffBase":record.diff_base,"scanType":record.scan_type}).to_string()).unwrap();
            std::os::unix::fs::symlink(&other,&task).unwrap();
        }
        let before=web_start_snapshot(&connection);
        let started=std::time::Instant::now();
        assert!(publish_workbench_fixture(&root,&connection,&record,&plan,&draft,false).is_err());
        assert!(started.elapsed()<Duration::from_secs(5),"special file must not block publication");
        assert_eq!(web_start_snapshot(&connection),before);
        assert_eq!(connection.query_row("SELECT count(*) FROM source_scope_contracts",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}
