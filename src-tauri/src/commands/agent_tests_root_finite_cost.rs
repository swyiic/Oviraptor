// Original provider facts distinguish a short estimate from a hard-cap overrun.
fn root_finite_cost_original_call(limit: i64, expected_known: bool) {
    let _real = RealSpecialistTransport::enter();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|_| {
        let mut response: JsonValue = serde_json::from_str(&proposal_model_response(
            &root_tick_valid_text("original finite paid invoice"),
        ))
        .unwrap();
        response["usage"] = json!({
            "prompt_tokens":50_000,"completion_tokens":10,"total_tokens":50_010,
            "prompt_tokens_details":{"cached_tokens":10_000}
        });
        (200, "application/json", response.to_string())
    }));
    let f = root_tick_fixture_protocol_limits(
        "finite-original-paid-cost",
        &format!("http://127.0.0.1:{port}/v1"),
        true,
        (limit, 20),
    );
    let db = db::open(&f.context.db_path).unwrap();
    let native: String = db
        .query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [&f.actor.root_run_id],
            |r| r.get(0),
        )
        .unwrap();
    let result = native_coordinator_tick(&f.context, &f.actor);
    assert_eq!(seen.lock().unwrap().len(), 1);
    let (call, dispatch): (String, String) = db
        .query_row("SELECT call_id,receipt_json FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='dispatch'", [&f.actor.root_run_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    let dispatch: JsonValue = serde_json::from_str(&dispatch).unwrap();
    let held = dispatch["reservedTokens"].as_i64().unwrap();
    assert!(held > 0 && held < 50_010, "actual estimate {held}");
    let invoice: String = db
        .query_row("SELECT receipt_json FROM agent_root_model_journal WHERE call_id=?1 AND phase='received'", [&call], |r| r.get(0))
        .unwrap();
    let invoice: JsonValue = serde_json::from_str(&invoice).unwrap();
    assert_eq!(invoice["usageReported"], true);
    assert_eq!(
        invoice["usage"],
        json!({"inputTokens":50_000,"cachedInputTokens":10_000,"outputTokens":10,"totalTokens":50_010,"modelRequests":1})
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_root_tick_terminal_proofs WHERE call_id=?1",
            [&call],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(db.query_row("SELECT hard_limit FROM agent_budget_limits WHERE root_run_id=?1 AND dimension='model_input_tokens'", [&f.actor.root_run_id], |r| r.get::<_, i64>(0)).unwrap(), limit);
    assert_eq!(
        db.query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [&f.actor.root_run_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        native
    );
    if expected_known {
        assert!(limit > 50_010);
        assert!(
            result.is_ok(),
            "consistent original invoice fits the frozen hard cap but was withheld: {:?}",
            result.err()
        );
        for (dimension, actual) in [
            ("model_input_tokens", 50_000),
            ("model_cached_tokens", 10_000),
            ("model_output_tokens", 10),
            ("model_requests", 1),
        ] {
            let b = crate::agent_runtime::multi_agent::budget::balance(
                &db,
                &f.actor.root_run_id,
                None,
                dimension,
            )
            .unwrap();
            assert_eq!(b.consumed, actual, "{dimension}");
            assert_eq!(b.indeterminate, 0, "{dimension}");
            assert_eq!(b.reserved, 0, "{dimension}");
        }
        assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 1);
    } else {
        assert!(limit < 50_010);
        assert_eq!(
            result.err().unwrap(),
            "budget_indeterminate_requires_reconciliation"
        );
        let b = crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &f.actor.root_run_id,
            None,
            "model_input_tokens",
        )
        .unwrap();
        assert_eq!(b.indeterminate, held);
        assert_eq!(b.consumed, 0);
        assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 0);
    }
    let before = (web_mode_test_rows(&db), single_finally_physical(&db));
    let replay = native_coordinator_tick(&f.context, &f.actor);
    assert_eq!(replay.is_ok(), expected_known);
    assert_eq!(seen.lock().unwrap().len(), 1, "no SDK repayment");
    web_mode_assert_rows(&db, &before.0);
    assert_eq!(single_finally_physical(&db), before.1);
}
#[test]
fn root_finite_actual_consistent_paid_cost_above_estimate_within_original_cap_is_known() {
    root_finite_cost_original_call(200_000, true);
}
#[test]
fn root_finite_actual_paid_cost_above_original_cap_preserves_invoice_and_blocks_repayment() {
    root_finite_cost_original_call(40_000, false);
}

