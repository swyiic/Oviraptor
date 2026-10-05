fn source_regression_fixture() -> (PathBuf, PathBuf, PathBuf) {
    source_regression_fixture_for_kind("cicd")
}

fn source_regression_fixture_for_kind(kind: &str) -> (PathBuf, PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!("oviraptor-source-regression-{}", Uuid::new_v4()));
    let repo = root.join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("app.py"), "print('fixture')\n").unwrap();
    let db_path = db::initialize(&root.join("app")).unwrap();
    db::open(&db_path).unwrap().execute(
        "INSERT INTO sentinel_scans(id,project_name,status,scan_type,source_path,attempt_count) VALUES('source-regression','fixture','scanning',?1,?2,1)",
        params![kind,repo.to_str().unwrap()],
    ).unwrap();
    let connection = db::open(&db_path).unwrap();
    connection.execute("INSERT INTO sentinel_scan_attempts(scan_id,attempt_number) VALUES('source-regression',1)", []).unwrap();
    if kind=="cicd" {
        store_workbench_ci_policy(&connection,"source-regression",1,&json!({"maxCritical":0,"maxHigh":0,"blockRelease":true})).unwrap();
    }
    store_workbench_source_scope(&connection,"source-regression",1,
        &WorkbenchSourceScope::new(repo.to_str().unwrap(),kind,"full","").unwrap()).unwrap();
    (root, repo, db_path)
}

fn source_regression_outcome(engine: &str, scratch: &Path) -> analyzer::AnalyzerOutcome {
    let sarif_path = scratch.join(format!("{engine}.sarif"));
    fs::write(&sarif_path, json!({
        "version":"2.1.0", "runs":[{"tool":{"driver":{"name":engine}},"results":[]}]
    }).to_string()).unwrap();
    analyzer::AnalyzerOutcome {
        sarif_sha256: crate::artifact_import::canonical::sha256_hex(&fs::read(&sarif_path).unwrap()),
        engine: AnalyzerEngine::parse(engine).unwrap(),
        status: analyzer::AnalyzerStatus::Ran {exit: Some(0)},
        version: format!("{engine}-fixture-version"), rule_pack_digest: format!("{engine}-rules"),
        image_digest: "a".repeat(64), network_disabled: true, repository_read_only: true,
        args: vec![], sarif_path: Some(sarif_path), truncated: false, duration: Duration::ZERO,
        invocation_key: format!("{engine}-fixture-invocation"), reused: false,
    }
}

