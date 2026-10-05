// §13.2 — Original bytes and current report/audit format acceptance.

#[test]
fn imported_originals_stay_byte_identical_to_the_source() {
    let root = sandbox("imp001");
    let source = source_dir(&root);
    write_file(&source, "llm-hook.jsonl", "{\"kind\":\"model_call_started\",\"requestId\":\"source-integrity\"}\n");
    write_json(&source, "model-prompt-audit.json", &serde_json::json!({"instruction":"source integrity"}));
    let before = fingerprint_tree(&source);
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    import_at(&connection, &root);
    let rows: Vec<(String, String, i64)> = connection
        .prepare(
            "SELECT f.relative_path, f.content_hash, o.bytes
                 FROM import_bundle_files f JOIN artifact_objects o ON o.id=f.artifact_object_id",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert_eq!(rows.len(), before.len(), "每个当前源文件都应有对象记录：{rows:?}");
    for (relative, hash, bytes) in rows {
        let (source_hash, source_bytes, _, _) = before
            .get(&relative)
            .unwrap_or_else(|| panic!("源树里没有 {relative}"));
        assert_eq!(&hash, source_hash, "{relative} 内容哈希漂移");
        assert_eq!(bytes as u64, *source_bytes, "{relative} 长度漂移");
        let stored = PathBuf::from(
            connection
                .query_row(
                    "SELECT storage_path FROM artifact_objects WHERE content_hash=?1",
                    [&hash],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
        );
        let stored_bytes = fs::read(&stored).unwrap_or_else(|error| {
            panic!("原文副本 {relative} 不可读：{error}");
        });
        assert_eq!(
            crate::artifact_import::canonical::sha256_hex(&stored_bytes),
            hash,
            "{relative} 的原文不是逐字节相同的"
        );
    }
    assert_eq!(fingerprint_tree(&source), before, "导入不得改写源文件");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn current_report_and_model_audit_keep_independent_records() {
    let root = sandbox("imp004");
    let bundle = source_dir(&root).join("bundle");
    write_json(&bundle, "model-prompt-audit.json", &serde_json::json!({"instruction":"当前审计"}));
    write_json(&bundle, "findings.sarif", &sarif_document(vec![sarif_result(
        "SARIF-ONLY", "error", "当前报告", &[("src/a.py", 3)], None,
    )]));
    let connection = open_connection(&initialize_db(&root));
    assert_eq!(import_at(&connection, &root).outcomes[0].status, BundleStatus::Imported);
    let findings = current_envelopes(&connection, RecordKind::FindingCandidate);
    assert_eq!(findings.len(), 1);
    assert_eq!(scalar_text(&findings[0], "rule_id"), "SARIF-ONLY");
    let audits = current_envelopes(&connection, RecordKind::EventTrace);
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0]["extensions"]["instruction"], "当前审计");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn current_audit_preserves_unicode_quotes_newlines_and_empty_values() {
    let root = sandbox("imp011");
    let source = source_dir(&root);
    let instruction = "中文 \"引号\"\n第二行";
    write_json(&source, "model-prompt-audit.json", &serde_json::json!({
        "instruction": instruction, "content": "", "extra_note": "",
    }));
    let before = fingerprint_tree(&source);
    let connection = open_connection(&initialize_db(&root));
    import_at(&connection, &root);
    let audits = current_envelopes(&connection, RecordKind::EventTrace);
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0]["extensions"]["instruction"], instruction);
    assert_eq!(audits[0]["extensions"]["extra_note"], "");
    assert_eq!(audits[0]["payload"]["content"], "");
    assert_eq!(fingerprint_tree(&source), before);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn current_report_message_is_preserved_as_inert_text() {
    let root = sandbox("imp012");
    let source = source_dir(&root);
    let message = "# 不可信文本\n<script>alert('inert')</script>";
    write_json(&source, "findings.sarif", &sarif_document(vec![sarif_result(
        "INERT", "warning", message, &[("src/current.rs", 1)], None,
    )]));
    let before = fingerprint_tree(&source);
    let connection = open_connection(&initialize_db(&root));
    import_at(&connection, &root);
    let records = current_envelopes(&connection, RecordKind::FindingCandidate);
    assert_eq!(records.len(), 1);
    assert_eq!(scalar_text(&records[0], "title"), message);
    assert_eq!(scalar_text(&records[0], "message"), message);
    assert!(payload_of(&records[0]).get("html").is_none());
    assert!(payload_of(&records[0]).get("script").is_none());
    assert_eq!(fingerprint_tree(&source), before);
    fs::remove_dir_all(root).unwrap();
}
