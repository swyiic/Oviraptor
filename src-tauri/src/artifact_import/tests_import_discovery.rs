// Directory completeness is a precondition for replacing a historical projection.

#[cfg(unix)]
#[test]
fn discovery_symlink_roots_are_rejected_without_blocking_healthy_roots() {
    let root = sandbox("discovery-root-links");
    let healthy = root.join("healthy");
    let outside = root.join("outside");
    write_json(&healthy, "model-prompt-audit.json", &serde_json::json!({"instruction":"healthy"}));
    write_json(&outside, "model-prompt-audit.json", &serde_json::json!({"instruction":"outside"}));
    let linked = root.join("linked");
    let linked_file = root.join("linked-file");
    let dangling = root.join("dangling");
    std::os::unix::fs::symlink(&outside, &linked).unwrap();
    std::os::unix::fs::symlink(outside.join("model-prompt-audit.json"), &linked_file).unwrap();
    std::os::unix::fs::symlink(root.join("absent"), &dangling).unwrap();
    let before = fingerprint_tree(&outside);
    let results: Vec<_> = [linked.clone(), linked.join(""), linked.join("."), linked_file, dangling]
        .into_iter()
        .map(|path| {
            let result = discovery::discover(&[&path, &healthy], 8);
            (path, result)
        })
        .collect();
    let after = fingerprint_tree(&outside);
    fs::remove_dir_all(&root).unwrap();
    assert_eq!(before, after);
    for (path, (bundles, notes)) in results {
        assert_eq!(bundles.len(), 1, "must not follow {}", path.display());
        assert_eq!(bundles[0].source_dir, healthy);
        assert_eq!(notes.len(), 1, "one rejection for {}", path.display());
        assert_eq!(notes[0].code, "symlink_rejected");
        assert_eq!(notes[0].severity, Severity::Error);
        assert_eq!(notes[0].source_path, path.display().to_string());
    }
}

#[test]
fn discovery_non_directory_roots_report_errors_but_absent_defaults_remain_optional() {
    let root = sandbox("discovery-root-types");
    write_json(&root, "model-prompt-audit.json", &serde_json::json!({}));
    let file = root.join("model-prompt-audit.json");
    let missing = root.join("absent");
    let (bundles, notes) = discovery::discover(&[&file, &missing], 8);
    fs::remove_dir_all(&root).unwrap();
    assert!(bundles.is_empty());
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].code, "root_not_directory");
    assert_eq!(notes[0].source_path, file.display().to_string());
}

#[cfg(unix)]
#[test]
fn discovery_fifo_root_is_rejected_without_opening_it() {
    use std::os::unix::ffi::OsStrExt;
    let root = sandbox("discovery-special-roots");
    fs::create_dir_all(&root).unwrap();
    let fifo = root.join("pipe");
    let raw = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(raw.as_ptr(), 0o600) }, 0);
    let (bundles, notes) = discovery::discover(&[&fifo], 8);
    fs::remove_dir_all(&root).unwrap();
    assert!(bundles.is_empty());
    assert_eq!(notes.len(), 1);
    assert!(notes.iter().all(|note| note.code == "special_file_rejected"));
}

#[cfg(unix)]
#[test]
fn discovery_replaced_root_preserves_history_and_retries_after_link_removal() {
    let root = sandbox("discovery-root-replacement");
    single_finding_bundle(&root, "high");
    let source = source_dir(&root);
    let connection = open_connection(&initialize_db(&root));
    assert_eq!(import_at(&connection, &root).outcomes[0].status, BundleStatus::Imported);
    let previous = current_envelopes(&connection, RecordKind::FindingCandidate);
    let revisions = table_count(&connection, "import_record_revisions");
    let objects = table_count(&connection, "artifact_objects");
    let preserved = root.join("preserved");
    let outside = root.join("outside");
    fs::rename(&source, &preserved).unwrap();
    write_json(&outside, "model-prompt-audit.json", &serde_json::json!({"instruction":"unselected"}));
    let before = (fingerprint_tree(&preserved), fingerprint_tree(&outside));
    std::os::unix::fs::symlink(&outside, &source).unwrap();
    let rejected = import_at(&connection, &root);
    let after = (fingerprint_tree(&preserved), fingerprint_tree(&outside));
    let remaining = current_envelopes(&connection, RecordKind::FindingCandidate);
    let counts = (table_count(&connection, "import_record_revisions"), table_count(&connection, "artifact_objects"));
    fs::remove_file(&source).unwrap();
    fs::rename(&preserved, &source).unwrap();
    let retried = import_at(&connection, &root);
    drop(connection);
    fs::remove_dir_all(&root).unwrap();

    assert!(rejected.outcomes.is_empty(), "linked root must not reach manifest or commit");
    assert!(has_code(&rejected, "symlink_rejected"));
    assert_eq!(before, after);
    assert_eq!(remaining, previous);
    assert_eq!(counts, (revisions, objects));
    assert_eq!(retried.outcomes[0].status, BundleStatus::Unchanged);
}

