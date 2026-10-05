#[test]
fn run_retirement_rejects_old_statuses_usage_and_corrupt_json() {
    for status in ["succeeded-with-warnings", "completed", "done", "success", "interrupted",
        "stopped", "cancelled", "aborted_with_results", "failed", "in_progress"] {
        let bytes = serde_json::to_vec(&serde_json::json!({"run_id":"old", "status":status,
            "cli_version":"retired-version", "target":"https://retired.invalid",
            "llm_usage":{"requests":"18","total_tokens":6200}})).unwrap();
        let (rows, notes) = parse_audit_fixture("run.json", &bytes, &Limits::default());
        assert!(rows.is_empty(), "retired run must not create state or usage: {status}");
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].code, "unrecognized_artifact");
    }
    let (rows, notes) = parse_audit_fixture("run.json", b"{broken", &Limits::default());
    assert!(rows.is_empty());
    assert_eq!(notes[0].code, "unrecognized_artifact");
}

#[test]
fn run_retirement_old_only_directory_is_not_imported_or_modified() {
    let root = sandbox("retired-run-only");
    let source = source_dir(&root);
    write_json(&source, "run.json", &serde_json::json!({"run_id":"old","status":"success"}));
    let before = fingerprint_tree(&source);
    let (bundles, notes) = discovery::discover(&[&source], 8);
    assert!(bundles.is_empty());
    assert!(notes.is_empty());
    let db = open_connection(&initialize_db(&root));
    assert!(import_at(&db, &root).outcomes.is_empty());
    for table in ["import_bundles", "import_bundle_files", "import_record_revisions",
        "import_projection_memberships", "artifact_objects", "agent_runs"] {
        assert_eq!(table_count(&db, table), 0, "retired run populated {table}");
    }
    assert_eq!(fingerprint_tree(&source), before);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn run_retirement_mixed_bundle_ignores_old_metadata_and_preserves_current_sources() {
    let root = sandbox("retired-run-mixed");
    let source = source_dir(&root);
    for path in ["run.json", "nested/run.json"] {
        write_json(&source, path, &serde_json::json!({"run_id":"old", "cli_version":"retired-version",
            "target":"https://retired.invalid", "llm_usage":{"total_tokens":99999}}));
    }
    write_json(&source, "model-prompt-audit.json", &serde_json::json!({"instruction":"current audit"}));
    let mut sarif = sarif_document(vec![sarif_result("CURRENT", "warning", "current candidate", &[], None)]);
    sarif["runs"][0]["properties"] = serde_json::json!({"reportLabel":"current report"});
    write_json(&source, "findings.sarif", &sarif);
    let before = fingerprint_tree(&source);
    let (bundles, _) = discovery::discover(&[&source], 8);
    assert_eq!(bundles.len(), 1);
    assert_eq!(bundles[0].files.len(), 2);
    // Even a direct caller supplying retired bytes cannot choose projection
    // identity from them. Discovery is not the only retirement boundary.
    let retired_payloads = vec![("run.json".to_string(), br#"{"run_id":"retired-identity"}"#.to_vec())];
    assert_eq!(scope::resolve(&source, &bundles[0], &retired_payloads).scan_id, "run-root");
    let db = open_connection(&initialize_db(&root));
    let report = import_at(&db, &root);
    assert_eq!(report.outcomes.len(), 1);
    assert_eq!(report.outcomes[0].status, BundleStatus::Imported);
    assert_eq!(current_envelopes(&db, RecordKind::RunState).len(), 1);
    assert_eq!(current_envelopes(&db, RecordKind::FindingCandidate).len(), 1);
    assert_eq!(current_envelopes(&db, RecordKind::EventTrace).len(), 1);
    assert!(current_envelopes(&db, RecordKind::Usage).is_empty());
    for kind in [RecordKind::RunState, RecordKind::FindingCandidate, RecordKind::EventTrace] {
        for row in current_envelopes(&db, kind) {
            let json = serde_json::to_string(&row).unwrap();
            assert!(!json.contains("retired-version") && !json.contains("retired.invalid"));
            assert_eq!(row.pointer("/claim/executionEligible"), Some(&serde_json::json!(false)));
        }
    }
    assert_eq!(table_count(&db, "import_bundle_files"), 2);
    assert_eq!(table_count(&db, "artifact_objects"), 2);
    assert_eq!(table_count(&db, "agent_runs"), 0);
    assert!(has_code(&import_at(&db, &root), "bundle_unchanged"));
    assert_eq!(fingerprint_tree(&source), before);
    fs::remove_dir_all(root).unwrap();
}
