#[test]
fn current_hook_bad_middle_line_is_reported_without_losing_later_rows() {
    let root = sandbox("trace-bad-line");
    let source = source_dir(&root);
    write_file(&source, "llm-hook.jsonl", "{\"requestId\":\"first\"}\n{broken\n{\"requestId\":\"last\"}\n");
    let before = fingerprint_tree(&source);
    let connection = open_connection(&initialize_db(&root));
    let result = import_at(&connection, &root);
    assert!(has_code(&result, "jsonl_bad_line"));
    let events = current_envelopes(&connection, RecordKind::EventTrace);
    assert_eq!(events.len(), 2);
    assert_eq!(events[1]["provenance"]["sourceRecordPointer"], "/3");
    assert_eq!(fingerprint_tree(&source), before);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn historical_trace_adapter_upgrade_reparses_without_rewriting_revisions() {
    let root = sandbox("trace-upgrade");
    write_json(&source_dir(&root), "model-prompt-audit.json", &serde_json::json!({"instruction":"old"}));
    let connection = open_connection(&initialize_db(&root));
    import_at(&connection, &root);
    let original: String = connection.query_row("SELECT envelope_json FROM import_record_revisions LIMIT 1", [], |row| row.get(0)).unwrap();
    let count = table_count(&connection, "import_record_revisions");
    // Simulate the pre-versioned signature written by adapter v1. Source bytes
    // are unchanged; a signature mismatch must still run reconciliation.
    connection.execute("UPDATE import_bundles SET signature='pre-versioned-signature'", []).unwrap();
    let upgraded = import_at(&connection, &root);
    assert!(!has_code(&upgraded, "bundle_unchanged"));
    let stored: String = connection.query_row("SELECT signature FROM import_bundles", [], |row| row.get(0)).unwrap();
    assert!(stored.ends_with(&format!("\u{1}adapter={}", canonical::ADAPTER_VERSION)));
    assert_eq!(table_count(&connection, "import_record_revisions"), count);
    assert_eq!(connection.query_row("SELECT envelope_json FROM import_record_revisions LIMIT 1", [], |row| row.get::<_, String>(0)).unwrap(), original);
    assert!(has_code(&import_at(&connection, &root), "bundle_unchanged"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn current_hook_preserves_each_record_and_isolated_session_identity() {
    let root = sandbox("trace-identity");
    let source = source_dir(&root);
    for directory in ["one", "two"] {
        let rows = (1..=4).map(|index| serde_json::json!({
            "kind":"model_call", "requestId":"same-request", "session_id":"root", "recordedAt":format!("t{index}")
        }).to_string()).collect::<Vec<_>>().join("\n");
        write_file(&source.join(directory), "llm-hook.jsonl", &rows);
    }
    let before = fingerprint_tree(&source);
    let connection = open_connection(&initialize_db(&root));
    import_at(&connection, &root);
    let rows = current_envelopes(&connection, RecordKind::EventTrace);
    assert_eq!(rows.len(), 8, "same IDs must not merge distinct lines or files");
    assert!(rows.iter().all(|row| scalar_text(row, "trace_kind") == "model_hook"));
    let keys: std::collections::BTreeSet<_> = rows.iter().map(|row| scalar_text(row, "trace_session_key")).collect();
    assert_eq!(keys.len(), 2, "session identity must be source-bound and non-secret");
    assert!(keys.iter().all(|key| !key.is_empty() && key != "root"));
    assert_eq!(fingerprint_tree(&source), before);
    assert!(import_at(&connection, &root).outcomes.iter().all(|outcome| outcome.status == BundleStatus::Unchanged));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn current_hook_and_prompt_audit_are_imported_redacted_without_touching_sources() {
    let root = sandbox("trace-hook");
    let source = source_dir(&root);
    write_file(&source, "llm-hook.jsonl", &[
        serde_json::json!({"kind":"model_call_started","requestId":"r1","recordedAt":"t1"}).to_string(),
        serde_json::json!({"kind":"model_call","requestId":"r1","recordedAt":"t2","status":"200","request":{"headers":{"Authorization":SECRET_MARKER}},"usage":{"total_tokens":12}}).to_string(),
        "{truncated".into(),
    ].join("\n"));
    write_json(&source, "model-prompt-audit.json", &serde_json::json!({
        "captureMode":"full", "instruction":format!("Authorized test\nAuthorization: {SECRET_MARKER}"),
        "exactModelRequest":true
    }));
    let before = fingerprint_tree(&source);
    let connection = open_connection(&initialize_db(&root));
    let result = import_at(&connection, &root);
    assert!(has_code(&result, "jsonl_tail_truncated"));
    let rows = current_envelopes(&connection, RecordKind::EventTrace);
    assert_eq!(rows.iter().filter(|row| scalar_text(row, "trace_kind") == "model_hook").count(), 2);
    assert_eq!(rows.iter().filter(|row| scalar_text(row, "trace_kind") == "prompt_audit").count(), 1);
    assert!(!serde_json::to_string(&rows).unwrap().contains(SECRET_MARKER));
    assert_eq!(fingerprint_tree(&source), before);
    assert!(rows.iter().all(|row| row.pointer("/claim/executionEligible") == Some(&serde_json::json!(false))));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn audit_retirement_old_prompt_name_neither_discovers_nor_joins_current_bundle() {
    let root = sandbox("retired-audit-discovery");
    let source = source_dir(&root);
    let old_name = "strix-prompt-audit.json";
    write_json(&source, old_name, &serde_json::json!({"instruction":"obsolete-only"}));
    let (bundles, notes) = discovery::discover(&[source.as_path()], 8);
    assert!(bundles.is_empty(), "obsolete name must not anchor a bundle");
    assert!(notes.is_empty());
    write_json(&source, "model-prompt-audit.json", &serde_json::json!({"instruction":"current-only"}));
    let before = fingerprint_tree(&source);
    let connection = open_connection(&initialize_db(&root));
    import_at(&connection, &root);
    let rows = current_envelopes(&connection, RecordKind::EventTrace);
    assert_eq!(rows.len(), 1);
    assert_eq!(scalar_text(&rows[0], "trace_source"), "model-prompt-audit.json");
    assert_eq!(connection.query_row("SELECT count(*) FROM import_bundle_files WHERE relative_path=?1", [old_name], |row| row.get::<_, i64>(0)).unwrap(), 0);
    assert!(!serde_json::to_string(&rows).unwrap().contains("obsolete-only"));
    assert_eq!(fingerprint_tree(&source), before);
    fs::remove_dir_all(root).unwrap();
}

fn parse_audit_fixture(name: &str, bytes: &[u8], limits: &Limits) -> (Vec<canonical::CanonicalRecord>, Vec<diagnostics::Diagnostic>) {
    let manifest = manifest::Manifest { bundle_id: "audit-fixture".into(), canonical: String::new(), files: Vec::new() };
    let payloads = vec![(name.to_string(), bytes.to_vec())];
    let context = adapters::ParseContext {
        bundle_id: &manifest.bundle_id, manifest: &manifest, payloads: &payloads,
        limits,
    };
    adapters::parse_bundle(&context)
}

#[test]
fn audit_retirement_direct_dispatch_rejects_old_prompt_name() {
    let bytes = br#"{"instruction":"obsolete-only"}"#;
    let (rows, notes) = parse_audit_fixture("strix-prompt-audit.json", bytes, &Limits::default());
    assert!(rows.is_empty(), "direct dispatch must not bypass discovery retirement");
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].code, "unrecognized_artifact");
    let (rows, notes) = parse_audit_fixture("model-prompt-audit.json", bytes, &Limits::default());
    assert_eq!(rows.len(), 1);
    assert!(notes.is_empty());
}

#[test]
fn current_audit_codec_keeps_valid_lines_and_reports_resource_limits() {
    let limits = Limits { json_line_bytes: 40, json_depth: 2, ..Limits::default() };
    let stream = format!("{{\"kind\":\"first\"}}\n{{bad\n{}\n{{\"a\":{{\"b\":{{}}}}}}\n{{\"kind\":\"last\"}}\n{{tail", "x".repeat(41));
    let (rows, notes) = parse_audit_fixture("llm-hook.jsonl", stream.as_bytes(), &limits);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].provenance.source_record_pointer, "/1");
    assert_eq!(rows[1].provenance.source_record_pointer, "/5");
    let codes: Vec<_> = notes.iter().map(|note| note.code.as_str()).collect();
    assert_eq!(codes, ["jsonl_bad_line", "line_length_limit", "json_depth_limit", "jsonl_tail_truncated"]);
    for (bytes, code) in [(br#"{"a":{"b":{}}}"#.as_slice(), "json_depth_limit"), (b"{".as_slice(), "json_invalid")] {
        let (rows, notes) = parse_audit_fixture("model-prompt-audit.json", bytes, &limits);
        assert!(rows.is_empty());
        assert_eq!(notes[0].code, code);
    }
}

#[test]
fn current_audit_codec_keeps_duplicates_utf8_and_physical_line_positions() {
    let first = serde_json::json!({"session_id":"first", "message":"协作消息 🦖"}).to_string();
    let second = serde_json::json!({"session_id":"second", "message":"协作消息 🦖"}).to_string();
    let stream = format!("\n{first}\n{first}\n{second}\n{{broken\n\n");
    let (rows, notes) = parse_audit_fixture("llm-hook.jsonl", stream.as_bytes(), &Limits::default());
    assert_eq!(rows.len(), 3, "repeated messages are separate facts");
    let pointers: Vec<_> = rows.iter().map(|row| row.provenance.source_record_pointer.as_str()).collect();
    assert_eq!(pointers, ["/2", "/3", "/4"]);
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].code, "jsonl_bad_line", "a trailing blank physical line remains observable");
    let envelopes: Vec<_> = rows.iter().map(|row| serde_json::to_value(row).unwrap()).collect();
    assert_eq!(scalar_text(&envelopes[0], "trace_session_key"), scalar_text(&envelopes[1], "trace_session_key"));
    assert_ne!(scalar_text(&envelopes[0], "trace_session_key"), scalar_text(&envelopes[2], "trace_session_key"));
    assert!(serde_json::to_string(&envelopes).unwrap().contains("协作消息 🦖"));
}
