// Actual fresh publisher and historical Mode-only creator; never repair old rows.
fn web_finance_historical_fixture(mode: &str) -> (WebModeFixture, AgentRunContext, Seen) {
    let f = web_mode_fixture(mode, "https://authorized.example.test");
    let (port, _, seen) = spawn_model(vec![proposal_model_response(
        "{\"summary\":\"historical frozen evidence\"}",
    )]);
    let mut c = test_context(&f.path, &f.target, vec![AgentIdentity::anonymous()]);
    c.scan_id = f.scan.clone();
    c.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    c.target_dir = f.work.join("url-pipeline/target-00001");
    c.log_path = f.work.join("runner.log");
    fs::create_dir_all(&c.target_dir).unwrap();
    fs::write(c.target_dir.join(".oviraptor-scan-id"), &f.scan).unwrap();
    fs::write(
        c.target_dir.join("frontend-evidence.json"),
        c.evidence.to_string(),
    )
    .unwrap();
    let db = db::open(&f.path).unwrap();
    let proof = private_web_mode_on(&db, &f.scan, 1).unwrap();
    let mode = crate::agent_runtime::web_mode::root::NewRootModeDeclaration::from_verified(
        &proof,
        &f.target,
        &c.execution_plan.hash(),
    )
    .unwrap();
    // Exact pre-finance creator semantics, available only in cfg(test).
    crate::agent_runtime::store::record_historical_mode_plan_for_test(
        &db,
        &f.scan,
        1,
        &f.target,
        c.execution_plan.backend,
        &c.execution_plan.hash(),
        &c.execution_plan.as_json(),
        &mode,
    )
    .unwrap();
    c.run = Some(runtime_open_run(&f.path, &f.scan, &c.route).unwrap());
    bind_agent_evidence_location(&c).unwrap();
    assert_eq!(
        native_frozen_web_root_mode(&c).unwrap().as_str(),
        mode.mode().as_str()
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM agent_root_budget_attempts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    (f, c, seen)
}

#[test]
fn web_fresh_finance_actual_new_root_freezes_control_and_first_c_before_open_or_model_setup() {
    use crate::agent_runtime::multi_agent::{budget, lease};
    for mode in ["single", "multi"] {
        let f = web_mode_fixture(mode, "https://authorized.example.test");
        let plan = web_mode_test_plan(&f);
        let original = plan.as_json().to_string();
        persist_frozen_web_execution_plan(&f.path, &f.scan, 1, &f.target, &plan).unwrap();
        let db = db::open(&f.path).unwrap();
        let root: String = db
            .query_row(
                "SELECT id FROM agent_runs WHERE scan_id=?1 AND role='coordinator'",
                [&f.scan],
                |r| r.get(0),
            )
            .unwrap();
        let ceiling: [i64; 4] = db
            .query_row(
                "SELECT soft_token_budget,hard_token_budget,soft_request_budget,hard_request_budget
            FROM agent_runs WHERE id=?1",
                [&root],
                |r| Ok([r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?]),
            )
            .unwrap();
        assert_eq!(
            ceiling,
            [
                plan.soft_uncached_tokens,
                plan.hard_total_tokens,
                plan.soft_model_requests,
                plan.hard_model_requests
            ],
            "no later apply_run_plan grant"
        );
        assert_eq!(
            db.query_row(
                "SELECT plan_json FROM agent_runs WHERE id=?1",
                [&root],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            original
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM agent_budget_limits WHERE root_run_id=?1",
                [&root],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            10
        );
        assert!(db.query_row("SELECT EXISTS(SELECT 1 FROM agent_budget_clock_origins o JOIN agent_runs r ON r.id=o.root_run_id
            WHERE r.id=?1 AND o.started_at=r.created_at)",[&root],|r|r.get::<_,bool>(0)).unwrap());
        let owner = budget::root::RootOwner::load_original(&db, &root).unwrap();
        owner.require_executable(&db).unwrap();
        let control: String = db
            .query_row(
                "SELECT contract_json FROM agent_root_budget_attempts WHERE root_run_id=?1",
                [&root],
                |r| r.get(0),
            )
            .unwrap();
        if mode == "single" {
            assert!(json(control.clone())["coordinator"].is_null());
            assert_eq!(
                db.query_row(
                    "SELECT count(*) FROM agent_coordinator_leases WHERE root_run_id=?1",
                    [&root],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
        } else {
            let actor=db.query_row("SELECT scan_id,attempt_number,target_key,root_run_id,lease_epoch,fencing_token,lease_expires_at
                FROM agent_coordinator_leases WHERE root_run_id=?1",[&root],|r|Ok(lease::CoordinatorLease{scan_id:r.get(0)?,attempt_number:r.get(1)?,target_key:r.get(2)?,root_run_id:r.get(3)?,lease_epoch:r.get(4)?,fencing_token:r.get(5)?,lease_expires_at:r.get(6)?})).unwrap();
            assert_eq!(actor.lease_epoch, 1);
            assert!(Uuid::parse_str(&actor.fencing_token).is_ok());
            owner.require_original_coordinator(&db, &actor).unwrap();
            lease::validate_coordinator_lease(&db, &actor).unwrap();
        }
        for table in [
            "agent_budget_entries",
            "agent_assignments",
            "agent_root_model_journal",
            "agent_specialist_calls",
        ] {
            assert_eq!(
                db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                0,
                "creation must not execute or invent a charge"
            );
        }
        let original_finance = fresh_multi_model_financial_rows_for_web_test(&db);
        let mut c = test_context(&f.path, &f.target, vec![AgentIdentity::anonymous()]);
        c.scan_id = f.scan.clone();
        c.execution_plan = plan;
        c.run = Some(runtime_open_run(&f.path, &f.scan, &c.route).unwrap());
        assert_eq!(
            fresh_multi_model_financial_rows_for_web_test(&db),
            original_finance,
            "open compares the same born financial owner rather than issuing one"
        );
    }
}

fn fresh_multi_model_financial_rows_for_web_test(
    db: &rusqlite::Connection,
) -> Vec<(String, Vec<Vec<rusqlite::types::Value>>)> {
    web_mode_test_rows(db)
        .into_iter()
        .filter(|(table, _)| {
            matches!(
                table.as_str(),
                "agent_root_budget_attempts"
                    | "agent_budget_limits"
                    | "agent_budget_clock_origins"
                    | "agent_budget_entries"
            )
        })
        .collect()
}

#[test]
fn web_fresh_finance_historical_single_missing_owner_never_uses_zero_history_as_creation() {
    let (f, c, seen) = web_finance_historical_fixture("single");
    let db = db::open(&f.path).unwrap();
    let before = web_mode_test_rows(&db);
    assert!(
        native_prepare_single_budget(&c).is_err(),
        "pre-finance Mode Root is historical even with zero usage"
    );
    web_mode_assert_rows(&db, &before);
    assert_eq!(seen.lock().unwrap().len(), 0);
}

#[test]
fn web_fresh_finance_historical_multi_c1_or_reacquired_c2_cannot_mint_control_or_claim() {
    use crate::agent_runtime::multi_agent::{
        budget::root::{model::RootModelCall, RootOwner},
        lease,
    };
    for epoch in [1, 2] {
        for api in ["control", "model_claim"] {
            let (f, c, seen) = web_finance_historical_fixture("multi");
            let db = db::open(&f.path).unwrap();
            let root = &c.run.as_ref().unwrap().run_id;
            let mut actor =
                lease::acquire_coordinator_lease(&db, &f.scan, 1, &f.target, root, 600).unwrap();
            if epoch == 2 {
                db.execute("UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00' WHERE root_run_id=?1",[root]).unwrap();
                actor = lease::acquire_coordinator_lease(&db, &f.scan, 1, &f.target, root, 600)
                    .unwrap();
            }
            assert_eq!(actor.lease_epoch, epoch);
            let before = web_mode_test_rows(&db);
            let result = (|| -> Result<(), String> {
                let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
                if api == "control" {
                    RootOwner::initialize_control(&tx, root)?;
                } else {
                    RootModelCall::claim(&tx, root, 1, &"a".repeat(64), 1000)?;
                }
                tx.commit().map_err(|e| e.to_string())
            })();
            assert!(
                result.is_err(),
                "{api}: historical missing control adopted C{epoch}"
            );
            web_mode_assert_rows(&db, &before);
            assert_eq!(seen.lock().unwrap().len(), 0);
        }
    }
}

#[test]
fn web_fresh_finance_historical_multi_prepare_rejects_missing_original_owner_before_all_writes_or_sdk(
) {
    use crate::agent_runtime::multi_agent::lease;
    let (f, mut c, seen) = web_finance_historical_fixture("multi");
    let db = db::open(&f.path).unwrap();
    let root = &c.run.as_ref().unwrap().run_id;
    lease::acquire_coordinator_lease(&db, &f.scan, 1, &f.target, root, 600).unwrap();
    db.execute("UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00' WHERE root_run_id=?1",[root]).unwrap();
    let actor = lease::acquire_coordinator_lease(&db, &f.scan, 1, &f.target, root, 600).unwrap();
    assert_eq!(actor.lease_epoch, 2);
    let before = web_mode_test_rows(&db);
    let caller_plan = c.execution_plan.as_json();
    let _real = RealSpecialistTransport::enter();
    assert!(
        multi_agent_prepare(&mut c).is_err(),
        "a newly reacquired C does not grant the old Mode Root finance"
    );
    web_mode_assert_rows(&db, &before);
    assert_eq!(c.execution_plan.as_json(), caller_plan);
    assert_eq!(seen.lock().unwrap().len(), 0);
}

#[test]
fn web_fresh_finance_born_writer_financial_ignore_or_collateral_rolls_back_entire_root() {
    for mode in ["single", "multi"] {
        for fault in [
        "BEFORE INSERT ON agent_root_budget_attempts BEGIN SELECT RAISE(IGNORE); END;",
        "BEFORE INSERT ON agent_budget_limits BEGIN SELECT RAISE(IGNORE); END;",
        "BEFORE INSERT ON agent_budget_clock_origins BEGIN SELECT RAISE(IGNORE); END;",
        "AFTER INSERT ON agent_root_budget_attempts BEGIN UPDATE projects SET status='archived'; END;",
        "AFTER INSERT ON sentinel_checkpoints WHEN NEW.stage='agent_execution_plan' BEGIN UPDATE agent_coordinator_leases SET fencing_token='replaced'; END;",
        "AFTER INSERT ON agent_budget_limits BEGIN UPDATE agent_runs SET hard_request_budget=999; END;",
        "AFTER INSERT ON agent_root_budget_attempts BEGIN INSERT OR REPLACE INTO agent_root_budget_attempts SELECT * FROM agent_root_budget_attempts; END;",
    ] {
        let f=web_mode_fixture(mode,"https://authorized.example.test");let db=db::open(&f.path).unwrap();
        db.execute_batch(&format!("CREATE TRIGGER born_finance_fault {fault}")).unwrap();
        let before=web_mode_test_rows(&db);let plan=web_mode_test_plan(&f);
        assert!(persist_frozen_web_execution_plan(&f.path,&f.scan,1,&f.target,&plan).is_err(),"{mode}: {fault}");
        web_mode_assert_rows(&db,&before);
    }
    }
}
