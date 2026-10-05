// Existing production Root tick + real localhost SDK gate + public readonly replay.
// Include only after g2 Root fixtures and SDK v2 contracts; no logger/SDK shim.
fn root_tick_sdk_page(path: &Path, scan: &str) -> JsonValue {
    serde_json::to_value(
        crate::agent_runtime::model::diagnostics::replay::read(path, scan, 1, None, None, 0, 300)
            .unwrap(),
    )
    .unwrap()
}
fn root_tick_sdk_rows(page: &JsonValue, round: i64) -> Vec<JsonValue> {
    page["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["domain"] == "root" && r["round"] == round)
        .cloned()
        .collect()
}
fn root_tick_sdk_stages(rows: &[JsonValue]) -> Vec<&str> {
    rows.iter().map(|r| r["stage"].as_str().unwrap()).collect()
}
fn root_tick_sdk_assert_original(db: &rusqlite::Connection, root: &str, rows: &[JsonValue]) {
    for row in rows {
        let exact: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM agent_root_model_journal j
            JOIN agent_root_tick_receipts p ON p.call_id=j.call_id AND p.root_run_id=j.root_run_id
            AND p.round=j.round AND p.request_hash=j.request_hash AND p.lease_attempt_id=j.lease_attempt_id AND p.phase='request'
            WHERE j.root_run_id=?1 AND j.phase='dispatch' AND j.call_id=?2 AND j.round=?3
            AND j.request_hash=?4 AND j.lease_attempt_id=?5)",params![root,row["dispatchKey"].as_str().unwrap(),
                row["round"].as_i64().unwrap(),row["requestHash"].as_str().unwrap(),row["leaseAttemptId"].as_str().unwrap()],|r|r.get(0)).unwrap();
        assert!(exact);
        assert_eq!(row["rootRunId"], root);
        assert_eq!(row["runId"], root);
        assert!(row["workerId"].is_null() && row["assignmentId"].is_null());
    }
}

