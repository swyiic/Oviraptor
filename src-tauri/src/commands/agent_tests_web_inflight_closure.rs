// Actual current signed Web executor and original parent, with no target I/O.
#[test]
fn web_inflight_closure_original_sdk_blocks_root_terminal_until_actual_exit() {
    use std::sync::{Arc,Mutex,mpsc};
    let (arrive_tx,arrive)=mpsc::channel();let (release_tx,release)=mpsc::channel();let release=Mutex::new(release);
    let (port,seen,_stop)=spawn_endpoint(Arc::new(move |_|{
        let _=arrive_tx.send(());let _=release.lock().unwrap().recv_timeout(Duration::from_secs(8));
        (200,"application/json",model_round(&[],20))
    }));
    let (directory,mut context,parent)=specialist_inflight_original_executor_context();
    context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
    let client=AgentModelClient::new(agent_model_profile(&context.environment,None).unwrap(),&[]);
    let db=db::open(&context.db_path).unwrap();
    let child=context.run.as_ref().unwrap().run_id.clone();
    let actor=db.query_row("SELECT c.scan_id,c.attempt_number,c.target_key,c.root_run_id,c.lease_epoch,c.fencing_token,c.lease_expires_at
        FROM agent_coordinator_leases c JOIN agent_runs r ON r.root_run_id=c.root_run_id WHERE r.id=?1",[&child],
        |r|Ok(crate::agent_runtime::multi_agent::lease::CoordinatorLease {scan_id:r.get(0)?,attempt_number:r.get(1)?,target_key:r.get(2)?,
            root_run_id:r.get(3)?,lease_epoch:r.get(4)?,fencing_token:r.get(5)?,lease_expires_at:r.get(6)?})).unwrap();
    let outcome=AgentTargetOutcome::incomplete("original Web SDK is still in flight");
    let (closing,unchanged,early)=std::thread::scope(|scope|{
        let (done_tx,done)=mpsc::channel();let c=&context;let client=&client;
        scope.spawn(move ||{let _=done_tx.send(native_model_transport(c,client,vec![json!({"role":"user","content":"original Web read-only SDK boundary"})],&[],1));});
        arrive.recv_timeout(Duration::from_secs(5)).unwrap();
        let before=super::tests::application_table_snapshot(&db);
        let closing=finish_coordinator_run(&db,&actor,&outcome);
        let unchanged=super::tests::application_table_snapshot(&db)==before;
        db.execute("UPDATE agent_runs SET cancel_requested_at=datetime('now','localtime') WHERE id=?1",[&actor.root_run_id]).unwrap();
        let early=done.recv_timeout(Duration::from_secs(3));let _=release_tx.send(());
        let returned=match early {Ok(r)=>Some(r),Err(_)=>{let _=done.recv_timeout(Duration::from_secs(5));None}};
        (closing,unchanged,returned)
    });
    assert!(closing.is_err(),"original Root published terminal while actual Web SDK was still in flight");
    assert!(unchanged,"busy Web SDK must preserve every original application row");
    assert!(early.is_some_and(|r|r.is_err()),"original cancellation must return from SDK before provider release");
    use crate::agent_runtime::multi_agent::attempts::audit_rows::Rows;
    let fees=Rows::read(&db,"SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND dimension<>'wall_time_ms' ORDER BY rowid",[&actor.root_run_id]).unwrap();
    let calls=Rows::read(&db,"SELECT rowid,* FROM agent_web_model_journal WHERE root_run_id=?1 ORDER BY rowid",[&actor.root_run_id]).unwrap();
    finish_coordinator_run(&db,&actor,&outcome).unwrap();
    // The Root final clock is independent; every original Web fee stays intact.
    let after=Rows::read(&db,"SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND dimension<>'wall_time_ms' ORDER BY rowid",[&actor.root_run_id]).unwrap();
    assert!(after==fees);assert!(!fees.values.is_empty());
    assert!(Rows::read(&db,"SELECT rowid,* FROM agent_web_model_journal WHERE root_run_id=?1 ORDER BY rowid",[&actor.root_run_id]).unwrap()==calls);
    let replay=super::tests::application_table_snapshot(&db);finish_coordinator_run(&db,&actor,&outcome).unwrap();assert!(super::tests::application_table_snapshot(&db)==replay);
    let deadline=Instant::now()+Duration::from_secs(1);while seen.lock().unwrap().is_empty() && Instant::now()<deadline {std::thread::yield_now();}
    assert_eq!(seen.lock().unwrap().len(),1);
    drop(db);drop(context);drop(parent);fs::remove_dir_all(directory).unwrap();
}

