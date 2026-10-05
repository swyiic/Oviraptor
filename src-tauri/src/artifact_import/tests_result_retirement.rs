// Retired JSON envelopes and text formats must stay outside current imports.
fn retired_result_files() -> Vec<(&'static str, &'static str)> {
    vec![
        ("vulnerabilities.json", r#"[{"id":"old","title":"retired","severity":"high"}]"#),
        ("array/vulnerabilities.json", r#"[{"id":"old","title":"retired"}]"#),
        ("findings/vulnerabilities.json", r#"{"findings":[{"id":"old","title":"retired"}]}"#),
        ("results/vulnerabilities.json", r#"{"results":[{"id":"old","title":"retired"}]}"#),
        ("items/vulnerabilities.json", r#"{"items":[{"id":"old","title":"retired"}]}"#),
        ("envelope/vulnerabilities.json", r#"{"vulnerabilities":[{"id":"old","title":"retired"}]}"#),
        ("broken/vulnerabilities.json", r#"{"vulnerabilities":["#),
        (
            "vulnerabilities.csv",
            "\u{feff}id,title,severity,endpoint\r\nc1,旧结果,high,/old\r\n",
        ),
        (
            "nested/vulnerabilities.csv",
            "title,endpoint\n\"quoted\nbody\",/old\n",
        ),
        (
            "vulnerabilities/old.md",
            "# 旧报告\n严重程度：high\n<script>inert</script>\n",
        ),
        ("nested/vulnerabilities/old.MD", "# 旧报告\nCVSS：9.8\n"),
    ]
}

#[test]
fn result_retirement_direct_dispatch_rejects_old_formats_even_when_well_formed() {
    for (path, body) in retired_result_files() {
        for limit in [0, 20] {
            let limits = Limits {
                records: limit,
                ..Limits::default()
            };
            let (records, notes) = parse_audit_fixture(path, body.as_bytes(), &limits);
            assert!(
                records.is_empty(),
                "old format still produces records: {path}"
            );
            assert_eq!(notes.len(), 1, "{path}: {notes:?}");
            assert_eq!(notes[0].code, "unrecognized_artifact");
        }
    }
}

#[test]
fn result_retirement_old_only_tree_cannot_create_a_bundle_or_store_originals() {
    let root = sandbox("text-result-retired-only");
    let source = source_dir(&root);
    for (path, body) in retired_result_files() {
        write_file(&source, path, body);
    }
    let before = fingerprint_tree(&source);
    let (bundles, notes) = discovery::discover(&[&source], 8);
    assert!(
        bundles.is_empty(),
        "retired result must not discover a bundle: {bundles:?}"
    );
    assert!(notes.is_empty());
    let connection = open_connection(&initialize_db(&root));
    let result = import_at(&connection, &root);
    assert!(result.outcomes.is_empty());
    for table in [
        "import_bundles",
        "import_bundle_files",
        "artifact_objects",
        "import_record_revisions",
        "import_projection_memberships",
        "sentinel_findings",
        "agent_runs",
    ] {
        assert_eq!(table_count(&connection, table), 0, "{table}");
    }
    assert!(fingerprint_tree(&root.join("cas")).is_empty());
    assert_eq!(fingerprint_tree(&source), before);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn result_retirement_mixed_bundle_preserves_current_sources_and_ignores_old_changes() {
    let root = sandbox("text-result-retired-mixed");
    let source = source_dir(&root);
    for (path, body) in retired_result_files() {
        write_file(&source, path, body);
    }
    write_json(
        &source,
        "model-prompt-audit.json",
        &serde_json::json!({"instruction":"current JSON"}),
    );
    write_json(
        &source,
        "findings.sarif",
        &sarif_document(vec![sarif_result(
            "CURRENT",
            "warning",
            "current report",
            &[("src/current.rs", 2)],
            None,
        )]),
    );
    let before = fingerprint_tree(&source);
    let (bundles, notes) = discovery::discover(&[&source], 8);
    assert!(notes.is_empty());
    assert_eq!(bundles.len(), 1);
    assert_eq!(
        bundles[0].files.len(),
        2,
        "old result must not enter a mixed manifest"
    );
    let connection = open_connection(&initialize_db(&root));
    let first = import_at(&connection, &root);
    assert_eq!(first.outcomes[0].status, BundleStatus::Imported);
    assert_eq!(table_count(&connection, "import_bundle_files"), 2);
    assert_eq!(table_count(&connection, "artifact_objects"), 2);
    let findings = current_envelopes(&connection, RecordKind::FindingCandidate);
    assert_eq!(findings.len(), 1);
    assert_eq!(scalar_text(&findings[0], "rule_id"), "CURRENT");
    assert_eq!(
        current_envelopes(&connection, RecordKind::EventTrace).len(),
        1
    );
    assert_eq!(table_count(&connection, "agent_runs"), 0);
    assert_eq!(fingerprint_tree(&source), before);
    let revisions = table_count(&connection, "import_record_revisions");
    for (path, _) in retired_result_files() {
        write_file(&source, path, "changed retired bytes");
    }
    let changed = fingerprint_tree(&source);
    let second = import_at(&connection, &root);
    assert_eq!(second.outcomes[0].status, BundleStatus::Unchanged);
    assert_eq!(
        table_count(&connection, "import_record_revisions"),
        revisions
    );
    assert_eq!(table_count(&connection, "artifact_objects"), 2);
    assert_eq!(fingerprint_tree(&source), changed);
    fs::remove_dir_all(root).unwrap();
}
