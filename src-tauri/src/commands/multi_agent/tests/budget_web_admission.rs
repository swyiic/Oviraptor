#[test]
fn budget_web_executor_rechecks_root_unknown_cost_before_each_model_round() {
    let f = web_financial_producer_fixture();
    let path = f.context().db_path.clone();
    let run = f.context().run.as_ref().unwrap().run_id.clone();
    let entered = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let calls = entered.clone();
    f.set_web_reply(move |_| {
        if calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            let db = db::open(&path).unwrap();
            let tx = db.unchecked_transaction().unwrap();
            let (lease, assignment) =
                crate::agent_runtime::multi_agent::budget::target::child_owner(&tx, &run)
                    .unwrap()
                    .unwrap();
            use crate::agent_runtime::multi_agent::budget::{self, Kind};
            budget::append(
                &tx,
                &lease,
                &assignment,
                "target_requests",
                Kind::Reserve,
                1,
                "sibling-held",
                "sibling-original",
            )
            .unwrap();
            budget::append_unknown_cost(
                &tx,
                &lease,
                &assignment,
                "target_requests",
                1,
                "sibling-original",
            )
            .unwrap();
            tx.commit().unwrap();
        }
        (200, "application/json", model_round(&[], 10))
    });
    let mut context = f.context().clone();
    let result = NativeAgentBackend.execute(&context);
    assert_eq!(
        entered.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "{}",
        result.detail()
    );
    assert_eq!(
        result.terminal_code(),
        terminal_code::REQUEST_RECONCILIATION_REQUIRED,
        "{}",
        result.detail()
    );
    assert_eq!(result.terminal_status(), "paused");
    f.assert_calls(1);
    let db = db::open(&context.db_path).unwrap();
    let child = &context.run.as_ref().unwrap().run_id;
    assert_eq!(db.query_row("SELECT count(*) FROM agent_web_model_journal WHERE child_run_id=?1 AND phase='received'",[child],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_events WHERE run_id=?1 AND event_type='model_round_completed'",[child],|r|r.get::<_,i64>(0)).unwrap(),1);
    let actor = web_inflight_original_actor(&db, child);
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &actor.root_run_id,
            None,
            "target_requests"
        )
        .unwrap()
        .indeterminate,
        1
    );
    use crate::agent_runtime::multi_agent::attempts::audit_rows::Rows;
    let fees = Rows::read(
        &db,
        "SELECT rowid,* FROM agent_budget_entries WHERE dimension<>'wall_time_ms' ORDER BY rowid",
        [],
    )
    .unwrap();
    context.resume = true;
    assert_eq!(
        NativeAgentBackend.execute(&context).terminal_code(),
        terminal_code::REQUEST_RECONCILIATION_REQUIRED
    );
    assert!(Rows::read(&db,"SELECT rowid,* FROM agent_budget_entries WHERE dimension<>'wall_time_ms' ORDER BY rowid",[]).unwrap()==fees);
    assert_eq!(entered.load(std::sync::atomic::Ordering::SeqCst), 1);
    f.assert_calls(1);
}

