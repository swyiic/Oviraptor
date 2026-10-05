// Genuine Single creator/startup/frozen owner and actual blocked provider.
#[test]
fn single_model_inflight_original_sdk_refuses_financial_exit_until_actual_return() {single_model_inflight_boundary(1);}
#[test]
fn single_model_inflight_second_actual_sdk_retains_first_paid_round_and_refuses_exit() {single_model_inflight_boundary(2);}
fn single_model_inflight_boundary(blocked:usize) {
    use crate::agent_runtime::multi_agent::budget::root::RootOwner;
    use std::sync::{Arc, Mutex, mpsc};
    let (f, mut context)=single_exit_private_fixture();
    let (arrived_tx,arrived)=mpsc::channel();
    let (release_tx,release)=mpsc::channel();let release=Mutex::new(release);
    let count=std::sync::atomic::AtomicUsize::new(0);
    let (port,seen,_stop)=spawn_endpoint(Arc::new(move |_| {
        if count.fetch_add(1,std::sync::atomic::Ordering::SeqCst)+1<blocked {return (200,"application/json",model_round(&[],20));}
        let _=arrived_tx.send(());let _=release.lock().unwrap().recv_timeout(Duration::from_secs(8));
        (200,"application/json",model_round(&[],20))
    }));
    context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
    let db=db::open(&f.path).unwrap();let root=&context.run.as_ref().unwrap().run_id;
    let (closed,unchanged,early)=std::thread::scope(|scope| {
        let (done_tx,done)=mpsc::channel();let context=&context;
        scope.spawn(move || {
            let client=AgentModelClient::new(agent_model_profile(&context.environment,None).unwrap(),&[]);
            for round in 1..=blocked {
                let result=native_model_transport(context,&client,vec![json!({"role":"user","content":"actual in-flight Single original request"})],&[],round as i64);
                if round<blocked {
                    let reply=result.unwrap_or_else(|_|panic!("actual first paid SDK must return"));
                    context.run.as_ref().unwrap().model_round(round as i64,&reply.usage,&[]).unwrap();
                } else {let _=done_tx.send(result.is_err());}
            }
        });
        arrived.recv_timeout(Duration::from_secs(5)).expect("actual original Single SDK must arrive");
        assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='dispatch'",[root],|r|r.get::<_,i64>(0)).unwrap(),blocked as i64);
        assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_journal WHERE phase='received'",[],|r|r.get::<_,i64>(0)).unwrap(),blocked as i64-1);
        let original=RootOwner::load_single(&db,root).unwrap();let before=single_finally_physical(&db);
        let closed=original.close_single_finance(&db);let unchanged=single_finally_physical(&db)==before;
        db.execute("UPDATE sentinel_scans SET status='pausing' WHERE id=?1",[&f.scan]).unwrap();
        let early=done.recv_timeout(Duration::from_secs(3));let _=release_tx.send(());
        let early=match early {Ok(v)=>Some(v),Err(_)=>{let _=done.recv_timeout(Duration::from_secs(5));None}};
        (closed,unchanged,early)
    });
    let error=closed.expect_err("Single finance exited while original actual SDK was still in flight");
    assert!(error.contains("single_model_transport_not_idle"),"{error}");
    assert!(unchanged,"busy original Single SDK must preserve every original row");
    assert_eq!(early,Some(true),"original pause must return from SDK before provider release");
    let until=std::time::Instant::now()+Duration::from_secs(2);
    while seen.lock().unwrap().len()<blocked && std::time::Instant::now()<until {std::thread::yield_now();}
    assert_eq!(seen.lock().unwrap().len(),blocked);
    use crate::agent_runtime::multi_agent::attempts::audit_rows::Rows;
    let fees=Rows::read(&db,"SELECT rowid,* FROM agent_budget_entries WHERE dimension<>'wall_time_ms' ORDER BY rowid",[]).unwrap();
    let calls=Rows::read(&db,"SELECT rowid,* FROM agent_root_model_journal ORDER BY rowid",[]).unwrap();
    let owner=RootOwner::load_single(&db,root).unwrap();let fact=owner.close_single_finance(&db).unwrap();
    assert!(Rows::read(&db,"SELECT rowid,* FROM agent_budget_entries WHERE dimension<>'wall_time_ms' ORDER BY rowid",[]).unwrap()==fees);
    assert!(Rows::read(&db,"SELECT rowid,* FROM agent_root_model_journal ORDER BY rowid",[]).unwrap()==calls);
    let before=single_finally_physical(&db);assert_eq!(owner.close_single_finance(&db).unwrap(),fact);
    assert_eq!(single_finally_physical(&db),before);assert!(owner.require_live(&db).is_err());
}