#[test]
fn source_pipeline_carries_actual_analyzer_versions_into_the_ci_freeze() {
    let (root, repo, db_path) = source_regression_fixture_for_kind("cicd");
    let report = run_native_source_scan_using(
        &db_path, &root.join("app"), "source-regression", 1, &root.join("work"),
        repo.to_str().unwrap(), "cicd", "main",
        |_, engine, _, _, scratch, _| Ok(source_regression_outcome(engine, scratch)),
    ).unwrap();
    let frozen = report["gate"]["freeze"]["analyzers"].as_array().unwrap();
    assert_eq!(frozen.len(), 2, "the real source entry must not pass an empty outcome list: {report}");
    assert!(frozen.iter().all(|row| row["version"].as_str().unwrap().ends_with("-fixture-version")));
    assert!(!report["gate"]["freeze"]["rulePackDigest"].as_str().unwrap().is_empty());
    assert!(report["importedBundles"].as_u64().unwrap() > 0, "engine-named SARIF must actually reach the importer");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_pipeline_rejects_a_changed_snapshot_before_importing_results() {
    let (root, repo, db_path) = source_regression_fixture_for_kind("code");
    let report = run_native_source_scan_using(
        &db_path, &root.join("app"), "source-regression", 1, &root.join("work"),
        repo.to_str().unwrap(), "code", "",
        |_, engine, snapshot, _, scratch, _| {
            fs::write(snapshot.root.join("app.py"), "changed during analysis\n").unwrap();
            Ok(source_regression_outcome(engine, scratch))
        },
    );
    assert!(report.is_err(), "an outcome from a changed source cannot claim the frozen hash");
    assert!(!root.join("app/artifact-import-cas").exists(), "unreproducible evidence must not enter the importer");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_pipeline_exposes_corrupt_artifact_imports_as_coverage_gaps() {
    let (root, repo, db_path) = source_regression_fixture_for_kind("code");
    let report = run_native_source_scan_using(
        &db_path, &root.join("app"), "source-regression", 1, &root.join("work"),
        repo.to_str().unwrap(), "code", "",
        |_, engine, _, _, scratch, _| {
            let mut outcome = source_regression_outcome(engine, scratch);
            fs::write(outcome.sarif_path.as_ref().unwrap(), "{broken SARIF").unwrap();
            outcome.sarif_sha256 = crate::artifact_import::canonical::sha256_hex(b"{broken SARIF");
            Ok(outcome)
        },
    ).unwrap();
    assert!(report["gaps"].as_array().unwrap().iter().any(|gap|
        gap.as_str().unwrap_or_default().starts_with("artifact_import:")), "corruption is not coverage: {report}");
    let plan = NativeSourcePlan::load(&db::open(&db_path).unwrap(), "source-regression", 1).unwrap().unwrap();
    assert!(plan.gaps.iter().any(|gap| gap.starts_with("artifact_import:")));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_completion_cannot_overwrite_pause_cancellation_a_new_attempt_or_a_terminal() {
    let (root, _, db_path) = source_regression_fixture_for_kind("code");
    let connection = db::open(&db_path).unwrap();
    for (status, attempt) in [("pausing", 1), ("paused", 1), ("cancelled", 1),
        ("failed", 1), ("completed", 1), ("scanning", 2)] {
        connection.execute("UPDATE sentinel_scans SET status=?1,attempt_count=?2,current_checkpoint='unchanged' WHERE id='source-regression'", params![status,attempt]).unwrap();
        let changed = finish_native_source_attempt(&db_path, "source-regression", 1, "completed", "late completion").unwrap();
        assert!(!changed, "late source worker overwrote {status}/attempt {attempt}");
        let row: (String,String) = connection.query_row("SELECT status,current_checkpoint FROM sentinel_scans WHERE id='source-regression'", [], |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
        assert_eq!(row, (status.to_string(), "unchanged".to_string()));
    }
    connection.execute("UPDATE sentinel_scans SET status='scanning',attempt_count=1 WHERE id='source-regression'", []).unwrap();
    assert!(finish_native_source_attempt(&db_path, "source-regression", 1, "completed_with_gaps", "valid completion").unwrap());
    assert!(!finish_native_source_attempt(&db_path, "source-regression", 1, "failed", "duplicate completion").unwrap());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_pipeline_does_not_import_leftovers_or_failed_analyzer_outputs() {
    let (root, repo, db_path) = source_regression_fixture_for_kind("code");
    let report = run_native_source_scan_using(
        &db_path, &root.join("app"), "source-regression", 1, &root.join("work"),
        repo.to_str().unwrap(), "code", "",
        |_, engine, _, _, scratch, _| {
            let mut outcome = source_regression_outcome(engine, scratch);
            outcome.status = analyzer::AnalyzerStatus::Failed { exit: Some(2), detail: "fixture failure".into() };
            fs::write(scratch.join("unrelated.sarif"), "{broken old artifact").unwrap();
            Ok(outcome)
        },
    ).unwrap();
    assert_eq!(report["importedBundles"], 0, "only an accepted outcome can enter this attempt's import roots");
    assert_eq!(report["failedImportBundles"], 0, "stale files must not even be offered to the importer");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_worker_observes_attempt_replacement_before_import_or_ci_gate_write() {
    let (root, repo, db_path) = source_regression_fixture_for_kind("cicd");
    let mut calls = 0;
    let outcome = run_native_source_scan_using(
        &db_path, &root.join("app"), "source-regression", 1, &root.join("work"),
        repo.to_str().unwrap(), "cicd", "main",
        |connection, engine, _, _, scratch, cancelled| {
            calls += 1;
            assert!(!cancelled());
            connection.execute("UPDATE sentinel_scans SET attempt_count=2 WHERE id='source-regression'", []).unwrap();
            assert!(cancelled(), "a replaced attempt must stop even if the scan itself is still scanning");
            Ok(source_regression_outcome(engine, scratch))
        },
    );
    assert!(outcome.is_err());
    assert_eq!(calls, 1, "a fenced worker must not launch another analyzer");
    assert!(!root.join("app/artifact-import-cas").exists());
    assert!(NativeSourcePlan::load(&db::open(&db_path).unwrap(), "source-regression", 1).unwrap().is_none());
    fs::remove_dir_all(root).unwrap();
}
