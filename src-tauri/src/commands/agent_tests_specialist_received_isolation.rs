// An incurred provider bill does not authorize receipt-trigger business writes.
#[test]
fn specialist_received_actual_publication_denies_collateral_business_and_keeps_original_bill() {
    for (trigger, fee_saved) in [
        ("AFTER UPDATE OF state ON agent_specialist_calls WHEN NEW.state='received' BEGIN UPDATE projects SET status='archived'; END;", true),
        ("AFTER INSERT ON agent_events WHEN NEW.event_type='model_round_completed' BEGIN UPDATE projects SET status='archived'; END;", true),
        ("AFTER INSERT ON agent_snapshots BEGIN UPDATE projects SET status='archived'; END;", true),
        ("AFTER INSERT ON agent_budget_entries WHEN NEW.kind='consume' BEGIN UPDATE projects SET status='archived'; END;", false),
    ] {
        let (root,mut context,lease,child)=specialist_financial_fixture();let db=db::open(&context.db_path).unwrap();
        db.execute_batch(&format!("CREATE TRIGGER received_collateral {trigger}")).unwrap();
        let captured=std::sync::Arc::new(std::sync::Mutex::new(None));let capture=captured.clone();let path=context.db_path.clone();
        let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(move |_| {*capture.lock().unwrap()=Some(web_mode_test_rows(&db::open(&path).unwrap()));(200,"application/json",proposal_model_response("original provider response"))}));
        context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
        let result=multi_agent_child_round_transport(&context,&lease,&child,"readonly",json!({}));
        let before=captured.lock().unwrap().clone().unwrap();let after=web_mode_test_rows(&db);
        assert_eq!(after.iter().find(|(n,_)|n=="projects").unwrap().1,before.iter().find(|(n,_)|n=="projects").unwrap().1,"received receipt mutated business records");
        let error=result.unwrap_err();assert_eq!(error.contains("specialist_cost_after_publication_failure:"),!fee_saved,"{error}");
        specialist_financial_unchanged(&db,&before,fee_saved);
        let cost=crate::agent_runtime::multi_agent::budget::balance(&db,&lease.root_run_id,Some(&child.assignment_id),"model_requests").unwrap();
        assert_eq!((cost.consumed,cost.indeterminate),(i64::from(fee_saved),0));
        let before_replay=web_mode_test_rows(&db);assert!(multi_agent_child_round_transport(&context,&lease,&child,"readonly",json!({})).unwrap_err().contains("outcome_unknown_requires_reconciliation"));
        web_mode_assert_rows(&db,&before_replay);assert_eq!(seen.lock().unwrap().len(),1);
        drop(db);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn specialist_received_actual_missing_or_replaced_delivery_emitter_preserves_original_bill() {
    for replacement in ["", "CREATE TRIGGER agent_collaboration_directive_delivery AFTER UPDATE OF payload_json ON agent_user_directives BEGIN SELECT 1; END;"] {
        let (root,mut context,lease,child)=specialist_financial_fixture();let db=db::open(&context.db_path).unwrap();
        db.execute_batch(&format!("DROP TRIGGER agent_collaboration_directive_delivery;{replacement}")).unwrap();
        let captured=std::sync::Arc::new(std::sync::Mutex::new(None));let capture=captured.clone();let path=context.db_path.clone();
        let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(move |_| {*capture.lock().unwrap()=Some(web_mode_test_rows(&db::open(&path).unwrap()));(200,"application/json",proposal_model_response("original provider response"))}));
        context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
        let error=multi_agent_child_round_transport(&context,&lease,&child,"readonly",json!({})).unwrap_err();
        assert!(error.contains("specialist_receipt_emitter_contract_conflict") && !error.contains("specialist_cost_after_publication_failure:"),"{error}");
        specialist_financial_unchanged(&db,captured.lock().unwrap().as_ref().unwrap(),true);
        let cost=crate::agent_runtime::multi_agent::budget::balance(&db,&lease.root_run_id,Some(&child.assignment_id),"model_requests").unwrap();assert_eq!((cost.consumed,cost.indeterminate),(1,0));
        let before=web_mode_test_rows(&db);assert!(multi_agent_child_round_transport(&context,&lease,&child,"readonly",json!({})).is_err());web_mode_assert_rows(&db,&before);assert_eq!(seen.lock().unwrap().len(),1);
        drop(db);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn specialist_received_local_normal_preserves_caller_hook_and_late_fee_fault_is_atomic() {
    use crate::agent_runtime::{multi_agent::specialist, store::UsageDelta};
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    for late in [false, true] {
        let (root, context, lease, child) = specialist_financial_fixture();
        let db = db::open(&context.db_path).unwrap();
        let specialist::Start::Dispatch(call) =
            specialist::start(&db, &lease, &child, &json!({"messages":[]})).unwrap()
        else {
            panic!()
        };
        let count = Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        db.authorizer(Some(move |c: AuthContext<'_>| match c.action {
            AuthAction::Insert {
                table_name: "projects",
            } => {
                seen.fetch_add(1, Ordering::SeqCst);
                Authorization::Deny
            }
            _ => Authorization::Allow,
        }))
        .unwrap();
        if late {
            db.execute("UPDATE agent_assignment_attempts SET expires_at=datetime('now','localtime','-1 second') WHERE assignment_id=?1",[&child.assignment_id]).unwrap();
            db.execute_batch("CREATE TRIGGER received_late_collateral AFTER INSERT ON agent_model_cost_facts BEGIN UPDATE projects SET status='archived'; END;").unwrap();
        }
        let usage = UsageDelta {
            input_tokens: 10,
            output_tokens: 10,
            total_tokens: 20,
            model_requests: 1,
            ..Default::default()
        };
        let before = web_mode_test_rows(&db);
        let result = specialist::record_received(&db, &call, "original response", false, &usage);
        if late {
            assert!(result.is_err());
            web_mode_assert_rows(&db, &before);
        } else {
            result.unwrap();
        }
        let before = web_mode_test_rows(&db);
        assert!(db
            .execute(
                "INSERT INTO projects(id,name) VALUES(9004,'caller still owns hook')",
                []
            )
            .is_err());
        assert_eq!(count.load(Ordering::SeqCst), 1);
        web_mode_assert_rows(&db, &before);
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}
