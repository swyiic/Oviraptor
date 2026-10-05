#[test]
fn assignment_attempt_pause_is_atomic_and_preserves_original_unknown_cost() {
    use crate::agent_runtime::{contract::AgentRole,multi_agent::{attempts,budget,source,scheduler,specialist}};
    for fault in ["live","IGNORE","ABORT"] {
        let (root,db,_,lease)=source_specialist_fixture();
        let role=AgentRole::RepoMapper;
        let slice=source::task_slice(&db,&lease,role).unwrap();
        let child=scheduler::prepare_readonly_child(&db,&lease,role,"source_results_ready",&slice,8_000).unwrap();
        let specialist::Start::Dispatch(call)=specialist::start(&db,&lease,&child,&source_specialist_request(&db,&lease,role)).unwrap() else {panic!()};
        specialist::record_uncertain(&db,&call,"provider_outcome_unknown").unwrap();
        let original=attempts::current(&db,&lease,&child.assignment_id).unwrap();
        let cost=budget::balance(&db,&lease.root_run_id,Some(&child.assignment_id),"model_requests").unwrap();
        assert_eq!(cost.indeterminate,1);
        if fault!="live" {
            let raise=if fault=="IGNORE" {"IGNORE"} else {"ABORT,'attempt pause failed'"};
            db.execute_batch(&format!("CREATE TRIGGER attempt_pause_fault BEFORE UPDATE ON agent_assignment_attempts BEGIN SELECT RAISE({raise}); END;")).unwrap();
        }
        let before=application_table_snapshot(&db);
        let result=stop_failed_child_preserving_usage(&db,&lease,&child,"provider outcome unknown");
        if fault=="live" {
            result.unwrap();
            let worker=attempts::current(&db,&lease,&child.assignment_id).unwrap();
            assert_eq!(worker.state,"paused");
            assert_eq!((&worker.id,&worker.worker_id,&worker.fencing_token),(&original.id,&original.worker_id,&original.fencing_token));
            assert_eq!(budget::balance(&db,&lease.root_run_id,Some(&child.assignment_id),"model_requests").unwrap(),cost);
            assert!(attempts::require_live_for_run(&db,&child.run_id).is_err());
        } else {
            assert!(result.is_err(),"{fault}");
            assert_application_tables_unchanged(&db,&before);
        }
        drop(db);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn assignment_attempt_completion_rejects_missing_finish_timestamp_and_rolls_back() {
    use crate::agent_runtime::{contract::AgentRole,multi_agent::{source,scheduler,specialist}};
    let (root,db,_,lease)=source_specialist_fixture();
    let role=AgentRole::RepoMapper;
    let slice=source::task_slice(&db,&lease,role).unwrap();
    let child=scheduler::prepare_readonly_child(&db,&lease,role,"source_results_ready",&slice,8_000).unwrap();
    let specialist::Start::Dispatch(call)=specialist::start(&db,&lease,&child,&source_specialist_request(&db,&lease,role)).unwrap() else {panic!()};
    let usage=AgentTokenUsage {input_tokens:10,cached_input_tokens:0,output_tokens:5,total_tokens:15,model_requests:1};
    specialist::record_received(&db,&call,"saved real result",false,&usage).unwrap();
    db.execute_batch("CREATE TRIGGER attempt_finish_timestamp_fault AFTER UPDATE OF state ON agent_assignment_attempts
        WHEN NEW.state='completed' BEGIN UPDATE agent_assignment_attempts SET finished_at='' WHERE id=NEW.id; END;").unwrap();
    let before=application_table_snapshot(&db);
    assert!(complete_readonly_assessment(&db,&lease,&child,&usage,
        &json!({"sourceTask":slice,"summary":"saved real result"})).is_err(),
        "a terminal worker without its closing timestamp must not release the original reservation");
    assert_application_tables_unchanged(&db,&before);
    drop(db);fs::remove_dir_all(root).unwrap();
}
