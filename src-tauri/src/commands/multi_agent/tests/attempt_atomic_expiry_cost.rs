#[test]
fn assignment_attempt_atomic_expiry_worker_deadline_routes_specialist_to_fee_only_fact() {
    use crate::agent_runtime::multi_agent::{budget, lease as authority, specialist};
    for phase in ["received", "uncertain", "unsent"] {
        let (root, context, lease, child) = specialist_journal_fixture();
        let db = db::open(&context.db_path).unwrap();
        let specialist::Start::Dispatch(call) =
            specialist::start(&db, &lease, &child, &json!({"messages":[]})).unwrap()
        else {
            panic!()
        };
        expire_worker_deadline(&db, &child.run_id);
        authority::validate_coordinator_lease(&db, &lease).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        match phase {
            "received" => {
                assert!(specialist::record_received(
                    &db,
                    &call,
                    "late response",
                    false,
                    &late_web_model_response().usage
                )
                .is_err());
            }
            "uncertain" => {
                specialist::record_uncertain(&db, &call, "unknown original call").unwrap()
            }
            _ => specialist::record_not_sent(&db, &call, "user_cancelled").unwrap(),
        }
        let fact: String = db
            .query_row(
                "SELECT phase FROM agent_model_cost_facts WHERE child_run_id=?1",
                [&child.run_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(fact, phase);
        let cost = budget::balance(
            &db,
            &lease.root_run_id,
            Some(&child.assignment_id),
            "model_requests",
        )
        .unwrap();
        assert_eq!(
            (cost.consumed, cost.indeterminate, cost.reserved),
            match phase {
                "received" => (1, 0, 0),
                "uncertain" => (0, 1, 0),
                _ => (0, 0, 1),
            }
        );
        assert_eq!(
            budget::admission::require_determinate(&db, &lease.root_run_id).is_ok(),
            phase != "uncertain"
        );
        let after = super::tests::application_table_snapshot(&db);
        for (table, rows) in before {
            if !["agent_budget_entries", "agent_model_cost_facts"].contains(&table.as_str()) {
                assert_eq!(
                    after.iter().find(|(name, _)| name == &table).unwrap().1,
                    rows,
                    "{phase}: {table}"
                );
            }
        }
        assert!(specialist::start(&db, &lease, &child, &json!({"messages":[]})).is_err());
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn assignment_attempt_atomic_expiry_pending_original_call_blocks_fresh_budget_without_refund() {
    use crate::agent_runtime::multi_agent::{budget, specialist};
    let (root, context, lease, child) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    let specialist::Start::Dispatch(_) =
        specialist::start(&db, &lease, &child, &json!({"messages":[]})).unwrap()
    else {
        panic!()
    };
    budget::admission::require_determinate(&db, &lease.root_run_id).unwrap();
    expire_worker_deadline(&db, &child.run_id);
    let before = super::tests::application_table_snapshot(&db);
    assert!(
        budget::admission::require_determinate(&db, &lease.root_run_id).is_err(),
        "a stopped pending call has unknown actual cost even before its callback arrives"
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(
        budget::balance(
            &db,
            &lease.root_run_id,
            Some(&child.assignment_id),
            "model_requests"
        )
        .unwrap()
        .reserved,
        1
    );
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_atomic_expiry_missing_worker_never_makes_pending_cost_determinate() {
    use crate::agent_runtime::multi_agent::{budget, specialist};
    let (root, context, lease, child) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    let specialist::Start::Dispatch(_) =
        specialist::start(&db, &lease, &child, &json!({"messages":[]})).unwrap()
    else {
        panic!()
    };
    let intact = super::tests::application_table_snapshot(&db);
    assert!(db
        .execute(
            "DELETE FROM agent_assignment_attempts WHERE child_run_id=?1",
            [&child.run_id]
        )
        .unwrap_err()
        .to_string()
        .contains("assignment_attempt_audit_immutable"));
    assert_eq!(super::tests::application_table_snapshot(&db), intact);
    // Restore corruption only in this temporary DB; preserve the production
    // deletion guard above rather than relaxing it to make the fixture work.
    db.execute_batch("DROP TRIGGER assignment_attempt_no_delete")
        .unwrap();
    db.execute(
        "DELETE FROM agent_assignment_attempts WHERE child_run_id=?1",
        [&child.run_id],
    )
    .unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert!(
        budget::admission::require_determinate(&db, &lease.root_run_id).is_err(),
        "missing worker audit cannot hide an unresolved original dispatch"
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
