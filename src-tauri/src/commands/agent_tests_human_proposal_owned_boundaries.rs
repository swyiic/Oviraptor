#[test]
fn human_proposal_owned_sdk_missing_or_foreign_parent_rejects_before_grant_or_mailbox() {
    for damage in [
        "missing_parent",
        "missing_run",
        "foreign_run",
        "foreign_scope",
    ] {
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
            (
                200,
                "application/json",
                proposal_model_response(valid_proposal_text()),
            )
        }));
        let (mut f, _id) =
            human_owned_fixture("human-preflight", &format!("http://127.0.0.1:{port}/v1"));
        let mut inbox = take_human_directives(&f.context).unwrap();
        let db = db::open(&f.context.db_path).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        match damage {
            "missing_parent" => f.context.supervision = None,
            "missing_run" => f.context.run = None,
            "foreign_run" => f.context.run.as_mut().unwrap().run_id = "foreign-root".into(),
            _ => f.context.target_url = "https://foreign.example.test".into(),
        }
        assert!(
            apply_human_proposal_actions(&f.context, &mut inbox).is_err(),
            "{damage}"
        );
        assert_eq!(
            super::tests::application_table_snapshot(&db),
            before,
            "{damage}: no new grants, worker, consumption or fee"
        );
        assert_eq!(seen.lock().unwrap().len(), 0);
    }
}

#[test]
fn human_proposal_owned_sdk_paid_projection_preserves_caller_hook_and_refuses_open_transaction() {
    use crate::agent_runtime::multi_agent::directive::proposals;
    use rusqlite::hooks::{AuthAction, Authorization};
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            proposal_model_response(valid_proposal_text()),
        )
    }));
    let (f, id) = human_owned_fixture(
        "human-private-projection",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let mut inbox = take_human_directives(&f.context).unwrap();
    let db = db::open(&f.context.db_path).unwrap();
    db.execute_batch("CREATE TRIGGER human_hold_projection BEFORE UPDATE OF state ON agent_directive_proposals WHEN NEW.state='received' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    assert!(apply_human_proposal_actions(&f.context, &mut inbox).is_err());
    assert_eq!(human_owned_calls(&db), 1);
    assert_eq!(human_owned_acked(&db), 0);
    db.execute_batch("DROP TRIGGER human_hold_projection")
        .unwrap();
    let before = super::tests::application_table_snapshot(&db);
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert_eq!(
        proposals::owned::record_received(&db, &f.actor, &id).unwrap_err(),
        "proposal_owned_private_transaction_required"
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    db.execute_batch("ROLLBACK").unwrap();
    db.authorizer(Some(|ctx: rusqlite::hooks::AuthContext<'_>| {
        match ctx.action {
            AuthAction::Update {
                table_name: "projects",
                ..
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }
    }))
    .unwrap();
    assert_eq!(
        proposals::owned::record_received(&db, &f.actor, &id)
            .unwrap()
            .state,
        "received"
    );
    assert!(db
        .execute("UPDATE projects SET name='caller-hook-must-survive'", [])
        .is_err());
    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "the original receipt is reused locally"
    );
    let paid = super::tests::application_table_snapshot(&db);
    proposals::owned::record_received(&db, &f.actor, &id).unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), paid);
    assert_eq!(
        human_owned_acked(&db),
        0,
        "projection alone is not delivery"
    );
}
