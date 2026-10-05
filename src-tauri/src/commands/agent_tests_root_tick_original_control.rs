// Original control is a pure financial/live identity gate, never an issuer.
#[test]
fn coordinator_tick_original_control_never_mints_missing_owner_or_adopts_replacement() {
    use crate::agent_runtime::multi_agent::budget::root::model::RootModelCall;
    for scope in [
        "missing_original",
        "same_original",
        "rotated_c",
        "expired_c",
    ] {
        let (directory, path, root, mut actor) = if scope == "missing_original" {
            multi_agent_test_root("tick-control-old-no-mode", 60_000, 20)
        } else {
            multi_agent_new_task_root("tick-control-born", 60_000, 20)
        };
        let db = db::open(&path).unwrap();
        match scope {
            "rotated_c" => {
                db.execute("UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1,fencing_token=?1 WHERE root_run_id=?2",
                params![uuid::Uuid::new_v4().to_string(),root]).unwrap();
                let pair:(i64,String)=db.query_row("SELECT lease_epoch,fencing_token FROM agent_coordinator_leases WHERE root_run_id=?1",[&root],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
                actor.lease_epoch = pair.0;
                actor.fencing_token = pair.1;
            }
            "expired_c" => {
                db.execute("UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00' WHERE root_run_id=?1",[&root]).unwrap();
            }
            _ => {}
        }
        let before = web_mode_test_rows(&db);
        let tx = db.unchecked_transaction().unwrap();
        let result = RootModelCall::initialize_coordinator_control(&tx, &actor);
        if scope == "same_original" {
            assert!(
                result.is_ok(),
                "same true-born original control must be readable: {result:?}"
            );
        } else {
            assert!(
                result.is_err(),
                "{scope}: current labels/C cannot create or take over original financial authority"
            );
        }
        tx.rollback().unwrap();
        web_mode_assert_rows(&db, &before);
        assert_eq!(
            db.query_row("SELECT count(*) FROM agent_root_model_journal", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            0
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM agent_root_tick_receipts", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            0
        );
        drop(db);
        fs::remove_dir_all(directory).unwrap();
    }
}
