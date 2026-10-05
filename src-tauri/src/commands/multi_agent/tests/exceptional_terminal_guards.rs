#[test]
fn exceptional_terminal_actual_publication_fault_keeps_prior_fact_and_all_original_rows() {
    use crate::agent_runtime::multi_agent::budget::clock::elapsed_fact;
    for effect in [
        "business",
        "other-root",
        "IGNORE",
        "ABORT",
        "FAIL",
        "event-IGNORE",
    ] {
        let (directory, db, root, lease) = exceptional_short_root();
        let fact = elapsed_fact::record_if_exceptional(&db, &lease)
            .unwrap()
            .unwrap();
        let mut sibling = crate::agent_runtime::store::AgentRunRow::new(
            format!("{root}-other"),
            lease.scan_id.clone(),
            1,
            lease.target_key.clone(),
            crate::agent_runtime::contract::AgentBackendKind::Native,
            crate::agent_runtime::contract::AgentRole::Coordinator,
            "plan",
            "evidence",
        );
        sibling.root_run_id = sibling.id.clone();
        crate::agent_runtime::store::create_run(&db, &sibling).unwrap();
        let body = match effect {
            "business" => "INSERT INTO projects(name) VALUES('overhard-publication-business')",
            "other-root" => {
                "UPDATE agent_runs SET terminal_reason='overhard-other-root' WHERE id<>NEW.id"
            }
            "IGNORE" => "SELECT RAISE(IGNORE)",
            "ABORT" => "SELECT RAISE(ABORT,'overhard-publication-fault')",
            _ => "SELECT RAISE(FAIL,'overhard-publication-fault')",
        };
        if effect == "event-IGNORE" {
            db.execute_batch(&format!("CREATE TRIGGER overhard_terminal_event_ignore BEFORE INSERT ON agent_collaboration_events
                WHEN NEW.entity_id='{root}' AND json_extract(NEW.payload_json,'$.status')='terminal'
                BEGIN SELECT RAISE(IGNORE); END;")).unwrap();
        } else {
            db.execute_batch(&format!(
                "CREATE TRIGGER overhard_terminal_fault BEFORE UPDATE OF status ON agent_runs
                WHEN NEW.status='terminal' BEGIN {body}; END;"
            ))
            .unwrap();
        }
        let before = super::tests::application_table_snapshot(&db);
        let physical = final_elapsed_physical_rows(&db);
        assert!(
            finish_coordinator_run(&db, &lease, &AgentTargetOutcome::Cancelled).is_err(),
            "{effect}"
        );
        assert!(
            super::tests::application_table_snapshot(&db) == before,
            "{effect}"
        );
        assert_eq!(final_elapsed_physical_rows(&db), physical, "{effect}");
        assert_eq!(
            elapsed_fact::read(&db, &root).unwrap().unwrap(),
            fact,
            "legally committed financial fact survives publication rollback"
        );
        drop(db);
        fs::remove_dir_all(directory).unwrap();
    }
}

// Actual shared finisher, actual append_event, original guarded Source writer;
// a financial publication contract, not the full Source specialist/gate run.
fn exceptional_terminal_source_publication(
    tx: &rusqlite::Transaction<'_>,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    wrong_scope: bool,
) -> Result<(), String> {
    use crate::agent_runtime::{contract::AgentEventKind, store};
    let outcome = AgentTargetOutcome::Cancelled;
    let receipt = finish_coordinator_run_in_transaction(tx, lease, &outcome)?;
    let event_root = if wrong_scope {
        format!("{}-wrong-scope", lease.root_run_id)
    } else {
        lease.root_run_id.clone()
    };
    store::append_event(
        tx,
        &event_root,
        AgentEventKind::TerminalReduced,
        &json!({"sourceClosureVersion":1,"financialContractOnly":true}),
        &[],
    )?;
    receipt.verify_closed(
        tx,
        outcome.terminal_status(),
        outcome.terminal_code(),
        &crate::agent_runtime::secrets::redact_text_with(&outcome.detail(), None),
    )
}

#[test]
fn exceptional_terminal_actual_source_publication_closes_same_fact_cutoff_and_pins_costs() {
    use crate::agent_runtime::multi_agent::budget::clock;
    let (directory, db, root, lease) = exceptional_short_root();
    let fact = clock::elapsed_fact::record_if_exceptional(&db, &lease)
        .unwrap()
        .unwrap();
    let financial = exceptional_terminal_financial_rows(&db);
    clock::closure_write(&db, &lease, true, |tx| {
        exceptional_terminal_source_publication(tx, &lease, false)
    })
    .unwrap();
    assert_eq!(
        db.query_row(
            "SELECT finished_at FROM agent_runs WHERE id=?1",
            [&root],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        fact.cutoff
    );
    assert_eq!(exceptional_terminal_financial_rows(&db), financial);
    assert_eq!(
        clock::elapsed_fact::read(&db, &root).unwrap().unwrap(),
        fact
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
fn exceptional_terminal_actual_source_scope_and_trigger_faults_keep_prior_fact_and_original_rows() {
    use crate::agent_runtime::multi_agent::budget::clock;
    for effect in [
        "wrong-scope",
        "business",
        "other-root",
        "IGNORE",
        "ABORT",
        "FAIL",
    ] {
        let (directory, db, root, lease) = exceptional_short_root();
        let fact = clock::elapsed_fact::record_if_exceptional(&db, &lease)
            .unwrap()
            .unwrap();
        let mut sibling = crate::agent_runtime::store::AgentRunRow::new(
            format!("{root}-wrong-scope"),
            lease.scan_id.clone(),
            1,
            lease.target_key.clone(),
            crate::agent_runtime::contract::AgentBackendKind::Native,
            crate::agent_runtime::contract::AgentRole::Coordinator,
            "plan",
            "evidence",
        );
        sibling.root_run_id = sibling.id.clone();
        crate::agent_runtime::store::create_run(&db, &sibling).unwrap();
        let body=match effect {
            "business"=>"INSERT INTO projects(name) VALUES('overhard-source-business')",
            "other-root"=>"UPDATE agent_runs SET terminal_reason='overhard-source-other-root' WHERE id<>NEW.run_id",
            "IGNORE"=>"SELECT RAISE(IGNORE)","ABORT"=>"SELECT RAISE(ABORT,'overhard-source-fault')",
            _=>"SELECT RAISE(FAIL,'overhard-source-fault')",
        };
        if effect != "wrong-scope" {
            db.execute_batch(&format!(
                "CREATE TRIGGER overhard_source_event_fault BEFORE INSERT ON agent_events
                WHEN NEW.event_type='terminal_reduced' BEGIN {body}; END;"
            ))
            .unwrap();
        }
        let before = super::tests::application_table_snapshot(&db);
        let physical = final_elapsed_physical_rows(&db);
        let result = clock::closure_write(&db, &lease, true, |tx| {
            exceptional_terminal_source_publication(tx, &lease, effect == "wrong-scope")
        });
        assert!(result.is_err(), "{effect}: {result:?}");
        assert!(
            super::tests::application_table_snapshot(&db) == before,
            "{effect}"
        );
        assert_eq!(final_elapsed_physical_rows(&db), physical, "{effect}");
        assert_eq!(
            clock::elapsed_fact::read(&db, &root).unwrap().unwrap(),
            fact
        );
        drop(db);
        fs::remove_dir_all(directory).unwrap();
    }
}
