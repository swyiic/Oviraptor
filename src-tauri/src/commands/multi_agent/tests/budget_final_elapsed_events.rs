// Actual shared finisher + actual Source TerminalReduced producer, in the same
// private transaction interface used by production Source closure after fix.
// These are financial/publication contracts, not full Source role acceptance.
fn final_elapsed_source_publication(
    tx: &rusqlite::Transaction<'_>,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<(), String> {
    use crate::agent_runtime::{contract::AgentEventKind, store};
    let outcome = AgentTargetOutcome::Cancelled;
    let _ = finish_coordinator_run_in_transaction(tx, lease, &outcome)?;
    let payload = json!({"sourceClosureVersion":1,"financialContractOnly":true});
    let seq = store::append_event(
        tx,
        &lease.root_run_id,
        AgentEventKind::TerminalReduced,
        &payload,
        &[],
    )?;
    let exact: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_events WHERE run_id=?1 AND sequence=?2
        AND event_type='terminal_reduced' AND payload_json=?3)",
            params![lease.root_run_id, seq, payload.to_string()],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !exact {
        return Err("source_completion_terminal_unconfirmed".into());
    }
    Ok(())
}

#[test]
fn final_elapsed_actual_source_event_side_effects_roll_back_every_old_row() {
    use crate::agent_runtime::multi_agent::budget::clock;
    for effect in [
        "business",
        "other-root",
        "other-event",
        "IGNORE",
        "ABORT",
        "FAIL",
    ] {
        let (directory, db, root, lease) = final_elapsed_fixture("final-elapsed-source-event");
        let mut sibling = crate::agent_runtime::store::AgentRunRow::new(
            format!("{root}-sibling"),
            lease.scan_id.clone(),
            1,
            lease.target_key.clone(),
            crate::agent_runtime::contract::AgentBackendKind::Native,
            crate::agent_runtime::contract::AgentRole::Coordinator,
            "other-root-plan",
            "other-root-evidence",
        );
        sibling.root_run_id = sibling.id.clone();
        crate::agent_runtime::store::create_run(&db, &sibling).unwrap();
        let body=match effect {
            "business"=>"INSERT INTO projects(name) VALUES('source-finalization-collateral')",
            "other-root"=>"UPDATE agent_runs SET terminal_reason='source-event-collateral' WHERE id<>NEW.run_id",
            "other-event"=>"INSERT INTO agent_collaboration_events(entity_type,entity_id,event_type,payload_json) VALUES('agent_run','wrong-root','agent_run','{}')",
            "IGNORE"=>"SELECT RAISE(IGNORE)",
            "ABORT"=>"SELECT RAISE(ABORT,'source-financial-event-fault')",
            _=>"SELECT RAISE(FAIL,'source-financial-event-fault')",
        };
        db.execute_batch(&format!(
            "CREATE TRIGGER source_financial_fault BEFORE INSERT ON agent_events
            WHEN NEW.event_type='terminal_reduced' BEGIN {body}; END;"
        ))
        .unwrap();
        let physical = final_elapsed_physical_rows(&db);
        let before = super::tests::application_table_snapshot(&db);
        let result = clock::closure_write(&db, &lease, true, |tx| {
            final_elapsed_source_publication(tx, &lease)
        });
        assert!(result.is_err(), "{effect}: {result:?}");
        assert!(
            super::tests::application_table_snapshot(&db) == before,
            "{effect}"
        );
        assert_eq!(final_elapsed_physical_rows(&db), physical, "{effect}");
        drop(db);
        fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn final_elapsed_actual_builtin_emitter_name_cannot_hide_replacement_or_temp_shadow() {
    for shadow in [false, true] {
        let (directory, db, _root, lease) = final_elapsed_fixture("final-elapsed-emitter-owner");
        if shadow {
            db.execute_batch(
                "CREATE TEMP TRIGGER agent_collaboration_run_update AFTER UPDATE ON main.agent_runs
                BEGIN INSERT INTO projects(name) VALUES('shadowed-emitter-business'); END;",
            )
            .unwrap();
        } else {
            db.execute_batch(
                "DROP TRIGGER agent_collaboration_run_update;
                CREATE TRIGGER agent_collaboration_run_update AFTER UPDATE ON agent_runs
                BEGIN INSERT INTO projects(name) VALUES('replaced-emitter-business'); END;",
            )
            .unwrap();
        }
        let before = super::tests::application_table_snapshot(&db);
        assert!(finish_coordinator_run(&db, &lease, &AgentTargetOutcome::Cancelled).is_err());
        assert!(super::tests::application_table_snapshot(&db) == before);
        drop(db);
        fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn final_elapsed_actual_source_publication_keeps_normal_canonical_events() {
    use crate::agent_runtime::multi_agent::budget::clock;
    let (directory, db, root, lease) = final_elapsed_fixture("final-elapsed-source-normal");
    clock::closure_write(&db, &lease, true, |tx| {
        final_elapsed_source_publication(tx, &lease)
    })
    .unwrap();
    let events:i64=db.query_row("SELECT count(*) FROM agent_collaboration_events WHERE entity_id=?1
        AND entity_type='agent_run' AND event_type='agent_run' AND scan_id=?2 AND attempt_number=?3
        AND json_extract(payload_json,'$.role')='coordinator' AND json_extract(payload_json,'$.status')='terminal'",
        params![root,lease.scan_id,lease.attempt_number],|r|r.get(0)).unwrap();
    assert_eq!(
        events, 1,
        "normal durable terminal notification remains canonical"
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_events WHERE run_id=?1 AND event_type='terminal_reduced'",
            [&root],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    drop(db);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn final_elapsed_actual_root_terminal_event_ignore_cannot_commit_missing_receipt() {
    let (directory, db, root, lease) = final_elapsed_fixture("final-elapsed-root-event-ignore");
    db.execute_batch(&format!(
        "CREATE TRIGGER root_terminal_event_ignore BEFORE INSERT ON agent_collaboration_events
        WHEN NEW.entity_id='{root}' AND json_extract(NEW.payload_json,'$.status')='terminal'
        BEGIN SELECT RAISE(IGNORE); END;"
    ))
    .unwrap();
    let physical = final_elapsed_physical_rows(&db);
    let before = super::tests::application_table_snapshot(&db);
    assert!(finish_coordinator_run(&db, &lease, &AgentTargetOutcome::Cancelled).is_err());
    assert!(super::tests::application_table_snapshot(&db) == before);
    assert_eq!(final_elapsed_physical_rows(&db), physical);
    drop(db);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn final_elapsed_source_writer_rejects_wrong_native_event_scope() {
    use crate::agent_runtime::{contract::AgentEventKind, multi_agent::budget::clock, store};
    let (directory, db, root, lease) = final_elapsed_fixture("final-elapsed-wrong-native-event");
    let mut sibling = store::AgentRunRow::new(
        format!("{root}-sibling"),
        lease.scan_id.clone(),
        1,
        lease.target_key.clone(),
        crate::agent_runtime::contract::AgentBackendKind::Native,
        crate::agent_runtime::contract::AgentRole::Coordinator,
        "other-root-plan",
        "other-root-evidence",
    );
    sibling.root_run_id = sibling.id.clone();
    store::create_run(&db, &sibling).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let physical = final_elapsed_physical_rows(&db);
    let result = clock::closure_write(&db, &lease, true, |tx| {
        let _ = finish_coordinator_run_in_transaction(tx, &lease, &AgentTargetOutcome::Cancelled)?;
        store::append_event(
            tx,
            &sibling.id,
            AgentEventKind::TerminalReduced,
            &json!({"scope":"wrong"}),
            &[],
        )?;
        Ok(())
    });
    assert!(result.is_err(), "{result:?}");
    assert!(super::tests::application_table_snapshot(&db) == before);
    assert_eq!(final_elapsed_physical_rows(&db), physical);
    drop(db);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn final_elapsed_source_writer_preserves_complete_old_native_event_rows_with_recursive_off() {
    use crate::agent_runtime::{contract::AgentEventKind, multi_agent::budget::clock, store};
    let (directory, db, root, lease) = final_elapsed_fixture("final-elapsed-old-native-event");
    db.execute_batch("PRAGMA recursive_triggers=OFF").unwrap();
    store::append_event(
        &db,
        &root,
        AgentEventKind::TerminalReduced,
        &json!({"historical":"preserve"}),
        &[],
    )
    .unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let physical = final_elapsed_physical_rows(&db);
    let result = clock::closure_write(&db, &lease, true, |tx| {
        tx.execute("INSERT OR REPLACE INTO agent_events(id,run_id,sequence,event_type,payload_json,artifact_refs_json,created_at)
            SELECT id,run_id,sequence,event_type,'{}',artifact_refs_json,created_at FROM agent_events WHERE run_id=?1",[&root])
            .map_err(|e|e.to_string())?;
        final_elapsed_source_publication(tx, &lease)
    });
    assert!(result.is_err(), "{result:?}");
    assert!(super::tests::application_table_snapshot(&db) == before);
    assert_eq!(final_elapsed_physical_rows(&db), physical);
    drop(db);
    fs::remove_dir_all(directory).unwrap();
}
