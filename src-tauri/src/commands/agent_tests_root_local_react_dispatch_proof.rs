#[test]
fn root_local_react_actual_final_authority_rechecks_original_local_publication() {
    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = count.clone();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |_| {
        let turn = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let body = if turn == 0 {
            root_local_wire(
                json!([root_local_call(
                    "call-dispatch-proof",
                    "snapshot.read",
                    json!({})
                )]),
                "",
                true,
            )
        } else {
            proposal_model_response(&root_tick_valid_text(
                "final paid decision must bind original local record",
            ))
        };
        (200, "application/json", body)
    }));
    let f = root_tick_fixture(
        "root-local-last-proof",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let db = db::open(&f.context.db_path).unwrap();
    let final_receipt = native_coordinator_tick(&f.context, &f.actor).unwrap();
    assert_eq!(seen.lock().unwrap().len(), 2);
    final_receipt.tick.require_executable(&db).unwrap();
    let sequence:i64=db.query_row("SELECT json_extract(fact_json,'$.eventSequence') FROM agent_root_tick_receipts WHERE root_run_id=?1 AND round=1 AND phase='publication'",[&f.actor.root_run_id],|r|r.get(0)).unwrap();
    db.execute(
        "UPDATE agent_events SET created_at=created_at||' changed' WHERE run_id=?1 AND sequence=?2",
        params![f.actor.root_run_id, sequence],
    )
    .unwrap();
    let before = web_mode_test_rows(&db);
    assert!(final_receipt.tick.require_executable(&db).is_err(),"the actual dispatch guard must reject a changed earlier local publication even after the final SDK completed");
    assert!(final_receipt
        .tick
        .published(&db, &final_receipt.saved)
        .is_err());
    assert_eq!(seen.lock().unwrap().len(), 2);
    web_mode_assert_rows(&db, &before);
}
