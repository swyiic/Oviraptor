use super::*;
pub(crate) fn write_contract(path: &Path, root: &str, call: &RootModelCall, case: &str) {
    let main = crate::db::open(path).unwrap();
    match case {
        "owner_trigger" | "row_trigger" => {
            let table = if case == "owner_trigger" {
                "native_sdk_log_owners"
            } else {
                "native_sdk_log_rows"
            };
            main.execute_batch(&format!("CREATE TRIGGER sdk_business AFTER INSERT ON {table} BEGIN INSERT INTO projects(name) VALUES('sdk-unapproved-effect'); END;")).unwrap();
            let before = snapshot(&main);
            let mut private = persist::connection(path).unwrap();
            let owner = original(&private, root);
            if case == "owner_trigger" {
                assert!(persist::begin(&mut private, &owner).is_err());
            } else {
                // Seed owner before creating the row trigger so both SDK02 and fixed begin
                // execute the actual protected row insertion, not a test replacement.
                main.execute_batch("DROP TRIGGER sdk_business;").unwrap();
                let (mut private, owner) = start(path, root);
                main.execute_batch("CREATE TRIGGER sdk_business AFTER INSERT ON native_sdk_log_rows BEGIN INSERT INTO projects(name) VALUES('sdk-unapproved-effect'); END;").unwrap();
                let before = snapshot(&main);
                assert!(persist::append(&mut private, &owner, "sent", "", "").is_err());
                assert_eq!(snapshot(&main), before);
                return;
            }
            assert_eq!(snapshot(&main), before);
        }
        "cross_lane" => {
            main.execute("INSERT INTO agent_runs(id,scan_id,attempt_number,status,backend,role,plan_json) SELECT 'sdk-other-root',scan_id,attempt_number,'completed','native','coordinator',plan_json FROM agent_runs WHERE id=?1",[root]).unwrap();
            let (mut private, owner) = start(path, root);
            for effect in [
                "UPDATE agent_runs SET plan_json='{}' WHERE id='sdk-other-root'",
                "UPDATE agent_budget_ledger SET spent_tokens=spent_tokens+1",
                "UPDATE sentinel_scans SET task_name='sdk-unapproved-name'",
                "DELETE FROM agent_root_model_journal WHERE phase='dispatch'",
            ] {
                main.execute_batch(&format!("CREATE TRIGGER sdk_collateral AFTER INSERT ON native_sdk_log_rows BEGIN {effect}; END;")).unwrap();
                let before = snapshot(&main);
                assert!(persist::append(&mut private, &owner, "sent", "", "").is_err());
                assert_eq!(snapshot(&main), before);
                main.execute_batch("DROP TRIGGER sdk_collateral;").unwrap();
            }
        }
        "atomic_begin_ignore" => {
            main.execute_batch("CREATE TRIGGER sdk_ignore_prepared BEFORE INSERT ON native_sdk_log_rows WHEN NEW.stage='prepared' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
            let before = snapshot(&main);
            let mut private = persist::connection(path).unwrap();
            let owner = original(&private, root);
            assert!(
                persist::begin(&mut private, &owner).is_err(),
                "owner + prepared must commit atomically or neither exists"
            );
            assert_eq!(snapshot(&main), before);
        }
        "row_ignore" => {
            let (mut private, owner) = start(path, root);
            main.execute_batch("CREATE TRIGGER sdk_ignore BEFORE INSERT ON native_sdk_log_rows WHEN NEW.stage='sent' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
            let before = snapshot(&main);
            assert!(persist::append(&mut private, &owner, "sent", "", "").is_err());
            assert_eq!(snapshot(&main), before);
            assert_eq!(stages(&main, &owner.owner_id), ["prepared"]);
        }
        "replace_unique" => {
            let (_private, owner) = start(path, root);
            main.pragma_update(None, "recursive_triggers", false)
                .unwrap();
            let before = snapshot(&main);
            for sql in [
    "INSERT OR REPLACE INTO native_sdk_log_owners SELECT * FROM native_sdk_log_owners LIMIT 1",
    "INSERT OR REPLACE INTO native_sdk_log_owners(owner_id,domain,dispatch_key,scan_id,attempt_number,root_run_id,run_id,assignment_id,lease_attempt_id,worker_id,round_number,request_hash,identity_hash,created_at) SELECT lower(hex(randomblob(32))),domain,dispatch_key,scan_id,attempt_number,root_run_id,run_id,assignment_id,lease_attempt_id,worker_id,round_number,request_hash,identity_hash,created_at FROM native_sdk_log_owners LIMIT 1",
    "INSERT OR REPLACE INTO native_sdk_log_rows SELECT * FROM native_sdk_log_rows LIMIT 1",
    "INSERT OR REPLACE INTO native_sdk_log_rows(owner_id,ordinal,stage,cost_phase,terminal_state,created_at) SELECT owner_id,ordinal,'sent','','',created_at FROM native_sdk_log_rows LIMIT 1",
    "INSERT OR REPLACE INTO native_sdk_log_rows(owner_id,ordinal,stage,cost_phase,terminal_state,created_at) SELECT owner_id,2,stage,cost_phase,terminal_state,created_at FROM native_sdk_log_rows LIMIT 1",
   ] {assert!(main.execute_batch(sql).is_err(),"all UNIQUE replacement paths must fail");assert_eq!(snapshot(&main),before);}
            assert_eq!(stages(&main, &owner.owner_id), ["prepared"]);
        }
        "stage_order" => {
            let (mut private, owner) = start(path, root);
            let before = snapshot(&main);
            for (stage, cost, terminal) in [
                ("validated", "", ""),
                ("response_received", "", ""),
                ("terminal", "received", "returned"),
                ("terminal", "", "returned"),
                ("cost_saved", "received", ""),
                ("sent", "received", ""),
            ] {
                assert!(
                    persist::append(&mut private, &owner, stage, cost, terminal).is_err(),
                    "stage {stage} cannot imply missing lifecycle evidence"
                );
                assert_eq!(snapshot(&main), before);
            }
            persist::append(&mut private, &owner, "sent", "", "").unwrap();
            assert_eq!(stages(&main, &owner.owner_id), ["prepared", "sent"]);
        }
        "foreign_owner" => {
            let (mut private, owner) = start(path, root);
            let before = snapshot(&main);
            for field in 0..6 {
                let mut wrong = owner.clone();
                match field {
                    0 => wrong.scan_id = "foreign-scan".into(),
                    1 => wrong.attempt += 1,
                    2 => wrong.run_id = "foreign-run".into(),
                    3 => wrong.request_hash = "b".repeat(64),
                    4 => wrong.lease_attempt_id = uuid::Uuid::new_v4().to_string(),
                    _ => wrong.dispatch_key = "b".repeat(64),
                }
                assert!(persist::append(&mut private, &wrong, "sent", "", "").is_err());
                assert_eq!(snapshot(&main), before);
            }
        }
        "gap_closed" => {
            let (mut private, owner) = start(path, root);
            persist::gap(&mut private, &owner, "sent").unwrap();
            let before = snapshot(&main);
            assert!(
                persist::append(&mut private, &owner, "sent", "", "").is_err(),
                "a closed diagnostic gap cannot grow its committed ordinal"
            );
            assert_eq!(snapshot(&main), before);
            assert_eq!(
                read(path, &owner.scan_id, 1, None, 0)["gaps"][0]["afterOrdinal"],
                1
            );
        }
        "gap_trigger" | "gap_ignore" | "gap_replace" => {
            let (mut private, owner) = start(path, root);
            if case == "gap_trigger" {
                main.execute_batch("CREATE TRIGGER sdk_gap_collateral AFTER INSERT ON native_sdk_log_gaps BEGIN INSERT INTO projects(name) VALUES('sdk-gap-unapproved-effect'); END;").unwrap();
                let before = snapshot(&main);
                assert!(persist::gap(&mut private, &owner, "sent").is_err());
                assert_eq!(snapshot(&main), before);
            } else if case == "gap_ignore" {
                main.execute_batch("CREATE TRIGGER sdk_gap_ignore BEFORE INSERT ON native_sdk_log_gaps BEGIN SELECT RAISE(IGNORE); END;").unwrap();
                let before = snapshot(&main);
                assert!(persist::gap(&mut private, &owner, "sent").is_err());
                assert_eq!(snapshot(&main), before);
            } else {
                persist::gap(&mut private, &owner, "sent").unwrap();
                main.pragma_update(None, "recursive_triggers", false)
                    .unwrap();
                let before = snapshot(&main);
                for sql in ["INSERT OR REPLACE INTO native_sdk_log_gaps SELECT * FROM native_sdk_log_gaps LIMIT 1","INSERT OR REPLACE INTO native_sdk_log_gaps(owner_id,after_ordinal,failed_stage,code,created_at) SELECT owner_id,after_ordinal,'response_received',code,created_at FROM native_sdk_log_gaps LIMIT 1"] {
     assert!(main.execute_batch(sql).is_err());assert_eq!(snapshot(&main),before);
    }
                let v = read(path, &owner.scan_id, 1, None, 0);
                assert_eq!(v["gaps"][0]["failedStage"], "sent");
                assert_eq!(v["rows"].as_array().unwrap().len(), 1);
            }
        }
        "late_closed" => {
            let (mut private, owner) = start(path, root);
            persist::append(&mut private, &owner, "sent", "", "").unwrap();
            main.execute(
                "UPDATE sentinel_scans SET status='paused' WHERE id=?1",
                [&owner.scan_id],
            )
            .unwrap();
            let tx = main.unchecked_transaction().unwrap();
            call.terminal(&tx, "uncertain", None, "sdk-contract-cancel")
                .unwrap();
            tx.commit().unwrap();
            let before = snapshot(&main)
                .into_iter()
                .filter(|(n, _)| !n.starts_with("native_sdk_log_"))
                .collect::<Vec<_>>();
            persist::append(&mut private, &owner, "cost_saved", "uncertain", "").unwrap();
            persist::append(&mut private, &owner, "terminal", "uncertain", "uncertain").unwrap();
            assert_eq!(
                snapshot(&main)
                    .into_iter()
                    .filter(|(n, _)| !n.starts_with("native_sdk_log_"))
                    .collect::<Vec<_>>(),
                before
            );
            assert_eq!(
                main.query_row(
                    "SELECT status FROM sentinel_scans WHERE id=?1",
                    [&owner.scan_id],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
                "paused"
            );
            assert_eq!(
                crate::agent_runtime::multi_agent::budget::balance(
                    &main,
                    root,
                    None,
                    "model_requests"
                )
                .unwrap()
                .indeterminate,
                1
            );
            assert_eq!(
                read(path, &owner.scan_id, 1, None, 0)["rows"][3]["terminalState"],
                "uncertain"
            );
            assert!(
                persist::append(&mut private, &owner, "sent", "", "").is_err(),
                "late diagnostics cannot resume a closed SDK owner"
            );
        }
        _ => panic!("unknown SDK contract"),
    }
}
