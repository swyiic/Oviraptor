struct ReviewReceiptFixture {
    root: std::path::PathBuf,
    context: AgentRunContext,
    lease: crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    candidate: JsonValue,
}

fn review_receipt_fixture() -> ReviewReceiptFixture {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };
    let (root, path, run_id, lease) = multi_agent_test_root("review-recovery", 60_000, 20);
    let connection = db::open(&path).unwrap();
    let mut context = test_context(&path, &lease.target_key, vec![]);
    context.scan_id = lease.scan_id.clone();
    context.run = Some(AgentRunLedger {
        db_path: path,
        run_id: run_id.clone(),
    });
    stage_agent_finding(
        &context,
        "web",
        "evidence",
        "recovery",
        "frozen title",
        "info",
        &serde_json::json!({"fact":"frozen"}),
    )
    .unwrap();
    let child = scheduler::schedule_child(
        &connection,
        &lease,
        AgentRole::EvidenceReviewer,
        AgentLane::Review,
        "candidate_ready",
        &serde_json::json!({"candidateId":"receipt","candidateRevision":1}),
        1,
        &["evidence.read".into(), "review.write".into()],
        8000,
        1,
    )
    .unwrap();
    scheduler::mark_child_running(&connection, &lease, &child).unwrap();
    let candidate = serde_json::json!({"findingCandidates":pending_agent_finding_candidates(&connection,&run_id).unwrap()});
    open_review_request(
        &connection,
        &lease,
        &child,
        "review-receipt",
        "receipt",
        1,
        &candidate.to_string(),
        &context.target_dir,
    )
    .unwrap();
    ReviewReceiptFixture {
        root,
        context,
        lease,
        child,
        candidate,
    }
}

fn review_receipt_response() -> String {
    serde_json::json!({"verdict":"confirmed","reasonCodes":["fixture_evidence"],"missingEvidence":[],
        "confidence":0.9,"summary":"reviewed"}).to_string()
}

fn assert_review_receipt_completed(connection: &rusqlite::Connection) {
    let state:(i64,i64,i64,i64,i64,i64,i64) = connection.query_row(
        "SELECT spent_tokens,spent_requests, \
         (SELECT COUNT(*) FROM agent_finding_candidates WHERE status='published'), \
         (SELECT COUNT(*) FROM sentinel_findings), \
         (SELECT COUNT(*) FROM agent_messages WHERE kind='review_decision' AND acknowledged_at<>'' AND delivery_attempts=1), \
         (SELECT COUNT(*) FROM agent_runs WHERE terminal_code='review_receipt_reconciled'), \
         (SELECT COUNT(*) FROM agent_assignments WHERE state='completed' AND budget_settled_at<>'' AND reserved_tokens=0 AND reserved_requests=0) \
         FROM agent_budget_ledger",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?)),
    ).unwrap();
    assert_eq!(state, (20, 1, 1, 1, 1, 1, 1));
    let live:i64 = connection.query_row("SELECT (SELECT COUNT(*) FROM agent_capability_leases WHERE revoked_at='') + (SELECT COUNT(*) FROM agent_lane_leases)",[],|r|r.get(0)).unwrap();
    assert_eq!(live, 0);
}

