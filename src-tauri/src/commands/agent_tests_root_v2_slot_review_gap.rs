fn root_v2_slot_review_open(
    h: &AgentHarness,
    session: &MultiAgentSession,
    candidate_id: &str,
    revision: i64,
) -> (
    crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    String,
    JsonValue,
) {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };
    let db = db::open(&h.db_path).unwrap();
    let child = scheduler::schedule_child(
        &db,
        &session.lease,
        AgentRole::EvidenceReviewer,
        AgentLane::Review,
        "candidate_ready",
        &json!({"candidateId":candidate_id,"candidateRevision":revision}),
        revision,
        &["evidence.read".into(), "review.write".into()],
        8000,
        1,
    )
    .unwrap();
    scheduler::mark_child_running(&db, &session.lease, &child).unwrap();
    let request = format!("v2-review-{candidate_id}-{revision}");
    let candidate = json!({"evidence":h.context.evidence,
        "persistedFactRefs":review_fact_refs(&db,&session.lease.root_run_id,&h.context.target_dir).unwrap(),
        "findingCandidates":[]});
    open_review_request(
        &db,
        &session.lease,
        &child,
        &request,
        candidate_id,
        revision,
        &candidate.to_string(),
        &h.context.target_dir,
    )
    .unwrap();
    (child, request, candidate)
}

fn root_v2_slot_insufficient_review() -> String {
    json!({"verdict":"insufficient_evidence","reasonCodes":["missing_control"],
        "missingEvidence":["missing control"],"confidence":0.2,
        "summary":"needs an independent control"})
    .to_string()
}

fn root_v2_slot_no_reactivation(db: &rusqlite::Connection) {
    db.execute_batch(
        "CREATE TRIGGER v2_no_saved_reactivation BEFORE UPDATE OF status ON agent_runs
        WHEN NEW.status='running' BEGIN SELECT RAISE(ABORT,'reactivation forbidden'); END;
        CREATE TRIGGER v2_no_saved_capability BEFORE INSERT ON agent_capability_leases
        BEGIN SELECT RAISE(ABORT,'new capability forbidden'); END;
        CREATE TRIGGER v2_no_saved_restore BEFORE UPDATE OF revoked_at ON agent_capability_leases
        WHEN NEW.revoked_at='' BEGIN SELECT RAISE(ABORT,'capability restoration forbidden'); END;
        CREATE TRIGGER v2_no_saved_renew BEFORE UPDATE ON agent_coordinator_leases
        BEGIN SELECT RAISE(ABORT,'coordinator renewal forbidden'); END;",
    )
    .unwrap();
}

