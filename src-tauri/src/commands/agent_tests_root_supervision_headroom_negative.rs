#[test]
fn root_supervision_headroom_insufficient_token_or_request_capacity_refuses_before_any_web_grant() {
    let _real = RealSpecialistTransport::enter();
    for limits in [(31_000, 20), (60_000, 5)] {
        let h = root_supervision_headroom_fixture(limits, 0);
        let f = &h.f;
        let db = db::open(&f.context.db_path).unwrap();
        let before = web_mode_test_rows(&db);
        assert_eq!(
            h.admit().unwrap_err(),
            "multi_agent_executor_budget_unavailable"
        );
        web_mode_assert_rows(&db, &before);
        assert_eq!(h.seen.lock().unwrap().len(), 3);
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM agent_assignments WHERE role='web_executor'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            crate::agent_runtime::multi_agent::budget::balance(
                &db,
                &f.actor.root_run_id,
                Some(""),
                "model_requests"
            )
            .unwrap()
            .consumed,
            2
        );
    }
}
#[test]
fn root_supervision_headroom_actual_unknown_feedback_keeps_original_worker_and_debt_without_retry()
{
    let _real = RealSpecialistTransport::enter();
    let h = root_supervision_headroom_fixture((60_000, 7), 1);
    let f = &h.f;
    let db = db::open(&f.context.db_path).unwrap();
    let executor = h.admit().unwrap();
    use crate::agent_runtime::multi_agent::{attempts::audit_rows::Rows, budget, scheduler};
    scheduler::start_child_or_release(&db, &f.actor, &executor).unwrap();
    let identity = h.close_identity();
    let held = Rows::read(
        &db,
        "SELECT rowid,* FROM agent_assignments WHERE id=?1",
        [&executor.assignment_id],
    )
    .unwrap();
    let error = native_coordinator_identity_feedback(&f.context, &f.actor, &h.mapper, &identity)
        .err()
        .unwrap();
    assert!(error.starts_with("root_tick_uncertain:"), "{error}");
    assert_eq!(h.seen.lock().unwrap().len(), 5);
    assert!(
        Rows::read(
            &db,
            "SELECT rowid,* FROM agent_assignments WHERE id=?1",
            [&executor.assignment_id]
        )
        .unwrap()
            == held
    );
    assert_eq!(
        budget::balance(&db, &f.actor.root_run_id, Some(""), "model_requests")
            .unwrap()
            .indeterminate,
        1
    );
    assert!(
        budget::balance(&db, &f.actor.root_run_id, Some(""), "model_input_tokens")
            .unwrap()
            .indeterminate
            > 0
    );
    let before = web_mode_test_rows(&db);
    assert!(
        native_coordinator_identity_feedback(&f.context, &f.actor, &h.mapper, &identity).is_err()
    );
    assert_eq!(
        h.admit().unwrap_err(),
        "budget_indeterminate_requires_reconciliation"
    );
    web_mode_assert_rows(&db, &before);
    assert_eq!(h.seen.lock().unwrap().len(), 5);
}
