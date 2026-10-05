#[test]
fn root_v2_slot_expired_unsent_replacement_preserves_batch_and_original_worker() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{attempts, budget, scheduler},
    };
    let mut h = root_v2_slot_harness("v2-unsent-replace", 1);
    root_v2_slot_retarget_model(
        &mut h,
        vec![
            proposal_model_response("{\"summary\":\"initial frozen evidence\",\"priorityContracts\":[],\"risks\":[]}"),
            proposal_model_response("{\"summary\":\"replacement frozen evidence\",\"priorityContracts\":[],\"risks\":[]}"),
        ],
    );
    let _real = RealSpecialistTransport::enter();
    let mut session = multi_agent_prepare(&mut h.context).unwrap();
    let db = db::open(&h.db_path).unwrap();
    let original = scheduler::schedule_child(
        &db,
        &session.lease,
        AgentRole::SpaApiMapper,
        AgentLane::ReadOnlyAnalysis,
        "v2-unsent-replace",
        &json!({"fixture":true}),
        1,
        &["evidence.read".into(), "mailbox.write".into()],
        1000,
        1,
    )
    .unwrap();
    scheduler::start_child_or_release(&db, &session.lease, &original).unwrap();
    expire_worker_deadline(&db, &original.run_id);
    stop_failed_child_preserving_usage(&db, &session.lease, &original, "expired").unwrap();
    let old = expired_saved_worker_row(&db, &original.run_id);
    let origin: String = db
        .query_row(
            "SELECT started_at FROM agent_budget_clock_origins WHERE root_run_id=?1",
            [&session.lease.root_run_id],
            |r| r.get(0),
        )
        .unwrap();
    let replacement =
        scheduler::reassign_undispatched_expired(&db, &session.lease, &original).unwrap();
    assert_eq!(replacement.assignment_id, original.assignment_id);
    assert_ne!(replacement.run_id, original.run_id);
    assert_eq!(
        attempts::current(&db, &session.lease, &replacement.assignment_id)
            .unwrap()
            .lease_epoch,
        2
    );
    assert_eq!(expired_saved_worker_row(&db, &original.run_id), old);
    assert!(attempts::require_live_for_run(&db, &original.run_id).is_err());
    assert_eq!(
        budget::balance(&db, &session.lease.root_run_id, None, "concurrency_batches").unwrap(),
        budget::Balance::default(),
        "actual batch 1 belongs to no child execution slot or worker rotation"
    );
    assert_eq!(
        db.query_row(
            "SELECT started_at FROM agent_budget_clock_origins WHERE root_run_id=?1",
            [&session.lease.root_run_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        origin
    );
    let stable = super::tests::application_table_snapshot(&db);
    assert_eq!(
        scheduler::reassign_undispatched_expired(&db, &session.lease, &original).unwrap(),
        replacement
    );
    assert!(super::tests::application_table_snapshot(&db) == stable);
    scheduler::start_child_or_release(&db, &session.lease, &replacement).unwrap();
    let (text, usage) = multi_agent_child_round_transport(
        &h.context,
        &session.lease,
        &replacement,
        "You are an independent SPA/API Mapper. Analyze supplied frozen evidence only; no tools.",
        json!({"frozenEvidence":["/api/orders"],"target":h.context.target_url}),
    )
    .unwrap();
    deliver_readonly_assessment(
        &db,
        &session.lease,
        &replacement,
        &usage,
        &json!({"summary":text}),
        None,
    )
    .unwrap();
    assert_eq!(
        h.model_seen.lock().unwrap().len(),
        4,
        "replacement uses a real SDK request exactly once"
    );
    assert_eq!(expired_saved_worker_row(&db, &original.run_id), old);
    assert_eq!(
        budget::balance(&db, &session.lease.root_run_id, None, "concurrency_batches").unwrap(),
        budget::Balance::default()
    );
    multi_agent_finish_execution(
        &h.context,
        &mut session,
        &AgentTargetOutcome::incomplete("v2 replacement proof"),
    )
    .unwrap();
    drop(session);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}

#[test]
fn root_v2_slot_saved_readonly_receipt_keeps_original_no_send_and_releases_its_lane() {
    use crate::agent_runtime::multi_agent::budget;
    let mut h = root_v2_slot_harness("v2-saved-readonly", 1);
    let root = h.context.run.as_ref().unwrap().run_id.clone();
    let db = db::open(&h.db_path).unwrap();
    db.execute_batch("CREATE TRIGGER v2_break_delivery BEFORE UPDATE OF acknowledged_at ON agent_messages WHEN NEW.kind='evidence_summary' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    let _real = RealSpecialistTransport::enter();
    assert!(multi_agent_prepare(&mut h.context).is_err());
    assert_eq!(h.model_seen.lock().unwrap().len(), 2);
    let saved_worker: String = db
        .query_row(
            "SELECT child_run_id FROM agent_assignments WHERE role='spa_api_mapper'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let call: String=db.query_row("SELECT request_hash FROM agent_specialist_calls WHERE child_run_id=?1 AND state='received'",
        [&saved_worker],|r|r.get(0)).unwrap();
    assert_eq!(
        budget::balance(&db, &root, None, "concurrency_batches").unwrap(),
        budget::Balance::default(),
        "received paused work keeps its lane, without holding the actual race-batch allowance"
    );
    assert_eq!(db.query_row("SELECT count(*) FROM agent_lane_leases WHERE assignment_id=(SELECT assignment_id FROM agent_runs WHERE id=?1)",
        [&saved_worker],|r|r.get::<_,i64>(0)).unwrap(),1);
    db.execute_batch("DROP TRIGGER v2_break_delivery;
        CREATE TRIGGER v2_no_analysis_reactivation BEFORE UPDATE OF state ON agent_assignments
          WHEN OLD.state IN ('paused','completed') AND NEW.state='running' BEGIN SELECT RAISE(ABORT,'reactivation forbidden'); END;
        CREATE TRIGGER v2_no_analysis_regrant BEFORE INSERT ON agent_capability_leases
          WHEN NEW.child_run_id IN (SELECT child_run_id FROM agent_assignments WHERE role='spa_api_mapper')
          BEGIN SELECT RAISE(ABORT,'regrant forbidden'); END;").unwrap();
    let mut session = multi_agent_prepare(&mut h.context).unwrap();
    assert_eq!(
        h.model_seen.lock().unwrap().len(),
        3,
        "paid Mapper is not retried; its newly closed result gets one actual Root decision"
    );
    assert_eq!(db.query_row("SELECT request_hash FROM agent_specialist_calls WHERE child_run_id=?1 AND state='received'",
        [&saved_worker],|r|r.get::<_,String>(0)).unwrap(),call);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_lane_leases WHERE assignment_id=(SELECT assignment_id FROM agent_runs WHERE id=?1)",
        [&saved_worker],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(
        budget::balance(&db, &root, None, "concurrency_batches").unwrap(),
        budget::Balance::default()
    );
    assert_eq!(
        budget::balance(
            &db,
            &root,
            Some(&session.mapper.assignment_id),
            "model_requests"
        )
        .unwrap()
        .consumed,
        1
    );
    multi_agent_finish_execution(
        &h.context,
        &mut session,
        &AgentTargetOutcome::incomplete("v2 saved receipt proof"),
    )
    .unwrap();
    drop(session);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}
