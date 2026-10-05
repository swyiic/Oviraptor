#[test]
fn source_result_same_bytes_keep_both_engines_without_duplicate_candidates() {
    let mut analyzers = Vec::new();
    let p = source_broker_original_probe_using(
        "full",
        true,
        |_, db, record, report| {
            let receipt = crate::native_pipeline::results::AnalysisResults::load(
                db,
                &source_result_view(db, record),
            )
            .unwrap();
            let candidates: Vec<_> = receipt
                .records
                .iter()
                .filter(|r| r.record.record_kind == "finding_candidate")
                .collect();
            assert_eq!(candidates.len(), 2);
            assert_eq!(
                candidates
                    .iter()
                    .map(|r| r.engine.as_str())
                    .collect::<std::collections::BTreeSet<_>>(),
                ["codeql", "semgrep"].into_iter().collect()
            );
            assert_eq!(
                candidates[0].record, candidates[1].record,
                "same canonical revision reused"
            );
            assert_eq!(report["sourceClaims"].as_array().unwrap().len(), 1);
            assert_eq!(
                report["sourceClaims"][0]["acceptedSources"]
                    .as_array()
                    .unwrap()
                    .len(),
                2
            );
            let key = report["sourceClaims"][0]["key"].clone();
            vec![vec![
                ("analyzer.list_results", json!({"engine":"","limit":1})),
                (
                    "analyzer.list_results",
                    json!({"engine":"semgrep","limit":1}),
                ),
                (
                    "analyzer.list_results",
                    json!({"engine":"codeql","limit":1}),
                ),
                ("analyzer.get_result", json!({"key":key})),
            ]]
        },
        None,
        |_, _| {},
        |engine, scratch| {
            analyzers.push(engine.to_owned());
            let mut outcome = source_regression_outcome(engine, scratch);
            let bytes = source_result_sarif("shared-driver", "app.py", "shared-candidate");
            fs::write(outcome.sarif_path.as_ref().unwrap(), &bytes).unwrap();
            outcome.sarif_sha256 = crate::artifact_import::canonical::sha256_hex(bytes.as_bytes());
            outcome
        },
    );
    assert_eq!(analyzers, ["semgrep", "codeql"]);
    assert!(p.result.is_ok(), "{:?}", p.result);
    assert_eq!(p.calls, 8);
    let result = p.output(1, 3);
    assert!(result.get("error").is_none(), "{result}");
    assert_eq!(result["acceptedSources"].as_array().unwrap().len(), 2);
    for (i, engine) in ["", "semgrep", "codeql"].into_iter().enumerate() {
        let list = p.output(1, i as i64);
        assert_eq!(list["candidates"].as_array().unwrap().len(), 1, "{list}");
        assert_eq!(list["truncated"], false);
        assert_eq!(
            list["candidates"][0]["acceptedSources"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        if !engine.is_empty() {
            assert_eq!(list["candidates"][0]["engine"], engine);
        }
        assert_eq!(
            result["revisionHash"],
            list["candidates"][0]["revisionHash"]
        );
        assert_eq!(result["key"], list["candidates"][0]["key"]);
    }
    assert_eq!(
        p.result.as_ref().unwrap()["independentCandidateReviewCompleted"],
        true
    );
    p.cleanup();
}

#[test]
fn source_result_distinct_reports_keep_every_merged_candidate_origin() {
    let p = source_broker_original_probe_using(
        "full",
        true,
        |_, db, record, report| {
            let receipt = crate::native_pipeline::results::AnalysisResults::load(
                db,
                &source_result_view(db, record),
            )
            .unwrap();
            assert_eq!(receipt.artifacts.len(), 2);
            assert_ne!(receipt.artifacts[0].sha256, receipt.artifacts[1].sha256);
            let candidates = receipt.candidates(db).unwrap();
            assert_eq!(candidates.len(), 1);
            assert_eq!(candidates[0].sources.len(), 2);
            assert_eq!(
                candidates[0]
                    .sources
                    .iter()
                    .map(|s| s.record.source_artifact_id.as_str())
                    .collect::<std::collections::BTreeSet<_>>(),
                receipt
                    .artifacts
                    .iter()
                    .map(|a| a.sha256.as_str())
                    .collect()
            );
            assert_eq!(
                candidates[0].sources[0].record.revision_id,
                candidates[0].sources[1].record.revision_id
            );
            for source in &candidates[0].sources {
                let envelope = source.record.read(db).unwrap();
                assert!(envelope.get("contributingArtifacts").is_none());
                assert_eq!(envelope, candidates[0].envelope);
            }
            assert_eq!(report["sourceClaims"].as_array().unwrap().len(), 1);
            assert_eq!(
                report["sourceClaims"][0]["acceptedSources"]
                    .as_array()
                    .unwrap()
                    .len(),
                2
            );
            vec![vec![
                ("analyzer.list_results", json!({"engine":"semgrep"})),
                ("analyzer.list_results", json!({"engine":"codeql"})),
            ]]
        },
        None,
        |_, _| {},
        |engine, scratch| source_result_outcome(engine, scratch, "app.py", "shared-rule"),
    );
    assert!(p.result.is_ok(), "{:?}", p.result);
    assert_eq!(p.calls, 8);
    for (i, engine) in ["semgrep", "codeql"].into_iter().enumerate() {
        let list = p.output(1, i as i64);
        assert_eq!(list["candidates"].as_array().unwrap().len(), 1, "{list}");
        assert_eq!(list["candidates"][0]["engine"], engine);
        assert_eq!(
            list["candidates"][0]["acceptedSources"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }
    assert_eq!(
        p.result.as_ref().unwrap()["independentCandidateReviewCompleted"],
        true
    );
    p.cleanup();
}

#[test]
fn source_result_receipt_out_of_view_locations_are_gaps_not_active_claims() {
    let p = source_broker_original_probe_using(
        "diff",
        true,
        |_, db, _, report| {
            assert_eq!(report["sourceClaims"], json!([]));
            assert!(report["gaps"].as_array().unwrap().iter().any(|g| g
                .as_str()
                .unwrap()
                .starts_with("analysis_result_location_outside_view:")));
            assert!(db.query_row("SELECT count(*) FROM import_record_revisions WHERE record_kind='finding_candidate'",[],|r|r.get::<_,i64>(0)).unwrap()>0);
            vec![vec![("analyzer.list_results", json!({}))]]
        },
        None,
        |_, _| {},
        |engine, scratch| source_result_outcome(engine, scratch, "untouched.py", "outside-view"),
    );
    assert!(p.result.is_ok(), "{:?}", p.result);
    assert_eq!(p.calls, 7);
    assert_eq!(p.output(1, 0)["candidates"], json!([]));
    p.cleanup();
}

#[test]
fn source_result_receipt_keeps_failed_analyzers_visible() {
    let p = source_broker_original_probe_using(
        "diff",
        true,
        |_, _, _, _| vec![vec![("analyzer.list_results", json!({}))]],
        None,
        |_, _| {},
        |engine, scratch| {
            let mut outcome = source_regression_outcome(engine, scratch);
            outcome.status = analyzer::AnalyzerStatus::Failed {
                exit: Some(2),
                detail: "fixture".into(),
            };
            outcome
        },
    );
    assert!(p.result.is_ok(), "{:?}", p.result);
    assert_eq!(p.calls, 7);
    let result = p.output(1, 0);
    assert_eq!(result["candidates"], json!([]));
    assert_eq!(result["runs"][0]["status"], "failed", "{result}");
    assert!(result["gaps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|g| g.as_str().unwrap().contains("semgrep")));
    p.cleanup();
}
