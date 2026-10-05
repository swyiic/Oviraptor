#[test]
fn source_fresh_finance_production_first_grant_has_original_control_ten_limits_and_clock() {
    let (root, db, record) = source_coordinator_fixture();
    db.execute_batch("CREATE TRIGGER source_finance_first_grant BEFORE INSERT ON agent_assignment_attempts BEGIN
        SELECT CASE WHEN
          (SELECT COUNT(*) FROM agent_root_budget_attempts WHERE root_run_id=NEW.root_run_id)=1
          AND (SELECT COUNT(*) FROM agent_budget_limits WHERE root_run_id=NEW.root_run_id)=10
          AND EXISTS(SELECT 1 FROM agent_budget_clock_origins o JOIN agent_runs r ON r.id=o.root_run_id
            WHERE r.id=NEW.root_run_id AND r.created_at=o.started_at)
          AND EXISTS(SELECT 1 FROM agent_coordinator_leases c JOIN agent_root_budget_attempts b ON b.root_run_id=c.root_run_id
            WHERE c.root_run_id=NEW.root_run_id AND c.lease_epoch=1
              AND json_extract(b.contract_json,'$.coordinator.epoch')=c.lease_epoch
              AND json_extract(b.contract_json,'$.coordinator.fence')=c.fencing_token)
          AND NOT EXISTS(SELECT 1 FROM agent_budget_limits WHERE root_run_id=NEW.root_run_id
            AND dimension IN ('target_requests','browser_actions','controlled_writes','upload_bytes') AND hard_limit<>0)
        THEN RAISE(ABORT,'source_finance_observer_confirmed')
        ELSE RAISE(ABORT,'source_finance_missing_before_first_grant') END; END;").unwrap();
    let error = run_native_source_assessments(
        &root.join("oviraptor.sqlite3"),
        &record.scan_id,
        1,
        &root.join("attempt-0001"),
    )
    .unwrap_err();
    assert!(
        error.contains("source_finance_observer_confirmed"),
        "{error}"
    );
    assert!(
        !error.contains("source_finance_missing_before_first_grant"),
        "{error}"
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM agent_specialist_calls", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM agent_source_model_rounds", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM agent_root_model_journal", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_fresh_finance_actual_creator_atomic_faults_preserve_all_application_rows() {
    for fault in [
        "BEFORE INSERT ON agent_coordinator_leases BEGIN SELECT RAISE(IGNORE); END;",
        "BEFORE INSERT ON agent_root_budget_attempts BEGIN SELECT RAISE(IGNORE); END;",
        "BEFORE INSERT ON agent_budget_limits BEGIN SELECT RAISE(IGNORE); END;",
        "BEFORE INSERT ON agent_budget_clock_origins BEGIN SELECT RAISE(IGNORE); END;",
        "AFTER INSERT ON agent_runs WHEN NEW.role='coordinator' BEGIN UPDATE projects SET status='archived'; END;",
        "AFTER INSERT ON agent_coordinator_leases BEGIN UPDATE agent_runs SET created_at=datetime('now','-1 minute','localtime') WHERE id=NEW.root_run_id; END;",
        "AFTER INSERT ON agent_root_budget_attempts BEGIN UPDATE projects SET status='archived'; END;",
        "AFTER INSERT ON agent_budget_clock_origins BEGIN UPDATE projects SET status='archived'; END;",
    ] {
        let (root,db,record)=source_coordinator_fixture();
        db.execute_batch(&format!("CREATE TRIGGER source_finance_fault {fault}")).unwrap();
        let before=deletion_snapshot(&db);
        assert!(prepare_native_source_coordinator(&db,&record.scan_id,1,&root.join("attempt-0001")).is_err(),"{fault}");
        assert_eq!(deletion_snapshot(&db),before,"{fault}");
        drop(db);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_fresh_finance_old_missing_owner_is_readable_but_never_dispatches_or_backfills() {
    for schema in [1, 2, 3, 4] {
        let (root, db, record) = source_coordinator_fixture();
        let registered = prepare_native_source_coordinator_schema(
            &db,
            &record.scan_id,
            1,
            &root.join("attempt-0001"),
            schema,
        )
        .unwrap();
        let before = deletion_snapshot(&db);
        let error = run_native_source_assessments(
            &root.join("oviraptor.sqlite3"),
            &record.scan_id,
            1,
            &root.join("attempt-0001"),
        )
        .unwrap_err();
        assert_eq!(error, "source_root_original_finance_missing");
        assert_eq!(
            deletion_snapshot(&db),
            before,
            "schema {schema} cannot get a new C/control/limit/origin or fee"
        );
        assert_eq!(
            prepare_native_source_coordinator(&db, &record.scan_id, 1, &root.join("attempt-0001"))
                .unwrap(),
            registered
        );
        assert_eq!(deletion_snapshot(&db), before);
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_fresh_finance_original_replaced_coordinator_refuses_without_adopting_or_new_fee() {
    let (root, db, record) = source_coordinator_fixture();
    let registered =
        prepare_native_source_coordinator(&db, &record.scan_id, 1, &root.join("attempt-0001"))
            .unwrap();
    let id: String = db
        .query_row(
            "SELECT id FROM agent_root_budget_attempts WHERE root_run_id=?1",
            [&registered.run_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(Uuid::parse_str(&id).is_ok());
    db.execute("UPDATE agent_coordinator_leases SET fencing_token=?1,lease_epoch=lease_epoch+1 WHERE root_run_id=?2",
        params![Uuid::new_v4().to_string(),registered.run_id]).unwrap();
    let before = deletion_snapshot(&db);
    let error = run_native_source_assessments(
        &root.join("oviraptor.sqlite3"),
        &record.scan_id,
        1,
        &root.join("attempt-0001"),
    )
    .unwrap_err();
    assert_eq!(error, "budget_root_original_owner_conflict");
    assert_eq!(deletion_snapshot(&db), before);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
