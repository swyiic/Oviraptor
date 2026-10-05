#[test]
fn source_coverage_budget_saved_paused_receipt_releases_slot_atomically_without_grants() {
    use crate::agent_runtime::{
        contract::AgentRole,
        multi_agent::{budget, scheduler::ScheduledChild},
    };
    for coverage in [true, false] {
        let (root, connection, _, result) = source_reviewer_execution_fixture_using_calls(
            if coverage { "no_candidates" } else { "valid" },
            None,
            if coverage { 4 } else { 3 },
            None,
            7,
            |root, connection, record, lease| {
                if coverage {
                    source_coverage_checkpoint_fixture(
                        root,
                        connection,
                        record,
                        lease,
                        "paused_received",
                    );
                } else {
                    source_reviewer_checkpoint_fixture(
                        root,
                        connection,
                        record,
                        lease,
                        "paused_received",
                    );
                }
                let (assignment_id, run_id) = connection.query_row(
                    "SELECT id,child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND trigger_code=?2",
                    params![lease.root_run_id,if coverage {"source_coverage_ready"} else {"source_candidates_ready"}], |r|Ok((r.get(0)?,r.get(1)?)),
                ).unwrap();
                let child = ScheduledChild {
                    assignment_id,
                    run_id,
                    role: AgentRole::EvidenceReviewer,
                };
                let own = || {
                    budget::balance(
                        connection,
                        &lease.root_run_id,
                        Some(&child.assignment_id),
                        "concurrency_batches",
                    )
                    .unwrap()
                };
                assert_eq!(own().reserved, 1);
                let database = root.join("oviraptor.sqlite3");
                let work = root.join("attempt-0001");
                let (model, runtime, _) =
                    verify_source_runtime_contract(connection, &record.scan_id, 1, &work).unwrap();
                let proxy = source_runtime_proxy(&runtime).unwrap();
                let context = SpecialistTransportContext { supervision: None,
                    db_path: &database,
                    scan_id: &record.scan_id,
                    attempt_number: 1,
                    target_key: &lease.target_key,
                    run_id: &lease.root_run_id,
                    environment: &model,
                    proxy,
                    usage_dir: &work,
                    deadline: Some(std::time::Instant::now() + Duration::from_secs(180)),
                };
                // A saved receipt is local completion authority only. No live
                // capabilities may be restored to release a worker's slot.
                connection.execute_batch("CREATE TRIGGER no_coverage_regrant_insert BEFORE INSERT ON agent_capability_leases
                    WHEN NEW.revoked_at='' BEGIN SELECT RAISE(ABORT,'no regrant'); END;
                    CREATE TRIGGER no_coverage_regrant_update BEFORE UPDATE ON agent_capability_leases
                    WHEN NEW.revoked_at='' BEGIN SELECT RAISE(ABORT,'no renewal'); END;").unwrap();
                let complete = || {
                    if coverage {
                        complete_source_coverage_review(connection, &context, lease, &child)
                    } else {
                        complete_source_candidate_review(connection, &context, lease, &child)
                    }
                };
                for fault in ["IGNORE", "ABORT", "cancel"] {
                    let body: String = if fault == "cancel" {
                        "UPDATE agent_runs SET cancel_requested_at='cancelled-during-slot-release' WHERE role='coordinator';".into()
                    } else if fault == "IGNORE" {
                        "SELECT RAISE(IGNORE);".into()
                    } else {
                        "SELECT RAISE(ABORT,'slot release unavailable');".into()
                    };
                    connection.execute_batch(&format!("CREATE TRIGGER fail_coverage_slot {} INSERT ON agent_budget_entries
                        WHEN NEW.dimension='concurrency_batches' AND NEW.kind='release' BEGIN {body} END;",
                        if fault=="cancel" {"AFTER"} else {"BEFORE"})).unwrap();
                    let before = source_reviewer_reentry_state(connection);
                    assert!(complete().is_err(), "{coverage}/{fault}");
                    assert_eq!(source_reviewer_reentry_state(connection), before);
                    assert_eq!(own().reserved, 1);
                    // Fault removal permits only the same local receipt to
                    // complete, without a new provider request or new grants.
                    connection
                        .execute_batch("DROP TRIGGER fail_coverage_slot")
                        .unwrap();
                }
                let payload = complete()?;
                assert_eq!(own().reserved, 0);
                assert_eq!(connection.query_row("SELECT count(*) FROM agent_capability_leases WHERE root_run_id=?1 AND revoked_at=''",
                    [&lease.root_run_id], |r|r.get::<_,i64>(0)).unwrap(), 0);
                assert_eq!(connection.query_row("SELECT spent_requests,reserved_requests FROM agent_budget_ledger WHERE root_run_id=?1",
                    [&lease.root_run_id], |r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?))).unwrap(), (7,0));
                let completed_state = source_reviewer_reentry_state(connection);
                assert_eq!(complete()?, payload);
                assert_eq!(source_reviewer_reentry_state(connection), completed_state);
                Ok(payload)
            },
        );
        assert!(result.is_ok());
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}