#[test]
fn reviewer_receipt_recovery_is_atomic_and_never_reactivates_or_recharges() {
    for settled in [false, true] {
        for fault in [
            "BEFORE UPDATE OF state ON agent_assignments WHEN NEW.state='completed'",
            "BEFORE UPDATE OF status ON agent_runs WHEN NEW.terminal_code='review_receipt_reconciled'",
            "BEFORE INSERT ON agent_review_decisions",
            "BEFORE UPDATE OF decision_id ON agent_review_requests",
            "BEFORE INSERT ON agent_messages",
            "BEFORE UPDATE OF acknowledged_at ON agent_messages",
            "BEFORE INSERT ON sentinel_findings",
            "BEFORE UPDATE OF status ON agent_finding_candidates WHEN NEW.status='published'",
            "BEFORE INSERT ON agent_collaboration_events",
            "BEFORE INSERT ON agent_collaboration_events WHEN NEW.event_type='agent_run'",
            "BEFORE INSERT ON agent_collaboration_events WHEN NEW.event_type='assignment'",
            "BEFORE INSERT ON agent_collaboration_events WHEN NEW.event_type='mailbox_message'",
            "BEFORE INSERT ON agent_collaboration_events WHEN NEW.event_type='review_gate'",
            "BEFORE UPDATE OF spent_tokens ON agent_budget_ledger",
            "BEFORE UPDATE OF used_tokens ON agent_runs",
            "BEFORE UPDATE OF budget_settled_at ON agent_assignments",
            "BEFORE DELETE ON agent_lane_leases",
            "", // Successful recovery without a fault.
        ] {
            if settled && (fault.contains("spent_tokens") || fault.contains("used_tokens")
                || fault.contains("budget_settled_at") || fault.contains("agent_lane_leases")) { continue; }
            for action in ["IGNORE","ABORT,'receipt recovery fault'"] {
                let mut f = review_receipt_fixture();
                let (port,seen,_stop) = spawn_endpoint(std::sync::Arc::new(|_|(200,"application/json",proposal_model_response(&review_receipt_response()))));
                f.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
                let connection = db::open(&f.context.db_path).unwrap();
                let (_,usage) = multi_agent_child_round_transport(&f.context,&f.lease,&f.child,"review fixture",f.candidate.clone()).unwrap();
                if settled { settle_child_usage(&connection,&f.lease,&f.child,&usage).unwrap(); }
                finish_failed_review(&connection,&f.lease,&f.child,"review-receipt","delivery interrupted").unwrap();
                connection.execute_batch("CREATE TRIGGER no_reactivation BEFORE UPDATE OF status ON agent_runs WHEN NEW.status='running' BEGIN SELECT RAISE(ABORT,'reactivation'); END;
                    CREATE TRIGGER no_new_capability BEFORE INSERT ON agent_capability_leases BEGIN SELECT RAISE(ABORT,'new capability'); END;
                    CREATE TRIGGER no_restored_capability BEFORE UPDATE OF revoked_at ON agent_capability_leases WHEN NEW.revoked_at='' BEGIN SELECT RAISE(ABORT,'restored capability'); END;
                    CREATE TRIGGER no_lease_renewal BEFORE UPDATE ON agent_coordinator_leases BEGIN SELECT RAISE(ABORT,'lease renewal'); END;").unwrap();
                let before = review_delivery_snapshot(&connection);
                if !fault.is_empty() {
                    connection.execute_batch(&format!("CREATE TRIGGER recovery_fault {fault} BEGIN SELECT RAISE({action}); END;")).unwrap();
                    let result = recover_received_review(&connection,&f.lease,"receipt",1,&f.candidate.to_string(),&f.context.target_dir);
                    assert!(result.is_err(),"{settled}/{fault}/{action}: {result:?}");
                    assert_eq!(before,review_delivery_snapshot(&connection),"{settled}/{fault}/{action}");
                    connection.execute_batch("DROP TRIGGER recovery_fault").unwrap();
                }
                recover_received_review(&connection,&f.lease,"receipt",1,&f.candidate.to_string(),&f.context.target_dir).unwrap();
                assert_review_receipt_completed(&connection);
                let before = review_delivery_snapshot(&connection);
                assert!(recover_received_review(&connection,&f.lease,"receipt",1,&f.candidate.to_string(),&f.context.target_dir).is_err());
                assert_eq!(before,review_delivery_snapshot(&connection));
                assert_eq!(seen.lock().unwrap().len(),1);
                drop(connection);
                std::fs::remove_dir_all(f.root).unwrap();
            }
        }
    }
}

#[test]
fn reviewer_receipt_recovery_rejects_changed_authority_or_receipt() {
    for change in [
        "UPDATE sentinel_scans SET status='paused'",
        "UPDATE sentinel_scans SET attempt_count=2",
        "UPDATE agent_runs SET status='terminal' WHERE role='coordinator'",
        "UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00'",
        "UPDATE agent_coordinator_leases SET fencing_token='other-owner'",
        "UPDATE agent_specialist_calls SET state='uncertain'",
        "UPDATE agent_specialist_calls SET response_json='{}'",
        "UPDATE agent_specialist_calls SET request_json='{}'",
        "UPDATE agent_assignments SET task_slice_json='{}'",
        "UPDATE agent_lane_leases SET lane='target_touching'",
        "DELETE FROM agent_lane_leases",
        "UPDATE agent_specialist_calls SET event_sequence=0",
        "UPDATE agent_specialist_calls SET response_hash='damaged'",
        "UPDATE agent_capability_leases SET revoked_at=''",
        "UPDATE agent_finding_candidates SET title='changed'",
        "UPDATE agent_review_requests SET candidate_json='{}'",
    ] {
        let mut f = review_receipt_fixture();
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
            (
                200,
                "application/json",
                proposal_model_response(&review_receipt_response()),
            )
        }));
        f.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
        let connection = db::open(&f.context.db_path).unwrap();
        multi_agent_child_round_transport(
            &f.context,
            &f.lease,
            &f.child,
            "review fixture",
            f.candidate.clone(),
        )
        .unwrap();
        finish_failed_review(
            &connection,
            &f.lease,
            &f.child,
            "review-receipt",
            "interrupted",
        )
        .unwrap();
        if change.contains("agent_specialist_calls") {
            // Normal writes cannot alter receipts. Simulate damaged storage
            // separately to exercise the recovery reader's integrity checks.
            assert!(connection.execute_batch(change).is_err());
            connection
                .execute_batch("DROP TRIGGER agent_specialist_call_immutable")
                .unwrap();
        }
        connection.execute_batch(change).unwrap();
        let before = review_delivery_snapshot(&connection);
        assert!(
            recover_received_review(
                &connection,
                &f.lease,
                "receipt",
                1,
                &f.candidate.to_string(),
                &f.context.target_dir
            )
            .is_err(),
            "{change}"
        );
        assert_eq!(before, review_delivery_snapshot(&connection), "{change}");
        assert_eq!(seen.lock().unwrap().len(), 1);
        drop(connection);
        std::fs::remove_dir_all(f.root).unwrap();
    }
}