#[test]
fn budget_web_executor_unknown_transport_holds_cost_without_hidden_retry_or_secret_leak() {
    let f = web_financial_producer_fixture();
    f.set_web_reply(|_|(500,"application/json",r#"{"error":{"message":"fixture-model-failure","password":"private-model-failure-secret"}}"#.into()));
    let outcome = NativeAgentBackend.execute(f.context());
    f.assert_calls(1);
    assert_eq!(
        outcome.terminal_code(),
        terminal_code::REQUEST_RECONCILIATION_REQUIRED
    );
    assert!(outcome.detail().contains("fixture-model-failure"));
    assert!(!outcome.detail().contains("private-model-failure-secret"));
    let db = db::open(&f.context().db_path).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let (lease, assignment) = crate::agent_runtime::multi_agent::budget::target::child_owner(
        &tx,
        &f.context().run.as_ref().unwrap().run_id,
    )
    .unwrap()
    .unwrap();
    let b = crate::agent_runtime::multi_agent::budget::balance(
        &tx,
        &lease.root_run_id,
        Some(&assignment),
        "model_requests",
    )
    .unwrap();
    assert_eq!((b.reserved, b.consumed, b.indeterminate), (0, 0, 1));
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::model::release_unsent(&tx, &lease, &assignment)
            .unwrap_err(),
        "budget_indeterminate_requires_reconciliation"
    );
    tx.rollback().unwrap();
}

#[test]
fn budget_web_model_claim_precedes_transport_and_receipt_precedes_more_work() {
    let f = web_financial_producer_fixture();
    let path = f.context().db_path.clone();
    let child = f.context().run.as_ref().unwrap().run_id.clone();
    let claimed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let observed = claimed.clone();
    f.set_web_reply(move |_| {let db=db::open(&path).unwrap();observed.store(db.query_row("SELECT EXISTS(SELECT 1 FROM agent_web_model_journal WHERE child_run_id=?1 AND round=1 AND phase='dispatch')",[&child],|r|r.get(0)).unwrap(),std::sync::atomic::Ordering::SeqCst);(200,"application/json",model_round(&[],10))});
    let _ = NativeAgentBackend.execute(f.context());
    assert!(
        claimed.load(std::sync::atomic::Ordering::SeqCst),
        "provider needs durable original-owner claim"
    );
    f.assert_calls(1);
    let db = db::open(&f.context().db_path).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let (lease, assignment) = crate::agent_runtime::multi_agent::budget::target::child_owner(
        &tx,
        &f.context().run.as_ref().unwrap().run_id,
    )
    .unwrap()
    .unwrap();
    let b = crate::agent_runtime::multi_agent::budget::balance(
        &tx,
        &lease.root_run_id,
        Some(&assignment),
        "model_requests",
    )
    .unwrap();
    assert_eq!((b.reserved, b.consumed, b.indeterminate), (0, 1, 0));
    assert_eq!(tx.query_row("SELECT count(*) FROM agent_web_model_journal WHERE child_run_id=?1 AND phase='received'",[&f.context().run.as_ref().unwrap().run_id],|r|r.get::<_,i64>(0)).unwrap(),1);
    tx.rollback().unwrap();
}

#[test]
fn budget_web_model_missing_provider_usage_keeps_estimates_unknown() {
    let f = web_financial_producer_fixture();
    let grant = f.grant().0;
    f.set_web_reply(|_| {
        (
            200,
            "application/json",
            r#"{"choices":[{"message":{"content":"no provider usage"},"finish_reason":"stop"}]}"#
                .into(),
        )
    });
    let result = NativeAgentBackend.execute(f.context());
    assert_eq!(
        result.terminal_code(),
        terminal_code::REQUEST_RECONCILIATION_REQUIRED,
        "{}",
        result.detail()
    );
    f.assert_calls(1);
    let db = db::open(&f.context().db_path).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let (lease, assignment) = crate::agent_runtime::multi_agent::budget::target::child_owner(
        &tx,
        &f.context().run.as_ref().unwrap().run_id,
    )
    .unwrap()
    .unwrap();
    let b = crate::agent_runtime::multi_agent::budget::balance(
        &tx,
        &lease.root_run_id,
        Some(&assignment),
        "model_input_tokens",
    )
    .unwrap();
    assert_eq!(
        (b.reserved, b.consumed, b.indeterminate),
        (0, 0, grant),
        "actual original grant estimate is not a provider invoice"
    );
    let q = crate::agent_runtime::multi_agent::budget::balance(
        &tx,
        &lease.root_run_id,
        Some(&assignment),
        "model_requests",
    )
    .unwrap();
    assert_eq!((q.reserved, q.consumed, q.indeterminate), (0, 1, 0));
    tx.rollback().unwrap();
}

#[test]
fn budget_web_model_combined_token_overrun_preserves_invoice_and_stops_work() {
    let f = web_financial_producer_fixture();
    let grant = f.grant().0;
    let half = grant / 2 + 1;
    f.set_web_reply(move |_| {
        let mut body: JsonValue = serde_json::from_str(&model_round(&[], half)).unwrap();
        body["usage"]["completion_tokens"] = json!(half);
        body["usage"]["total_tokens"] = json!(2 * half);
        (200, "application/json", body.to_string())
    });
    let result = NativeAgentBackend.execute(f.context());
    assert_eq!(
        result.terminal_code(),
        terminal_code::REQUEST_RECONCILIATION_REQUIRED,
        "{}",
        result.detail()
    );
    f.assert_calls(1);
    let db = db::open(&f.context().db_path).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let (lease, assignment) = crate::agent_runtime::multi_agent::budget::target::child_owner(
        &tx,
        &f.context().run.as_ref().unwrap().run_id,
    )
    .unwrap()
    .unwrap();
    let b = crate::agent_runtime::multi_agent::budget::balance(
        &tx,
        &lease.root_run_id,
        Some(&assignment),
        "model_input_tokens",
    )
    .unwrap();
    assert_eq!((b.reserved, b.consumed, b.indeterminate), (0, 0, grant));
    assert_eq!(tx.query_row("SELECT json_extract(receipt_json,'$.usage.totalTokens') FROM agent_web_model_journal WHERE child_run_id=?1 AND phase='received'",[&f.context().run.as_ref().unwrap().run_id],|r|r.get::<_,i64>(0)).unwrap(),2*half);
    tx.rollback().unwrap();
}
