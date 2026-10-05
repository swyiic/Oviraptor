// Real fresh Single creator/frozen owner, actual localhost headers blocked.
#[test]
fn single_http_inflight_actual_original_target_request_blocks_financial_exit_until_return() {
    use crate::agent_runtime::multi_agent::budget::root::RootOwner;
    use std::sync::{Arc,Mutex,mpsc};
    let (arrive_tx,arrive)=mpsc::channel();let (release_tx,release)=mpsc::channel();let release=Mutex::new(release);
    let (port,seen,_stop)=spawn_endpoint(Arc::new(move |_| {
        let _=arrive_tx.send(());let _=release.lock().unwrap().recv_timeout(Duration::from_secs(8));
        (200,"text/plain","actual original Single target response".into())
    }));
    let target=format!("http://127.0.0.1:{port}/app");
    let f=web_mode_fixture("single",&target);let plan=web_mode_test_plan(&f);
    persist_frozen_web_execution_plan(&f.path,&f.scan,1,&f.target,&plan).unwrap();
    let mut context=test_context(&f.path,&f.target,vec![AgentIdentity::anonymous()]);
    context.scan_id=f.scan.clone();context.execution_plan=plan;context.run=runtime_open_run(&f.path,&f.scan,&context.route);
    context.target_dir=f.root.join("http-target");fs::create_dir_all(&context.target_dir).unwrap();
    let (mut runtime,request)=single_target_start(&context);
    let db=db::open(&f.path).unwrap();let root=&context.run.as_ref().unwrap().run_id;
    let (closed,unchanged,returned)=std::thread::scope(|scope| {
        let context=&context;let worker=scope.spawn(move ||agent_http_exchange(context,&mut runtime,&request));
        arrive.recv_timeout(Duration::from_secs(5)).expect("actual original Single HTTP must arrive");
        assert_eq!(db.query_row("SELECT count(*) FROM agent_http_request_claims WHERE run_id=?1 AND response_status=0",[root],|r|r.get::<_,i64>(0)).unwrap(),1);
        let original=RootOwner::load_single(&db,root).unwrap();let before=single_finally_physical(&db);
        let closed=original.close_single_finance(&db);let unchanged=single_finally_physical(&db)==before;
        let _=release_tx.send(());(closed,unchanged,worker.join().unwrap())
    });
    let error=closed.expect_err("Single finance exited while original actual HTTP remained in flight");
    assert!(error.contains("single_target_transport_not_idle"),"{error}");
    assert!(unchanged,"busy Single HTTP must preserve all original application rows");
    assert_eq!(returned.unwrap()["status"],200);assert_eq!(seen.lock().unwrap().len(),1);
    use crate::agent_runtime::multi_agent::attempts::audit_rows::Rows;
    let fees=Rows::read(&db,"SELECT rowid,* FROM agent_budget_entries WHERE dimension<>'wall_time_ms' ORDER BY rowid",[]).unwrap();
    let requests=Rows::read(&db,"SELECT rowid,* FROM agent_http_request_claims ORDER BY rowid",[]).unwrap();
    let owner=RootOwner::load_single(&db,root).unwrap();let fact=owner.close_single_finance(&db).unwrap();
    assert!(Rows::read(&db,"SELECT rowid,* FROM agent_budget_entries WHERE dimension<>'wall_time_ms' ORDER BY rowid",[]).unwrap()==fees);
    assert!(Rows::read(&db,"SELECT rowid,* FROM agent_http_request_claims ORDER BY rowid",[]).unwrap()==requests);
    let before=single_finally_physical(&db);assert_eq!(owner.close_single_finance(&db).unwrap(),fact);assert_eq!(single_finally_physical(&db),before);
    assert!(owner.require_live(&db).is_err());
}

