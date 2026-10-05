#[test]
fn coordinator_mapper_allocation_paid_root_request_floor_and_active_reservation_deny_zero_write() {
    for pressure in [false, true] {
        let (f, decision, seen) =
            mapper_allocation_fixture((23_000, if pressure { 20 } else { 2 }));
        let db = db::open(&f.context.db_path).unwrap();
        if pressure {
            crate::agent_runtime::multi_agent::scheduler::prepare_readonly_child(
                &db,
                &f.actor,
                crate::agent_runtime::contract::AgentRole::SpaApiMapper,
                "other_frozen_inventory",
                &json!({"objective":"current held capacity"}),
                7_980,
            )
            .unwrap();
        }
        let before = web_mode_test_rows(&db);
        let error = native_coordinator_prepare_mapper(
            &f.context,
            &f.actor,
            &decision,
            &bootstrap_mapper_task(&f),
        )
        .unwrap_err();
        assert_eq!(error, "mapper_allocation_reviewer_floor_exhausted");
        web_mode_assert_rows(&db, &before);
        assert_eq!(
            seen.lock().unwrap().len(),
            1,
            "paid Root retained; no Mapper SDK or refund"
        );
    }
}

#[test]
fn coordinator_mapper_allocation_original_grant_and_revoked_worker_replay_deny_zero_write() {
    for damage in [
        "tokens",
        "requests",
        "task",
        "invoice_guard",
        "revoked_cap",
        "expired_worker",
        "lane",
        "root",
    ] {
        let (f, decision, seen) = mapper_allocation_fixture((23_000, 20));
        let task = bootstrap_mapper_task(&f);
        let mapper =
            native_coordinator_prepare_mapper(&f.context, &f.actor, &decision, &task).unwrap();
        let db = db::open(&f.context.db_path).unwrap();
        match damage {
            "tokens" => {
                db.execute(
                    "UPDATE agent_assignments SET reserved_tokens=8000 WHERE id=?1",
                    [&mapper.assignment_id],
                )
                .unwrap();
            }
            "requests" => {
                db.execute(
                    "UPDATE agent_assignments SET reserved_requests=2 WHERE id=?1",
                    [&mapper.assignment_id],
                )
                .unwrap();
            }
            "task" => {
                db.execute("UPDATE agent_assignments SET task_slice_json=json_set(task_slice_json,'$.rootDecision.rustPolicy.reviewerFloor.modelTokens',0) WHERE id=?1",[&mapper.assignment_id]).unwrap();
            }
            "invoice_guard" => {
                db.execute_batch("DROP TRIGGER budget_entry_no_update")
                    .unwrap();
            }
            "revoked_cap" => {
                db.execute("UPDATE agent_capability_leases SET revoked_at=datetime('now','localtime') WHERE assignment_id=?1",[&mapper.assignment_id]).unwrap();
            }
            "expired_worker" => {
                db.execute("UPDATE agent_assignment_attempts SET expires_at='2000-01-01 00:00:00' WHERE assignment_id=?1",[&mapper.assignment_id]).unwrap();
            }
            "lane" => {
                db.execute(
                    "DELETE FROM agent_lane_leases WHERE assignment_id=?1",
                    [&mapper.assignment_id],
                )
                .unwrap();
            }
            "root" => {
                db.execute(
                    "UPDATE agent_runs SET status='paused' WHERE id=?1",
                    [&f.actor.root_run_id],
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        if damage == "expired_worker" {
            human_directive_wait_original_worker_expiry(&db,
                f.context.supervision.as_ref().unwrap(), &mapper.run_id);
        }
        let before = web_mode_test_rows(&db);
        assert!(
            native_coordinator_prepare_mapper(&f.context, &f.actor, &decision, &task).is_err(),
            "{damage}"
        );
        web_mode_assert_rows(&db, &before);
        assert_eq!(seen.lock().unwrap().len(), 1);
    }
}

#[test]
fn coordinator_mapper_allocation_invoice_write_faults_rollback_paid_grant_and_business() {
    for sql in [
        "CREATE TRIGGER mapper_fault BEFORE INSERT ON agent_budget_entries WHEN NEW.assignment_id<>'' AND NEW.dimension='model_cached_tokens' BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER mapper_fault AFTER INSERT ON agent_assignments BEGIN UPDATE projects SET name='escaped'; END;",
        "CREATE TRIGGER mapper_fault BEFORE UPDATE OF state ON agent_assignments WHEN NEW.state='running' BEGIN SELECT RAISE(IGNORE); END;",
    ] {
        let (f,decision,seen)=mapper_allocation_fixture((23_000,20));
        let db=db::open(&f.context.db_path).unwrap(); db.execute_batch(sql).unwrap();
        let before=web_mode_test_rows(&db);
        assert!(native_coordinator_prepare_mapper(&f.context,&f.actor,&decision,&bootstrap_mapper_task(&f)).is_err());
        web_mode_assert_rows(&db,&before);
        assert_eq!(seen.lock().unwrap().len(),1);
    }
}

#[test]
fn coordinator_mapper_allocation_actual_unknown_sdk_retains_debt_and_cannot_reallocate() {
    let _real = RealSpecialistTransport::enter();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|request| {
        if request.contains("You are the Root Coordinator") {
            (
                200,
                "application/json",
                changed_fact_response(&request, true),
            )
        } else {
            (
                503,
                "application/json",
                json!({"error":"actual unknown Mapper outcome"}).to_string(),
            )
        }
    }));
    let f = root_tick_fixture_protocol_limits(
        "mapper-allocation-unknown",
        &format!("http://127.0.0.1:{port}/v1"),
        true,
        (23_000, 20),
    );
    let decision = native_coordinator_tick(&f.context, &f.actor).unwrap();
    let task = bootstrap_mapper_task(&f);
    let mapper = native_coordinator_prepare_mapper(&f.context, &f.actor, &decision, &task).unwrap();
    assert!(multi_agent_child_round(
        &f.context,
        &f.actor,
        &mapper,
        "Independent Mapper",
        json!({"frozenEvidence":f.context.evidence})
    )
    .unwrap_err()
    .contains("503"));
    let db = db::open(&f.context.db_path).unwrap();
    let debt = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &f.actor.root_run_id,
        Some(&mapper.assignment_id),
        "model_requests",
    )
    .unwrap();
    assert_eq!((debt.consumed, debt.indeterminate), (0, 1));
    let before = web_mode_test_rows(&db);
    assert!(native_coordinator_prepare_mapper(&f.context, &f.actor, &decision, &task).is_err());
    web_mode_assert_rows(&db, &before);
    assert_eq!(
        seen.lock().unwrap().len(),
        2,
        "no replacement worker or SDK after actual unknown bill"
    );
}

