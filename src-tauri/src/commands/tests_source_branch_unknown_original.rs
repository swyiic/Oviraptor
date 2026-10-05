#[test]
fn source_branch_actual_unknown_sdk_usage_is_not_refunded_or_reported_complete() {
    use crate::agent_runtime::multi_agent::budget;
    let (root, db, record, report, guard, calls) = source_branch_production_report(true);
    let path = root.join("oviraptor.sqlite3");
    let (actor,state,terminal):(String,String,String)=db.query_row("SELECT id,status,terminal_state FROM agent_runs WHERE scan_id=?1 AND role='coordinator'",[&record.scan_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    eprintln!(
        "Actual unknown Source SDK: calls={calls}, Root={state}/{terminal}, report={}",
        report["sourceMultiAgent"]
    );
    assert_eq!(calls, 1);
    assert_eq!(report["sourceMultiAgent"]["status"], "incomplete");
    let before = source_exit_snapshot(&db);
    let balances = [
        "model_requests",
        "model_input_tokens",
        "model_output_tokens",
    ]
    .map(|dimension| {
        let b = budget::balance(&db, &actor, None, dimension).unwrap();
        eprintln!(
            "actual unknown {dimension}: consumed={}, indeterminate={}",
            b.consumed, b.indeterminate
        );
        (b.consumed, b.indeterminate)
    });
    assert_eq!((state.as_str(), terminal.as_str()), ("terminal", "paused"));
    assert_eq!(balances, [(0, 1), (0, 4406), (0, 4406)]);
    let result = finish_native_source_branch(&path, &record.scan_id, 1, &report);
    eprintln!("Actual unknown Source branch consumption: {result:?}");
    assert_eq!(result, Ok(true));
    let scan: String = db
        .query_row(
            "SELECT status FROM sentinel_scans WHERE id=?1",
            [&record.scan_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(scan, "partial");
    let after = source_exit_snapshot(&db);
    for ((name, old), (actual, new)) in before.iter().zip(after.iter()) {
        assert_eq!(name, actual);
        if ![
            "sentinel_scans",
            "sentinel_scan_attempts",
            "sentinel_targets",
            "native_scan_branches",
        ]
        .contains(&name.as_str())
        {
            assert_eq!(old, new, "unknown original rows changed {name}");
        }
    }
    let after = source_exit_snapshot(&db);
    let again = finish_native_source_branch(&path, &record.scan_id, 1, &report);
    assert_eq!(again, Ok(false));
    let tx = db.unchecked_transaction().unwrap();
    budget::clock::FinalClock::verify_original_exit(&tx, &actor).unwrap();
    tx.rollback().unwrap();
    assert_eq!(source_exit_snapshot(&db), after);
    for (dimension, expected) in [
        "model_requests",
        "model_input_tokens",
        "model_output_tokens",
    ]
    .into_iter()
    .zip(balances)
    {
        let b = budget::balance(&db, &actor, None, dimension).unwrap();
        assert_eq!((b.consumed, b.indeterminate), expected);
    }
    drop(guard);
    assert_eq!(source_exit_snapshot(&db), after);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
