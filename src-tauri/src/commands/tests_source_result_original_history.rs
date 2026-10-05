#[test]
fn source_result_receipt_late_history_cannot_replace_actual_attempt_results() {
    let claims = Arc::new(std::sync::Mutex::new(Vec::<JsonValue>::new()));
    let original_claims = claims.clone();
    let imported = Arc::new(std::sync::Mutex::new(None::<String>));
    let late = imported.clone();
    let p = source_broker_original_probe_scripted(
        "diff",
        true,
        move |root, db, record, report| {
            assert_eq!(report["sourceClaims"].as_array().unwrap().len(), 1);
            *original_claims.lock().unwrap() = report["sourceClaims"].as_array().unwrap().clone();
            let actual = report["sourceClaims"][0]["key"]
                .as_str()
                .unwrap()
                .to_owned();
            let frozen = crate::native_pipeline::results::AnalysisResults::load(
                db,
                &source_result_view(db, record),
            )
            .unwrap();
            let original_receipt = serde_json::to_string(&frozen).unwrap();
            let original_digest = frozen.digest();
            let root = root.to_owned();
            let scan_id = record.scan_id.clone();
            let record = record.clone();
            SourceBrokerOriginalScript {
                rounds: vec![vec![("analyzer.list_results", json!({}))]],
                before_analyst_response: Some(Box::new(move |number| {
                    if number != 1 {
                        return None;
                    }
                    let db = db::open(&root.join("oviraptor.sqlite3")).unwrap();
                    let first:String=db.query_row("SELECT t.output_json FROM agent_source_tool_receipts t JOIN agent_runs r ON r.id=t.child_run_id WHERE r.scan_id=?1 AND r.attempt_number=1 AND r.role='source_analyst' AND t.round_number=1 AND t.call_index=0 AND t.state='completed'",[&scan_id],|r|r.get(0)).unwrap();
                    let before: JsonValue = serde_json::from_str(&first).unwrap();
                    assert_eq!(before["candidates"].as_array().unwrap().len(), 1);
                    assert_eq!(before["candidates"][0]["key"], actual);
                    let historical = source_result_import_history(&root, &db, &scan_id);
                    assert_ne!(historical, actual);
                    *late.lock().unwrap() = Some(historical.clone());
                    let after = crate::native_pipeline::results::AnalysisResults::load(
                        &db,
                        &source_result_view(&db, &record),
                    )
                    .unwrap();
                    assert_eq!(serde_json::to_string(&after).unwrap(), original_receipt);
                    assert_eq!(after.digest(), original_digest);
                    Some(vec![
                        ("analyzer.list_results", json!({})),
                        ("analyzer.get_result", json!({"key":historical})),
                        ("analyzer.get_result", json!({"key":actual})),
                        ("analyzer.list_results", json!({"engine":"semgrep"})),
                        ("analyzer.list_results", json!({"engine":"historical"})),
                    ])
                })),
            }
        },
        None,
        |_, _| {},
        |engine, scratch| source_result_outcome(engine, scratch, "app.py", "actual-analysis"),
    );
    assert!(p.result.is_ok(), "{:?}", p.result);
    assert_eq!(p.calls, 9);
    assert!(
        imported.lock().unwrap().is_some(),
        "late importer actually executed between Source tool rounds"
    );
    let before = p.output(1, 0);
    let after = p.output(2, 0);
    assert_eq!(
        after["candidates"], before["candidates"],
        "late history must not replace original accepted results"
    );
    assert_eq!(
        after["analysisResultsDigest"],
        before["analysisResultsDigest"]
    );
    assert_eq!(p.output(2, 1)["code"], "result_not_found");
    let actual = p.output(2, 2);
    assert!(actual.get("error").is_none(), "{actual}");
    assert_eq!(actual["key"], before["candidates"][0]["key"]);
    assert_eq!(
        actual["revisionHash"],
        before["candidates"][0]["revisionHash"]
    );
    assert_eq!(
        actual["analysisResultsDigest"],
        before["analysisResultsDigest"]
    );
    let view = source_result_view(&p.db, &p.record);
    assert_eq!(
        source_claims_for_analysis(&p.db, &view).unwrap(),
        *claims.lock().unwrap()
    );
    assert_eq!(
        p.output(2, 3)["candidates"].as_array().unwrap().len(),
        1,
        "actual engine filter"
    );
    assert_eq!(p.output(2, 4)["candidates"], json!([]));
    assert_eq!(
        p.result.as_ref().unwrap()["independentCandidateReviewCompleted"],
        true
    );
    p.cleanup();
}
