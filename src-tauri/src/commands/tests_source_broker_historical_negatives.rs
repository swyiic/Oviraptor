#[test]
fn source_broker_scope_deleted_receipt_is_not_a_full_repository_grant() {
    let (root,connection,record)=analysis_view_fixture("full",true);
    let context=source_broker_context(&root,&connection,&record);
    analysis_view_run(&root,&record,|_,engine,_,_,scratch,_|Ok(source_regression_outcome(engine,scratch))).unwrap();
    assert!(agent_run_has_frozen_source(&context));
    connection.execute_batch("DROP TRIGGER analysis_view_no_delete; DELETE FROM source_analysis_views;").unwrap();
    assert!(!agent_run_has_frozen_source(&context),"a complete provenance snapshot does not replace an analysis receipt");
    let answer=agent_execute_source_tool(&context,"repo.inventory",&json!({}));
    assert!(answer.get("error").is_some(),"{answer}");
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_broker_scope_stopped_replaced_or_unbound_runs_cannot_read() {
    for change in ["no-ledger","wrong-database","scan-stopped","attempt-stopped","attempt-replaced",
        "run-stopped","run-cancelled","missing-root","foreign-root","legacy-policy","missing-view","view-tamper"] {
        let (root,connection,record)=analysis_view_fixture("full",true);
        let mut context=source_broker_context(&root,&connection,&record);
        analysis_view_run(&root,&record,|_,engine,_,_,scratch,_|Ok(source_regression_outcome(engine,scratch))).unwrap();
        match change {
            "no-ledger"=>context.run=None,
            "wrong-database"=>context.run.as_mut().unwrap().db_path=root.join("other.sqlite3"),
            "scan-stopped"=>{connection.execute("UPDATE sentinel_scans SET status='paused'",[]).unwrap();},
            "attempt-stopped"=>{connection.execute("UPDATE sentinel_scan_attempts SET status='paused'",[]).unwrap();},
            "attempt-replaced"=>{connection.execute("UPDATE sentinel_scans SET attempt_count=2",[]).unwrap();},
            "run-stopped"=>{connection.execute("UPDATE agent_runs SET status='completed'",[]).unwrap();},
            "run-cancelled"=>{connection.execute("UPDATE agent_runs SET cancel_requested_at='requested'",[]).unwrap();},
            "missing-root"=>{connection.execute("UPDATE agent_runs SET root_run_id=''",[]).unwrap();},
            "foreign-root"=>{connection.execute("UPDATE agent_runs SET root_run_id='foreign'",[]).unwrap();},
            "legacy-policy"=>connection.execute_batch("DROP TRIGGER source_scope_no_update; UPDATE source_scope_contracts SET analysis_policy_version=0;").unwrap(),
            "missing-view"=>{
                let path:String=connection.query_row("SELECT view_root FROM source_analysis_views",[],|r|r.get(0)).unwrap();
                fs::remove_dir_all(path).unwrap();
            },
            _=>{
                let path:String=connection.query_row("SELECT view_root FROM source_analysis_views",[],|r|r.get(0)).unwrap();
                fs::remove_file(Path::new(&path).join("app.py")).unwrap();
                fs::write(Path::new(&path).join("app.py"),"modified").unwrap();
            }
        }
        assert!(!agent_run_has_frozen_source(&context),"{change}");
        let answer=agent_execute_source_tool(&context,"repo.read_slice",&json!({"path":"app.py"}));
        assert!(answer.get("error").is_some(),"{change}: {answer}");
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_broker_scope_unavailable_diff_never_advertises_full_source_tools() {
    let (root,connection,record)=analysis_view_fixture("diff",false);
    let context=source_broker_context(&root,&connection,&record);
    analysis_view_run(&root,&record,|_,_,_,_,_,_|panic!("unavailable diff")).unwrap();
    assert!(!agent_run_has_frozen_source(&context));
    assert_eq!(agent_tool_specs_for(&context).len(),7);
    assert!(agent_execute_source_tool(&context,"repo.inventory",&json!({})).get("error").is_some());
    drop(connection);fs::remove_dir_all(root).unwrap();
}
