#[test]
fn web_mode_new_ordinary_creator_freezes_single_multi_and_only_new_default() {
    use crate::agent_runtime::web_mode::{self, WebMode};
    for (input, expected) in [
        (Some("single"), WebMode::Single),
        (Some("multi"), WebMode::Multi),
        (None, WebMode::Multi),
    ] {
        let root = std::env::temp_dir().join(format!("oviraptor-mode-create-{}", Uuid::new_v4()));
        let path = db::initialize(&root).unwrap();
        let db = db::open(&path).unwrap();
        db.execute("INSERT INTO projects(id,name) VALUES(9001,'Mode')", [])
            .unwrap();
        let scan = web_mode_test_draft(&db, input, "https://mode.example.test/app").unwrap();
        assert_eq!(web_mode::draft::read(&db, &scan.id).unwrap().1, expected);
        let policy: String = db
            .query_row(
                "SELECT policy_json FROM sentinel_scan_contexts WHERE scan_id=?1",
                [&scan.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            WebMode::from_policy(&serde_json::from_str(&policy).unwrap()).unwrap(),
            expected
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM agent_runs", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn web_mode_unknown_input_and_creation_trigger_are_atomic() {
    for fault in ["unknown","BEFORE INSERT ON native_web_mode_drafts BEGIN SELECT RAISE(IGNORE); END;",
        "AFTER INSERT ON native_web_mode_drafts BEGIN UPDATE projects SET status='archived'; END;",
        "AFTER UPDATE OF policy_json ON sentinel_scan_contexts BEGIN UPDATE sentinel_scans SET status='completed'; END;"] {
        let root=std::env::temp_dir().join(format!("oviraptor-mode-fault-{}",Uuid::new_v4()));
        let path=db::initialize(&root).unwrap();let db=db::open(&path).unwrap();
        db.execute("INSERT INTO projects(id,name) VALUES(9001,'Mode')",[]).unwrap();
        if fault!="unknown" {db.execute_batch(&format!("CREATE TRIGGER mode_fault {fault}")).unwrap();}
        let before=web_mode_test_rows(&db);
        let input=if fault=="unknown" {"SINGLE"}else {"single"};
        assert!(web_mode_test_draft(&db,Some(input),"https://mode.example.test/app").is_err());
        web_mode_assert_rows(&db,&before);drop(db);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn web_mode_old_draft_and_shadow_policy_never_get_signed_on_startup() {
    for shadow in [
        "{\"orchestration\":{\"version\":1,\"mode\":\"single\"}}",
        "{}",
    ] {
        let (root, _, mut db) = web_mode_legacy_start_fixture();
        db.execute("UPDATE sentinel_scan_contexts SET policy_json=?1", [shadow])
            .unwrap();
        let before = web_mode_test_rows(&db);
        assert!(run_web_start_fixture(&mut db, &root, WebStartMode::Confirm).is_err());
        web_mode_assert_rows(&db, &before);
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn web_mode_creator_cannot_late_register_an_existing_draft() {
    let (root, _, mut db) = web_mode_legacy_start_fixture();
    let before = web_mode_test_rows(&db);
    // register_new is creation-only and must require the creator's fresh proof,
    // not merely status=draft/attempt=0, even when the old draft is untouched.
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let window = crate::agent_runtime::web_mode::draft::DraftCreationWindow::begin(&tx).unwrap();
    let result = crate::agent_runtime::web_mode::draft::register_new(
        window,
        "start-test",
        crate::agent_runtime::web_mode::WebMode::Multi,
    );
    assert!(result.is_err());
    drop(result);
    drop(tx);
    web_mode_assert_rows(&db, &before);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
