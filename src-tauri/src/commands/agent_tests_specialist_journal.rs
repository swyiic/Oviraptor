fn specialist_journal_fixture() -> (
    std::path::PathBuf,
    AgentRunContext,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
) {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };
    let (root, path, run_id, lease) = multi_agent_test_root("specialist-journal", 60_000, 20);
    let connection = db::open(&path).unwrap();
    let child = scheduler::schedule_child(
        &connection,
        &lease,
        AgentRole::SpaApiMapper,
        AgentLane::ReadOnlyAnalysis,
        "journal-test",
        &serde_json::json!({"fixture":true}),
        1,
        &["evidence.read".into(), "mailbox.write".into()],
        8000,
        1,
    )
    .unwrap();
    scheduler::start_child_or_release(&connection, &lease, &child).unwrap();
    let mut context = test_context(&path, &lease.target_key, vec![AgentIdentity::anonymous()]);
    context.scan_id = lease.scan_id.clone();
    context.run = Some(AgentRunLedger {
        db_path: path,
        run_id,
    });
    (root, context, lease, child)
}

#[test]
fn specialist_journal_real_replay_does_not_call_charge_or_restore_permissions() {
    let (root, mut context, lease, child) = specialist_journal_fixture();
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            proposal_model_response("{\"summary\":\"saved\"}"),
        )
    }));
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let result = multi_agent_child_round_transport(
        &context,
        &lease,
        &child,
        "readonly",
        serde_json::json!({"evidence":"fixed"}),
    )
    .unwrap();
    let connection = db::open(&context.db_path).unwrap();
    stop_failed_child_preserving_usage(
        &connection,
        &lease,
        &child,
        "fixture interruption after receipt",
    )
    .unwrap();
    for _ in 0..2 {
        let replay = multi_agent_child_round_transport(
            &context,
            &lease,
            &child,
            "readonly",
            serde_json::json!({"evidence":"fixed"}),
        )
        .unwrap();
        assert_eq!(replay, result);
    }
    assert_specialist_usage_pending(&connection, "spa_api_mapper", 8000);
    assert_eq!(seen.lock().unwrap().len(), 1);
    let events:i64=connection.query_row("SELECT COUNT(*) FROM agent_events WHERE run_id=?1 AND event_type='model_round_completed'",[&child.run_id],|r|r.get(0)).unwrap();
    assert_eq!(events, 1);
    let spent: (i64, i64) = connection
        .query_row(
            "SELECT spent_tokens,spent_requests FROM agent_budget_ledger",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(spent, (0, 0), "receipt is not yet business settlement");
    let error = multi_agent_child_round_transport(
        &context,
        &lease,
        &child,
        "readonly",
        serde_json::json!({"evidence":"changed"}),
    )
    .unwrap_err();
    assert!(error.contains("request_changed"), "{error}");
    assert_eq!(seen.lock().unwrap().len(), 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn specialist_journal_concurrent_transport_dispatches_once() {
    let (root, mut context, lease, child) = specialist_journal_fixture();
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (200, "application/json", proposal_model_response("saved"))
    }));
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let context = context.clone();
            let lease = lease.clone();
            let child = child.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                multi_agent_child_round_transport(
                    &context,
                    &lease,
                    &child,
                    "readonly",
                    serde_json::json!({}),
                )
            })
        })
        .collect();
    barrier.wait();
    let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert!(results.iter().any(Result::is_ok), "{results:?}");
    for error in results.iter().filter_map(|r| r.as_ref().err()) {
        assert!(
            error.contains("outcome_unknown_requires_reconciliation")
                || error.contains("specialist_transport_not_idle"),
            "{error}"
        );
    }
    assert_eq!(seen.lock().unwrap().len(), 1);
    let replay = multi_agent_child_round_transport(
        &context,
        &lease,
        &child,
        "readonly",
        serde_json::json!({}),
    )
    .unwrap();
    assert_eq!(replay.1.total_tokens, 20);
    assert_eq!(seen.lock().unwrap().len(), 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn specialist_journal_unknown_call_is_durable_and_not_retried() {
    let (root, mut context, lease, child) = specialist_journal_fixture();
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            503,
            "application/json",
            "{\"error\":\"unavailable\"}".into(),
        )
    }));
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    assert!(multi_agent_child_round_transport(
        &context,
        &lease,
        &child,
        "readonly",
        serde_json::json!({})
    )
    .is_err());
    let error = multi_agent_child_round_transport(
        &context,
        &lease,
        &child,
        "readonly",
        serde_json::json!({}),
    )
    .unwrap_err();
    assert!(
        error.contains("outcome_unknown_requires_reconciliation"),
        "{error}"
    );
    assert_eq!(seen.lock().unwrap().len(), 1);
    let connection = db::open(&context.db_path).unwrap();
    let state: String = connection
        .query_row("SELECT state FROM agent_specialist_calls", [], |r| r.get(0))
        .unwrap();
    assert_eq!(state, "uncertain");
    assert!(
        crate::agent_runtime::store::read_snapshot(&connection, &child.run_id)
            .unwrap()
            .is_none()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn specialist_journal_response_event_and_snapshot_failures_are_atomic() {
    for action in ["RAISE(ABORT,'injected_receipt')", "RAISE(IGNORE)"] {
        for target in [
            "BEFORE UPDATE OF state ON agent_specialist_calls WHEN NEW.state='received'",
            "BEFORE INSERT ON agent_events WHEN NEW.event_type='model_round_completed'",
            "BEFORE INSERT ON agent_snapshots",
        ] {
            let (root, mut context, lease, child) = specialist_journal_fixture();
            let connection = db::open(&context.db_path).unwrap();
            connection
                .execute_batch(&format!(
                    "CREATE TRIGGER break_receipt {target} BEGIN SELECT {action}; END;"
                ))
                .unwrap();
            let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
                (200, "application/json", proposal_model_response("saved"))
            }));
            context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
            assert!(
                multi_agent_child_round_transport(
                    &context,
                    &lease,
                    &child,
                    "readonly",
                    serde_json::json!({})
                )
                .is_err(),
                "{target}: {action}"
            );
            let state:(String,i64,i64)=connection.query_row("SELECT state,(SELECT COUNT(*) FROM agent_events WHERE event_type='model_round_completed'),(SELECT COUNT(*) FROM agent_snapshots) FROM agent_specialist_calls",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
            assert_eq!(state, ("executing".into(), 0, 0), "{target}: {action}");
            connection
                .execute_batch("DROP TRIGGER break_receipt")
                .unwrap();
            let error = multi_agent_child_round_transport(
                &context,
                &lease,
                &child,
                "readonly",
                serde_json::json!({}),
            )
            .unwrap_err();
            assert!(
                error.contains("outcome_unknown_requires_reconciliation"),
                "{error}"
            );
            assert_eq!(seen.lock().unwrap().len(), 1);
            let _ = fs::remove_dir_all(root);
        }
    }
}

