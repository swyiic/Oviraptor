#[test]
fn root_local_react_reviewer_floor_is_enforced_by_original_before_insert_ceilings() {
    for limits in [(16_000, 20), (60_000, 1)] {
        let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|_| {
            (
                200,
                "application/json",
                proposal_model_response(&root_tick_valid_text(
                    "reviewer floor must block before SDK",
                )),
            )
        }));
        let f = root_tick_fixture_protocol_limits(
            "root-local-reviewer-floor",
            &format!("http://127.0.0.1:{port}/v1"),
            true,
            limits,
        );
        let db = db::open(&f.context.db_path).unwrap();
        let prepared = native_coordinator_request(&f.context, &f.actor).unwrap();
        assert!(
            limits.1 == 1 || prepared.estimate > limits.0 - 15_000,
            "the token fixture must actually leave less than the frozen reviewer floor"
        );
        let before = web_mode_test_rows(&db);
        assert_eq!(
            native_coordinator_tick(&f.context, &f.actor).err().unwrap(),
            "child_budget_reservation_exceeded_or_stale"
        );
        assert_eq!(seen.lock().unwrap().len(), 0);
        assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "request"), 0);
        web_mode_assert_rows(&db, &before);
        assert_eq!(
            db.query_row(
                "SELECT hard_token_budget,hard_request_budget FROM agent_runs WHERE id=?1",
                [&f.actor.root_run_id],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
            )
            .unwrap(),
            limits
        );
    }
}
