// §13.2 损坏与安全（§COR-001 … §COR-010）。

#[test]
fn one_broken_bundle_never_blocks_the_healthy_ones() {
    let root = sandbox("cor001");
    write_file(
        &source_dir(&root).join("broken"),
        "model-prompt-audit.json",
        "{ this is not json",
    );
    let good = copy_fixture_bundle(&root, "producer_1_6_2");
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    let summary = import_at(&connection, &root);
    assert_eq!(summary.outcomes.len(), 2, "{:?}", summary.outcomes);
    assert_eq!(
        summary
            .outcomes
            .iter()
            .filter(|outcome| outcome.status == BundleStatus::Failed)
            .count(),
        1,
        "坏 bundle 必须单独失败"
    );
    assert_eq!(
        summary
            .outcomes
            .iter()
            .find(|outcome| outcome.source_path == good.display().to_string())
            .map(|outcome| outcome.status),
        Some(BundleStatus::Imported),
        "健康 bundle 必须照常导入：{:?}",
        summary.outcomes
    );
    assert!(has_code(&summary, "json_invalid"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_broken_report_still_lets_a_healthy_sarif_import() {
    let root = sandbox("cor002");
    let bundle = source_dir(&root).join("bundle");
    write_file(&bundle, "broken.sarif", "{\"runs\": [");
    write_json(
        &bundle,
        "findings.sarif",
        &sarif_document(vec![sarif_result(
            "RULE-OK",
            "error",
            "SARIF 仍然可用",
            &[("src/ok.py", 4)],
            None,
        )]),
    );
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    let summary = import_at(&connection, &root);
    assert!(has_code(&summary, "sarif_invalid"), "{:?}", codes(&summary));
    let envelopes = current_envelopes(&connection, RecordKind::FindingCandidate);
    assert_eq!(envelopes.len(), 1, "坏 JSON 不该拖垮 SARIF");
    assert_eq!(scalar_text(&envelopes[0], "rule_id"), "RULE-OK");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn jsonl_tail_truncation_and_middle_bad_line_behave_differently() {
    let root = sandbox("cor003");
    let bundle = source_dir(&root).join("bundle");
    write_file(
            &bundle,
            "llm-hook.jsonl",
            "{\"type\":\"message\",\"call_id\":\"c1\"}\n{\"type\":\"message\",\"call_id\":\"c2\"}\n{\"type\":\"message\",\"call_",
        );
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    let summary = import_at(&connection, &root);
    assert!(
        has_code(&summary, "jsonl_tail_truncated"),
        "{:?}",
        codes(&summary)
    );
    assert_eq!(
        current_envelopes(&connection, RecordKind::EventTrace).len(),
        2,
        "尾行截断不能丢掉前面的有效行"
    );
    assert_eq!(
        severity_of(&summary, "jsonl_tail_truncated"),
        Some(Severity::Info),
        "尾行截断是已知情况，不该按错误处理"
    );

    let other = sandbox("cor003b");
    let bundle_b = source_dir(&other).join("bundle");
    write_file(
        &bundle_b,
        "llm-hook.jsonl",
        "{\"call_id\":\"c1\"}\n{ broken middle }\n{\"call_id\":\"c3\"}\n{\"call_id\":\"c4\"}\n",
    );
    let db_b = initialize_db(&other);
    let connection_b = open_connection(&db_b);
    let summary_b = import_at(&connection_b, &other);
    assert!(
        has_code(&summary_b, "jsonl_bad_line"),
        "{:?}",
        codes(&summary_b)
    );
    let traces = current_envelopes(&connection_b, RecordKind::EventTrace);
    assert_eq!(traces.len(), 3, "中间坏行不能吞掉后续行");
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(other);
}



#[test]
fn symlink_device_and_escaping_paths_are_rejected() {
    let root = sandbox("cor006");
    let bundle = source_dir(&root).join("bundle");
    write_json(
        &bundle,
        "model-prompt-audit.json",
        &serde_json::json!({"instruction":"r"}),
    );
    let outside = source_dir(&root).join("outside.json");
    fs::write(&outside, "{}").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, bundle.join("findings.sarif")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(source_dir(&root).join(".."), bundle.join("escapes")).unwrap();
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    let summary = import_at(&connection, &root);
    assert!(
        has_code(&summary, "symlink_rejected"),
        "§COR-006：符号链接必须拒绝：{:?}",
        codes(&summary)
    );
    // A path that is not below the bundle root cannot enter a manifest.
    let escaping = vec![outside.clone()];
    let failure = crate::artifact_import::manifest::build(
        &bundle,
        &bundle,
        &escaping,
        &crate::artifact_import::Limits::default(),
    )
    .err()
    .unwrap_or_default();
    assert_eq!(failure.len(), 1, "{failure:?}");
    assert_eq!(failure[0].code, "path_traversal_rejected");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn every_limit_produces_a_controlled_failure() {
    let root = sandbox("cor007");
    let bundle = source_dir(&root).join("bundle");
    write_file(&bundle, "model-prompt-audit.json", &" ".repeat(4000));
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    let tight = Limits {
        file_bytes: 128,
        ..Limits::default()
    };
    let summary = import_dir(&connection, &root, &tight);
    assert_eq!(summary.outcomes.len(), 1);
    assert_eq!(summary.outcomes[0].status, BundleStatus::Failed);
    assert!(
        has_code(&summary, "file_bytes_limit"),
        "{:?}",
        codes(&summary)
    );
    assert_eq!(table_count(&connection, "import_bundles"), 0);

    let deep = sandbox("cor007b");
    let bundle_b = source_dir(&deep).join("bundle");
    write_file(
        &bundle_b,
        "model-prompt-audit.json",
        "{\"a\":[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]}",
    );
    let db_b = initialize_db(&deep);
    let connection_b = open_connection(&db_b);
    let tight_depth = Limits {
        json_depth: 6,
        ..Limits::default()
    };
    let summary_b = import_dir(&connection_b, &deep, &tight_depth);
    assert!(
        has_code(&summary_b, "json_depth_limit"),
        "深度超限必须受控失败：{:?}",
        codes(&summary_b)
    );

    let many = sandbox("cor007c");
    let bundle_c = source_dir(&many).join("bundle");
    let rows: Vec<serde_json::Value> = (0..12)
            .map(|index| {
                sarif_result(&format!("rule-{index}"), "warning", "行", &[],
                    Some(serde_json::json!({"endpoint":format!("/api/{index}")})))
            })
            .collect();
    write_json(&bundle_c, "findings.sarif", &sarif_document(rows));
    let db_c = initialize_db(&many);
    let connection_c = open_connection(&db_c);
    let few = Limits {
        records: 5,
        ..Limits::default()
    };
    let summary_c = import_dir(&connection_c, &many, &few);
    assert!(
        has_code(&summary_c, "record_count_limit"),
        "{:?}",
        codes(&summary_c)
    );
    assert!(
        summary_c
            .outcomes
            .iter()
            .all(|outcome| outcome.revisions == 0),
        "超限不得留下投影"
    );

    let long_line = sandbox("cor007d");
    let bundle_d = source_dir(&long_line).join("bundle");
    write_file(
        &bundle_d,
        "llm-hook.jsonl",
        &format!("{{\"call_id\":\"a\",\"x\":\"{}\"}}\n", "y".repeat(5000)),
    );
    let db_d = initialize_db(&long_line);
    let connection_d = open_connection(&db_d);
    let narrow = Limits {
        json_line_bytes: 512,
        ..Limits::default()
    };
    let summary_d = import_dir(&connection_d, &long_line, &narrow);
    assert!(
        has_code(&summary_d, "line_length_limit"),
        "{:?}",
        codes(&summary_d)
    );
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(deep);
    let _ = fs::remove_dir_all(many);
    let _ = fs::remove_dir_all(long_line);
}

#[test]
fn credentials_stay_in_the_original_and_never_reach_rows_or_views() {
    let root = sandbox("cor008");
    let bundle = source_dir(&root).join("bundle");
    write_json(
        &bundle,
        "model-prompt-audit.json",
        &serde_json::json!({"instruction":"secret-run"}),
    );
    write_json(
        &bundle,
        "findings.sarif",
        &sarif_document(vec![sarif_result("s1", "warning", "含凭据的条目", &[], Some(serde_json::json!({"endpoint":"/api/x","poc_description":format!("复现需要 password={SECRET_MARKER} 才能触发")})))]),
    );
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    import_at(&connection, &root);
    let envelopes = current_envelopes(&connection, RecordKind::FindingCandidate);
    assert_eq!(envelopes.len(), 1);
    let json = serde_json::to_string(&envelopes).unwrap();
    assert!(
        !json.contains(SECRET_MARKER),
        "§COR-008：凭据值不得进入记录行：{json}"
    );
    assert!(
        json.contains("<redacted:"),
        "应留下可关联的脱敏标记：{json}"
    );
    let rows: Vec<String> = connection
        .prepare(
            "SELECT code || '|' || detail FROM import_diagnostics UNION ALL
                 SELECT display_text FROM artifact_objects UNION ALL
                 SELECT envelope_json FROM import_record_revisions",
        )
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert!(
        rows.iter().all(|row| !row.contains(SECRET_MARKER)),
        "§COR-008：任何数据库文本列都不该出现凭据"
    );
    let originals: Vec<(String, String, i64)> = connection
        .prepare("SELECT content_hash, storage_path, secret_material FROM artifact_objects")
        .unwrap()
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    let key =
        crate::artifact_import::sealing::Key::load_or_create(&root.join("artifact-import.key"))
            .unwrap();
    let mut sealed = 0usize;
    let mut plain = 0usize;
    for (content_hash, storage_path, secret_material) in &originals {
        let blob = fs::read(storage_path).unwrap();
        if *secret_material == 1 {
            sealed += 1;
            // 缺口 2：文件权限不是加密。磁盘上必须是解不开的密文，
            // 而“无损”则由解密后的字节与源文件哈希一致来证明。
            assert!(
                crate::artifact_import::sealing::is_sealed(&blob),
                "{storage_path} 被标记为含凭据却没有封存"
            );
            assert!(
                !String::from_utf8_lossy(&blob).contains(SECRET_MARKER),
                "密文里不能读到明文凭据"
            );
            let recovered = crate::artifact_import::sealing::open(&key, &blob).unwrap();
            assert_eq!(
                crate::artifact_import::canonical::sha256_hex(&recovered),
                *content_hash,
                "§IMP-001：解密后的原文必须逐字节一致"
            );
            assert!(String::from_utf8_lossy(&recovered).contains(SECRET_MARKER));
        } else {
            plain += 1;
            assert!(
                !crate::artifact_import::sealing::is_sealed(&blob),
                "无凭据的原文不该被加密，否则无法直接归档"
            );
            assert_eq!(
                crate::artifact_import::canonical::sha256_hex(&blob),
                *content_hash,
                "§IMP-001：明文原文逐字节一致"
            );
        }
    }
    assert_eq!(sealed, 1, "只有含凭据的那份原文需要封存");
    assert!(plain >= 1, "model-prompt-audit.json 这类无凭据原文保持明文");
    let _ = fs::remove_dir_all(root);
}

/// Nested JSON credentials remain sealed; parsing current artifacts creates no scratch copy.
#[test]
fn escaped_current_audit_credentials_are_sealed_without_plaintext_copies() {
    let root = sandbox("cor008b");
    let bundle = source_dir(&root);
    let original = serde_json::json!({"kind":"model_call","requestId":"r",
        "request":{"body":serde_json::json!({"password":SECRET_MARKER}).to_string()}}).to_string();
    write_file(&bundle, "llm-hook.jsonl", &original);
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    let summary = import_at(&connection, &root);
    assert!(
        summary.outcomes[0].status == BundleStatus::Imported,
        "{:?}",
        codes(&summary)
    );
    let (storage_path, secret_material): (String, i64) = connection
        .query_row(
            "SELECT o.storage_path, o.secret_material FROM artifact_objects o
                 JOIN import_bundle_files f ON f.artifact_object_id=o.id
                 WHERE f.relative_path='llm-hook.jsonl'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(secret_material, 1, "嵌套 JSON 字符串里的凭据也要被识别");
    let blob = fs::read(&storage_path).unwrap();
    assert!(
        crate::artifact_import::sealing::is_sealed(&blob),
        "当前审计原文必须封存，而不是靠文件权限"
    );
    assert!(!String::from_utf8_lossy(&blob).contains(SECRET_MARKER));
    let key =
        crate::artifact_import::sealing::Key::load_or_create(&root.join("artifact-import.key"))
            .unwrap();
    let recovered = crate::artifact_import::sealing::open(&key, &blob).unwrap();
    assert_eq!(
        recovered,
        original.as_bytes(),
        "§IMP-001：当前审计原文解密后必须逐字节相同"
    );

    // scratch 与 app 数据库都不许留下明文；用户自己的输入目录不在检查范围内。
    let mut scanned = 0usize;
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path == source_dir(&root) {
                    continue;
                }
                stack.push(path);
                continue;
            }
            if path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                == Some("artifact-import.key".to_string())
            {
                continue;
            }
            scanned += 1;
            let bytes = fs::read(&path).unwrap_or_default();
            assert!(
                !String::from_utf8_lossy(&bytes).contains(SECRET_MARKER),
                "§COR-008：{} 里出现了明文凭据",
                path.display()
            );
        }
    }
    assert!(scanned > 0, "必须真的扫描过落盘文件");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn importing_never_changes_the_source_tree() {
    let root = sandbox("cor009");
    copy_fixture_bundle(&root, "producer_1_5_3");
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    let before = fingerprint_tree(&source_dir(&root));
    import_at(&connection, &root);
    import_at(&connection, &root);
    assert_eq!(
        fingerprint_tree(&source_dir(&root)),
        before,
        "§COR-009：源树必须完全不变"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn corrupt_or_unknown_input_never_becomes_a_result() {
    let root = sandbox("cor010");
    let bundle = source_dir(&root).join("bundle");
    write_file(&bundle, "model-prompt-audit.json", "[]{");
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    let summary = import_at(&connection, &root);
    assert_eq!(
        table_count(&connection, "import_projection_memberships"),
        0,
        "{:?}",
        summary.outcomes
    );
    assert_eq!(table_count(&connection, "sentinel_findings"), 0);
    assert_eq!(table_count(&connection, "agent_runs"), 0);
    assert!(summary
        .outcomes
        .iter()
        .any(|outcome| outcome.status == BundleStatus::Failed));
    assert!(
        !summary
            .outcomes
            .iter()
            .any(|outcome| outcome.status == BundleStatus::Imported),
        "损坏输入不能报成成功"
    );
    let _ = fs::remove_dir_all(root);
}
