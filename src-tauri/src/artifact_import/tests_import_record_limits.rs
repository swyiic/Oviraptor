// Current-format resource limits remain enforced after retiring text adapters.
fn write_limited_sarif(bundle: &Path, count: usize, title: &str) {
    let rows = (0..count)
        .map(|index| {
            sarif_result(
                &format!("CURRENT-{index}"),
                "warning",
                title,
                &[("src/current.rs", index as i64 + 1)],
                None,
            )
        })
        .collect();
    write_json(bundle, "findings.sarif", &sarif_document(rows));
}

#[test]
fn current_record_limit_refuses_whole_bundle_without_objects_or_partial_writes() {
    for (count, limit) in [(4, 3), (1, 0)] {
        let root = sandbox("current-record-limit");
        let source = source_dir(&root);
        write_limited_sarif(&source, count, "current source");
        let before = fingerprint_tree(&source);
        let connection = open_connection(&initialize_db(&root));
        let limits = Limits {
            records: limit,
            ..Limits::default()
        };
        let summary = import_dir(&connection, &root, &limits);
        assert_eq!(summary.outcomes.len(), 1);
        assert_eq!(summary.outcomes[0].status, BundleStatus::Failed);
        assert!(has_code(&summary, "record_count_limit"));
        assert!(summary.outcomes[0].committed_records.is_none());
        for table in [
            "import_bundles",
            "import_bundle_files",
            "import_record_revisions",
            "artifact_objects",
            "sentinel_scans",
            "sentinel_findings",
            "agent_runs",
        ] {
            assert_eq!(table_count(&connection, table), 0, "{table}");
        }
        assert!(fingerprint_tree(&root.join("cas")).is_empty());
        assert_eq!(fingerprint_tree(&source), before);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn current_record_limit_is_checked_before_duplicate_reconciliation() {
    let root = sandbox("current-record-duplicates");
    let source = source_dir(&root);
    let row = sarif_result(
        "SAME",
        "warning",
        "same finding",
        &[("src/current.rs", 1)],
        None,
    );
    write_json(&source, "findings.sarif", &sarif_document(vec![row; 4]));
    let connection = open_connection(&initialize_db(&root));
    let limits = Limits {
        records: 3,
        ..Limits::default()
    };
    let summary = import_dir(&connection, &root, &limits);
    assert_eq!(summary.outcomes[0].status, BundleStatus::Failed);
    assert!(has_code(&summary, "record_count_limit"));
    assert_eq!(table_count(&connection, "artifact_objects"), 0);
    assert_eq!(table_count(&connection, "import_record_revisions"), 0);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn current_record_limit_failure_preserves_projection_and_valid_retry_is_idempotent() {
    let root = sandbox("current-record-retry");
    let source = source_dir(&root);
    write_limited_sarif(&source, 1, "original");
    let connection = open_connection(&initialize_db(&root));
    let limits = Limits {
        records: 3,
        ..Limits::default()
    };
    assert_eq!(
        import_dir(&connection, &root, &limits).outcomes[0].status,
        BundleStatus::Imported
    );
    let original = current_envelopes(&connection, RecordKind::FindingCandidate);
    assert_eq!(original.len(), 1);
    let revisions = table_count(&connection, "import_record_revisions");
    let objects = table_count(&connection, "artifact_objects");
    write_limited_sarif(&source, 4, "replacement");
    let rejected = import_dir(&connection, &root, &limits);
    assert_eq!(rejected.outcomes[0].status, BundleStatus::Failed);
    assert!(has_code(&rejected, "record_count_limit"));
    assert_eq!(
        current_envelopes(&connection, RecordKind::FindingCandidate),
        original
    );
    assert_eq!(
        table_count(&connection, "import_record_revisions"),
        revisions
    );
    assert_eq!(table_count(&connection, "artifact_objects"), objects);
    write_limited_sarif(&source, 1, "replacement");
    let before = fingerprint_tree(&source);
    assert_eq!(
        import_dir(&connection, &root, &limits).outcomes[0].status,
        BundleStatus::Imported
    );
    let current = current_envelopes(&connection, RecordKind::FindingCandidate);
    assert_eq!(current.len(), 1);
    assert_eq!(current[0]["payload"]["title"], "replacement");
    assert_eq!(current[0]["claim"]["authority"], "historical_external");
    assert_eq!(current[0]["claim"]["executionEligible"], false);
    assert_eq!(current[0]["claim"]["readOnly"], true);
    let revisions = table_count(&connection, "import_record_revisions");
    assert_eq!(
        import_dir(&connection, &root, &limits).outcomes[0].status,
        BundleStatus::Unchanged
    );
    assert_eq!(
        table_count(&connection, "import_record_revisions"),
        revisions
    );
    assert_eq!(fingerprint_tree(&source), before);
    assert_eq!(table_count(&connection, "sentinel_scans"), 0);
    assert_eq!(table_count(&connection, "agent_runs"), 0);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn current_record_limit_failure_does_not_block_an_independent_healthy_bundle() {
    let root = sandbox("current-record-neighbor");
    let source = source_dir(&root);
    write_limited_sarif(&source.join("bad"), 4, "over limit");
    write_limited_sarif(&source.join("good"), 1, "neighbor");
    let before = fingerprint_tree(&source);
    let connection = open_connection(&initialize_db(&root));
    let summary = import_dir(
        &connection,
        &root,
        &Limits {
            records: 3,
            ..Limits::default()
        },
    );
    assert_eq!(summary.outcomes.len(), 2);
    assert_eq!(
        summary
            .outcomes
            .iter()
            .filter(|o| o.status == BundleStatus::Failed)
            .count(),
        1
    );
    assert_eq!(
        summary
            .outcomes
            .iter()
            .filter(|o| o.status == BundleStatus::Imported)
            .count(),
        1
    );
    let current = current_envelopes(&connection, RecordKind::FindingCandidate);
    assert_eq!(current.len(), 1);
    assert_eq!(current[0]["payload"]["title"], "neighbor");
    assert_eq!(table_count(&connection, "import_bundles"), 1);
    assert_eq!(table_count(&connection, "sentinel_scans"), 0);
    assert_eq!(table_count(&connection, "agent_runs"), 0);
    assert_eq!(fingerprint_tree(&source), before);
    fs::remove_dir_all(root).unwrap();
}
