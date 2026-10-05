// Actual bounded SDK → original paid receipt → actual saved-mailbox authority.
// The positive is RED specifically at current Client saved-message role routing.
#[test]
fn client_side_actual_paid_receipt_saved_mailbox_binds_original_task_summary_and_correlation() {
    use crate::agent_runtime::{
        contract::AgentRole,
        multi_agent::{mailbox, scheduler},
    };
    let (directory, path, _, lease, mut context, slice) =
        client_readonly_negative_fixture("client-paid-saved-route");
    let raw = r#"{"summary":"原本地配置观察；尚缺浏览器行为","observationRefs":["csp-1"],"gaps":["missing_browser_validation"],"candidates":[]}"#;
    let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(move |_| {
        (200, "application/json", proposal_model_response(raw))
    }));
    let _guard = ClientSideReadonlyTestGuard {
        root: directory,
        stop,
    };
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let db = db::open(&path).unwrap();
    let child = scheduler::prepare_readonly_child(
        &db,
        &lease,
        AgentRole::ClientSide,
        "client_side_frozen_observations_ready",
        &slice,
        8000,
    )
    .unwrap();
    let (text, usage) = multi_agent_child_round_transport(
        &context,
        &lease,
        &child,
        "独立只读本地配置，严格JSON，无工具",
        slice.clone(),
    )
    .unwrap();
    let worker: String = db
        .query_row(
            "SELECT id FROM agent_assignment_attempts WHERE child_run_id=?1",
            [&child.run_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(uuid::Uuid::parse_str(&worker).is_ok());
    assert!(db.query_row("SELECT EXISTS(SELECT 1 FROM agent_budget_entries WHERE root_run_id=?1
        AND assignment_id=?2 AND lease_attempt_id=?3 AND dimension='model_requests' AND kind='consume' AND amount=1)",
        params![lease.root_run_id,child.assignment_id,worker],|r|r.get::<_,bool>(0)).unwrap());
    let original = json!({"summary":text,"clientSideTask":slice});
    let correlation = format!("client-side:{}", child.assignment_id);
    let before = receipt_database_snapshot(&db);
    for fault in [
        "changed_task",
        "changed_summary",
        "extra_claim",
        "changed_correlation",
    ] {
        let mut payload = original.clone();
        let mut key = correlation.clone();
        match fault {
            "changed_task" => {
                payload["clientSideTask"]["observations"][0]["value"] = json!("invented config")
            }
            "changed_summary" => payload["summary"] = json!("invented response"),
            "extra_claim" => payload["confirmed"] = json!(true),
            _ => key = "client-side:other-assignment".into(),
        }
        let error = {
            let tx =
                rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
                    .unwrap();
            mailbox::send_saved_specialist(
                &tx,
                &lease,
                &child.run_id,
                &lease.root_run_id,
                "client_side",
                "coordinator",
                "evidence_summary",
                &key,
                &child.assignment_id,
                1,
                &payload,
            )
            .unwrap_err()
        };
        assert!(
            error.contains("mailbox_saved_result_mismatch"),
            "{fault}: {error}"
        );
        assert_eq!(
            receipt_database_snapshot(&db),
            before,
            "{fault}: actual original rows/fees/UUID/rowid preserved"
        );
        assert_eq!(seen.lock().unwrap().len(), 1);
    }
    // Calling the existing actual API with the original received result must
    // pass. Roll back this standalone probe; Stage01 proves durable ACK/closure.
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let message = mailbox::send_saved_specialist(
        &tx,
        &lease,
        &child.run_id,
        &lease.root_run_id,
        "client_side",
        "coordinator",
        "evidence_summary",
        &correlation,
        &child.assignment_id,
        1,
        &original,
    )
    .unwrap();
    assert!(uuid::Uuid::parse_str(&message).is_ok());
    assert_eq!(db.query_row("SELECT count(*) FROM agent_messages WHERE id=?1 AND from_run_id=?2 AND to_run_id=?3
        AND from_agent='client_side' AND to_agent='coordinator' AND kind='evidence_summary' AND delivery_attempts=0",
        params![message,child.run_id,lease.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),1);
    tx.rollback().unwrap();
    assert_eq!(receipt_database_snapshot(&db), before);
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert!(usage.model_requests == 1);
}
