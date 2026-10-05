// Live model callee must not create finance on an old registered no-Mode Root.
#[test]
fn original_root_no_mode_missing_owner_actual_sdk_is_rejected_without_any_write() {
    let mut h = agent_harness(
        "old-no-mode-sdk",
        mock_site,
        vec![AgentIdentity::anonymous()],
    );
    freeze_harness_plan(&h);
    h.context.run = runtime_open_run(&h.db_path, &h.context.scan_id, &h.context.route);
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|_| {
        (200, "application/json", model_round(&[], 20))
    }));
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let client = AgentModelClient::new(
        agent_model_profile(&h.context.environment, None).unwrap(),
        &[],
    );
    let db = db::open(&h.db_path).unwrap();
    let before = web_mode_test_rows(&db);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_root_mode_definitions",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM agent_root_budget_attempts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let result = native_model_transport(
        &h.context,
        &client,
        vec![
            json!({"role":"user","content":"missing original finance never confers SDK authority"}),
        ],
        &[],
        1,
    );
    assert!(
        result.is_err(),
        "registered old no-Mode Native Root used pristine fallback to dispatch"
    );
    assert_eq!(
        seen.lock().unwrap().len(),
        0,
        "actual live SDK callee must refuse before provider I/O"
    );
    web_mode_assert_rows(&db, &before);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}

#[test]
fn original_root_no_mode_missing_owner_control_and_claim_do_not_mint_on_zero_usage() {
    use crate::agent_runtime::multi_agent::budget::root::{model::RootModelCall, RootOwner};
    for api in ["control", "claim"] {
        let mut h = agent_harness(
            "old-no-mode-control",
            mock_site,
            vec![AgentIdentity::anonymous()],
        );
        freeze_harness_plan(&h);
        h.context.run = runtime_open_run(&h.db_path, &h.context.scan_id, &h.context.route);
        let db = db::open(&h.db_path).unwrap();
        let root = &h.context.run.as_ref().unwrap().run_id;
        let before = web_mode_test_rows(&db);
        let result = (|| -> Result<(), String> {
            let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
            if api == "control" {
                RootOwner::initialize_control(&tx, root)?;
            } else {
                RootModelCall::claim(&tx, root, 1, &"a".repeat(64), 1000)?;
            }
            tx.commit().map_err(|e| e.to_string())
        })();
        assert!(
            result.is_err(),
            "{api}: original authority cannot depend on Mode sidecar existence"
        );
        web_mode_assert_rows(&db, &before);
        assert_eq!(h.model_seen.lock().unwrap().len(), 0);
        drop(db);
        fs::remove_dir_all(h.root).unwrap();
    }
}
