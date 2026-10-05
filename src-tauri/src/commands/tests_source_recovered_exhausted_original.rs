// Signed Source publication, original born Root, real SDK checkpoint/reentry.
#[test]
fn source_recovered_exhaustion_original_paid_rounds_close_root_without_resending() {
    source_recovered_exhaustion_original_fixture(
        "tool_exhausted",
        "first_tool_received",
        5,
        "repo_mapper",
    );
}

#[test]
fn source_recovered_mapper_finish_then_analyst_exhaustion_closes_actual_failed_child_root() {
    source_recovered_exhaustion_original_fixture(
        "analyst_exhausted",
        "first_tool_finish_received",
        7,
        "source_analyst",
    );
}

fn source_recovered_exhaustion_original_fixture(
    mode: &'static str,
    checkpoint: &str,
    requests: i64,
    failed_role: &str,
) {
    use crate::agent_runtime::multi_agent::budget;
    let (root, db, actor, result) = source_reviewer_execution_fixture_using_calls(
        mode,
        None,
        4,
        None,
        requests as usize,
        |root, db, record, actor| {
            let path = root.join("oviraptor.sqlite3");
            let guard =
                claim_workbench_pipeline_branch(&path, &record.scan_id, 1, "source").unwrap();
            source_reviewer_checkpoint_fixture(root, db, record, actor, checkpoint);
            let original = source_exit_snapshot(db);
            let first:(String,String,i64)=db.query_row("SELECT c.state,t.state,c.round_number
                FROM agent_source_model_rounds c JOIN agent_source_tool_receipts t ON t.assignment_id=c.assignment_id AND t.round_number=c.round_number
                WHERE c.root_run_id=?1 ORDER BY c.round_number DESC LIMIT 1",[&actor.root_run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
            assert_eq!(
                first,
                (
                    "received".into(),
                    "planned".into(),
                    if failed_role == "repo_mapper" { 1 } else { 2 }
                )
            );
            let result = run_native_source_assessments(
                &path,
                &record.scan_id,
                1,
                &root.join("attempt-0001"),
            );
            assert_eq!(
                result.as_ref().unwrap_err(),
                "source_tool_phase_round_budget_exhausted_without_finish"
            );
            let state: (String, String) = db
                .query_row(
                    "SELECT status,terminal_state FROM agent_runs WHERE id=?1",
                    [&actor.root_run_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!(
                state,
                ("terminal".into(), "paused".into()),
                "known recovered failed child must not leave its Root live"
            );
            let tx = db.unchecked_transaction().unwrap();
            budget::clock::FinalClock::verify_original_exit(&tx, &actor.root_run_id).unwrap();
            tx.rollback().unwrap();
            let ledger:(i64,i64,i64,i64)=db.query_row("SELECT spent_tokens,spent_requests,reserved_tokens,reserved_requests FROM agent_budget_ledger WHERE root_run_id=?1",
                [&actor.root_run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
            assert_eq!(ledger, (requests * 20, requests, 0, 0));
            assert_eq!(db.query_row("SELECT a.role FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE a.coordinator_run_id=?1 AND a.state='failed' AND r.terminal_code='child_failed'",[&actor.root_run_id],|r|r.get::<_,String>(0)).unwrap(),failed_role);
            for (dimension, value) in [
                ("model_requests", requests),
                ("model_input_tokens", requests * 10),
                ("model_output_tokens", requests * 10),
            ] {
                let balance = budget::balance(db, &actor.root_run_id, None, dimension).unwrap();
                assert_eq!(
                    (balance.consumed, balance.reserved, balance.indeterminate),
                    (value, 0, 0)
                );
            }
            let after = source_exit_snapshot(db);
            for name in ["agent_specialist_calls", "agent_root_budget_attempts"] {
                assert_eq!(
                    original.iter().find(|(n, _)| n == name),
                    after.iter().find(|(n, _)| n == name),
                    "original {name}"
                );
            }
            for name in [
                "agent_budget_entries",
                "agent_model_cost_facts",
                "agent_source_model_rounds",
            ] {
                let old = &original.iter().find(|(n, _)| n == name).unwrap().1;
                let new = &after.iter().find(|(n, _)| n == name).unwrap().1;
                assert!(
                    old.iter().all(|row| new.contains(row)),
                    "original paid row/rowid changed: {name}"
                );
            }
            assert_eq!(db.query_row("SELECT count(*) FROM agent_source_model_rounds WHERE root_run_id=?1 AND state='received'",[&actor.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),requests-2);
            assert_eq!(db.query_row("SELECT count(*) FROM agent_messages WHERE root_run_id=?1 AND kind='source_tool_result'",[&actor.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),if failed_role=="repo_mapper" {0} else {1});
            assert_eq!(
                db.query_row(
                    "SELECT count(*) FROM agent_source_review_decisions WHERE root_run_id=?1",
                    [&actor.root_run_id],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
            let raw = json!({"error":result.as_ref().unwrap_err()});
            assert!(finish_native_source_branch(&path, &record.scan_id, 1, &raw).unwrap());
            drop(guard);
            assert_eq!(
                db.query_row(
                    "SELECT status FROM sentinel_scans WHERE id=?1",
                    [&record.scan_id],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
                "partial"
            );
            let closed = source_exit_snapshot(db);
            assert!(run_native_source_assessments(
                &path,
                &record.scan_id,
                1,
                &root.join("attempt-0001")
            )
            .is_err());
            assert_eq!(
                source_exit_snapshot(db),
                closed,
                "terminal reentry cannot renew, resend or change any row"
            );
            result
        },
    );
    assert!(result.is_err());
    let tx = db.unchecked_transaction().unwrap();
    budget::clock::FinalClock::verify_original_exit(&tx, &actor.root_run_id).unwrap();
    tx.rollback().unwrap();
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
