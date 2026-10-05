// Actual paid Root SDK; ordinary own-event metadata edits cannot be adopted.
#[test]
fn coordinator_tick_publication_event_physical_metadata_is_original_and_replay_readonly() {
    for field in ["id", "created_at"] {
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
            (
                200,
                "application/json",
                proposal_model_response(&root_tick_valid_text("original public fact")),
            )
        }));
        let f = root_tick_fixture(
            "tick-event-physical",
            &format!("http://127.0.0.1:{port}/v1"),
        );
        let db = db::open(&f.context.db_path).unwrap();
        let original = native_coordinator_tick(&f.context, &f.actor).unwrap();
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 1);
        let (id, created): (i64, String) = db
            .query_row(
                "SELECT id,created_at FROM agent_events WHERE run_id=?1 AND sequence=?2",
                params![f.actor.root_run_id, original.event_sequence],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        if field == "id" {
            assert_eq!(
                db.execute(
                    "UPDATE agent_events SET id=id+1000000 WHERE run_id=?1 AND sequence=?2",
                    params![f.actor.root_run_id, original.event_sequence],
                )
                .unwrap(),
                1
            );
        } else {
            assert_eq!(db.execute(
                "UPDATE agent_events SET created_at='2000-01-01 00:00:00' WHERE run_id=?1 AND sequence=?2",
                params![f.actor.root_run_id, original.event_sequence],
            ).unwrap(), 1);
        }
        let changed = web_mode_test_rows(&db);
        let replay = native_coordinator_tick(&f.context, &f.actor);
        assert!(
            replay.is_err(),
            "{field}: original paid publication must reject changed physical event"
        );
        assert_eq!(
            seen.lock().unwrap().len(),
            1,
            "{field}: paid replay must use zero SDK"
        );
        web_mode_assert_rows(&db, &changed);
        assert_eq!(
            db.execute(
                "UPDATE agent_events SET id=?1,created_at=?2 WHERE run_id=?3 AND sequence=?4",
                params![id, created, f.actor.root_run_id, original.event_sequence],
            )
            .unwrap(),
            1
        );
        let restored = web_mode_test_rows(&db);
        let replay = native_coordinator_tick(&f.context, &f.actor).unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.event_sequence, original.event_sequence);
        assert_eq!(replay.summary, original.summary);
        assert_eq!(seen.lock().unwrap().len(), 1);
        web_mode_assert_rows(&db, &restored);
    }
}

#[test]
fn coordinator_tick_old_publication_without_event_proof_is_not_repaired_or_reissued() {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            proposal_model_response(&root_tick_valid_text("historical public fact")),
        )
    }));
    let f = root_tick_fixture(
        "tick-old-event-proof",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let db = db::open(&f.context.db_path).unwrap();
    native_coordinator_tick(&f.context, &f.actor).unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1);
    // Isolated historical-format corruption fixture: this is not a normal
    // production write or the feature red. Restore the exact trigger afterwards.
    let immutable: String = db
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='trigger' AND name='root_tick_no_update'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    db.execute_batch("DROP TRIGGER root_tick_no_update;")
        .unwrap();
    assert_eq!(db.execute(
        "UPDATE agent_root_tick_receipts SET fact_json=json_remove(fact_json,'$.eventProof') WHERE root_run_id=?1 AND phase='publication'",
        [&f.actor.root_run_id],
    ).unwrap(), 1);
    db.execute_batch(&immutable).unwrap();
    let before = web_mode_test_rows(&db);
    assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
    assert_eq!(seen.lock().unwrap().len(), 1);
    web_mode_assert_rows(&db, &before);
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "request"), 1);
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "decision"), 1);
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 1);
}
