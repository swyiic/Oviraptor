// Actual original Root SDK, not a financial seed or UI simulation.
#[test]
fn root_inflight_closure_actual_original_sdk_blocks_terminal_until_return_and_preserves_bill() {
    root_inflight_closure_original_boundary(false);
}
#[test]
fn root_inflight_closure_actual_second_sdk_keeps_first_paid_local_tool_and_original_native_rows() {
    root_inflight_closure_original_boundary(true);
}
fn root_inflight_closure_original_boundary(paid_first: bool) {
    use std::sync::{Arc,Mutex,mpsc};
    let (arrive_tx,arrive)=mpsc::channel();let (release_tx,release)=mpsc::channel();let release=Mutex::new(release);
    let counter=std::sync::atomic::AtomicUsize::new(0);
    let (port,seen,_stop)=spawn_endpoint(Arc::new(move |_|{
        if paid_first && counter.fetch_add(1,std::sync::atomic::Ordering::SeqCst)==0 {
            return (200,"application/json",json!({"choices":[{"message":{"role":"assistant","tool_calls":[{"id":"original-first-snapshot","type":"function","function":{"name":"snapshot.read","arguments":"{}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}}).to_string());
        }
        let _=arrive_tx.send(());let _=release.lock().unwrap().recv_timeout(Duration::from_secs(8));
        (200,"application/json",sdk_exact_model_body(&root_tick_valid_text("original blocked Root")))
    }));
    let f=root_tick_fixture("root-terminal-inflight",&format!("http://127.0.0.1:{port}/v1"));
    let db=db::open(&f.context.db_path).unwrap();let outcome=AgentTargetOutcome::incomplete("original Root SDK still in flight");
    let (closing,unchanged,early)=std::thread::scope(|scope|{
        let (done_tx,done)=mpsc::channel();let (context,actor)=(&f.context,&f.actor);
        scope.spawn(move ||{let _=done_tx.send(native_coordinator_tick(context,actor));});
        arrive.recv_timeout(Duration::from_secs(5)).unwrap();
        let before=super::tests::application_table_snapshot(&db);
        let closing=finish_coordinator_run(&db,&f.actor,&outcome);
        let unchanged=super::tests::application_table_snapshot(&db)==before;
        db.execute("UPDATE agent_runs SET cancel_requested_at=datetime('now','localtime') WHERE id=?1",[&f.actor.root_run_id]).unwrap();
        let early=done.recv_timeout(Duration::from_secs(3));let _=release_tx.send(());
        let returned=match early {Ok(r)=>Some(r),Err(_)=>{let _=done.recv_timeout(Duration::from_secs(5));None}};
        (closing,unchanged,returned)
    });
    assert!(closing.is_err(),"Root terminal was published before actual original SDK exit");
    assert!(unchanged,"busy original SDK must preserve every prior application row");
    assert!(early.is_some_and(|r|r.is_err()),"original cancellation must exit SDK before provider release");
    use crate::agent_runtime::multi_agent::attempts::audit_rows::Rows;
    let fees=Rows::read(&db,"SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND source_id IN (SELECT call_id FROM agent_root_model_journal WHERE root_run_id=?1) ORDER BY rowid",[&f.actor.root_run_id]).unwrap();
    let journal=Rows::read(&db,"SELECT rowid,* FROM agent_root_model_journal WHERE root_run_id=?1 ORDER BY rowid",[&f.actor.root_run_id]).unwrap();
    finish_coordinator_run(&db,&f.actor,&outcome).unwrap();
    assert!(Rows::read(&db,"SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND source_id IN (SELECT call_id FROM agent_root_model_journal WHERE root_run_id=?1) ORDER BY rowid",[&f.actor.root_run_id]).unwrap()==fees);
    assert!(Rows::read(&db,"SELECT rowid,* FROM agent_root_model_journal WHERE root_run_id=?1 ORDER BY rowid",[&f.actor.root_run_id]).unwrap()==journal);
    let replay=super::tests::application_table_snapshot(&db);finish_coordinator_run(&db,&f.actor,&outcome).unwrap();
    assert!(super::tests::application_table_snapshot(&db)==replay);let fee=crate::agent_runtime::multi_agent::budget::balance(&db,&f.actor.root_run_id,None,"model_requests").unwrap();
    assert_eq!((fee.consumed,fee.indeterminate),(i64::from(paid_first),1));
    let deadline=std::time::Instant::now()+Duration::from_secs(1);
    while seen.lock().unwrap().len()<1+usize::from(paid_first) && std::time::Instant::now()<deadline {std::thread::yield_now();}
    assert_eq!(seen.lock().unwrap().len(),1+usize::from(paid_first));
}

fn root_inflight_closure_finished_original() -> (RootTickFixture,Seen) {
    let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(|_|(503,"application/json",r#"{"error":{"message":"fixture unavailable"}}"#.into())));
    let f=root_tick_fixture("root-original-missing-proof",&format!("http://127.0.0.1:{port}/v1"));
    assert!(native_coordinator_tick(&f.context,&f.actor).is_err());
    (f,seen)
}
fn root_inflight_closure_original_path(f:&RootTickFixture)->PathBuf {
    use sha2::Digest;
    let db=fs::canonicalize(&f.context.db_path).unwrap();let mut name=db.file_name().unwrap().to_os_string();name.push(".invocations");
    let key=serde_json::to_vec(&(&f.actor.scan_id,f.actor.attempt_number,"root-decision-sdk",&f.actor.root_run_id)).unwrap();
    db.with_file_name(name).join(format!("{:x}.lock",sha2::Sha256::digest(key)))
}
#[test]
fn root_inflight_closure_actual_missing_original_or_foreign_lock_keeps_all_rows_and_never_creates_proof() {
    for foreign in [false,true] {
        let (f,seen)=root_inflight_closure_finished_original();let path=root_inflight_closure_original_path(&f);assert!(path.is_file());
        fs::remove_file(&path).unwrap(); // Only temporary corruption after actual original SDK returned.
        if foreign {drop(crate::agent_runtime::execution_owner::claim_native_invocation(&f.context.db_path,&f.actor.scan_id,
            f.actor.attempt_number,"root-decision-sdk","foreign-root").unwrap());}
        let db=db::open(&f.context.db_path).unwrap();let before=super::tests::application_table_snapshot(&db);
        let error=finish_coordinator_run(&db,&f.actor,&AgentTargetOutcome::incomplete("original physical proof missing")).unwrap_err();
        assert!(error.contains("root_decision_transport_original_exit_proof_missing"),"{error}");
        assert!(super::tests::application_table_snapshot(&db)==before);assert!(!path.exists());assert_eq!(seen.lock().unwrap().len(),1);
    }
}
#[test]
fn root_inflight_closure_actual_changed_original_request_hash_or_fact_cannot_close_or_change_bill() {
    for damage in [
        "UPDATE agent_root_tick_receipts SET request_hash='ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff' WHERE phase='request'",
        "UPDATE agent_root_tick_receipts SET fact_json=json_set(fact_json,'$.request.owner',json('{}')) WHERE phase='request'",
    ] {
        let (f,seen)=root_inflight_closure_finished_original();let db=db::open(&f.context.db_path).unwrap();
        db.execute_batch("DROP TRIGGER root_tick_no_update").unwrap();db.execute_batch(damage).unwrap();
        let before=super::tests::application_table_snapshot(&db);
        assert!(finish_coordinator_run(&db,&f.actor,&AgentTargetOutcome::incomplete("original dispatch damaged")).is_err(),"{damage}");
        assert!(super::tests::application_table_snapshot(&db)==before,"{damage}");assert_eq!(seen.lock().unwrap().len(),1);
    }
}
