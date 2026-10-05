// Genuine creator/C/worker; provider cost must survive business publication faults.
fn specialist_financial_fixture() -> (
    PathBuf,
    AgentRunContext,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
) {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };
    let (root, path, run_id, lease) = multi_agent_new_task_root("specialist-financial", 60_000, 20);
    let db = db::open(&path).unwrap();
    let child = scheduler::schedule_child(
        &db,
        &lease,
        AgentRole::SpaApiMapper,
        AgentLane::ReadOnlyAnalysis,
        "original-cost",
        &json!({"fixture":true}),
        1,
        &["evidence.read".into(), "mailbox.write".into()],
        8000,
        1,
    )
    .unwrap();
    scheduler::start_child_or_release(&db, &lease, &child).unwrap();
    let mut context = test_context(&path, &lease.target_key, vec![AgentIdentity::anonymous()]);
    context.scan_id = lease.scan_id.clone();
    context.run = Some(AgentRunLedger {
        db_path: path,
        run_id,
    });
    (root, context, lease, child)
}
fn specialist_financial_unchanged(
    db: &rusqlite::Connection,
    before: &[(String, Vec<Vec<rusqlite::types::Value>>)],
    paid: bool,
) {
    let after = web_mode_test_rows(db);
    for ((name, rows), (old, original)) in after.iter().zip(before) {
        assert_eq!(name, old);
        if name.starts_with("native_sdk_log_")
            || name == "sqlite_sequence"
            || paid
                && matches!(
                    name.as_str(),
                    "agent_model_cost_facts" | "agent_budget_entries"
                )
        {
            continue;
        }
        assert_eq!(rows, original, "unexpected application change: {name}");
    }
}
#[test]
fn specialist_financial_actual_received_keeps_original_bill_after_publication_faults() {
    for action in ["RAISE(ABORT,'paid_publication_fault')", "RAISE(IGNORE)"] {
        for target in [
            "BEFORE UPDATE OF state ON agent_specialist_calls WHEN NEW.state='received'",
            "BEFORE INSERT ON agent_events WHEN NEW.event_type='model_round_completed'",
            "BEFORE INSERT ON agent_snapshots",
        ] {
            let (root, mut context, lease, child) = specialist_financial_fixture();
            let db = db::open(&context.db_path).unwrap();
            db.execute_batch(&format!(
                "CREATE TRIGGER paid_publication_fault {target} BEGIN SELECT {action}; END;"
            ))
            .unwrap();
            let captured = std::sync::Arc::new(std::sync::Mutex::new(None));
            let capture = captured.clone();
            let path = context.db_path.clone();
            let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(move |_| {
                *capture.lock().unwrap() = Some(web_mode_test_rows(&db::open(&path).unwrap()));
                (
                    200,
                    "application/json",
                    proposal_model_response("paid original result"),
                )
            }));
            context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
            let error = multi_agent_child_round_transport(
                &context,
                &lease,
                &child,
                "readonly",
                json!({"evidence":"frozen"}),
            )
            .unwrap_err();
            assert!(
                !error.contains("specialist_cost_after_publication_failure:"),
                "{error}"
            );
            assert_eq!(seen.lock().unwrap().len(), 1);
            let cost = crate::agent_runtime::multi_agent::budget::balance(
                &db,
                &lease.root_run_id,
                Some(&child.assignment_id),
                "model_requests",
            )
            .unwrap();
            assert_eq!(
                (cost.consumed, cost.indeterminate),
                (1, 0),
                "{target}: {action}: paid SDK cost was rolled back"
            );
            let fact:(String,String,String,String,String)=db.query_row("SELECT root_run_id,assignment_id,child_run_id,request_hash,phase FROM agent_model_cost_facts WHERE family='specialist'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
            assert_eq!(
                (fact.0, fact.1, fact.2, fact.4),
                (
                    lease.root_run_id.clone(),
                    child.assignment_id.clone(),
                    child.run_id.clone(),
                    "received".into()
                )
            );
            let hash: String = db
                .query_row("SELECT request_hash FROM agent_specialist_calls", [], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(fact.3, hash);
            let state:(String,i64,i64,i64)=db.query_row("SELECT state,event_sequence,(SELECT count(*) FROM agent_events WHERE event_type='model_round_completed'),(SELECT count(*) FROM agent_snapshots) FROM agent_specialist_calls",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
            assert_eq!(state, ("executing".into(), 0, 0, 0));
            specialist_financial_unchanged(&db, captured.lock().unwrap().as_ref().unwrap(), true);
            let before = web_mode_test_rows(&db);
            let error = multi_agent_child_round_transport(
                &context,
                &lease,
                &child,
                "readonly",
                json!({"evidence":"frozen"}),
            )
            .unwrap_err();
            assert!(
                error.contains("outcome_unknown_requires_reconciliation"),
                "{error}"
            );
            web_mode_assert_rows(&db, &before);
            assert_eq!(seen.lock().unwrap().len(), 1);
            drop(db);
            fs::remove_dir_all(root).unwrap();
        }
    }
}
#[test]
fn specialist_financial_actual_failed_cost_writer_rolls_back_and_never_retries() {
    for fault in ["BEFORE INSERT ON agent_model_cost_facts BEGIN SELECT RAISE(IGNORE); END;","AFTER INSERT ON agent_model_cost_facts BEGIN UPDATE projects SET status='archived'; END;","AFTER INSERT ON agent_budget_entries WHEN NEW.kind='consume' AND NEW.dimension='model_requests' BEGIN UPDATE projects SET status='archived'; END;"] {
        let (root,mut context,lease,child)=specialist_financial_fixture();let db=db::open(&context.db_path).unwrap();
        db.execute_batch(&format!("CREATE TRIGGER paid_fault BEFORE INSERT ON agent_snapshots BEGIN SELECT RAISE(IGNORE); END; CREATE TRIGGER cost_fault {fault}")).unwrap();
        let captured=std::sync::Arc::new(std::sync::Mutex::new(None));let capture=captured.clone();let path=context.db_path.clone();
        let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(move |_| {*capture.lock().unwrap()=Some(web_mode_test_rows(&db::open(&path).unwrap()));(200,"application/json",proposal_model_response("paid original result"))}));
        context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
        let error=multi_agent_child_round_transport(&context,&lease,&child,"readonly",json!({})).unwrap_err();
        assert!(error.contains("specialist_cost_after_publication_failure:"),"missing cost persistence refusal: {error}");
        specialist_financial_unchanged(&db,captured.lock().unwrap().as_ref().unwrap(),false);
        assert_eq!(db.query_row("SELECT count(*) FROM agent_model_cost_facts",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        let before=web_mode_test_rows(&db);assert!(multi_agent_child_round_transport(&context,&lease,&child,"readonly",json!({})).unwrap_err().contains("outcome_unknown_requires_reconciliation"));
        web_mode_assert_rows(&db,&before);assert_eq!(seen.lock().unwrap().len(),1);
        drop(db);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn specialist_financial_private_cost_preserves_caller_hook_and_exact_replay() {
    use crate::agent_runtime::{multi_agent::specialist, store::UsageDelta};
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
    db.execute_batch("CREATE TRIGGER reject_business BEFORE INSERT ON agent_snapshots BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    let denied = Arc::new(AtomicUsize::new(0));
    let count = denied.clone();
    db.authorizer(Some(move |c: AuthContext<'_>| match c.action {
        AuthAction::Insert {
            table_name: "projects",
        } => {
            count.fetch_add(1, Ordering::SeqCst);
            Authorization::Deny
        }
        _ => Authorization::Allow,
    }))
    .unwrap();
    let usage = UsageDelta {
        input_tokens: 10,
        output_tokens: 10,
        total_tokens: 20,
        model_requests: 1,
        ..Default::default()
    };
    let error =
        specialist::record_received(&db, &call, "original paid result", false, &usage).unwrap_err();
    assert!(
        !error.contains("specialist_cost_after_publication_failure:"),
        "{error}"
    );
    let before = web_mode_test_rows(&db);
    assert!(db
        .execute(
            "INSERT INTO projects(id,name) VALUES(9002,'caller hook')",
            []
        )
        .is_err());
    assert_eq!(denied.load(Ordering::SeqCst), 1);
    web_mode_assert_rows(&db, &before);
    let repeated =
        specialist::record_received(&db, &call, "original paid result", false, &usage).unwrap_err();
    assert_eq!(repeated, error);
    web_mode_assert_rows(&db, &before);
    let conflict =
        specialist::record_received(&db, &call, "changed paid result", false, &usage).unwrap_err();
    assert!(
        conflict.contains("model_cost_fact_replay_conflict"),
        "{conflict}"
    );
    web_mode_assert_rows(&db, &before);
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &lease.root_run_id,
            Some(&child.assignment_id),
            "model_requests"
        )
        .unwrap()
        .consumed,
        1
    );
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn specialist_financial_private_cost_never_joins_or_commits_caller_transaction() {
    use crate::agent_runtime::{multi_agent::specialist, store::UsageDelta};
    let (root, context, lease, child) = specialist_financial_fixture();
    let db = db::open(&context.db_path).unwrap();
    let specialist::Start::Dispatch(call) =
        specialist::start(&db, &lease, &child, &json!({"messages":[]})).unwrap()
    else {
        panic!()
    };
    let tx = db.unchecked_transaction().unwrap();
    let before = web_mode_test_rows(&tx);
    let usage = UsageDelta {
        input_tokens: 10,
        output_tokens: 10,
        total_tokens: 20,
        model_requests: 1,
        ..Default::default()
    };
    let error = specialist::record_received(
        &tx,
        &call,
        "paid but caller owns transaction",
        false,
        &usage,
    )
    .unwrap_err();
    assert!(
        error.contains("specialist_failed_cost_private_transaction_required"),
        "{error}"
    );
    assert!(!db.is_autocommit());
    web_mode_assert_rows(&tx, &before);
    tx.rollback().unwrap();
    web_mode_assert_rows(&db, &before);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
