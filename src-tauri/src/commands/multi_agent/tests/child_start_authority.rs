#[test]
fn child_start_authority_rejects_damaged_grants_without_writing() {
    use crate::agent_runtime::multi_agent::scheduler;
    for mutation in [
        "UPDATE agent_capability_leases SET revoked_at='revoked'",
        "DELETE FROM agent_capability_leases WHERE capability='mailbox.write'",
        "UPDATE agent_capability_leases SET lease_expires_at='2000-01-01'",
        "DELETE FROM agent_lane_leases",
        "UPDATE agent_contract_owners SET state='released'",
        "UPDATE agent_budget_ledger SET fencing_token='another-owner'",
        "UPDATE agent_assignments SET lease_expires_at='2000-01-01'",
        "UPDATE agent_runs SET cancel_requested_at='cancelled' WHERE role='spa_api_mapper'",
        "UPDATE agent_runs SET target_url='https://foreign.example.invalid' WHERE role='spa_api_mapper'",
    ] {
        let (root,path,_,lease)=multi_agent_test_root("child-start-authority",1000,10);
        let db=db::open(&path).unwrap();
        let child=schedule_authority_fixture(&db,&lease).unwrap();
        db.execute_batch(mutation).unwrap();
        let before=receipt_database_snapshot(&db);
        assert!(scheduler::mark_child_running(&db,&lease,&child).is_err(),"{mutation}");
        assert_eq!(receipt_database_snapshot(&db),before,"{mutation}");
        drop(db);std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn child_start_authority_rechecks_after_writes_and_rolls_back_all_changes() {
    use crate::agent_runtime::multi_agent::scheduler;
    for mutation in [
        "UPDATE agent_capability_leases SET revoked_at='revoked';",
        "DELETE FROM agent_lane_leases;",
        "UPDATE agent_contract_owners SET state='released';",
        "UPDATE agent_runs SET cancel_requested_at='cancelled' WHERE role='coordinator';",
        "UPDATE agent_runs SET target_url='https://foreign.example.invalid' WHERE role='spa_api_mapper';",
        "UPDATE agent_assignments SET task_slice_json='{}';",
        "UPDATE agent_assignments SET state='leased'; UPDATE agent_runs SET status='prepared' WHERE role='spa_api_mapper';",
    ] {
        let (root,path,_,lease)=multi_agent_test_root("child-start-postcondition",1000,10);
        let db=db::open(&path).unwrap();
        let child=schedule_authority_fixture(&db,&lease).unwrap();
        db.execute_batch(&format!("CREATE TRIGGER damage_start AFTER UPDATE OF status ON agent_runs
            WHEN NEW.role='spa_api_mapper' AND NEW.status='running' BEGIN {mutation} END;")).unwrap();
        let before=receipt_database_snapshot(&db);
        assert!(scheduler::mark_child_running(&db,&lease,&child).is_err(),"{mutation}");
        assert_eq!(receipt_database_snapshot(&db),before,"{mutation}");
        db.execute_batch("DROP TRIGGER damage_start").unwrap();
        scheduler::mark_child_running(&db,&lease,&child).unwrap();
        let running:(String,String)=db.query_row("SELECT a.state,r.status FROM agent_assignments a
            JOIN agent_runs r ON r.id=a.child_run_id WHERE a.id=?1",[&child.assignment_id],
            |r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(running,("running".into(),"running".into()));
        drop(db);std::fs::remove_dir_all(root).unwrap();
    }
}