#[test]
fn root_v2_slot_saved_reviewer_requires_held_lane_before_release_and_never_resends() {
    use crate::agent_runtime::multi_agent::{attempts, budget};
    for ignore_delete in [false, true] {
        let mut h = root_v2_slot_harness("v2-saved-reviewer", 1);
        root_v2_slot_retarget_model(
            &mut h,
            vec![
                proposal_model_response("{\"summary\":\"frozen evidence\",\"priorityContracts\":[],\"risks\":[]}"),
                proposal_model_response(&root_v2_slot_insufficient_review()),
            ],
        );
        let _real = RealSpecialistTransport::enter();
        let mut session = multi_agent_prepare(&mut h.context).unwrap();
        multi_agent_finish_execution(
            &h.context,
            &mut session,
            &AgentTargetOutcome::incomplete("no target execution"),
        )
        .unwrap();
        let (child, request, candidate) = root_v2_slot_review_open(&h, &session, "v2-reviewer", 1);
        let db = db::open(&h.db_path).unwrap();
        let (text, _) = multi_agent_child_round_transport(
            &h.context,
            &session.lease,
            &child,
            "Independent Reviewer; frozen evidence only and no tools.",
            candidate.clone(),
        )
        .unwrap();
        assert_eq!(text, root_v2_slot_insufficient_review());
        assert_eq!(h.model_seen.lock().unwrap().len(), 4);
        finish_failed_review(
            &db,
            &session.lease,
            &child,
            &request,
            "saved response delivery interrupted",
        )
        .unwrap();
        let original = attempts::current(&db, &session.lease, &child.assignment_id).unwrap();
        root_v2_slot_no_reactivation(&db);
        if ignore_delete {
            db.execute_batch("CREATE TRIGGER v2_ignore_saved_lane BEFORE DELETE ON agent_lane_leases BEGIN SELECT RAISE(IGNORE); END;").unwrap();
            let before = super::tests::application_table_snapshot(&db);
            assert!(recover_received_review(
                &db,
                &session.lease,
                "v2-reviewer",
                1,
                &candidate.to_string(),
                &h.context.target_dir
            )
            .is_err());
            assert!(super::tests::application_table_snapshot(&db) == before);
            assert_eq!(h.model_seen.lock().unwrap().len(), 4);
            db.execute_batch("DROP TRIGGER v2_ignore_saved_lane")
                .unwrap();
        }
        recover_received_review(
            &db,
            &session.lease,
            "v2-reviewer",
            1,
            &candidate.to_string(),
            &h.context.target_dir,
        )
        .unwrap();
        let after = attempts::current(&db, &session.lease, &child.assignment_id).unwrap();
        assert_eq!(after.id, original.id);
        assert_eq!(after.child_run_id, original.child_run_id);
        assert_eq!(after.worker_id, original.worker_id);
        assert_eq!(after.state, "completed");
        assert_eq!(
            budget::balance(&db, &session.lease.root_run_id, None, "concurrency_batches").unwrap(),
            budget::Balance::default()
        );
        assert_eq!(
            budget::balance(
                &db,
                &session.lease.root_run_id,
                Some(&child.assignment_id),
                "model_requests"
            )
            .unwrap()
            .consumed,
            1
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM agent_lane_leases WHERE assignment_id=?1",
                [&child.assignment_id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM agent_review_decisions WHERE reviewer_run_id=?1",
                [&child.run_id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        let before = super::tests::application_table_snapshot(&db);
        assert!(recover_received_review(
            &db,
            &session.lease,
            "v2-reviewer",
            1,
            &candidate.to_string(),
            &h.context.target_dir
        )
        .is_err());
        assert!(super::tests::application_table_snapshot(&db) == before);
        assert_eq!(h.model_seen.lock().unwrap().len(), 4);
        assert!(h.site_seen.lock().unwrap().is_empty());
        drop(session);
        drop(db);
        fs::remove_dir_all(h.root).unwrap();
    }
}

#[test]
fn root_v2_slot_saved_investigator_releases_held_lane_with_actual_reviewer_receipt() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{attempts, budget, scheduler},
    };
    for ignore_delete in [false, true] {
        let mut h = root_v2_slot_harness("v2-saved-investigator", 1);
        root_v2_slot_retarget_model(
            &mut h,
            vec![
                proposal_model_response("{\"summary\":\"frozen evidence\",\"priorityContracts\":[],\"risks\":[]}"),
                proposal_model_response(&root_v2_slot_insufficient_review()),
                proposal_model_response(&gap_receipt_response()),
            ],
        );
        let _real = RealSpecialistTransport::enter();
        let mut session = multi_agent_prepare(&mut h.context).unwrap();
        multi_agent_finish_execution(
            &h.context,
            &mut session,
            &AgentTargetOutcome::incomplete("no target execution"),
        )
        .unwrap();
        let (reviewer, request, candidate) = root_v2_slot_review_open(&h, &session, "v2-gap", 2);
        let db = db::open(&h.db_path).unwrap();
        let (text, usage) = multi_agent_child_round_transport(
            &h.context,
            &session.lease,
            &reviewer,
            "Independent Reviewer; frozen evidence only and no tools.",
            candidate,
        )
        .unwrap();
        let decision = validated_review_decision(&text).unwrap();
        settle_child_usage(&db, &session.lease, &reviewer, &usage).unwrap();
        complete_review_delivery(
            &db,
            &session.lease,
            &reviewer,
            &request,
            "v2-gap",
            2,
            &decision,
            &h.context.target_dir,
        )
        .unwrap();
        let missing = json!(["missing control"]);
        let (candidate, gap, refs) = sealed_gap_review_candidate(
            &db,
            &session.lease.root_run_id,
            "v2-gap",
            2,
            &missing,
            &h.context.target_dir,
        )
        .unwrap();
        let input = json!({"candidateId":"v2-gap","evidenceRevision":2,
            "reviewerMissingEvidence":gap,"trustedFactRefs":refs,"frozenCandidate":candidate});
        let child = scheduler::schedule_child(
            &db,
            &session.lease,
            AgentRole::DeepInvestigator,
            AgentLane::ReadOnlyAnalysis,
            "reviewer_insufficient_evidence",
            &json!({"candidateId":"v2-gap","revision":2,"missingEvidence":missing}),
            2,
            &["evidence.read".into(), "mailbox.write".into()],
            4000,
            1,
        )
        .unwrap();
        scheduler::mark_child_running(&db, &session.lease, &child).unwrap();
        multi_agent_child_round_transport(
            &h.context,
            &session.lease,
            &child,
            "Independent Investigator; assess the sealed Reviewer gap without tools.",
            input.clone(),
        )
        .unwrap();
        assert_eq!(h.model_seen.lock().unwrap().len(), 5);
        stop_failed_child_preserving_usage(
            &db,
            &session.lease,
            &child,
            "saved response delivery interrupted",
        )
        .unwrap();
        let original = attempts::current(&db, &session.lease, &child.assignment_id).unwrap();
        root_v2_slot_no_reactivation(&db);
        let recover = || {
            complete_gap_delivery(
                &db, &h.context, &session, &child, "v2-gap", 2, &missing, &input, "", true,
            )
        };
        if ignore_delete {
            db.execute_batch("CREATE TRIGGER v2_ignore_saved_lane BEFORE DELETE ON agent_lane_leases BEGIN SELECT RAISE(IGNORE); END;").unwrap();
            let before = super::tests::application_table_snapshot(&db);
            assert!(recover().is_err());
            assert!(super::tests::application_table_snapshot(&db) == before);
            assert_eq!(h.model_seen.lock().unwrap().len(), 5);
            db.execute_batch("DROP TRIGGER v2_ignore_saved_lane")
                .unwrap();
        }
        recover().unwrap();
        let after = attempts::current(&db, &session.lease, &child.assignment_id).unwrap();
        assert_eq!(after.id, original.id);
        assert_eq!(after.child_run_id, original.child_run_id);
        assert_eq!(after.worker_id, original.worker_id);
        assert_eq!(after.state, "completed");
        assert_eq!(
            budget::balance(&db, &session.lease.root_run_id, None, "concurrency_batches").unwrap(),
            budget::Balance::default()
        );
        assert_eq!(
            budget::balance(
                &db,
                &session.lease.root_run_id,
                Some(&child.assignment_id),
                "model_requests"
            )
            .unwrap()
            .consumed,
            1
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM agent_lane_leases WHERE assignment_id=?1",
                [&child.assignment_id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        let before = super::tests::application_table_snapshot(&db);
        recover().unwrap();
        assert!(super::tests::application_table_snapshot(&db) == before);
        assert_eq!(h.model_seen.lock().unwrap().len(), 5);
        assert!(h.site_seen.lock().unwrap().is_empty());
        drop(session);
        drop(db);
        fs::remove_dir_all(h.root).unwrap();
    }
}
