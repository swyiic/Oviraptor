// Unknown-call persistence cannot acquire business writes through SQLite triggers.
#[test]
fn specialist_phase_actual_unknown_receipt_denies_collateral_business_writes() {
    for (trigger, fee_saved) in [
        ("AFTER UPDATE OF state ON agent_specialist_calls WHEN NEW.state='uncertain' BEGIN UPDATE projects SET status='archived'; END;", true),
        ("AFTER INSERT ON agent_budget_entries WHEN NEW.kind='forfeit' BEGIN UPDATE projects SET status='archived'; END;", false),
    ] {
        let (root, mut context, lease, child) = specialist_financial_fixture();
        let db = db::open(&context.db_path).unwrap();
        db.execute_batch(&format!("CREATE TRIGGER phase_collateral {trigger}")).unwrap();
        let captured = std::sync::Arc::new(std::sync::Mutex::new(None));
        let capture = captured.clone();
        let path = context.db_path.clone();
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(move |_| {
            *capture.lock().unwrap() = Some(web_mode_test_rows(&db::open(&path).unwrap()));
            (503, "application/json", json!({"error":"original provider failure"}).to_string())
        }));
        context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
        let error = multi_agent_child_round_transport(&context, &lease, &child, "readonly", json!({})).unwrap_err();
        let before = captured.lock().unwrap().clone().unwrap();
        let projects_before = &before.iter().find(|(name, _)| name == "projects").unwrap().1;
        let all_after = web_mode_test_rows(&db);
        let projects_after = &all_after.iter().find(|(name, _)| name == "projects").unwrap().1;
        assert_eq!(projects_after, projects_before, "unknown model receipt mutated business records");
        assert!(error.contains("503") && error.contains("specialist_dispatch_receipt:"), "{error}");
        assert_eq!(error.contains("specialist_cost_after_uncertain_failure:"), !fee_saved, "{error}");
        specialist_financial_unchanged(&db, &before, fee_saved);
        let cost = crate::agent_runtime::multi_agent::budget::balance(&db, &lease.root_run_id, Some(&child.assignment_id), "model_requests").unwrap();
        assert_eq!((cost.consumed, cost.indeterminate), (0, i64::from(fee_saved)));
        let before_replay = web_mode_test_rows(&db);
        assert!(multi_agent_child_round_transport(&context, &lease, &child, "readonly", json!({})).unwrap_err().contains("outcome_unknown_requires_reconciliation"));
        web_mode_assert_rows(&db, &before_replay);
        assert_eq!(seen.lock().unwrap().len(), 1);
        drop(db);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn specialist_phase_local_unknown_preserves_caller_hook_and_open_transaction() {
    use crate::agent_runtime::multi_agent::specialist;
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let (root, context, lease, child) = specialist_financial_fixture();
    let db = db::open(&context.db_path).unwrap();
    let specialist::Start::Dispatch(call) =
        specialist::start(&db, &lease, &child, &json!({"messages":[]})).unwrap()
    else {
        panic!()
    };
    let tx = db.unchecked_transaction().unwrap();
    let before = web_mode_test_rows(&tx);
    let error = specialist::record_uncertain(&tx, &call, "original unknown outcome").unwrap_err();
    assert!(
        error.contains("specialist_phase_private_transaction_required"),
        "{error}"
    );
    assert!(!db.is_autocommit());
    web_mode_assert_rows(&tx, &before);
    tx.rollback().unwrap();
    web_mode_assert_rows(&db, &before);
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
    specialist::record_uncertain(&db, &call, "original unknown outcome").unwrap();
    let before = web_mode_test_rows(&db);
    assert!(db
        .execute(
            "INSERT INTO projects(id,name) VALUES(9003,'retained caller hook')",
            []
        )
        .is_err());
    assert_eq!(count.load(Ordering::SeqCst), 1);
    web_mode_assert_rows(&db, &before);
    assert!(specialist::record_uncertain(&db, &call, "different unknown outcome").is_err());
    web_mode_assert_rows(&db, &before);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
