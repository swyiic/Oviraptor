#[test]
fn ordered_execution_actual_entry_commits_original_dispatch_while_provider_alive_then_preserves_written_order(
) {
    use std::sync::{mpsc, Arc, Mutex};
    let (arrive_tx, arrive) = mpsc::channel();
    let (release_tx, release) = mpsc::channel();
    let release = Arc::new(Mutex::new(release));
    let (port, seen, _stop) = spawn_endpoint(Arc::new(move |_| {
        let _ = arrive_tx.send(());
        let _ = release.lock().unwrap().recv_timeout(Duration::from_secs(8));
        (
            200,
            "application/json",
            proposal_model_response(valid_proposal_text()),
        )
    }));
    let (mut f, id) = ordered_exec_fixture(
        "ordered-real-dispatch",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let db = db::open(&f.context.db_path).unwrap();
    let native = ordered_exec_native(&db, &f.actor.root_run_id);
    let review:String=db.query_row("SELECT receipt_json FROM agent_directive_human_reviews WHERE draft_id=(SELECT source_draft_id FROM agent_user_directives WHERE id=?1)",[&id],|r|r.get(0)).unwrap();
    std::thread::scope(|scope| {
        let (done_tx, done) = mpsc::channel();
        let f = &f;
        scope.spawn(move || {
            let _ = done_tx.send(ordered_exec_apply(f));
        });
        arrive.recv_timeout(Duration::from_secs(5)).unwrap();
        let child:String=db.query_row("SELECT child_run_id FROM agent_directive_ordered_actions WHERE directive_id=?1 AND action_order=1",[&id],|r|r.get(0)).unwrap();
        let committed:i64=db.query_row("SELECT COUNT(*) FROM agent_specialist_calls c JOIN agent_directive_ordered_actions a ON a.child_run_id=c.child_run_id AND a.assignment_id=c.assignment_id
        JOIN agent_assignment_attempts w ON w.child_run_id=c.child_run_id WHERE a.directive_id=?1 AND a.action_order=1 AND a.state='executing'
        AND c.state='executing' AND w.state='running' AND json_extract(c.request_json,'$.humanDirectiveDispatch.workerId')=w.worker_id
        AND json_extract(c.request_json,'$.humanDirectiveDispatch.actionId')=a.action_id",[&id],|r|r.get(0)).unwrap();
        assert_eq!(committed, 1);
        assert!(done.try_recv().is_err());
        assert_eq!(
            ordered_exec_count(&db, "agent_directive_ordered_receipts"),
            0
        );
        let sdk = ordered_exec_sdk(f, &child);
        assert_eq!(
            sdk.iter()
                .map(|r| r["stage"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["prepared", "sent"]
        );
        assert!(sdk
            .iter()
            .all(|r| r["domain"] == "specialist" && !r["workerId"].as_str().unwrap().is_empty()));
        assert_eq!(
            ordered_exec_projection(&db, &id)["actions"][1]["state"],
            "not_started"
        );
        release_tx.send(()).unwrap();
        assert_eq!(
            done.recv_timeout(Duration::from_secs(5))
                .unwrap()
                .unwrap()
                .len(),
            1
        );
    });
    assert_eq!(seen.lock().unwrap().len(), 1);
    let first = ordered_exec_receipts(&db, &id);
    assert_eq!(first.len(), 1);
    assert_eq!(first[0]["role"], "deep_investigator");
    assert_eq!(first[0]["usage"]["totalTokens"], 20);
    assert_eq!(first[0]["usage"]["modelRequests"], 1);
    assert_eq!(first[0]["independentReviewApproved"], false);
    assert!(first[0]["reviewerReceipt"].is_null());
    f.context.evidence = json!({"updated_after_first":"must_not_replace_original_frozen_evidence"});
    std::thread::scope(|scope| {
        let (done_tx, done) = mpsc::channel();
        let f = &f;
        scope.spawn(move || {
            let _ = done_tx.send(ordered_exec_apply(f));
        });
        arrive.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(ordered_exec_receipts(&db, &id), first);
        assert_eq!(ordered_exec_count(&db, "agent_specialist_calls"), 2);
        release_tx.send(()).unwrap();
        assert_eq!(
            done.recv_timeout(Duration::from_secs(5))
                .unwrap()
                .unwrap()
                .len(),
            2
        );
    });
    let both = ordered_exec_receipts(&db, &id);
    assert_eq!(both.len(), 2);
    assert_eq!(both[1]["role"], "spa_api_mapper");
    assert_eq!(both[1]["predecessor"]["receiptId"], first[0]["receiptId"]);
    assert_ne!(both[0]["workerId"], both[1]["workerId"]);
    assert_ne!(both[0]["childRunId"], both[1]["childRunId"]);
    for receipt in &both {
        let child = receipt["childRunId"].as_str().unwrap();
        let original_body: String = db
            .query_row(
                "SELECT response_json FROM agent_specialist_calls WHERE child_run_id=?1",
                [child],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<JsonValue>(&original_body).unwrap()["text"],
            valid_proposal_text(),
            "safe diagnostics and local summaries preserve the original model bytes"
        );
        let sdk = ordered_exec_sdk(&f, child);
        assert_eq!(
            sdk.iter()
                .map(|r| r["stage"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "prepared",
                "sent",
                "response_received",
                "cost_saved",
                "validated",
                "terminal"
            ]
        );
        assert_eq!(sdk.last().unwrap()["terminalState"], "returned");
        assert!(sdk
            .iter()
            .all(|r| r["domain"] == "specialist" && r["workerId"] == receipt["workerId"]));
        assert!(!json!(sdk).to_string().contains("独立评估已有路由"));
        assert_eq!(
            receipt["rootNativeContractHash"],
            crate::agent_runtime::store::stable_hash(&native)
        );
    }
    let inputs: Vec<JsonValue> = {
        let mut q=db.prepare("SELECT input_json FROM agent_directive_ordered_actions WHERE directive_id=?1 ORDER BY action_order").unwrap();
        q.query_map([&id], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|r| serde_json::from_str(&r.unwrap()).unwrap())
            .collect()
    };
    assert_eq!(inputs[0]["frozenEvidence"], inputs[1]["frozenEvidence"]);
    assert_eq!(
        inputs[1]["previousAssessment"]["receiptHash"],
        crate::agent_runtime::store::stable_hash(&first[0].to_string())
    );
    assert!(!inputs[1]
        .to_string()
        .contains("must_not_replace_original_frozen_evidence"));
    let used: (i64, i64) = db
        .query_row(
            "SELECT spent_tokens,spent_requests FROM agent_budget_ledger WHERE root_run_id=?1",
            [&f.actor.root_run_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(used, (40, 2));
    assert_eq!(ordered_exec_native(&db, &f.actor.root_run_id), native);
    let current_review:String=db.query_row("SELECT receipt_json FROM agent_directive_human_reviews WHERE draft_id=(SELECT source_draft_id FROM agent_user_directives WHERE id=?1)",[&id],|r|r.get(0)).unwrap();
    assert_eq!(
        current_review, review,
        "immutable approval is not overwritten with executionState"
    );
    let projection = ordered_exec_projection(&db, &id);
    assert_eq!(projection["completedAssessments"], 2);
    assert_eq!(projection["state"], "completed");
    let timeline = native_scan_status_after(&db, &f.context.scan_id, None).unwrap();
    let item = timeline["timeline"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == id && r["eventType"] == "user_directive")
        .unwrap();
    assert_eq!(item["orderedAssessmentExecution"], projection);
    let before = ordered_exec_count(&db, "native_sdk_log_rows");
    assert_eq!(ordered_exec_apply(&f).unwrap().len(), 2);
    assert_eq!(seen.lock().unwrap().len(), 2);
    assert_eq!(ordered_exec_receipts(&db, &id), both);
    assert_eq!(
        ordered_exec_count(&db, "native_sdk_log_rows"),
        before,
        "paid replay emits no fake new SDK stage"
    );
}

#[test]
fn ordered_execution_unreported_or_invalid_first_response_never_dispatches_the_second_action() {
    for unknown in [true, false] {
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(move |_| {
            let mut response: JsonValue =
                serde_json::from_str(&proposal_model_response(if unknown {
                    valid_proposal_text()
                } else {
                    "invalid"
                }))
                .unwrap();
            if unknown {
                response.as_object_mut().unwrap().remove("usage");
            }
            (200, "application/json", response.to_string())
        }));
        let (f, id) = ordered_exec_fixture(
            "ordered-withheld-response",
            &format!("http://127.0.0.1:{port}/v1"),
        );
        let db = db::open(&f.context.db_path).unwrap();
        let result = ordered_exec_apply(&f);
        if unknown {
            assert!(result.is_err());
            assert!(ordered_exec_receipts(&db, &id).is_empty());
        } else {
            result.unwrap();
            assert_eq!(
                ordered_exec_receipts(&db, &id)[0]["outcome"],
                "invalid_assessment"
            );
        }
        let _ = ordered_exec_apply(&f);
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert_eq!(ordered_exec_count(&db, "agent_specialist_calls"), 1);
        assert_eq!(
            ordered_exec_projection(&db, &id)["actions"][1]["state"],
            "not_started"
        );
        let assignment: String = db
            .query_row(
                "SELECT assignment_id FROM agent_directive_ordered_actions WHERE directive_id=?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        let cost = crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &f.actor.root_run_id,
            Some(&assignment),
            "model_requests",
        )
        .unwrap();
        assert_eq!(cost.consumed + cost.indeterminate, 1);
    }
}

#[test]
fn ordered_execution_v2_confirmation_and_unknown_old_executing_are_never_adopted() {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            proposal_model_response(valid_proposal_text()),
        )
    }));
    let f = root_tick_fixture("ordered-old-v2", &format!("http://127.0.0.1:{port}/v1"));
    let db = db::open(&f.context.db_path).unwrap();
    let id = ordered_exec_seed_v2(&db, &f.actor);
    let raw:String=db.query_row("SELECT receipt_json FROM agent_directive_human_reviews WHERE draft_id=(SELECT source_draft_id FROM agent_user_directives WHERE id=?1)",[&id],|r|r.get(0)).unwrap();
    ordered_exec_apply(&f).unwrap();
    let reason: String = db
        .query_row(
            "SELECT rejection_code FROM agent_user_directives WHERE id=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(reason, "proposal_ordered_execution_not_connected");
    assert_eq!(
        ordered_exec_count(&db, "agent_directive_ordered_actions"),
        0
    );
    assert_eq!(db.query_row("SELECT receipt_json FROM agent_directive_human_reviews WHERE draft_id=(SELECT source_draft_id FROM agent_user_directives WHERE id=?1)",[&id],|r|r.get::<_,String>(0)).unwrap(),raw);
    assert_eq!(seen.lock().unwrap().len(), 0);
    // Current v3 checkpoint marked executing without original dispatch is
    // still unknown. It cannot manufacture a receipt or a replacement call.
    let id = confirm_queue_directive(&db, &f.actor, "@mapper 然后 @investigator 评估已有证据");
    let inbox = take_human_directives(&f.context).unwrap();
    let actor = inbox.lease.as_ref().unwrap();
    use crate::agent_runtime::multi_agent::directive::ordered_execution as ordered;
    let job = ordered::prepare_next(&db, actor, &f.context.evidence, |_| Ok(()))
        .unwrap()
        .unwrap();
    ordered::consume_request(&db, actor, &job, |_| Ok(())).unwrap();
    ordered::start(&db, actor, &id, 1, |_| Ok(())).unwrap();
    ordered_exec_apply(&f).unwrap();
    assert_eq!(seen.lock().unwrap().len(), 0);
    assert!(ordered_exec_receipts(&db, &id).is_empty());
    assert_eq!(
        ordered_exec_projection(&db, &id)["actions"][0]["state"],
        "outcome_unknown"
    );
}
#[test]
fn ordered_execution_terminal_root_and_replaced_c_can_read_paid_history_without_replaying_actions()
{
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            proposal_model_response(valid_proposal_text()),
        )
    }));
    let (f, id) = ordered_exec_fixture(
        "ordered-historical-paid",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let db = db::open(&f.context.db_path).unwrap();
    ordered_exec_apply(&f).unwrap();
    let original = ordered_exec_receipts(&db, &id);
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    close_human_directives_in_transaction(&tx, &f.actor, "test_ordered_parent_terminal").unwrap();
    tx.execute("UPDATE agent_runs SET status='terminal',terminal_state='completed',terminal_code='test_ordered_parent_terminal' WHERE id=?1",[&f.actor.root_run_id]).unwrap();
    tx.commit().unwrap();
    db.execute(
        "UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1,fencing_token=?1",
        [uuid::Uuid::new_v4().to_string()],
    )
    .unwrap();
    let changes = db.total_changes();
    let history = ordered_exec_projection(&db, &id);
    assert_eq!(history["completedAssessments"], 1);
    assert_eq!(history["state"], "deferred");
    assert_eq!(history["actions"][1]["state"], "not_started");
    assert_eq!(ordered_exec_receipts(&db, &id), original);
    assert_eq!(db.total_changes(), changes);
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert!(ordered_exec_apply(&f).is_err());
    assert_eq!(seen.lock().unwrap().len(), 1);
}
