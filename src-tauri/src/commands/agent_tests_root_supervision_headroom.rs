// Actual SDK feedback while the original Web grant is running and held.
fn root_supervision_headroom_feedback_case(limits: (i64, i64), reply: u8, total_calls: usize) {
    let _real = RealSpecialistTransport::enter();
    let h = root_supervision_headroom_fixture(limits, reply);
    let f = &h.f;
    let db = db::open(&f.context.db_path).unwrap();
    let executor = h.admit().unwrap();
    use crate::agent_runtime::multi_agent::{attempts::audit_rows::Rows, budget, scheduler};
    scheduler::start_child_or_release(&db, &f.actor, &executor).unwrap();
    let grant = budget::model::original_web_grant(&db, &f.actor, &executor.assignment_id).unwrap();
    let identity = h.close_identity();
    let held = Rows::read(
        &db,
        "SELECT rowid,* FROM agent_assignments WHERE id=?1",
        [&executor.assignment_id],
    )
    .unwrap();
    let decision=native_coordinator_identity_feedback(&f.context,&f.actor,&h.mapper,&identity).expect("Root must assess the actual closed new fact without spending Reviewer floor or the held worker grant");
    assert!(!decision.replayed);
    assert_eq!(h.seen.lock().unwrap().len(), total_calls);
    assert_eq!(
        budget::model::original_web_grant(&db, &f.actor, &executor.assignment_id).unwrap(),
        grant
    );
    assert!(
        Rows::read(
            &db,
            "SELECT rowid,* FROM agent_assignments WHERE id=?1",
            [&executor.assignment_id]
        )
        .unwrap()
            == held
    );
    budget::root::require_child_capacity(&db, &f.actor.root_run_id, 15_000, 1).unwrap();
    let root_calls = if reply == 2 { 4 } else { 3 };
    assert_eq!(
        budget::balance(&db, &f.actor.root_run_id, Some(""), "model_requests")
            .unwrap()
            .consumed,
        root_calls
    );
    if reply == 2 {
        let wires = h.seen.lock().unwrap();
        let body: JsonValue =
            serde_json::from_str(wires.last().unwrap().split_once("\r\n\r\n").unwrap().1).unwrap();
        assert!(body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["role"] == "tool" && m["tool_call_id"] == "held-budget-observation"));
    }
    let before = web_mode_test_rows(&db);
    assert!(
        native_coordinator_identity_feedback(&f.context, &f.actor, &h.mapper, &identity)
            .unwrap()
            .replayed
    );
    web_mode_assert_rows(&db, &before);
    assert_eq!(h.seen.lock().unwrap().len(), total_calls);
}
#[test]
fn root_supervision_headroom_actual_identity_feedback_runs_while_original_executor_holds_budget() {
    root_supervision_headroom_feedback_case((60_000, 7), 0, 5);
}
#[test]
fn root_supervision_headroom_actual_local_budget_tool_and_second_sdk_keep_running_worker_original()
{
    root_supervision_headroom_feedback_case((60_000, 8), 2, 6);
}
