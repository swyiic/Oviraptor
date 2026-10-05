fn child_completion_fixture() -> (
    PathBuf, rusqlite::Connection,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
) {
    use crate::agent_runtime::{contract::{AgentRole,AgentLane},multi_agent::scheduler};
    let (root,path,_,lease) = multi_agent_test_root("child-completion",1000,10);
    let connection = db::open(&path).unwrap();
    let child = scheduler::schedule_child(&connection,&lease,AgentRole::SpaApiMapper,
        AgentLane::ReadOnlyAnalysis,"completion",&serde_json::json!({}),1,
        &["evidence.read".into(),"mailbox.write".into()],80,2).unwrap();
    scheduler::mark_child_running(&connection,&lease,&child).unwrap();
    (root,connection,lease,child)
}

#[test]
fn child_completion_silent_and_explicit_write_failures_roll_back_every_resource() {
    use crate::agent_runtime::multi_agent::scheduler;
    for success in [true,false] {
        for action in ["IGNORE","ABORT,'fault'"] {
            for target in [
                "BEFORE UPDATE OF reserved_tokens ON agent_budget_ledger",
                "BEFORE UPDATE OF state ON agent_assignments",
                "BEFORE UPDATE OF status ON agent_runs",
                "BEFORE UPDATE OF revoked_at ON agent_capability_leases",
                "BEFORE DELETE ON agent_lane_leases",
            ] {
                let (root,connection,lease,child) = child_completion_fixture();
                let before = receipt_database_snapshot(&connection);
                connection.execute_batch(&format!("CREATE TRIGGER completion_fault {target} BEGIN SELECT RAISE({action}); END;")).unwrap();
                assert!(scheduler::finish_child(&connection,&lease,&child,success,"done").is_err(),"{success}/{target}/{action}");
                assert_eq!(receipt_database_snapshot(&connection),before,"{success}/{target}/{action}");
                drop(connection);
                fs::remove_dir_all(root).unwrap();
            }
        }
    }
}

