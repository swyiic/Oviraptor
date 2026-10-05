#[test]
fn exceptional_elapsed_actual_financial_fact_faults_roll_back_all_original_state() {
    for effect in [
        "business",
        "other-root",
        "fee-replace",
        "other-fact",
        "IGNORE",
        "ABORT",
        "FAIL",
    ] {
        let (directory, db, root, lease) = final_elapsed_fixture("elapsed-financial-fault");
        // Existing real Root clock invoices, without sending a model request.
        let tx = db.unchecked_transaction().unwrap();
        let _ = crate::agent_runtime::multi_agent::budget::root::model::RootModelCall::claim(
            &tx,
            &root,
            1,
            &"a".repeat(64),
            10,
        )
        .unwrap();
        tx.commit().unwrap();
        let mut sibling = crate::agent_runtime::store::AgentRunRow::new(
            format!("{root}-sibling"),
            lease.scan_id.clone(),
            1,
            lease.target_key.clone(),
            crate::agent_runtime::contract::AgentBackendKind::Native,
            crate::agent_runtime::contract::AgentRole::Coordinator,
            "sibling-plan",
            "sibling-evidence",
        );
        sibling.root_run_id = sibling.id.clone();
        crate::agent_runtime::store::create_run(&db, &sibling).unwrap();
        db.execute("UPDATE agent_coordinator_leases SET lease_expires_at=datetime('now','-1 day','localtime') WHERE root_run_id=?1",[&root]).unwrap();
        let body=match effect {
            "business"=>"INSERT INTO projects(name) VALUES('unapproved-financial-fact-business')",
            "other-root"=>"UPDATE agent_runs SET terminal_reason='unapproved-financial-fact-root' WHERE id<>NEW.root_run_id",
            "fee-replace"=>"INSERT OR REPLACE INTO agent_budget_entries(entry_id,root_run_id,assignment_id,lease_attempt_id,dimension,kind,amount,idempotency_key,source_id,created_at) SELECT entry_id,root_run_id,assignment_id,lease_attempt_id,dimension,kind,amount,idempotency_key,source_id,created_at FROM agent_budget_entries WHERE root_run_id=NEW.root_run_id",
            "other-fact"=>"DELETE FROM agent_root_elapsed_facts",
            "IGNORE"=>"SELECT RAISE(IGNORE)",
            "ABORT"=>"SELECT RAISE(ABORT,'original-financial-fact-fault')",
            _=>"SELECT RAISE(FAIL,'original-financial-fact-fault')",
        };
        db.execute_batch(&format!("CREATE TRIGGER elapsed_original_fact_fault BEFORE INSERT ON agent_root_elapsed_facts BEGIN {body}; END;")).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        let physical = final_elapsed_physical_rows(&db);
        assert!(finish_coordinator_run(&db, &lease, &AgentTargetOutcome::Cancelled).is_err());
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
fn exceptional_elapsed_actual_financial_fact_all_unique_replace_are_immutable_recursive_off() {
    use crate::agent_runtime::multi_agent::budget::clock::elapsed_fact;
    for collision in ["root", "primary"] {
        let (directory, db, root, lease) = final_elapsed_fixture("elapsed-immutable-original-fact");
        db.execute("UPDATE agent_coordinator_leases SET lease_expires_at=datetime('now','-1 day','localtime') WHERE root_run_id=?1",[&root]).unwrap();
        assert!(finish_coordinator_run(&db, &lease, &AgentTargetOutcome::Cancelled).is_err());
        let fact = elapsed_fact::read(&db, &root)
            .unwrap()
            .expect("actual financial fact was recorded");
        db.execute_batch("PRAGMA recursive_triggers=OFF").unwrap();
        let before = super::tests::application_table_snapshot(&db);
        let id = if collision == "root" {
            uuid::Uuid::new_v4().to_string()
        } else {
            fact.id
        };
        let replacement_root = if collision == "root" {
            root.clone()
        } else {
            format!("{root}-different-root")
        };
        assert!(db.execute("INSERT OR REPLACE INTO agent_root_elapsed_facts(fact_id,root_run_id,owner_kind,owner_id,assignment_id,
            scan_id,attempt_number,target_key,coordinator_epoch,coordinator_fence,origin,cutoff,hard_limit_ms,elapsed_ms,journaled_ms,
            unsettled_ms,exhausted,binding_hash,wall_hash,created_at)
            SELECT ?2,?3,owner_kind,owner_id,assignment_id,scan_id,attempt_number,target_key,coordinator_epoch,coordinator_fence,
                origin,cutoff,hard_limit_ms,elapsed_ms,journaled_ms,unsettled_ms,exhausted,binding_hash,wall_hash,created_at
            FROM agent_root_elapsed_facts WHERE root_run_id=?1",params![root,id,replacement_root]).is_err(),"{collision}");
        assert!(super::tests::application_table_snapshot(&db) == before);
        drop(db);
        fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn exceptional_elapsed_actual_late_root_costs_can_settle_without_any_result_or_work_grant() {
    use crate::agent_runtime::{
        model::gateway::ModelResponse,
        multi_agent::budget::{self, clock::elapsed_fact, root::model::RootModelCall},
        store::UsageDelta,
    };
    let (directory, db, root, lease) = final_elapsed_fixture("elapsed-late-original-invoice");
    let call = {
        let tx = db.unchecked_transaction().unwrap();
        let call = RootModelCall::claim(&tx, &root, 1, &"a".repeat(64), 10).unwrap();
        tx.commit().unwrap();
        call
    };
    db.execute("UPDATE agent_coordinator_leases SET lease_expires_at=datetime('now','-1 day','localtime') WHERE root_run_id=?1",[&root]).unwrap();
    assert!(finish_coordinator_run(&db, &lease, &AgentTargetOutcome::Cancelled).is_err());
    let fact = elapsed_fact::read(&db, &root).unwrap().unwrap();
    let wall = budget::balance(&db, &root, None, "wall_time_ms").unwrap();
    // Typed billing fixture tests the actual original terminal/permission APIs,
    // not a real SDK bill or model behavior acceptance.
    let response = ModelResponse {
        usage: UsageDelta {
            input_tokens: 2,
            output_tokens: 1,
            total_tokens: 3,
            model_requests: 1,
            ..UsageDelta::default()
        },
        usage_reported: true,
        ..ModelResponse::default()
    };
    let tx = db.unchecked_transaction().unwrap();
    assert!(!call.terminal(&tx, "received", Some(&response), "").unwrap());
    tx.commit().unwrap();
    assert_eq!(
        budget::balance(&db, &root, None, "model_input_tokens")
            .unwrap()
            .consumed,
        2
    );
    assert_eq!(
        budget::balance(&db, &root, None, "model_output_tokens")
            .unwrap()
            .consumed,
        1
    );
    assert_eq!(
        budget::balance(&db, &root, None, "wall_time_ms").unwrap(),
        wall
    );
    assert_eq!(elapsed_fact::read(&db, &root).unwrap().unwrap(), fact);
    assert!(
        call.require_next_work(&db).is_err(),
        "late original costs do not permit output use or new work"
    );
    assert!(budget::admission::require_determinate(&db, &root).is_err());
    drop(db);
    fs::remove_dir_all(directory).unwrap();
}
