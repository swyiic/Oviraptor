// Actual finisher/finalization only, no model/target/browser transport.
fn exceptional_terminal_financial_rows(db: &rusqlite::Connection) -> Vec<(String, Vec<String>)> {
    super::tests::application_table_snapshot(db)
        .into_iter()
        .filter(|(t, _)| {
            matches!(
                t.as_str(),
                "agent_root_budget_attempts"
                    | "agent_budget_limits"
                    | "agent_budget_clock_origins"
                    | "agent_budget_entries"
                    | "agent_root_elapsed_facts"
            )
        })
        .collect()
}

#[test]
fn exceptional_terminal_actual_noncompletion_closes_with_full_original_fact_without_ledger_clipping(
) {
    use crate::agent_runtime::multi_agent::budget::{self, clock::elapsed_fact};
    for outcome in [
        AgentTargetOutcome::Cancelled,
        AgentTargetOutcome::failed("original provider failure without any send"),
        AgentTargetOutcome::incomplete("original unfinished work"),
        AgentTargetOutcome::Limited(AgentStop::new(
            terminal_code::HARD_WALL_TIME_BUDGET,
            "original wall stop",
        )),
        AgentTargetOutcome::ResumeIncompatible(AgentStop::new(
            terminal_code::RESUME_INCOMPATIBLE,
            "original checkpoint incompatibility",
        )),
    ] {
        let (directory, db, root, lease) = exceptional_short_root();
        let origin: String = db
            .query_row(
                "SELECT started_at FROM agent_budget_clock_origins WHERE root_run_id=?1",
                [&root],
                |r| r.get(0),
            )
            .unwrap();
        let original_physical: Vec<Vec<String>> = final_elapsed_physical_rows(&db)
            .into_iter()
            .skip(1)
            .collect();
        let result = finish_coordinator_run(&db, &lease, &outcome);
        assert!(result.is_ok(), "{}: {result:?}", outcome.terminal_code());
        let fact = elapsed_fact::read(&db, &root).unwrap().unwrap();
        assert!(fact.exhausted && fact.elapsed >= 1050 && fact.unsettled > 0);
        assert_eq!((fact.hard, fact.journaled), (1000, 0));
        let row:(String,String,String,String)=db.query_row("SELECT status,terminal_state,terminal_code,finished_at FROM agent_runs WHERE id=?1",[&root],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(
            row,
            (
                "terminal".into(),
                outcome.terminal_status().into(),
                outcome.terminal_code().into(),
                fact.cutoff.clone()
            )
        );
        assert_eq!(
            db.query_row(
                "SELECT started_at FROM agent_budget_clock_origins WHERE root_run_id=?1",
                [&root],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            origin
        );
        assert_eq!(
            budget::balance(&db, &root, None, "wall_time_ms").unwrap(),
            Default::default()
        );
        assert_eq!(
            final_elapsed_physical_rows(&db)
                .into_iter()
                .skip(1)
                .collect::<Vec<_>>(),
            original_physical,
            "original control/clock/limits/fee rowids and values cannot change"
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM agent_assignments", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert!(budget::admission::require_determinate(&db, &root).is_err());
        let rows = super::tests::application_table_snapshot(&db);
        let physical = final_elapsed_physical_rows(&db);
        std::thread::sleep(std::time::Duration::from_millis(35));
        finish_coordinator_run(&db, &lease, &outcome).unwrap();
        assert!(
            super::tests::application_table_snapshot(&db) == rows,
            "replay cannot add a terminal event or post-close fee"
        );
        assert_eq!(final_elapsed_physical_rows(&db), physical);
        assert_eq!(elapsed_fact::read(&db, &root).unwrap().unwrap(), fact);
        drop(db);
        fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn exceptional_terminal_actual_completed_and_bounded_result_use_real_limited_finalization_fallback()
{
    use crate::agent_runtime::multi_agent::budget::clock::elapsed_fact;
    for outcome in [
        AgentTargetOutcome::Completed(AgentCompletion::without_ledger(
            "reported completed",
            terminal_code::LEDGER_COMPLETE,
        )),
        AgentTargetOutcome::BoundedCompleted(AgentCompletion::bounded("reported bounded complete")),
    ] {
        let (directory, db, root, lease) = exceptional_short_root();
        assert_eq!(
            finish_coordinator_run(&db, &lease, &outcome).unwrap_err(),
            "budget_wall_time_exhausted"
        );
        let fact = elapsed_fact::read(&db, &root).unwrap().unwrap();
        let original = exceptional_terminal_financial_rows(&db);
        let mut context = test_context(
            std::path::Path::new(db.path().unwrap()),
            &lease.target_key,
            Vec::new(),
        );
        context.scan_id = lease.scan_id.clone();
        context.attempt_number = lease.attempt_number;
        context.target_dir = directory.clone();
        context.log_path = directory.join("financial-terminal.log");
        let result = finalize_agent_target(&context, &lease, outcome);
        assert!(
            matches!(&result, AgentTargetOutcome::Limited(_)),
            "{result:?}"
        );
        assert_eq!(result.terminal_code(), terminal_code::HARD_WALL_TIME_BUDGET);
        let actual: (String, String, String) = db
            .query_row(
                "SELECT terminal_state,terminal_code,finished_at FROM agent_runs WHERE id=?1",
                [&root],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            actual,
            (
                "protected_stop".into(),
                terminal_code::HARD_WALL_TIME_BUDGET.into(),
                fact.cutoff.clone()
            )
        );
        assert_eq!(exceptional_terminal_financial_rows(&db), original);
        assert_eq!(elapsed_fact::read(&db, &root).unwrap().unwrap(), fact);
        drop(db);
        fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn exceptional_terminal_saved_fact_does_not_let_expired_or_successor_c_publish() {
    use crate::agent_runtime::multi_agent::budget::clock::elapsed_fact;
    let (directory, db, root, lease) = final_elapsed_fixture("exceptional-terminal-no-grant");
    let expires: String = db
        .query_row(
            "SELECT lease_expires_at FROM agent_coordinator_leases WHERE root_run_id=?1",
            [&root],
            |r| r.get(0),
        )
        .unwrap();
    db.execute("UPDATE agent_coordinator_leases SET lease_expires_at=datetime('now','-1 day','localtime') WHERE root_run_id=?1",[&root]).unwrap();
    elapsed_fact::record_if_exceptional(&db, &lease)
        .unwrap()
        .unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert_eq!(
        finish_coordinator_run(&db, &lease, &AgentTargetOutcome::Cancelled).unwrap_err(),
        "stale_coordinator_fencing_token"
    );
    assert!(super::tests::application_table_snapshot(&db) == before);
    let mut successor = lease.clone();
    successor.lease_epoch += 1;
    successor.fencing_token = uuid::Uuid::new_v4().to_string();
    db.execute("UPDATE agent_coordinator_leases SET lease_epoch=?2,fencing_token=?3,lease_expires_at=?4 WHERE root_run_id=?1",params![root,successor.lease_epoch,successor.fencing_token,expires]).unwrap();
    crate::agent_runtime::multi_agent::lease::validate_coordinator_lease(&db, &successor).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert!(finish_coordinator_run(&db, &successor, &AgentTargetOutcome::Cancelled).is_err());
    assert!(super::tests::application_table_snapshot(&db) == before);
    drop(db);
    fs::remove_dir_all(directory).unwrap();
}
