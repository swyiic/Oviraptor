#[test]
fn coordinator_tick_original_evidence_bytes_changed_during_actual_sdk_preserves_bill_not_publication(
) {
    let file = std::sync::Arc::new(std::sync::Mutex::new(None::<PathBuf>));
    let writer = file.clone();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |_| {
        let path = writer.lock().unwrap().clone().unwrap();
        let bytes = fs::read(&path).unwrap();
        let value: JsonValue = serde_json::from_slice(&bytes).unwrap();
        fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
        (
            200,
            "application/json",
            proposal_model_response(&root_tick_valid_text("original frozen semantic facts")),
        )
    }));
    let f = root_tick_fixture(
        "tick-original-bytes",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let evidence = f.context.target_dir.join("frontend-evidence.json");
    let original = fs::read(&evidence).unwrap();
    *file.lock().unwrap() = Some(evidence.clone());
    let db = db::open(&f.context.db_path).unwrap();
    let plan: String = db
        .query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [&f.actor.root_run_id],
            |r| r.get(0),
        )
        .unwrap();
    let result = native_coordinator_tick(&f.context, &f.actor);
    assert!(matches!(result,Err(ref code) if code=="root_tick_original_basis_changed"),
        "actual SDK changed original evidence bytes; semantically equal JSON must not silently replace the captured basis");
    assert_eq!(seen.lock().unwrap().len(), 1);
    let changed = fs::read(&evidence).unwrap();
    assert_ne!(changed, original);
    assert_eq!(
        serde_json::from_slice::<JsonValue>(&changed).unwrap(),
        serde_json::from_slice::<JsonValue>(&original).unwrap()
    );
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "request"), 1);
    assert_eq!(
        root_tick_count(&db, &f.actor.root_run_id, "decision"),
        1,
        "retain validated original semantics alongside its fee"
    );
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 0);
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &f.actor.root_run_id,
            Some(""),
            "model_requests"
        )
        .unwrap()
        .consumed,
        1
    );
    let received:i64=db.query_row("SELECT COUNT(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='received'",[&f.actor.root_run_id],|r|r.get(0)).unwrap();
    assert_eq!(received, 1);
    fs::write(&evidence, &original).unwrap();
    let restored = native_coordinator_tick(&f.context, &f.actor).unwrap();
    assert!(restored.replayed);
    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "restore uses the original paid receipt, never a second provider request"
    );
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 1);
    let after: String = db
        .query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [&f.actor.root_run_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(after, plan, "original Native plan bytes unchanged");
}
