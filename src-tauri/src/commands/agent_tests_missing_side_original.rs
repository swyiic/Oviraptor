// Real original Single entry. Session rows are first bound by the ordinary
// creator/startup; deleting one is a disposable negative, never a grant issuer.
fn missing_side_original_dispatch() -> (AgentHarness, OwnedAgentTargetOutcome) {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let mut h = fresh_single_production_harness_with_sessions(
        "missing-side-original",
        unequal_role_site,
        &[
            ("session-a", "cookie-alpha", "Bearer alpha"),
            ("session-b", "cookie-beta", "Bearer beta"),
        ],
    );
    freeze_fresh_single_production_harness(&mut h);
    let db = db::open(&h.db_path).unwrap();
    let policy: String = db
        .query_row(
            "SELECT policy_json FROM sentinel_scan_contexts WHERE scan_id=?1",
            [&h.context.scan_id],
            |r| r.get(0),
        )
        .unwrap();
    let left: String = db.query_row(
        "SELECT session_json FROM browser_auth_sessions WHERE id='session-a' AND owner_scan_id=?1",
        [&h.context.scan_id], |r| r.get(0),
    ).unwrap();
    let path = h.db_path.clone();
    let scan = h.context.scan_id.clone();
    let rounds = AtomicUsize::new(0);
    let (port, seen, stop) = spawn_endpoint(Arc::new(move |_| {
        assert_eq!(
            rounds.fetch_add(1, Ordering::SeqCst),
            0,
            "no follow-up SDK after the missing side"
        );
        let db = db::open(&path).unwrap();
        assert_eq!(
            db.execute(
                "DELETE FROM browser_auth_sessions WHERE id='session-b' AND owner_scan_id=?1",
                [&scan],
            )
            .unwrap(),
            1
        );
        (
            200,
            "application/json",
            model_round(
                &[(
                    "compare_identities",
                    json!({"leftIdentity":"session-a","rightIdentity":"session-b",
                "method":"GET","path":"/api/profile","family":"authorization","contractKey":"idor|/api/profile"}),
                )],
                900,
            ),
        )
    }));
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    h.model_seen = seen;
    let owned = execute_fresh_single_production_harness(&mut h);
    stop.store(true, Ordering::SeqCst);
    assert_eq!(
        db.query_row(
            "SELECT policy_json FROM sentinel_scan_contexts WHERE scan_id=?1",
            [&h.context.scan_id],
            |r| r.get::<_, String>(0),
        )
        .unwrap(),
        policy,
        "missing B cannot downgrade the original two-account policy"
    );
    assert_eq!(db.query_row(
        "SELECT session_json FROM browser_auth_sessions WHERE id='session-a' AND owner_scan_id=?1",
        [&h.context.scan_id], |r| r.get::<_, String>(0),
    ).unwrap(), left, "the still-valid account's captured material is unchanged");
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM browser_auth_sessions WHERE id='session-b'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap(),
        0,
        "no restoration or surrogate identity is allowed"
    );
    (h, owned)
}

fn missing_side_original_verify(h: &AgentHarness, owned: &OwnedAgentTargetOutcome, calls: i64) {
    use crate::agent_runtime::multi_agent::budget;
    assert!(
        matches!(owned.outcome, AgentTargetOutcome::Incomplete(_)),
        "{:?}",
        owned.outcome
    );
    assert_eq!(
        owned.outcome.terminal_code(),
        terminal_code::EXECUTION_AUTHORIZATION_DENIED
    );
    assert!(
        owned
            .outcome
            .detail()
            .contains("tool_identity_binding_denied"),
        "{}",
        owned.outcome.detail()
    );
    assert!(
        h.site_seen.lock().unwrap().is_empty(),
        "neither side can emit partial HTTP"
    );
    assert_eq!(h.model_seen.lock().unwrap().len() as i64, calls);
    let db = db::open(&h.db_path).unwrap();
    let root = owned.original_terminal.root_run_id.as_ref().unwrap();
    let owner = budget::root::RootOwner::load_single(&db, root).unwrap();
    owner.read_single_exit(&db).unwrap();
    for dimension in budget::DIMENSIONS {
        let balance = budget::balance(&db, root, None, dimension).unwrap();
        assert_eq!(
            (balance.reserved, balance.indeterminate),
            (0, 0),
            "{dimension}"
        );
    }
    let balance = |dimension| budget::balance(&db, root, None, dimension).unwrap();
    assert_eq!(balance("model_requests").consumed, calls);
    assert_eq!(balance("model_input_tokens").consumed > 0, calls > 0);
    assert_eq!(balance("model_output_tokens").consumed > 0, calls > 0);
    assert_eq!(balance("target_requests").consumed, 0);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM tool_invocations WHERE run_id=?1",
            [root],
            |r| r.get::<_, i64>(0),
        )
        .unwrap(),
        0,
        "Root response cannot grant the proposed comparison after revocation"
    );
    assert!(
        NativeAgentState::load(&h.db_path, &h.context.scan_id, &h.context.target_url)
            .unwrap()
            .is_none(),
        "no fake resumable checkpoint or authorization coverage"
    );
    assert!(findings_for(&h.db_path, AGENT_VULNERABILITY_STAGE).is_empty());
    assert!(findings_for(&h.db_path, AGENT_COVERAGE_STAGE).is_empty());
    let models = h.model_seen.lock().unwrap().join("\n");
    for secret in ["cookie-alpha", "cookie-beta", "Bearer alpha", "Bearer beta"] {
        assert!(!models.contains(secret));
    }
    let mut tally = AgentPipelineTally::default();
    assert!(record_owned_agent_target_outcome(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        owned,
        &mut tally
    ));
    assert_eq!(tally.counted(), 1);
    assert_eq!(
        tally.failed, 0,
        "missing evidence does not blame the target"
    );
    assert_eq!(
        db.query_row(
            "SELECT status FROM sentinel_targets WHERE scan_id=?1 AND url=?2",
            params![h.context.scan_id, h.context.target_url],
            |r| r.get::<_, String>(0),
        )
        .unwrap(),
        "paused"
    );
    let terminal = read_agent_checkpoint(
        &h.db_path,
        &h.context.scan_id,
        &h.context.target_url,
        "agent_terminal",
    );
    assert_eq!(terminal["status"], "paused");
    let original = single_finally_physical(&db);
    assert!(record_owned_agent_target_outcome(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        owned,
        &mut tally
    ));
    assert_eq!(tally.counted(), 1);
    assert_eq!(
        single_finally_physical(&db),
        original,
        "owned replay preserves every original row/rowid/fee"
    );
    let reported_completion = record_agent_target_outcome(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        AgentTargetOutcome::Completed(AgentCompletion::without_ledger(
            "forged full coverage",
            AGENT_STOP_FINISH,
        )),
        &mut tally,
    );
    let after = read_agent_checkpoint(
        &h.db_path,
        &h.context.scan_id,
        &h.context.target_url,
        "agent_terminal",
    );
    assert_eq!(
        after["code"],
        terminal_code::EXECUTION_AUTHORIZATION_DENIED,
        "an incoming completion must not replace the canonical authorization refusal"
    );
    assert_eq!(
        after["stop"]["code"],
        terminal_code::EXECUTION_AUTHORIZATION_DENIED,
        "the nested stop must retain the original receipt's refusal too"
    );
    assert!(
        reported_completion,
        "canonical original receipt replay is acknowledged"
    );
    assert_eq!(tally.counted(), 1);
    assert!(
        single_finally_physical(&db) == original,
        "canonical replay preserves all original typed rows/rowids/fees"
    );
    assert!(h.site_seen.lock().unwrap().is_empty());
    assert_eq!(h.model_seen.lock().unwrap().len() as i64, calls);
}

