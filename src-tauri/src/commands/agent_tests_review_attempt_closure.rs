struct ReviewAttemptClosureCase<'a> {
    context: &'a mut AgentRunContext,
    lease: &'a crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &'a crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    candidate: JsonValue,
    candidate_id: &'a str,
    request_id: &'a str,
    response: String,
}

impl ReviewAttemptClosureCase<'_> {
    fn exercise(&mut self, table: &str, damaged_table: &str, mutation: &str, recovery: bool) {
        use crate::agent_runtime::multi_agent::attempts;
        let response = self.response.clone();
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(move |_| {
            (200, "application/json", proposal_model_response(&response))
        }));
        self.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
        let connection = db::open(&self.context.db_path).unwrap();
        let (text, usage) = multi_agent_child_round_transport(
            self.context,
            self.lease,
            self.child,
            "review fixture",
            self.candidate.clone(),
        )
        .unwrap();
        settle_child_usage(&connection, self.lease, self.child, &usage).unwrap();
        let decision = validated_review_decision(&text).unwrap();
        if recovery {
            finish_failed_review(
                &connection,
                self.lease,
                self.child,
                self.request_id,
                "delivery interrupted",
            )
            .unwrap();
            // Local publication of a saved result is not an execution grant.
            connection.execute(
                "UPDATE agent_assignment_attempts SET expires_at='2000-01-01' WHERE child_run_id=?1",
                [&self.child.run_id],
            ).unwrap();
            connection.execute_batch("CREATE TRIGGER closure_no_reactivation BEFORE UPDATE OF status ON agent_runs WHEN NEW.status='running' BEGIN SELECT RAISE(ABORT,'reactivation'); END;
                CREATE TRIGGER closure_no_capability BEFORE INSERT ON agent_capability_leases BEGIN SELECT RAISE(ABORT,'new capability'); END;
                CREATE TRIGGER closure_no_restored_capability BEFORE UPDATE OF revoked_at ON agent_capability_leases WHEN NEW.revoked_at='' BEGIN SELECT RAISE(ABORT,'restored capability'); END;
                CREATE TRIGGER closure_no_lease_renewal BEFORE UPDATE ON agent_coordinator_leases BEGIN SELECT RAISE(ABORT,'lease renewal'); END;"
            ).unwrap();
        }
        let original =
            attempts::current(&connection, self.lease, &self.child.assignment_id).unwrap();
        let before = super::tests::application_table_snapshot(&connection);
        let (key, id) = match damaged_table {
            "agent_assignment_attempts" => ("child_run_id", &self.child.run_id),
            "agent_assignments" => ("id", &self.child.assignment_id),
            "agent_runs" => ("id", &self.child.run_id),
            "agent_coordinator_leases" => ("root_run_id", &self.lease.root_run_id),
            _ => panic!("unexpected closed row"),
        };
        connection
            .execute_batch(&format!(
                "CREATE TRIGGER final_attempt_fault AFTER INSERT ON {table} BEGIN
             UPDATE {damaged_table} SET {mutation} WHERE {key}='{id}'; END;",
            ))
            .unwrap();
        let result = if recovery {
            recover_received_review(
                &connection,
                self.lease,
                self.candidate_id,
                1,
                &self.candidate.to_string(),
                &self.context.target_dir,
            )
            .map(|_| ())
        } else {
            complete_review_delivery(
                &connection,
                self.lease,
                self.child,
                self.request_id,
                self.candidate_id,
                1,
                &decision,
                &self.context.target_dir,
            )
        };
        assert!(
            result.is_err(),
            "final {table}/{mutation}/recovery={recovery} escaped: {result:?}"
        );
        assert_eq!(
            before,
            super::tests::application_table_snapshot(&connection),
            "final {table}/{mutation}/recovery={recovery} must roll back every business table"
        );
        connection
            .execute_batch("DROP TRIGGER final_attempt_fault")
            .unwrap();
        if recovery {
            recover_received_review(
                &connection,
                self.lease,
                self.candidate_id,
                1,
                &self.candidate.to_string(),
                &self.context.target_dir,
            )
            .unwrap();
        } else {
            complete_review_delivery(
                &connection,
                self.lease,
                self.child,
                self.request_id,
                self.candidate_id,
                1,
                &decision,
                &self.context.target_dir,
            )
            .unwrap();
        }
        let closed = attempts::current(&connection, self.lease, &self.child.assignment_id).unwrap();
        if recovery {
            assert_eq!(
                closed, original,
                "failed worker audit must remain unchanged after saved publication"
            );
        } else {
            assert_eq!(closed.id, original.id);
            assert_eq!(closed.worker_id, original.worker_id);
            assert_eq!(closed.fencing_token, original.fencing_token);
            assert_eq!(closed.expires_at, original.expires_at);
            assert_eq!(closed.state, "completed");
            assert!(!closed.finished_at.is_empty());
            assert!(closed.failure_class.is_empty());
        }
        assert!(attempts::require_live_for_run(&connection, &self.child.run_id).is_err());
        assert_eq!(
            seen.lock().unwrap().len(),
            1,
            "local retry must not request another model result"
        );
    }
}

