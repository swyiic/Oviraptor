// Actual ordinary provider failure must not leave executable local authority.
#[test]
fn specialist_readonly_closure_actual_503_pauses_original_worker_and_keeps_unknown_bill() {
    let (root, context, actor, child, seen) = specialist_inflight_finished_original();
    let db = db::open(&context.db_path).unwrap();
    let before = specialist_readonly_closure_frozen(&db, &child);
    let outcome = AgentTargetOutcome::incomplete("original readonly assessment requires reconciliation");
    finish_coordinator_run(&db, &actor, &outcome).unwrap();
    let state: (String,String,String,i64,i64) = db.query_row("SELECT a.state,r.status,x.state,
        (SELECT count(*) FROM agent_capability_leases WHERE child_run_id=r.id AND revoked_at=''),
        (SELECT count(*) FROM agent_lane_leases WHERE assignment_id=a.id)
        FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id JOIN agent_assignment_attempts x ON x.assignment_id=a.id
        WHERE a.id=?1", [&child.assignment_id], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
    assert_eq!(state, ("paused".into(),"paused".into(),"paused".into(),0,0), "Root terminal must revoke original readonly authority without completing or refunding unknown usage");
    assert!(specialist_readonly_closure_frozen(&db,&child)==before);
    let replay = super::tests::application_table_snapshot(&db);
    finish_coordinator_run(&db,&actor,&outcome).unwrap();
    assert!(super::tests::application_table_snapshot(&db)==replay);
    assert!(multi_agent_child_round_transport(&context,&actor,&child,"readonly",json!({})).is_err());
    assert!(super::tests::application_table_snapshot(&db)==replay);
    assert_eq!(seen.lock().unwrap().len(),1);
    drop(db); drop(context); fs::remove_dir_all(root).unwrap();
}
fn specialist_readonly_closure_frozen(db: &rusqlite::Connection,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
) -> Vec<crate::agent_runtime::multi_agent::attempts::audit_rows::Rows> {
    use crate::agent_runtime::multi_agent::attempts::audit_rows::Rows;
    vec![
        Rows::read(db,"SELECT rowid,* FROM agent_budget_entries WHERE assignment_id=?1 ORDER BY rowid",[&child.assignment_id]).unwrap(),
        Rows::read(db,"SELECT rowid,* FROM agent_model_cost_facts WHERE assignment_id=?1 ORDER BY rowid",[&child.assignment_id]).unwrap(),
        Rows::read(db,"SELECT rowid,* FROM agent_specialist_calls WHERE assignment_id=?1 ORDER BY rowid",[&child.assignment_id]).unwrap(),
        Rows::read(db,"SELECT rowid,* FROM agent_events WHERE run_id=?1 ORDER BY rowid",[&child.run_id]).unwrap(),
        Rows::read(db,"SELECT rowid,* FROM agent_snapshots WHERE run_id=?1 ORDER BY rowid",[&child.run_id]).unwrap(),
        Rows::read(db,"SELECT reserved_tokens,reserved_requests,budget_settled_at FROM agent_assignments WHERE id=?1",[&child.assignment_id]).unwrap(),
    ]
}

#[test]
fn specialist_readonly_closure_actual_received_three_roles_keep_original_paid_receipts() {
    use crate::agent_runtime::{contract::{AgentRole,AgentLane},multi_agent::scheduler};
    for role in [AgentRole::SpaApiMapper,AgentRole::IdentitySession,AgentRole::DeepInvestigator] {
        let (root,path,run,actor)=multi_agent_new_task_root("original-paid-readonly",60000,20);
        let db=db::open(&path).unwrap();
        if role==AgentRole::IdentitySession { configure_specialist_identity(&db,&actor); }
        let child=scheduler::schedule_child(&db,&actor,role,AgentLane::ReadOnlyAnalysis,"original-paid",&json!({}),1,
            &["evidence.read".into(),"mailbox.write".into()],8000,1).unwrap();
        scheduler::start_child_or_release(&db,&actor,&child).unwrap();
        let mut context=test_context(&path,&actor.target_key,vec![AgentIdentity::anonymous()]);
        context.scan_id=actor.scan_id.clone();context.run=Some(AgentRunLedger{db_path:path,run_id:run});
        let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(|_|(200,"application/json",proposal_model_response("original paid readonly assessment"))));
        context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
        multi_agent_child_round_transport(&context,&actor,&child,"readonly",json!({})).unwrap();
        let before=specialist_readonly_closure_frozen(&db,&child);
        let outcome=AgentTargetOutcome::incomplete("original paid assessment has not been delivered");
        finish_coordinator_run(&db,&actor,&outcome).unwrap();
        let state:(String,String,String,i64,i64)=db.query_row("SELECT a.state,r.status,x.state,
            (SELECT count(*) FROM agent_capability_leases WHERE child_run_id=r.id AND revoked_at=''),
            (SELECT count(*) FROM agent_lane_leases WHERE assignment_id=a.id)
            FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id JOIN agent_assignment_attempts x ON x.assignment_id=a.id
            WHERE a.id=?1",[&child.assignment_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
        assert_eq!(state,("paused".into(),"paused".into(),"paused".into(),0,0));
        assert!(specialist_readonly_closure_frozen(&db,&child)==before);
        let paid=crate::agent_runtime::multi_agent::budget::balance(&db,&actor.root_run_id,Some(&child.assignment_id),"model_requests").unwrap();
        assert_eq!((paid.consumed,paid.indeterminate),(1,0));
        let replay=super::tests::application_table_snapshot(&db);
        finish_coordinator_run(&db,&actor,&outcome).unwrap();
        assert!(super::tests::application_table_snapshot(&db)==replay);
        assert_eq!(seen.lock().unwrap().len(),1);
        drop(db);drop(context);fs::remove_dir_all(root).unwrap();
    }
}
#[test]
fn specialist_readonly_closure_actual_seven_write_faults_roll_back_every_original_row() {
    for fault in [
        "BEFORE UPDATE OF state ON agent_assignments WHEN NEW.state='paused' BEGIN SELECT RAISE(IGNORE); END;",
        "BEFORE UPDATE OF status ON agent_runs WHEN NEW.status='paused' BEGIN SELECT RAISE(IGNORE); END;",
        "BEFORE UPDATE OF state ON agent_assignment_attempts WHEN NEW.state='paused' BEGIN SELECT RAISE(IGNORE); END;",
        "BEFORE UPDATE OF revoked_at ON agent_capability_leases BEGIN SELECT RAISE(IGNORE); END;",
        "BEFORE DELETE ON agent_lane_leases BEGIN SELECT RAISE(IGNORE); END;",
        "AFTER UPDATE OF state ON agent_assignments WHEN NEW.state='paused' BEGIN UPDATE projects SET status='archived'; END;",
        "AFTER UPDATE OF state ON agent_assignments WHEN NEW.state='paused' BEGIN UPDATE agent_budget_ledger SET reserved_requests=0; END;",
    ] {
        let (root,context,actor,_child,seen)=specialist_inflight_finished_original();
        let db=db::open(&context.db_path).unwrap();
        db.execute_batch(&format!("CREATE TRIGGER readonly_closure_fault {fault}")).unwrap();
        let before=super::tests::application_table_snapshot(&db);
        assert!(finish_coordinator_run(&db,&actor,&AgentTargetOutcome::incomplete("fault must retain original costs and authority")).is_err(),"{fault}");
        assert!(super::tests::application_table_snapshot(&db)==before,"{fault}");
        assert_eq!(seen.lock().unwrap().len(),1);
        drop(db);drop(context);fs::remove_dir_all(root).unwrap();
    }
}
#[test]
fn specialist_readonly_closure_actual_foreign_capability_or_lane_cannot_be_revoked_as_original() {
    for damage in [
        "UPDATE agent_capability_leases SET fencing_token='foreign-coordinator' WHERE revoked_at=''",
        "UPDATE agent_lane_leases SET target_key='http://foreign.invalid'",
    ] {
        let (root,context,actor,_child,seen)=specialist_inflight_finished_original();
        let db=db::open(&context.db_path).unwrap();
        db.execute_batch(damage).unwrap();
        let before=super::tests::application_table_snapshot(&db);
        assert!(finish_coordinator_run(&db,&actor,&AgentTargetOutcome::incomplete("foreign cleanup scope")).is_err(),"{damage}");
        assert!(super::tests::application_table_snapshot(&db)==before,"{damage}");
        assert_eq!(seen.lock().unwrap().len(),1);
        drop(db);drop(context);fs::remove_dir_all(root).unwrap();
    }
}
#[test]
fn specialist_readonly_closure_actual_paid_completed_sibling_is_unchanged_when_second_is_paused() {
    use crate::agent_runtime::{contract::{AgentRole,AgentLane},multi_agent::scheduler};
    let (root,mut context,actor,first)=specialist_financial_fixture();
    let count=std::sync::atomic::AtomicUsize::new(0);
    let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(move |_|{
        if count.fetch_add(1,std::sync::atomic::Ordering::SeqCst)==0 {
            (200,"application/json",proposal_model_response("original completed paid sibling"))
        } else {(503,"application/json",r#"{"error":{"message":"fixture unavailable"}}"#.into())}
    }));
    context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
    let (_,usage)=multi_agent_child_round_transport(&context,&actor,&first,"readonly",json!({})).unwrap();
    let db=db::open(&context.db_path).unwrap();settle_child_usage(&db,&actor,&first,&usage).unwrap();
    scheduler::finish_child(&db,&actor,&first,true,"original paid result completed").unwrap();
    use crate::agent_runtime::multi_agent::attempts::audit_rows::Rows;
    let previous=vec![Rows::read(&db,"SELECT rowid,* FROM agent_assignments WHERE id=?1",[&first.assignment_id]).unwrap(),
        Rows::read(&db,"SELECT rowid,* FROM agent_runs WHERE id=?1",[&first.run_id]).unwrap(),
        Rows::read(&db,"SELECT rowid,* FROM agent_assignment_attempts WHERE assignment_id=?1",[&first.assignment_id]).unwrap()];
    let first_fee=specialist_readonly_closure_frozen(&db,&first);
    let second=scheduler::schedule_child(&db,&actor,AgentRole::DeepInvestigator,AgentLane::ReadOnlyAnalysis,"original-second",&json!({}),2,
        &["evidence.read".into(),"mailbox.write".into()],8000,1).unwrap();
    scheduler::start_child_or_release(&db,&actor,&second).unwrap();
    assert!(multi_agent_child_round_transport(&context,&actor,&second,"readonly",json!({})).is_err());
    let second_fee=specialist_readonly_closure_frozen(&db,&second);
    finish_coordinator_run(&db,&actor,&AgentTargetOutcome::incomplete("only original second requires reconciliation")).unwrap();
    let after=vec![Rows::read(&db,"SELECT rowid,* FROM agent_assignments WHERE id=?1",[&first.assignment_id]).unwrap(),
        Rows::read(&db,"SELECT rowid,* FROM agent_runs WHERE id=?1",[&first.run_id]).unwrap(),
        Rows::read(&db,"SELECT rowid,* FROM agent_assignment_attempts WHERE assignment_id=?1",[&first.assignment_id]).unwrap()];
    assert!(after==previous);assert!(specialist_readonly_closure_frozen(&db,&first)==first_fee);
    assert!(specialist_readonly_closure_frozen(&db,&second)==second_fee);
    let state:String=db.query_row("SELECT state FROM agent_assignments WHERE id=?1",[&second.assignment_id],|r|r.get(0)).unwrap();assert_eq!(state,"paused");
    assert_eq!(seen.lock().unwrap().len(),2);
    drop(db);drop(context);fs::remove_dir_all(root).unwrap();
}
