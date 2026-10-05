fn retired_coverage_fixture() -> serde_json::Value {
    serde_json::json!({"schema_version":1,
        "entries":[{"surface":"retired-tested","outcome":"tested"}],
        "gaps":[{"surface":"retired-gap","detail":"not checked"}]})
}

#[test]
fn coverage_retirement_direct_dispatch_rejects_all_old_schema_variants() {
    for schema in [serde_json::json!(1), serde_json::json!("1"), serde_json::json!(9)] {
        let mut value = retired_coverage_fixture();
        value["schema_version"] = schema;
        let (rows, notes) = parse_audit_fixture("coverage.json", &serde_json::to_vec(&value).unwrap(), &Limits::default());
        assert!(rows.is_empty(), "old coverage schema was accepted");
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].code, "unrecognized_artifact");
    }
    let (rows, notes) = parse_audit_fixture("coverage.json", b"{truncated", &Limits::default());
    assert!(rows.is_empty());
    assert_eq!(notes[0].code, "unrecognized_artifact", "must not invoke retired JSON parsing");
}

#[test]
fn coverage_retirement_old_only_directory_cannot_create_import_rows() {
    let root = sandbox("retired-coverage-only");
    let source = source_dir(&root);
    write_json(&source, "coverage.json", &retired_coverage_fixture());
    let before = fingerprint_tree(&source);
    let (bundles, notes) = discovery::discover(&[&source], 8);
    assert!(bundles.is_empty());
    assert!(notes.is_empty());
    let connection = open_connection(&initialize_db(&root));
    assert!(import_at(&connection, &root).outcomes.is_empty());
    for table in ["import_bundles", "import_bundle_files", "import_record_revisions", "import_projection_memberships", "artifact_objects"] {
        assert_eq!(table_count(&connection, table), 0, "retired coverage created {table}");
    }
    assert_eq!(fingerprint_tree(&source), before);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn coverage_retirement_mixed_bundle_preserves_standard_semantics_not_old_records() {
    let root = sandbox("retired-coverage-mixed");
    let source = source_dir(&root);
    for name in ["coverage.json", "nested/coverage.json"] {
        write_json(&source, name, &retired_coverage_fixture());
    }
    write_json(&source, "model-prompt-audit.json", &serde_json::json!({"instruction":"current-audit"}));
    let mut pass = sarif_result("CURRENT-PASS", "warning", "standard coverage", &[("one.rs", 1)], None);
    pass["kind"] = serde_json::json!("pass");
    write_json(&source, "findings.sarif", &sarif_document(vec![pass,
        sarif_result("CURRENT-FINDING", "warning", "standard candidate", &[("two.rs", 2)], None)]));
    let before = fingerprint_tree(&source);
    let (bundles, notes) = discovery::discover(&[&source], 8);
    assert!(notes.is_empty());
    assert_eq!(bundles.len(), 1);
    assert_eq!(bundles[0].files.len(), 2);
    let connection = open_connection(&initialize_db(&root));
    let summary = import_at(&connection, &root);
    assert_eq!(summary.outcomes.len(), 1);
    assert!(!has_code(&summary, "unrecognized_artifact"), "retired files must not reach parsing");
    let coverage = current_envelopes(&connection, RecordKind::Coverage);
    assert_eq!(coverage.len(), 1);
    assert_eq!(scalar_text(&coverage[0], "rule_id"), "CURRENT-PASS");
    let findings = current_envelopes(&connection, RecordKind::FindingCandidate);
    assert_eq!(findings.len(), 1);
    assert_eq!(scalar_text(&findings[0], "rule_id"), "CURRENT-FINDING");
    assert_eq!(current_envelopes(&connection, RecordKind::EventTrace).len(), 1);
    assert_eq!(table_count(&connection, "import_bundle_files"), 2);
    assert_eq!(table_count(&connection, "artifact_objects"), 2);
    for row in coverage.iter().chain(&findings) {
        assert_eq!(row.pointer("/claim/executionEligible"), Some(&serde_json::json!(false)));
    }
    for table in ["sentinel_findings", "agent_runs"] { assert_eq!(table_count(&connection, table), 0); }
    assert!(has_code(&import_at(&connection, &root), "bundle_unchanged"));
    assert_eq!(fingerprint_tree(&source), before);
    fs::remove_dir_all(root).unwrap();
}
