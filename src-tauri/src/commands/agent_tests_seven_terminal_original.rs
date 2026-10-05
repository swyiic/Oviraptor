include!("agent_tests_seven_terminal_producers.rs");

fn seven_terminal_verify(h: &AgentHarness, owned: &OwnedAgentTargetOutcome, expected: &str) {
    use crate::agent_runtime::{contract::TerminalState, multi_agent::budget};
    assert_eq!(
        owned.outcome.terminal_status(),
        expected,
        "{:?}",
        owned.outcome
    );
    let db = db::open(&h.db_path).unwrap();
    let root = h.context.run.as_ref().unwrap().run_id.as_str();
    assert_eq!(owned.original_terminal.root_run_id.as_deref(), Some(root));
    let multi = expected == "resume_incompatible";
    if multi {
        let tx = db.unchecked_transaction().unwrap();
        budget::clock::FinalClock::verify_original_exit(&tx, root).unwrap();
        tx.rollback().unwrap();
    } else {
        budget::root::RootOwner::load_single(&db, root)
            .unwrap()
            .read_single_exit(&db)
            .unwrap();
    }
    let fees: Vec<_> = budget::DIMENSIONS
        .iter()
        .map(|d| (*d, budget::balance(&db, root, None, d).unwrap()))
        .collect();
    let models = h.model_seen.lock().unwrap().len() as i64;
    let requests = h.site_seen.lock().unwrap().len() as i64;
    for (d, b) in &fees {
        assert_eq!((b.reserved, b.indeterminate), (0, 0), "{expected}:{d}");
        if *d == "model_requests" {
            assert_eq!(b.consumed, models);
        }
        if *d == "target_requests" {
            assert_eq!(b.consumed, requests);
        }
        if matches!(*d, "model_input_tokens" | "model_output_tokens") {
            assert_eq!(b.consumed > 0, models > 0);
        }
    }
    match expected {
        "completed" => {
            // The proposed early finish is refused; two actual no-progress
            // windows follow. Every provider round remains in original fees.
            assert_eq!(
                models,
                1 + 2 * h.context.execution_plan.no_progress_window.max(1)
            );
            assert_eq!(requests, 2);
            assert!(owned
                .outcome
                .completion()
                .unwrap()
                .uncovered_families
                .is_empty());
        }
        "completed_with_gaps" => {
            assert_eq!(
                models,
                1 + 2 * h.context.execution_plan.no_progress_window.max(1)
            );
            assert_eq!(requests, 1);
            assert!(!owned
                .outcome
                .completion()
                .unwrap()
                .uncovered_families
                .is_empty());
        }
        "paused" | "resume_incompatible" => assert_eq!((models, requests), (1, 0)),
        "protected_stop" => {
            assert_eq!((models, requests), (1, 1));
            assert_eq!(owned.outcome.terminal_code(), AGENT_STOP_WAF);
        }
        "failed" | "cancelled" => assert_eq!((models, requests), (0, 0)),
        _ => unreachable!(),
    }
    assert!(findings_for(&h.db_path, AGENT_VULNERABILITY_STAGE).is_empty());
    let mut tally = AgentPipelineTally::default();
    let continued = record_owned_agent_target_outcome(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        owned,
        &mut tally,
    );
    assert_eq!(continued, expected != "cancelled");
    assert_eq!(tally.counted(), usize::from(expected != "cancelled"));
    let (rows, stored): (i64, String) = db
        .query_row(
            "SELECT count(*),status FROM sentinel_targets WHERE scan_id=?1 AND url=?2",
            params![h.context.scan_id, h.context.target_url],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(rows, 1, "one target keeps exactly one row");
    assert_eq!(stored, expected);
    let checkpoint = read_agent_checkpoint(
        &h.db_path,
        &h.context.scan_id,
        &h.context.target_url,
        "agent_terminal",
    );
    assert_eq!(checkpoint["status"], expected);
    let reduced: String = db
        .query_row(
            "SELECT terminal_state FROM agent_runs WHERE id=?1",
            [root],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        TerminalState::parse(&reduced).unwrap().to_sentinel_status(),
        expected
    );
    for (d, b) in fees {
        let current = budget::balance(&db, root, None, d).unwrap();
        assert_eq!(
            (current.consumed, current.reserved, current.indeterminate),
            (b.consumed, b.reserved, b.indeterminate),
            "consumer cannot change original fees:{d}"
        );
    }
    let physical = single_finally_physical(&db);
    assert_eq!(
        record_owned_agent_target_outcome(
            &h.db_path,
            &h.context.scan_id,
            &h.context.route,
            owned,
            &mut tally
        ),
        continued
    );
    assert_eq!(tally.counted(), usize::from(expected != "cancelled"));
    assert_eq!(
        single_finally_physical(&db),
        physical,
        "all typed rows/rowids/fees remain on replay:{expected}"
    );
    let mut wrong = h.context.route.clone();
    wrong.url.push_str("/wrong-target");
    assert!(!record_owned_agent_target_outcome(
        &h.db_path,
        &h.context.scan_id,
        &wrong,
        owned,
        &mut tally
    ));
    assert_eq!(
        single_finally_physical(&db),
        physical,
        "wrong target cannot adopt the original outcome"
    );
    db.execute(
        "UPDATE sentinel_scans SET attempt_count=2 WHERE id=?1",
        [&h.context.scan_id],
    )
    .unwrap();
    let rotated = single_finally_physical(&db);
    assert!(!record_owned_agent_target_outcome(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        owned,
        &mut tally
    ));
    assert_eq!(
        single_finally_physical(&db),
        rotated,
        "late owned callback cannot write either attempt"
    );
    assert_eq!(h.model_seen.lock().unwrap().len() as i64, models);
    assert_eq!(h.site_seen.lock().unwrap().len() as i64, requests);
}

#[test]
fn every_target_ends_in_exactly_one_terminal_state() {
    for expected in [
        "completed",
        "completed_with_gaps",
        "paused",
        "protected_stop",
        "cancelled",
        "resume_incompatible",
        "failed",
    ] {
        let (h, owned) = match expected {
            "paused" => missing_side_original_dispatch(),
            "resume_incompatible" => seven_terminal_resume(),
            "cancelled" => seven_terminal_queued_stop("cancelled"),
            other => seven_terminal_single(other),
        };
        seven_terminal_verify(&h, &owned, expected);
        drop(owned);
        fs::remove_dir_all(h.root).unwrap();
    }
    // Preserve the original non-fatal vocabulary assertions.
    for reason in [
        "接口返回 HTTP 401 未登录，仅记录权限边界",
        "接口返回 HTTP 403 无权访问，仅记录权限边界",
        "零确认漏洞，覆盖账本已收口",
        "任务只有一个身份，未做账号 A/B 对照",
    ] {
        let outcome = AgentTargetOutcome::incomplete(reason);
        assert_ne!(outcome.terminal_status(), "failed", "{reason}");
        assert!(!outcome.stop().is_some_and(|stop| stop.requires_fuse()));
    }
    assert_ne!(
        AgentTargetOutcome::Completed(AgentCompletion::without_ledger(
            "没有发现漏洞",
            AGENT_STOP_FINISH
        ))
        .terminal_status(),
        "failed"
    );
}

#[test]
fn original_queued_single_pause_is_not_a_cancelled_terminal() {
    let (h, owned) = seven_terminal_queued_stop("pausing");
    let db = db::open(&h.db_path).unwrap();
    let root = &h.context.run.as_ref().unwrap().run_id;
    let mut tally = AgentPipelineTally::default();
    assert!(!record_owned_agent_target_outcome(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        &owned,
        &mut tally
    ));
    assert_eq!(tally.counted(), 0);
    let (status, terminal): (String, String) = db
        .query_row(
            "SELECT status,terminal_state FROM agent_runs WHERE id=?1",
            [root],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, "paused");
    assert!(terminal.is_empty());
    assert_eq!(
        db.query_row(
            "SELECT status FROM sentinel_targets WHERE scan_id=?1 AND url=?2",
            params![h.context.scan_id, h.context.target_url],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "paused"
    );
    assert!(read_agent_checkpoint(
        &h.db_path,
        &h.context.scan_id,
        &h.context.target_url,
        "agent_terminal"
    )
    .is_null());
    let original = single_finally_physical(&db);
    assert!(!record_owned_agent_target_outcome(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        &owned,
        &mut tally
    ));
    assert_eq!(single_finally_physical(&db), original);
    assert_eq!(tally.counted(), 0);
    drop(owned);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}