#[test]
fn coordinator_mapper_allocation_two_paid_root_rounds_use_current_balance_not_observation() {
    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let n = count.clone();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |request| {
        let body = if n.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            root_local_wire(
                json!([{"id":"allocation-budget-0","type":"function","function":{"name":"capability_budget.read","arguments":"{}"}}]),
                "",
                true,
            )
        } else {
            changed_fact_response(&request, true)
        };
        (200, "application/json", body)
    }));
    let f = root_tick_fixture_protocol_limits(
        "mapper-allocation-two-root",
        &format!("http://127.0.0.1:{port}/v1"),
        true,
        (60_000, 20),
    );
    let decision = native_coordinator_tick(&f.context, &f.actor).unwrap();
    assert_eq!(seen.lock().unwrap().len(), 2);
    let mapper = native_coordinator_prepare_mapper(
        &f.context,
        &f.actor,
        &decision,
        &bootstrap_mapper_task(&f),
    )
    .unwrap();
    let db = db::open(&f.context.db_path).unwrap();
    let tokens: i64 = db
        .query_row(
            "SELECT reserved_tokens FROM agent_assignments WHERE id=?1",
            [&mapper.assignment_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        tokens, 8_000,
        "both original paid calls are debited before allocating"
    );
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::root::remaining_child_capacity(
            &db,
            &f.actor.root_run_id
        )
        .unwrap(),
        (Some(51_960), Some(17))
    );
    let before = web_mode_test_rows(&db);
    assert_eq!(
        native_coordinator_prepare_mapper(
            &f.context,
            &f.actor,
            &decision,
            &bootstrap_mapper_task(&f)
        )
        .unwrap(),
        mapper
    );
    web_mode_assert_rows(&db, &before);
    assert_eq!(seen.lock().unwrap().len(), 2);
}
