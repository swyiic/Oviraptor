#[test]
fn assignment_attempt_directive_root_closure_preserves_worker_state_and_original_cost() {
    use crate::agent_runtime::multi_agent::{attempts,budget,directive::proposals};
    for dispatched in [false,true] {
        for fault in ["healthy","IGNORE","ABORT"] {
            let (root,context,id, _original_parent)=proposal_fixture("attempt-directive-close","@mapper 请分析已有证据");
            let lease=take_human_directives(&context).unwrap().lease.unwrap();
            let db=db::open(&context.db_path).unwrap();
            let job=proposals::prepare_next(&db,&lease,&context.evidence).unwrap().unwrap();
            if dispatched {
                consume_proposal_mailbox(&db,&lease,&job.child.run_id,&job.request_message_id,"human_assessment_request",&job.input).unwrap();
                proposals::start(&db,&lease,&id).unwrap();
            }
            let original=attempts::current(&db,&lease,&job.child.assignment_id).unwrap();
            if fault!="healthy" {
                let raise=if fault=="IGNORE" {"IGNORE"} else {"ABORT,'attempt closure failed'"};
                db.execute_batch(&format!("CREATE TRIGGER attempt_closure_fault BEFORE UPDATE ON agent_assignment_attempts BEGIN SELECT RAISE({raise}); END;")).unwrap();
            }
            let before=receipt_database_snapshot(&db);
            let outcome=finish_coordinator_run(&db,&lease,&AgentTargetOutcome::Cancelled);
            if fault=="healthy" {
                outcome.unwrap();let worker=attempts::current(&db,&lease,&job.child.assignment_id).unwrap();
                assert_eq!(worker.state,if dispatched {"paused"} else {"cancelled"});
                assert_eq!((&worker.id,&worker.worker_id,&worker.fencing_token),(&original.id,&original.worker_id,&original.fencing_token));
                let b=budget::balance(&db,&lease.root_run_id,Some(&job.child.assignment_id),"model_requests").unwrap();
                assert_eq!((b.reserved,b.consumed,b.indeterminate),if dispatched {(1,0,0)} else {(0,0,0)});
                assert!(attempts::require_live_for_run(&db,&job.child.run_id).is_err());
            } else {
                assert!(outcome.is_err(),"dispatched={dispatched} {fault}");
                assert_eq!(receipt_database_snapshot(&db),before);
            }
            drop(db);std::fs::remove_dir_all(root).unwrap();
        }
    }
}

#[test]
fn assignment_attempt_abandoned_proposal_stays_bound_to_original_worker_epoch() {
    use crate::agent_runtime::multi_agent::{attempts,directive::proposals,lease};
    for replaced in [false,true] {
        let (root,context,id, _original_parent)=proposal_fixture("attempt-directive-abandoned","@mapper 请分析已有证据");
        let original=take_human_directives(&context).unwrap().lease.unwrap();
        let db=db::open(&context.db_path).unwrap();
        let job=proposals::prepare_next(&db,&original,&context.evidence).unwrap().unwrap();
        consume_proposal_mailbox(&db,&original,&job.child.run_id,&job.request_message_id,"human_assessment_request",&job.input).unwrap();
        proposals::start(&db,&original,&id).unwrap();
        let worker=attempts::current(&db,&original,&job.child.assignment_id).unwrap();
        if replaced {db.execute("UPDATE agent_coordinator_leases SET lease_expires_at=datetime('now','-1 second','localtime')",[]).unwrap();}
        let current=lease::acquire_coordinator_lease(&db,&original.scan_id,original.attempt_number,&original.target_key,&original.root_run_id,600).unwrap();
        let owner=claim_native_invocation(&context.db_path,&current.scan_id,current.attempt_number,"target",&current.target_key).unwrap();
        assert_eq!(reconcile_abandoned_human_proposals(&db,&current,&owner).unwrap(),1);
        let paused=attempts::current(&db,&original,&job.child.assignment_id).unwrap();
        assert_eq!(paused.state,"paused");
        assert_eq!((&paused.id,&paused.worker_id,&paused.fencing_token,&paused.coordinator_fencing_token),
            (&worker.id,&worker.worker_id,&worker.fencing_token,&worker.coordinator_fencing_token));
        assert!(attempts::require_live_for_run(&db,&job.child.run_id).is_err());
        drop(owner);drop(db);std::fs::remove_dir_all(root).unwrap();
    }
}
