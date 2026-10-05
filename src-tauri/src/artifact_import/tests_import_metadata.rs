// Current audit streams, source identity and credential isolation.
// Retired run/status/usage and coverage rejection have dedicated suites.


#[test]
fn recon_stages_and_current_audit_streams_remain_importable() {
    let root = sandbox("imp016");
    let bundle = copy_fixture_bundle(&root, "producer_1_5_3");
    write_file(&bundle, "llm-hook.jsonl", "{\"kind\":\"model_call_started\",\"requestId\":\"audit-root\"}\n");
    write_file(&bundle, "nested/llm-hook.jsonl", "{\"kind\":\"model_call\",\"requestId\":\"audit-nested\"}\n");
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    let summary = import_at(&connection, &root);
    assert!(
        !has_code(&summary, "unrecognized_artifact"),
        "{:?}",
        codes(&summary)
    );
    let evidence = current_envelopes(&connection, RecordKind::EvidenceNote);
    let stages = evidence
        .iter()
        .filter(|row| {
            ["s1", "s2", "s3", "s4", "s5", "summary"].contains(&scalar_text(row, "stage").as_str())
        })
        .count();
    assert_eq!(stages, 6, "S1–S5 与 summary 都要成为证据记录");
    assert!(
        evidence
            .iter()
            .any(|row| scalar_text(row, "url").contains("fixture.invalid")
                && payload_of(row).get("apis").is_some()),
        "旧 recon 必须作为证据导入"
    );
    let events: Vec<_> = current_envelopes(&connection, RecordKind::EventTrace)
        .into_iter()
        .filter(|row| row.pointer("/provenance/importAdapter").and_then(serde_json::Value::as_str) == Some("model_audit"))
        .collect();
    assert_eq!(events.len(), 2, "current audits must remain independently addressable");
    let artifacts: Vec<String> = events
        .iter()
        .map(|row| {
            row.pointer("/provenance/sourceArtifactId")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    let distinct = artifacts
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    assert!(
        distinct.len() == 2,
        "事件流至少要来自两个不同文件：{artifacts:?}"
    );
    for wanted in ["producer_1_5_3/llm-hook.jsonl", "producer_1_5_3/nested/llm-hook.jsonl"] {
        assert!(events.iter().any(|row| scalar_text(row, "trace_source") == wanted), "missing audit source {wanted}");
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn current_hook_session_credentials_are_redacted_without_changing_source() {
    let root = sandbox("audit-session-privacy");
    let source = source_dir(&root);
    let input = serde_json::json!({"kind":"model_call", "requestId":"r", "session_id":"private-session",
        "request":{"messages":[{"role":"user","content":"password=do-not-store"}]}}).to_string();
    write_file(&source, "llm-hook.jsonl", &input);
    let before = fingerprint_tree(&source);
    let connection = open_connection(&initialize_db(&root));
    import_at(&connection, &root);
    let traces = current_envelopes(&connection, RecordKind::EventTrace);
    assert_eq!(traces.len(), 1);
    assert!(scalar_text(&traces[0], "session_id").starts_with("<redacted"));
    let displayed = serde_json::to_string(&traces).unwrap();
    assert!(!displayed.contains("private-session"));
    assert!(!displayed.contains(SECRET_MARKER));
    assert_eq!(fingerprint_tree(&source), before);
    fs::remove_dir_all(root).unwrap();
}