#[test]
fn specialist_journal_dispatch_must_be_durable_before_http() {
    for action in ["RAISE(ABORT,'injected_dispatch')", "RAISE(IGNORE)"] {
        let (root, mut context, lease, child) = specialist_journal_fixture();
        let connection = db::open(&context.db_path).unwrap();
        connection.execute_batch(&format!("CREATE TRIGGER break_dispatch BEFORE INSERT ON agent_specialist_calls BEGIN SELECT {action}; END;")).unwrap();
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
            (200, "application/json", proposal_model_response("saved"))
        }));
        context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
        assert!(multi_agent_child_round_transport(
            &context,
            &lease,
            &child,
            "readonly",
            serde_json::json!({})
        )
        .is_err());
        assert_eq!(seen.lock().unwrap().len(), 0);
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn specialist_journal_replaced_fence_cannot_record_late_response() {
    let (root, mut context, lease, child) = specialist_journal_fixture();
    let path = context.db_path.clone();
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(move |_| {
        let connection = db::open(&path).unwrap();
        connection
            .execute(
                "UPDATE agent_coordinator_leases SET fencing_token='replacement'",
                [],
            )
            .unwrap();
        (200, "application/json", proposal_model_response("late"))
    }));
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let error = multi_agent_child_round_transport(
        &context,
        &lease,
        &child,
        "readonly",
        serde_json::json!({}),
    )
    .unwrap_err();
    assert!(error.contains("stale_coordinator_fencing_token"), "{error}");
    assert_eq!(seen.lock().unwrap().len(), 1);
    let connection = db::open(&context.db_path).unwrap();
    let state: (String, i64) = connection
        .query_row(
            "SELECT state,event_sequence FROM agent_specialist_calls",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, ("executing".into(), 0));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn specialist_journal_corrupt_receipts_are_not_replayed() {
    for corruption in [
        "UPDATE agent_specialist_calls SET response_json='{}'",
        "UPDATE agent_specialist_calls SET usage_json='{}'",
        "UPDATE agent_specialist_calls SET request_json='{}'",
        "UPDATE agent_events SET payload_json='{}' WHERE event_type='model_round_completed'",
        "UPDATE agent_snapshots SET snapshot_json='{}'",
    ] {
        let (root, mut context, lease, child) = specialist_journal_fixture();
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
            (200, "application/json", proposal_model_response("saved"))
        }));
        context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
        multi_agent_child_round_transport(
            &context,
            &lease,
            &child,
            "readonly",
            serde_json::json!({}),
        )
        .unwrap();
        let connection = db::open(&context.db_path).unwrap();
        assert!(
            connection
                .execute("UPDATE agent_specialist_calls SET response_json='{}'", [])
                .is_err(),
            "receipt must be immutable"
        );
        connection
            .execute_batch("DROP TRIGGER agent_specialist_call_immutable")
            .unwrap();
        connection.execute_batch(corruption).unwrap();
        assert!(
            multi_agent_child_round_transport(
                &context,
                &lease,
                &child,
                "readonly",
                serde_json::json!({})
            )
            .is_err(),
            "{corruption}"
        );
        assert_eq!(seen.lock().unwrap().len(), 1);
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn specialist_journal_dispatch_rejects_inactive_revoked_or_legacy_work() {
    for mutation in [
        "UPDATE sentinel_scans SET status='paused'",
        "UPDATE sentinel_scans SET attempt_count=2",
        "INSERT INTO sentinel_deleted_scans(scan_id) SELECT id FROM sentinel_scans",
        "UPDATE agent_runs SET status='terminal' WHERE role='coordinator'",
        "UPDATE agent_assignments SET state='paused'",
        "UPDATE agent_capability_leases SET revoked_at='revoked'",
        "UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01'",
        "INSERT INTO agent_events(run_id,sequence,event_type,payload_json) SELECT id,1,'model_round_completed','{}' FROM agent_runs WHERE role='spa_api_mapper'",
    ] {
        let (root,mut context,lease,child)=specialist_journal_fixture();
        let connection=db::open(&context.db_path).unwrap();
        connection.execute_batch(mutation).unwrap();
        let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(|_|(200,"application/json",proposal_model_response("saved"))));
        context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
        assert!(multi_agent_child_round_transport(&context,&lease,&child,"readonly",serde_json::json!({})).is_err(),"{mutation}");
        assert_eq!(seen.lock().unwrap().len(),0,"{mutation}");
        let count:i64=connection.query_row("SELECT COUNT(*) FROM agent_specialist_calls",[],|r|r.get(0)).unwrap();
        assert_eq!(count,0);
        let _=fs::remove_dir_all(root);
    }
}