fn web_inflight_original_actor(db:&rusqlite::Connection,child:&str)->crate::agent_runtime::multi_agent::lease::CoordinatorLease {
    db.query_row("SELECT c.scan_id,c.attempt_number,c.target_key,c.root_run_id,c.lease_epoch,c.fencing_token,c.lease_expires_at
        FROM agent_coordinator_leases c JOIN agent_runs r ON r.root_run_id=c.root_run_id WHERE r.id=?1",[child],
        |r|Ok(crate::agent_runtime::multi_agent::lease::CoordinatorLease {scan_id:r.get(0)?,attempt_number:r.get(1)?,target_key:r.get(2)?,
            root_run_id:r.get(3)?,lease_epoch:r.get(4)?,fencing_token:r.get(5)?,lease_expires_at:r.get(6)?})).unwrap()
}
fn web_inflight_original_path(context:&AgentRunContext)->PathBuf {
    use sha2::Digest;
    let path=fs::canonicalize(&context.db_path).unwrap();let mut name=path.file_name().unwrap().to_os_string();name.push(".invocations");
    let key=serde_json::to_vec(&(&context.scan_id,context.attempt_number,"web-executor-sdk",&context.run.as_ref().unwrap().run_id)).unwrap();
    path.with_file_name(name).join(format!("{:x}.lock",sha2::Sha256::digest(key)))
}
fn web_inflight_actual_finished(paid:bool)->(PathBuf,AgentRunContext,crate::agent_runtime::multi_agent::supervisor::WorkerSupervisor,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,AgentModelClient,Seen) {
    let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(move |_|if paid {(200,"application/json",model_round(&[],20))}
        else {(503,"application/json",r#"{"error":{"message":"fixture unavailable"}}"#.into())}));
    let (directory,mut context,parent)=specialist_inflight_original_executor_context();
    context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
    let client=AgentModelClient::new(agent_model_profile(&context.environment,None).unwrap(),&[]);
    let db=db::open(&context.db_path).unwrap();let actor=web_inflight_original_actor(&db,&context.run.as_ref().unwrap().run_id);
    let result=native_model_transport(&context,&client,vec![json!({"role":"user","content":"original completed Web SDK"})],&[],1);
    assert_eq!(result.is_ok(),paid);assert_eq!(seen.lock().unwrap().len(),1);assert!(web_inflight_original_path(&context).is_file());
    (directory,context,parent,actor,client,seen)
}
#[test]
fn web_inflight_closure_actual_missing_or_foreign_original_lock_never_creates_proof_on_close_or_reentry() {
    for foreign in [false,true] {
        let (directory,context,parent,actor,client,seen)=web_inflight_actual_finished(false);
        let path=web_inflight_original_path(&context);fs::remove_file(&path).unwrap();
        if foreign {drop(crate::agent_runtime::execution_owner::claim_native_invocation(&context.db_path,&actor.scan_id,
            actor.attempt_number,"web-executor-sdk","foreign-web-worker").unwrap());}
        let db=db::open(&context.db_path).unwrap();let before=super::tests::application_table_snapshot(&db);
        let error=finish_coordinator_run(&db,&actor,&AgentTargetOutcome::incomplete("original Web proof missing")).unwrap_err();
        assert!(error.contains("web_model_transport_original_exit_proof_missing"),"{error}");
        assert!(super::tests::application_table_snapshot(&db)==before);assert!(!path.exists());
        assert!(native_model_transport(&context,&client,vec![],&[],1).is_err());
        assert!(super::tests::application_table_snapshot(&db)==before);assert!(!path.exists());assert_eq!(seen.lock().unwrap().len(),1);
        drop(db);drop(context);drop(parent);fs::remove_dir_all(directory).unwrap();
    }
}
#[test]
fn web_inflight_closure_actual_changed_original_hash_dispatch_or_worker_role_keeps_every_row() {
    for damage in [
        "UPDATE agent_web_model_journal SET request_hash='ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff' WHERE phase='dispatch'",
        "UPDATE agent_web_model_journal SET receipt_json='{\"changed\":true}' WHERE phase='dispatch'",
        "UPDATE agent_runs SET role='deep_investigator' WHERE role='web_executor'",
    ] {
        let (directory,context,parent,actor,_client,seen)=web_inflight_actual_finished(false);
        let db=db::open(&context.db_path).unwrap();
        db.execute_batch("DROP TRIGGER web_model_journal_no_update").unwrap();db.execute_batch(damage).unwrap();
        let before=super::tests::application_table_snapshot(&db);
        assert!(finish_coordinator_run(&db,&actor,&AgentTargetOutcome::incomplete("original Web scope damaged")).is_err(),"{damage}");
        assert!(super::tests::application_table_snapshot(&db)==before,"{damage}");assert_eq!(seen.lock().unwrap().len(),1);
        drop(db);drop(context);drop(parent);fs::remove_dir_all(directory).unwrap();
    }
}
#[test]
fn web_inflight_closure_actual_paid_sdk_preserves_original_fee_and_never_accepts_another_round_after_terminal() {
    let (directory,context,parent,actor,client,seen)=web_inflight_actual_finished(true);
    let db=db::open(&context.db_path).unwrap();
    use crate::agent_runtime::multi_agent::attempts::audit_rows::Rows;
    let fees=Rows::read(&db,"SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND dimension<>'wall_time_ms' ORDER BY rowid",[&actor.root_run_id]).unwrap();
    let calls=Rows::read(&db,"SELECT rowid,* FROM agent_web_model_journal WHERE root_run_id=?1 ORDER BY rowid",[&actor.root_run_id]).unwrap();
    finish_coordinator_run(&db,&actor,&AgentTargetOutcome::incomplete("actual Web SDK returned")).unwrap();
    assert!(Rows::read(&db,"SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND dimension<>'wall_time_ms' ORDER BY rowid",[&actor.root_run_id]).unwrap()==fees);
    assert!(Rows::read(&db,"SELECT rowid,* FROM agent_web_model_journal WHERE root_run_id=?1 ORDER BY rowid",[&actor.root_run_id]).unwrap()==calls);
    let balance=crate::agent_runtime::multi_agent::budget::balance(&db,&actor.root_run_id,None,"model_requests").unwrap();
    assert_eq!((balance.consumed,balance.indeterminate),(1,0));
    let before=super::tests::application_table_snapshot(&db);assert!(native_model_transport(&context,&client,vec![],&[],2).is_err());
    finish_coordinator_run(&db,&actor,&AgentTargetOutcome::incomplete("actual Web SDK returned")).unwrap();
    assert!(super::tests::application_table_snapshot(&db)==before);assert_eq!(seen.lock().unwrap().len(),1);
    drop(db);drop(context);drop(parent);fs::remove_dir_all(directory).unwrap();
}
#[test]
fn web_inflight_closure_pristine_no_sdk_does_not_create_an_original_exit_inode() {
    let (directory,context,parent)=specialist_inflight_original_executor_context();
    let db=db::open(&context.db_path).unwrap();let actor=web_inflight_original_actor(&db,&context.run.as_ref().unwrap().run_id);
    let path=web_inflight_original_path(&context);assert!(!path.exists());
    let count:i64=db.query_row("SELECT count(*) FROM agent_web_model_journal",[],|r|r.get(0)).unwrap();assert_eq!(count,0);
    finish_coordinator_run(&db,&actor,&AgentTargetOutcome::incomplete("pristine Web SDK was never dispatched")).unwrap();
    assert!(!path.exists());
    drop(db);drop(context);drop(parent);fs::remove_dir_all(directory).unwrap();
}

// Actual signed Root bootstrap and actual Root/Mapper SDK responses.
fn web_closure_prepare_harness(tag:&str)->(AgentHarness,Seen) {
    let mut h=fresh_multi_production_harness(tag);
    let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(|request|
        (200,"application/json",fresh_directive_closure_wire_response(&request)
            .expect("original closure fixture expects Root/Mapper SDK"))));
    h.context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
    freeze_fresh_multi_production_harness(&mut h);
    (h,seen)
}
