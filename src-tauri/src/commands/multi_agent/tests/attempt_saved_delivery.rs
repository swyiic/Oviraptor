fn attempt_saved_call(
    db:&rusqlite::Connection,
    lease:&crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child:&crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    input:&JsonValue,response:&str,
)->AgentTokenUsage {
    use crate::agent_runtime::multi_agent::specialist;
    let request=serde_json::json!({"schemaVersion":1,"tools":[],"messages":[
        {"role":"system","content":"saved response fixture"},{"role":"user","content":input.to_string()}]});
    let specialist::Start::Dispatch(call)=specialist::start(db,lease,child,&request).unwrap() else {panic!()};
    let usage=AgentTokenUsage {input_tokens:10,cached_input_tokens:0,output_tokens:10,total_tokens:20,model_requests:1};
    specialist::record_received(db,&call,response,false,&usage).unwrap();
    usage
}

#[test]
fn assignment_attempt_saved_reviewer_delivery_closes_paused_worker_without_resurrecting_failed_worker() {
    use crate::agent_runtime::multi_agent::{attempts,budget};
    for settled in [false,true] {
        let f=review_receipt_fixture();let db=db::open(&f.context.db_path).unwrap();
        let usage=attempt_saved_call(&db,&f.lease,&f.child,&f.candidate,&review_receipt_response());
        if settled {settle_child_usage(&db,&f.lease,&f.child,&usage).unwrap();}
        finish_failed_review(&db,&f.lease,&f.child,"review-receipt","delivery interrupted").unwrap();
        let original=attempts::current(&db,&f.lease,&f.child.assignment_id).unwrap();
        assert_eq!(original.state,if settled {"failed"} else {"paused"});
        recover_received_review(&db,&f.lease,"receipt",1,&f.candidate.to_string(),&f.context.target_dir).unwrap();
        assert_review_receipt_completed(&db);
        let closed=attempts::current(&db,&f.lease,&f.child.assignment_id).unwrap();
        assert_eq!(closed.state,if settled {"failed"} else {"completed"});
        if settled {assert_eq!(closed,original,"local publication cannot rewrite a failed worker as successfully executed");}
        assert_eq!(budget::balance(&db,&f.lease.root_run_id,Some(&f.child.assignment_id),"concurrency_batches").unwrap().reserved,0);
        assert!(attempts::require_live_for_run(&db,&f.child.run_id).is_err());
        drop(db);std::fs::remove_dir_all(f.root).unwrap();
    }
}

#[test]
fn assignment_attempt_saved_investigator_delivery_releases_only_its_slot() {
    use crate::agent_runtime::multi_agent::{attempts,budget};
    for settled in [false,true] {
        let f=gap_receipt_fixture();let db=db::open(&f.context.db_path).unwrap();
        let usage=attempt_saved_call(&db,&f.session.lease,&f.child,&f.input,&gap_receipt_response());
        if settled {settle_child_usage(&db,&f.session.lease,&f.child,&usage).unwrap();}
        stop_failed_child_preserving_usage(&db,&f.session.lease,&f.child,"delivery interrupted").unwrap();
        let original=attempts::current(&db,&f.session.lease,&f.child.assignment_id).unwrap();
        let sibling=budget::balance(&db,&f.session.lease.root_run_id,Some(&f.session.executor.assignment_id),"concurrency_batches").unwrap();
        recover_gap_fixture(&f,&db).unwrap();assert_gap_receipt_completed(&db);
        let closed=attempts::current(&db,&f.session.lease,&f.child.assignment_id).unwrap();
        assert_eq!(closed.state,if settled {"failed"} else {"completed"});
        if settled {assert_eq!(closed,original);}
        assert_eq!(budget::balance(&db,&f.session.lease.root_run_id,Some(&f.child.assignment_id),"concurrency_batches").unwrap().reserved,0);
        assert_eq!(budget::balance(&db,&f.session.lease.root_run_id,Some(&f.session.executor.assignment_id),"concurrency_batches").unwrap(),sibling);
        drop(db);std::fs::remove_dir_all(f.root).unwrap();
    }
}
