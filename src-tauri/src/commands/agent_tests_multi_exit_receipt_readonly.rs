fn multi_exit_receipt_read(db: &rusqlite::Connection, root: &str) -> Result<(), String> {
    let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
    crate::agent_runtime::multi_agent::budget::clock::FinalClock::verify_original_exit(&tx, root)
}

#[test]
fn multi_exit_receipt_actual_paid_finish_and_replay_preserve_original_sources() {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            sdk_exact_model_body(&root_tick_valid_text("durable financial closure")),
        )
    }));
    let f = root_tick_fixture(
        "multi-exit-paid-original",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    native_coordinator_tick(&f.context, &f.actor).unwrap();
    let db = db::open(&f.context.db_path).unwrap();
    let outcome = AgentTargetOutcome::incomplete("original partial paid exit");
    finish_coordinator_run(&db, &f.actor, &outcome).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let file = fs::read(f.context.target_dir.join("frontend-evidence.json")).unwrap();
    let row: (String,String,String) = db.query_row("SELECT receipt_id,control_id,fact_json FROM agent_multi_exit_receipts WHERE root_run_id=?1",[&f.actor.root_run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    let value: JsonValue = serde_json::from_str(&row.2).unwrap();
    assert_eq!(value["receiptId"], row.0);
    assert_eq!(value["control"], row.1);
    assert_eq!(value["dimensions"].as_array().unwrap().len(), 10);
    for _ in 0..2 {
        multi_exit_receipt_read(&db, &f.actor.root_run_id).unwrap();
        finish_coordinator_run(&db, &f.actor, &outcome).unwrap();
        assert!(super::tests::application_table_snapshot(&db) == before);
    }
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(
        fs::read(f.context.target_dir.join("frontend-evidence.json")).unwrap(),
        file
    );
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::clock::FinalClock::verify_original_exit(
            &db,
            &f.actor.root_run_id
        )
        .unwrap_err(),
        "budget_multi_exit_requires_snapshot"
    );
    assert!(super::tests::application_table_snapshot(&db) == before);
}

#[test]
fn multi_exit_receipt_actual_paid_readonly_after_original_c_expires_grants_no_sdk() {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            sdk_exact_model_body(&root_tick_valid_text("expired C original fact")),
        )
    }));
    let mut f = root_tick_fixture_protocol_limits_timeout(
        "multi-exit-original-expiry",
        &format!("http://127.0.0.1:{port}/v1"),
        true,
        (60_000, 20),
        Some(4),
    );
    native_coordinator_tick(&f.context, &f.actor).unwrap();
    let db = db::open(&f.context.db_path).unwrap();
    let outcome = AgentTargetOutcome::incomplete("original C closes once");
    finish_coordinator_run(&db, &f.actor, &outcome).unwrap();
    drop(f.parent.take());
    let before = super::tests::application_table_snapshot(&db);
    let deadline = std::time::Instant::now() + Duration::from_secs(6);
    while db.query_row("SELECT lease_expires_at>datetime('now','localtime') FROM agent_coordinator_leases WHERE root_run_id=?1",[&f.actor.root_run_id],|r|r.get::<_,bool>(0)).unwrap() {
        assert!(std::time::Instant::now()<deadline,"original C did not expire within the declared fixture ceiling");
        std::thread::sleep(Duration::from_millis(50));
    }
    let readonly = rusqlite::Connection::open_with_flags(
        &f.context.db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    multi_exit_receipt_read(&readonly, &f.actor.root_run_id).unwrap();
    assert!(
        crate::agent_runtime::multi_agent::lease::validate_coordinator_lease(&db, &f.actor)
            .is_err()
    );
    assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert!(super::tests::application_table_snapshot(&db) == before);
}

#[test]
fn multi_exit_receipt_actual_unknown_cost_closes_finance_without_settling_or_deleting() {
    for missing_usage in [false, true] {
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(move |_| {
            if missing_usage {
                let mut v: JsonValue = serde_json::from_str(&sdk_exact_model_body(
                    &root_tick_valid_text("usage missing"),
                ))
                .unwrap();
                v.as_object_mut().unwrap().remove("usage");
                (200, "application/json", v.to_string())
            } else {
                (
                    503,
                    "application/json",
                    r#"{"error":{"message":"original unknown cost"}}"#.into(),
                )
            }
        }));
        let mut f = root_tick_fixture(
            "multi-exit-unknown-cost",
            &format!("http://127.0.0.1:{port}/v1"),
        );
        assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
        let db = db::open(&f.context.db_path).unwrap();
        let outcome = AgentTargetOutcome::incomplete("unknown original bill stays unsettled");
        finish_coordinator_run(&db, &f.actor, &outcome).unwrap();
        drop(f.parent.take());
        let before = super::tests::application_table_snapshot(&db);
        multi_exit_receipt_read(&db, &f.actor.root_run_id).unwrap();
        finish_coordinator_run(&db, &f.actor, &outcome).unwrap();
        assert!(super::tests::application_table_snapshot(&db) == before);
        let text: String = db
            .query_row(
                "SELECT fact_json FROM agent_multi_exit_receipts WHERE root_run_id=?1",
                [&f.actor.root_run_id],
                |r| r.get(0),
            )
            .unwrap();
        let value: JsonValue = serde_json::from_str(&text).unwrap();
        assert!(value["dimensions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["indeterminate"].as_i64().unwrap() > 0));
        assert!(
            crate::agent_runtime::multi_agent::budget::admission::require_settled_for_completion(
                &db,
                &f.actor.root_run_id
            )
            .is_err()
        );
        // Deliberately supplied old UI status cannot turn financial exit into deletion authority.
        db.execute(
            "UPDATE sentinel_scans SET status='paused' WHERE id=?1",
            [&f.actor.scan_id],
        )
        .unwrap();
        let closed = super::tests::application_table_snapshot(&db);
        assert!(delete_sentinel_scan_inner(&f.context.db_path, &f.actor.scan_id).is_err());
        assert!(super::tests::application_table_snapshot(&db) == closed);
        assert_eq!(seen.lock().unwrap().len(), 1);
    }
}
