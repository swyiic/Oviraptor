// The timeout is set before the actual Root INSERT; no C/ledger backfill.
fn human_owned_short_clock(endpoint: &str) -> (RootTickFixture, String) {
    let f = root_tick_fixture_protocol_limits_timeout("short-human-clock", endpoint, true, (60_000, 20), Some(3));
    let db = db::open(&f.context.db_path).unwrap();
    let id = confirm_queue_directive(&db, &f.actor, "@investigator 请分析已有冻结证据");
    (f, id)
}

#[test]
fn human_proposal_owned_sdk_original_deadline_is_not_restarted_for_the_human_request() {
    use std::sync::{mpsc, Arc, Mutex};
    let (arrived_tx, arrived) = mpsc::channel();
    let (release_tx, release) = mpsc::channel();
    let release = Arc::new(Mutex::new(release));
    let (port, seen, _stop) = spawn_endpoint(Arc::new(move |_| {
        let _ = arrived_tx.send(());
        let _ = release.lock().unwrap().recv_timeout(Duration::from_secs(8));
        (
            200,
            "application/json",
            proposal_model_response(valid_proposal_text()),
        )
    }));
    let (f, _id) = human_owned_short_clock(&format!("http://127.0.0.1:{port}/v1"));
    let db = db::open(&f.context.db_path).unwrap();
    let remaining =
        crate::agent_runtime::multi_agent::budget::clock::remaining(&db, &f.actor.root_run_id)
            .unwrap();
    assert!(remaining <= Duration::from_secs(3));
    let mut inbox = take_human_directives(&f.context).unwrap();
    let early = std::thread::scope(|scope| {
        let (done_tx, done) = mpsc::channel();
        let context = &f.context;
        scope.spawn(move || {
            let _ = done_tx.send(apply_human_proposal_actions(context, &mut inbox));
        });
        arrived.recv_timeout(Duration::from_secs(2)).unwrap();
        let result = done.recv_timeout(remaining + Duration::from_secs(1));
        release_tx.send(()).unwrap();
        match result {
            Ok(result) => Some(result),
            Err(_) => {
                let _ = done.recv_timeout(Duration::from_secs(4));
                None
            }
        }
    });
    human_owned_wait_seen(&seen);
    assert!(
        early.is_some_and(|r| r.is_err()),
        "model request inherits the original elapsed Root deadline"
    );
    assert_eq!(human_owned_acked(&db), 0);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn human_proposal_owned_sdk_projection_business_trigger_rolls_back_without_losing_original_paid_fee(
) {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            proposal_model_response(valid_proposal_text()),
        )
    }));
    let (f, id) = human_owned_fixture(
        "human-owned-business-guard",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let db = db::open(&f.context.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    db.execute_batch("CREATE TRIGGER human_business_after_received AFTER UPDATE OF state ON agent_directive_proposals
        WHEN NEW.state='received' BEGIN UPDATE projects SET name='side-effect'; END;").unwrap();
    let mut inbox = take_human_directives(&f.context).unwrap();
    assert!(apply_human_proposal_actions(&f.context, &mut inbox).is_err());
    let after = super::tests::application_table_snapshot(&db);
    assert_eq!(
        before.iter().find(|(name, _)| name == "projects"),
        after.iter().find(|(name, _)| name == "projects")
    );
    assert_eq!(human_owned_acked(&db), 0);
    let state:(String,String)=db.query_row("SELECT p.state,c.state FROM agent_directive_proposals p JOIN agent_specialist_calls c ON c.child_run_id=p.child_run_id WHERE p.directive_id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(state, ("executing".into(), "received".into()));
    db.execute_batch("DROP TRIGGER human_business_after_received")
        .unwrap();
    assert_eq!(
        apply_human_proposal_actions(&f.context, &mut inbox)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn human_proposal_owned_sdk_old_unknown_direct_dispatch_is_not_adopted_or_retried() {
    use crate::agent_runtime::multi_agent::directive::proposals;
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            proposal_model_response(valid_proposal_text()),
        )
    }));
    let (f, id) = human_owned_fixture(
        "human-owned-legacy-executing",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let mut inbox = take_human_directives(&f.context).unwrap();
    let actor = inbox.lease.as_ref().unwrap();
    let db = db::open(&f.context.db_path).unwrap();
    let job = proposals::prepare_next(&db, actor, &f.context.evidence)
        .unwrap()
        .unwrap();
    consume_proposal_mailbox(
        &db,
        actor,
        &job.child.run_id,
        &job.request_message_id,
        "human_assessment_request",
        &job.input,
    )
    .unwrap();
    assert!(proposals::start(&db, actor, &id).unwrap());
    let old = receipt_database_snapshot(&db);
    assert!(apply_human_proposal_actions(&f.context, &mut inbox)
        .unwrap()
        .is_empty());
    assert_eq!(receipt_database_snapshot(&db), old);
    assert_eq!(human_owned_calls(&db), 0);
    assert_eq!(human_owned_acked(&db), 0);
    assert_eq!(seen.lock().unwrap().len(), 0);
}

#[test]
fn human_proposal_owned_sdk_dispatch_trigger_cannot_write_business_or_another_root_before_send() {
    for body in [
        "UPDATE projects SET name='unauthorized-model-trigger'",
        "UPDATE agent_runs SET plan_json='{}' WHERE role='coordinator'",
    ] {
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
            (
                200,
                "application/json",
                proposal_model_response(valid_proposal_text()),
            )
        }));
        let (f, _id) = human_owned_fixture(
            "human-owned-dispatch-sql-guard",
            &format!("http://127.0.0.1:{port}/v1"),
        );
        let db = db::open(&f.context.db_path).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        db.execute_batch(&format!("CREATE TRIGGER human_dispatch_business BEFORE INSERT ON agent_specialist_calls BEGIN {body}; END;")).unwrap();
        let mut inbox = take_human_directives(&f.context).unwrap();
        assert!(
            apply_human_proposal_actions(&f.context, &mut inbox).is_err(),
            "{body}"
        );
        let after = super::tests::application_table_snapshot(&db);
        assert_eq!(
            before.iter().find(|(name, _)| name == "projects"),
            after.iter().find(|(name, _)| name == "projects")
        );
        let plan: String = db
            .query_row(
                "SELECT plan_json FROM agent_runs WHERE id=?1",
                [&f.actor.root_run_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(plan, f.context.execution_plan.as_json().to_string());
        assert_eq!(
            human_owned_calls(&db),
            0,
            "the actual original dispatch transaction rolled back"
        );
        assert_eq!(human_owned_acked(&db), 0);
        assert_eq!(seen.lock().unwrap().len(), 0);
    }
}