#[test]
fn native_sdk_log_coordinator_bootstrap_real_gate_commits_before_reply_and_return() {
    use std::sync::{mpsc, Arc, Mutex};
    let raw = format!(
        " \n{} \t",
        root_tick_valid_text("password=private-root-sdk-response")
    );
    let body = sdk_exact_model_body(&raw);
    let (arrive, arrived) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let released = Arc::new(Mutex::new(released));
    let (port, seen, _stop) = spawn_endpoint(Arc::new(move |_| {
        let _ = arrive.send(());
        let _ = released
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5));
        (200, "application/json", body.clone())
    }));
    let f = root_tick_fixture(
        "sdk-coordinator-gate",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let db = db::open(&f.context.db_path).unwrap();
    let native: String = db
        .query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [&f.actor.root_run_id],
            |r| r.get(0),
        )
        .unwrap();
    let (live, result) = std::thread::scope(|scope| {
        let (done, rx) = mpsc::channel();
        let context = &f.context;
        let actor = &f.actor;
        scope.spawn(move || {
            let _ = done.send(native_coordinator_tick(context, actor));
        });
        arrived.recv_timeout(Duration::from_secs(5)).unwrap();
        let live = root_tick_sdk_page(&f.context.db_path, &f.context.scan_id);
        assert!(rx.try_recv().is_err());
        let _ = release.send(());
        (live, rx.recv_timeout(Duration::from_secs(5)).unwrap())
    });
    let returned = result.unwrap();
    assert!(!returned.replayed);
    let before = root_tick_sdk_rows(&live, 1);
    assert_eq!(root_tick_sdk_stages(&before), ["prepared", "sent"]);
    root_tick_sdk_assert_original(&db, &f.actor.root_run_id, &before);
    let page = root_tick_sdk_page(&f.context.db_path, &f.context.scan_id);
    let rows = root_tick_sdk_rows(&page, 1);
    assert_eq!(
        root_tick_sdk_stages(&rows),
        [
            "prepared",
            "sent",
            "response_received",
            "cost_saved",
            "validated",
            "terminal"
        ]
    );
    root_tick_sdk_assert_original(&db, &f.actor.root_run_id, &rows);
    assert_eq!(rows.last().unwrap()["terminalState"], "returned");
    assert_eq!(seen.lock().unwrap().len(), 1);
    let hash:String=db.query_row("SELECT json_extract(receipt_json,'$.responseHash') FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='received'",[&f.actor.root_run_id],|r|r.get(0)).unwrap();
    assert_eq!(
        hash,
        crate::agent_runtime::store::stable_hash(&json!({"text":raw,"tools":[],
            "usage":{"inputTokens":7,"cachedInputTokens":2,"outputTokens":5,"totalTokens":12,"modelRequests":1},
            "reported":true,"finish":"stop"}).to_string()),
        "original financial hash retains exact text bytes and usage contract"
    );
    assert_eq!(
        db.query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [&f.actor.root_run_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        native
    );
    for forbidden in [
        "private-root-sdk-response",
        "You are the Root Coordinator",
        "messages",
        "credentialBinding",
        "responseHash",
        "reasoning",
    ] {
        assert!(
            !page.to_string().contains(forbidden),
            "public diagnostics retained {forbidden}"
        );
    }
    let snapshot = super::tests::application_table_snapshot(&db);
    assert!(
        native_coordinator_tick(&f.context, &f.actor)
            .unwrap()
            .replayed
    );
    assert_eq!(
        root_tick_sdk_page(&f.context.db_path, &f.context.scan_id),
        page
    );
    assert_eq!(super::tests::application_table_snapshot(&db), snapshot);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn native_sdk_log_coordinator_paid_withheld_recovery_never_creates_replay_diagnostics() {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            sdk_exact_model_body(&root_tick_valid_text("paid retained decision")),
        )
    }));
    let f = root_tick_fixture(
        "sdk-coordinator-paid-replay",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let db = db::open(&f.context.db_path).unwrap();
    db.execute_batch("CREATE TRIGGER sdk_coordinator_publication_fault BEFORE INSERT ON agent_events WHEN NEW.event_type='model_round_completed' BEGIN SELECT RAISE(ABORT,'withheld'); END;").unwrap();
    assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
    let page = root_tick_sdk_page(&f.context.db_path, &f.context.scan_id);
    let rows = root_tick_sdk_rows(&page, 1);
    assert_eq!(
        root_tick_sdk_stages(&rows),
        [
            "prepared",
            "sent",
            "response_received",
            "cost_saved",
            "terminal"
        ]
    );
    assert_eq!(rows.last().unwrap()["terminalState"], "withheld");
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "decision"), 1);
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 0);
    let cost_before: Vec<String> = db
        .prepare(
            "SELECT receipt_json FROM agent_root_model_journal WHERE root_run_id=?1 ORDER BY rowid",
        )
        .unwrap()
        .query_map([&f.actor.root_run_id], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    db.execute_batch("DROP TRIGGER sdk_coordinator_publication_fault")
        .unwrap();
    assert!(
        native_coordinator_tick(&f.context, &f.actor)
            .unwrap()
            .replayed
    );
    assert_eq!(
        root_tick_sdk_page(&f.context.db_path, &f.context.scan_id),
        page,
        "local publication recovery is not another physical SDK return"
    );
    let cost_after: Vec<String> = db
        .prepare(
            "SELECT receipt_json FROM agent_root_model_journal WHERE root_run_id=?1 ORDER BY rowid",
        )
        .unwrap()
        .query_map([&f.actor.root_run_id], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(cost_after, cost_before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn native_sdk_log_coordinator_unknown_or_invalid_semantics_records_no_validated_completion() {
    for unknown in [true, false] {
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(move |_| {
            let text = if unknown {
                root_tick_valid_text("unknown bill")
            } else {
                "{\"schemaVersion\":1,\"reasoning\":\"private-root-chain\"}".into()
            };
            let mut body: JsonValue = serde_json::from_str(&sdk_exact_model_body(&text)).unwrap();
            if unknown {
                body.as_object_mut().unwrap().remove("usage");
            }
            (200, "application/json", body.to_string())
        }));
        let f = root_tick_fixture(
            "sdk-coordinator-invalid",
            &format!("http://127.0.0.1:{port}/v1"),
        );
        assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
        let page = root_tick_sdk_page(&f.context.db_path, &f.context.scan_id);
        let rows = root_tick_sdk_rows(&page, 1);
        assert_eq!(
            root_tick_sdk_stages(&rows),
            [
                "prepared",
                "sent",
                "response_received",
                "cost_saved",
                "terminal"
            ]
        );
        assert_eq!(rows.last().unwrap()["terminalState"], "withheld");
        assert!(!page.to_string().contains("private-root-chain"));
        assert_eq!(seen.lock().unwrap().len(), 1);
    }
}

#[test]
fn native_sdk_log_coordinator_sent_gap_is_public_while_real_sdk_continues_once() {
    use std::sync::{mpsc, Arc, Mutex};
    let (arrive, arrived) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let released = Arc::new(Mutex::new(released));
    let (port, seen, _stop) = spawn_endpoint(Arc::new(move |_| {
        let _ = arrive.send(());
        let _ = released
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5));
        (
            200,
            "application/json",
            sdk_exact_model_body(&root_tick_valid_text("same actual Root call")),
        )
    }));
    let f = root_tick_fixture(
        "sdk-coordinator-gap",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let db = db::open(&f.context.db_path).unwrap();
    db.execute_batch("CREATE TRIGGER sdk_root_sent_ignore BEFORE INSERT ON native_sdk_log_rows WHEN NEW.stage='sent' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    let (live, result) = std::thread::scope(|scope| {
        let (done, rx) = mpsc::channel();
        let context = &f.context;
        let actor = &f.actor;
        scope.spawn(move || {
            let _ = done.send(native_coordinator_tick(context, actor));
        });
        arrived.recv_timeout(Duration::from_secs(5)).unwrap();
        let live = root_tick_sdk_page(&f.context.db_path, &f.context.scan_id);
        assert!(rx.try_recv().is_err());
        let _ = release.send(());
        (live, rx.recv_timeout(Duration::from_secs(5)).unwrap())
    });
    result.unwrap();
    assert_eq!(
        root_tick_sdk_stages(&root_tick_sdk_rows(&live, 1)),
        ["prepared"]
    );
    assert_eq!(live["gaps"].as_array().unwrap().len(), 1);
    assert_eq!(live["gaps"][0]["failedStage"], "sent");
    let page = root_tick_sdk_page(&f.context.db_path, &f.context.scan_id);
    assert_eq!(page["gaps"], live["gaps"]);
    assert_eq!(
        root_tick_sdk_stages(&root_tick_sdk_rows(&page, 1)),
        ["prepared"]
    );
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 1);
    assert_eq!(seen.lock().unwrap().len(), 1);
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
}
