#[test]
fn executor_allocation_original_receipts_scope_and_foreign_target_history_reject_zero_write() {
    let _real = RealSpecialistTransport::enter();
    for damage in [
        "tokens",
        "requests",
        "invoice_guard",
        "task",
        "task_root_floor",
        "task_root_requests",
        "capability",
        "worker",
        "foreign_run",
        "foreign_assignment",
    ] {
        let (f, mapper, frame, decision, seen) =
            executor_allocation_closed_mapper_fixture((60_000, 6));
        let child = executor_allocation_admit(&f, &mapper, &frame, &decision).unwrap();
        let db = db::open(&f.context.db_path).unwrap();
        match damage {
            "tokens" => {
                db.execute(
                    "UPDATE agent_assignments SET reserved_tokens=8000 WHERE id=?1",
                    [&child.assignment_id],
                )
                .unwrap();
            }
            "requests" => {
                db.execute(
                    "UPDATE agent_assignments SET reserved_requests=2 WHERE id=?1",
                    [&child.assignment_id],
                )
                .unwrap();
            }
            "invoice_guard" => {
                db.execute_batch("DROP TRIGGER budget_entry_no_update")
                    .unwrap();
            }
            "task" => {
                db.execute("UPDATE agent_assignments SET task_slice_json=json_remove(task_slice_json,'$.rootDecision.rustPolicy.allocationRule') WHERE id=?1",[&child.assignment_id]).unwrap();
            }
            "task_root_floor" => {
                db.execute("UPDATE agent_assignments SET task_slice_json=json_remove(task_slice_json,'$.rootDecision.rustPolicy.rootSupervisionFloor') WHERE id=?1",[&child.assignment_id]).unwrap();
            }
            "task_root_requests" => {
                db.execute("UPDATE agent_assignments SET task_slice_json=json_set(task_slice_json,'$.rootDecision.rustPolicy.rootSupervisionFloor.modelRequests',0) WHERE id=?1",[&child.assignment_id]).unwrap();
            }
            "capability" => {
                db.execute("UPDATE agent_capability_leases SET revoked_at=datetime('now','localtime') WHERE assignment_id=?1",[&child.assignment_id]).unwrap();
            }
            "worker" => {
                db.execute("UPDATE agent_assignment_attempts SET expires_at='2000-01-01 00:00:00' WHERE assignment_id=?1",[&child.assignment_id]).unwrap();
            }
            "foreign_run" => {
                use crate::agent_runtime::{
                    contract::{AgentBackendKind, AgentLane, AgentRole, MultiAgentPolicy},
                    store::{self, AgentRunRow},
                };
                let mut r = AgentRunRow::new(
                    "foreign-one-sided-target",
                    f.actor.scan_id.clone(),
                    f.actor.attempt_number,
                    f.actor.target_key.clone(),
                    AgentBackendKind::Native,
                    AgentRole::WebExecutor,
                    "foreign",
                    "",
                );
                r.parent_run_id = Some(f.actor.root_run_id.clone());
                r.root_run_id = f.actor.root_run_id.clone();
                r.lane = Some(AgentLane::TargetTouching);
                r.orchestration_policy = MultiAgentPolicy::Multi;
                store::create_run(&db, &r).unwrap();
            }
            "foreign_assignment" => {
                use crate::agent_runtime::{
                    contract::{AgentLane, AgentRole},
                    multi_agent::assignment::{self, AgentAssignment},
                };
                let a = AgentAssignment::new(
                    "foreign-one-sided-assignment",
                    f.actor.root_run_id.clone(),
                    AgentRole::DeepInvestigator,
                    AgentLane::TargetTouching,
                    f.actor.target_key.clone(),
                    "foreign",
                );
                assignment::insert_assignment(&db, &a).unwrap();
            }
            _ => unreachable!(),
        }
        if damage == "worker" {
            // Let the real original parent complete its separate expiry transaction.
            // Compare the denied caller only after original withdrawal is proven.
            human_directive_wait_original_worker_expiry(
                &db,
                f.context.supervision.as_ref().unwrap(),
                &child.run_id,
            );
        }
        let before = (web_mode_test_rows(&db), single_finally_physical(&db));
        assert!(
            executor_allocation_admit(&f, &mapper, &frame, &decision).is_err(),
            "{damage}"
        );
        web_mode_assert_rows(&db, &before.0);
        assert_eq!(single_finally_physical(&db), before.1);
        assert_eq!(seen.lock().unwrap().len(), 3);
    }
}
#[test]
fn executor_allocation_paid_issuance_faults_keep_every_row_and_original_sdk_fee() {
    let _real = RealSpecialistTransport::enter();
    for sql in [
        "CREATE TRIGGER executor_allocation_fault BEFORE INSERT ON agent_budget_entries WHEN NEW.assignment_id<>'' AND NEW.dimension='model_cached_tokens' BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER executor_allocation_fault AFTER INSERT ON agent_assignments BEGIN UPDATE projects SET name='escaped'; END;",
        "CREATE TRIGGER executor_allocation_fault BEFORE INSERT ON agent_capability_leases BEGIN SELECT RAISE(IGNORE); END;",
    ] {
        let (f,mapper,frame,decision,seen)=executor_allocation_closed_mapper_fixture((60_000,6));
        let db=db::open(&f.context.db_path).unwrap();db.execute_batch(sql).unwrap();
        let before=web_mode_test_rows(&db);
        assert!(executor_allocation_admit(&f,&mapper,&frame,&decision).is_err());
        web_mode_assert_rows(&db,&before);
        assert_eq!(seen.lock().unwrap().len(),3);
    }
}
#[test]
fn executor_allocation_concurrent_same_paid_fact_has_one_original_grant() {
    let _real = RealSpecialistTransport::enter();
    let (f, mapper, frame, decision, seen) = executor_allocation_closed_mapper_fixture((60_000, 6));
    let barrier = std::sync::Barrier::new(2);
    let (one, two) = std::thread::scope(|scope| {
        let one = scope.spawn(|| {
            barrier.wait();
            executor_allocation_admit(&f, &mapper, &frame, &decision)
        });
        let two = scope.spawn(|| {
            barrier.wait();
            executor_allocation_admit(&f, &mapper, &frame, &decision)
        });
        (one.join().unwrap().unwrap(), two.join().unwrap().unwrap())
    });
    assert_eq!(one, two);
    let db = db::open(&f.context.db_path).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM agent_assignment_attempts WHERE assignment_id=?1",
            [&one.assignment_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(db.query_row("SELECT COUNT(*) FROM agent_budget_entries WHERE assignment_id=?1 AND kind='reserve' AND source_id=?2 AND dimension IN ('model_input_tokens','model_cached_tokens','model_output_tokens','model_requests')",params![one.assignment_id,format!("assignment:{}",one.assignment_id)],|r|r.get::<_,i64>(0)).unwrap(),4);
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::root::remaining_child_capacity(
            &db,
            &f.actor.root_run_id
        )
        .unwrap(),
        (Some(31_000), Some(2))
    );
    assert_eq!(
        seen.lock().unwrap().len(),
        3,
        "actual Root/Mapper ancestors; executor SDK parallelism not claimed"
    );
}