#[test]
fn e2e_missing_side_stops_at_insufficient_evidence() {
    let (h, owned) = missing_side_original_dispatch();
    missing_side_original_verify(&h, &owned, 1);
    drop(owned);
    fs::remove_dir_all(h.root).unwrap();
}

#[test]
fn fresh_single_missing_side_before_sdk_refuses_without_partial_auth_grants() {
    use crate::agent_runtime::multi_agent::budget;
    let mut h = fresh_single_production_harness_with_sessions(
        "missing-side-before-sdk",
        unequal_role_site,
        &[
            ("session-a", "cookie-alpha", "Bearer alpha"),
            ("session-b", "cookie-beta", "Bearer beta"),
        ],
    );
    freeze_fresh_single_production_harness(&mut h);
    let db = db::open(&h.db_path).unwrap();
    assert_eq!(
        db.execute(
            "DELETE FROM browser_auth_sessions WHERE id='session-b' AND owner_scan_id=?1",
            [&h.context.scan_id],
        )
        .unwrap(),
        1
    );
    let original = single_finally_physical(&db);
    let prepared = PreparedFrontendTarget {
        position: 1,
        route: h.context.route.clone(),
        target_dir: h.context.target_dir.clone(),
        proxy: None,
        browser: None,
    };
    let settings = json!({});
    let owned = run_agent_target(
        &prepared,
        AgentTargetExecution {
            db_path: &h.db_path,
            scan_id: &h.context.scan_id,
            attempt_number: 1,
            settings: &settings,
            environment: &h.context.environment,
            adaptive: &AgentBudgetSettings::from_json(&settings),
            log_path: &h.context.log_path,
        },
    )
    .unwrap();
    assert!(
        matches!(owned.outcome, AgentTargetOutcome::Failed(_)),
        "{:?}",
        owned.outcome
    );
    assert_eq!(owned.outcome.terminal_code(), AGENT_STOP_PERSISTENCE);
    assert!(owned
        .outcome
        .detail()
        .contains("agent_attempt_plan_frozen_conflict"));
    assert!(
        owned.original_terminal.root_run_id.is_none(),
        "plan refusal does not acquire or infer original Root ownership"
    );
    assert!(
        single_finally_physical(&db) == original,
        "entry refusal cannot rewrite any original row/rowid"
    );
    assert!(h.model_seen.lock().unwrap().is_empty());
    assert!(h.site_seen.lock().unwrap().is_empty());
    let root = &h.context.run.as_ref().unwrap().run_id;
    budget::root::RootOwner::load_single(&db, root).unwrap();
    for dimension in budget::DIMENSIONS {
        let value = budget::balance(&db, root, None, dimension).unwrap();
        assert_eq!(
            (value.consumed, value.reserved, value.indeterminate),
            (0, 0, 0),
            "{dimension}"
        );
    }
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_single_exit_receipts WHERE root_run_id=?1",
            [root],
            |r| r.get::<_, i64>(0),
        )
        .unwrap(),
        0,
        "no executed caller exit may be forged"
    );
    let mut tally = AgentPipelineTally::default();
    assert!(!record_owned_agent_target_outcome(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        &owned,
        &mut tally
    ));
    assert_eq!(tally.counted(), 0);
    assert_eq!(single_finally_physical(&db), original);
    assert!(
        NativeAgentState::load(&h.db_path, &h.context.scan_id, &h.context.target_url)
            .unwrap()
            .is_none()
    );
    drop(owned);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}

include!("agent_tests_single_unknown_replay.rs");
