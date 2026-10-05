// Use the born 30-second clock, never a rewritten ceiling or clock origin.
#[test]
fn source_financial_exit_uses_one_cutoff_when_commit_crosses_original_deadline() {
    let (root, db, c, result) = source_reviewer_execution_fixture_configured_before_birth(
        "valid",
        None,
        3,
        None,
        1,
        (None, |db, record| {
            record.policy["webModeCeiling"] = json!("quick");
            db.execute("UPDATE config_profiles SET settings_json=json_set(settings_json,'$.agentQuickTimeout',30)",[]).unwrap();
        }),
        |root, db, record, c| {
            source_reviewer_checkpoint_fixture(root, db, record, c, "first_assessment_unknown");
            let paid = source_exit_paid_rows(db, &c.root_run_id);
            let original = c.clone();
            let path = root.join("oviraptor.sqlite3");
            crate::agent_runtime::multi_agent::budget::clock::observation::checkpoint_once_for_test(
                move || {
                    let db = crate::db::open(&path).unwrap();
                    let end = std::time::Instant::now() + Duration::from_secs(40);
                    loop {
                        match crate::agent_runtime::multi_agent::budget::clock::remaining(
                            &db,
                            &original.root_run_id,
                        ) {
                            Err(e) if e == "budget_wall_time_exhausted" => break,
                            Ok(_) => {
                                assert!(std::time::Instant::now() < end);
                                std::thread::sleep(Duration::from_millis(20));
                            }
                            other => panic!("{other:?}"),
                        }
                    }
                },
            );
            let result = run_native_source_assessments(
                &root.join("oviraptor.sqlite3"),
                &record.scan_id,
                1,
                &root.join("attempt-0001"),
            );
            let error = result.as_ref().unwrap_err();
            assert!(
                error.contains("source_initial_received_binding_invalid"),
                "{error}"
            );
            assert!(
                !error.contains("source_financial_exit:"),
                "original financial exit sampled a later cutoff: {error}"
            );
            assert_eq!(source_exit_paid_rows(db, &c.root_run_id), paid);
            assert!(source_exit_wall(db, &c.root_run_id) < 30_000);
            assert!(
                crate::agent_runtime::multi_agent::budget::clock::elapsed_fact::read(
                    db,
                    &c.root_run_id
                )
                .unwrap()
                .is_none()
            );
            result
        },
    );
    assert!(result.is_err());
    assert!(
        crate::agent_runtime::multi_agent::budget::clock::remaining(&db, &c.root_run_id).is_err()
    );
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
