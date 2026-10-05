// Actual original paid/uncertain Human Root calls, not UI-created obligations.
fn human_obligation_original_call(db: &rusqlite::Connection, root: &str) -> String {
    db.query_row("SELECT call_id FROM agent_root_tick_receipts WHERE root_run_id=?1 AND phase='request' AND json_extract(fact_json,'$.request.basis.phase')='human-directive' ORDER BY round DESC LIMIT 1",[root],|r|r.get(0)).unwrap()
}
#[test]
fn root_human_unknown_assessment_original_thread_snapshot_retains_usage_without_cursor_or_repayment(
) {
    for (reply, phase, state) in [
        (5, "received", "cost_unconfirmed"),
        (3, "uncertain", "dispatch_unconfirmed"),
    ] {
        let f = human_root_producer(reply);
        let db = db::open(&f.f.h.db_path).unwrap();
        let actor = &f.f.session.as_ref().unwrap().lease;
        let directive = human_chat_confirm(&db, actor, "team");
        let outcome = NativeAgentBackend.execute(&f.f.h.context);
        if reply == 5 {
            assert_eq!(
                outcome.terminal_code(),
                terminal_code::REQUEST_RECONCILIATION_REQUIRED
            );
        }
        assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 5);
        assert!(f.web.lock().unwrap().is_empty());
        assert!(f.f.h.site_seen.lock().unwrap().is_empty());
        let call = human_obligation_original_call(&db, &actor.root_run_id);
        let terminal: i64 = db
            .query_row(
                "SELECT count(*) FROM agent_root_model_journal WHERE call_id=?1 AND phase=?2",
                params![call, phase],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            terminal, 1,
            "actual terminal phase for reply {reply}: {outcome:?}"
        );
        assert_eq!(root_tick_count(&db, &actor.root_run_id, "publication"), 3);
        let before = (web_mode_test_rows(&db), single_finally_physical(&db));
        let first =
            native_scan_status_for_attempt(&db, &actor.scan_id, None, Some(actor.attempt_number))
                .unwrap();
        let obligations = &first["humanAssessmentObligations"];
        assert_eq!(
            obligations["schemaVersion"], 1,
            "unknown original Human assessment is absent from status"
        );
        assert_eq!(obligations["executionAllowed"], false);
        assert_eq!(obligations["automaticResumeAllowed"], false);
        let items = obligations["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        let row = &items[0];
        assert_eq!(row["callId"], call);
        assert_eq!(row["rootRunId"], actor.root_run_id);
        assert_eq!(row["state"], state);
        assert_eq!(row["humanDirective"]["directiveId"], directive);
        assert_eq!(row["humanDirective"]["threadKey"], "team");
        assert_eq!(row["targetKey"], actor.target_key);
        if reply == 5 {
            assert_eq!(row["reportedUsage"]["inputTokens"], 100_000);
            assert_eq!(row["reportedUsage"]["modelRequests"], 1);
        } else {
            assert!(row["reportedUsage"].is_null());
        }
        let cursor = first["latestSequence"].as_i64().unwrap();
        let replay = native_scan_status_for_attempt(
            &db,
            &actor.scan_id,
            Some(cursor),
            Some(actor.attempt_number),
        )
        .unwrap();
        assert_eq!(replay["humanAssessmentObligations"], *obligations);
        assert_eq!(replay["latestSequence"], cursor);
        web_mode_assert_rows(&db, &before.0);
        assert_eq!(single_finally_physical(&db), before.1);
        assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 5);
    }
}
#[test]
fn root_human_unknown_assessment_snapshot_keeps_original_thread_after_parent_exit_and_current_damage(
) {
    let mut f = human_root_producer(5);
    let db = db::open(&f.f.h.db_path).unwrap();
    let actor = f.f.session.as_ref().unwrap().lease.clone();
    human_chat_confirm(&db, &actor, "team");
    let outcome = NativeAgentBackend.execute(&f.f.h.context);
    assert_eq!(
        outcome.terminal_code(),
        terminal_code::REQUEST_RECONCILIATION_REQUIRED
    );
    db.execute(
        "UPDATE agent_user_directives SET thread_key='foreign-thread' WHERE root_run_id=?1",
        [&actor.root_run_id],
    )
    .unwrap();
    drop(f.f.session.take());
    assert!(check_native_human_directive_actor_on(&db, &f.f.h.context, &actor).is_err());
    let before = (web_mode_test_rows(&db), single_finally_physical(&db));
    let status =
        native_scan_status_for_attempt(&db, &actor.scan_id, None, Some(actor.attempt_number))
            .unwrap();
    let rows = status["humanAssessmentObligations"]["items"]
        .as_array()
        .expect("original unresolved call must remain readable");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["humanDirective"]["threadKey"], "team");
    assert_eq!(rows[0]["state"], "cost_unconfirmed");
    assert_eq!(rows[0]["reportedUsage"]["inputTokens"], 100_000);
    web_mode_assert_rows(&db, &before.0);
    assert_eq!(single_finally_physical(&db), before.1);
    assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 5);
    assert!(f.web.lock().unwrap().is_empty());
}
#[test]
fn root_human_unknown_assessment_snapshot_rejects_original_request_tamper_without_write() {
    let f = human_root_producer(5);
    let db = db::open(&f.f.h.db_path).unwrap();
    let actor = &f.f.session.as_ref().unwrap().lease;
    human_chat_confirm(&db, actor, "team");
    let outcome = NativeAgentBackend.execute(&f.f.h.context);
    assert_eq!(
        outcome.terminal_code(),
        terminal_code::REQUEST_RECONCILIATION_REQUIRED
    );
    let call = human_obligation_original_call(&db, &actor.root_run_id);
    db.execute_batch("DROP TRIGGER root_tick_no_update")
        .unwrap();
    db.execute("UPDATE agent_root_tick_receipts SET fact_json=json_set(fact_json,'$.request.basis.changedFact.semantic.threadKey','foreign-thread') WHERE call_id=?1 AND phase='request'",[call]).unwrap();
    let before = (web_mode_test_rows(&db), single_finally_physical(&db));
    let error =
        native_scan_status_for_attempt(&db, &actor.scan_id, None, Some(actor.attempt_number))
            .expect_err("a changed original input must not disappear or redirect an obligation");
    assert_eq!(error, "root_tick_original_input_conflict");
    web_mode_assert_rows(&db, &before.0);
    assert_eq!(single_finally_physical(&db), before.1);
    assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 5);
}
#[test]
fn root_human_unpublished_received_rejects_original_invoice_and_physical_cost_damage() {
    for fault in [
        "invoice_usage",
        "invoice_hash",
        "journal_binding",
        "terminal_amount",
        "terminal_identity",
    ] {
        let f = human_root_producer(5);
        let db = db::open(&f.f.h.db_path).unwrap();
        let actor = &f.f.session.as_ref().unwrap().lease;
        human_chat_confirm(&db, actor, "team");
        assert_eq!(
            NativeAgentBackend.execute(&f.f.h.context).terminal_code(),
            terminal_code::REQUEST_RECONCILIATION_REQUIRED
        );
        let call = human_obligation_original_call(&db, &actor.root_run_id);
        // Damage only the fresh temporary fixture after its actual original SDK receipt.
        if fault.starts_with("invoice_") {
            let text: String = db.query_row("SELECT receipt_json FROM agent_root_model_journal WHERE call_id=?1 AND phase='received'", [&call], |r| r.get(0)).unwrap();
            let mut invoice: JsonValue = serde_json::from_str(&text).unwrap();
            if fault == "invoice_usage" {
                invoice["usage"]["inputTokens"] = 100_001.into();
                invoice["usage"]["totalTokens"] = 100_001.into();
            } else {
                invoice["responseHash"] =
                    "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".into();
            }
            db.execute_batch("DROP TRIGGER root_model_no_update")
                .unwrap();
            assert_eq!(db.execute("UPDATE agent_root_model_journal SET receipt_json=?1 WHERE call_id=?2 AND phase='received'", params![invoice.to_string(), call]).unwrap(), 1);
        } else if fault == "journal_binding" {
            db.execute_batch("DROP TRIGGER root_model_no_update")
                .unwrap();
            assert_eq!(db.execute("UPDATE agent_root_model_journal SET round=round+1 WHERE call_id=?1 AND phase='received'", [&call]).unwrap(), 1);
        } else {
            db.execute_batch("DROP TRIGGER budget_entry_no_update")
                .unwrap();
            let change = if fault == "terminal_amount" {
                "amount=amount+1"
            } else {
                "rowid=rowid+100000"
            };
            assert_eq!(db.execute(&format!("UPDATE agent_budget_entries SET {change} WHERE source_id=?1 AND dimension='model_input_tokens' AND kind='forfeit'"), [&call]).unwrap(), 1);
        }
        let before = (web_mode_test_rows(&db), single_finally_physical(&db));
        let error =
            native_scan_status_for_attempt(&db, &actor.scan_id, None, Some(actor.attempt_number))
                .expect_err(fault);
        assert!(
            matches!(
                error.as_str(),
                "root_tick_terminal_proof_changed"
                    | "budget_root_call_binding_conflict"
                    | "root_tick_original_cost_changed"
                    | "root_tick_decision_changed"
            ),
            "{fault}: {error}"
        );
        web_mode_assert_rows(&db, &before.0);
        assert_eq!(single_finally_physical(&db), before.1, "{fault}");
        assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 5);
        assert!(f.web.lock().unwrap().is_empty());
        assert!(f.f.h.site_seen.lock().unwrap().is_empty());
    }
}
#[test]
fn root_human_unpublished_uncertain_rejects_terminal_cost_and_original_reserve_damage() {
    for fault in ["terminal_amount", "terminal_source", "reserve_identity"] {
        let f = human_root_producer(3);
        let db = db::open(&f.f.h.db_path).unwrap();
        let actor = &f.f.session.as_ref().unwrap().lease;
        human_chat_confirm(&db, actor, "team");
        let _outcome = NativeAgentBackend.execute(&f.f.h.context);
        let call = human_obligation_original_call(&db, &actor.root_run_id);
        assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_journal WHERE call_id=?1 AND phase='uncertain'", [&call], |r| r.get::<_,i64>(0)).unwrap(), 1);
        db.execute_batch("DROP TRIGGER budget_entry_no_update")
            .unwrap();
        let (change, kind) = match fault {
            "terminal_amount" => ("amount=amount+1", "forfeit"),
            "terminal_source" => ("source_id='foreign-model-call'", "forfeit"),
            _ => ("rowid=rowid+100000", "reserve"),
        };
        assert_eq!(db.execute(&format!("UPDATE agent_budget_entries SET {change} WHERE source_id=?1 AND dimension='model_input_tokens' AND kind=?2"), params![call,kind]).unwrap(), 1);
        let before = (web_mode_test_rows(&db), single_finally_physical(&db));
        let error =
            native_scan_status_for_attempt(&db, &actor.scan_id, None, Some(actor.attempt_number))
                .expect_err(fault);
        assert!(
            matches!(
                error.as_str(),
                "root_tick_original_cost_changed" | "root_tick_request_changed"
            ),
            "{fault}: {error}"
        );
        web_mode_assert_rows(&db, &before.0);
        assert_eq!(single_finally_physical(&db), before.1, "{fault}");
        assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 5);
        assert!(f.web.lock().unwrap().is_empty());
    }
}
#[test]
fn root_human_unpublished_snapshot_isolates_scan_and_attempt_without_granting_authority() {
    use crate::agent_runtime::multi_agent::budget::root::model::tick::Tick;
    let f = human_root_producer(5);
    let db = db::open(&f.f.h.db_path).unwrap();
    let actor = &f.f.session.as_ref().unwrap().lease;
    human_chat_confirm(&db, actor, "team");
    assert_eq!(
        NativeAgentBackend.execute(&f.f.h.context).terminal_code(),
        terminal_code::REQUEST_RECONCILIATION_REQUIRED
    );
    let call = human_obligation_original_call(&db, &actor.root_run_id);
    let before = (web_mode_test_rows(&db), single_finally_physical(&db));
    {
        let tx = db.unchecked_transaction().unwrap();
        for (scan, attempt) in [
            ("foreign-scan", actor.attempt_number),
            (actor.scan_id.as_str(), actor.attempt_number + 1),
        ] {
            let other = native_human_assessment_obligations(&tx, scan, attempt).unwrap();
            assert_eq!(other["items"], json!([]));
            assert_eq!(other["truncated"], false);
            assert_eq!(other["executionAllowed"], false);
            assert_eq!(other["automaticResumeAllowed"], false);
            assert_eq!(
                Tick::project_unpublished_human(&tx, &actor.root_run_id, &call, scan, attempt)
                    .unwrap_err(),
                "root_tick_unpublished_scope_conflict"
            );
        }
        let original =
            native_human_assessment_obligations(&tx, &actor.scan_id, actor.attempt_number).unwrap();
        assert_eq!(original["items"].as_array().unwrap().len(), 1);
        assert_eq!(original["items"][0]["callId"], call);
        assert_eq!(original["executionAllowed"], false);
        assert_eq!(original["automaticResumeAllowed"], false);
    }
    web_mode_assert_rows(&db, &before.0);
    assert_eq!(single_finally_physical(&db), before.1);
    assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 5);
    assert!(f.web.lock().unwrap().is_empty());
}
