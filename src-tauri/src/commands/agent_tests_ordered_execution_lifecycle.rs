#[test]
fn ordered_execution_original_c_pause_cancel_and_supervisor_loss_stop_actual_sdk_before_return() {
    use std::sync::{mpsc, Arc, Mutex};
    for change in ["replace_c", "pause", "cancel", "stop_parent"] {
        let (arrive_tx, arrive) = mpsc::channel();
        let (release_tx, release) = mpsc::channel();
        let release = Arc::new(Mutex::new(release));
        let (port, seen, _stop) = spawn_endpoint(Arc::new(move |_| {
            let _ = arrive_tx.send(());
            let _ = release.lock().unwrap().recv_timeout(Duration::from_secs(8));
            (
                200,
                "application/json",
                proposal_model_response(valid_proposal_text()),
            )
        }));
        let (mut f, id) = ordered_exec_fixture(
            "ordered-original-life",
            &format!("http://127.0.0.1:{port}/v1"),
        );
        let db = db::open(&f.context.db_path).unwrap();
        let native = ordered_exec_native(&db, &f.actor.root_run_id);
        let mut inbox = take_human_directives(&f.context).unwrap();
        let early = std::thread::scope(|scope| {
            let (done_tx, done) = mpsc::channel();
            let context = &f.context;
            scope.spawn(move || {
                let _ = done_tx.send(apply_human_proposal_actions(context, &mut inbox));
            });
            arrive.recv_timeout(Duration::from_secs(5)).unwrap();
            match change {
                "replace_c" => {
                    db.execute("UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1,fencing_token=?1",[uuid::Uuid::new_v4().to_string()]).unwrap();
                }
                "pause" => {
                    db.execute(
                        "UPDATE sentinel_scans SET status='pausing' WHERE id=?1",
                        [&f.actor.scan_id],
                    )
                    .unwrap();
                }
                "cancel" => {
                    db.execute(
                        "UPDATE sentinel_scans SET status='cancelling' WHERE id=?1",
                        [&f.actor.scan_id],
                    )
                    .unwrap();
                }
                _ => drop(f.parent.take()),
            }
            let early = done.recv_timeout(Duration::from_secs(3));
            release_tx.send(()).unwrap();
            match early {
                Ok(r) => Some(r),
                Err(_) => {
                    let _ = done.recv_timeout(Duration::from_secs(5));
                    None
                }
            }
        });
        assert!(
            early.is_some_and(|r| r.is_err()),
            "{change}: transport must return before blocked provider response"
        );
        // Endpoint capture occurs after its blocked handler returns.
        let capture_deadline = std::time::Instant::now() + Duration::from_secs(1);
        while seen.lock().unwrap().is_empty() && std::time::Instant::now() < capture_deadline {
            std::thread::yield_now();
        }
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert_eq!(
            ordered_exec_count(&db, "agent_directive_ordered_receipts"),
            0
        );
        assert_eq!(
            ordered_exec_count(&db, "agent_directive_ordered_actions"),
            1
        );
        let assignment: String = db
            .query_row(
                "SELECT assignment_id FROM agent_directive_ordered_actions WHERE directive_id=?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        let cost = crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &f.actor.root_run_id,
            Some(&assignment),
            "model_requests",
        )
        .unwrap();
        assert_eq!(
            cost.consumed + cost.indeterminate,
            1,
            "cancelled request retains original financial owner"
        );
        assert_eq!(ordered_exec_native(&db, &f.actor.root_run_id), native);
        let ack:i64=db.query_row("SELECT COUNT(*) FROM agent_messages WHERE kind='human_ordered_assessment_result' AND acknowledged_at<>''",[],|r|r.get(0)).unwrap();
        assert_eq!(ack, 0);
    }
}