#[test]
fn reviewer_final_publication_preserves_the_same_closed_attempt() {
    for recovery in [false, true] {
        for mutation in [
            "finished_at=''",
            "failure_class='damaged-after-publication'",
            "heartbeat_at='2000-01-01'",
            "expires_at='2099-01-01'",
        ] {
            let mut f = review_receipt_fixture();
            ReviewAttemptClosureCase {
                context: &mut f.context,
                lease: &f.lease,
                child: &f.child,
                candidate: f.candidate,
                candidate_id: "receipt",
                request_id: "review-receipt",
                response: review_receipt_response(),
            }
            .exercise(
                "sentinel_findings",
                "agent_assignment_attempts",
                mutation,
                recovery,
            );
            std::fs::remove_dir_all(f.root).unwrap();
        }
    }
}

#[test]
fn reviewer_final_gap_receipt_preserves_the_same_closed_attempt() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };
    for recovery in [false, true] {
        for mutation in [
            "finished_at=''",
            "failure_class='damaged-after-gap-receipt'",
        ] {
            let mut f = gap_review_fixture(true);
            let connection = db::open(&f.context.db_path).unwrap();
            let candidate = {
                let tx = connection.unchecked_transaction().unwrap();
                json!({"findingCandidates":[],
                    "gapFollowup":gap_followup_review_context(&tx,&f.session.lease.root_run_id,&f.context.target_dir).unwrap(),
                    "persistedFactRefs":review_fact_refs(&tx,&f.session.lease.root_run_id,&f.context.target_dir).unwrap()})
            };
            let child = scheduler::schedule_child(
                &connection,
                &f.session.lease,
                AgentRole::EvidenceReviewer,
                AgentLane::Review,
                "candidate_ready",
                &json!({"candidateId":"closure-gap","candidateRevision":1}),
                1,
                &["evidence.read".into(), "review.write".into()],
                8000,
                1,
            )
            .unwrap();
            scheduler::mark_child_running(&connection, &f.session.lease, &child).unwrap();
            open_review_request(
                &connection,
                &f.session.lease,
                &child,
                "review-closure-gap",
                "closure-gap",
                1,
                &candidate.to_string(),
                &f.context.target_dir,
            )
            .unwrap();
            let response = gap_review_response(&candidate, "rejected").to_string();
            ReviewAttemptClosureCase {
                context: &mut f.context,
                lease: &f.session.lease,
                child: &child,
                candidate,
                candidate_id: "closure-gap",
                request_id: "review-closure-gap",
                response,
            }
            .exercise(
                "agent_gap_review_receipts",
                "agent_assignment_attempts",
                mutation,
                recovery,
            );
        }
    }
}

#[test]
fn reviewer_final_publication_preserves_original_closed_run_and_assignment() {
    for recovery in [false, true] {
        for (table, mutation) in [
            ("agent_assignments", "finished_at=''"),
            (
                "agent_assignments",
                "failure_class='damaged-after-publication'",
            ),
            ("agent_runs", "finished_at=''"),
            ("agent_runs", "terminal_reason='damaged-after-publication'"),
        ] {
            let mut f = review_receipt_fixture();
            ReviewAttemptClosureCase {
                context: &mut f.context,
                lease: &f.lease,
                child: &f.child,
                candidate: f.candidate,
                candidate_id: "receipt",
                request_id: "review-receipt",
                response: review_receipt_response(),
            }
            .exercise("sentinel_findings", table, mutation, recovery);
            std::fs::remove_dir_all(f.root).unwrap();
        }
    }
}

#[test]
fn reviewer_final_publication_preserves_original_coordinator_authority() {
    for mutation in [
        "fencing_token='replaced-after-publication'",
        "lease_expires_at='2000-01-01'",
    ] {
        let mut f = review_receipt_fixture();
        ReviewAttemptClosureCase {
            context: &mut f.context,
            lease: &f.lease,
            child: &f.child,
            candidate: f.candidate,
            candidate_id: "receipt",
            request_id: "review-receipt",
            response: review_receipt_response(),
        }
        .exercise(
            "sentinel_findings",
            "agent_coordinator_leases",
            mutation,
            false,
        );
        std::fs::remove_dir_all(f.root).unwrap();
    }
}

#[test]
fn reviewer_final_attempt_proof_does_not_admit_expired_live_delivery() {
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
    let (text, usage) = multi_agent_child_round_transport(
        &f.context,
        &f.lease,
        &f.child,
        "review fixture",
        f.candidate.clone(),
    )
    .unwrap();
    settle_child_usage(&connection, &f.lease, &f.child, &usage).unwrap();
    connection
        .execute(
            "UPDATE agent_assignment_attempts SET expires_at='2000-01-01' WHERE child_run_id=?1",
            [&f.child.run_id],
        )
        .unwrap();
    let before = super::tests::application_table_snapshot(&connection);
    let decision = validated_review_decision(&text).unwrap();
    assert!(complete_review_delivery(
        &connection,
        &f.lease,
        &f.child,
        "review-receipt",
        "receipt",
        1,
        &decision,
        &f.context.target_dir,
    )
    .is_err());
    assert_eq!(
        before,
        super::tests::application_table_snapshot(&connection)
    );
    assert_eq!(seen.lock().unwrap().len(), 1);
    drop(connection);
    std::fs::remove_dir_all(f.root).unwrap();
}
