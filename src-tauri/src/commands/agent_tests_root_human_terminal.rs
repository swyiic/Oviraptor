// Actual billed Human SDK output whose semantic summary is invalid.
fn human_terminal_malformed_producer() -> HumanRootProducer {
    let f = human_root_producer(6);
    let db = db::open(&f.f.h.db_path).unwrap();
    let actor = &f.f.session.as_ref().unwrap().lease;
    human_chat_confirm(&db, actor, "team");
    let _outcome = NativeAgentBackend.execute(&f.f.h.context);
    let call = human_obligation_original_call(&db, &actor.root_run_id);
    assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 5);
    assert_eq!(f.human.lock().unwrap().len(), 1);
    assert!(f.web.lock().unwrap().is_empty());
    assert!(f.f.h.site_seen.lock().unwrap().is_empty());
    assert_eq!(root_tick_count(&db, &actor.root_run_id, "publication"), 3);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_root_tick_receipts WHERE call_id=?1 AND phase='decision'",
            [&call],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_root_model_journal WHERE call_id=?1 AND phase='received'",
            [&call],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    let balance = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &actor.root_run_id,
        Some(""),
        "model_requests",
    )
    .unwrap();
    assert_eq!((balance.consumed, balance.indeterminate), (4, 0));
    f
}
#[test]
fn root_human_terminal_actual_invalid_semantics_retains_original_known_bill_without_summary_or_retry(
) {
    let f = human_terminal_malformed_producer();
    let db = db::open(&f.f.h.db_path).unwrap();
    let actor = &f.f.session.as_ref().unwrap().lease;
    let call = human_obligation_original_call(&db, &actor.root_run_id);
    let invoice: String = db.query_row("SELECT receipt_json FROM agent_root_model_journal WHERE call_id=?1 AND phase='received'", [&call], |r|r.get(0)).unwrap();
    let invoice: JsonValue = serde_json::from_str(&invoice).unwrap();
    let before = (web_mode_test_rows(&db), single_finally_physical(&db));
    let status =
        native_scan_status_for_attempt(&db, &actor.scan_id, None, Some(actor.attempt_number))
            .unwrap();
    let rows = status["humanAssessmentObligations"]["items"]
        .as_array()
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["state"], "assessment_unpublished");
    assert_eq!(rows[0]["humanDirective"]["threadKey"], "team");
    assert_eq!(rows[0]["reportedUsage"], invoice["usage"]);
    assert_eq!(
        status["humanAssessmentObligations"]["executionAllowed"],
        false
    );
    assert_eq!(
        status["humanAssessmentObligations"]["automaticResumeAllowed"],
        false
    );
    assert!(!format!("{:?}", before.0).contains("PRIVATE_INVALID_HUMAN_RESPONSE"));
    web_mode_assert_rows(&db, &before.0);
    assert_eq!(single_finally_physical(&db), before.1);
    assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 5);
}
fn human_terminal_malformed_original_damage_refuses(fault: &str) {
    let f = human_terminal_malformed_producer();
    let db = db::open(&f.f.h.db_path).unwrap();
    let actor = &f.f.session.as_ref().unwrap().lease;
    let call = human_obligation_original_call(&db, &actor.root_run_id);
    // Corrupt only this fresh temporary fixture, after the original real bill.
    if fault == "physical_cost" {
        db.execute_batch("DROP TRIGGER budget_entry_no_update")
            .unwrap();
        assert_eq!(db.execute("UPDATE agent_budget_entries SET rowid=rowid+100000 WHERE source_id=?1 AND dimension='model_requests' AND kind='consume'", [&call]).unwrap(), 1);
    } else {
        let text: String = db.query_row("SELECT receipt_json FROM agent_root_model_journal WHERE call_id=?1 AND phase='received'", [&call], |r|r.get(0)).unwrap();
        let mut invoice: JsonValue = serde_json::from_str(&text).unwrap();
        invoice["responseHash"] =
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".into();
        db.execute_batch("DROP TRIGGER root_model_no_update")
            .unwrap();
        assert_eq!(db.execute("UPDATE agent_root_model_journal SET receipt_json=?1 WHERE call_id=?2 AND phase='received'", params![invoice.to_string(),call]).unwrap(),1);
    }
    let before = (web_mode_test_rows(&db), single_finally_physical(&db));
    let result =
        native_scan_status_for_attempt(&db, &actor.scan_id, None, Some(actor.attempt_number));
    web_mode_assert_rows(&db, &before.0);
    assert_eq!(single_finally_physical(&db), before.1);
    assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 5);
    assert!(f.web.lock().unwrap().is_empty());
    assert!(result.is_err(), "unpublished invalid semantic result accepted changed original {fault} without a terminal proof");
}
#[test]
fn root_human_terminal_invalid_semantics_rejects_replaced_original_physical_cost() {
    human_terminal_malformed_original_damage_refuses("physical_cost");
}
#[test]
fn root_human_terminal_invalid_semantics_rejects_changed_original_response_hash() {
    human_terminal_malformed_original_damage_refuses("invoice_hash");
}
#[test]
fn root_human_terminal_proof_writer_faults_preserve_original_bill_and_never_retry() {
    for body in [
        "SELECT RAISE(IGNORE);",
        "SELECT RAISE(ABORT,'proof-abort');",
        "SELECT RAISE(ROLLBACK,'proof-rollback');",
        "UPDATE agent_budget_ledger SET spent_tokens=spent_tokens+1;",
        "UPDATE agent_root_model_journal SET request_hash='foreign-proof-write' WHERE phase='received';",
        "UPDATE agent_root_tick_terminal_proofs SET fact_json='{}' WHERE call_id=NEW.call_id;",
    ] {
        let f=human_root_producer(6);
        let db=db::open(&f.f.h.db_path).unwrap();
        let actor=&f.f.session.as_ref().unwrap().lease;
        human_chat_confirm(&db,actor,"team");
        db.execute_batch(&format!("CREATE TRIGGER fail_human_terminal_proof BEFORE INSERT ON agent_root_tick_terminal_proofs BEGIN {body} END;")).unwrap();
        let _outcome=NativeAgentBackend.execute(&f.f.h.context);
        let call=human_obligation_original_call(&db,&actor.root_run_id);
        assert_eq!(f.f.h.model_seen.lock().unwrap().len(),5,"{body}");
        assert!(f.web.lock().unwrap().is_empty());
        assert!(f.f.h.site_seen.lock().unwrap().is_empty());
        assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_journal WHERE call_id=?1 AND phase='received'",[&call],|r|r.get::<_,i64>(0)).unwrap(),1,"{body}");
        assert_eq!(db.query_row("SELECT count(*) FROM agent_root_tick_terminal_proofs WHERE call_id=?1",[&call],|r|r.get::<_,i64>(0)).unwrap(),0,"{body}");
        let balance=crate::agent_runtime::multi_agent::budget::balance(&db,&actor.root_run_id,Some(""),"model_requests").unwrap();
        assert_eq!((balance.consumed,balance.indeterminate),(4,0),"{body}");
        assert_eq!(root_tick_count(&db,&actor.root_run_id,"publication"),3);
        let before=(web_mode_test_rows(&db),single_finally_physical(&db));
        let error=native_scan_status_for_attempt(&db,&actor.scan_id,None,Some(actor.attempt_number)).unwrap_err();
        assert_eq!(error,"root_tick_terminal_proof_missing","{body}");
        // Original frame replay cannot repair a proof, claim another dispatch,
        // restore its owner, refund the bill or hide it behind a new call.
        let text:String=db.query_row("SELECT fact_json FROM agent_root_tick_receipts WHERE call_id=?1 AND phase='request'",[&call],|r|r.get(0)).unwrap();
        let request:JsonValue=serde_json::from_str(&text).unwrap();
        let (round,tokens):(i64,i64)=db.query_row("SELECT round,json_extract(receipt_json,'$.reservedTokens') FROM agent_root_model_journal WHERE call_id=?1 AND phase='dispatch'",[&call],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        {
            let tx=db.unchecked_transaction().unwrap();
            assert!(crate::agent_runtime::multi_agent::budget::root::model::tick::Tick::begin(&tx,&actor.root_run_id,round,&request["request"]["basis"],&request["request"]["request"],tokens).is_err(),"{body}");
        }
        web_mode_assert_rows(&db,&before.0);
        assert_eq!(single_finally_physical(&db),before.1,"{body}");
        assert_eq!(f.f.h.model_seen.lock().unwrap().len(),5);
    }
}
#[test]
fn root_human_terminal_original_proof_is_immutable_under_update_delete_and_replace() {
    let f = human_terminal_malformed_producer();
    let db = db::open(&f.f.h.db_path).unwrap();
    let actor = &f.f.session.as_ref().unwrap().lease;
    let call = human_obligation_original_call(&db, &actor.root_run_id);
    let before = (web_mode_test_rows(&db), single_finally_physical(&db));
    for sql in [
        "UPDATE agent_root_tick_terminal_proofs SET fact_json='{}' WHERE call_id=?1",
        "DELETE FROM agent_root_tick_terminal_proofs WHERE call_id=?1",
        "INSERT OR REPLACE INTO agent_root_tick_terminal_proofs SELECT * FROM agent_root_tick_terminal_proofs WHERE call_id=?1",
        "INSERT OR IGNORE INTO agent_root_tick_terminal_proofs SELECT * FROM agent_root_tick_terminal_proofs WHERE call_id=?1",
    ] { assert!(db.execute(sql,[&call]).is_err(),"{sql}"); }
    web_mode_assert_rows(&db, &before.0);
    assert_eq!(single_finally_physical(&db), before.1);
    let status =
        native_scan_status_for_attempt(&db, &actor.scan_id, None, Some(actor.attempt_number))
            .unwrap();
    assert_eq!(
        status["humanAssessmentObligations"]["items"][0]["state"],
        "assessment_unpublished"
    );
    assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 5);
}
#[test]
fn root_human_terminal_damaged_stored_proof_refuses_after_parent_exit_without_write() {
    let mut f = human_terminal_malformed_producer();
    let db = db::open(&f.f.h.db_path).unwrap();
    let actor = f.f.session.as_ref().unwrap().lease.clone();
    let call = human_obligation_original_call(&db, &actor.root_run_id);
    drop(f.f.session.take());
    assert!(check_native_human_directive_actor_on(&db, &f.f.h.context, &actor).is_err());
    let text: String = db
        .query_row(
            "SELECT fact_json FROM agent_root_tick_terminal_proofs WHERE call_id=?1",
            [&call],
            |r| r.get(0),
        )
        .unwrap();
    let mut proof: JsonValue = serde_json::from_str(&text).unwrap();
    proof["costProof"][0]["rowid"] =
        (proof["costProof"][0]["rowid"].as_i64().unwrap() + 100000).into();
    db.execute_batch("DROP TRIGGER root_tick_terminal_proof_no_update")
        .unwrap();
    assert_eq!(
        db.execute(
            "UPDATE agent_root_tick_terminal_proofs SET fact_json=?1 WHERE call_id=?2",
            params![proof.to_string(), call]
        )
        .unwrap(),
        1
    );
    let before = (web_mode_test_rows(&db), single_finally_physical(&db));
    assert_eq!(
        native_scan_status_for_attempt(&db, &actor.scan_id, None, Some(actor.attempt_number))
            .unwrap_err(),
        "root_tick_terminal_proof_changed"
    );
    web_mode_assert_rows(&db, &before.0);
    assert_eq!(single_finally_physical(&db), before.1);
    assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 5);
}
#[test]
fn root_terminal_actual_final_guard_binds_original_parent_terminal_proof_row() {
    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = count.clone();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |_| {
        let turn = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let body = if turn == 0 {
            root_local_wire(
                json!([root_local_call(
                    "original-terminal-parent",
                    "snapshot.read",
                    json!({})
                )]),
                "",
                true,
            )
        } else {
            proposal_model_response(&root_tick_valid_text("final original terminal parent"))
        };
        (200, "application/json", body)
    }));
    let f = root_tick_fixture(
        "root-terminal-parent-guard",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let db = db::open(&f.context.db_path).unwrap();
    let receipt = native_coordinator_tick(&f.context, &f.actor).unwrap();
    assert_eq!(seen.lock().unwrap().len(), 2);
    receipt.tick.require_executable(&db).unwrap();
    // Only move the original first proof's physical identity in this fixture;
    // the original invoice, ledger, request and publication remain untouched.
    db.execute_batch("DROP TRIGGER root_tick_terminal_proof_no_update")
        .unwrap();
    assert_eq!(db.execute("UPDATE agent_root_tick_terminal_proofs SET rowid=rowid+100000 WHERE root_run_id=?1 AND round=1",[&f.actor.root_run_id]).unwrap(),1);
    let before = (web_mode_test_rows(&db), single_finally_physical(&db));
    let rejected = receipt.tick.require_executable(&db).is_err();
    web_mode_assert_rows(&db, &before.0);
    assert_eq!(single_finally_physical(&db), before.1);
    assert_eq!(seen.lock().unwrap().len(), 2);
    assert!(
        rejected,
        "final dispatch accepted a replaced original parent terminal proof row"
    );
}
