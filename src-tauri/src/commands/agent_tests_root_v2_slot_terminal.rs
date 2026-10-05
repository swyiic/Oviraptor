fn root_v2_slot_terminal_saved_fixture(
    tag: &str,
) -> (
    AgentHarness,
    String,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) {
    let mut h = root_v2_slot_harness(tag, 1);
    root_v2_slot_retarget_model(
        &mut h,
        vec![
            proposal_model_response("{\"summary\":\"initial frozen evidence\",\"priorityContracts\":[],\"risks\":[]}"),
            proposal_model_response(valid_proposal_text()),
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
    let lease = session.lease.clone();
    h.context.run = Some(AgentRunLedger {
        db_path: h.db_path.clone(),
        run_id: lease.root_run_id.clone(),
    });
    let db = db::open(&h.db_path).unwrap();
    let id = confirm_queue_directive(&db, &lease, "@mapper 请分析已有冻结证据");
    db.execute_batch("CREATE TRIGGER v2_fail_live_result BEFORE INSERT ON agent_messages WHEN NEW.kind='human_assessment_result' BEGIN SELECT RAISE(ABORT,'retain_received_v2'); END;").unwrap();
    let mut inbox = take_human_directives(&h.context).unwrap();
    let failed = apply_human_proposal_actions(&h.context, &mut inbox).unwrap_err();
    assert!(failed.contains("retain_received_v2"), "{failed}");
    assert_eq!(
        h.model_seen.lock().unwrap().len(),
        4,
        "two actual Root decisions, initial Mapper and actual human-assessment SDK"
    );
    assert_eq!(
        db.query_row(
            "SELECT state FROM agent_directive_proposals WHERE directive_id=?1",
            [&id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "received"
    );
    db.execute_batch("DROP TRIGGER v2_fail_live_result")
        .unwrap();
    finish_coordinator_run(&db, &lease, &AgentTargetOutcome::Cancelled).unwrap();
    drop(session);
    db.execute("UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00' WHERE root_run_id=?1",
        [&lease.root_run_id]).unwrap();
    drop(db);
    (h, id, lease)
}

fn root_v2_slot_root_row(db: &rusqlite::Connection, root: &str) -> Vec<rusqlite::types::Value> {
    let mut query = db
        .prepare("SELECT rowid,* FROM agent_runs WHERE id=?1")
        .unwrap();
    let count = query.column_count();
    query
        .query_row([root], |row| {
            (0..count).map(|column| row.get(column)).collect()
        })
        .unwrap()
}

#[test]
fn root_v2_slot_terminal_human_receipt_is_local_once_and_does_not_reopen_or_release_batch() {
    use crate::agent_runtime::multi_agent::{
        budget, directive::reconciliation::reconcile_received,
    };
    let (h, id, lease) = root_v2_slot_terminal_saved_fixture("v2-terminal-receipt");
    let db = db::open(&h.db_path).unwrap();
    let root_before = root_v2_slot_root_row(&db, &lease.root_run_id);
    let definition: String = db
        .query_row(
            "SELECT definition_json FROM agent_root_budget_definitions WHERE root_run_id=?1",
            [&lease.root_run_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        budget::balance(&db, &lease.root_run_id, None, "concurrency_batches").unwrap(),
        budget::Balance::default(),
        "pending advisory work holds only its execution lane"
    );
    let result = reconcile_received(&db, &lease.scan_id, lease.attempt_number, &id).unwrap();
    assert_eq!(result["status"], "completed");
    assert_eq!(result["modelRequests"], 0);
    assert_eq!(result["targetRequests"], 0);
    assert_eq!(h.model_seen.lock().unwrap().len(), 4);
    assert_eq!(
        root_v2_slot_root_row(&db, &lease.root_run_id),
        root_before,
        "all Root values and rowid stay terminal"
    );
    assert_eq!(
        db.query_row(
            "SELECT definition_json FROM agent_root_budget_definitions WHERE root_run_id=?1",
            [&lease.root_run_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        definition
    );
    assert_eq!(
        budget::balance(&db, &lease.root_run_id, None, "concurrency_batches").unwrap(),
        budget::Balance::default()
    );
    let before = super::tests::application_table_snapshot(&db);
    assert_eq!(
        reconcile_received(&db, &lease.scan_id, lease.attempt_number, &id).unwrap(),
        result
    );
    assert!(super::tests::application_table_snapshot(&db) == before);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_lane_leases WHERE scan_id=?1 AND attempt_number=?2",
            params![lease.scan_id, lease.attempt_number],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(h.model_seen.lock().unwrap().len(), 4);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}

#[test]
fn root_v2_slot_terminal_receipt_ignored_lane_delete_rolls_back_all_local_settlement() {
    use crate::agent_runtime::multi_agent::directive::reconciliation::reconcile_received;
    let (h, id, lease) = root_v2_slot_terminal_saved_fixture("v2-terminal-slot-ignore");
    let db = db::open(&h.db_path).unwrap();
    db.execute_batch("CREATE TRIGGER v2_ignore_lane_delete BEFORE DELETE ON agent_lane_leases BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert!(reconcile_received(&db, &lease.scan_id, lease.attempt_number, &id).is_err());
    assert!(
        super::tests::application_table_snapshot(&db) == before,
        "a skipped lane release cannot commit a financial or result receipt"
    );
    assert_eq!(h.model_seen.lock().unwrap().len(), 4);
    db.execute_batch("DROP TRIGGER v2_ignore_lane_delete")
        .unwrap();
    reconcile_received(&db, &lease.scan_id, lease.attempt_number, &id).unwrap();
    assert_eq!(h.model_seen.lock().unwrap().len(), 4);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}
