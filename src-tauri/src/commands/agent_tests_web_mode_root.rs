fn web_mode_test_plan(fixture: &WebModeFixture) -> AgentExecutionPlan {
    test_plan_for("standard", &fixture.target).with_attempt(1)
}
fn web_mode_test_budget(
    fixture: &WebModeFixture,
    plan: &AgentExecutionPlan,
) -> crate::agent_runtime::multi_agent::budget::root_definition::NewRootBudgetDeclaration {
    let mut context = test_context(&fixture.path, &fixture.target, vec![]);
    context.scan_id = fixture.scan.clone();
    context.execution_plan = plan.clone();
    root_v2_slot_declaration(&context, 1)
}
fn web_mode_test_record(fixture: &WebModeFixture, budget: bool, mode: bool) -> Result<(), String> {
    use crate::agent_runtime::{store, web_mode::root::NewRootModeDeclaration};
    let db = db::open(&fixture.path)?;
    let plan = web_mode_test_plan(fixture);
    let proof = private_web_mode_on(&db, &fixture.scan, 1)?;
    let mode = mode
        .then(|| NewRootModeDeclaration::from_verified(&proof, &fixture.target, &plan.hash()))
        .transpose()?;
    let budget = budget.then(|| web_mode_test_budget(fixture, &plan));
    store::record_attempt_plan_with_declarations(
        &db,
        &fixture.scan,
        1,
        &fixture.target,
        plan.backend,
        &plan.hash(),
        &plan.as_json(),
        budget.as_ref(),
        mode.as_ref(),
    )
}

#[test]
fn web_mode_same_fresh_root_mode_only_budget_only_and_both_keep_native_bytes() {
    use crate::agent_runtime::{multi_agent::budget::root_definition, store, web_mode::root};
    for (has_budget, has_mode) in [(false, true), (true, false), (true, true)] {
        for mode in ["single", "multi"] {
            let fixture = web_mode_fixture(mode, "https://mode.example.test/app");
            let db = db::open(&fixture.path).unwrap();
            let original = web_mode_test_plan(&fixture).as_json().to_string();
            web_mode_test_record(&fixture, has_budget, has_mode).unwrap();
            let root: String = db
                .query_row(
                    "SELECT id FROM agent_runs WHERE scan_id=?1 AND role='coordinator'",
                    [&fixture.scan],
                    |r| r.get(0),
                )
                .unwrap();
            let text: String = db
                .query_row(
                    "SELECT plan_json FROM agent_runs WHERE id=?1",
                    [&root],
                    |r| r.get(0),
                )
                .unwrap();
            let projection:String=db.query_row("SELECT raw_json FROM sentinel_checkpoints WHERE scan_id=?1 AND url=?2 AND stage='agent_execution_plan'",params![fixture.scan,fixture.target],|r|r.get(0)).unwrap();
            assert_eq!(text, original);
            assert_eq!(projection, original);
            assert_eq!(
                root_definition::read(&db, &root).unwrap().is_some(),
                has_budget
            );
            assert_eq!(root::read(&db, &root).unwrap().is_some(), has_mode);
            let policy: String = db
                .query_row(
                    "SELECT orchestration_policy FROM agent_runs WHERE id=?1",
                    [&root],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(policy, if has_mode { mode } else { "single" });
            let before = web_mode_test_rows(&db);
            let plan = web_mode_test_plan(&fixture);
            assert!(web_mode_test_record(&fixture, has_budget, has_mode).is_err());
            web_mode_assert_rows(&db, &before);
            // Existing no-declaration compare keeps exact current Native JSON.
            store::record_attempt_plan(
                &db,
                &fixture.scan,
                1,
                &fixture.target,
                plan.backend,
                &plan.hash(),
                &plan.as_json(),
            )
            .unwrap();
        }
    }
}

#[test]
fn web_mode_joint_writer_denies_ignore_collateral_and_replace_atomically() {
    for has_budget in [false, true] {
        for fault in ["BEFORE INSERT ON agent_root_mode_definitions BEGIN SELECT RAISE(IGNORE); END;",
            "AFTER INSERT ON agent_root_mode_definitions BEGIN UPDATE agent_runs SET heartbeat_at='tampered'; END;",
            "AFTER INSERT ON agent_root_mode_definitions BEGIN UPDATE native_web_mode_receipts SET signature=zeroblob(32); END;",
            "AFTER INSERT ON agent_root_mode_definitions BEGIN INSERT OR REPLACE INTO agent_root_mode_definitions SELECT * FROM agent_root_mode_definitions; END;",
            "AFTER INSERT ON sentinel_checkpoints WHEN NEW.stage='agent_execution_plan' BEGIN UPDATE projects SET status='archived'; END;"] {
            let fixture=web_mode_fixture("multi","https://mode.example.test/app");let db=db::open(&fixture.path).unwrap();
            db.execute_batch(&format!("CREATE TRIGGER mode_fault {fault}")).unwrap();let before=web_mode_test_rows(&db);
            assert!(web_mode_test_record(&fixture,has_budget,true).is_err(),"{fault}");web_mode_assert_rows(&db,&before);
        }
    }
}

#[test]
fn web_mode_fresh_root_factory_rejects_json_reconstruction_and_old_zero_usage_root() {
    use crate::agent_runtime::{store, web_mode::root::NewRootModeDeclaration};
    let fixture = web_mode_fixture("multi", "https://mode.example.test/app");
    let db = db::open(&fixture.path).unwrap();
    let plan = web_mode_test_plan(&fixture);
    let proof = private_web_mode_on(&db, &fixture.scan, 1).unwrap();
    let declaration =
        NewRootModeDeclaration::from_verified(&proof, &fixture.target, &plan.hash()).unwrap();
    let decoded: NewRootModeDeclaration =
        serde_json::from_value(serde_json::to_value(declaration).unwrap()).unwrap();
    let before = web_mode_test_rows(&db);
    assert!(store::record_attempt_plan_with_declarations(
        &db,
        &fixture.scan,
        1,
        &fixture.target,
        plan.backend,
        &plan.hash(),
        &plan.as_json(),
        None,
        Some(&decoded)
    )
    .is_err());
    web_mode_assert_rows(&db, &before);
    persist_agent_execution_plan(&fixture.path, &fixture.scan, 1, &fixture.target, &plan).unwrap();
    let before = web_mode_test_rows(&db);
    assert!(persist_frozen_web_execution_plan(
        &fixture.path,
        &fixture.scan,
        1,
        &fixture.target,
        &plan
    )
    .is_err());
    web_mode_assert_rows(&db, &before);
}
