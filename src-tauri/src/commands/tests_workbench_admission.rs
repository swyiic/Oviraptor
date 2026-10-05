fn workbench_admission_fixture() -> (PathBuf,PathBuf,rusqlite::Connection) {
    let (root,connection,mut record,plan,scope)=workbench_start_fixture();
    let repo=root.join("repository");
    fs::create_dir(&repo).unwrap();
    record.source_path=repo.to_string_lossy().into_owned();
    publish_workbench_fixture(&root,&connection,&record,&plan,&scope,false).unwrap();
    let db_path=PathBuf::from(connection.path().unwrap());
    (root,db_path,connection)
}

fn workbench_admission_snapshot(connection: &rusqlite::Connection) -> JsonValue {
    let mut statements=Vec::new();
    for sql in [
        "SELECT json_array(status,current_checkpoint,attempt_count) FROM sentinel_scans ORDER BY id",
        "SELECT json_array(status,checkpoint,stop_reason,finished_at) FROM sentinel_scan_attempts ORDER BY scan_id,attempt_number",
        "SELECT json_array(branch,status,checkpoint,report_json) FROM native_scan_branches ORDER BY branch",
        "SELECT json_array(branch,claim_id,claimed_at) FROM native_branch_dispatches ORDER BY branch",
        "SELECT json_array(url,status,last_attempt_number) FROM sentinel_targets ORDER BY id",
    ] {
        let mut stmt=connection.prepare(sql).unwrap();
        let rows=stmt.query_map([],|row|row.get::<_,String>(0)).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
        statements.push(rows);
    }
    json!(statements)
}

