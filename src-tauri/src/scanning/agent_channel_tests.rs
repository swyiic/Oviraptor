use super::*;

fn ledger() -> rusqlite::Connection {
    let c = rusqlite::Connection::open_in_memory().unwrap();
    c.execute_batch("PRAGMA foreign_keys=ON;CREATE TABLE projects(id INTEGER PRIMARY KEY,status TEXT);INSERT INTO projects VALUES(1,'active'),(2,'active'),(3,'archived');").unwrap();
    c
}

#[test]
fn prepared_operations_refuse_replay_or_changed_payload_and_keep_project_scope() {
    let c = ledger();
    let args = json!({"taskId":"real-task"});
    assert!(claim(&c, 1, "start-once", "start", &args)
        .unwrap()
        .is_none());
    let again = claim(&c, 1, "start-once", "start", &args).unwrap().unwrap();
    assert_eq!(again["outcomeUnknown"], true);
    assert_eq!(again["retryable"], false);
    assert_eq!(
        claim(&c, 1, "start-once", "resume", &args).unwrap_err(),
        "scan_operation_conflict"
    );
    assert_eq!(
        claim(&c, 1, "start-once", "start", &json!({"taskId":"other"})).unwrap_err(),
        "scan_operation_conflict"
    );
    assert!(claim(&c, 2, "start-once", "start", &args)
        .unwrap()
        .is_none());
    assert!(claim(&c, 3, "archived", "start", &args).is_err());
    assert!(claim(&c, 3, "pause-archived", "pause", &args)
        .unwrap()
        .is_none());
    let count: i64 = c
        .query_row("SELECT COUNT(*) FROM agent_scan_operations", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 3);
}

#[test]
fn completed_operation_returns_historical_receipt_without_second_mutation() {
    let c = ledger();
    let args = json!({"assetIds":[42]});
    assert!(claim(&c, 1, "draft", "from_assets", &args)
        .unwrap()
        .is_none());
    let receipt = json!({"projectId":1,"task":{"id":"engine-draft","status":"draft"},"receiptIsHistorical":true});
    c.execute(
        "UPDATE agent_scan_operations SET state='completed',receipt_json=?1",
        [receipt.to_string()],
    )
    .unwrap();
    assert_eq!(
        claim(&c, 1, "draft", "from_assets", &args).unwrap(),
        Some(receipt)
    );
}

#[test]
fn channel_arguments_reject_scope_paths_arbitrary_urls_and_empty_or_duplicate_assets() {
    for (action, args) in [
        ("start", json!({"taskId":"x","projectId":2})),
        ("resume", json!({"taskId":"x","dbPath":"/tmp/foreign"})),
        ("from_assets", json!({"assetIds":[]})),
        ("from_assets", json!({"assetIds":[1,1]})),
        (
            "from_assets",
            json!({"assetIds":[1],"targets":["https://outside.test"]}),
        ),
        ("from_assets", json!({"assetIds":[-1]})),
        ("list", json!({"offset":-1})),
        ("list", json!({"limit":21})),
    ] {
        assert!(arguments(action, &args).is_err(), "{action} {args}");
    }
    assert!(arguments("from_assets", &json!({"assetIds":[1,2],"scanMode":"quick"})).is_ok());
    assert!(arguments("status", &json!({"taskId":"native-id"})).is_ok());
}

#[test]
fn original_asset_draft_is_visible_in_same_owner_and_safe_projection() {
    let root = std::env::temp_dir().join(format!("ovi-scan-channel-{}", Uuid::new_v4()));
    let path = db::initialize(&root).unwrap();
    let c = db::open(&path).unwrap();
    c.execute("INSERT INTO projects(id,name,status) VALUES(301,'channel-one','active'),(302,'channel-two','active')",[]).unwrap();
    c.execute("INSERT INTO assets(id,asset_key,company,host,link) VALUES(5001,'agent-channel-asset','fixture','channel.example.test','https://channel.example.test')",[]).unwrap();
    c.execute(
        "INSERT INTO project_assets(project_id,asset_id) VALUES(301,5001)",
        [],
    )
    .unwrap();
    let args = json!({"assetIds":[5001],"scanMode":"quick"});
    assert!(claim(&c, 301, "real-draft", "from_assets", &args)
        .unwrap()
        .is_none());
    let draft = create_sentinel_asset_scan_with_receipt_in(
        &c,
        301,
        vec![5001],
        Some("quick".into()),
        |tx, scan| creation_receipt(tx, 301, "real-draft", scan).map(|_| ()),
    )
    .unwrap();
    assert_eq!(draft.status, "draft");
    assert_eq!(draft.attempt_count, 0);
    assert!(owned(&c, 302, &draft.id).is_err());
    let value = projection(&c, &owned(&c, 301, &draft.id).unwrap()).unwrap();
    assert_eq!(value["targetCount"], 1);
    assert_eq!(value["executionStarted"], false);
    assert!(value.get("taskPath").is_none());
    assert!(value.get("sourcePath").is_none());
    drop(c);
    let c = db::open(&path).unwrap();
    let saved = claim(&c, 301, "real-draft", "from_assets", &args)
        .unwrap()
        .unwrap();
    assert_eq!(saved["task"]["id"], draft.id);
    assert_eq!(saved["receiptIsHistorical"], true);
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM sentinel_scans WHERE project_id=301",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    // A late receipt failure must roll back the owner's draft and signed
    // creation identity together, leaving only the non-replayable preparation.
    assert!(claim(&c, 301, "receipt-failure", "from_assets", &args)
        .unwrap()
        .is_none());
    let failed =
        create_sentinel_asset_scan_with_receipt_in(&c, 301, vec![5001], None, |tx, scan| {
            creation_receipt(tx, 301, "receipt-failure", scan)?;
            Err("injected_late_receipt_failure".into())
        });
    assert!(failed.is_err());
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM sentinel_scans WHERE project_id=301",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        c.query_row(
            "SELECT state FROM agent_scan_operations WHERE operation_id='receipt-failure'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "prepared"
    );
    c.execute(
        "INSERT INTO sentinel_deleted_scans(scan_id) VALUES(?1)",
        [&draft.id],
    )
    .unwrap();
    assert!(owned(&c, 301, &draft.id).is_err());
    drop(c);
    std::fs::remove_dir_all(root).unwrap();
}
