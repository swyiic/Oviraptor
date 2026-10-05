fn directive_delivery_fixture(name: &str) -> (PathBuf, AgentRunContext, String, LiveProposalGuard) {
    live_proposal_fixture_limits(name, "请优先复核已有证据", (10_000, 20))
}

#[test]
fn directive_delivery_does_not_claim_application_before_a_model_request() {
    let (root, context, id, _original_parent) = directive_delivery_fixture("directive-not-applied");
    let inbox = take_human_directives(&context).unwrap();
    assert_eq!(inbox.items[0].id, id);
    let connection = db::open(&context.db_path).unwrap();
    let status: String = connection
        .query_row(
            "SELECT status FROM agent_user_directives WHERE id=?1",
            [&id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "accepted", "loading text is not an applied action");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn directive_delivery_recovers_a_claim_after_interrupted_collection() {
    use crate::agent_runtime::multi_agent::directive;
    let (root, context, id, _original_parent) = directive_delivery_fixture("directive-claim-recovery");
    let connection = db::open(&context.db_path).unwrap();
    let fence = native_human_directive_actor_on(&connection, &context).unwrap();
    directive::claim_pending_directives(&connection, &fence, 20).unwrap();
    let inbox = take_human_directives(&context).unwrap();
    assert_eq!(inbox.items[0].id, id);
    let status: String = connection
        .query_row(
            "SELECT status FROM agent_user_directives WHERE id=?1",
            [&id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        status, "accepted",
        "a durable claim must not become stranded"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn directive_delivery_rehydrates_accepted_text_without_claiming_completion() {
    let (root, context, id, _original_parent) = directive_delivery_fixture("directive-context-recovery");
    let first = take_human_directives(&context).unwrap();
    let recovered = take_human_directives(&context).unwrap();
    assert_eq!(first.items, recovered.items);
    assert_eq!(recovered.items[0].id, id);
    let messages = recovered.messages();
    let (compacted, _) = agent_compact_messages(&messages, 1);
    assert_eq!(
        compacted, messages,
        "compaction cannot shorten a human constraint"
    );
    let connection = db::open(&context.db_path).unwrap();
    finish_coordinator_run(&connection, recovered.lease.as_ref().unwrap(),
        &AgentTargetOutcome::BoundedCompleted(AgentCompletion::bounded("coverage fixture"))).unwrap();
    let status: String = connection.query_row("SELECT status FROM agent_user_directives WHERE id=?1", [&id], |r| r.get(0)).unwrap();
    assert_eq!(status, "deferred", "successful overall review is not a directive action receipt");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn directive_delivery_surfaces_missing_runs_and_storage_errors() {
    let (root, mut context, _, _original_parent) = directive_delivery_fixture("directive-storage-failure");
    context.run.as_mut().unwrap().run_id = "missing-run".into();
    assert!(take_human_directives(&context)
        .unwrap_err()
        .contains("web_mode_directive_scope_missing"));
    context.db_path = root.join("missing-parent/database.sqlite");
    assert!(take_human_directives(&context).is_err());
    context.run = None;
    assert!(
        take_human_directives(&context).unwrap().items.is_empty(),
        "standalone mode has no inbox"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn directive_delivery_receipt_event_and_timeline_commit_or_roll_back_together() {
    let (root, context, id, _original_parent) = directive_delivery_fixture("directive-delivery-atomic");
    let inbox = take_human_directives(&context).unwrap();
    let connection = db::open(&context.db_path).unwrap();
    let cursor: i64 = connection
        .query_row(
            "SELECT COALESCE(MAX(sequence),0) FROM agent_collaboration_events",
            [],
            |row| row.get(0),
        )
        .unwrap();
    connection.execute_batch(
        "CREATE TRIGGER fail_directive_delivery BEFORE INSERT ON agent_collaboration_events \
         WHEN NEW.event_type='user_directive' AND json_extract(NEW.payload_json,'$.deliveryState')='model_received' \
         BEGIN SELECT RAISE(ABORT,'delivery timeline unavailable'); END;",
    ).unwrap();
    let ledger = context.run.as_ref().unwrap();
    let error = ledger
        .model_round_with_directives(1, &AgentTokenUsage::default(), &[], &inbox)
        .unwrap_err();
    assert!(error.contains("delivery timeline unavailable"));
    let receipt: Option<String> = connection.query_row(
        "SELECT json_extract(payload_json,'$.modelDelivery.state') FROM agent_user_directives WHERE id=?1",
        [&id], |row| row.get(0),
    ).unwrap();
    assert!(receipt.is_none());
    let events: i64 = connection.query_row(
        "SELECT count(*) FROM agent_events WHERE run_id=?1 AND event_type='model_round_completed'",
        [&ledger.run_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(events, 0);
    connection
        .execute_batch("DROP TRIGGER fail_directive_delivery")
        .unwrap();
    ledger
        .model_round_with_directives(1, &AgentTokenUsage::default(), &[], &inbox)
        .unwrap();
    let (status, sequence, run): (String, i64, String) = connection
        .query_row(
            "SELECT status,json_extract(payload_json,'$.modelDelivery.eventSequence'),\
         json_extract(payload_json,'$.modelDelivery.runId') FROM agent_user_directives WHERE id=?1",
            [&id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(status, "accepted");
    let payload: String = connection
        .query_row(
            "SELECT payload_json FROM agent_events WHERE run_id=?1 AND sequence=?2",
            params![run, sequence],
            |row| row.get(0),
        )
        .unwrap();
    let payload: JsonValue = serde_json::from_str(&payload).unwrap();
    assert_eq!(payload["deliveredDirectiveIds"], serde_json::json!([id]));
    let projection = native_scan_status_after(&connection, &context.scan_id, Some(cursor)).unwrap();
    let timeline = projection["timeline"].as_array().unwrap();
    let message = timeline.iter().find(|message| message["id"] == id).unwrap();
    assert_eq!(message["deliveryState"], "model_received");
    assert_eq!(message["status"], "accepted");
    // A fresh collector still restores the instruction after a successful delivery.
    assert_eq!(take_human_directives(&context).unwrap().items, inbox.items);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn directive_delivery_old_lease_cannot_publish_a_receipt_and_is_visibly_deferred() {
    let (root, context, id, _original_parent) = directive_delivery_fixture("directive-delivery-fence");
    let inbox = take_human_directives(&context).unwrap();
    let connection = db::open(&context.db_path).unwrap();
    connection.execute(
        "UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1,fencing_token='new-owner' WHERE root_run_id=?1",
        [&context.run.as_ref().unwrap().run_id],
    ).unwrap();
    assert!(context
        .run
        .as_ref()
        .unwrap()
        .model_round_with_directives(1, &AgentTokenUsage::default(), &[], &inbox,)
        .is_err());
    let before = web_mode_test_rows(&connection);
    assert!(take_human_directives(&context).is_err());
    web_mode_assert_rows(&connection, &before);
    let (status, reason, receipt): (String, String, Option<String>) = connection
        .query_row(
            "SELECT status,rejection_code,json_extract(payload_json,'$.modelDelivery.state') \
         FROM agent_user_directives WHERE id=?1",
            [&id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(status, "accepted", "a replacement C cannot mutate the original inbox");
    assert_eq!(reason, "");
    assert!(receipt.is_none());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn directive_delivery_rejects_tampered_context_on_resume() {
    let (root, context, id, _original_parent) = directive_delivery_fixture("directive-delivery-integrity");
    take_human_directives(&context).unwrap();
    let connection = db::open(&context.db_path).unwrap();
    connection.execute("UPDATE agent_user_directives SET text_redacted='changed after confirmation' WHERE id=?1", [&id]).unwrap();
    assert!(take_human_directives(&context).unwrap().items.is_empty());
    let status: String = connection
        .query_row(
            "SELECT status FROM agent_user_directives WHERE id=?1",
            [&id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "rejected");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn directive_delivery_native_model_single_does_not_adopt_coordinator_instruction() {
    use crate::agent_runtime::multi_agent::lease;
    for successful in [false, true] {
        let mut harness = agent_harness(
            "directive-delivery-native",
            mock_site,
            vec![AgentIdentity::anonymous()],
        );
        harness.context.execution_plan.max_turns = 2;
        persist_agent_execution_plan(
            &harness.db_path,
            "agent-scan",
            1,
            &harness.context.target_url,
            &harness.context.execution_plan,
        )
        .unwrap();
        let ledger =
            runtime_open_run(&harness.db_path, "agent-scan", &harness.context.route).unwrap();
        harness.context.run = Some(ledger.clone());
        let connection = db::open(&harness.db_path).unwrap();
        let fence = lease::acquire_coordinator_lease(
            &connection,
            "agent-scan",
            1,
            &harness.context.target_url,
            &ledger.run_id,
            600,
        )
        .unwrap();
        // This lower-level delivery test owns its original financial fixture
        // before any SDK/Native state. It does not exercise live Root creation.
        let tx = connection.unchecked_transaction().unwrap();
        crate::agent_runtime::multi_agent::budget::root::RootOwner::initialize_financial_fixture_for_test(
            &tx, &ledger.run_id,
        ).unwrap();
        tx.commit().unwrap();
        let id = confirmed_directive_for_lease(&connection, &fence);
        let original: String = connection.query_row(
            "SELECT json_array(rowid,id,status,payload_json,claim_run_id,claimed_at,accepted_at,applied_at,finished_at,updated_at) FROM agent_user_directives WHERE id=?1",
            [&id], |r| r.get(0),
        ).unwrap();
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(move |_| {
            if successful {
                (200, "application/json", model_round(&[], 10))
            } else {
                (401, "application/json", r#"{"error":{"message":"test authentication failure","type":"authentication_error"}}"#.into())
            }
        }));
        harness.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
        let outcome = NativeAgentBackend.execute(&harness.context);
        let requests = seen.lock().unwrap();
        assert_eq!(
            requests.len(),
            if successful { 2 } else { 1 },
            "must exercise the actual model transport: {outcome:?}"
        );
        // Parse JSON rather than depending on the provider's Unicode escaping.
        for request in requests.iter() {
            let request: JsonValue =
                serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
            assert_eq!(
                request["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|message| {
                        message["content"].as_str().is_some_and(|text| {
                            text.contains(&id) && text.contains("请优先复核已有证据")
                        })
                    })
                    .count(),
                0,
                "Single cannot adopt a Coordinator inbox or message execution grant"
            );
        }
        let (status, receipt): (String, Option<String>) = connection.query_row(
            "SELECT status,json_extract(payload_json,'$.modelDelivery.state') FROM agent_user_directives WHERE id=?1",
            [&id], |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(status, "pending");
        assert!(receipt.is_none(), "a Single SDK response cannot acknowledge a Coordinator message");
        let saved: String = connection.query_row(
            "SELECT json_array(rowid,id,status,payload_json,claim_run_id,claimed_at,accepted_at,applied_at,finished_at,updated_at) FROM agent_user_directives WHERE id=?1",
            [&id], |r| r.get(0),
        ).unwrap();
        assert_eq!(saved, original, "the unowned directive must remain byte-for-byte untouched");
        let rounds: i64 = connection.query_row(
            "SELECT count(*) FROM agent_events WHERE run_id=?1 AND event_type='model_round_completed'",
            [&ledger.run_id], |row| row.get(0),
        ).unwrap();
        assert_eq!(rounds, if successful { 2 } else { 0 });
        drop(requests);
        let _ = std::fs::remove_dir_all(harness.root);
    }
}
