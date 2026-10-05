fn single_projection_frozen_finance(tag: &str) -> AgentHarness {
    let h = single_finally_fixture(tag);
    assert!(matches!(
        NativeAgentBackend.execute(&h.context),
        AgentTargetOutcome::Failed(_)
    ));
    h
}
fn single_projection_outcome() -> AgentTargetOutcome {
    // Typed production reporting branch only, not actual model/reviewer success.
    AgentTargetOutcome::Completed(AgentCompletion::without_ledger(
        "projection proof",
        AGENT_STOP_DERIVED,
    ))
}
#[test]
fn single_finally_projection_faults_restore_all_rows_after_original_finance_commit() {
    for table in [
        "agent_runs",
        "agent_events",
        "agent_snapshots",
        "agent_collaboration_events",
        "agent_single_projection_receipts",
    ] {
        for fault in ["IGNORE", "ABORT", "FAIL", "business", "foreign-root"] {
            let h = single_projection_frozen_finance("single-projection-fault");
            let db = db::open(&h.db_path).unwrap();
            let sibling = crate::agent_runtime::store::AgentRunRow::new(
                Uuid::new_v4().to_string(),
                h.context.scan_id.clone(),
                1,
                "https://single-other.invalid",
                AgentBackendKind::Native,
                crate::agent_runtime::contract::AgentRole::Coordinator,
                "plan",
                "evidence",
            );
            crate::agent_runtime::store::create_run(&db, &sibling).unwrap();
            let effect = match fault {
                "business" => {
                    "INSERT INTO projects(name) VALUES('forbidden-single-projection')".into()
                }
                "foreign-root" => format!(
                    "UPDATE agent_runs SET terminal_reason='forbidden' WHERE id='{}'",
                    sibling.id
                ),
                _ => format!(
                    "SELECT RAISE({fault}{})",
                    if fault == "IGNORE" {
                        ""
                    } else {
                        ",'projection-fault'"
                    }
                ),
            };
            let operation = if table == "agent_runs" {
                "UPDATE OF status"
            } else {
                "INSERT"
            };
            db.execute_batch(&format!("CREATE TRIGGER single_projection_fault BEFORE {operation} ON {table} BEGIN {effect}; END;")).unwrap();
            let before = single_finally_physical(&db);
            let result = record_runtime_terminal_facts_checked(
                &h.db_path,
                &h.context.scan_id,
                &h.context.route,
                &single_projection_outcome(),
            );
            assert!(result.is_err(), "{table}/{fault}: {result:?}");
            assert_eq!(
                single_finally_physical(&db),
                before,
                "complete publication rollback {table}/{fault}"
            );
            assert!(h.model_seen.lock().unwrap().is_empty());
            assert!(h.site_seen.lock().unwrap().is_empty());
            drop(db);
            single_target_cleanup(h);
        }
    }
}
#[test]
fn single_finally_projection_modified_canonical_emitter_cannot_publish() {
    for effect in [
        "SELECT RAISE(IGNORE)",
        "INSERT INTO projects(name) VALUES('forbidden-single-emitter')",
    ] {
        let h = single_projection_frozen_finance("single-projection-emitter");
        let db = db::open(&h.db_path).unwrap();
        db.execute_batch(&format!("DROP TRIGGER agent_collaboration_run_update; CREATE TRIGGER agent_collaboration_run_update AFTER UPDATE OF status ON agent_runs BEGIN {effect}; END;")).unwrap();
        let before = single_finally_physical(&db);
        assert!(record_runtime_terminal_facts_checked(
            &h.db_path,
            &h.context.scan_id,
            &h.context.route,
            &single_projection_outcome()
        )
        .is_err());
        assert_eq!(single_finally_physical(&db), before);
        drop(db);
        single_target_cleanup(h);
    }
}
#[test]
fn single_finally_projection_four_unique_replace_and_first_exit_replay_are_immutable() {
    let h = single_projection_frozen_finance("single-projection-unique");
    let first = record_runtime_terminal_facts_checked(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        &AgentTargetOutcome::failed("first original failure"),
    )
    .unwrap()
    .unwrap();
    let db = db::open(&h.db_path).unwrap();
    let root = &h.context.run.as_ref().unwrap().run_id;
    let (id,control,financial):(String,String,String)=db.query_row("SELECT receipt_id,control_id,financial_receipt_id FROM agent_single_projection_receipts WHERE root_run_id=?1",[root],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    db.execute_batch("PRAGMA recursive_triggers=OFF").unwrap();
    let before = single_finally_physical(&db);
    for collision in 0..4 {
        let fields = [id.clone(), root.clone(), control.clone(), financial.clone()]
            .into_iter()
            .enumerate()
            .map(|(i, v)| {
                if i == collision {
                    v
                } else {
                    Uuid::new_v4().to_string()
                }
            })
            .collect::<Vec<_>>();
        assert!(db.execute("INSERT OR REPLACE INTO agent_single_projection_receipts(receipt_id,root_run_id,control_id,financial_receipt_id,proof_json,created_at) SELECT ?2,?3,?4,?5,proof_json,created_at FROM agent_single_projection_receipts WHERE root_run_id=?1",params![root,fields[0],fields[1],fields[2],fields[3]]).is_err(),"unique {collision}");
        assert_eq!(single_finally_physical(&db), before);
    }
    for sql in [
        "UPDATE agent_single_projection_receipts SET rowid=rowid+100",
        "DELETE FROM agent_single_projection_receipts",
    ] {
        assert!(db.execute_batch(sql).is_err());
        assert_eq!(single_finally_physical(&db), before);
    }
    let replay = record_runtime_terminal_facts_checked(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        &single_projection_outcome(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(replay, first, "the first committed original terminal wins");
    assert_eq!(single_finally_physical(&db), before);
    let (finished,fact):(String,String)=db.query_row("SELECT r.finished_at,f.fact_json FROM agent_runs r JOIN agent_single_exit_receipts f ON f.root_run_id=r.id WHERE r.id=?1",[root],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(
        JsonValue::from(finished),
        serde_json::from_str::<JsonValue>(&fact).unwrap()["cutoff"]
    );
    drop(db);
    single_target_cleanup(h);
}
#[test]
fn single_finally_projection_corrupt_saved_event_or_snapshot_denies_replay_without_repairs() {
    for table in ["agent_events", "agent_snapshots", "agent_runs"] {
        let h = single_projection_frozen_finance("single-projection-corrupt");
        record_runtime_terminal_facts_checked(
            &h.db_path,
            &h.context.scan_id,
            &h.context.route,
            &single_projection_outcome(),
        )
        .unwrap();
        let db = db::open(&h.db_path).unwrap();
        let root = &h.context.run.as_ref().unwrap().run_id;
        let sql=match table {
            "agent_events"=>"UPDATE agent_events SET payload_json='{}' WHERE run_id=?1 AND event_type='terminal_reduced'",
            "agent_snapshots"=>"UPDATE agent_snapshots SET snapshot_json='{}' WHERE run_id=?1",
            _=>"UPDATE agent_runs SET terminal_reason='altered original result' WHERE id=?1",
        };
        assert_eq!(db.execute(sql, [root]).unwrap(), 1);
        let before = single_finally_physical(&db);
        assert!(
            record_runtime_terminal_facts_checked(
                &h.db_path,
                &h.context.scan_id,
                &h.context.route,
                &single_projection_outcome()
            )
            .is_err(),
            "{table}"
        );
        assert_eq!(
            single_finally_physical(&db),
            before,
            "no silent reconstruction: {table}"
        );
        drop(db);
        single_target_cleanup(h);
    }
}
