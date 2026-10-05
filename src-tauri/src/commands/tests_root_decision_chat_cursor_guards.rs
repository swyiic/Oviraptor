// Existing production tick/status APIs, actual localhost provider; no projection shim.
#[test]
fn root_decision_chat_mapping_ignore_and_cross_lane_trigger_preserve_paid_bill() {
    for action in ["SELECT RAISE(IGNORE)","UPDATE projects SET name=name||'-mapping'",
        "INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json) VALUES('foreign',91,'root_decision','alias','root_decision','{}')"] {
        let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(|_|(200,"application/json",proposal_model_response(&root_tick_valid_text("paid mapping fact")))));
        let f=root_tick_fixture("chat-mapping-final",&format!("http://127.0.0.1:{port}/v1"));
        let db=db::open(&f.context.db_path).unwrap();
        let projects:Vec<(i64,String)>=db.prepare("SELECT id,name FROM projects ORDER BY id").unwrap()
            .query_map([],|r|Ok((r.get(0)?,r.get(1)?))).unwrap().map(Result::unwrap).collect();
        db.execute_batch(&format!("CREATE TRIGGER root_chat_mapping_fault BEFORE INSERT ON agent_root_tick_timeline_receipts BEGIN {action}; END;")).unwrap();
        assert!(native_coordinator_tick(&f.context,&f.actor).is_err(),"mapping fault accepted: {action}");
        assert_eq!(root_tick_count(&db,&f.actor.root_run_id,"decision"),1);
        assert_eq!(root_tick_count(&db,&f.actor.root_run_id,"publication"),0);
        assert_eq!(db.query_row("SELECT count(*) FROM agent_root_tick_timeline_receipts",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert_eq!(db.query_row("SELECT count(*) FROM agent_collaboration_events WHERE event_type='root_decision'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        let after:Vec<(i64,String)>=db.prepare("SELECT id,name FROM projects ORDER BY id").unwrap()
            .query_map([],|r|Ok((r.get(0)?,r.get(1)?))).unwrap().map(Result::unwrap).collect();
        assert_eq!(projects,after); assert_eq!(seen.lock().unwrap().len(),1);
        db.execute_batch("DROP TRIGGER root_chat_mapping_fault;").unwrap();
        let paid=native_coordinator_tick(&f.context,&f.actor).unwrap();
        assert!(paid.replayed); assert_eq!(seen.lock().unwrap().len(),1);
        assert_eq!(root_tick_count(&db,&f.actor.root_run_id,"publication"),1);
    }
}

#[test]
fn root_decision_chat_replace_all_receipt_unique_keys_and_original_event_cursor_is_blocked() {
    let (port, _, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            proposal_model_response(&root_tick_valid_text("immutable chat cursor")),
        )
    }));
    let mut f = root_tick_fixture("chat-no-replace", &format!("http://127.0.0.1:{port}/v1"));
    native_coordinator_tick(&f.context, &f.actor).unwrap();
    drop(f.parent.take());
    let db = db::open(&f.context.db_path).unwrap();
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM agent_root_tick_timeline_receipts",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 1,
        "actual published Root has no immutable chat cursor binding"
    );
    db.pragma_update(None, "recursive_triggers", "OFF").unwrap();
    let before = super::tests::application_table_snapshot(&db);
    // Each fixture collides exactly one of the mapping table's unique keys.
    for key in ["call", "cursor", "entity", "root_event"] {
        let call = if key == "call" {
            "call_id"
        } else {
            "printf('%064d',9)"
        };
        let cursor = if key == "cursor" {
            "collaboration_sequence"
        } else {
            "collaboration_sequence+100000"
        };
        let entity = if key == "entity" {
            "entity_id"
        } else {
            "'independent-test-entity'"
        };
        let event = if key == "root_event" {
            "model_event_sequence"
        } else {
            "model_event_sequence+100000"
        };
        let sql=format!("INSERT OR REPLACE INTO agent_root_tick_timeline_receipts
            (call_id,root_run_id,lease_attempt_id,model_event_sequence,collaboration_sequence,scan_id,attempt_number,entity_id,payload_json,event_created_at)
            SELECT {call},root_run_id,lease_attempt_id,{event},{cursor},scan_id,attempt_number,{entity},payload_json,event_created_at
            FROM agent_root_tick_timeline_receipts LIMIT 1");
        assert!(
            db.execute_batch(&sql).is_err(),
            "REPLACE unique key accepted: {key}"
        );
        assert_eq!(super::tests::application_table_snapshot(&db), before);
    }
    for sql in ["INSERT OR REPLACE INTO agent_collaboration_events(sequence,scan_id,attempt_number,entity_type,entity_id,event_type,payload_json,created_at)
            SELECT collaboration_sequence,'foreign',92,'other','other','other','{}',event_created_at FROM agent_root_tick_timeline_receipts",
        "INSERT OR REPLACE INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json)
            SELECT scan_id,attempt_number,'root_decision',entity_id,'root_decision',payload_json FROM agent_root_tick_timeline_receipts"] {
        assert!(db.execute_batch(sql).is_err(),"original collaboration identity replaced");
        assert_eq!(super::tests::application_table_snapshot(&db),before);
    }
    assert!(native_scan_status(&db, &f.context.scan_id).is_ok());
}

