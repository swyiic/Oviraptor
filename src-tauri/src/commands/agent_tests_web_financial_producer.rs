#[test]
fn web_financial_producer_actual_original_creator_and_paid_root_reach_one_web_sdk() {
    let f = web_financial_producer_fixture();
    let outcome = NativeAgentBackend.execute(f.context());
    assert_eq!(f.web_seen.lock().unwrap().len(), 1, "{}", outcome.detail());
    assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 5);
    assert!(f.f.h.site_seen.lock().unwrap().is_empty());
    let db = db::open(&f.f.h.db_path).unwrap();
    let s = f.f.session.as_ref().unwrap();
    for (dimension, paid) in [
        ("model_input_tokens", 10),
        ("model_output_tokens", 40),
        ("model_cached_tokens", 0),
        ("model_requests", 1),
    ] {
        let b = crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &s.lease.root_run_id,
            Some(&s.executor.assignment_id),
            dimension,
        )
        .unwrap();
        assert_eq!(b.consumed, paid);
        assert_eq!(b.indeterminate, 0);
    }
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &s.lease.root_run_id,
            Some(""),
            "model_requests"
        )
        .unwrap()
        .consumed,
        3
    );
    assert_eq!(root_tick_count(&db, &s.lease.root_run_id, "publication"), 3);
    let before = web_mode_test_rows(&db);
    native_coordinator_budget_before_executor(f.context()).unwrap();
    web_mode_assert_rows(&db, &before);
    assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 5);
}
#[test]
fn web_financial_producer_actual_unknown_web_keeps_original_paid_root_and_worker_debt() {
    let f = web_financial_producer_fixture();
    f.set_web_reply(|_| {
        (
            503,
            "application/json",
            json!({"error":"actual unknown Web"}).to_string(),
        )
    });
    let outcome = NativeAgentBackend.execute(f.context());
    assert_eq!(
        outcome.terminal_code(),
        terminal_code::REQUEST_RECONCILIATION_REQUIRED,
        "{}",
        outcome.detail()
    );
    assert_eq!(f.web_seen.lock().unwrap().len(), 1);
    assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 5);
    assert!(f.f.h.site_seen.lock().unwrap().is_empty());
    let db = db::open(&f.f.h.db_path).unwrap();
    let s = f.f.session.as_ref().unwrap();
    let b = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &s.lease.root_run_id,
        Some(&s.executor.assignment_id),
        "model_requests",
    )
    .unwrap();
    assert_eq!((b.reserved, b.consumed, b.indeterminate), (0, 0, 1));
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &s.lease.root_run_id,
            Some(&s.executor.assignment_id),
            "model_input_tokens"
        )
        .unwrap()
        .indeterminate,
        f.grant().0
    );
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &s.lease.root_run_id,
            Some(""),
            "model_requests"
        )
        .unwrap()
        .consumed,
        3
    );
    let before = web_mode_test_rows(&db);
    assert!(native_coordinator_budget_before_executor(f.context()).is_err());
    web_mode_assert_rows(&db, &before);
    assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 5);
}

#[test]
fn web_financial_producer_unknown_sibling_blocks_proposed_tool_before_tool_begin() {
    let f = web_financial_producer_fixture();
    let path = f.context().db_path.clone();
    let child = f.context().run.as_ref().unwrap().run_id.clone();
    let target = f.context().target_url.clone();
    f.set_web_reply(move |_| {
        let db = db::open(&path).unwrap();
        let tx = db.unchecked_transaction().unwrap();
        let (lease, assignment) = crate::agent_runtime::multi_agent::budget::target::child_owner(&tx, &child).unwrap().unwrap();
        use crate::agent_runtime::multi_agent::budget::{self, Kind};
        budget::append(&tx, &lease, &assignment, "target_requests", Kind::Reserve, 1, "original-sibling-held", "original-sibling").unwrap();
        budget::append_unknown_cost(&tx, &lease, &assignment, "target_requests", 1, "original-sibling").unwrap();
        tx.commit().unwrap();
        (200, "application/json", model_round(&[("replay_http", json!({"identity":"anonymous","method":"GET","url":format!("{target}/"),"family":"authorization"}))], 10))
    });
    let outcome = NativeAgentBackend.execute(f.context());
    assert_eq!(
        outcome.terminal_code(),
        terminal_code::REQUEST_RECONCILIATION_REQUIRED,
        "{}",
        outcome.detail()
    );
    f.assert_calls(1);
    let db = db::open(&f.context().db_path).unwrap();
    let child = &f.context().run.as_ref().unwrap().run_id;
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM tool_invocations WHERE run_id=?1",
            [child],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(db.query_row("SELECT count(*) FROM agent_web_model_journal WHERE child_run_id=?1 AND phase='received'", [child], |r| r.get::<_, i64>(0)).unwrap(), 1);
}

#[test]
fn web_financial_producer_foreign_scope_gate_refuses_zero_write_zero_sdk() {
    let f = web_financial_producer_fixture();
    let db = db::open(&f.context().db_path).unwrap();
    let before = web_mode_test_rows(&db);
    for kind in 0..3 {
        let mut context = f.context().clone();
        match kind {
            0 => context.scan_id = "foreign-original-scan".into(),
            1 => context.attempt_number += 1,
            2 => context.target_url.push_str("/foreign-scope"),
            _ => unreachable!(),
        }
        assert!(native_web_require_determinate(&context).is_err());
        web_mode_assert_rows(&db, &before);
    }
    f.assert_calls(0);
}
