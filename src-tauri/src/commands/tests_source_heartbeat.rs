#[test]
fn source_tool_dispatch_heartbeat_keeps_live_calls_without_resurrecting_expired_work() {
    use crate::agent_runtime::{contract::{AgentRole,AgentLane},multi_agent::{source,scheduler,source_rounds}};
    for mutation in ["expired_attempt","ignored_attempt","aborted_attempt","ignored_assignment","live","expired_root","expired_assignment","expired_capability","revoked","no_active_call","ignored_root","ignored_capability","ignored_run","post_write_cancel"] {
        let environment=source_specialist_test_environment(9);
        let (root,connection,_record,lease)=source_tool_true_born_fixture_model(Some(&environment));
        connection.execute("UPDATE agent_runs SET status='running',started_at=datetime('now','localtime') WHERE id=?1",[&lease.root_run_id]).unwrap();
        let slice=source::tool_task_slice(&connection,&lease,AgentRole::RepoMapper,1).unwrap();
        let capabilities=source::tool_capabilities(AgentRole::RepoMapper).unwrap();
        let child=scheduler::schedule_child(&connection,&lease,AgentRole::RepoMapper,AgentLane::ReadOnlyAnalysis,"source_tools_ready",&slice,1,&capabilities,24_000,3).unwrap();
        scheduler::mark_child_running(&connection,&lease,&child).unwrap();
        let database=root.join("oviraptor.sqlite3");let work=root.join("attempt-0001");
        let context=SpecialistTransportContext { supervision: None,db_path:&database,scan_id:&lease.scan_id,attempt_number:1,target_key:&lease.target_key,
            run_id:&lease.root_run_id,environment:&environment,proxy:None,usage_dir:&work,deadline:Some(std::time::Instant::now()+Duration::from_secs(60))};
        let request=json!({"messages":[{"role":"user","content":json!({"sourceTask":slice}).to_string()}],"tools":capabilities.iter().map(|name|json!({"type":"function","function":{"name":name}})).collect::<Vec<_>>()});
        let source_rounds::Start::Dispatch(call)=source_rounds::start_authorized(&connection,&lease,&child,1,&request,8_000,
            |tx|authorize_source_tool_phase(tx,&context,&lease,&child)).unwrap() else {panic!()};
        connection.execute("UPDATE agent_coordinator_leases SET lease_expires_at=datetime('now','+60 seconds','localtime')",[]).unwrap();
        connection.execute("UPDATE agent_assignments SET lease_expires_at=datetime('now','+60 seconds','localtime')",[]).unwrap();
        connection.execute("UPDATE agent_capability_leases SET lease_expires_at=datetime('now','+60 seconds','localtime')",[]).unwrap();
        match mutation {
            "expired_attempt"=>{connection.execute("UPDATE agent_assignment_attempts SET expires_at='2000-01-01'",[]).unwrap();},
            "ignored_attempt"=>connection.execute_batch("CREATE TRIGGER attempt_heartbeat_ignore BEFORE UPDATE ON agent_assignment_attempts BEGIN SELECT RAISE(IGNORE); END;").unwrap(),
            "aborted_attempt"=>connection.execute_batch("CREATE TRIGGER attempt_heartbeat_abort BEFORE UPDATE ON agent_assignment_attempts BEGIN SELECT RAISE(ABORT,'attempt heartbeat failed'); END;").unwrap(),
            "expired_root"=>{connection.execute("UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01'",[]).unwrap();},
            "expired_assignment"=>{connection.execute("UPDATE agent_assignments SET lease_expires_at='2000-01-01'",[]).unwrap();},
            "expired_capability"=>{connection.execute("UPDATE agent_capability_leases SET lease_expires_at='2000-01-01'",[]).unwrap();},
            "revoked"=>{connection.execute("UPDATE agent_capability_leases SET revoked_at='revoked' WHERE capability='repo.inventory'",[]).unwrap();},
            "no_active_call"=>source_rounds::record_uncertain(&connection,&call,"transport_unknown").unwrap(),
            "post_write_cancel"=>connection.execute_batch("CREATE TRIGGER heartbeat_cancel AFTER UPDATE ON agent_assignments BEGIN UPDATE agent_runs SET cancel_requested_at='cancelled'; END;").unwrap(),
            ignored if ignored.starts_with("ignored_")=>{
                let table=match ignored {"ignored_root"=>"agent_coordinator_leases","ignored_assignment"=>"agent_assignments","ignored_capability"=>"agent_capability_leases",_=>"agent_runs"};
                connection.execute_batch(&format!("CREATE TRIGGER heartbeat_ignore BEFORE UPDATE ON {table} BEGIN SELECT RAISE(IGNORE); END;")).unwrap();
            },
            _=>{},
        }
        let before:String=connection.query_row("SELECT lease_expires_at FROM agent_coordinator_leases",[],|r|r.get(0)).unwrap();
        let snapshot=application_table_snapshot(&connection);
        let original=crate::agent_runtime::multi_agent::attempts::current(&connection,&lease,&child.assignment_id).unwrap();
        let tx=rusqlite::Transaction::new_unchecked(&connection,rusqlite::TransactionBehavior::Immediate).unwrap();
        let outcome=heartbeat_source_tool_phase(&tx,&context,&lease,&child);
        if mutation=="live" {
            outcome.unwrap();tx.commit().unwrap();
            let mut expected_attempt=original;
            expected_attempt.expires_at=connection.query_row("SELECT lease_expires_at FROM agent_coordinator_leases",[],|r|r.get(0)).unwrap();
            assert_eq!(crate::agent_runtime::multi_agent::attempts::current(&connection,&lease,&child.assignment_id).unwrap(),expected_attempt);
            assert!(connection.query_row("SELECT a.lease_expires_at=x.expires_at AND r.lease_expires_at=x.expires_at FROM agent_assignment_attempts x JOIN agent_assignments a ON a.id=x.assignment_id JOIN agent_runs r ON r.id=x.child_run_id",[],|r|r.get::<_,bool>(0)).unwrap());
            assert!(connection.query_row("SELECT lease_expires_at>datetime('now','+500 seconds','localtime') AND lease_epoch=?1 AND fencing_token=?2 FROM agent_assignments",params![lease.lease_epoch,lease.fencing_token],|r|r.get::<_,bool>(0)).unwrap());
        } else {
            assert!(outcome.is_err(),"{mutation}");drop(tx);
            assert_application_tables_unchanged(&connection,&snapshot);
            assert_eq!(connection.query_row("SELECT lease_expires_at FROM agent_coordinator_leases",[],|r|r.get::<_,String>(0)).unwrap(),before,"{mutation}");
        }
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_source_model_rounds",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        assert_eq!(connection.query_row("SELECT reserved_tokens FROM agent_assignments",[],|r|r.get::<_,i64>(0)).unwrap(),24_000);
        assert_eq!(connection.query_row("SELECT spent_tokens FROM agent_budget_ledger",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_dispatch_heartbeat_renews_live_calls_without_new_fences_or_resurrecting_revocation() {
    use crate::agent_runtime::{contract::AgentRole,multi_agent::{source,scheduler,specialist}};
    for mutation in ["expired_attempt","ignored_attempt","aborted_attempt","ignored_assignment","live","expired_root","expired_capability","revoked","lost_lane","ignored_write",
        "post_extra_capability","post_lost_lane","post_cancel","ignored_run_write"] {
        let (root,connection,_record,lease)=source_tool_true_born_fixture_model(None);
        let slice=source::task_slice(&connection,&lease,AgentRole::RepoMapper).unwrap();
        let child=scheduler::prepare_readonly_child(&connection,&lease,AgentRole::RepoMapper,"source_results_ready",&slice,8_000).unwrap();
        let request=source_specialist_request(&connection,&lease,AgentRole::RepoMapper);
        assert!(matches!(specialist::start(&connection,&lease,&child,&request).unwrap(),specialist::Start::Dispatch(_)));
        connection.execute("UPDATE agent_coordinator_leases SET lease_expires_at=datetime('now','+60 seconds','localtime') WHERE root_run_id=?1",[&lease.root_run_id]).unwrap();
        connection.execute("UPDATE agent_capability_leases SET lease_expires_at=datetime('now','+60 seconds','localtime') WHERE assignment_id=?1",[&child.assignment_id]).unwrap();
        match mutation {
            "expired_attempt"=>{connection.execute("UPDATE agent_assignment_attempts SET expires_at='2000-01-01'",[]).unwrap();},
            "ignored_attempt"=>connection.execute_batch("CREATE TRIGGER attempt_heartbeat_ignore BEFORE UPDATE ON agent_assignment_attempts BEGIN SELECT RAISE(IGNORE); END;").unwrap(),
            "aborted_attempt"=>connection.execute_batch("CREATE TRIGGER attempt_heartbeat_abort BEFORE UPDATE ON agent_assignment_attempts BEGIN SELECT RAISE(ABORT,'attempt heartbeat failed'); END;").unwrap(),
            "expired_root"=>{connection.execute("UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01'",[]).unwrap();},
            "expired_capability"=>{connection.execute("UPDATE agent_capability_leases SET lease_expires_at='2000-01-01' WHERE capability='mailbox.write'",[]).unwrap();},
            "revoked"=>{connection.execute("UPDATE agent_capability_leases SET revoked_at='revoked' WHERE capability='evidence.read'",[]).unwrap();},
            "lost_lane"=>{connection.execute("DELETE FROM agent_lane_leases",[]).unwrap();},
            "ignored_assignment"=>connection.execute_batch("CREATE TRIGGER assignment_heartbeat_ignore BEFORE UPDATE ON agent_assignments BEGIN SELECT RAISE(IGNORE); END;").unwrap(),
            "ignored_write"=>connection.execute_batch("CREATE TRIGGER heartbeat_fault BEFORE UPDATE ON agent_capability_leases BEGIN SELECT RAISE(IGNORE); END;").unwrap(),
            "post_extra_capability"=>connection.execute_batch("CREATE TRIGGER heartbeat_fault AFTER UPDATE ON agent_runs BEGIN
                INSERT OR IGNORE INTO agent_capability_leases(id,root_run_id,assignment_id,child_run_id,capability,lease_epoch,fencing_token,lease_expires_at)
                SELECT 'injected-capability',root_run_id,assignment_id,child_run_id,'target.http',lease_epoch,fencing_token,lease_expires_at
                FROM agent_capability_leases WHERE capability='evidence.read'; END;").unwrap(),
            "post_lost_lane"=>connection.execute_batch("CREATE TRIGGER heartbeat_fault AFTER UPDATE ON agent_runs BEGIN DELETE FROM agent_lane_leases; END;").unwrap(),
            "post_cancel"=>connection.execute_batch("CREATE TRIGGER heartbeat_fault AFTER UPDATE OF heartbeat_at ON agent_runs BEGIN UPDATE agent_runs SET cancel_requested_at='cancelled' WHERE id=NEW.id; END;").unwrap(),
            "ignored_run_write"=>connection.execute_batch("CREATE TRIGGER heartbeat_fault BEFORE UPDATE ON agent_runs BEGIN SELECT RAISE(IGNORE); END;").unwrap(),
            _=>{},
        }
        let before: String=connection.query_row("SELECT lease_expires_at FROM agent_coordinator_leases",[],|r|r.get(0)).unwrap();
        let snapshot=application_table_snapshot(&connection);
        let original=crate::agent_runtime::multi_agent::attempts::current(&connection,&lease,&child.assignment_id).unwrap();
        let tx=rusqlite::Transaction::new_unchecked(&connection,rusqlite::TransactionBehavior::Immediate).unwrap();
        let outcome=heartbeat_source_specialist(&tx,&lease,&child);
        if mutation=="live" {
            outcome.unwrap();tx.commit().unwrap();
            let mut expected_attempt=original;
            expected_attempt.expires_at=connection.query_row("SELECT lease_expires_at FROM agent_coordinator_leases",[],|r|r.get(0)).unwrap();
            assert_eq!(crate::agent_runtime::multi_agent::attempts::current(&connection,&lease,&child.assignment_id).unwrap(),expected_attempt);
            assert!(connection.query_row("SELECT a.lease_expires_at=x.expires_at AND r.lease_expires_at=x.expires_at FROM agent_assignment_attempts x JOIN agent_assignments a ON a.id=x.assignment_id JOIN agent_runs r ON r.id=x.child_run_id",[],|r|r.get::<_,bool>(0)).unwrap());
            let renewed:bool=connection.query_row("SELECT lease_epoch=?1 AND fencing_token=?2 AND lease_expires_at>datetime('now','+500 seconds','localtime') FROM agent_coordinator_leases",params![lease.lease_epoch,lease.fencing_token],|r|r.get(0)).unwrap();
            assert!(renewed);
        } else {
            assert!(outcome.is_err(),"{mutation}");drop(tx);
            assert_application_tables_unchanged(&connection,&snapshot);
            assert_eq!(connection.query_row("SELECT lease_expires_at FROM agent_coordinator_leases",[],|r|r.get::<_,String>(0)).unwrap(),before,"{mutation}");
            assert_eq!(connection.query_row("SELECT count(*) FROM agent_capability_leases WHERE capability='target.http'",[],|r|r.get::<_,i64>(0)).unwrap(),0,"{mutation}");
        }
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_specialist_calls",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        let reserved:i64=connection.query_row("SELECT reserved_tokens FROM agent_assignments",[],|r|r.get(0)).unwrap();
        assert_eq!(reserved,8_000);
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