#[test]
fn reviewer_receipt_recovery_requires_the_exact_frozen_model_input_and_valid_decision() {
    for invalid_input in [false, true] {
        let mut f = review_receipt_fixture();
        let response = if invalid_input {
            review_receipt_response()
        } else {
            "{}".into()
        };
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(move |_| {
            (200, "application/json", proposal_model_response(&response))
        }));
        f.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
        let connection = db::open(&f.context.db_path).unwrap();
        let input = if invalid_input {
            serde_json::json!({"findingCandidates":[]})
        } else {
            f.candidate.clone()
        };
        multi_agent_child_round_transport(&f.context, &f.lease, &f.child, "review fixture", input)
            .unwrap();
        finish_failed_review(
            &connection,
            &f.lease,
            &f.child,
            "review-receipt",
            "interrupted",
        )
        .unwrap();
        let before = review_delivery_snapshot(&connection);
        let error = recover_received_review(
            &connection,
            &f.lease,
            "receipt",
            1,
            &f.candidate.to_string(),
            &f.context.target_dir,
        )
        .unwrap_err();
        assert!(
            error.contains(if invalid_input {
                "candidate_input_mismatch"
            } else {
                "review_decision_invalid_shape"
            }),
            "{error}"
        );
        assert_eq!(before, review_delivery_snapshot(&connection));
        assert_eq!(seen.lock().unwrap().len(), 1);
        drop(connection);
        std::fs::remove_dir_all(f.root).unwrap();
    }
}

