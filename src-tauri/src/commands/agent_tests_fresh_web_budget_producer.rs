#[test]
fn fresh_web_production_creator_freezes_v2_with_mode_and_exact_native_plan() {
    use crate::agent_runtime::{multi_agent::budget::root_definition, web_mode::root};
    for mode in ["single", "multi"] {
        let f = web_mode_fixture(mode, "https://fresh-budget.example.test/app");
        let db = db::open(&f.path).unwrap();
        let plan = web_mode_test_plan(&f);
        let original = plan.as_json().to_string();
        persist_frozen_web_execution_plan(&f.path, &f.scan, 1, &f.target, &plan).unwrap();
        let (id,text,projection):(String,String,String)=db.query_row("SELECT r.id,r.plan_json,p.raw_json
            FROM agent_runs r JOIN sentinel_checkpoints p ON p.scan_id=r.scan_id AND p.url=r.target_url
            AND p.stage='agent_execution_plan' WHERE r.scan_id=?1 AND r.role='coordinator'",
            [&f.scan],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        let declaration = root_definition::read(&db, &id)
            .unwrap()
            .expect("actual fresh creator must declare v2");
        assert_eq!((text, projection), (original.clone(), original));
        assert_eq!(declaration.execution_slot_capacity, 3);
        assert_eq!(
            declaration.limits[8],
            Some(0),
            "actual race budget is not ordinary child slots"
        );
        assert_eq!(declaration.limits[5..8], [Some(0); 3]);
        assert_eq!(declaration.limits[0], Some(plan.hard_total_tokens));
        assert_eq!(declaration.limits[3], Some(plan.hard_model_requests));
        assert_eq!(root::read(&db, &id).unwrap().unwrap().mode().as_str(), mode);
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM agent_coordinator_leases", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            i64::from(mode == "multi"),
            "born Multi owns C1; Single owns no C"
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM agent_budget_entries", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn fresh_web_production_budget_sidecar_faults_roll_back_whole_creation() {
    for fault in [
        "BEFORE INSERT ON agent_root_budget_definitions BEGIN SELECT RAISE(IGNORE); END;",
        "AFTER INSERT ON agent_root_budget_definitions BEGIN UPDATE projects SET status='archived'; END;",
        "AFTER INSERT ON agent_root_budget_definitions BEGIN UPDATE agent_runs SET heartbeat_at='changed'; END;",
        "AFTER INSERT ON agent_root_budget_definitions BEGIN INSERT OR REPLACE INTO agent_root_budget_definitions SELECT * FROM agent_root_budget_definitions; END;",
    ] {
        let f=web_mode_fixture("multi","https://fresh-budget.example.test/app");
        let db=db::open(&f.path).unwrap();
        db.execute_batch(&format!("CREATE TRIGGER producer_fault {fault}")).unwrap();
        let before=web_mode_test_rows(&db);
        assert!(persist_frozen_web_execution_plan(&f.path,&f.scan,1,&f.target,&web_mode_test_plan(&f)).is_err(),"{fault}");
        web_mode_assert_rows(&db,&before);
    }
}

#[test]
fn fresh_web_production_reentry_does_not_backfill_old_mode_only_root() {
    use crate::agent_runtime::multi_agent::{attempts::audit_rows::Rows, budget::root_definition};
    let f = web_mode_fixture("multi", "https://fresh-budget.example.test/app");
    let db = db::open(&f.path).unwrap();
    // Actual old Mode-only creation API under its private startup proof.
    let plan = web_mode_test_plan(&f);
    let proof = private_web_mode_on(&db, &f.scan, 1).unwrap();
    let mode = crate::agent_runtime::web_mode::root::NewRootModeDeclaration::from_verified(
        &proof,
        &f.target,
        &plan.hash(),
    )
    .unwrap();
    crate::agent_runtime::store::record_historical_mode_plan_for_test(
        &db,
        &f.scan,
        1,
        &f.target,
        plan.backend,
        &plan.hash(),
        &plan.as_json(),
        &mode,
    )
    .unwrap();
    let root: String = db
        .query_row(
            "SELECT id FROM agent_runs WHERE scan_id=?1",
            [&f.scan],
            |r| r.get(0),
        )
        .unwrap();
    let before = Rows::read(&db, "SELECT rowid,* FROM agent_runs ORDER BY rowid", []).unwrap();
    let definitions = Rows::read(
        &db,
        "SELECT rowid,* FROM agent_root_budget_definitions ORDER BY rowid",
        [],
    )
    .unwrap();
    persist_frozen_web_execution_plan(&f.path, &f.scan, 1, &f.target, &web_mode_test_plan(&f))
        .unwrap();
    assert!(
        Rows::read(&db, "SELECT rowid,* FROM agent_runs ORDER BY rowid", []).unwrap() == before
    );
    assert!(
        Rows::read(
            &db,
            "SELECT rowid,* FROM agent_root_budget_definitions ORDER BY rowid",
            []
        )
        .unwrap()
            == definitions
    );
    assert!(root_definition::read(&db, &root).unwrap().is_none());
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM agent_budget_limits", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM agent_root_budget_attempts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
