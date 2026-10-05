#[test]
fn source_financial_exit_exceptional_faults_rollback_fact_and_preserve_caller_authorizer() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    for fault in [
        "BEFORE INSERT ON agent_root_elapsed_facts BEGIN SELECT RAISE(IGNORE); END;",
        "AFTER INSERT ON agent_root_elapsed_facts BEGIN UPDATE projects SET status='archived'; END;",
    ] {
        let (root,db,_,result)=source_reviewer_execution_fixture_using_calls("valid",None,4,None,0,|root,db,record,c| {
            db.execute("UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00' WHERE root_run_id=?1",[&c.root_run_id]).unwrap();
            db.execute_batch(&format!("CREATE TRIGGER source_exit_exceptional_fault {fault}")).unwrap();
            let before=source_exit_snapshot(db);
            let denials=Arc::new(AtomicUsize::new(0));let original=denials.clone();
            db.authorizer(Some(move |context:AuthContext<'_>|match context.action {
                AuthAction::Insert {table_name:"projects"}=>{original.fetch_add(1,Ordering::SeqCst);Authorization::Deny},
                _=>Authorization::Allow,
            })).unwrap();
            let result=run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001"));
            let error=result.as_ref().unwrap_err();
            assert!(error.contains("source_financial_exit:"),"exceptional financial writer was skipped: {error}");
            assert!(!error.starts_with("source_financial_exit:"),"primary refusal must survive");
            assert_eq!(source_exit_snapshot(db),before,"failed financial writer changed original rows");
            assert!(db.execute("INSERT INTO projects(id,name) VALUES(9002,'caller hook must survive')",[]).is_err());
            assert_eq!(denials.load(Ordering::SeqCst),1,"private writer replaced caller hook");
            assert_eq!(source_exit_snapshot(db),before);
            result
        });
        assert!(result.is_err());drop(db);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_financial_exit_does_not_join_or_commit_a_callers_open_transaction() {
    let (root, db, _, result) = source_reviewer_execution_fixture_using_calls(
        "valid",
        None,
        4,
        None,
        0,
        |_, db, _, c| {
            let before = source_exit_snapshot(db);
            let tx = db.unchecked_transaction().unwrap();
            let error=crate::agent_runtime::multi_agent::budget::clock::observation::record_original_exit(&tx,c).unwrap_err();
            assert_eq!(error, "budget_exit_observation_transaction_required");
            assert!(!db.is_autocommit());
            assert_eq!(source_exit_snapshot(&tx), before);
            tx.rollback().unwrap();
            assert_eq!(source_exit_snapshot(db), before);
            Err(error)
        },
    );
    assert!(result.is_err());
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
