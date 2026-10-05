fn pause_fixture() -> (PathBuf,PathBuf,rusqlite::Connection) {
    let (root,path,connection) = web_start_fixture();
    connection.execute_batch("UPDATE sentinel_scans SET status='scanning',attempt_count=1,
        llm_requests=9,input_tokens=30,output_tokens=12,cached_tokens=4,total_tokens=42;
        INSERT INTO sentinel_scan_attempts(scan_id,attempt_number,llm_requests_start,input_tokens_start,
        output_tokens_start,cached_tokens_start,total_tokens_start) VALUES('start-test',1,2,10,2,1,12);
        INSERT INTO sentinel_scan_attempts(scan_id,attempt_number,status,stop_reason)
        VALUES('start-test',0,'failed','historical stop');").unwrap();
    (root,path,connection)
}

fn pause_status(connection: &rusqlite::Connection) -> String {
    connection.query_row("SELECT status FROM sentinel_scans WHERE id='start-test'",[],|r|r.get(0)).unwrap()
}

#[test]
fn scan_quiescence_each_live_owner_blocks_pause_and_new_admission() {
    for (kind,target) in [("branch","web"),("branch","source"),("frontend_producer","web"),
        ("target","https://start.example.test/one"),("frontend_recon","https://start.example.test/two")] {
        let (root,path,mut connection) = pause_fixture();
        let owner = claim_native_invocation(&path,"start-test",1,kind,target).unwrap();
        assert_eq!(request_sentinel_pause(&path,"start-test").unwrap(),1);
        assert_eq!(finish_sentinel_pause(&path,"start-test",1).unwrap_err(),"scan_quiescence_worker_active_or_unverifiable");
        assert_eq!(pause_status(&connection),"pausing");
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
        assert!(claim_scan_quiescence_in(&tx,&path,"start-test").is_err());
        drop(tx);
        drop(owner);
        assert!(finish_sentinel_pause(&path,"start-test",1).unwrap());
        assert_eq!(pause_status(&connection),"paused");
        let deltas:(i64,i64,i64,i64,i64) = connection.query_row(
            "SELECT llm_requests_delta,input_tokens_delta,output_tokens_delta,cached_tokens_delta,total_tokens_delta
             FROM sentinel_scan_attempts WHERE scan_id='start-test' AND attempt_number=1",[],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
        assert_eq!(deltas,(7,20,10,3,30));
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
        assert!(claim_scan_quiescence_in(&tx,&path,"start-test").is_ok());
        drop(tx);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn scan_quiescence_only_last_real_worker_finalizes_and_late_updates_do_not_revive() {
    let (root,path,connection) = pause_fixture();
    let producer = ScanWorkerOwner::claim(&path,"start-test",1,"frontend_producer","web").unwrap();
    let recon = ScanWorkerOwner::claim(&path,"start-test",1,"frontend_recon","https://start.example.test/one").unwrap();
    request_sentinel_pause(&path,"start-test").unwrap();
    drop(producer);
    assert_eq!(pause_status(&connection),"pausing");
    assert!(ScanWorkerOwner::claim(&path,"start-test",1,"frontend_recon","https://start.example.test/two").is_err());
    for status in ["scanning","completed","failed","cancelled","paused"] {
        let before=web_start_snapshot(&connection);
        sentinel_scan_update(&path,"start-test",status,"late progress");
        assert_eq!(web_start_snapshot(&connection),before);
    }
    drop(recon);
    assert_eq!(pause_status(&connection),"paused");
    let before=web_start_snapshot(&connection);
    sentinel_scan_update(&path,"start-test","scanning","late revival");
    assert_eq!(web_start_snapshot(&connection),before);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn scan_quiescence_blocked_local_http_worker_outlives_producer() {
    use std::io::{Read,Write};
    let (root,path,connection) = pause_fixture();
    let listener=std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url=format!("http://{}/",listener.local_addr().unwrap());
    connection.execute("UPDATE sentinel_targets SET url=?1 WHERE company='one'",[&url]).unwrap();
    let (arrived_tx,arrived_rx)=mpsc::channel();
    let (release_tx,release_rx)=mpsc::channel();
    let server=thread::spawn(move || {
        let (mut socket,_)=listener.accept().unwrap();
        socket.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
        let mut buffer=[0u8;4096];
        assert!(socket.read(&mut buffer).unwrap()>0);
        arrived_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(15)).unwrap();
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").unwrap();
    });
    let producer=ScanWorkerOwner::claim(&path,"start-test",1,"frontend_producer","web").unwrap();
    let owner=ScanWorkerOwner::claim(&path,"start-test",1,"frontend_recon",&url).unwrap();
    let worker=thread::spawn(move || {
        let _owner=owner;
        reqwest::blocking::Client::builder().no_proxy().timeout(Duration::from_secs(20)).build().unwrap()
            .get(url).send().unwrap().text().unwrap()
    });
    arrived_rx.recv_timeout(Duration::from_secs(15)).unwrap();
    request_sentinel_pause(&path,"start-test").unwrap();
    drop(producer);
    assert_eq!(pause_status(&connection),"pausing");
    assert!(finish_sentinel_pause(&path,"start-test",1).is_err());
    release_tx.send(()).unwrap();
    assert_eq!(worker.join().unwrap(),"ok");
    server.join().unwrap();
    assert_eq!(pause_status(&connection),"paused");
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn scan_quiescence_old_attempt_owners_and_partial_acquisitions_are_preserved() {
    let (root,path,connection)=pause_fixture();
    let old=claim_native_invocation(&path,"start-test",0,"frontend_recon","https://start.example.test/two").unwrap();
    request_sentinel_pause(&path,"start-test").unwrap();
    assert!(finish_sentinel_pause(&path,"start-test",1).is_err());
    // Failed aggregate acquisition must release its earlier locks.
    let probe=claim_native_invocation(&path,"start-test",0,"branch","web").unwrap();
    drop(probe);
    assert!(!finish_sentinel_pause(&path,"start-test",0).unwrap());
    assert_eq!(pause_status(&connection),"pausing");
    drop(old);
    assert!(finish_sentinel_pause(&path,"start-test",1).unwrap());
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn scan_quiescence_unconfirmed_process_and_container_records_are_never_deleted() {
    for registration in [
        "INSERT INTO sentinel_processes(scan_id,process_id,engine) VALUES('start-test',12345,'native')",
        "INSERT INTO analyzer_container_receipts(receipt_id,invocation_key,scan_id,attempt_number,purpose,container_name,owner_token,create_args_json,cleanup_status)
         VALUES('receipt','key','start-test',1,'analysis','container','owner','[]','unconfirmed')",
    ] {
        let (root,path,connection)=pause_fixture();
        connection.execute_batch(registration).unwrap();
        request_sentinel_pause(&path,"start-test").unwrap();
        let before=scan_pause_preservation_hash(&connection,"start-test",1).unwrap();
        assert_eq!(finish_sentinel_pause(&path,"start-test",1).unwrap_err(),"scan_quiescence_cleanup_unconfirmed");
        assert_eq!(scan_pause_preservation_hash(&connection,"start-test",1).unwrap(),before);
        assert_eq!(pause_status(&connection),"pausing");
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn scan_quiescence_transaction_faults_preserve_all_prior_state() {
    for finalizing in [false,true] {
        for table in ["sentinel_scans","sentinel_scan_attempts"] {
            for fault in ["ABORT,'injected'","IGNORE"] {
                let (root,path,connection)=pause_fixture();
                if finalizing { request_sentinel_pause(&path,"start-test").unwrap(); }
                let before=web_start_snapshot(&connection);
                connection.execute_batch(&format!("CREATE TRIGGER pause_fault BEFORE UPDATE ON {table} BEGIN SELECT RAISE({fault}); END;")).unwrap();
                let failed=if finalizing { finish_sentinel_pause(&path,"start-test",1).is_err() }
                    else { request_sentinel_pause(&path,"start-test").is_err() };
                assert!(failed,"{finalizing}/{table}/{fault}");
                assert_eq!(web_start_snapshot(&connection),before,"{finalizing}/{table}/{fault}");
                drop(connection);
                fs::remove_dir_all(root).unwrap();
            }
        }
    }
}

#[test]
fn scan_quiescence_after_trigger_corruption_rolls_back_accounting_and_history() {
    for finalizing in [false,true] {
        for corruption in [
            "UPDATE sentinel_scan_attempts SET total_tokens_delta=999 WHERE attempt_number=1;",
            "UPDATE sentinel_scan_attempts SET stop_reason='forged' WHERE attempt_number=1;",
            "UPDATE sentinel_scan_attempts SET checkpoint='forged' WHERE attempt_number=0;",
            "UPDATE sentinel_scans SET total_tokens=999;",
            "UPDATE sentinel_targets SET status='completed';",
        ] {
            let (root,path,connection)=pause_fixture();
            if finalizing { request_sentinel_pause(&path,"start-test").unwrap(); }
            let before=web_start_snapshot(&connection);
            connection.execute_batch(&format!("CREATE TRIGGER pause_corrupt AFTER UPDATE OF status ON sentinel_scans BEGIN {corruption} END;")).unwrap();
            let failed=if finalizing { finish_sentinel_pause(&path,"start-test",1).is_err() }
                else { request_sentinel_pause(&path,"start-test").is_err() };
            assert!(failed,"{finalizing}/{corruption}");
            assert_eq!(web_start_snapshot(&connection),before);
            drop(connection);
            fs::remove_dir_all(root).unwrap();
        }
    }
}

#[test]
fn scan_quiescence_missing_attempt_fails_and_stale_callback_cannot_pause_new_attempt() {
    let (root,path,connection)=pause_fixture();
    connection.execute("DELETE FROM sentinel_scan_attempts WHERE attempt_number=1",[]).unwrap();
    assert!(request_sentinel_pause(&path,"start-test").is_err());
    assert_eq!(pause_status(&connection),"scanning");
    connection.execute_batch("UPDATE sentinel_scans SET status='pausing',attempt_count=2;
        INSERT INTO sentinel_scan_attempts(scan_id,attempt_number) VALUES('start-test',2);").unwrap();
    let before=web_start_snapshot(&connection);
    assert!(!finish_sentinel_pause(&path,"start-test",1).unwrap());
    assert_eq!(web_start_snapshot(&connection),before);
    assert!(finish_sentinel_pause(&path,"start-test",2).unwrap());
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn scan_quiescence_final_worker_callback_is_not_lost_to_ui_lifecycle_lock() {
    let (root,path,connection)=pause_fixture();
    let owner=ScanWorkerOwner::claim(&path,"start-test",1,"frontend_producer","web").unwrap();
    request_sentinel_pause(&path,"start-test").unwrap();
    let lifecycle=claim_scan_control(&path,"start-test").unwrap();
    drop(owner);
    assert_eq!(pause_status(&connection),"paused");
    drop(lifecycle);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn scan_quiescence_agent_only_historical_target_is_still_a_live_owner() {
    let (root,path,connection)=pause_fixture();
    connection.execute("INSERT INTO agent_runs(id,scan_id,attempt_number,target_url)
        VALUES('old-run','start-test',3,'https://historical.example.test/removed')",[]).unwrap();
    let owner=claim_native_invocation(&path,"start-test",3,"target","https://historical.example.test/removed").unwrap();
    request_sentinel_pause(&path,"start-test").unwrap();
    assert!(finish_sentinel_pause(&path,"start-test",1).is_err());
    assert_eq!(pause_status(&connection),"pausing");
    drop(owner);
    assert!(finish_sentinel_pause(&path,"start-test",1).unwrap());
    let count:i64=connection.query_row("SELECT COUNT(*) FROM agent_runs WHERE id='old-run'",[],|r|r.get(0)).unwrap();
    assert_eq!(count,1);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn scan_quiescence_real_branch_guard_releases_before_finalization_without_rewriting_stop() {
    for disarm in [false,true] {
        let (root,path,connection)=pause_fixture();
        connection.execute("UPDATE sentinel_scan_attempts SET stop_reason='original reason',checkpoint='original checkpoint',
            finished_at='2026-01-01 00:00:00' WHERE attempt_number=1",[]).unwrap();
        register_native_branches(&connection,"start-test",1,&["web","source"]).unwrap();
        let mut web=NativeBranchGuard::claim(&path,"start-test",1,"web").unwrap();
        let source=NativeBranchGuard::claim(&path,"start-test",1,"source").unwrap();
        if disarm { web.disarm(); }
        request_sentinel_pause(&path,"start-test").unwrap();
        // A pause does not erase already recorded stop evidence.
        drop(web);
        assert_eq!(pause_status(&connection),"pausing");
        drop(source);
        assert_eq!(pause_status(&connection),"paused");
        let history:(String,String,String)=connection.query_row(
            "SELECT stop_reason,checkpoint,finished_at FROM sentinel_scan_attempts WHERE attempt_number=1",[],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(history.0,"original reason");
        assert_eq!(history.1,"original checkpoint");
        assert_eq!(history.2,"2026-01-01 00:00:00");
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}