fn root_finite_cost_wire() -> JsonValue {
    let mut r: JsonValue = serde_json::from_str(&proposal_model_response(&root_tick_valid_text(
        "original actual fee",
    )))
    .unwrap();
    r["usage"] = json!({"prompt_tokens":50_000,"completion_tokens":10,"total_tokens":50_010,"prompt_tokens_details":{"cached_tokens":10_000}});
    r
}
#[test]
fn root_finite_actual_topup_writer_fault_keeps_committed_invoice_and_original_hold_without_retry() {
    let _real = RealSpecialistTransport::enter();
    for fault in [
        "SELECT RAISE(IGNORE)",
        "SELECT RAISE(ABORT,'finite_cost_abort')",
        "SELECT RAISE(ROLLBACK,'finite_cost_rollback')",
        "UPDATE agent_runs SET plan_json='{}'",
    ] {
        let path = std::sync::Arc::new(std::sync::Mutex::new(None::<PathBuf>));
        let writer = path.clone();
        let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |_| {
            let db = db::open(writer.lock().unwrap().as_ref().unwrap()).unwrap();
            db.execute_batch(&format!("CREATE TRIGGER finite_cost_fault BEFORE INSERT ON agent_budget_entries WHEN NEW.idempotency_key LIKE '%:actual' BEGIN {fault}; END;")).unwrap();
            (200, "application/json", root_finite_cost_wire().to_string())
        }));
        let f = root_tick_fixture_protocol_limits(
            "finite-paid-writer",
            &format!("http://127.0.0.1:{port}/v1"),
            true,
            (200_000, 20),
        );
        let db = db::open(&f.context.db_path).unwrap();
        let native: String = db
            .query_row(
                "SELECT plan_json FROM agent_runs WHERE id=?1",
                [&f.actor.root_run_id],
                |r| r.get(0),
            )
            .unwrap();
        *path.lock().unwrap() = Some(f.context.db_path.clone());
        assert!(
            native_coordinator_tick(&f.context, &f.actor).is_err(),
            "{fault}"
        );
        assert_eq!(seen.lock().unwrap().len(), 1);
        let (call,raw):(String,String)=db.query_row("SELECT call_id,receipt_json FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='received'",[&f.actor.root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        let invoice: JsonValue = serde_json::from_str(&raw).unwrap();
        assert_eq!(invoice["usage"]["totalTokens"], 50_010);
        assert_eq!(invoice["usageReported"], true);
        assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "decision"), 0);
        assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 0);
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM agent_budget_entries WHERE source_id=?1 AND kind<>'reserve'",
                [&call],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0,
            "cost transaction rolls back, not the committed invoice"
        );
        let b = crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &f.actor.root_run_id,
            Some(""),
            "model_requests",
        )
        .unwrap();
        assert_eq!((b.reserved, b.consumed, b.indeterminate), (1, 0, 0));
        assert_eq!(
            crate::agent_runtime::multi_agent::budget::admission::require_determinate(
                &db,
                &f.actor.root_run_id
            )
            .err(),
            Some("budget_indeterminate_requires_reconciliation".to_string()),
            "the committed but unsettled original invoice must stop fresh work"
        );
        assert_eq!(
            db.query_row(
                "SELECT plan_json FROM agent_runs WHERE id=?1",
                [&f.actor.root_run_id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            native
        );
        db.execute_batch("DROP TRIGGER finite_cost_fault").unwrap();
        let before = (web_mode_test_rows(&db), single_finally_physical(&db));
        assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
        web_mode_assert_rows(&db, &before.0);
        assert_eq!(single_finally_physical(&db), before.1);
        assert_eq!(seen.lock().unwrap().len(), 1);
    }
}
#[test]
fn root_finite_actual_inconsistent_or_missing_usage_cannot_gain_known_fee_or_retry() {
    let _real = RealSpecialistTransport::enter();
    for missing in [false, true] {
        let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |_| {
            let mut r = root_finite_cost_wire();
            if missing {
                r.as_object_mut().unwrap().remove("usage");
            } else {
                r["usage"]["total_tokens"] = 1.into();
            }
            (200, "application/json", r.to_string())
        }));
        let f = root_tick_fixture_protocol_limits(
            "finite-invalid-paid-usage",
            &format!("http://127.0.0.1:{port}/v1"),
            true,
            (200_000, 20),
        );
        let db = db::open(&f.context.db_path).unwrap();
        assert_eq!(
            native_coordinator_tick(&f.context, &f.actor).err().unwrap(),
            "budget_indeterminate_requires_reconciliation"
        );
        let raw:String=db.query_row("SELECT receipt_json FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='received'",[&f.actor.root_run_id],|r|r.get(0)).unwrap();
        assert_eq!(
            serde_json::from_str::<JsonValue>(&raw).unwrap()["usageReported"],
            false
        );
        assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 0);
        let before = (web_mode_test_rows(&db), single_finally_physical(&db));
        assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
        web_mode_assert_rows(&db, &before.0);
        assert_eq!(single_finally_physical(&db), before.1);
        assert_eq!(seen.lock().unwrap().len(), 1);
    }
}
#[test]
fn root_finite_actual_human_invoice_cannot_borrow_original_held_child_capacity() {
    let f = human_root_producer(7);
    let db = db::open(&f.f.h.db_path).unwrap();
    let actor = f.f.session.as_ref().unwrap().lease.clone();
    let captured = std::sync::Arc::new(std::sync::Mutex::new(None));
    let read = captured.clone();
    let root = actor.root_run_id.clone();
    *f.f.boundary.lock().unwrap() = Some(Box::new(move |db| {
        let available =
            crate::agent_runtime::multi_agent::budget::root::remaining_child_capacity(db, &root)
                .unwrap()
                .0
                .unwrap();
        let held:i64=db.query_row("SELECT json_extract(j.receipt_json,'$.reservedTokens') FROM agent_root_model_journal j JOIN agent_root_tick_receipts p ON p.call_id=j.call_id AND p.phase='request' WHERE j.root_run_id=?1 AND j.phase='dispatch' AND json_extract(p.fact_json,'$.request.basis.phase')='human-directive'",[&root],|r|r.get(0)).unwrap();
        let child:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_budget_ledger WHERE root_run_id=?1 AND reserved_tokens>0)",[&root],|r|r.get(0)).unwrap();
        *read.lock().unwrap() = Some((available, held, child));
    }));
    human_chat_confirm(&db, &actor, "team");
    let outcome = NativeAgentBackend.execute(&f.f.h.context);
    assert_eq!(
        outcome.terminal_code(),
        terminal_code::REQUEST_RECONCILIATION_REQUIRED
    );
    let (remaining, held, child) = captured.lock().unwrap().unwrap();
    assert!(child);
    assert!(
        held < 50_010 && remaining < 50_010 - held,
        "{remaining}/{held}"
    );
    assert_eq!(
        db.query_row(
            "SELECT hard_token_budget FROM agent_runs WHERE id=?1",
            [&actor.root_run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        60_000
    );
    assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 5);
    assert!(f.web.lock().unwrap().is_empty());
    assert!(f.f.h.site_seen.lock().unwrap().is_empty());
    let before = (web_mode_test_rows(&db), single_finally_physical(&db));
    let status =
        native_scan_status_for_attempt(&db, &actor.scan_id, None, Some(actor.attempt_number))
            .unwrap();
    let item = &status["humanAssessmentObligations"]["items"][0];
    assert_eq!(item["state"], "cost_unconfirmed");
    assert_eq!(item["reportedUsage"]["totalTokens"], 50_010);
    web_mode_assert_rows(&db, &before.0);
    assert_eq!(single_finally_physical(&db), before.1);
    assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 5);
}

