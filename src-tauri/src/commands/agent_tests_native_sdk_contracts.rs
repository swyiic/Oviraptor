// Original Root claim uses existing production budgeting, never a mock grant.
fn sdk_owned_contract(case: &str) {
    let h = root_budget_owned_fixture_for_test(&format!("sdk-contract-{case}"));
    let db = db::open(&h.db_path).unwrap();
    let root = &h.context.run.as_ref().unwrap().run_id;
    let native = db
        .query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [root],
            |r| r.get::<_, String>(0),
        )
        .unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let call = crate::agent_runtime::multi_agent::budget::root::model::RootModelCall::claim(
        &tx,
        root,
        1,
        &"a".repeat(64),
        1000,
    )
    .unwrap();
    tx.commit().unwrap();
    if matches!(
        case,
        "wal"
            | "scopes"
            | "empty_corrupt_owner"
            | "corrupt_cost_fact"
            | "gap"
            | "original_root_contract"
    ) {
        crate::agent_runtime::model::diagnostics::contract_tests::read_contract(
            &h.db_path, root, case,
        );
    } else {
        crate::agent_runtime::model::diagnostics::contract_tests::write_contract(
            &h.db_path, root, &call, case,
        );
    }
    if case != "original_root_contract" {
        assert_eq!(
            db.query_row(
                "SELECT plan_json FROM agent_runs WHERE id=?1",
                [root],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            native
        );
    }
    assert!(h.site_seen.lock().unwrap().is_empty());
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}
#[test]
fn native_sdk_log_guard_owner_trigger_full_application_rollback() {
    sdk_owned_contract("owner_trigger");
}
#[test]
fn native_sdk_log_guard_row_trigger_full_application_rollback() {
    sdk_owned_contract("row_trigger");
}
#[test]
fn native_sdk_log_guard_cross_lane_sql_intents_full_application_rollback() {
    sdk_owned_contract("cross_lane");
}
#[test]
fn native_sdk_log_guard_owner_and_prepared_ignore_atomic_rollback() {
    sdk_owned_contract("atomic_begin_ignore");
}
#[test]
fn native_sdk_log_guard_row_ignore_no_cursor_commit() {
    sdk_owned_contract("row_ignore");
}
#[test]
fn native_sdk_log_guard_every_unique_replace_recursive_triggers_off() {
    sdk_owned_contract("replace_unique");
}
#[test]
fn native_sdk_log_guard_false_validated_returned_and_unverified_cost_rejected() {
    sdk_owned_contract("stage_order");
}
#[test]
fn native_sdk_log_guard_wrong_original_owner_fields_rejected() {
    sdk_owned_contract("foreign_owner");
}
#[test]
fn native_sdk_log_guard_closed_original_owner_can_record_debt_without_regrant() {
    sdk_owned_contract("late_closed");
}
#[test]
fn native_sdk_log_replay_actual_wal_snapshot_consistent_during_writer_commit() {
    sdk_owned_contract("wal");
}
#[test]
fn native_sdk_log_replay_scope_attempt_deleted_and_no_create_negative() {
    sdk_owned_contract("scopes");
}
#[test]
fn native_sdk_log_replay_empty_corrupt_owner_is_not_available_success() {
    sdk_owned_contract("empty_corrupt_owner");
}
#[test]
fn native_sdk_log_replay_unfinished_prefix_has_explicit_incomplete_owner() {
    sdk_owned_contract("gap");
}

#[test]
fn native_sdk_log_replay_saved_cost_requires_original_fee_fact() {
    sdk_owned_contract("corrupt_cost_fact");
}

#[test]
fn native_sdk_log_real_gate_gap_committed_without_changing_model_bytes_or_cost() {
    use std::sync::{mpsc, Arc, Mutex};
    let (tx, arrival) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let gate = Arc::new(Mutex::new(gate));
    let raw = "real SDK exact bytes despite diagnostic fault";
    let (port, seen, _stop) = spawn_endpoint(Arc::new(move |_| {
        let _ = tx.send(());
        let _ = gate.lock().unwrap().recv_timeout(Duration::from_secs(5));
        (200, "application/json", sdk_exact_model_body(raw))
    }));
    let mut h = root_budget_owned_fixture_for_test("sdk-live-diagnostic-gap");
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let client = AgentModelClient::new(
        agent_model_profile(&h.context.environment, None).unwrap(),
        &[],
    );
    let db = db::open(&h.db_path).unwrap();
    db.execute_batch("CREATE TRIGGER sdk_live_ignore BEFORE INSERT ON native_sdk_log_rows WHEN NEW.stage='sent' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    let root = &h.context.run.as_ref().unwrap().run_id;
    let native = db
        .query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [root],
            |r| r.get::<_, String>(0),
        )
        .unwrap();
    let (live, result) = std::thread::scope(|scope| {
        let (done, received) = mpsc::channel();
        let ctx = &h.context;
        let client = &client;
        scope.spawn(move || {
            let _ = done.send(native_model_transport(ctx, client, vec![], &[], 1));
        });
        arrival.recv_timeout(Duration::from_secs(5)).unwrap();
        let live = serde_json::to_value(
            crate::agent_runtime::model::diagnostics::replay::read(
                &h.db_path,
                &ctx.scan_id,
                1,
                None,
                None,
                0,
                300,
            )
            .unwrap(),
        )
        .unwrap();
        assert!(received.try_recv().is_err());
        let _ = release.send(());
        (live, received.recv_timeout(Duration::from_secs(5)).unwrap())
    });
    assert_eq!(
        result
            .unwrap_or_else(|_| panic!("original SDK result failed"))
            .text
            .as_bytes(),
        raw.as_bytes()
    );
    assert_eq!(live["rows"][0]["stage"], "prepared");
    assert!(
        live["gaps"].as_array().is_some_and(|g| g
            .iter()
            .any(|gap| gap["failedStage"] == "sent" && gap["code"] == "stage_write_failed")),
        "failed observer must already have an explicit committed gap before provider replies"
    );
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(&db, root, None, "model_requests")
            .unwrap()
            .consumed,
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [root],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        native
    );
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert!(h.site_seen.lock().unwrap().is_empty());
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}

// These require SDK v2 gap schema: apply AFTER the fix, not as baseline red.
#[test]
fn native_sdk_log_gap_guard_trigger_full_application_rollback() {
    sdk_owned_contract("gap_trigger");
}
#[test]
fn native_sdk_log_gap_guard_ignore_exact_write_rollback() {
    sdk_owned_contract("gap_ignore");
}
#[test]
fn native_sdk_log_gap_guard_all_unique_replace_recursive_triggers_off() {
    sdk_owned_contract("gap_replace");
}

#[test]
fn native_sdk_log_gap_guard_closed_owner_no_later_stage() {
    sdk_owned_contract("gap_closed");
}

#[test]
fn native_sdk_log_replay_original_root_contract_drift_is_rejected_without_repair() {
    sdk_owned_contract("original_root_contract");
}
