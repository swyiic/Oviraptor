#[test]
fn human_proposal_owned_sdk_original_dispatch_is_committed_while_actual_provider_is_alive() {
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
    let (f, id) = human_owned_fixture("human-owned-gate", &format!("http://127.0.0.1:{port}/v1"));
    let mut inbox = take_human_directives(&f.context).unwrap();
    let db = db::open(&f.context.db_path).unwrap();
    let mut committed = 0;
    let mut live_sdk_stages = Vec::new();
    std::thread::scope(|scope| {
        let (done_tx, done) = mpsc::channel();
        let context = &f.context;
        scope.spawn(move || {
            let _ = done_tx.send(apply_human_proposal_actions(context, &mut inbox));
        });
        arrived.recv_timeout(Duration::from_secs(5)).unwrap();
        committed = db.query_row("SELECT count(*) FROM agent_specialist_calls c JOIN agent_directive_proposals p
            ON p.assignment_id=c.assignment_id AND p.child_run_id=c.child_run_id
            JOIN agent_assignment_attempts x ON x.child_run_id=c.child_run_id AND x.assignment_id=c.assignment_id
            WHERE p.directive_id=?1 AND c.state='executing' AND x.state='running'
            AND json_extract(c.request_json,'$.humanDirectiveDispatch.directiveId')=p.directive_id
            AND json_extract(c.request_json,'$.humanDirectiveDispatch.workerId')=x.worker_id
            AND json_extract(c.request_json,'$.humanDirectiveDispatch.leaseAttemptId')=x.id", [&id], |r| r.get(0)).unwrap();
        assert!(
            done.try_recv().is_err(),
            "real provider is gated, so this is not an EOF-only assertion"
        );
        let child: String = db
            .query_row(
                "SELECT child_run_id FROM agent_directive_proposals WHERE directive_id=?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        let snapshot = crate::agent_runtime::model::diagnostics::replay::read(
            &f.context.db_path,
            &f.actor.scan_id,
            1,
            None,
            None,
            0,
            300,
        )
        .unwrap();
        live_sdk_stages = snapshot
            .rows
            .iter()
            .filter(|r| r.owner.run_id == child)
            .map(|r| r.stage.clone())
            .collect::<Vec<_>>();
        release_tx.send(()).unwrap();
        assert_eq!(
            done.recv_timeout(Duration::from_secs(5))
                .unwrap()
                .unwrap()
                .len(),
            1
        );
    });
    human_owned_wait_seen(&seen);
    assert_eq!(
        committed, 1,
        "SDK send must follow committed original worker dispatch proof"
    );
    assert_eq!(
        live_sdk_stages,
        vec!["prepared", "sent"],
        "safe stages are committed before the gated response"
    );
    assert_eq!(human_owned_calls(&db), 1);
    assert_eq!(human_owned_acked(&db), 1);
}

#[test]
fn human_proposal_owned_sdk_changed_c_pause_and_parent_stop_withhold_and_retain_original_fee() {
    use crate::agent_runtime::multi_agent::budget;
    use std::sync::{mpsc, Arc, Mutex};
    for change in ["replace_c", "pause", "parent_stop"] {
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
        let (mut f, id) =
            human_owned_fixture("human-owned-cancel", &format!("http://127.0.0.1:{port}/v1"));
        let mut inbox = take_human_directives(&f.context).unwrap();
        let db = db::open(&f.context.db_path).unwrap();
        let early = std::thread::scope(|scope| {
            let (done_tx, done) = mpsc::channel();
            let context = &f.context;
            scope.spawn(move || {
                let _ = done_tx.send(apply_human_proposal_actions(context, &mut inbox));
            });
            arrived.recv_timeout(Duration::from_secs(5)).unwrap();
            match change {
                "replace_c" => {
                    db.execute("UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1,fencing_token=?1", [uuid::Uuid::new_v4().to_string()]).unwrap();
                }
                "pause" => {
                    db.execute(
                        "UPDATE sentinel_scans SET status='pausing' WHERE id=?1",
                        [&f.actor.scan_id],
                    )
                    .unwrap();
                }
                _ => drop(f.parent.take()),
            }
            let early = done.recv_timeout(Duration::from_secs(3));
            release_tx.send(()).unwrap();
            match early {
                Ok(result) => Some(result),
                Err(_) => {
                    let _ = done.recv_timeout(Duration::from_secs(5));
                    None
                }
            }
        });
        human_owned_wait_seen(&seen);
        assert!(
            early.is_some_and(|result| result.is_err()),
            "{change}: original owner must cancel before a blocked response is released"
        );
        let (assignment, child):(String,String)=db.query_row("SELECT assignment_id,child_run_id FROM agent_directive_proposals WHERE directive_id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        let cost = budget::balance(
            &db,
            &f.actor.root_run_id,
            Some(&assignment),
            "model_requests",
        )
        .unwrap();
        assert_eq!(
            cost.consumed + cost.indeterminate,
            1,
            "{change}: not zero-cost and not new owner's fee"
        );
        assert_eq!(
            human_owned_acked(&db),
            0,
            "{change}: fee capture is not publication authority"
        );
        let events:i64=db.query_row("SELECT count(*) FROM agent_events WHERE run_id=?1 AND event_type='model_round_completed'",[&child],|r|r.get(0)).unwrap();
        assert_eq!(
            events, 0,
            "unknown cancelled transport is not a completed model receipt"
        );
        assert_eq!(seen.lock().unwrap().len(), 1);
    }
}

#[test]
fn human_proposal_owned_sdk_unreported_usage_preserves_indeterminate_and_never_acknowledges() {
    use crate::agent_runtime::multi_agent::budget;
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        let mut v: JsonValue =
            serde_json::from_str(&proposal_model_response(valid_proposal_text())).unwrap();
        v.as_object_mut().unwrap().remove("usage");
        (200, "application/json", v.to_string())
    }));
    let (f, id) = human_owned_fixture(
        "human-owned-unreported",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let mut inbox = take_human_directives(&f.context).unwrap();
    assert!(apply_human_proposal_actions(&f.context, &mut inbox).is_err());
    let db = db::open(&f.context.db_path).unwrap();
    let assignment: String = db
        .query_row(
            "SELECT assignment_id FROM agent_directive_proposals WHERE directive_id=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    let count:i64=db.query_row("SELECT count(*) FROM agent_specialist_calls WHERE assignment_id=?1 AND state='received'
        AND json_extract(response_json,'$.usageReported')=0 AND json_extract(response_json,'$.rejection')='model_usage_requires_reconciliation'",[&assignment],|r|r.get(0)).unwrap();
    assert_eq!(count, 1);
    let requests = budget::balance(
        &db,
        &f.actor.root_run_id,
        Some(&assignment),
        "model_requests",
    )
    .unwrap();
    assert_eq!(requests.consumed, 1);
    let tokens = budget::balance(
        &db,
        &f.actor.root_run_id,
        Some(&assignment),
        "model_input_tokens",
    )
    .unwrap();
    assert!(tokens.indeterminate > 0);
    assert_eq!(human_owned_acked(&db), 0);
    let before = receipt_database_snapshot(&db);
    let _ = apply_human_proposal_actions(&f.context, &mut inbox);
    assert_eq!(receipt_database_snapshot(&db), before);
    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "unknown usage cannot be rerouted or retried"
    );
}

