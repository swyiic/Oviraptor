// Ordinary creator and captured original Root: an anonymous control is a
// credential-free comparison, never a second captured authenticated account.
#[test]
fn fresh_single_authenticated_anonymous_control_uses_actual_entry_and_isolates_credentials() {
    let mut h = fresh_single_production_harness_with_sessions(
        "authenticated-anonymous-control",
        |request| {
            let authenticated =
                request.contains("cookie-alpha") && request.contains("Bearer alpha");
            if authenticated {
                (
                    200,
                    "application/json",
                    r#"{"userId":"self","role":"member"}"#.into(),
                )
            } else {
                (
                    401,
                    "application/json",
                    r#"{"error":"login_required"}"#.into(),
                )
            }
        },
        &[("session-a", "cookie-alpha", "Bearer alpha")],
    );
    freeze_fresh_single_production_harness(&mut h);
    retarget_model(
        &mut h,
        vec![
            model_round(
                &[(
                    "compare_identities",
                    json!({
                        "leftIdentity":"session-a", "rightIdentity":"anonymous", "method":"GET",
                        "path":"/api/profile", "family":"authentication_session",
                    }),
                )],
                100,
            ),
            model_round(
                &[(
                    "finish_target",
                    json!({
                        "coverage":closing_ledger(&["authentication_session"]),
                        "stopReason":"authenticated endpoint denies anonymous control"
                    }),
                )],
                100,
            ),
        ],
    );
    let owned = execute_fresh_single_production_harness(&mut h);
    let requests = h.site_seen.lock().unwrap();
    assert_eq!(
        requests.len(),
        2,
        "the actual authenticated task must execute its credential-free anonymous control"
    );
    let authenticated: Vec<_> = requests
        .iter()
        .filter(|wire| wire.contains("cookie-alpha"))
        .collect();
    assert_eq!(authenticated.len(), 1);
    assert!(authenticated[0].contains("Bearer alpha"));
    let anonymous: Vec<_> = requests
        .iter()
        .filter(|wire| !wire.contains("cookie-alpha"))
        .collect();
    assert_eq!(anonymous.len(), 1);
    assert!(anonymous[0].starts_with("GET /api/profile HTTP/"));
    assert!(!anonymous[0].to_ascii_lowercase().contains("cookie:"));
    assert!(!anonymous[0].to_ascii_lowercase().contains("authorization:"));
    drop(requests);
    let models = h.model_seen.lock().unwrap().join("\n");
    assert!(!models.contains("cookie-alpha") && !models.contains("Bearer alpha"));
    assert!(models.contains("当前身份") && models.contains("匿名会话"));
    assert!(!models.contains("身份 A（已认证）") && !models.contains("身份 B（已认证）"));
    let status = match &owned.outcome {
        AgentTargetOutcome::Completed(_) => "completed",
        AgentTargetOutcome::BoundedCompleted(_) => "completed_with_gaps",
        other => panic!(
            "normal denied anonymous access must close as accounted work: {}",
            other.detail()
        ),
    };
    let mut tally = AgentPipelineTally::default();
    assert!(record_owned_agent_target_outcome(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        &owned,
        &mut tally
    ));
    assert_fresh_single_surface(&h, &h.context.target_url, status, 2, 0);
    let db = db::open(&h.db_path).unwrap();
    let (mode, ids) = crate::auth_session::validated_scan_identities(
        &db,
        &h.context.scan_id,
        &h.context.target_url,
    )
    .unwrap();
    assert!(matches!(
        mode,
        crate::auth_session::ScanIdentityMode::SingleIdentity
    ));
    assert_eq!(ids, vec!["session-a"]);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM browser_auth_sessions WHERE owner_scan_id=?1",
            [&h.context.scan_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn anonymous_control_cannot_replace_bound_accounts_or_authorize_forged_handles() {
    let mut h = fresh_single_production_harness_with_sessions(
        "anonymous-control-binding-negatives",
        equal_role_site,
        &[
            ("session-a", "cookie-alpha", "Bearer alpha"),
            ("session-b", "cookie-beta", "Bearer beta"),
        ],
    );
    freeze_fresh_single_production_harness(&mut h);
    let db = db::open(&h.db_path).unwrap();
    let original = single_finally_physical(&db);
    let mut full = h.context.clone();
    full.identities.push(AgentIdentity::anonymous());
    assert_eq!(
        agent_authorize_tool_on(&db, &h.context, "replay_http"),
        Ok(())
    );
    assert_eq!(agent_authorize_tool_on(&db, &full, "replay_http"), Ok(()));
    let mut duplicate_control = full.clone();
    duplicate_control
        .identities
        .push(AgentIdentity::anonymous());
    let mut alias = full.clone();
    alias.identities.last_mut().unwrap().key = "guest".into();
    let mut credential_control = full.clone();
    credential_control.identities.last_mut().unwrap().session_id = Some("session-a".into());
    let mut flag = full.clone();
    flag.identities.last_mut().unwrap().anonymous = false;
    let mut captured_flag = full.clone();
    captured_flag.identities[0].anonymous = true;
    let mut extra_account = full.clone();
    extra_account
        .identities
        .push(AgentIdentity::scoped("unbound-account"));
    let mut missing_account = full.clone();
    missing_account.identities.remove(0);
    let mut control_only = full.clone();
    control_only.identities = vec![AgentIdentity::anonymous()];
    for (tag, context) in [
        ("duplicate", duplicate_control),
        ("alias", alias),
        ("credential", credential_control),
        ("flag", flag),
        ("captured-flag", captured_flag),
        ("extra-account", extra_account),
        ("missing-account", missing_account),
        ("control-only", control_only),
    ] {
        assert_eq!(
            agent_authorize_tool_on(&db, &context, "replay_http"),
            Err("tool_identity_binding_denied"),
            "{tag}"
        );
        assert_eq!(
            single_finally_physical(&db),
            original,
            "{tag} must not change any application row or fee"
        );
    }
    assert!(h.model_seen.lock().unwrap().is_empty());
    assert!(h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn anonymous_control_revocation_after_first_http_stops_second_side_and_preserves_paid_facts() {
    use std::sync::{Arc, Mutex};
    let binding = Arc::new(Mutex::new(None::<(PathBuf, String)>));
    let site_binding = binding.clone();
    let mut h = fresh_single_production_harness_with_sessions(
        "anonymous-control-mid-http-revocation",
        move |request| {
            assert!(request.contains("cookie-alpha") && request.contains("Bearer alpha"));
            let (path, scan) = site_binding.lock().unwrap().clone().unwrap();
            let db = db::open(&path).unwrap();
            assert_eq!(db.execute("UPDATE browser_auth_sessions SET expires_at='2000-01-01T00:00:00Z' WHERE id='session-a' AND owner_scan_id=?1",[scan]).unwrap(),1);
            (
                200,
                "application/json",
                r#"{"userId":"self","role":"member"}"#.into(),
            )
        },
        &[("session-a", "cookie-alpha", "Bearer alpha")],
    );
    freeze_fresh_single_production_harness(&mut h);
    *binding.lock().unwrap() = Some((h.db_path.clone(), h.context.scan_id.clone()));
    retarget_model(
        &mut h,
        vec![model_round(
            &[(
                "compare_identities",
                json!({
                    "leftIdentity":"session-a", "rightIdentity":"anonymous", "method":"GET",
                    "path":"/api/profile", "family":"authentication_session",
                }),
            )],
            100,
        )],
    );
    let owned = execute_fresh_single_production_harness(&mut h);
    assert_eq!(
        h.site_seen.lock().unwrap().len(),
        1,
        "revocation during the first received HTTP must block the anonymous second side"
    );
    assert_eq!(h.model_seen.lock().unwrap().len(), 1);
    assert_eq!(
        owned.outcome.terminal_code(),
        terminal_code::EXECUTION_AUTHORIZATION_DENIED,
        "{}",
        owned.outcome.detail()
    );
    let db = db::open(&h.db_path).unwrap();
    let root = &h.context.run.as_ref().unwrap().run_id;
    let owner =
        crate::agent_runtime::multi_agent::budget::root::RootOwner::load_single(&db, root).unwrap();
    owner.read_single_exit(&db).unwrap();
    for (dimension, expected) in [("model_requests", 1), ("target_requests", 1)] {
        let paid =
            crate::agent_runtime::multi_agent::budget::balance(&db, root, Some(""), dimension)
                .unwrap();
        assert_eq!(
            (paid.consumed, paid.reserved, paid.indeterminate),
            (expected, 0, 0)
        );
    }
    let mut tally = AgentPipelineTally::default();
    assert!(record_owned_agent_target_outcome(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        &owned,
        &mut tally
    ));
    assert_eq!(tally.counted(), 1);
    assert_eq!(
        db.query_row(
            "SELECT status FROM sentinel_targets WHERE scan_id=?1 AND url=?2",
            params![h.context.scan_id, h.context.target_url],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "paused"
    );
    assert!(findings_for(&h.db_path, AGENT_VULNERABILITY_STAGE).is_empty());
    let original = single_finally_physical(&db);
    assert!(record_owned_agent_target_outcome(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        &owned,
        &mut tally
    ));
    assert_eq!(single_finally_physical(&db), original);
    assert_eq!(tally.counted(), 1);
    assert_eq!(h.site_seen.lock().unwrap().len(), 1);
    assert_eq!(h.model_seen.lock().unwrap().len(), 1);
}
