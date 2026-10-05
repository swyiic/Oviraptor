const MULTI_EXIT_RECEIPT_SCHEMA: &str =
    include_str!("../agent_runtime/multi_agent/budget/clock/finalization/exit_receipt/schema.sql");

fn multi_exit_original_paid_fixture(tag: &str) -> (RootTickFixture, Seen) {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            sdk_exact_model_body(&root_tick_valid_text("original before financial exit")),
        )
    }));
    let f = root_tick_fixture(tag, &format!("http://127.0.0.1:{port}/v1"));
    native_coordinator_tick(&f.context, &f.actor).unwrap();
    (f, seen)
}

#[test]
fn multi_exit_receipt_actual_ignore_business_and_cross_root_writes_roll_back_complete_exit() {
    for sql in [
        "CREATE TRIGGER multi_exit_fixture_fault BEFORE INSERT ON agent_multi_exit_receipts BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER multi_exit_fixture_fault AFTER INSERT ON agent_multi_exit_receipts BEGIN UPDATE projects SET name='forbidden exit collateral'; END;",
        "CREATE TRIGGER multi_exit_fixture_fault AFTER INSERT ON agent_multi_exit_receipts BEGIN INSERT INTO agent_multi_exit_receipts(receipt_id,root_run_id,control_id,fact_json) VALUES('foreign-receipt','foreign-root',NEW.control_id,NEW.fact_json); END;",
    ] {
        let (f,seen)=multi_exit_original_paid_fixture("multi-exit-private-fault");
        let db=db::open(&f.context.db_path).unwrap();
        db.execute_batch(sql).unwrap();
        let before=super::tests::application_table_snapshot(&db);
        let outcome=AgentTargetOutcome::incomplete("original closure must commit together");
        assert!(finish_coordinator_run(&db,&f.actor,&outcome).is_err(),"{sql}");
        assert!(super::tests::application_table_snapshot(&db)==before,"{sql}");
        assert_eq!(seen.lock().unwrap().len(),1);
        db.execute_batch("DROP TRIGGER multi_exit_fixture_fault").unwrap();
        finish_coordinator_run(&db,&f.actor,&outcome).unwrap();
        multi_exit_receipt_read(&db,&f.actor.root_run_id).unwrap();
        assert_eq!(seen.lock().unwrap().len(),1);
    }
}