#[test]
fn human_proposal_owned_sdk_original_paid_receipt_survives_withheld_local_projection_and_replays_without_http(
) {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            proposal_model_response(valid_proposal_text()),
        )
    }));
    let (f, id) = human_owned_fixture(
        "human-owned-projection-fault",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let db = db::open(&f.context.db_path).unwrap();
    db.execute_batch(
        "CREATE TRIGGER ignore_human_response BEFORE UPDATE OF state ON agent_directive_proposals
        WHEN NEW.state='received' BEGIN SELECT RAISE(IGNORE); END;",
    )
    .unwrap();
    let mut inbox = take_human_directives(&f.context).unwrap();
    assert!(apply_human_proposal_actions(&f.context, &mut inbox).is_err());
    assert_eq!(human_owned_calls(&db), 1);
    let states:(String,String,i64)=db.query_row("SELECT p.state,c.state,(SELECT count(*) FROM agent_events WHERE run_id=p.child_run_id AND event_type='model_round_completed')
        FROM agent_directive_proposals p JOIN agent_specialist_calls c ON c.child_run_id=p.child_run_id WHERE p.directive_id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(states, ("executing".into(), "received".into(), 1));
    assert_eq!(human_owned_acked(&db), 0);
    db.execute_batch("DROP TRIGGER ignore_human_response")
        .unwrap();
    assert_eq!(
        apply_human_proposal_actions(&f.context, &mut inbox)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(human_owned_acked(&db), 1);
    assert_eq!(seen.lock().unwrap().len(), 1);
    let event = native_scan_status(&db, &f.actor.scan_id).unwrap();
    let item = event["timeline"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["id"] == id)
        .unwrap();
    assert_eq!(item["proposalAction"]["state"], "completed");
    finish_coordinator_run(&db, &f.actor, &AgentTargetOutcome::Cancelled).unwrap();
    db.execute(
        "UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1,fencing_token=?1",
        [uuid::Uuid::new_v4().to_string()],
    )
    .unwrap();
    let before = receipt_database_snapshot(&db);
    let history = native_scan_status(&db, &f.actor.scan_id).unwrap();
    let item = history["timeline"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["id"] == id)
        .unwrap();
    assert_eq!(
        item["proposalAction"]["state"], "completed",
        "history does not adopt new C"
    );
    assert_eq!(before, receipt_database_snapshot(&db));
}

#[test]
fn human_proposal_owned_sdk_paid_local_projection_cannot_replace_c_or_publish() {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            proposal_model_response(valid_proposal_text()),
        )
    }));
    let (f, id) = human_owned_fixture(
        "human-owned-paid-late",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let db = db::open(&f.context.db_path).unwrap();
    db.execute_batch("CREATE TRIGGER change_c_after_paid AFTER UPDATE OF state ON agent_directive_proposals WHEN NEW.state='received'
        BEGIN UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1,fencing_token='late-replaced-c'; END;").unwrap();
    let mut inbox = take_human_directives(&f.context).unwrap();
    assert!(apply_human_proposal_actions(&f.context, &mut inbox).is_err());
    assert_eq!(human_owned_acked(&db), 0);
    let row:(String,String)=db.query_row("SELECT p.state,c.state FROM agent_directive_proposals p JOIN agent_specialist_calls c ON c.child_run_id=p.child_run_id WHERE p.directive_id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(row, ("executing".into(), "received".into()));
    let cost:i64=db.query_row("SELECT SUM(amount) FROM agent_budget_entries WHERE assignment_id=(SELECT assignment_id FROM agent_directive_proposals WHERE directive_id=?1) AND dimension='model_requests' AND kind='consume'",[&id],|r|r.get(0)).unwrap();
    assert_eq!(
        cost, 1,
        "committed original fee survives loss of publication authority"
    );
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn human_proposal_owned_sdk_route_replays_typed_committed_safe_stages_without_model_body() {
    use crate::agent_runtime::model::diagnostics::replay;
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            proposal_model_response(valid_proposal_text()),
        )
    }));
    let (f, id) = human_owned_fixture(
        "human-owned-sdk-public-reader",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let mut inbox = take_human_directives(&f.context).unwrap();
    assert_eq!(
        apply_human_proposal_actions(&f.context, &mut inbox)
            .unwrap()
            .len(),
        1
    );
    let snapshot =
        replay::read(&f.context.db_path, &f.actor.scan_id, 1, None, None, 0, 300).unwrap();
    let value = serde_json::to_value(&snapshot).unwrap();
    let db = db::open(&f.context.db_path).unwrap();
    let child: String = db
        .query_row(
            "SELECT child_run_id FROM agent_directive_proposals WHERE directive_id=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    let rows: Vec<_> = value["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v["runId"] == child)
        .collect();
    assert_eq!(
        rows.iter()
            .map(|v| v["stage"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec![
            "prepared",
            "sent",
            "response_received",
            "cost_saved",
            "validated",
            "terminal"
        ]
    );
    assert!(rows
        .iter()
        .all(|v| v["domain"] == "specialist" && !v["workerId"].as_str().unwrap().is_empty()));
    assert_eq!(rows.last().unwrap()["terminalState"], "returned");
    assert!(!value.to_string().contains("独立评估已有路由"));
    assert!(!value.to_string().contains("请分析已有冻结证据"));
    assert!(!value.to_string().contains("dispatchClaimId"));
    let before = value.clone();
    assert_eq!(
        apply_human_proposal_actions(&f.context, &mut inbox)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        serde_json::to_value(
            replay::read(&f.context.db_path, &f.actor.scan_id, 1, None, None, 0, 300).unwrap()
        )
        .unwrap(),
        before
    );
    assert_eq!(seen.lock().unwrap().len(), 1);
}