#[test]
fn reviewer_receipt_recovery_runs_in_production_and_replay_checks_actual_publication() {
    for persistent in [false, true] {
        let (root, path, run_id, lease) =
            multi_agent_test_root("review-recovery-production", 60_000, 20);
        let connection = db::open(&path).unwrap();
        let mut context = test_context(&path, &lease.target_key, vec![AgentIdentity::anonymous()]);
        context.scan_id = lease.scan_id.clone();
        context.run = Some(AgentRunLedger {
            db_path: path,
            run_id,
        });
        stage_agent_finding(
            &context,
            "web",
            "evidence",
            "production",
            "frozen title",
            "info",
            &serde_json::json!({"fact":"frozen"}),
        )
        .unwrap();
        let mut session = multi_agent_prepare(&mut context).unwrap();
        let outcome = || AgentTargetOutcome::incomplete("fixture executor concluded");
        multi_agent_finish_execution(&context, &mut session, &outcome()).unwrap();
        let condition = if persistent {
            "1"
        } else {
            "OLD.state='running'"
        };
        connection
            .execute_batch(&format!(
                "CREATE TRIGGER original_delivery_fault BEFORE UPDATE OF state ON agent_assignments
            WHEN NEW.role='evidence_reviewer' AND NEW.state='completed' AND {condition}
            BEGIN SELECT RAISE(ABORT,'original delivery fault'); END;"
            ))
            .unwrap();
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
            (
                200,
                "application/json",
                proposal_model_response(&review_receipt_response()),
            )
        }));
        context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
        let _real = RealSpecialistTransport::enter();
        let result = multi_agent_review(&context, &mut session, outcome());
        if persistent {
            assert!(
                matches!(result, AgentTargetOutcome::Failed(_)),
                "{result:?}"
            );
            connection
                .execute_batch("DROP TRIGGER original_delivery_fault")
                .unwrap();
            let result = multi_agent_review(&context, &mut session, outcome());
            assert!(
                matches!(result,AgentTargetOutcome::Incomplete(ref reason) if reason.reason=="fixture executor concluded"),
                "{result:?}"
            );
        } else {
            assert!(
                matches!(result,AgentTargetOutcome::Incomplete(ref reason) if reason.reason=="fixture executor concluded"),
                "{result:?}"
            );
        }
        let row:(i64,i64,i64) = connection.query_row("SELECT used_tokens,used_requests,(SELECT COUNT(*) FROM sentinel_findings) FROM agent_runs WHERE role='evidence_reviewer' AND terminal_code='review_receipt_reconciled'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(row, (20, 1, 1));
        let before = review_delivery_snapshot(&connection);
        multi_agent_review(&context, &mut session, outcome());
        assert_eq!(before, review_delivery_snapshot(&connection));
        // Same status is insufficient when either the published row or the
        // candidate diverges from what the Reviewer actually saw.
        for corrupt in ["UPDATE sentinel_findings SET title='tampered'",
            "UPDATE agent_finding_candidates SET title='tampered'; UPDATE sentinel_findings SET title='tampered'",
            "DELETE FROM sentinel_findings"] {
            connection.execute_batch(corrupt).unwrap();
            let before = review_delivery_snapshot(&connection);
            let result = multi_agent_review(&context,&mut session,outcome());
            assert!(matches!(result,AgentTargetOutcome::Incomplete(ref reason) if reason.reason.contains("publication_incomplete")),"{corrupt}: {result:?}");
            assert_eq!(before,review_delivery_snapshot(&connection));
            // Restore the fixture between corruptions so the missing-row case
            // cannot pass solely because a preceding title mismatch remains.
            connection.execute_batch("UPDATE agent_finding_candidates SET title='frozen title'; UPDATE sentinel_findings SET title='frozen title'").unwrap();
        }
        assert_eq!(seen.lock().unwrap().len(), 1);
        drop(connection);
        std::fs::remove_dir_all(root).unwrap();
    }
}