#[test]
fn multi_exit_receipt_actual_missing_original_receipt_is_not_backfilled_on_terminal_replay() {
    let (f, seen) = multi_exit_original_paid_fixture("multi-exit-missing-original");
    let db = db::open(&f.context.db_path).unwrap();
    let outcome = AgentTargetOutcome::incomplete("original closure receipt required");
    finish_coordinator_run(&db, &f.actor, &outcome).unwrap();
    // Isolated temporary corruption. Restore the exact immutable schema so the
    // refusal proves absent original evidence, not just a missing trigger.
    db.execute_batch("DROP TRIGGER multi_exit_no_delete")
        .unwrap();
    db.execute(
        "DELETE FROM agent_multi_exit_receipts WHERE root_run_id=?1",
        [&f.actor.root_run_id],
    )
    .unwrap();
    db.execute_batch(MULTI_EXIT_RECEIPT_SCHEMA).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert_eq!(
        multi_exit_receipt_read(&db, &f.actor.root_run_id).unwrap_err(),
        "budget_multi_exit_original_receipt_missing"
    );
    assert_eq!(
        finish_coordinator_run(&db, &f.actor, &outcome).unwrap_err(),
        "budget_multi_exit_original_receipt_missing"
    );
    assert!(super::tests::application_table_snapshot(&db) == before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn multi_exit_receipt_actual_immutable_update_delete_and_replace_are_refused() {
    let (f, seen) = multi_exit_original_paid_fixture("multi-exit-immutable");
    let db = db::open(&f.context.db_path).unwrap();
    finish_coordinator_run(
        &db,
        &f.actor,
        &AgentTargetOutcome::incomplete("immutable original closure"),
    )
    .unwrap();
    let before = super::tests::application_table_snapshot(&db);
    for sql in [
        "UPDATE agent_multi_exit_receipts SET fact_json='{}' WHERE root_run_id=?1",
        "DELETE FROM agent_multi_exit_receipts WHERE root_run_id=?1",
        "INSERT OR REPLACE INTO agent_multi_exit_receipts SELECT receipt_id,root_run_id,control_id,fact_json,created_at FROM agent_multi_exit_receipts WHERE root_run_id=?1",
        "INSERT OR REPLACE INTO agent_multi_exit_receipts SELECT 'different-immutable-id',root_run_id,control_id,fact_json,created_at FROM agent_multi_exit_receipts WHERE root_run_id=?1",
    ] {
        assert!(db.execute(sql,[&f.actor.root_run_id]).is_err(),"{sql}");
        assert!(super::tests::application_table_snapshot(&db)==before);
    }
    multi_exit_receipt_read(&db, &f.actor.root_run_id).unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn multi_exit_receipt_actual_changed_event_or_original_physical_root_is_refused() {
    for sql in [
        "UPDATE agent_collaboration_events SET created_at=created_at||'-changed' WHERE entity_id=?1 AND entity_type='agent_run' AND json_extract(payload_json,'$.status')='terminal'",
        "UPDATE agent_runs SET rowid=rowid+100000 WHERE id=?1",
    ] {
        let (f,seen)=multi_exit_original_paid_fixture("multi-exit-physical-source");
        let db=db::open(&f.context.db_path).unwrap();
        let outcome=AgentTargetOutcome::incomplete("original physical financial sources");
        finish_coordinator_run(&db,&f.actor,&outcome).unwrap();
        assert_eq!(db.execute(sql,[&f.actor.root_run_id]).unwrap(),1);
        let before=super::tests::application_table_snapshot(&db);
        assert_eq!(multi_exit_receipt_read(&db,&f.actor.root_run_id).unwrap_err(),"budget_multi_exit_original_receipt_conflict");
        assert!(finish_coordinator_run(&db,&f.actor,&outcome).is_err());
        assert!(super::tests::application_table_snapshot(&db)==before);
        assert_eq!(seen.lock().unwrap().len(),1);
    }
}

#[test]
fn multi_exit_receipt_actual_mutated_payload_and_weakened_schema_are_not_accepted() {
    for damage in ["schema", "unknown_field", "foreign_control"] {
        let (f, seen) = multi_exit_original_paid_fixture("multi-exit-original-corruption");
        let db = db::open(&f.context.db_path).unwrap();
        let outcome = AgentTargetOutcome::incomplete("original strict closure receipt");
        finish_coordinator_run(&db, &f.actor, &outcome).unwrap();
        if damage == "schema" {
            db.execute_batch("DROP TRIGGER multi_exit_no_replace")
                .unwrap();
        } else {
            db.execute_batch("DROP TRIGGER multi_exit_no_update")
                .unwrap();
            if damage == "unknown_field" {
                db.execute("UPDATE agent_multi_exit_receipts SET fact_json=json_set(fact_json,'$.retrospective',1) WHERE root_run_id=?1",[&f.actor.root_run_id]).unwrap();
            } else {
                let control: String = db
                    .query_row(
                        "SELECT control_id FROM agent_multi_exit_receipts WHERE root_run_id=?1",
                        [&f.actor.root_run_id],
                        |r| r.get(0),
                    )
                    .unwrap();
                db.execute("UPDATE agent_multi_exit_receipts SET fact_json=replace(fact_json,?2,?3) WHERE root_run_id=?1",params![f.actor.root_run_id,control,Uuid::new_v4().to_string()]).unwrap();
            }
            db.execute_batch(MULTI_EXIT_RECEIPT_SCHEMA).unwrap();
        }
        let before = super::tests::application_table_snapshot(&db);
        assert!(
            multi_exit_receipt_read(&db, &f.actor.root_run_id).is_err(),
            "{damage}"
        );
        assert!(
            finish_coordinator_run(&db, &f.actor, &outcome).is_err(),
            "{damage}"
        );
        assert!(super::tests::application_table_snapshot(&db) == before);
        assert_eq!(seen.lock().unwrap().len(), 1);
    }
}
