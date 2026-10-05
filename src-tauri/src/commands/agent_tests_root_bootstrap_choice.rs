// A real provider answer must select the original offered bootstrap step.
#[test]
fn coordinator_bootstrap_defer_or_unoffered_step_never_creates_mapper_or_grant() {
    for step in [None, Some("dispatch:web_executor")] {
        let _real = RealSpecialistTransport::enter();
        let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |request| {
            let response = if request.contains("You are the Root Coordinator")
                && !request.contains("mapper-output")
            {
                proposal_model_response(&json!({"schemaVersion":1,"observed":["original frontend evidence"],
                    "missing":[],"suggestions":step.map(|s|vec![s]).unwrap_or_default(),"costNotes":[],"risks":[]}).to_string())
            } else {
                changed_fact_response(&request, true)
            };
            (200, "application/json", response)
        }));
        let mut f = root_tick_fixture(
            "bootstrap-choice-refusal",
            &format!("http://127.0.0.1:{port}/v1"),
        );
        drop(f.parent.take());
        let result = multi_agent_prepare(&mut f.context);
        assert!(
            result.is_err(),
            "paid bootstrap did not select Mapper but preparation dispatched it"
        );
        assert_eq!(
            result.err().unwrap(),
            "root_bootstrap_did_not_select_mapper"
        );
        assert!(
            matches!(
                multi_agent_bootstrap_outcome("root_bootstrap_did_not_select_mapper"),
                AgentTargetOutcome::Incomplete(_)
            ),
            "a paid Root defer must remain incomplete, not a tool failure"
        );
        assert_eq!(
            seen.lock().unwrap().len(),
            1,
            "one paid Root answer, no Mapper or later Root SDK"
        );
        let db = db::open(&f.context.db_path).unwrap();
        assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 1);
        for table in [
            "agent_assignments",
            "agent_assignment_attempts",
            "agent_lane_leases",
            "agent_capability_leases",
            "agent_specialist_calls",
        ] {
            assert_eq!(
                db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                0,
                "{table}"
            );
        }
        assert_eq!(
            crate::agent_runtime::multi_agent::budget::balance(
                &db,
                &f.actor.root_run_id,
                None,
                "model_requests"
            )
            .unwrap()
            .consumed,
            1
        );
    }
}
