use super::*;
pub(crate) fn read_contract(path: &Path, root: &str, case: &str) {
    let (mut writer, owner) = start(path, root);
    let main = crate::db::open(path).unwrap();
    match case {
        "wal" => {
            assert_eq!(
                main.query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0))
                    .unwrap(),
                "wal"
            );
            let mut reader = rusqlite::Connection::open_with_flags(
                path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .unwrap();
            let tx = reader
                .transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)
                .unwrap();
            let anchored: i64 = tx
                .query_row(
                    "SELECT attempt_count FROM sentinel_scans WHERE id=?1",
                    [&owner.scan_id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(anchored, 1);
            let writer_path = path.to_path_buf();
            let bound = owner.clone();
            std::thread::scope(|scope| {
                scope.spawn(move || {
    let mut writer=persist::connection(&writer_path).unwrap();let main=crate::db::open(&writer_path).unwrap();
    persist::append(&mut writer,&bound,"sent","","").unwrap();
    main.execute("INSERT INTO sentinel_scan_attempts(scan_id,attempt_number,status) VALUES(?1,2,'scanning')",[&bound.scan_id]).unwrap();
    main.execute("UPDATE sentinel_scans SET attempt_count=2 WHERE id=?1",[&bound.scan_id]).unwrap();
   }).join().unwrap()
            });
            let old = replay::read_snapshot(&tx, &owner.scan_id, 0, None, None, 0, 300).unwrap();
            let value = serde_json::to_value(old).unwrap();
            assert_eq!(value["attempt"], 1);
            assert_eq!(value["latestSequence"], value["rows"][0]["sequence"]);
            assert_eq!(value["rows"].as_array().unwrap().len(), 1);
            tx.commit().unwrap();
            let fresh = read(path, &owner.scan_id, 0, Some(1), 1);
            assert_eq!(fresh["attempt"], 2);
            assert_eq!(fresh["resetCursor"], true);
            assert_eq!(fresh["afterSequence"], 0);
        }
        "scopes" => {
            assert!(replay::read(path, &owner.scan_id, 2, None, None, 0, 300).is_err());
            assert!(replay::read(path, &owner.scan_id, 1, Some(2), None, 0, 300).is_err());
            assert!(
                replay::read(path, &owner.scan_id, 1, None, Some(&"b".repeat(64)), 0, 300).is_err()
            );
            assert!(replay::read(path, &owner.scan_id, 1, Some(1), None, 999, 300).is_err());
            let missing = path.with_file_name("sdk-no-create.sqlite3");
            assert!(!missing.exists());
            assert!(replay::read(&missing, &owner.scan_id, 1, None, None, 0, 300).is_err());
            assert!(!missing.exists());
            main.execute(
                "INSERT INTO sentinel_deleted_scans(scan_id) VALUES(?1)",
                [&owner.scan_id],
            )
            .unwrap();
            assert!(replay::read(path, &owner.scan_id, 1, None, None, 0, 300).is_err());
        }
        "original_root_contract" => {
            main.execute("UPDATE agent_runs SET plan_json='{}' WHERE id=?1", [root])
                .unwrap();
            let before = snapshot(&main);
            assert!(
                replay::read(path, &owner.scan_id, 1, None, None, 0, 300).is_err(),
                "original dispatch identity cannot validate a changed financial Root contract"
            );
            assert_eq!(
                snapshot(&main),
                before,
                "read must reject without repairing original business data"
            );
        }
        "empty_corrupt_owner" => {
            main.execute_batch("DROP TRIGGER native_sdk_log_row_no_delete; DROP TRIGGER native_sdk_log_owner_no_update; DELETE FROM native_sdk_log_rows;").unwrap();
            main.execute(
                "UPDATE native_sdk_log_owners SET identity_hash=?1 WHERE owner_id=?2",
                params!["b".repeat(64), owner.owner_id],
            )
            .unwrap();
            assert!(
                replay::read(path, &owner.scan_id, 1, None, None, 0, 300).is_err(),
                "available=true cannot bypass identity verification for an empty owner"
            );
        }
        "corrupt_cost_fact" => {
            persist::append(&mut writer, &owner, "sent", "", "").unwrap();
            persist::append(&mut writer, &owner, "response_received", "", "").unwrap();
            // Simulate collateral direct INSERT, not the protected SDK writer. Original
            // accounting stays untouched; a reader must not turn this into paid evidence.
            let before = snapshot(&main)
                .into_iter()
                .filter(|(n, _)| !n.starts_with("native_sdk_log_"))
                .collect::<Vec<_>>();
            main.execute("INSERT INTO native_sdk_log_rows(owner_id,ordinal,stage,cost_phase,terminal_state,created_at) VALUES(?1,4,'cost_saved','received','','external-corrupt')",[&owner.owner_id]).unwrap();
            assert!(
                replay::read(path, &owner.scan_id, 1, None, None, 0, 300).is_err(),
                "saved cost phase without original accounting fact is not trustworthy"
            );
            assert_eq!(
                snapshot(&main)
                    .into_iter()
                    .filter(|(n, _)| !n.starts_with("native_sdk_log_"))
                    .collect::<Vec<_>>(),
                before
            );
        }
        "gap" => {
            main.execute_batch("CREATE TRIGGER sdk_gap_ignore BEFORE INSERT ON native_sdk_log_rows WHEN NEW.stage='sent' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
            // No synthetic stage is inserted. Actual SDK observer failure is covered
            // separately at the real localhost model gate.
            assert!(persist::append(&mut writer, &owner, "sent", "", "").is_err());
            let v = read(path, &owner.scan_id, 1, None, 0);
            assert!(
                v["incompleteOwners"]
                    .as_array()
                    .is_some_and(|o| o.iter().any(|id| id == &json!(owner.owner_id))),
                "unfinished committed prefix must remain explicit, not complete"
            );
        }
        _ => panic!("unknown SDK reader contract"),
    }
}