fn single_http_inflight_fixture(port:u16)->(WebModeFixture,AgentRunContext) {
    let f=web_mode_fixture("single",&format!("http://127.0.0.1:{port}/app"));let plan=web_mode_test_plan(&f);
    persist_frozen_web_execution_plan(&f.path,&f.scan,1,&f.target,&plan).unwrap();
    let mut context=test_context(&f.path,&f.target,vec![AgentIdentity::anonymous()]);
    context.scan_id=f.scan.clone();context.execution_plan=plan;context.run=runtime_open_run(&f.path,&f.scan,&context.route);
    context.target_dir=f.root.join("http-target");fs::create_dir_all(&context.target_dir).unwrap();(f,context)
}
fn single_http_inflight_path(f:&WebModeFixture,context:&AgentRunContext)->PathBuf {
    use sha2::Digest;
    let db=db::open(&f.path).unwrap();let root=&context.run.as_ref().unwrap().run_id;
    let source:String=db.query_row("SELECT 'http:'||run_id||':'||invocation_id||':'||request_index||':'||request_hash FROM agent_http_request_claims WHERE run_id=?1 ORDER BY ordinal LIMIT 1",[root],|r|r.get(0)).unwrap();
    let canonical=fs::canonicalize(&f.path).unwrap();let mut name=canonical.file_name().unwrap().to_os_string();name.push(".invocations");
    let key=serde_json::to_vec(&(&f.scan,1,"single-target-http",&source)).unwrap();
    canonical.with_file_name(name).join(format!("{:x}.lock",sha2::Sha256::digest(key)))
}
#[test]
fn single_http_inflight_actual_received_headers_cannot_exit_before_body_or_publish_paused_output() {
    use crate::agent_runtime::{multi_agent::budget::root::RootOwner,multi_agent::attempts::audit_rows::Rows};
    use std::io::Write;
    for (paused,second) in [(false,false),(true,false),(false,true)] {
        let listener=std::net::TcpListener::bind("127.0.0.1:0").unwrap();let port=listener.local_addr().unwrap().port();
        let (headers_tx,headers)=std::sync::mpsc::channel();let (release_tx,release)=std::sync::mpsc::channel();
        let server=std::thread::spawn(move || {
            if second {
                let (mut first,_)=listener.accept().unwrap();read_http_request(&mut first).unwrap();
                first.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\npaid").unwrap();
            }
            let (mut stream,_)=listener.accept().unwrap();let request=read_http_request(&mut stream).unwrap();
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 4\r\nConnection: close\r\n\r\n").unwrap();stream.flush().unwrap();
            let _=headers_tx.send(());let _=release.recv_timeout(Duration::from_secs(8));let _=stream.write_all(b"body");request
        });
        let (f,context)=single_http_inflight_fixture(port);let db=db::open(&f.path).unwrap();let root=&context.run.as_ref().unwrap().run_id;
        let (mut runtime,request)=single_target_start(&context);
        if second {assert_eq!(agent_http_exchange(&context,&mut runtime,&request).unwrap()["status"],200);}
        let expected=if second {2} else {1};
        let (closed,unchanged,returned)=std::thread::scope(|scope| {
            let context=&context;let worker=scope.spawn(move || {
                let output=agent_http_exchange(context,&mut runtime,&request);(output,runtime.requests.len(),runtime.coverage.len())
            });
            headers.recv_timeout(Duration::from_secs(5)).expect("real HTTP response headers must be sent");
            let deadline=Instant::now()+Duration::from_secs(3);
            let received=loop {
                if db.query_row("SELECT count(*) FROM agent_http_request_claims WHERE run_id=?1 AND response_status=200",[root],|r|r.get::<_,i64>(0)).unwrap()==expected {break true;}
                if Instant::now()>=deadline {break false;} std::thread::sleep(Duration::from_millis(10));
            };
            let before=single_finally_physical(&db);let closed=RootOwner::load_single(&db,root).unwrap().close_single_finance(&db);let unchanged=single_finally_physical(&db)==before;
            if paused {db.execute("UPDATE sentinel_scans SET status='pausing' WHERE id=?1",[&f.scan]).unwrap();}
            let _=release_tx.send(());let returned=worker.join().unwrap();assert!(received,"original received header row must exist while body is withheld");(closed,unchanged,returned)
        });
        assert!(server.join().unwrap().starts_with("GET /app/plain "));
        let error=closed.expect_err("known response headers must not authorize exit while actual response body is blocked");
        assert!(error.contains("single_target_transport_not_idle"),"{error}");assert!(unchanged);
        if paused {assert!(returned.0.is_err());assert_eq!((returned.1,returned.2),(0,0));}
        else {assert_eq!(returned.0.unwrap()["status"],200);assert_eq!(returned.1,expected as usize);}
        let b=crate::agent_runtime::multi_agent::budget::balance(&db,root,Some(""),"target_requests").unwrap();assert_eq!((b.reserved,b.consumed,b.indeterminate),(0,expected,0));
        let fees=Rows::read(&db,"SELECT rowid,* FROM agent_budget_entries WHERE dimension<>'wall_time_ms' ORDER BY rowid",[]).unwrap();
        let before_claims=Rows::read(&db,"SELECT rowid,* FROM agent_http_request_claims ORDER BY rowid",[]).unwrap();
        let owner=RootOwner::load_single(&db,root).unwrap();let fact=owner.close_single_finance(&db).unwrap();
        assert!(Rows::read(&db,"SELECT rowid,* FROM agent_budget_entries WHERE dimension<>'wall_time_ms' ORDER BY rowid",[]).unwrap()==fees);
        assert!(Rows::read(&db,"SELECT rowid,* FROM agent_http_request_claims ORDER BY rowid",[]).unwrap()==before_claims);
        let before=single_finally_physical(&db);assert_eq!(owner.close_single_finance(&db).unwrap(),fact);assert_eq!(single_finally_physical(&db),before);
    }
}
#[test]
fn single_http_inflight_actual_missing_or_foreign_original_inode_denies_exit_and_next_target_without_create() {
    use crate::agent_runtime::multi_agent::budget::root::RootOwner;
    for foreign in [false,true] {
        let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(|_|(200,"text/plain","paid target".into())));
        let (f,context)=single_http_inflight_fixture(port);let (mut runtime,mut request)=single_target_start(&context);
        assert_eq!(agent_http_exchange(&context,&mut runtime,&request).unwrap()["status"],200);
        let db=db::open(&f.path).unwrap();let root=&context.run.as_ref().unwrap().run_id;let owner=RootOwner::load_single(&db,root).unwrap();
        let path=single_http_inflight_path(&f,&context);assert!(path.is_file());fs::remove_file(&path).unwrap();
        if foreign {drop(crate::agent_runtime::execution_owner::claim_native_invocation(&f.path,&f.scan,1,"single-target-http","foreign-original-http").unwrap());}
        let before=single_finally_physical(&db);let error=owner.close_single_finance(&db).unwrap_err();
        assert!(error.contains("single_target_transport_original_exit_proof_missing"),"{error}");assert_eq!(single_finally_physical(&db),before);assert!(!path.exists());
        request.url.push_str("-new");let next=agent_http_exchange(&context,&mut runtime,&request);
        let unchanged=single_finally_physical(&db)==before;let requests=seen.lock().unwrap().len();
        let error=next.expect_err("missing actual original exit proof cannot authorize a new target call");
        assert!(error.to_string().contains("single_target_transport_original_exit_proof_missing"),"{error}");assert!(unchanged);assert!(!path.exists());assert_eq!(requests,1);
    }
}
#[test]
fn single_http_inflight_actual_paid_original_scope_or_invocation_damage_keeps_every_row() {
    use crate::agent_runtime::multi_agent::budget::root::RootOwner;
    for damage in [
        "DROP TRIGGER immutable_http_request_claim; UPDATE agent_http_request_claims SET request_hash='ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff'",
        "DROP TRIGGER immutable_http_request_claim; UPDATE agent_http_request_claims SET attempt_number=attempt_number+1",
        "UPDATE tool_invocations SET tool_name='foreign_http'",
    ] {
        let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(|_|(200,"text/plain","paid target".into())));
        let (f,context)=single_http_inflight_fixture(port);let (mut runtime,request)=single_target_start(&context);
        assert_eq!(agent_http_exchange(&context,&mut runtime,&request).unwrap()["status"],200);
        let db=db::open(&f.path).unwrap();let owner=RootOwner::load_single(&db,&context.run.as_ref().unwrap().run_id).unwrap();
        db.execute_batch(damage).unwrap();let before=single_finally_physical(&db);
        assert!(owner.close_single_finance(&db).is_err(),"{damage}");assert_eq!(single_finally_physical(&db),before);assert_eq!(seen.lock().unwrap().len(),1);
    }
}
#[test]
fn single_http_inflight_actual_unknown_headers_failed_tool_can_close_but_never_retry() {
    use crate::agent_runtime::multi_agent::budget::root::RootOwner;
    let listener=std::net::TcpListener::bind("127.0.0.1:0").unwrap();let port=listener.local_addr().unwrap().port();
    let server=std::thread::spawn(move || {let (mut stream,_)=listener.accept().unwrap();let request=read_http_request(&mut stream).unwrap();(listener,request)});
    let (f,context)=single_http_inflight_fixture(port);let (mut runtime,mut request)=single_target_start(&context);
    assert_eq!(agent_http_exchange(&context,&mut runtime,&request).unwrap_err()["phase"],"awaiting_headers");let (listener,_)=server.join().unwrap();
    let db=db::open(&f.path).unwrap();let root=&context.run.as_ref().unwrap().run_id;
    db.execute("UPDATE tool_invocations SET status='failed' WHERE run_id=?1",[root]).unwrap();
    let claims=crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,"SELECT rowid,* FROM agent_http_request_claims ORDER BY rowid",[]).unwrap();
    let owner=RootOwner::load_single(&db,root).unwrap();owner.close_single_finance(&db).unwrap();
    let b=crate::agent_runtime::multi_agent::budget::balance(&db,root,Some(""),"target_requests").unwrap();assert_eq!((b.reserved,b.consumed,b.indeterminate),(0,0,1));
    assert!(crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,"SELECT rowid,* FROM agent_http_request_claims ORDER BY rowid",[]).unwrap()==claims);
    let before=single_finally_physical(&db);request.url.push_str("-new");assert!(agent_http_exchange(&context,&mut runtime,&request).is_err());assert_eq!(single_finally_physical(&db),before);
    listener.set_nonblocking(true).unwrap();assert_eq!(listener.accept().unwrap_err().kind(),std::io::ErrorKind::WouldBlock);
}

#[test]
fn single_http_inflight_empty_original_history_closes_without_creating_http_proof() {
    let (f,context)=single_http_inflight_fixture(65530);let db=db::open(&f.path).unwrap();
    let canonical=fs::canonicalize(&f.path).unwrap();let mut name=canonical.file_name().unwrap().to_os_string();name.push(".invocations");
    let directory=canonical.with_file_name(name);assert!(!directory.exists());
    let owner=crate::agent_runtime::multi_agent::budget::root::RootOwner::load_single(&db,&context.run.as_ref().unwrap().run_id).unwrap();
    owner.close_single_finance(&db).unwrap();assert!(!directory.exists());
    assert_eq!(db.query_row("SELECT count(*) FROM agent_http_request_claims",[],|r|r.get::<_,i64>(0)).unwrap(),0);
}