fn single_model_inflight_paid() -> (WebModeFixture,AgentRunContext,Seen) {
    let (f,mut context)=single_exit_private_fixture();
    let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(|_|(200,"application/json",model_round(&[],20))));
    context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
    let client=AgentModelClient::new(agent_model_profile(&context.environment,None).unwrap(),&[]);
    let reply=native_model_transport(&context,&client,vec![json!({"role":"user","content":"actual paid Single exit fixture"})],&[],1)
        .unwrap_or_else(|_|panic!("actual paid original Single SDK must return"));
    context.run.as_ref().unwrap().model_round(1,&reply.usage,&[]).unwrap();
    assert_eq!(reply.usage.total_tokens,60);assert_eq!(seen.lock().unwrap().len(),1);
    (f,context,seen)
}
fn single_model_inflight_path(f:&WebModeFixture,context:&AgentRunContext)->PathBuf {
    use sha2::Digest;
    let canonical=fs::canonicalize(&f.path).unwrap();let mut name=canonical.file_name().unwrap().to_os_string();name.push(".invocations");
    let key=serde_json::to_vec(&(&f.scan,1,"single-root-sdk",&context.run.as_ref().unwrap().run_id)).unwrap();
    canonical.with_file_name(name).join(format!("{:x}.lock",sha2::Sha256::digest(key)))
}
#[test]
fn single_model_inflight_missing_or_foreign_inode_refuses_exit_and_next_sdk_without_creating_proof() {
    use crate::agent_runtime::multi_agent::budget::root::RootOwner;
    for foreign in [false,true] {
        let (f,context,seen)=single_model_inflight_paid();let db=db::open(&f.path).unwrap();let root=&context.run.as_ref().unwrap().run_id;
        let owner=RootOwner::load_single(&db,root).unwrap();let path=single_model_inflight_path(&f,&context);assert!(path.is_file());fs::remove_file(&path).unwrap();
        if foreign {drop(crate::agent_runtime::execution_owner::claim_native_invocation(&f.path,&f.scan,1,"single-root-sdk","foreign-single-root").unwrap());}
        let before=single_finally_physical(&db);let error=owner.close_single_finance(&db).unwrap_err();
        assert!(error.contains("single_model_transport_original_exit_proof_missing"),"{error}");
        assert_eq!(single_finally_physical(&db),before);assert!(!path.exists());
        let client=AgentModelClient::new(agent_model_profile(&context.environment,None).unwrap(),&[]);
        assert!(native_model_transport(&context,&client,vec![json!({"role":"user","content":"missing original Single proof cannot authorize second SDK"})],&[],2).is_err());
        assert_eq!(single_finally_physical(&db),before);assert!(!path.exists());assert_eq!(seen.lock().unwrap().len(),1);
    }
}
#[test]
fn single_model_inflight_original_hash_metadata_or_role_damage_preserves_every_row() {
    use crate::agent_runtime::multi_agent::budget::root::RootOwner;
    for damage in [
        "DROP TRIGGER root_model_no_update; UPDATE agent_root_model_journal SET request_hash='ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff' WHERE phase='dispatch'",
        "DROP TRIGGER root_model_no_update; UPDATE agent_root_model_journal SET receipt_json='{}' WHERE phase='dispatch'",
        "UPDATE agent_runs SET role='web_executor' WHERE role='coordinator'",
    ] {
        let (f,context,seen)=single_model_inflight_paid();let db=db::open(&f.path).unwrap();let owner=RootOwner::load_single(&db,&context.run.as_ref().unwrap().run_id).unwrap();
        db.execute_batch(damage).unwrap();let before=single_finally_physical(&db);
        assert!(owner.close_single_finance(&db).is_err(),"{damage}");assert_eq!(single_finally_physical(&db),before);assert_eq!(seen.lock().unwrap().len(),1);
    }
}
#[test]
fn single_model_inflight_paid_exit_preserves_original_fees_and_replay_cannot_authorize_sdk() {
    use crate::agent_runtime::{multi_agent::budget::root::RootOwner,multi_agent::attempts::audit_rows::Rows};
    let (f,context,seen)=single_model_inflight_paid();let db=db::open(&f.path).unwrap();let root=&context.run.as_ref().unwrap().run_id;
    let fees=Rows::read(&db,"SELECT rowid,* FROM agent_budget_entries WHERE dimension<>'wall_time_ms' ORDER BY rowid",[]).unwrap();
    let calls=Rows::read(&db,"SELECT rowid,* FROM agent_root_model_journal ORDER BY rowid",[]).unwrap();
    let owner=RootOwner::load_single(&db,root).unwrap();let fact=owner.close_single_finance(&db).unwrap();
    assert!(Rows::read(&db,"SELECT rowid,* FROM agent_budget_entries WHERE dimension<>'wall_time_ms' ORDER BY rowid",[]).unwrap()==fees);
    assert!(Rows::read(&db,"SELECT rowid,* FROM agent_root_model_journal ORDER BY rowid",[]).unwrap()==calls);
    let before=single_finally_physical(&db);assert_eq!(owner.close_single_finance(&db).unwrap(),fact);assert_eq!(single_finally_physical(&db),before);
    let client=AgentModelClient::new(agent_model_profile(&context.environment,None).unwrap(),&[]);
    assert!(native_model_transport(&context,&client,vec![json!({"role":"user","content":"exit is not a continuation grant"})],&[],2).is_err());
    assert_eq!(single_finally_physical(&db),before);assert_eq!(seen.lock().unwrap().len(),1);
}
#[test]
fn single_model_inflight_empty_journal_closes_without_creating_invocation_inode() {
    let (f,context)=single_exit_private_fixture();let db=db::open(&f.path).unwrap();let root=&context.run.as_ref().unwrap().run_id;
    let path=single_model_inflight_path(&f,&context);assert!(!path.exists());
    crate::agent_runtime::multi_agent::budget::root::RootOwner::load_single(&db,root).unwrap().close_single_finance(&db).unwrap();
    assert!(!path.exists());assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_journal",[],|r|r.get::<_,i64>(0)).unwrap(),0);
}
