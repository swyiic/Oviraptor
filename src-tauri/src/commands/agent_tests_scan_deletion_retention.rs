// Current immutable paid Root evidence must not be erased by UI status labels.
#[test]
fn scan_deletion_actual_paid_root_retains_original_audit_and_explains_refusal() {
    for relabel in [false, true] {
        let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(|_|(200,"application/json",sdk_exact_model_body(&root_tick_valid_text("original paid deletion proof")))));
        let mut f=root_tick_fixture("paid-deletion-retention",&format!("http://127.0.0.1:{port}/v1"));
        native_coordinator_tick(&f.context,&f.actor).unwrap();
        let db=db::open(&f.context.db_path).unwrap();
        finish_coordinator_run(&db,&f.actor,&AgentTargetOutcome::incomplete("closed original paid Root")).unwrap();
        drop(f.parent.take());
        assert_eq!(db.query_row("SELECT status FROM agent_runs WHERE id=?1",[&f.actor.root_run_id],|r|r.get::<_,String>(0)).unwrap(),"terminal");
        if relabel { db.execute("UPDATE agent_runs SET status='completed' WHERE id=?1",[&f.actor.root_run_id]).unwrap(); }
        // Only the UI label is supplied; it cannot settle or delete proof.
        db.execute("UPDATE sentinel_scans SET status='completed' WHERE id=?1",[&f.actor.scan_id]).unwrap();
        let (attempt,target):(i64,String)=db.query_row("SELECT attempt_number,target_url FROM agent_runs WHERE id=?1",[&f.actor.root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert!(crate::agent_runtime::execution_owner::probe_native_invocation(&f.context.db_path,&f.actor.scan_id,attempt,"target",&target).unwrap().is_none());
        let before=web_mode_test_rows(&db);
        let plan=fs::read(f.context.target_dir.join("frontend-evidence.json")).unwrap();
        let error=delete_sentinel_scan_inner(&f.context.db_path,&f.actor.scan_id).unwrap_err();
        web_mode_assert_rows(&db,&before);
        assert_eq!(seen.lock().unwrap().len(),1);
        assert_eq!(root_tick_count(&db,&f.actor.root_run_id,"publication"),1);
        assert_eq!(crate::agent_runtime::multi_agent::budget::balance(&db,&f.actor.root_run_id,Some(""),"model_requests").unwrap().consumed,1);
        assert_eq!(fs::read(f.context.target_dir.join("frontend-evidence.json")).unwrap(),plan);
        assert_eq!(error,"执行退出或清理尚未确认；任务和证据已保留：scan_quiescence_original_target_exit_missing");
        assert!(crate::agent_runtime::execution_owner::probe_native_invocation(&f.context.db_path,&f.actor.scan_id,attempt,"target",&target).unwrap().is_none());
    }
}
#[test]
fn scan_deletion_paid_other_scan_does_not_prevent_unrelated_draft_deletion() {
    let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(|_|(200,"application/json",sdk_exact_model_body(&root_tick_valid_text("independent original bill")))));
    let f=root_tick_fixture("deletion-exact-retention-scope",&format!("http://127.0.0.1:{port}/v1"));
    native_coordinator_tick(&f.context,&f.actor).unwrap();
    let db=db::open(&f.context.db_path).unwrap();
    db.execute("INSERT INTO sentinel_scans(id,project_id,project_name,status) SELECT 'unpaid-delete-draft',project_id,project_name,'draft' FROM sentinel_scans WHERE id=?1",[&f.actor.scan_id]).unwrap();
    let before=web_mode_test_rows(&db);
    delete_sentinel_scan_inner(&f.context.db_path,"unpaid-delete-draft").unwrap();
    let after=web_mode_test_rows(&db);
    for (table,rows) in &before {
        if !matches!(table.as_str(),"sentinel_scans"|"sentinel_deleted_scans") {
            assert_eq!(after.iter().find(|(name,_)|name==table).map(|(_,r)|r),Some(rows),"{table}");
        }
    }
    assert_eq!(db.query_row("SELECT count(*) FROM sentinel_scans WHERE id='unpaid-delete-draft'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(seen.lock().unwrap().len(),1);
    let unchanged=web_mode_test_rows(&db);
    delete_sentinel_scan_inner(&f.context.db_path,"unpaid-delete-draft").unwrap();
    web_mode_assert_rows(&db,&unchanged);
}
#[test]
fn scan_deletion_actual_unknown_root_preserves_original_unreconciled_bill() {
    let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(|_|(503,"application/json",r#"{"error":{"message":"original provider failure"}}"#.into())));
    let mut f=root_tick_fixture("deletion-unknown-original",&format!("http://127.0.0.1:{port}/v1"));
    assert!(native_coordinator_tick(&f.context,&f.actor).is_err());
    let db=db::open(&f.context.db_path).unwrap();
    finish_coordinator_run(&db,&f.actor,&AgentTargetOutcome::incomplete("original unknown bill retained")).unwrap();
    drop(f.parent.take());
    db.execute("UPDATE sentinel_scans SET status='paused' WHERE id=?1",[&f.actor.scan_id]).unwrap();
    let (attempt,target):(i64,String)=db.query_row("SELECT attempt_number,target_url FROM agent_runs WHERE id=?1",[&f.actor.root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert!(crate::agent_runtime::execution_owner::probe_native_invocation(&f.context.db_path,&f.actor.scan_id,attempt,"target",&target).unwrap().is_none());
    let before=web_mode_test_rows(&db);
    let error=delete_sentinel_scan_inner(&f.context.db_path,&f.actor.scan_id).unwrap_err();
    web_mode_assert_rows(&db,&before);
    assert_eq!(error,"执行退出或清理尚未确认；任务和证据已保留：scan_quiescence_original_target_exit_missing");
    assert!(crate::agent_runtime::execution_owner::probe_native_invocation(&f.context.db_path,&f.actor.scan_id,attempt,"target",&target).unwrap().is_none());
    assert_eq!(seen.lock().unwrap().len(),1);
    assert_eq!(root_tick_count(&db,&f.actor.root_run_id,"publication"),0);
    let bill=crate::agent_runtime::multi_agent::budget::balance(&db,&f.actor.root_run_id,Some(""),"model_requests").unwrap();
    assert_eq!((bill.consumed,bill.indeterminate),(0,1));
}
