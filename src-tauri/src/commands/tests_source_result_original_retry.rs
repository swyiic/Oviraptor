fn source_result_view_for_attempt(
    db: &rusqlite::Connection,
    scan: &str,
    attempt: i64,
) -> SourceAnalysisView {
    let scope = load_workbench_source_scope(db, scan, attempt).unwrap();
    let snapshot = RepositorySnapshot::restore(db, scan, attempt)
        .unwrap()
        .unwrap();
    let manifest = AnalysisManifest::select(&snapshot, scan, attempt, &json!(scope)).unwrap();
    SourceAnalysisView::restore(db, &manifest, &snapshot).unwrap()
}

fn source_result_retry_original_rows(db: &rusqlite::Connection) -> SourceExitSnapshot {
    source_exit_snapshot(db)
        .into_iter()
        .filter(|(name, _)| {
            name.contains("budget")
                || name.contains("sdk")
                || matches!(
                    name.as_str(),
                    "agent_source_model_rounds"
                        | "source_analysis_results"
                        | "import_record_revisions"
                )
        })
        .collect()
}

#[test]
fn source_result_receipt_new_attempt_owns_results_even_when_revisions_are_reused() {
    use crate::agent_runtime::multi_agent::budget;
    let first_report = Arc::new(std::sync::Mutex::new(JsonValue::Null));
    let first_capture = first_report.clone();
    let imported = Arc::new(std::sync::Mutex::new(None::<String>));
    let late = imported.clone();
    let second_output = Arc::new(std::sync::Mutex::new(JsonValue::Null));
    let second_capture = second_output.clone();
    let first_for_retry = first_report.clone();
    let p = source_broker_original_probe_continued(
        "diff",
        true,
        move |root, db, record, report| {
            assert_eq!(report["sourceClaims"].as_array().unwrap().len(), 1);
            *first_capture.lock().unwrap() = report.clone();
            let actual = report["sourceClaims"][0]["key"]
                .as_str()
                .unwrap()
                .to_owned();
            let frozen = crate::native_pipeline::results::AnalysisResults::load(
                db,
                &source_result_view(db, record),
            )
            .unwrap();
            assert_eq!(frozen.candidates(db).unwrap().len(), 1);
            let root = root.to_owned();
            let scan = record.scan_id.clone();
            SourceBrokerOriginalScript {
                rounds: vec![
                    vec![("analyzer.list_results", json!({}))],
                    vec![(
                        "assignment.finish",
                        json!({"summary":"Original result inspected; independent review required","gaps":[]}),
                    )],
                    vec![("analyzer.get_result", json!({"key":actual}))],
                ],
                before_analyst_response: Some(Box::new(move |number| {
                    if number != 3 {
                        return None;
                    }
                    let db = db::open(&root.join("oviraptor.sqlite3")).unwrap();
                    let first:String=db.query_row("SELECT t.output_json FROM agent_source_tool_receipts t JOIN agent_runs r ON r.id=t.child_run_id WHERE r.scan_id=?1 AND r.attempt_number=2 AND r.role='source_analyst' AND t.round_number=1 AND t.call_index=0 AND t.state='completed'",[&scan],|r|r.get(0)).unwrap();
                    let output: JsonValue = serde_json::from_str(&first).unwrap();
                    assert_eq!(output["attemptNumber"], 2);
                    assert_eq!(output["key"], actual);
                    let view = source_result_view_for_attempt(&db, &scan, 2);
                    let frozen =
                        crate::native_pipeline::results::AnalysisResults::load(&db, &view).unwrap();
                    let before = serde_json::to_string(&frozen).unwrap();
                    let historical = source_result_import_history(&root, &db, &scan);
                    assert_ne!(historical, actual);
                    *late.lock().unwrap() = Some(historical.clone());
                    assert_eq!(
                        serde_json::to_string(
                            &crate::native_pipeline::results::AnalysisResults::load(&db, &view)
                                .unwrap()
                        )
                        .unwrap(),
                        before
                    );
                    Some(vec![
                        ("analyzer.get_result", json!({"key":historical})),
                        ("analyzer.get_result", json!({"key":actual})),
                        ("analyzer.list_results", json!({})),
                    ])
                })),
            }
        },
        None,
        |_, _| {},
        |engine, scratch| source_result_outcome(engine, scratch, "app.py", "shared"),
        Some(Box::new(move |p, seen| {
            assert_eq!(p.calls, 8);
            assert_eq!(seen.lock().unwrap().len(), 8);
            let original = source_result_retry_original_rows(&p.db);
            let old_task = fs::read(p.root.join("attempt-0001/task.json")).unwrap();
            let status: String =
                p.db.query_row(
                    "SELECT status FROM sentinel_scans WHERE id=?1",
                    [&p.record.scan_id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                status, "completed_with_gaps",
                "actual Source branch reduction makes retry eligible"
            );
            let snapshot = source_exit_snapshot(&p.db);
            let (input, basis) = load_workbench_retry_input(&p.db, &p.record.scan_id).unwrap();
            assert_eq!(
                source_exit_snapshot(&p.db),
                snapshot,
                "retry preparation is read-only"
            );
            assert_eq!(basis.attempt, 1);
            assert_eq!(input.source_path, p.record.source_path);
            assert_eq!(input.scope_mode, p.record.scope_mode);
            assert_eq!(input.diff_base, p.record.diff_base);
            let mut next = p.record.clone();
            next.attempt = 2;
            next.retry_basis = Some(basis);
            let plan = ScanBackendPlan {
                scan_id: next.scan_id.clone(),
                attempt_number: 2,
                targets: vec![],
                requires_node: false,
                requires_browser: false,
            };
            publish_workbench_fixture(&p.root, &p.db, &next, &plan, "retry-original", true)
                .unwrap();
            assert_eq!(
                fs::read(p.root.join("attempt-0001/task.json")).unwrap(),
                old_task
            );
            let published = source_exit_snapshot(&p.db);
            let old = run_native_source_assessments(
                &p.root.join("oviraptor.sqlite3"),
                &p.record.scan_id,
                1,
                &p.root.join("attempt-0001"),
            );
            eprintln!("Original Source old attempt after retry: {old:?}");
            assert_eq!(old.unwrap_err(), "source_runtime_attempt_inactive");
            assert!(!finish_native_branch(
                &p.root.join("oviraptor.sqlite3"),
                &p.record.scan_id,
                1,
                "source",
                "completed_with_gaps",
                "stale callback",
                &json!({"forged":true})
            )
            .unwrap());
            assert_eq!(
                source_exit_snapshot(&p.db),
                published,
                "old execution and callback cannot change either attempt"
            );
            assert_eq!(
                seen.lock().unwrap().len(),
                8,
                "old attempt cannot dispatch any SDK"
            );
            let mut branch = claim_workbench_pipeline_branch(
                &p.root.join("oviraptor.sqlite3"),
                &next.scan_id,
                2,
                "source",
            )
            .unwrap();
            let work = p.root.join("attempt-0002");
            let report = run_native_source_scan_using(
                &p.root.join("oviraptor.sqlite3"),
                &p.root,
                &next.scan_id,
                2,
                &work,
                &next.source_path,
                "cicd",
                &next.diff_base,
                |_, engine, _, _, scratch, _| {
                    Ok(source_result_outcome(engine, scratch, "app.py", "shared"))
                },
            )
            .unwrap();
            let first = first_for_retry.lock().unwrap().clone();
            assert_ne!(
                report["analysisResultsDigest"],
                first["analysisResultsDigest"]
            );
            assert_eq!(
                report["sourceClaims"][0]["key"],
                first["sourceClaims"][0]["key"]
            );
            assert_eq!(
                report["sourceClaims"][0]["revisionHash"], first["sourceClaims"][0]["revisionHash"],
                "canonical revision dedup remains enabled"
            );
            let born = prepare_native_source_coordinator(&p.db, &next.scan_id, 2, &work).unwrap();
            let actor =
                native_source_fresh_finance::original_for_execution(&p.db, &born.run_id).unwrap();
            assert_ne!(actor.root_run_id, p.actor.root_run_id);
            assert_eq!(actor.attempt_number, 2);
            let assessments = run_native_source_assessments(
                &p.root.join("oviraptor.sqlite3"),
                &next.scan_id,
                2,
                &work,
            )
            .unwrap();
            assert_eq!(assessments["independentCandidateReviewCompleted"], true);
            assert_eq!(
                seen.lock().unwrap().len(),
                17,
                "original attempt SDK8 plus new attempt SDK9"
            );
            for (root, count) in [(&p.actor.root_run_id, 8), (&actor.root_run_id, 9)] {
                for (dimension, expected) in [
                    ("model_requests", count),
                    ("model_input_tokens", 10 * count),
                    ("model_output_tokens", 10 * count),
                    ("target_requests", 0),
                    ("browser_actions", 0),
                    ("controlled_writes", 0),
                    ("upload_bytes", 0),
                ] {
                    let cost = budget::balance(&p.db, root, None, dimension).unwrap();
                    assert_eq!(
                        (cost.consumed, cost.reserved, cost.indeterminate),
                        (expected, 0, 0),
                        "{root}/{dimension}"
                    );
                }
                let tx = p.db.unchecked_transaction().unwrap();
                budget::clock::FinalClock::verify_original_exit(&tx, root).unwrap();
                tx.rollback().unwrap();
            }
            let read = |round: i64, index: i64| -> JsonValue {
                let raw:String=p.db.query_row("SELECT t.output_json FROM agent_source_tool_receipts t JOIN agent_runs r ON r.id=t.child_run_id WHERE r.root_run_id=?1 AND r.role='source_analyst' AND t.round_number=?2 AND t.call_index=?3 AND t.state='completed'",params![actor.root_run_id,round,index],|r|r.get(0)).unwrap();
                serde_json::from_str(&raw).unwrap()
            };
            let before = read(1, 0);
            assert_eq!(before["attemptNumber"], 2);
            assert_eq!(
                before["analysisResultsDigest"],
                report["analysisResultsDigest"]
            );
            assert_eq!(read(2, 0)["code"], "result_not_found");
            assert_eq!(read(2, 1), before);
            assert_eq!(read(2, 2)["candidates"].as_array().unwrap().len(), 1);
            assert_eq!(
                p.db.query_row("SELECT count(*) FROM source_analysis_results", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                2
            );
            let current = source_result_retry_original_rows(&p.db);
            for (name, old) in original {
                let rows = &current.iter().find(|(n, _)| n == &name).unwrap().1;
                for row in old {
                    assert!(rows.contains(&row),"new attempt must preserve every original paid/material typed row and rowid:{name}");
                }
            }
            let closed = source_exit_snapshot(&p.db);
            assert!(run_native_source_assessments(
                &p.root.join("oviraptor.sqlite3"),
                &next.scan_id,
                2,
                &work
            )
            .is_err());
            assert_eq!(source_exit_snapshot(&p.db), closed);
            assert_eq!(seen.lock().unwrap().len(), 17);
            let mut merged = report;
            merge_native_source_assessments(&mut merged, assessments);
            assert!(finish_native_branch(
                &p.root.join("oviraptor.sqlite3"),
                &next.scan_id,
                2,
                "source",
                "completed_with_gaps",
                "New Source completed with reviewed gaps",
                &merged
            )
            .unwrap());
            branch.disarm();
            drop(branch);
            let statuses:Vec<(i64,String)>=p.db.prepare("SELECT attempt_number,status FROM sentinel_scan_attempts WHERE scan_id=?1 ORDER BY attempt_number").unwrap()
                .query_map([&p.record.scan_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
            assert_eq!(
                statuses,
                vec![
                    (1, "completed_with_gaps".into()),
                    (2, "completed_with_gaps".into())
                ]
            );
            let done = source_exit_snapshot(&p.db);
            assert!(!finish_native_branch(
                &p.root.join("oviraptor.sqlite3"),
                &p.record.scan_id,
                1,
                "source",
                "failed",
                "late old failure",
                &json!({"error":"late"})
            )
            .unwrap());
            assert_eq!(
                source_exit_snapshot(&p.db),
                done,
                "old callback cannot rewrite the completed retry either"
            );
            assert_eq!(seen.lock().unwrap().len(), 17);
            *second_capture.lock().unwrap() = before;
        })),
    );
    assert!(p.result.is_ok());
    assert_eq!(p.calls, 8);
    assert!(imported.lock().unwrap().is_some());
    assert_eq!(second_output.lock().unwrap()["attemptNumber"], 2);
    assert_eq!(
        p.db.query_row(
            "SELECT attempt_count FROM sentinel_scans WHERE id=?1",
            [&p.record.scan_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    p.cleanup();
}