#[test]
fn root_finite_actual_paid_extra_reservation_tamper_refuses_read_and_replay_without_new_effects() {
    let _real = RealSpecialistTransport::enter();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|_| {
        (200, "application/json", root_finite_cost_wire().to_string())
    }));
    let f = root_tick_fixture_protocol_limits(
        "finite-original-extra-tamper",
        &format!("http://127.0.0.1:{port}/v1"),
        true,
        (200_000, 20),
    );
    let db = db::open(&f.context.db_path).unwrap();
    native_coordinator_tick(&f.context, &f.actor).unwrap();
    db.execute_batch("DROP TRIGGER budget_entry_no_update")
        .unwrap();
    assert_eq!(db.execute("UPDATE agent_budget_entries SET amount=amount+1 WHERE root_run_id=?1 AND dimension='model_input_tokens' AND idempotency_key LIKE '%:actual'",[&f.actor.root_run_id]).unwrap(),1);
    let before = (web_mode_test_rows(&db), single_finally_physical(&db));
    assert!(native_scan_status_for_attempt(
        &db,
        &f.actor.scan_id,
        None,
        Some(f.actor.attempt_number)
    )
    .is_err());
    assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
    web_mode_assert_rows(&db, &before.0);
    assert_eq!(single_finally_physical(&db), before.1);
    assert_eq!(seen.lock().unwrap().len(), 1);
}
