// Real ordinary Single creator, paid SDK and original terminal consumer. TEMP only.
fn scan_deletion_paid_single_fixture() -> (WebModeFixture, AgentRunContext, OwnedAgentTargetOutcome, String) {
    let (f,context,owned,calls)=original_terminal_dispatch(true);
    assert!(calls>0);
    let mut tally=AgentPipelineTally::default();
    assert!(record_owned_agent_target_outcome(&f.path,&f.scan,&context.route,&owned,&mut tally));
    let db=db::open(&f.path).unwrap();
    let root=owned.original_terminal.root_run_id.as_ref().unwrap().clone();
    for dimension in crate::agent_runtime::multi_agent::budget::DIMENSIONS {
        let b=crate::agent_runtime::multi_agent::budget::balance(&db,&root,None,dimension).unwrap();
        assert_eq!((b.reserved,b.indeterminate),(0,0),"fixture is not settled: {dimension}");
    }
    assert_eq!(crate::agent_runtime::multi_agent::budget::balance(&db,&root,None,"model_requests").unwrap().consumed,calls as i64);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_single_projection_receipts WHERE root_run_id=?1",[&root],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(db.query_row("SELECT status FROM agent_runs WHERE id=?1",[&root],|r|r.get::<_,String>(0)).unwrap(),"terminal");
    db.execute("UPDATE sentinel_scans SET status='completed' WHERE id=?1",[&f.scan]).unwrap();
    (f,context,owned,root)
}
fn scan_deletion_assert_original_single_retained(relabel: bool) {
    let (f,context,owned,root)=scan_deletion_paid_single_fixture();
    // The live pipeline consumes the outcome before dropping its target lock.
    drop(owned);
    let db=db::open(&f.path).unwrap();
    if relabel {db.execute("UPDATE agent_runs SET status='completed' WHERE id=?1",[&root]).unwrap();}
    let file=context.target_dir.join("frontend-evidence.json");let native=fs::read(&file).unwrap();
    let rows=web_mode_test_rows(&db);
    for _ in 0..2 {
        let error=delete_sentinel_scan_inner(&f.path,&f.scan).unwrap_err();
        assert!(error.starts_with("native_paid_audit_retention_required:"),"{error}");
        web_mode_assert_rows(&db,&rows);
        assert_eq!(fs::read(&file).unwrap(),native);
    }
    assert!(crate::agent_runtime::multi_agent::budget::root::RootOwner::load_single(&db,&root).is_ok());
}
#[test]
fn scan_deletion_actual_settled_single_retains_original_paid_sources() {
    scan_deletion_assert_original_single_retained(false);
}
#[test]
fn scan_deletion_actual_single_old_status_cannot_delete_original_paid_sources() {
    scan_deletion_assert_original_single_retained(true);
}
#[test]
fn scan_deletion_single_original_target_owner_is_not_implied_by_financial_exit() {
    let (f,_context,owned,_root)=scan_deletion_paid_single_fixture();
    let db=db::open(&f.path).unwrap();let rows=web_mode_test_rows(&db);
    let error=delete_sentinel_scan_inner(&f.path,&f.scan).unwrap_err();
    assert!(error.contains("scan_quiescence_worker_active_or_unverifiable"),"{error}");
    web_mode_assert_rows(&db,&rows);
    drop(owned);
    assert!(delete_sentinel_scan_inner(&f.path,&f.scan).unwrap_err().starts_with("native_paid_audit_retention_required:"));
    web_mode_assert_rows(&db,&rows);
}
#[test]
fn scan_deletion_single_paid_sources_do_not_block_another_draft() {
    let (f,_context,owned,root)=scan_deletion_paid_single_fixture();drop(owned);
    let db=db::open(&f.path).unwrap();
    db.execute("INSERT INTO sentinel_scans(id,project_id,project_name,status) SELECT 'other-single-draft',project_id,project_name,'draft' FROM sentinel_scans WHERE id=?1",[&f.scan]).unwrap();
    let rows=web_mode_test_rows(&db);delete_sentinel_scan_inner(&f.path,"other-single-draft").unwrap();
    let after=web_mode_test_rows(&db);
    for (table,original) in &rows {
        if !matches!(table.as_str(),"sentinel_scans"|"sentinel_deleted_scans") {
            assert_eq!(after.iter().find(|(name,_)|name==table).map(|(_,r)|r),Some(original),"{table}");
        }
    }
    assert!(crate::agent_runtime::multi_agent::budget::root::RootOwner::load_single(&db,&root).is_ok());
    delete_sentinel_scan_inner(&f.path,"other-single-draft").unwrap();web_mode_assert_rows(&db,&after);
}
#[test]
fn scan_deletion_single_paid_journal_before_publication_retains_every_original_row() {
    let (f,mut context)=single_exit_private_fixture();
    let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(|_|(200,"application/json",model_round(&[],20))));
    context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
    let client=AgentModelClient::new(agent_model_profile(&context.environment,None).unwrap(),&[]);
    let response=native_model_transport(&context,&client,vec![json!({"role":"user","content":"original unpaid-exit boundary"})],&[],1)
        .unwrap_or_else(|_|panic!("actual original SDK must return before financial publication"));
    assert_eq!(response.usage.total_tokens,60);assert_eq!(seen.lock().unwrap().len(),1);
    let root=context.run.as_ref().unwrap().run_id.clone();
    let db=db::open(&f.path).unwrap();
    assert_eq!(db.query_row("SELECT count(*) FROM agent_single_exit_receipts WHERE root_run_id=?1",[&root],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_single_projection_receipts WHERE root_run_id=?1",[&root],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='received'",[&root],|r|r.get::<_,i64>(0)).unwrap(),1);
    db.execute("UPDATE sentinel_scans SET status='paused' WHERE id=?1",[&f.scan]).unwrap();
    let rows=web_mode_test_rows(&db);
    assert!(delete_sentinel_scan_inner(&f.path,&f.scan).unwrap_err().starts_with("native_paid_audit_retention_required:"));
    web_mode_assert_rows(&db,&rows);
    assert_eq!(seen.lock().unwrap().len(),1);
}
#[test]
fn scan_deletion_single_original_financial_exit_without_sdk_or_projection_retains_sources() {
    let (f,_context,owned,calls)=original_terminal_dispatch(false);assert_eq!(calls,0);
    let root=owned.original_terminal.root_run_id.as_ref().unwrap().clone();
    let db=db::open(&f.path).unwrap();
    let owner=crate::agent_runtime::multi_agent::budget::root::RootOwner::load_single(&db,&root).unwrap();
    owner.close_single_finance(&db).unwrap();drop(owned);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_journal WHERE root_run_id=?1",[&root],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_single_exit_receipts WHERE root_run_id=?1",[&root],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_single_projection_receipts WHERE root_run_id=?1",[&root],|r|r.get::<_,i64>(0)).unwrap(),0);
    db.execute("UPDATE sentinel_scans SET status='paused' WHERE id=?1",[&f.scan]).unwrap();
    let rows=web_mode_test_rows(&db);
    assert!(delete_sentinel_scan_inner(&f.path,&f.scan).unwrap_err().starts_with("native_paid_audit_retention_required:"));
    web_mode_assert_rows(&db,&rows);
}
