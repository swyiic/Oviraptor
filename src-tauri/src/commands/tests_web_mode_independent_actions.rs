#[test]
fn web_mode_closure_handoff_action_mints_only_its_new_draft_and_replay_is_readonly() {
    let (root, path, db, input) = closure_handoff_fixture();
    let original = administrative_closure_evidence(&db, &input.source_scan_id).unwrap();
    let receipt = create_closure_handoff_in(&db, &input).unwrap();
    let scan = receipt["scanId"].as_str().unwrap();
    assert_eq!(
        crate::agent_runtime::web_mode::draft::read(&db, scan)
            .unwrap()
            .1,
        crate::agent_runtime::web_mode::WebMode::Multi
    );
    assert!(crate::agent_runtime::web_mode::action::read_hash(&db, scan)
        .unwrap()
        .is_some());
    assert_eq!(
        administrative_closure_evidence_except_event(
            &db,
            &input.source_scan_id,
            Some(&input.request_id)
        )
        .unwrap(),
        original
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_runs WHERE scan_id=?1",
            [scan],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let before = web_mode_test_rows(&db);
    assert_eq!(create_closure_handoff_in(&db, &input).unwrap(), receipt);
    web_mode_assert_rows(&db, &before);
    drop(db);
    let reopened = db::open(&path).unwrap();
    assert!(
        crate::agent_runtime::web_mode::action::read_hash(&reopened, scan)
            .unwrap()
            .is_some()
    );
    drop(reopened);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn web_mode_independent_action_trigger_cannot_change_source_or_leave_unsigned_mode() {
    for fault in ["BEFORE INSERT ON native_web_mode_actions BEGIN SELECT RAISE(IGNORE); END;",
        "AFTER INSERT ON native_web_mode_actions BEGIN UPDATE sentinel_scans SET status='draft' WHERE id=NEW.scan_id; END;",
        "AFTER INSERT ON native_web_mode_actions BEGIN UPDATE native_web_closure_handoffs SET input_hash=printf('%064d',0); END;"] {
        let (root,_,db,input)=closure_handoff_fixture();db.execute_batch(&format!("CREATE TRIGGER action_fault {fault}")).unwrap();
        let before=web_mode_test_rows(&db);assert!(create_closure_handoff_in(&db,&input).is_err(),"{fault}");web_mode_assert_rows(&db,&before);
        drop(db);fs::remove_dir_all(root).unwrap();
    }
}
