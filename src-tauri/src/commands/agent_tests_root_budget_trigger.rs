// Full original creator -> Root/Mapper -> Web SDK/Broker -> Client -> Root chain.
#[test]
fn root_budget_trigger_actual_executor_entry_assesses_original_allocation_before_target_once() {
    let _real = RealSpecialistTransport::enter();
    let mut h = client_root_fixture("root-budget-trigger-production", false);
    h.finish().unwrap();
    let db = db::open(&h.h.db_path).unwrap();
    assert_eq!(h.root_cost(&db),4,"Root must assess the actual original allocation before the Web model/target, plus its existing bootstrap/Mapper/Client facts");
    assert_eq!(root_tick_count(&db, &h.root, "publication"), 4);
    assert_eq!(h.h.site_seen.lock().unwrap().len(), 1);
    let before = web_mode_test_rows(&db);
    let calls = h.h.model_seen.lock().unwrap().len();
    h.feedback(&db).unwrap();
    web_mode_assert_rows(&db, &before);
    assert_eq!(h.h.model_seen.lock().unwrap().len(), calls);
    let wires = h.h.model_seen.lock().unwrap();
    let budget = wires
        .iter()
        .position(|w| w.contains("You are the Root Coordinator") && w.contains("budget-allocation"))
        .expect("actual paid Root budget-change request");
    let executor = wires
        .iter()
        .position(|w| {
            w.contains("replay_http")
                && !w.contains("You are the Root Coordinator")
                && !w.contains("SPA/API Mapper")
        })
        .unwrap();
    assert!(
        budget < executor,
        "Root budget supervision precedes any Web executor SDK"
    );
}

#[test]
fn root_budget_trigger_same_running_worker_paid_fact_replay_is_zero_write_zero_sdk() {
    let _real = RealSpecialistTransport::enter();
    let f = budget_trigger_fixture(0);
    let db = db::open(&f.h.db_path).unwrap();
    let root = &f.session.as_ref().unwrap().lease.root_run_id;
    let child = &f.session.as_ref().unwrap().executor;
    let original = crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(
        &db,
        "SELECT rowid,* FROM agent_assignments WHERE id=?1",
        [&child.assignment_id],
    )
    .unwrap();
    native_coordinator_budget_before_executor(&f.h.context).unwrap();
    assert_eq!(f.h.model_seen.lock().unwrap().len(), 4);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 0);
    assert_eq!(root_tick_count(&db, root, "publication"), 3);
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(&db, root, Some(""), "model_requests")
            .unwrap()
            .consumed,
        3
    );
    assert!(
        crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(
            &db,
            "SELECT rowid,* FROM agent_assignments WHERE id=?1",
            [&child.assignment_id]
        )
        .unwrap()
            == original
    );
    let before = web_mode_test_rows(&db);
    native_coordinator_budget_before_executor(&f.h.context).unwrap();
    web_mode_assert_rows(&db, &before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), 4);
    // A paid assessment can never become another target-dispatch capability.
    let session = f.session.as_ref().unwrap();
    let root_context = native_coordinator_root_context(&f.h.context, &session.lease);
    let (origin, paid) =
        native_coordinator_mapper_decision(&root_context, &session.lease, &session.mapper).unwrap();
    let frame = NativeCoordinatorFrame::budget_allocation(
        &db,
        &root_context,
        &session.lease,
        child,
        origin,
        &paid,
    )
    .unwrap();
    let assessment =
        native_coordinator_tick_for_frame(&root_context, &session.lease, &frame).unwrap();
    assert!(assessment.replayed);
    assert_eq!(
        native_coordinator_rust_dispatch_policy(
            &root_context,
            &frame,
            &assessment,
            crate::agent_runtime::contract::AgentRole::WebExecutor,
            512,
            1,
            [Some(60000), Some(20), Some(32)]
        )
        .unwrap_err(),
        "root_trigger_role_conflict"
    );
    web_mode_assert_rows(&db, &before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), 4);
}
