#[test]
fn web_mode_original_private_receipt_survives_reopen_without_claim_or_mutation() {
    use crate::agent_runtime::web_mode::WebMode;
    for mode in ["single", "multi"] {
        let fixture = web_mode_fixture(mode, "https://mode.example.test/app");
        let db = db::open(&fixture.path).unwrap();
        let before = web_mode_test_rows(&db);
        let first = private_web_mode_on(&db, &fixture.scan, 1).unwrap();
        drop(db);
        let reopened = db::open(&fixture.path).unwrap();
        let second = private_web_mode_on(&reopened, &fixture.scan, 1).unwrap();
        assert_eq!(first, second);
        assert_eq!(
            second.fact().mode,
            if mode == "single" {
                WebMode::Single
            } else {
                WebMode::Multi
            }
        );
        web_mode_assert_rows(&reopened, &before);
        assert!(reopened.query_row("SELECT claim_id='' FROM native_branch_dispatches WHERE scan_id=?1 AND attempt_number=1 AND branch='web'",
            [&fixture.scan],|r|r.get::<_,bool>(0)).unwrap());
    }
}

#[test]
fn web_mode_actual_same_attempt_recovery_claim_preserves_original_private_mode() {
    let fixture = web_mode_fixture("single", "https://mode.example.test/app");
    let db = db::open(&fixture.path).unwrap();
    let original = private_web_mode_on(&db, &fixture.scan, 1).unwrap();
    let mut owned = claim_web_recovery(&fixture.path, &fixture.scan, 1, true, |db| {
        private_web_mode_on(db, &fixture.scan, 1)
    })
    .unwrap();
    assert_eq!(owned.inputs, original);
    assert!(db.query_row("SELECT claim_id<>'' AND claimed_at<>'' FROM native_branch_dispatches WHERE scan_id=?1 AND attempt_number=1 AND branch='web'",
        [&fixture.scan],|r|r.get::<_,bool>(0)).unwrap());
    assert_eq!(
        private_web_mode_on(&db, &fixture.scan, 1).unwrap(),
        original
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM agent_runs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    // Retain the real claim receipt. This test proves admission, not a worker run.
    owned.guard.disarm();
}

#[test]
fn web_mode_actual_recovery_cannot_sign_missing_old_mode_or_change_any_table() {
    let fixture = web_mode_fixture("multi", "https://mode.example.test/app");
    let db = db::open(&fixture.path).unwrap();
    db.execute_batch(
        "DROP TRIGGER web_mode_receipt_immutable_delete;DELETE FROM native_web_mode_receipts;",
    )
    .unwrap();
    let before = web_mode_test_rows(&db);
    let result = claim_web_recovery(&fixture.path, &fixture.scan, 1, true, |db| {
        private_web_mode_on(db, &fixture.scan, 1)
    });
    assert!(result.is_err());
    web_mode_assert_rows(&db, &before);
}

#[test]
fn web_mode_private_original_scope_denies_shadow_task_key_or_missing_receipt_readonly() {
    for fault in ["policy", "task", "key", "receipt"] {
        let fixture = web_mode_fixture("single", "https://mode.example.test/app");
        let db = db::open(&fixture.path).unwrap();
        match fault {
            "policy" => {
                db.execute("UPDATE sentinel_scan_contexts SET policy_json=json_set(policy_json,'$.orchestration.mode','multi') WHERE scan_id=?1",[&fixture.scan]).unwrap();
            }
            "task" => {
                fs::write(fixture.work.join("task.json"), "{}").unwrap();
            }
            "key" => {
                fs::write(fixture.work.join(WEB_DISPATCH_KEY_FILE), [99u8; 32]).unwrap();
            }
            "receipt" => {
                db.execute_batch("DROP TRIGGER web_mode_receipt_immutable_delete")
                    .unwrap();
                db.execute("DELETE FROM native_web_mode_receipts", [])
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let before = web_mode_test_rows(&db);
        assert!(
            private_web_mode_on(&db, &fixture.scan, 1).is_err(),
            "{fault}"
        );
        web_mode_assert_rows(&db, &before);
    }
}

#[test]
fn web_mode_last_receipt_publication_fault_rolls_back_the_whole_startup() {
    for body in ["BEFORE INSERT ON native_web_mode_receipts BEGIN SELECT RAISE(IGNORE); END;",
        "AFTER INSERT ON native_web_mode_receipts BEGIN UPDATE sentinel_scan_contexts SET policy_json='{}'; END;",
        "AFTER INSERT ON native_web_mode_receipts BEGIN UPDATE native_web_dispatch_bindings SET binding_tag=zeroblob(32); END;"] {
        let root=std::env::temp_dir().join(format!("oviraptor-mode-final-{}",Uuid::new_v4()));
        let path=db::initialize(&root).unwrap();let mut db=db::open(&path).unwrap();
        db.execute("INSERT INTO projects(id,name) VALUES(9001,'Mode')",[]).unwrap();
        let scan=web_mode_test_draft(&db,Some("single"),"https://mode.example.test/app").unwrap();
        db.execute_batch(&format!("CREATE TRIGGER mode_fault {body}")).unwrap();
        let before=web_mode_test_rows(&db);
        assert!(web_mode_test_start(&root,&mut db,&scan.id,WebStartMode::Confirm).is_err(),"{body}");
        web_mode_assert_rows(&db,&before);
        assert!(!root.join(SCAN_WORK_DIRECTORY).join(&scan.id).join("attempt-0001").exists());
        drop(db);fs::remove_dir_all(root).unwrap();
    }
}
