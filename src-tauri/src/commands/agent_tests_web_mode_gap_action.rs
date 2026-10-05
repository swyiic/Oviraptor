#[test]
fn web_mode_gap_action_actual_new_creation_and_submission_recovery_preserve_source() {
    let f = followup_fixture();
    let db = db::open(&f.context.db_path).unwrap();
    let input = followup_input(&f);
    let original = followup_preview(&f).unwrap().source_hash;
    let scan = create_agent_gap_followup_in(&db, &input).unwrap();
    assert_eq!(
        crate::agent_runtime::web_mode::draft::read(&db, &scan.id)
            .unwrap()
            .1,
        crate::agent_runtime::web_mode::WebMode::Multi
    );
    assert!(
        crate::agent_runtime::web_mode::action::read_hash(&db, &scan.id)
            .unwrap()
            .is_some()
    );
    assert_eq!(followup_preview(&f).unwrap().source_hash, original);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_runs WHERE scan_id=?1",
            [&scan.id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let before = web_mode_test_rows(&db);
    assert_eq!(
        create_agent_gap_followup_in(&db, &input).unwrap().id,
        scan.id
    );
    web_mode_assert_rows(&db, &before);
}
