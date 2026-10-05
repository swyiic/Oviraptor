// TESTS ONLY: include after g2/g3 Root fixture helpers in commands/agent_tests.rs.
// Not executed in the child task. Missing g2 compilation is not a behavioral red.

#[test]
fn root_decision_chat_actual_sdk_has_committed_attempt_cursor_and_readonly_replay() {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            proposal_model_response(&root_tick_valid_text("saved public observation")),
        )
    }));
    let mut f = root_tick_fixture(
        "decision-chat-cursor",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let db = db::open(&f.context.db_path).unwrap();
    let cursor = native_scan_status(&db, &f.context.scan_id).unwrap()["latestSequence"]
        .as_i64()
        .unwrap();
    let receipt = native_coordinator_tick(&f.context, &f.actor).unwrap();
    let replay = native_coordinator_tick(&f.context, &f.actor).unwrap();
    assert_eq!(replay.event_sequence, receipt.event_sequence);
    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "read/replay cannot call the SDK again"
    );
    drop(f.parent.take());
    let before = super::tests::application_table_snapshot(&db);
    let delta =
        native_scan_status_for_attempt(&db, &f.context.scan_id, Some(cursor), Some(1)).unwrap();
    let rows: Vec<_> = delta["timeline"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["eventType"] == "root_decision")
        .collect();
    assert_eq!(
        rows.len(),
        1,
        "actual committed ModelRoundCompleted was not projected into chat"
    );
    let item = rows[0];
    assert_eq!(item["fromRunId"], f.actor.root_run_id);
    assert_eq!(item["targetKey"], f.actor.target_key);
    assert_eq!(
        item["decisionRecord"]["modelEventSequence"],
        receipt.event_sequence
    );
    assert_eq!(item["decisionRecord"]["advisoryOnly"], true);
    assert_eq!(item["decisionRecord"]["summary"], receipt.summary.as_json());
    assert_eq!(item["decisionRecord"]["usage"]["modelRequests"], 1);
    let saved_cursor = item["sequence"].as_i64().unwrap();
    assert!(saved_cursor > cursor && saved_cursor <= delta["latestSequence"].as_i64().unwrap());
    let latest =
        native_scan_status_for_attempt(&db, &f.context.scan_id, Some(saved_cursor), Some(1))
            .unwrap();
    assert!(!latest["timeline"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["eventType"] == "root_decision"));
    let history = native_scan_timeline_page(&db, &f.context.scan_id, 1, saved_cursor + 1).unwrap();
    assert!(history["timeline"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row == item));
    assert_eq!(
        super::tests::application_table_snapshot(&db),
        before,
        "read must not backfill collaboration rows or authority"
    );
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn root_decision_chat_reader_refuses_altered_original_event_and_preserves_all_rows() {
    let (port, _, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            proposal_model_response(&root_tick_valid_text("original semantic receipt")),
        )
    }));
    let mut f = root_tick_fixture(
        "decision-chat-integrity",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let db = db::open(&f.context.db_path).unwrap();
    let receipt = native_coordinator_tick(&f.context, &f.actor).unwrap();
    drop(f.parent.take());
    db.execute(
        "UPDATE agent_events SET payload_json='{}' WHERE run_id=?1 AND sequence=?2",
        params![f.actor.root_run_id, receipt.event_sequence],
    )
    .unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert!(native_scan_status(&db, &f.context.scan_id).is_err(),
        "chat must verify the original publication/decision/request/invoice, not trust the new channel payload");
    assert_eq!(super::tests::application_table_snapshot(&db), before);
}

#[test]
fn root_decision_chat_collaboration_ignore_or_business_trigger_rolls_back_publication_only() {
    for fault in ["ignore", "business"] {
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
            (
                200,
                "application/json",
                proposal_model_response(&root_tick_valid_text("safe semantic receipt")),
            )
        }));
        let mut f = root_tick_fixture(
            "decision-chat-final-write",
            &format!("http://127.0.0.1:{port}/v1"),
        );
        let db = db::open(&f.context.db_path).unwrap();
        let action = if fault == "ignore" {
            "SELECT RAISE(IGNORE)"
        } else {
            "UPDATE projects SET name=name||'-collateral'"
        };
        db.execute_batch(&format!(
            "CREATE TRIGGER decision_channel_fault BEFORE INSERT ON agent_collaboration_events
            WHEN NEW.event_type='root_decision' BEGIN {action}; END;"
        ))
        .unwrap();
        let project_rows: Vec<(i64, String)> = db
            .prepare("SELECT id,name FROM projects ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert!(
            native_coordinator_tick(&f.context, &f.actor).is_err(),
            "{fault}: skipped or collateral channel write accepted"
        );
        assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "decision"), 1);
        assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 0);
        assert_eq!(db.query_row("SELECT count(*) FROM agent_events WHERE run_id=?1 AND event_type='model_round_completed'",[&f.actor.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert_eq!(
            seen.lock().unwrap().len(),
            1,
            "paid response cannot be replayed at the provider"
        );
        let after: Vec<(i64, String)> = db
            .prepare("SELECT id,name FROM projects ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(after, project_rows);
        drop(f.parent.take());
    }
}

#[test]
fn root_decision_chat_unknown_bill_and_new_attempt_never_gain_current_decision_status() {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        let mut response: JsonValue = serde_json::from_str(&proposal_model_response(
            &root_tick_valid_text("unsettled observation"),
        ))
        .unwrap();
        response.as_object_mut().unwrap().remove("usage");
        (200, "application/json", response.to_string())
    }));
    let mut f = root_tick_fixture(
        "decision-chat-unknown",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
    drop(f.parent.take());
    let db = db::open(&f.context.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let value = native_scan_status(&db, &f.context.scan_id).unwrap();
    assert!(!value["timeline"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["eventType"] == "root_decision"));
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(seen.lock().unwrap().len(), 1);
    // Fault injection affects only this newly created isolated fixture, never user data.
    db.execute(
        "UPDATE sentinel_scans SET attempt_count=2 WHERE id=?1",
        [&f.context.scan_id],
    )
    .unwrap();
    assert!(native_scan_timeline_page(
        &db,
        &f.context.scan_id,
        1,
        value["latestSequence"].as_i64().unwrap() + 1
    )
    .is_err());
}