#[test]
fn child_completion_checks_final_state_after_all_database_triggers() {
    use crate::agent_runtime::multi_agent::scheduler;
    for mutation in [
        "UPDATE agent_runs SET status='running' WHERE assignment_id=OLD.assignment_id;",
        "UPDATE agent_runs SET terminal_state='failed' WHERE assignment_id=OLD.assignment_id;",
        "UPDATE agent_runs SET terminal_reason='different' WHERE assignment_id=OLD.assignment_id;",
        "UPDATE agent_assignments SET reserved_tokens=1 WHERE id=OLD.assignment_id;",
        "UPDATE agent_assignments SET state='running' WHERE id=OLD.assignment_id;",
        "UPDATE agent_capability_leases SET revoked_at='' WHERE assignment_id=OLD.assignment_id;",
        "UPDATE agent_budget_ledger SET reserved_tokens=reserved_tokens+1;",
        "UPDATE agent_budget_ledger SET spent_tokens=spent_tokens+1;",
        "UPDATE agent_budget_ledger SET total_tokens=total_tokens+1;",
    ] {
        let (root,connection,lease,child) = child_completion_fixture();
        let before = receipt_database_snapshot(&connection);
        connection.execute_batch(&format!("CREATE TRIGGER completion_fault AFTER DELETE ON agent_lane_leases BEGIN {mutation} END;")).unwrap();
        assert!(scheduler::finish_child(&connection,&lease,&child,true,"done").is_err(),"{mutation}");
        assert_eq!(receipt_database_snapshot(&connection),before,"{mutation}");
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn child_completion_rejects_mismatched_role_run_target_and_fencing() {
    use crate::agent_runtime::{contract::AgentRole,multi_agent::scheduler};
    for mutation in [
        "handle", "UPDATE agent_runs SET target_url='https://other.example.test' WHERE role='spa_api_mapper'",
        "UPDATE agent_runs SET role='identity_session' WHERE role='spa_api_mapper'",
        "UPDATE agent_assignments SET target_key='https://other.example.test'",
        "UPDATE agent_assignments SET fencing_token='replacement'",
        "UPDATE agent_runs SET attempt_number=2 WHERE role='spa_api_mapper'",
        "UPDATE agent_runs SET assignment_id='' WHERE role='spa_api_mapper'",
    ] {
        let (root,connection,lease,mut child) = child_completion_fixture();
        if mutation=="handle" { child.role=AgentRole::IdentitySession; }
        else { connection.execute_batch(mutation).unwrap(); }
        let before = receipt_database_snapshot(&connection);
        assert!(scheduler::finish_child(&connection,&lease,&child,true,"done").is_err(),"{mutation}");
        assert_eq!(receipt_database_snapshot(&connection),before);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn child_completion_releases_only_owned_budget_lane_and_capabilities_once() {
    use crate::agent_runtime::{contract::{AgentRole,AgentLane},multi_agent::scheduler};
    let (root,connection,lease,child) = child_completion_fixture();
    let sibling = scheduler::schedule_child(&connection,&lease,AgentRole::EvidenceReviewer,
        AgentLane::Review,"other",&serde_json::json!({}),1,&["evidence.read".into()],60,1).unwrap();
    scheduler::finish_child(&connection,&lease,&child,true,"done").unwrap();
    let row:(i64,i64,i64,i64) = connection.query_row(
        "SELECT reserved_tokens,reserved_requests,spent_tokens,spent_requests FROM agent_budget_ledger",[],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(row,(60,1,0,0));
    assert_eq!(connection.query_row("SELECT assignment_id FROM agent_lane_leases",[],|r|r.get::<_,String>(0)).unwrap(),sibling.assignment_id);
    assert_eq!(connection.query_row("SELECT child_run_id FROM agent_capability_leases WHERE revoked_at=''",[],|r|r.get::<_,String>(0)).unwrap(),sibling.run_id);
    let before = receipt_database_snapshot(&connection);
    assert!(scheduler::finish_child(&connection,&lease,&child,true,"again").is_err());
    assert_eq!(receipt_database_snapshot(&connection),before);
    // An unstarted child can still be failed and release only its own reservation.
    scheduler::finish_child(&connection,&lease,&sibling,false,"not started").unwrap();
    assert_eq!(connection.query_row("SELECT COUNT(*) FROM agent_lane_leases",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn child_completion_executor_publishes_one_acknowledged_result_on_success() {
    let _real = RealSpecialistTransport::enter();
    let mut f = client_root_fixture("executor-delivery-positive", false);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
    f.finish().unwrap();
    let connection = db::open(&f.h.db_path).unwrap();
    let executor = &f.session.as_ref().unwrap().executor;
    let delivered: i64=connection.query_row(
        "SELECT COUNT(*) FROM agent_messages m JOIN agent_assignments a ON a.id=m.assignment_id \
         JOIN agent_runs r ON r.id=a.child_run_id WHERE m.kind='execution_result' \
         AND m.assignment_id=?1 AND m.delivered_at<>'' AND m.acknowledged_at<>'' AND m.delivery_attempts=1 \
         AND a.state='completed' AND r.status='terminal' AND r.terminal_state='completed'",
        [&executor.assignment_id],|r|r.get(0)).unwrap();
    assert_eq!(delivered,1);
    assert_eq!(connection.query_row("SELECT COUNT(*) FROM agent_lane_leases WHERE assignment_id=?1",
        [&executor.assignment_id],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(connection.query_row("SELECT COUNT(*) FROM agent_capability_leases WHERE child_run_id=?1 AND revoked_at=''",
        [&executor.run_id],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert!(f.h.model_seen.lock().unwrap().len() >= 5);
    let calls = f.h.model_seen.lock().unwrap().len();
    let invoices = fresh_multi_model_financial_rows(&connection);
    let paid_target = client_hook_target_costs(&connection);
    f.finish().unwrap();
    assert_eq!(f.h.model_seen.lock().unwrap().len(), calls);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
    assert_eq!(fresh_multi_model_financial_rows(&connection), invoices);
    assert_eq!(client_hook_target_costs(&connection), paid_target);
    assert_eq!(connection.query_row("SELECT COUNT(*) FROM agent_messages WHERE kind='execution_result'", [], |r|r.get::<_,i64>(0)).unwrap(), 1);
}

#[test]
fn child_completion_executor_result_is_not_published_before_resource_commit() {
    let _real = RealSpecialistTransport::enter();
    for action in ["IGNORE","ABORT,'fault'"] {
        let mut f = client_root_fixture("executor-delivery", false);
        let connection=db::open(&f.h.db_path).unwrap();
        let calls = f.h.model_seen.lock().unwrap().len();
        assert!(calls >= 4);
        assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
        let target_costs = client_hook_target_costs(&connection);
        let paid = |db: &rusqlite::Connection| db.query_row("SELECT COALESCE(SUM(amount),0) FROM agent_budget_entries WHERE kind='consume' AND dimension IN ('model_input_tokens','model_output_tokens','model_requests')", [], |r|r.get::<_,i64>(0)).unwrap();
        let original_paid = paid(&connection);
        assert!(original_paid > 0);
        connection.execute_batch(&format!("CREATE TRIGGER completion_fault BEFORE UPDATE OF status ON agent_runs
            WHEN OLD.role='web_executor' AND NEW.status='terminal' BEGIN SELECT RAISE({action}); END;")).unwrap();
        assert!(f.finish().is_err());
        assert_eq!(connection.query_row("SELECT COUNT(*) FROM agent_messages WHERE kind='execution_result'",[],|r|r.get::<_,i64>(0)).unwrap(),0,
            "an uncommitted executor completion must not appear in the team mailbox");
        assert_eq!(f.h.model_seen.lock().unwrap().len(), calls);
        assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
        assert_eq!(paid(&connection), original_paid, "no refund or second charge of the original paid work");
        assert_eq!(client_hook_target_costs(&connection), target_costs);
    }
}

fn target_delivery_fixture(role: crate::agent_runtime::contract::AgentRole) -> (
    PathBuf, rusqlite::Connection,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
) {
    use crate::agent_runtime::{
        contract::{AgentBackendKind,AgentLane,AgentRole,AgentRunStatus,MultiAgentPolicy},
        multi_agent::{lease as leases,scheduler},store::{self,AgentRunRow},
    };
    let (root,connection,lease)=if role==AgentRole::Authorization {
        let (root,path,app_data_dir,control)=authorization_fixture();
        let mut connection=db::open(&path).unwrap();
        let mut context=test_context(&path,&control.target_url,
            vec![AgentIdentity::scoped("session-a"),AgentIdentity::scoped("session-b")]);
        context.attempt_number=2;
        context.execution_plan=context.execution_plan.with_attempt(2);
        save_authorization_control_in(&mut connection,&app_data_dir,&control).unwrap();
        connection.execute("UPDATE sentinel_scans SET status='scanning',attempt_count=2 WHERE id=?1",[&control.scan_id]).unwrap();
        let mut run=AgentRunRow::new("delivery-root",&control.scan_id,2,&control.target_url,
            AgentBackendKind::Native,AgentRole::Coordinator,context.execution_plan.hash(),"evidence").with_budget(100,1000,2,10);
        run.status=AgentRunStatus::Running;
        run.root_run_id=run.id.clone();
        run.orchestration_policy=MultiAgentPolicy::Multi;
        run.lane=Some(AgentLane::ReadOnlyAnalysis);
        store::create_run(&connection,&run).unwrap();
        freeze_authorization_test_plan(&context);
        let lease=leases::acquire_coordinator_lease(&connection,&control.scan_id,2,&control.target_url,&run.id,600).unwrap();
        (root,connection,lease)
    } else {
        let (root,path,_,lease)=multi_agent_test_root("target-delivery",1000,10);
        (root,db::open(&path).unwrap(),lease)
    };
    let (capability,tokens,requests,spent)=if role==AgentRole::Authorization {
        ("authorization_probe",0,0,0)
    } else { ("replay_http",80,2,17) };
    let child=scheduler::schedule_child(&connection,&lease,role,AgentLane::TargetTouching,
        "delivery",&serde_json::json!({}),3,&[capability.into()],tokens,requests).unwrap();
    scheduler::mark_child_running(&connection,&lease,&child).unwrap();
    if role==AgentRole::Authorization {
        connection.execute("INSERT INTO agent_authorization_probe_claims(scan_id,attempt_number,target_url,contract_key,side,child_run_id,assignment_id) \
            SELECT scan_id,attempt_number,target_url,contract_key,'owner',?1,?2 FROM agent_authorization_controls \
            WHERE scan_id=?3 AND attempt_number=?4 AND target_url=?5",
            params![child.run_id,child.assignment_id,lease.scan_id,lease.attempt_number,lease.target_key]).unwrap();
    }
    settle_child_usage(&connection,&lease,&child,&AgentTokenUsage {
        // This fixture proves delivery rollback of a known bill. Total-only
        // usage intentionally stays indeterminate in the dedicated debt tests.
        input_tokens:spent,total_tokens:spent,model_requests:if role==AgentRole::Authorization {0} else {1},..Default::default()
    }).unwrap();
    (root,connection,lease,child)
}

#[test]
fn child_completion_target_delivery_rolls_back_faults_without_refunding_spent_usage() {
    use crate::agent_runtime::contract::AgentRole;
    for (role,kind) in [(AgentRole::WebExecutor,"execution_result"),(AgentRole::Authorization,"authorization_result")] {
        for fault in [
            "CREATE TRIGGER fault BEFORE INSERT ON agent_messages BEGIN SELECT RAISE(IGNORE); END;",
            "CREATE TRIGGER fault BEFORE UPDATE OF acknowledged_at ON agent_messages BEGIN SELECT RAISE(IGNORE); END;",
            "CREATE TRIGGER fault BEFORE UPDATE OF acknowledged_at ON agent_messages BEGIN SELECT RAISE(ABORT,'fault'); END;",
            "CREATE TRIGGER fault BEFORE UPDATE OF status ON agent_runs BEGIN SELECT RAISE(IGNORE); END;",
            "CREATE TRIGGER fault BEFORE UPDATE OF revoked_at ON agent_capability_leases BEGIN SELECT RAISE(IGNORE); END;",
            "CREATE TRIGGER fault BEFORE DELETE ON agent_lane_leases BEGIN SELECT RAISE(IGNORE); END;",
            "CREATE TRIGGER fault BEFORE INSERT ON agent_collaboration_events WHEN NEW.event_type='mailbox_message' BEGIN SELECT RAISE(IGNORE); END;",
            "CREATE TRIGGER fault BEFORE INSERT ON agent_collaboration_events WHEN NEW.event_type='assignment' BEGIN SELECT RAISE(IGNORE); END;",
            "CREATE TRIGGER fault BEFORE INSERT ON agent_collaboration_events WHEN NEW.event_type='agent_run' BEGIN SELECT RAISE(IGNORE); END;",
            "CREATE TRIGGER fault AFTER DELETE ON agent_lane_leases BEGIN UPDATE agent_messages SET payload_json='{}'; END;",
            "CREATE TRIGGER fault AFTER DELETE ON agent_lane_leases BEGIN UPDATE agent_messages SET acknowledged_at=''; END;",
            "CREATE TRIGGER fault AFTER DELETE ON agent_lane_leases BEGIN UPDATE agent_messages SET to_run_id='wrong-recipient'; END;",
            "CREATE TRIGGER fault AFTER DELETE ON agent_lane_leases BEGIN UPDATE agent_messages SET delivery_attempts=2; END;",
            "CREATE TRIGGER fault AFTER DELETE ON agent_lane_leases BEGIN UPDATE agent_assignments SET evidence_revision=evidence_revision+1; END;",
            "CREATE TRIGGER fault AFTER DELETE ON agent_lane_leases BEGIN UPDATE agent_assignments SET budget_settled_at=''; END;",
            "CREATE TRIGGER fault AFTER DELETE ON agent_lane_leases BEGIN UPDATE agent_runs SET status='paused' WHERE role='coordinator'; END;",
            "CREATE TABLE completion_parent(id INTEGER PRIMARY KEY); CREATE TABLE completion_deferred(parent_id INTEGER REFERENCES completion_parent(id) DEFERRABLE INITIALLY DEFERRED); CREATE TRIGGER fault AFTER DELETE ON agent_lane_leases BEGIN INSERT INTO completion_deferred VALUES(7); END;",
        ] {
            let (root,connection,lease,child)=target_delivery_fixture(role);
            connection.execute_batch(fault).unwrap();
            let before=receipt_database_snapshot(&connection);
            let result=complete_target_child_delivery(&connection,&lease,&child,kind,"result",
                &serde_json::json!({"summary":"known result"}),true,"known result");
            assert!(result.is_err(),"{role:?}/{fault}");
            assert_eq!(receipt_database_snapshot(&connection),before,"{role:?}/{fault}");
            assert_eq!(connection.query_row("SELECT spent_tokens,spent_requests FROM agent_budget_ledger",[],
                |r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?))).unwrap(),
                if role==AgentRole::Authorization {(0,0)} else {(17,1)});
            if role==AgentRole::Authorization {
                let target_usage = crate::agent_runtime::target_requests::authorization_usage(
                    &connection,&lease.scan_id,&[lease.attempt_number],&lease.target_key).unwrap();
                assert_eq!((target_usage.received,target_usage.unresolved),(0,1),
                    "delivery failure must not refund a claimed target request");
            }
            if fault.contains("completion_deferred") {
                assert!(result.unwrap_err().starts_with("target_child_delivery_commit:"));
                assert_eq!(connection.query_row("SELECT COUNT(*) FROM completion_deferred",[],|r|r.get::<_,i64>(0)).unwrap(),0);
            }
            drop(connection);
            fs::remove_dir_all(root).unwrap();
        }
    }
}

#[test]
fn child_completion_target_delivery_preserves_unrelated_mail_and_cannot_replay() {
    use crate::agent_runtime::{contract::AgentRole,multi_agent::mailbox};
    for (role,kind) in [(AgentRole::WebExecutor,"execution_result"),(AgentRole::Authorization,"authorization_result")] {
        for success in [true,false] {
            let (root,connection,lease,child)=target_delivery_fixture(role);
            let unrelated=mailbox::send(&connection,&lease,&child.run_id,&lease.root_run_id,role.as_str(),
                "coordinator","other_pending","other",&child.assignment_id,3,&serde_json::json!({})).unwrap();
            let payload=serde_json::json!({"summary":"known result"});
            complete_target_child_delivery(&connection,&lease,&child,kind,"result",&payload,success,"known result").unwrap();
            let delivered:(i64,String,String)=connection.query_row(
                "SELECT delivery_attempts,delivered_at,acknowledged_at FROM agent_messages WHERE kind=?1",[kind],
                |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
            assert_eq!(delivered.0,1);
            assert!(!delivered.1.is_empty() && !delivered.2.is_empty());
            assert_eq!(connection.query_row("SELECT delivery_attempts,acknowledged_at FROM agent_messages WHERE id=?1",[unrelated],
                |r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?))).unwrap(),(0,String::new()));
            assert_eq!(connection.query_row("SELECT terminal_state FROM agent_runs WHERE id=?1",[&child.run_id],
                |r|r.get::<_,String>(0)).unwrap(),if success {"completed"} else {"failed"});
            let before=receipt_database_snapshot(&connection);
            assert!(complete_target_child_delivery(&connection,&lease,&child,kind,"result",&payload,success,"known result").is_err());
            assert_eq!(receipt_database_snapshot(&connection),before);
            drop(connection);
            fs::remove_dir_all(root).unwrap();
        }
    }
}

#[test]
fn child_completion_requires_fresh_terminal_timeline_events() {
    use crate::agent_runtime::multi_agent::scheduler;
    for event in ["assignment","agent_run"] {
        let (root,connection,lease,child)=child_completion_fixture();
        connection.execute_batch(&format!("CREATE TRIGGER fault BEFORE INSERT ON agent_collaboration_events WHEN NEW.event_type='{event}' BEGIN SELECT RAISE(IGNORE); END;")).unwrap();
        let before=receipt_database_snapshot(&connection);
        assert!(scheduler::finish_child(&connection,&lease,&child,true,"done").unwrap_err().contains("child_finish_event_postcondition"));
        assert_eq!(receipt_database_snapshot(&connection),before);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}
