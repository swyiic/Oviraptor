#[test]
fn root_budget_invoice_replacement_through_round_unique_key_is_rejected() {
    root_budget_replacement_alias("'different-call'", "'received'");
}

#[test]
fn root_budget_invoice_replacement_through_terminal_unique_index_is_rejected() {
    root_budget_replacement_alias("call_id", "'uncertain'");
}

fn root_budget_replacement_alias(call_id: &str, phase: &str) {
    use crate::agent_runtime::multi_agent::budget::root::model::RootModelCall;
    let harness = root_budget_owned_fixture_for_test("root-invoice-replace-alias");
    let db = db::open(&harness.db_path).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let call = RootModelCall::claim(
        &tx,
        &harness.context.run.as_ref().unwrap().run_id,
        1,
        &"a".repeat(64),
        1000,
    )
    .unwrap();
    tx.commit().unwrap();
    let tx = db.unchecked_transaction().unwrap();
    assert!(!call
        .terminal(&tx, "received", Some(&late_web_model_response()), "")
        .unwrap());
    tx.commit().unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert!(db.execute_batch(&format!("INSERT OR REPLACE INTO agent_root_model_journal(call_id,root_run_id,lease_attempt_id,round,request_hash,phase,receipt_json,created_at)
        SELECT {call_id},root_run_id,lease_attempt_id,round,request_hash,{phase},receipt_json,created_at FROM agent_root_model_journal WHERE phase='received'")).is_err(), "an alternate unique collision must not delete the original immutable invoice");
    assert!(super::tests::application_table_snapshot(&db) == before);
    drop(db);
    fs::remove_dir_all(harness.root).unwrap();
}

#[test]
fn root_budget_known_unlimited_invoice_charges_actual_overage_without_unknown_fees() {
    use crate::agent_runtime::multi_agent::budget::{self, root::model::RootModelCall};
    let harness = root_budget_harness("root-unlimited-known-invoice");
    let db = db::open(&harness.db_path).unwrap();
    let root = &harness.context.run.as_ref().unwrap().run_id;
    db.execute(
        "UPDATE agent_runs SET hard_token_budget=0 WHERE id=?1",
        [root],
    )
    .unwrap();
    root_budget_fixture_owner_for_test(&harness);
    let tx = db.unchecked_transaction().unwrap();
    let call = RootModelCall::claim(&tx, root, 1, &"a".repeat(64), 10).unwrap();
    tx.commit().unwrap();
    let mut response = late_web_model_response();
    response.usage.input_tokens = 30;
    response.usage.output_tokens = 20;
    response.usage.total_tokens = 50;
    let tx = db.unchecked_transaction().unwrap();
    assert!(
        !call.terminal(&tx, "received", Some(&response), "").unwrap(),
        "complete known unlimited invoice must not become an unknown estimate"
    );
    tx.commit().unwrap();
    for (dimension, amount) in [
        ("model_input_tokens", 30),
        ("model_cached_tokens", 0),
        ("model_output_tokens", 20),
        ("model_requests", 1),
    ] {
        let b = budget::balance(&db, root, Some(""), dimension).unwrap();
        assert_eq!(
            (b.reserved, b.consumed, b.indeterminate),
            (0, amount, 0),
            "{dimension}"
        );
    }
    call.require_next_work(&db).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    assert!(call.terminal(&tx, "received", Some(&response), "").is_err());
    tx.rollback().unwrap();
    assert!(super::tests::application_table_snapshot(&db) == before);
    drop(db);
    fs::remove_dir_all(harness.root).unwrap();
}

#[test]
fn root_budget_fixture_birth_and_live_claim_faults_rollback_all_accounting() {
    use crate::agent_runtime::multi_agent::budget::root::model::RootModelCall;
    for table in [
        "agent_root_budget_attempts",
        "agent_budget_clock_origins",
        "agent_budget_limits",
        "agent_budget_entries",
        "agent_root_model_journal",
    ] {
        for behavior in ["IGNORE", "ABORT"] {
            let harness = root_budget_harness("root-claim-write-fault");
            let db = db::open(&harness.db_path).unwrap();
            let birth_fault=matches!(table,"agent_root_budget_attempts"|"agent_budget_clock_origins"|"agent_budget_limits");
            if !birth_fault {root_budget_fixture_owner_for_test(&harness);}
            db.execute_batch(&format!("CREATE TRIGGER root_write_fault BEFORE INSERT ON {table} BEGIN SELECT RAISE({behavior}{}); END;", if behavior == "ABORT" { ", 'injected-root-write-failure'" } else { "" })).unwrap();
            let before = super::tests::application_table_snapshot(&db);
            let tx = db.unchecked_transaction().unwrap();
            let rejected=if birth_fault {
                crate::agent_runtime::multi_agent::budget::root::RootOwner::initialize_financial_fixture_for_test(
                    &tx,&harness.context.run.as_ref().unwrap().run_id).map(|_|())
            } else {
                RootModelCall::claim(&tx,&harness.context.run.as_ref().unwrap().run_id,
                    1,&"a".repeat(64),1000).map(|_|())
            };
            assert!(rejected.is_err(),"{table}/{behavior}");
            tx.rollback().unwrap();
            assert!(
                super::tests::application_table_snapshot(&db) == before,
                "{table}/{behavior}"
            );
            drop(db);
            fs::remove_dir_all(harness.root).unwrap();
        }
    }
}

#[test]
fn root_budget_terminal_faults_preserve_original_claim_and_reject_trigger_collateral() {
    use crate::agent_runtime::multi_agent::budget::root::model::RootModelCall;
    for effect in [
        "SELECT RAISE(IGNORE)",
        "SELECT RAISE(ABORT,'terminal-failed')",
        "INSERT INTO projects(name) VALUES('root-terminal-collateral')",
        "UPDATE agent_runs SET role='foreign-root-owner' WHERE id=NEW.root_run_id",
    ] {
        let harness = root_budget_owned_fixture_for_test("root-terminal-write-fault");
        let db = db::open(&harness.db_path).unwrap();
        let tx = db.unchecked_transaction().unwrap();
        let call = RootModelCall::claim(
            &tx,
            &harness.context.run.as_ref().unwrap().run_id,
            1,
            &"a".repeat(64),
            1000,
        )
        .unwrap();
        tx.commit().unwrap();
        db.execute_batch(&format!("CREATE TRIGGER root_terminal_fault BEFORE INSERT ON agent_root_model_journal WHEN NEW.phase='received' BEGIN {effect}; END;")).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        let tx = db.unchecked_transaction().unwrap();
        assert!(
            call.terminal(&tx, "received", Some(&late_web_model_response()), "")
                .is_err(),
            "{effect}"
        );
        tx.rollback().unwrap();
        assert!(
            super::tests::application_table_snapshot(&db) == before,
            "{effect}"
        );
        drop(db);
        fs::remove_dir_all(harness.root).unwrap();
    }
}

#[test]
fn root_budget_history_and_partial_contract_cannot_become_new_authority() {
    use crate::agent_runtime::multi_agent::budget::root::model::RootModelCall;
    for historical in ["UPDATE agent_runs SET used_tokens=1", "UPDATE agent_runs SET reserved_requests=1", "INSERT INTO agent_budget_limits(root_run_id,dimension,hard_limit) SELECT id,'model_requests',1 FROM agent_runs"] {
        let harness = root_budget_harness("root-historical-authority");
        let db = db::open(&harness.db_path).unwrap();
        db.execute_batch(historical).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        let tx = db.unchecked_transaction().unwrap();
        assert!(RootModelCall::claim(&tx, &harness.context.run.as_ref().unwrap().run_id, 1, &"a".repeat(64), 1000).is_err());
        tx.rollback().unwrap();
        assert!(super::tests::application_table_snapshot(&db) == before);
        drop(db);
        fs::remove_dir_all(harness.root).unwrap();
    }
}

#[test]
fn root_budget_owner_and_model_facts_are_immutable_and_bound_to_original_database() {
    use crate::agent_runtime::multi_agent::budget::root::model::RootModelCall;
    let harness = root_budget_owned_fixture_for_test("root-immutable-original");
    let other = root_budget_harness("root-foreign-database");
    let db = db::open(&harness.db_path).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let call = RootModelCall::claim(
        &tx,
        &harness.context.run.as_ref().unwrap().run_id,
        1,
        &"a".repeat(64),
        1000,
    )
    .unwrap();
    tx.commit().unwrap();
    let before = super::tests::application_table_snapshot(&db);
    for statement in ["UPDATE agent_root_budget_attempts SET id='foreign'", "DELETE FROM agent_root_budget_attempts", "INSERT OR REPLACE INTO agent_root_budget_attempts SELECT * FROM agent_root_budget_attempts", "UPDATE agent_root_model_journal SET request_hash='foreign'", "DELETE FROM agent_root_model_journal", "INSERT OR REPLACE INTO agent_root_model_journal SELECT * FROM agent_root_model_journal"] {
        assert!(db.execute_batch(statement).is_err(), "{statement}");
        assert!(super::tests::application_table_snapshot(&db) == before);
    }
    let foreign = db::open(&other.db_path).unwrap();
    let foreign_before = super::tests::application_table_snapshot(&foreign);
    let tx = foreign.unchecked_transaction().unwrap();
    assert!(call
        .terminal(&tx, "received", Some(&late_web_model_response()), "")
        .is_err());
    tx.rollback().unwrap();
    assert!(super::tests::application_table_snapshot(&foreign) == foreign_before);
    assert!(super::tests::application_table_snapshot(&db) == before);
    drop(foreign);
    drop(db);
    fs::remove_dir_all(other.root).unwrap();
    fs::remove_dir_all(harness.root).unwrap();
}