#[test]
fn workbench_admission_spawned_source_worker_exits_without_starting_analysis() {
    let (root,path,connection)=workbench_admission_fixture();
    connection.execute("INSERT INTO environment_preparation_lease(singleton,owner) VALUES(1,'fixture')",[]).unwrap();
    let work=root.join("actual-worker");
    fs::create_dir(&work).unwrap();
    launch_native_source_pipeline(path.clone(),root.clone(),"wb-start".into(),1,work.clone(),
        root.join("repository").to_string_lossy().into_owned(),"greybox".into(),String::new()).unwrap();
    let deadline=std::time::Instant::now()+Duration::from_secs(5);
    loop {
        let log=fs::read_to_string(work.join("oviraptor-runner.log")).unwrap_or_default();
        if log.contains("源码分支未取得执行权") { break; }
        assert!(std::time::Instant::now()<deadline,"spawned source worker did not report admission failure");
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(connection.query_row("SELECT status FROM native_scan_branches WHERE branch='source'",[],|r|r.get::<_,String>(0)).unwrap(),"failed");
    assert!(RepositorySnapshot::restore(&connection,"wb-start",1).unwrap().is_none());
    assert_eq!(fs::read_dir(&work).unwrap().count(),1,"only the actual rejection log, no analyzer output");
    assert_eq!(connection.query_row("SELECT count(*) FROM native_branch_dispatches WHERE claim_id<>''",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn workbench_admission_real_claim_failure_settles_only_its_unexecuted_branch() {
    let (root,path,connection)=workbench_admission_fixture();
    connection.execute("INSERT INTO environment_preparation_lease(singleton,owner) VALUES(1,'fixture')",[]).unwrap();
    assert_eq!(claim_workbench_pipeline_branch(&path,"wb-start",1,"source").err().unwrap(),"native_dispatch_attempt_ineligible");
    let status=native_scan_status(&connection,"wb-start").unwrap();
    let source=status["branches"].as_array().unwrap().iter().find(|b|b["branch"]=="source").unwrap();
    assert_eq!(source["status"],"failed");
    assert_eq!(source["report"]["failurePhase"],"branch_admission");
    assert_eq!(source["report"]["executionStarted"],false);
    assert_eq!(connection.query_row("SELECT status FROM sentinel_scans WHERE id='wb-start'",[],|r|r.get::<_,String>(0)).unwrap(),"scanning");
    assert_eq!(connection.query_row("SELECT status FROM native_scan_branches WHERE branch='web'",[],|r|r.get::<_,String>(0)).unwrap(),"pending");
    // Exercise the actual Web launcher admission route, not a duplicate claim.
    assert!(claim_web_pipeline_branch(&path,"wb-start",1,&root,Path::new("unused-worker"),&mut None).is_err());
    assert_eq!(connection.query_row("SELECT status FROM sentinel_scans WHERE id='wb-start'",[],|r|r.get::<_,String>(0)).unwrap(),"failed");
    assert_eq!(connection.query_row("SELECT count(*) FROM sentinel_targets WHERE status='failed'",[],|r|r.get::<_,i64>(0)).unwrap(),2);
    assert_eq!(connection.query_row("SELECT count(*) FROM native_branch_dispatches WHERE claim_id<>'' OR claimed_at<>''",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    let before=workbench_admission_snapshot(&connection);
    assert!(!settle_unclaimed_workbench_admission(&path,"wb-start",1,"source","injected").unwrap());
    assert_eq!(before,workbench_admission_snapshot(&connection));
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn workbench_admission_duplicate_and_released_claim_never_rewrite_owner() {
    let (root,path,connection)=workbench_admission_fixture();
    let mut owner=claim_workbench_pipeline_branch(&path,"wb-start",1,"web").unwrap();
    let before=workbench_admission_snapshot(&connection);
    assert!(claim_workbench_pipeline_branch(&path,"wb-start",1,"web").is_err());
    assert!(!settle_unclaimed_workbench_admission(&path,"wb-start",1,"web","injected").unwrap());
    assert_eq!(before,workbench_admission_snapshot(&connection));
    owner.disarm();
    drop(owner);
    assert!(claim_workbench_pipeline_branch(&path,"wb-start",1,"web").is_err());
    assert_eq!(before,workbench_admission_snapshot(&connection));
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn workbench_admission_missing_partial_uncertain_or_noncurrent_receipts_are_not_unexecuted() {
    for sql in [
        "DELETE FROM native_branch_dispatches WHERE branch='source'",
        "PRAGMA ignore_check_constraints=ON; UPDATE native_branch_dispatches SET claim_id='unknown' WHERE branch='source'",
        "PRAGMA ignore_check_constraints=ON; UPDATE native_branch_dispatches SET claimed_at='unknown' WHERE branch='source'",
        "UPDATE sentinel_scans SET status='paused'",
        "UPDATE sentinel_scans SET status='pausing'",
        "UPDATE sentinel_scans SET attempt_count=2",
        "UPDATE sentinel_scans SET scan_type='web'",
        "UPDATE sentinel_scan_attempts SET status='paused'",
        "INSERT INTO sentinel_deleted_scans(scan_id) VALUES('wb-start')",
    ] {
        let (root,path,connection)=workbench_admission_fixture();
        connection.execute_batch(sql).unwrap();
        let before=workbench_admission_snapshot(&connection);
        assert!(!settle_unclaimed_workbench_admission(&path,"wb-start",1,"source","injected").unwrap(),"{sql}");
        assert_eq!(before,workbench_admission_snapshot(&connection),"{sql}");
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
    let (root,path,connection)=workbench_admission_fixture();
    for error in ["native_dispatch_commit_unconfirmed:injected","native_invocation_not_owned:injected"] {
        let before=workbench_admission_snapshot(&connection);
        assert!(!settle_unclaimed_workbench_admission(&path,"wb-start",1,"source",error).unwrap());
        assert_eq!(before,workbench_admission_snapshot(&connection));
    }
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn workbench_admission_faults_rollback_branch_targets_and_attempt_together() {
    for table in ["native_scan_branches","sentinel_scans","sentinel_scan_attempts","sentinel_targets"] {
        for action in ["RAISE(ABORT,'injected')","RAISE(IGNORE)"] {
            let (root,path,connection)=workbench_admission_fixture();
            // Both siblings fail, so scan/attempt *must* leave scanning and a
            // silently ignored attempt sync cannot pass on an unchanged status.
            assert!(settle_unclaimed_workbench_admission(&path,"wb-start",1,"source","injected").unwrap());
            connection.execute_batch(&format!("CREATE TRIGGER wb_admission_fault BEFORE UPDATE ON {table} BEGIN SELECT {action}; END;")).unwrap();
            let before=workbench_admission_snapshot(&connection);
            assert!(settle_unclaimed_workbench_admission(&path,"wb-start",1,"web","injected").is_err(),"{table}/{action}");
            assert_eq!(before,workbench_admission_snapshot(&connection),"{table}/{action}");
            connection.execute_batch("DROP TRIGGER wb_admission_fault").unwrap();
            assert!(settle_unclaimed_workbench_admission(&path,"wb-start",1,"web","injected").unwrap());
            drop(connection);
            fs::remove_dir_all(root).unwrap();
        }
    }
}

#[test]
fn workbench_admission_late_claim_or_sibling_tampering_rolls_back() {
    for change in [
        "UPDATE native_branch_dispatches SET claim_id='forged',claimed_at='forged' WHERE branch='source';",
        "UPDATE native_scan_branches SET status='failed' WHERE branch='web';",
        "DELETE FROM native_branch_dispatches WHERE branch='web';",
        "UPDATE native_scan_branches SET report_json='{}' WHERE branch='source';",
        "UPDATE sentinel_scans SET attempt_count=2;",
        "UPDATE sentinel_targets SET status='completed' WHERE url LIKE 'https:%';",
    ] {
        let (root,path,connection)=workbench_admission_fixture();
        connection.execute_batch(&format!("CREATE TRIGGER wb_admission_fault AFTER UPDATE OF status ON native_scan_branches WHEN NEW.branch='source' BEGIN {change} END;")).unwrap();
        let before=workbench_admission_snapshot(&connection);
        assert!(settle_unclaimed_workbench_admission(&path,"wb-start",1,"source","injected").is_err(),"{change}");
        assert_eq!(before,workbench_admission_snapshot(&connection),"{change}");
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn workbench_admission_pending_sibling_does_not_hide_a_failed_attempt_sync() {
    let (root,path,connection)=workbench_admission_fixture();
    connection.execute_batch("CREATE TRIGGER wb_admission_fault BEFORE UPDATE ON sentinel_scan_attempts BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    let before=workbench_admission_snapshot(&connection);
    assert_eq!(settle_unclaimed_workbench_admission(&path,"wb-start",1,"source","injected").unwrap_err(),"workbench_admission_branch_postcondition");
    assert_eq!(before,workbench_admission_snapshot(&connection));
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn workbench_admission_failed_source_does_not_cancel_running_web_and_is_not_success() {
    let (root,path,connection)=workbench_admission_fixture();
    let mut web=claim_workbench_pipeline_branch(&path,"wb-start",1,"web").unwrap();
    let siblings=workbench_admission_siblings(&connection,"wb-start",1,"source").unwrap();
    assert!(settle_unclaimed_workbench_admission(&path,"wb-start",1,"source","sensitive injected value").unwrap());
    assert_eq!(siblings,workbench_admission_siblings(&connection,"wb-start",1,"source").unwrap());
    finish_native_branch(&path,"wb-start",1,"web","completed","local fixture complete",&json!({})).unwrap();
    web.disarm();
    drop(web);
    assert_eq!(connection.query_row("SELECT status FROM sentinel_scans WHERE id='wb-start'",[],|r|r.get::<_,String>(0)).unwrap(),"partial");
    assert!(!workbench_admission_snapshot(&connection).to_string().contains("sensitive injected value"));
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn workbench_admission_settlement_racing_dispatch_has_one_winner() {
    for _ in 0..8 {
        let (root,path,connection)=workbench_admission_fixture();
        let barrier=Arc::new(std::sync::Barrier::new(2));
        let (claim_path,claim_barrier)=(path.clone(),Arc::clone(&barrier));
        let caller=thread::spawn(move || {
            claim_barrier.wait();
            match NativeBranchGuard::claim(&claim_path,"wb-start",1,"source") {
                Ok(mut guard) => {guard.disarm(); true},
                Err(_) => false,
            }
        });
        barrier.wait();
        let settled=settle_unclaimed_workbench_admission(&path,"wb-start",1,"source","injected").unwrap();
        let claimed=caller.join().unwrap();
        assert!(settled ^ claimed,"neither path may steal the other's live ownership");
        let (status,claim):(String,String)=connection.query_row("SELECT b.status,d.claim_id FROM native_scan_branches b JOIN native_branch_dispatches d USING(scan_id,attempt_number,branch) WHERE b.branch='source'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(status,if settled {"failed"} else {"pending"});
        assert_eq!(claim.is_empty(),settled);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}