#[test]
fn specialist_journal_unknown_receipt_write_error_preserves_provider_error() {
    let (root, mut context, lease, child) = specialist_journal_fixture();
    let connection = db::open(&context.db_path).unwrap();
    connection.execute_batch("CREATE TRIGGER break_unknown BEFORE UPDATE OF state ON agent_specialist_calls WHEN NEW.state='uncertain' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            503,
            "application/json",
            "{\"error\":\"unavailable\"}".into(),
        )
    }));
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let error = multi_agent_child_round_transport(
        &context,
        &lease,
        &child,
        "readonly",
        serde_json::json!({}),
    )
    .unwrap_err();
    assert!(
        error.contains("503") && error.contains("specialist_uncertain_persist_missing"),
        "{error}"
    );
    assert!(multi_agent_child_round_transport(
        &context,
        &lease,
        &child,
        "readonly",
        serde_json::json!({})
    )
    .unwrap_err()
    .contains("outcome_unknown_requires_reconciliation"));
    assert_eq!(seen.lock().unwrap().len(), 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn specialist_journal_completion_is_atomic_and_replay_settles_once() {
    for target in [
        "BEFORE UPDATE OF spent_tokens ON agent_budget_ledger",
        "BEFORE UPDATE OF state ON agent_assignments WHEN NEW.state='completed'",
        "BEFORE UPDATE OF status ON agent_runs WHEN NEW.status='terminal'",
        "BEFORE UPDATE OF revoked_at ON agent_capability_leases",
        "BEFORE DELETE ON agent_lane_leases",
        "BEFORE INSERT ON agent_messages",
        "BEFORE UPDATE OF acknowledged_at ON agent_messages",
    ] {
        let (root, mut context, lease, child) = specialist_journal_fixture();
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
            (200, "application/json", proposal_model_response("saved"))
        }));
        context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
        let (text, usage) = multi_agent_child_round_transport(
            &context,
            &lease,
            &child,
            "readonly",
            serde_json::json!({}),
        )
        .unwrap();
        let connection = db::open(&context.db_path).unwrap();
        let payload = serde_json::json!({"summary":text});
        connection
            .execute_batch(&format!(
                "CREATE TRIGGER break_completion {target} BEGIN SELECT RAISE(IGNORE); END;"
            ))
            .unwrap();
        assert!(
            complete_readonly_assessment(&connection, &lease, &child, &usage, &payload).is_err(),
            "{target}"
        );
        let row:(String,String,i64,i64,i64,i64)=connection.query_row(
            "SELECT a.state,r.status,a.reserved_tokens,b.spent_tokens,(SELECT COUNT(*) FROM agent_messages), \
             (SELECT COUNT(*) FROM agent_capability_leases WHERE revoked_at='') \
             FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id JOIN agent_budget_ledger b ON b.root_run_id=a.coordinator_run_id",
            [],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).unwrap();
        assert_eq!(
            row,
            ("running".into(), "running".into(), 8000, 0, 0, 2),
            "{target}"
        );
        connection
            .execute_batch("DROP TRIGGER break_completion")
            .unwrap();
        let replay = multi_agent_child_round_transport(
            &context,
            &lease,
            &child,
            "readonly",
            serde_json::json!({}),
        )
        .unwrap();
        assert_eq!((text, usage), replay);
        for _ in 0..2 {
            assert_eq!(
                complete_readonly_assessment(&connection, &lease, &child, &usage, &payload)
                    .unwrap(),
                payload
            );
        }
        let final_state:(i64,i64,i64,i64)=connection.query_row(
            "SELECT spent_tokens,spent_requests,(SELECT COUNT(*) FROM agent_messages WHERE acknowledged_at<>'' AND delivery_attempts=1),(SELECT COUNT(*) FROM agent_lane_leases) FROM agent_budget_ledger",
            [],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(final_state, (20, 1, 1, 0));
        assert!(complete_readonly_assessment(
            &connection,
            &lease,
            &child,
            &usage,
            &serde_json::json!({"summary":"changed"})
        )
        .is_err());
        assert_eq!(seen.lock().unwrap().len(), 1);
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn specialist_journal_pipeline_mailbox_failure_preserves_saved_response_and_budget() {
    for identity in [false, true] {
        let (port, seen, _stop) = specialist_original_endpoint(std::sync::Arc::new(|_| {
            (
                200,
                "application/json",
                proposal_model_response("{\"summary\":\"saved\"}"),
            )
        }));
        let (root, mut context, lease) = specialist_original_context("specialist-pipeline-failure", &format!("http://127.0.0.1:{port}/v1"));
        let connection = db::open(&context.db_path).unwrap();
        if identity {
            configure_specialist_identity(&connection, &lease);
        }
        let kind = if identity {
            "identity_assessment"
        } else {
            "evidence_summary"
        };
        connection.execute_batch(&format!("CREATE TRIGGER break_result BEFORE UPDATE OF acknowledged_at ON agent_messages WHEN NEW.kind='{kind}' BEGIN SELECT RAISE(IGNORE); END;")).unwrap();
        context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
        let _real = RealSpecialistTransport::enter();
        assert!(multi_agent_prepare(&mut context).is_err());
        assert_specialist_usage_pending(
            &connection,
            if identity {
                "identity_session"
            } else {
                "spa_api_mapper"
            },
            if identity { 4000 } else { 8000 },
        );
        let received: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM agent_specialist_calls WHERE state='received'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(received, if identity { 2 } else { 1 });
        assert_eq!(specialist_child_call_count(&seen), received as usize);
        let missing: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM agent_messages WHERE kind=?1",
                [kind],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            missing, 0,
            "failed result delivery cannot leave a completed-looking message"
        );
        let _ = fs::remove_dir_all(root);
    }
}