#[test]
fn discovery_truncation_preserves_existing_projection_and_allows_retry() {
    let root = sandbox("discovery-truncation");
    let bundle = single_finding_bundle(&root, "high");
    write_json(
        &bundle,
        "model-prompt-audit.json",
        &serde_json::json!({"instruction":"old-run"}),
    );
    let connection = open_connection(&initialize_db(&root));
    assert_eq!(
        import_at(&connection, &root).outcomes[0].status,
        BundleStatus::Imported
    );
    let previous = current_envelopes(&connection, RecordKind::FindingCandidate);
    assert!(!previous.is_empty());
    let revisions = table_count(&connection, "import_record_revisions");
    let objects = table_count(&connection, "artifact_objects");

    let nested = bundle.join("a/b/c/d/e/f/g/h/i");
    fs::create_dir_all(&nested).unwrap();
    fs::rename(
        bundle.join("findings.sarif"),
        nested.join("findings.sarif"),
    )
    .unwrap();
    let before = fingerprint_tree(&source_dir(&root));
    let truncated = import_at(&connection, &root);
    assert_eq!(
        current_envelopes(&connection, RecordKind::FindingCandidate),
        previous,
        "an incomplete directory must not revoke previously imported findings"
    );
    let outcome = &truncated.outcomes[0];
    assert_eq!(outcome.status, BundleStatus::Failed);
    assert_eq!((outcome.revisions, outcome.revoked), (0, 0));
    assert!(outcome.committed_records.is_none());
    assert!(has_code(&truncated, "depth_limit"));
    assert_eq!(
        table_count(&connection, "import_record_revisions"),
        revisions
    );
    assert_eq!(table_count(&connection, "artifact_objects"), objects);
    assert_eq!(fingerprint_tree(&source_dir(&root)), before);

    // Failure is per bundle, not a global refusal of healthy sibling runs.
    let sibling = source_dir(&root).join("healthy");
    write_json(
        &sibling,
        "model-prompt-audit.json",
        &serde_json::json!({"instruction":"healthy-run"}),
    );
    let mixed = import_at(&connection, &root);
    assert_eq!(mixed.outcomes.len(), 2);
    assert_eq!(mixed.outcomes[0].status, BundleStatus::Failed);
    assert_eq!(mixed.outcomes[1].status, BundleStatus::Imported);

    fs::rename(
        nested.join("findings.sarif"),
        bundle.join("findings.sarif"),
    )
    .unwrap();
    fs::remove_dir_all(bundle.join("a")).unwrap();
    let retried = import_at(&connection, &root);
    assert_eq!(retried.outcomes[0].status, BundleStatus::Unchanged);
    assert_eq!(
        current_envelopes(&connection, RecordKind::FindingCandidate),
        previous
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn discovery_depth_budget_is_relative_to_search_root_not_bundle_anchor() {
    let root = sandbox("discovery-depth-boundary");
    let source = source_dir(&root);
    let bundle = source.join("one/two");
    write_json(
        &bundle,
        "model-prompt-audit.json",
        &serde_json::json!({"instruction":"boundary"}),
    );
    write_json(&bundle, "three/model-prompt-audit.json", &serde_json::json!({}));
    let (bundles, notes) = discovery::discover(&[&source], 3);
    assert_eq!(bundles.len(), 1);
    assert_eq!(bundles[0].files.len(), 2);
    assert!(notes.is_empty());

    write_json(&bundle, "three/four/model-prompt-audit.json", &serde_json::json!({}));
    let (bundles, notes) = discovery::discover(&[&source], 3);
    assert_eq!(
        bundles[0].files.len(),
        2,
        "anchors must not reset the depth budget"
    );
    assert!(notes.iter().any(|note| note.code == "depth_limit"));
    assert_eq!(bundles[0].discovery_errors, notes);
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn discovery_rejected_entries_cannot_reuse_a_previously_complete_signature() {
    for relative in ["linked", "nested/linked"] {
        let root = sandbox("discovery-rejected-entry");
        let bundle = single_finding_bundle(&root, "high");
        let connection = open_connection(&initialize_db(&root));
        assert_eq!(
            import_at(&connection, &root).outcomes[0].status,
            BundleStatus::Imported
        );
        let previous = current_envelopes(&connection, RecordKind::FindingCandidate);
        let link = bundle.join(relative);
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&bundle, &link).unwrap();

        let summary = import_at(&connection, &root);
        let outcome = &summary.outcomes[0];
        assert_eq!(outcome.status, BundleStatus::Failed);
        assert_eq!((outcome.revisions, outcome.revoked), (0, 0));
        assert!(outcome.committed_records.is_none());
        assert!(outcome
            .diagnostics
            .iter()
            .any(|note| note.code == "symlink_rejected"));
        assert_eq!(
            current_envelopes(&connection, RecordKind::FindingCandidate),
            previous
        );

        fs::remove_file(link).unwrap();
        assert_eq!(
            import_at(&connection, &root).outcomes[0].status,
            BundleStatus::Unchanged
        );
        let _ = fs::remove_dir_all(root);
    }
}
