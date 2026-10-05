#[test]
fn coordinator_tick_actual_multi_prepare_uses_root_sdk_then_mapper_and_original_headroom() {
    let _real = RealSpecialistTransport::enter();
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|request| {
        let text = if request.contains("You are the Root Coordinator") {
            if request.contains("mapper-output") {
                json!({"schemaVersion":1,"observed":["new frozen bootstrap decision"],"missing":[],"suggestions":["dispatch:web_executor"],"costNotes":[],"risks":[]}).to_string()
            } else {json!({"schemaVersion":1,"observed":["new frozen bootstrap decision"],"missing":[],"suggestions":["dispatch:spa_api_mapper"],"costNotes":[],"risks":[]}).to_string()}
        } else {
            r#"{"summary":"actual Mapper input","priorityContracts":[],"risks":[]}"#.into()
        };
        (200, "application/json", proposal_model_response(&text))
    }));
    let mut f = root_tick_fixture(
        "tick-production-prepare",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    drop(f.parent.take());
    let mut session = multi_agent_prepare(&mut f.context).unwrap();
    let db = db::open(&f.context.db_path).unwrap();
    assert_eq!(
        seen.lock().unwrap().len(),
        3,
        "actual bootstrap Root, Mapper, then changed-fact Root SDK"
    );
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 2);
    assert!(
        f.context.evidence["multiAgentCoordinator"]["decisionSummary"]["observed"]
            .to_string()
            .contains("new frozen bootstrap decision")
    );
    let root_tokens = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &f.actor.root_run_id,
        Some(""),
        "model_input_tokens",
    )
    .unwrap()
    .consumed
        + crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &f.actor.root_run_id,
            Some(""),
            "model_output_tokens",
        )
        .unwrap()
        .consumed;
    let root_requests = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &f.actor.root_run_id,
        Some(""),
        "model_requests",
    )
    .unwrap()
    .consumed;
    let (total_tokens,total_requests,spent_tokens,spent_requests):(i64,i64,i64,i64)=db.query_row(
        "SELECT total_tokens,total_requests,spent_tokens,spent_requests FROM agent_budget_ledger WHERE root_run_id=?1",
        [&f.actor.root_run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    let grants: (i64, i64) = db
        .query_row(
            "SELECT reserved_tokens,reserved_requests FROM agent_assignments WHERE id=?1",
            [&session.executor.assignment_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        grants,
        (
            total_tokens
                - spent_tokens
                - root_tokens
                - 8_000
                - 2 * crate::agent_runtime::multi_agent::directive::proposals::PROPOSAL_TOKENS,
            total_requests - spent_requests - root_requests - 3
        ),
        "Root's paid calls preserve the Reviewer and both ordered human reservations"
    );
    multi_agent_finish_execution(
        &f.context,
        &mut session,
        &AgentTargetOutcome::incomplete("tick headroom proof"),
    )
    .unwrap();
    drop(session);
}
