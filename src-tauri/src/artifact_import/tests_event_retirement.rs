// Retired event streams must not re-enter the importer through discovery,
// nested bundle collection, or callers that dispatch payloads directly.
const RETIRED_EVENT_NAMES: &[&str] = &["events.jsonl", "events.ndjson"];

#[test]
fn event_retirement_direct_dispatch_rejects_both_old_stream_names() {
    for name in RETIRED_EVENT_NAMES {
        let (rows, notes) = parse_audit_fixture(
            name,
            b"{\"type\":\"message\",\"content\":\"retired-event\"}\n",
            &Limits::default(),
        );
        assert!(rows.is_empty(), "retired stream accepted: {name}");
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].code, "unrecognized_artifact");
    }
}

#[test]
fn event_retirement_stream_only_directories_do_not_anchor_bundles() {
    let root = sandbox("retired-event-anchors");
    let source = source_dir(&root);
    for name in RETIRED_EVENT_NAMES {
        write_file(&source.join(name), name, "{\"content\":\"retired-event\"}\n");
    }
    let before = fingerprint_tree(&source);
    let (bundles, notes) = discovery::discover(&[source.as_path()], 8);
    assert!(bundles.is_empty(), "retired streams still anchor bundles");
    assert!(notes.is_empty());
    let connection = open_connection(&initialize_db(&root));
    let result = import_at(&connection, &root);
    assert!(result.outcomes.is_empty());
    for table in ["import_bundles", "import_bundle_files", "import_record_revisions"] {
        assert_eq!(table_count(&connection, table), 0, "unexpected rows in {table}");
    }
    assert_eq!(fingerprint_tree(&source), before);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn event_retirement_mixed_bundle_keeps_current_audits_without_old_streams() {
    let root = sandbox("retired-event-mixed");
    let source = source_dir(&root);
    for name in RETIRED_EVENT_NAMES {
        for dir in [&source, &source.join("nested")] {
            write_file(dir, name, "{\"content\":\"retired-event\"}\n");
        }
    }
    write_file(&source, "llm-hook.jsonl", "{\"kind\":\"model_call_started\",\"requestId\":\"current\"}\n");
    write_json(&source, "model-prompt-audit.json", &serde_json::json!({"instruction":"current-prompt"}));
    let before = fingerprint_tree(&source);
    let (bundles, notes) = discovery::discover(&[source.as_path()], 8);
    assert!(notes.is_empty());
    assert_eq!(bundles.len(), 1);
    assert_eq!(bundles[0].files.len(), 2, "nested retired streams must be excluded too");
    let connection = open_connection(&initialize_db(&root));
    let imported = import_at(&connection, &root);
    assert_eq!(imported.outcomes.len(), 1);
    let rows = current_envelopes(&connection, RecordKind::EventTrace);
    assert_eq!(rows.len(), 2);
    let text = serde_json::to_string(&rows).unwrap();
    assert!(!text.contains("retired-event"));
    assert!(text.contains("current-prompt"));
    assert!(rows.iter().all(|row| row.pointer("/provenance/importAdapter").and_then(serde_json::Value::as_str) == Some("model_audit")));
    assert_eq!(table_count(&connection, "import_bundle_files"), 2);
    let mut statement = connection.prepare("SELECT relative_path FROM import_bundle_files").unwrap();
    let names = statement.query_map([], |row| row.get::<_, String>(0)).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
    assert!(names.iter().all(|name| matches!(name.as_str(), "llm-hook.jsonl" | "model-prompt-audit.json")));
    assert!(has_code(&import_at(&connection, &root), "bundle_unchanged"));
    assert_eq!(fingerprint_tree(&source), before);
    fs::remove_dir_all(root).unwrap();
}
