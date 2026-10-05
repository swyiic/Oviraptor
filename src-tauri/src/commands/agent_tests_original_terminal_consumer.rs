// Actual ordinary creator, admitted executor, and live pipeline consumer.
// All databases/endpoints are disposable; rotation is a test-only row change.
fn original_terminal_dispatch(
    sdk: bool,
) -> (
    WebModeFixture,
    AgentRunContext,
    OwnedAgentTargetOutcome,
    usize,
) {
    original_terminal_dispatch_reply(sdk,None)
}
fn original_terminal_dispatch_reply(sdk:bool,reply:Option<String>)->(WebModeFixture,AgentRunContext,OwnedAgentTargetOutcome,usize) {
    let (site_port, site_seen, _site_stop) = spawn_endpoint(std::sync::Arc::new(mock_site));
    let fixture = web_mode_fixture("single", &format!("http://127.0.0.1:{site_port}"));
    let (port, _stop, seen) = spawn_model(vec![reply.unwrap_or_else(||model_round(
        &[(
            "finish_target",
            serde_json::json!({"coverage":closing_ledger(&[]),"stopReason":"original consumer"}),
        )],
        100,
    ))]);
    let mut context = test_context(
        &fixture.path,
        &fixture.target,
        vec![AgentIdentity::anonymous()],
    );
    context.scan_id = fixture.scan.clone();
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    if !sdk {
        context.environment.deployment = "cloud".into();
        context.environment.llm.clear();
    }
    context.target_dir = fixture.work.join("url-pipeline/target-00001");
    fs::create_dir_all(&context.target_dir).unwrap();
    fs::write(context.target_dir.join(".oviraptor-scan-id"), &fixture.scan).unwrap();
    fs::write(
        context.target_dir.join("frontend-evidence.json"),
        context.evidence.to_string(),
    )
    .unwrap();
    context.log_path = fixture.work.join("oviraptor-runner.log");
    let prepared = PreparedFrontendTarget {
        position: 1,
        route: context.route.clone(),
        target_dir: context.target_dir.clone(),
        proxy: None,
        browser: None,
    };
    let settings = serde_json::json!({"agentBackendPolicy":"native"});
    let adaptive = AgentBudgetSettings::from_json(&settings);
    let _real = RealSpecialistTransport::enter();
    let owned = run_agent_target(
        &prepared,
        AgentTargetExecution {
            db_path: &fixture.path,
            scan_id: &fixture.scan,
            attempt_number: 1,
            settings: &settings,
            environment: &context.environment,
            adaptive: &adaptive,
            log_path: &context.log_path,
        },
    )
    .unwrap();
    let calls = seen.lock().unwrap().len();
    assert_eq!(site_seen.lock().unwrap().len(), 0);
    let db = db::open(&fixture.path).unwrap();
    let root:String=db.query_row("SELECT id FROM agent_runs WHERE scan_id=?1 AND attempt_number=1 AND role='coordinator'",[&fixture.scan],|r|r.get(0)).unwrap();
    assert_eq!(
        owned.original_terminal.root_run_id.as_deref(),
        Some(root.as_str())
    );
    assert_eq!(owned.original_terminal.attempt_number, 1);
    context.run = Some(AgentRunLedger {
        db_path: fixture.path.clone(),
        run_id: root,
    });
    (fixture, context, owned, calls)
}
#[test]
fn original_terminal_identity_live_consumer_setup_failure_uses_original_root_without_native_state()
{
    let (fixture, context, owned, calls) = original_terminal_dispatch(false);
    assert_eq!(calls, 0);
    assert!(
        matches!(owned.outcome, AgentTargetOutcome::Failed(_)),
        "{:?}",
        owned.outcome
    );
    assert!(NativeAgentState::read(&fixture.path, &fixture.scan, &fixture.target).is_none());
    let mut tally = AgentPipelineTally::default();
    assert!(record_owned_agent_target_outcome(
        &fixture.path,
        &fixture.scan,
        &context.route,
        &owned,
        &mut tally
    ));
    assert_eq!((tally.counted(), tally.failed), (1, 1));
    let db = db::open(&fixture.path).unwrap();
    let root = owned.original_terminal.root_run_id.as_ref().unwrap();
    let state: (String, String) = db
        .query_row(
            "SELECT status,terminal_code FROM agent_runs WHERE id=?1",
            [root],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, ("terminal".into(), AGENT_STOP_CONFIGURATION.into()));
}
#[test]
fn original_terminal_identity_live_consumer_real_sdk_preserves_original_invoice_owner() {
    let (fixture, context, owned, calls) = original_terminal_dispatch(true);
    assert!(calls > 0);
    assert!(
        !matches!(
            owned.outcome,
            AgentTargetOutcome::Failed(_) | AgentTargetOutcome::ResumeIncompatible(_)
        ),
        "{:?}",
        owned.outcome
    );
    let mut tally = AgentPipelineTally::default();
    assert!(record_owned_agent_target_outcome(
        &fixture.path,
        &fixture.scan,
        &context.route,
        &owned,
        &mut tally
    ));
    assert_eq!(tally.counted(), 1);
    assert_eq!(tally.failed, 0);
    let db = db::open(&fixture.path).unwrap();
    let root = owned.original_terminal.root_run_id.as_ref().unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_runs WHERE scan_id=?1",
            [&fixture.scan],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='received'",[root],|r|r.get::<_,i64>(0)).unwrap(),i64::try_from(calls).unwrap());
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_single_projection_receipts WHERE root_run_id=?1",
            [root],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM agent_coordinator_leases", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
}
#[test]
fn original_terminal_identity_live_consumer_late_callback_changes_no_current_or_original_row() {
    let (fixture, context, owned, calls) = original_terminal_dispatch(false);
    assert_eq!(calls, 0);
    let db = db::open(&fixture.path).unwrap();
    // Deliberate isolated rotation only: not proof of a full retry/startup flow.
    db.execute(
        "UPDATE sentinel_scans SET attempt_count=2 WHERE id=?1",
        [&fixture.scan],
    )
    .unwrap();
    let plan = context.execution_plan.clone().with_attempt(2);
    persist_agent_execution_plan(&fixture.path, &fixture.scan, 2, &fixture.target, &plan).unwrap();
    let next =
        runtime_open_run_for_attempt(&fixture.path, &fixture.scan, &context.route, 2).unwrap();
    assert_ne!(
        Some(&next.run_id),
        owned.original_terminal.root_run_id.as_ref()
    );
    let before = single_finally_physical(&db);
    let mut tally = AgentPipelineTally::default();
    assert!(!record_owned_agent_target_outcome(
        &fixture.path,
        &fixture.scan,
        &context.route,
        &owned,
        &mut tally
    ));
    assert_eq!(tally.counted(), 0);
    assert!(tally.failure_details.is_empty());
    assert_eq!(
        single_finally_physical(&db),
        before,
        "late outcome cannot publish finance, checkpoint, target, fuse, or business facts"
    );
}
#[test]
fn original_terminal_identity_live_consumer_ambiguous_root_changes_no_row_or_tally() {
    let (fixture, context, owned, calls) = original_terminal_dispatch(false);
    assert_eq!(calls, 0);
    let db = db::open(&fixture.path).unwrap();
    // Foreign alias is corruption, never a second original owner/financial grant.
    let cols = db
        .prepare("PRAGMA table_info(agent_runs)")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    let names = cols
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(",");
    let values = cols
        .iter()
        .map(|c| {
            if c == "id" {
                "?2".into()
            } else {
                format!("\"{c}\"")
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    db.execute(
        &format!("INSERT INTO agent_runs({names}) SELECT {values} FROM agent_runs WHERE id=?1"),
        params![
            owned.original_terminal.root_run_id.as_ref().unwrap(),
            Uuid::new_v4().to_string()
        ],
    )
    .unwrap();
    let before = single_finally_physical(&db);
    let mut tally = AgentPipelineTally::default();
    assert!(!record_owned_agent_target_outcome(
        &fixture.path,
        &fixture.scan,
        &context.route,
        &owned,
        &mut tally
    ));
    assert_eq!(tally.counted(), 0);
    assert_eq!(single_finally_physical(&db), before);
}
