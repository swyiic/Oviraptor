// Sent SDK outcome remains unknown even when its business phase cannot save.
#[test]
fn specialist_unknown_cost_actual_provider_error_keeps_original_debt_after_phase_fault() {
    for action in ["RAISE(IGNORE)", "RAISE(ABORT,'unknown_phase_fault')"] {
        let (root, mut context, lease, child) = specialist_financial_fixture();
        let db = db::open(&context.db_path).unwrap();
        db.execute_batch(&format!("CREATE TRIGGER unknown_phase_fault BEFORE UPDATE OF state ON agent_specialist_calls WHEN NEW.state='uncertain' BEGIN SELECT {action}; END;")).unwrap();
        let captured = std::sync::Arc::new(std::sync::Mutex::new(None));
        let capture = captured.clone();
        let path = context.db_path.clone();
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(move |_| {
            *capture.lock().unwrap() = Some(web_mode_test_rows(&db::open(&path).unwrap()));
            (
                503,
                "application/json",
                json!({"error":"original provider failure"}).to_string(),
            )
        }));
        context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
        let error =
            multi_agent_child_round_transport(&context, &lease, &child, "readonly", json!({}))
                .unwrap_err();
        assert!(
            error.contains("503") && error.contains("specialist_dispatch_receipt:"),
            "{error}"
        );
        assert!(
            !error.contains("specialist_cost_after_uncertain_failure:"),
            "{error}"
        );
        let cost = crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &lease.root_run_id,
            Some(&child.assignment_id),
            "model_requests",
        )
        .unwrap();
        assert_eq!(
            (cost.consumed, cost.indeterminate),
            (0, 1),
            "sent call lost its unknown bill"
        );
        assert!(
            crate::agent_runtime::multi_agent::budget::admission::require_determinate(
                &db,
                &lease.root_run_id
            )
            .is_err()
        );
        let fact: (String, String, String, String) = db
            .query_row(
                "SELECT root_run_id,assignment_id,child_run_id,phase FROM agent_model_cost_facts",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            fact,
            (
                lease.root_run_id.clone(),
                child.assignment_id.clone(),
                child.run_id.clone(),
                "uncertain".into()
            )
        );
        specialist_financial_unchanged(&db, captured.lock().unwrap().as_ref().unwrap(), true);
        let before = web_mode_test_rows(&db);
        assert!(
            multi_agent_child_round_transport(&context, &lease, &child, "readonly", json!({}))
                .unwrap_err()
                .contains("outcome_unknown_requires_reconciliation")
        );
        web_mode_assert_rows(&db, &before);
        assert_eq!(seen.lock().unwrap().len(), 1);
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}
#[test]
fn specialist_unknown_cost_actual_fee_trigger_cannot_write_business_or_reconcile() {
    for fault in ["AFTER INSERT ON agent_model_cost_facts BEGIN UPDATE projects SET status='archived'; END;","AFTER INSERT ON agent_budget_entries WHEN NEW.kind='forfeit' BEGIN UPDATE projects SET status='archived'; END;"] {
        let (root,mut context,lease,child)=specialist_financial_fixture();let db=db::open(&context.db_path).unwrap();
        db.execute_batch(&format!("CREATE TRIGGER unknown_phase_fault BEFORE UPDATE OF state ON agent_specialist_calls WHEN NEW.state='uncertain' BEGIN SELECT RAISE(IGNORE); END; CREATE TRIGGER unknown_cost_fault {fault}")).unwrap();
        let captured=std::sync::Arc::new(std::sync::Mutex::new(None));let capture=captured.clone();let path=context.db_path.clone();
        let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(move |_| {*capture.lock().unwrap()=Some(web_mode_test_rows(&db::open(&path).unwrap()));(503,"application/json",json!({"error":"original provider failure"}).to_string())}));
        context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
        let error=multi_agent_child_round_transport(&context,&lease,&child,"readonly",json!({})).unwrap_err();
        assert!(error.contains("503") && error.contains("specialist_cost_after_uncertain_failure:"),"{error}");
        specialist_financial_unchanged(&db,captured.lock().unwrap().as_ref().unwrap(),false);
        let before=web_mode_test_rows(&db);assert!(multi_agent_child_round_transport(&context,&lease,&child,"readonly",json!({})).unwrap_err().contains("outcome_unknown_requires_reconciliation"));
        web_mode_assert_rows(&db,&before);assert_eq!(seen.lock().unwrap().len(),1);
        drop(db);fs::remove_dir_all(root).unwrap();
    }
}