#[test]
fn ordered_execution_original_root_deadline_is_not_restarted_by_new_ordered_confirmation() {
    use std::sync::{mpsc, Arc, Mutex};
    let (arrive_tx, arrive) = mpsc::channel();
    let (release_tx, release) = mpsc::channel();
    let release = Arc::new(Mutex::new(release));
    let (port, seen, _stop) = spawn_endpoint(Arc::new(move |_| {
        let _ = arrive_tx.send(());
        let _ = release.lock().unwrap().recv_timeout(Duration::from_secs(8));
        (
            200,
            "application/json",
            proposal_model_response(valid_proposal_text()),
        )
    }));
    // Factory declares 3 seconds before original Native Root creation; no live
    // timeout/clock row is rewritten. The factory's old singleton is deferred.
    let (f, old) = human_owned_short_clock(&format!("http://127.0.0.1:{port}/v1"));
    let db = db::open(&f.context.db_path).unwrap();
    db.execute(
        "UPDATE agent_user_directives SET status='deferred' WHERE id=?1",
        [old],
    )
    .unwrap();
    let id = confirm_queue_directive(&db, &f.actor, "@mapper 然后 @investigator 评估已有证据");
    let remaining =
        crate::agent_runtime::multi_agent::budget::clock::remaining(&db, &f.actor.root_run_id)
            .unwrap();
    assert!(remaining <= Duration::from_secs(3));
    let early = std::thread::scope(|scope| {
        let (done_tx, done) = mpsc::channel();
        let f = &f;
        scope.spawn(move || {
            let _ = done_tx.send(ordered_exec_apply(f));
        });
        arrive.recv_timeout(Duration::from_secs(2)).unwrap();
        let early = done.recv_timeout(remaining + Duration::from_secs(1));
        release_tx.send(()).unwrap();
        match early {
            Ok(r) => Some(r),
            Err(_) => {
                let _ = done.recv_timeout(Duration::from_secs(5));
                None
            }
        }
    });
    assert!(early.is_some_and(|r| r.is_err()));
    // Endpoint capture occurs after its blocked handler returns.
    let capture_deadline = std::time::Instant::now() + Duration::from_secs(1);
    while seen.lock().unwrap().is_empty() && std::time::Instant::now() < capture_deadline {
        std::thread::yield_now();
    }
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(
        ordered_exec_count(&db, "agent_directive_ordered_receipts"),
        0
    );
    assert_eq!(
        ordered_exec_projection(&db, &id)["actions"][1]["state"],
        "not_started"
    );
}

#[test]
fn ordered_execution_next_dispatch_rechecks_predecessor_scope_budget_and_frozen_input() {
    for damage in [
      "UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1,fencing_token='changed-c'",
      "UPDATE sentinel_scans SET status='pausing'",
      "UPDATE agent_budget_ledger SET spent_tokens=total_tokens",
      "UPDATE agent_runs SET plan_json='{}' WHERE role='coordinator'",
      "UPDATE agent_messages SET acknowledged_at='' WHERE kind='human_ordered_assessment_result'",
      "DROP TRIGGER ordered_action_frozen; UPDATE agent_directive_ordered_actions SET input_json=json_set(input_json,'$.frozenEvidence.changed',1)",
      "DROP TRIGGER ordered_receipt_frozen; UPDATE agent_directive_ordered_receipts SET receipt_json=json_set(receipt_json,'$.workerId','wrong-worker')",
    ] {
      let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(|_|(200,"application/json",proposal_model_response(valid_proposal_text()))));
      let (f,id)=ordered_exec_fixture("ordered-second-negative",&format!("http://127.0.0.1:{port}/v1"));let db=db::open(&f.context.db_path).unwrap();
      ordered_exec_apply(&f).unwrap();let original=ordered_exec_receipts(&db,&id);
      db.execute_batch(damage).unwrap();assert!(ordered_exec_apply(&f).is_err(),"{damage}");
      assert_eq!(seen.lock().unwrap().len(),1);assert_eq!(ordered_exec_count(&db,"agent_directive_ordered_actions"),1);
      if !damage.contains("ordered_receipt") {assert_eq!(ordered_exec_receipts(&db,&id),original,"{damage}: original paid history is immutable");}
    }
}