#[test]
fn root_decision_chat_foreign_scope_or_changed_channel_cannot_hide_behind_page_cursor() {
    for assignment in [
        "scan_id='foreign'",
        "attempt_number=93",
        "payload_json='{}'",
        "entity_type='other'",
    ] {
        let (port, _, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
            (
                200,
                "application/json",
                proposal_model_response(&root_tick_valid_text("original scoped decision")),
            )
        }));
        let mut f = root_tick_fixture("chat-read-fence", &format!("http://127.0.0.1:{port}/v1"));
        native_coordinator_tick(&f.context, &f.actor).unwrap();
        drop(f.parent.take());
        let db = db::open(&f.context.db_path).unwrap();
        let count: i64 = db
            .query_row(
                "SELECT count(*) FROM agent_root_tick_timeline_receipts",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "missing actual cursor receipt before tamper");
        let latest: i64 = db
            .query_row(
                "SELECT max(sequence) FROM agent_collaboration_events WHERE scan_id=?1",
                [&f.context.scan_id],
                |r| r.get(0),
            )
            .unwrap();
        db.execute_batch(&format!(
            "UPDATE agent_collaboration_events SET {assignment} WHERE event_type='root_decision';"
        ))
        .unwrap();
        let before = super::tests::application_table_snapshot(&db);
        assert!(
            native_scan_status(&db, &f.context.scan_id).is_err(),
            "altered channel accepted: {assignment}"
        );
        assert!(native_scan_status_for_attempt(
            &db,
            &f.context.scan_id,
            Some(latest + 100),
            Some(1)
        )
        .is_err());
        assert!(native_scan_timeline_page(&db, &f.context.scan_id, 1, 1).is_err());
        assert_eq!(
            super::tests::application_table_snapshot(&db),
            before,
            "reader repaired or authorized damaged original rows"
        );
    }
}

#[test]
fn root_decision_chat_real_wal_snapshot_uses_matching_cursor_and_paid_records() {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            proposal_model_response(&root_tick_valid_text("WAL publication")),
        )
    }));
    let mut f = root_tick_fixture("chat-wal", &format!("http://127.0.0.1:{port}/v1"));
    let db = db::open(&f.context.db_path).unwrap();
    let old = db.unchecked_transaction().unwrap();
    let _: i64 = old
        .query_row(
            "SELECT attempt_count FROM sentinel_scans WHERE id=?1",
            [&f.context.scan_id],
            |r| r.get(0),
        )
        .unwrap();
    // Separate actual production writer/SDK commits after the WAL read snapshot.
    native_coordinator_tick(&f.context, &f.actor).unwrap();
    let window = TimelineWindow::latest(&old, &f.context.scan_id, 1, None).unwrap();
    let (rows, watermark) =
        native_status_timeline(&old, &f.context.scan_id, 1, &window, &json!({}), None).unwrap();
    assert!(!rows.iter().any(|row| row["eventType"] == "root_decision"));
    assert_eq!(watermark, window.watermark);
    old.commit().unwrap();
    drop(f.parent.take());
    let before = super::tests::application_table_snapshot(&db);
    let status = native_scan_status(&db, &f.context.scan_id).unwrap();
    let row = status["timeline"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["eventType"] == "root_decision")
        .expect("new public snapshot missed actual SDK publication");
    assert!(row["sequence"].as_i64().unwrap() <= status["latestSequence"].as_i64().unwrap());
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn root_decision_chat_channel_trigger_alias_and_preexisting_fake_entity_stop_publication() {
    for fault in ["trigger_alias", "preexisting_entity"] {
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
            (
                200,
                "application/json",
                proposal_model_response(&root_tick_valid_text("alias guard")),
            )
        }));
        let f = root_tick_fixture("chat-alias", &format!("http://127.0.0.1:{port}/v1"));
        let db = db::open(&f.context.db_path).unwrap();
        if fault == "trigger_alias" {
            db.execute_batch("CREATE TRIGGER chat_alias AFTER INSERT ON agent_collaboration_events WHEN NEW.event_type='root_decision'
                BEGIN INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json)
                VALUES('foreign',94,NEW.entity_type,NEW.entity_id,NEW.event_type,NEW.payload_json); END;").unwrap();
        } else {
            let next: i64 = db
                .query_row(
                    "SELECT COALESCE(max(sequence),0)+1 FROM agent_events WHERE run_id=?1",
                    [&f.actor.root_run_id],
                    |r| r.get(0),
                )
                .unwrap();
            db.execute("INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json)
                VALUES('foreign',95,'root_decision',?1,'root_decision','{}')",
                [json!([f.actor.root_run_id,next]).to_string()]).unwrap();
        }
        assert!(
            native_coordinator_tick(&f.context, &f.actor).is_err(),
            "{fault}: alias changed the real committed cursor"
        );
        assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "decision"), 1);
        assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 0);
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM agent_root_tick_timeline_receipts",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(seen.lock().unwrap().len(), 1);
    }
}
