// §9.7/§9.9 — Canonical identity, multi-report provenance and merged revisions.
// Product JSON/SARIF cross-format coverage lives in tests_source_findings.rs.

fn merge_fingerprint_fixtures(rows: Vec<serde_json::Value>) -> Vec<canonical::CanonicalRecord> {
    let records = rows.into_iter().enumerate().map(|(index, row)| {
        let payload = row.as_object().unwrap().clone();
        canonical::CanonicalRecord::new(canonical::RecordInput {
            kind: RecordKind::FindingCandidate, adapter: "fixture",
            producer: canonical::Producer::new("fixture", "1", "canonical"),
            bundle_id: "fixture".into(), source_artifact_id: format!("artifact-{index}"),
            pointer: format!("/{index}"), identity: canonical::finding_fingerprint(&payload),
            payload, known: adapters::FINDING_FIELDS,
        })
    }).collect();
    reconcile::merge_by_logical_key(records).into_values().collect()
}

#[test]
fn one_finding_across_reports_merges_with_field_provenance() {
    let root = sandbox("imp005");
    let bundle = source_dir(&root).join("bundle");
    write_json(
        &bundle,
        "primary.sarif",
        &sarif_document(vec![sarif_result("RULE-1", "warning", "越权读取订单", &[], Some(serde_json::json!({"severity":"high","endpoint":"/api/v1/order","cwe":"CWE-639"})))]),
    );
    write_json(
        &bundle,
        "findings.sarif",
        &sarif_document(vec![sarif_result(
            "RULE-1",
            "error",
            "越权读取订单",
            &[("src/order.py", 42)],
            Some(serde_json::json!({"cwe": "CWE-639", "endpoint": "/api/v1/order"})),
        )]),
    );
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    import_at(&connection, &root);
    let envelopes = current_envelopes(&connection, RecordKind::FindingCandidate);
    let sarif_rows = envelopes
        .iter()
        .filter(|envelope| scalar_text(envelope, "rule_id") == "RULE-1")
        .count();
    assert_eq!(sarif_rows, 1, "同一逻辑记录必须合并成一条：{sarif_rows}");
    let merged = envelope_where(&envelopes, "endpoint", "/api/v1/order")
        .cloned()
        .expect("合并后的记录");
    let origins = merged.get("fieldOrigins").cloned().unwrap_or_default();
    assert_eq!(
        origins
            .get("severity")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default(),
        "sarif:/runs/0/results/0",
        "严重度的来源要能追到具体适配器与条目"
    );
    assert!(
        scalar_text(&merged, "path") == "src/order.py",
        "SARIF 才有的字段应被合并进来：{merged}"
    );
    let _ = fs::remove_dir_all(root);
}

/// §9.7 缺口 8：五元组的每一维都必须参与区分，只差一维就是两个问题；
/// 五维全同的两条则必须并成一条。
#[test]
fn fingerprint_dimensions_separate_lookalike_findings() {
    let base = serde_json::json!({
        "cwe":"CWE-639","target":"https://example.invalid","endpoint":"/api/v1/order",
        "method":"GET","parameter":"orderId","region":"src/a.py:10","title":"基准条目"
    });
    let rows = vec![
        base.clone(),
        with_override(
            &base,
            serde_json::json!({"id":"b","region":"src/a.py:99","title":"只差区域"}),
        ),
        with_override(
            &base,
            serde_json::json!({"id":"c","method":"POST","title":"只差方法"}),
        ),
        with_override(
            &base,
            serde_json::json!({"id":"d","parameter":"buyerId","title":"只差参数"}),
        ),
        with_override(
            &base,
            serde_json::json!({"id":"e","endpoint":"/api/v2/order","title":"只差端点"}),
        ),
        // 五维与基准完全一致，只有标题不同：必须并到基准那一条上。
        with_override(&base, serde_json::json!({"id":"f","title":"重复条目"})),
    ];
    // Exercise the full canonical identity directly: standard SARIF does not
    // promote arbitrary HTTP method/parameter properties into identity fields.
    let envelopes: Vec<_> = merge_fingerprint_fixtures(rows).into_iter()
        .map(|row|serde_json::to_value(row).unwrap()).collect();
    let titles = envelopes
        .iter()
        .map(|row| scalar_text(row, "title"))
        .collect::<Vec<_>>();
    assert_eq!(
        envelopes.len(),
        5,
        "五个维度各自不同的条目不能互相吞并，第六条重复项要合并：{titles:?}"
    );
    let merged = envelopes
        .iter()
        .find(|row| scalar_text(row, "title") == "基准条目")
        .expect("基准条目必须在");
    assert!(
        merged
            .get("conflicts")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|conflicts| conflicts
                .iter()
                .any(
                    |entry| entry.get("field").and_then(serde_json::Value::as_str) == Some("title")
                )),
        "重复条目的标题差异要留下冲突记录：{merged}"
    );
}

/// Canonical URL split/case/space normalization must not depend on an adapter.
#[test]
fn fingerprint_normalizes_the_same_finding_across_canonical_inputs() {
    let rows = merge_fingerprint_fixtures(vec![
        serde_json::json!({"title":"first", "cwe":"  cwe-639 ",
            "target":"HTTPS://Example.Invalid/api/v1/order/", "method":"get", "parameter":" orderId "}),
        serde_json::json!({"title":"second", "cwe":"CWE-639",
            "target":"https://example.invalid", "endpoint":"/api/v1/order", "method":"GET", "parameter":"orderId"}),
    ]);
    assert_eq!(rows.len(), 1, "normalized identities must merge: {rows:?}");
    assert_eq!(rows[0].contributing_artifacts.len(), 2);
}

#[test]
fn a_supplement_from_another_report_revises_the_merged_record() {
    let root = sandbox("imp005d");
    let bundle = source_dir(&root).join("bundle");
    write_json(
        &bundle,
        "model-prompt-audit.json",
        &serde_json::json!({"instruction":"merge"}),
    );
    write_json(
        &bundle,
        "primary.sarif",
        &sarif_document(vec![sarif_result("v1", "warning", "越权读取订单", &[], Some(serde_json::json!({"severity":"high","endpoint":"/api/v1/order","method":"GET","cwe":"CWE-639"})))]),
    );
    let primary = bundle.join("primary.sarif");
    let primary_hash = crate::artifact_import::canonical::sha256_hex(&fs::read(&primary).unwrap());
    let properties = |extra: serde_json::Value| {
        let mut map = serde_json::json!({
            "cwe":"CWE-639","endpoint":"/api/v1/order","method":"GET"
        });
        if let (Some(object), Some(additions)) = (map.as_object_mut(), extra.as_object()) {
            for (key, value) in additions {
                object.insert(key.clone(), value.clone());
            }
        }
        map
    };
    write_json(
        &bundle,
        "findings.sarif",
        &sarif_document(vec![sarif_result(
            "RULE-1",
            "error",
            "越权读取订单",
            &[("src/order.py", 42)],
            Some(properties(serde_json::json!({}))),
        )]),
    );
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    import_at(&connection, &root);
    let keys = logical_keys(&connection, RecordKind::FindingCandidate);
    assert_eq!(keys.len(), 1, "首轮就该合并成一条：{keys:?}");
    let revisions = table_count(&connection, "import_record_revisions");

    // The supplement comes from SARIF's own evidence slot, so the primary SARIF
    // file is never touched.
    let mut supplemented = sarif_document(vec![sarif_result(
        "RULE-1",
        "error",
        "越权读取订单",
        &[("src/order.py", 42)],
        Some(properties(serde_json::json!({}))),
    )]);
    supplemented["runs"][0]["results"][0]["partialFingerprints"] =
        serde_json::json!({"primaryLocationLineHash": "abc123"});
    write_json(&bundle, "findings.sarif", &supplemented);
    let summary = import_at(&connection, &root);
    assert_eq!(
        logical_keys(&connection, RecordKind::FindingCandidate),
        keys,
        "补充字段不得改变逻辑身份：{:?}",
        codes(&summary)
    );
    assert_eq!(
        table_count(&connection, "import_record_revisions"),
        revisions + 1,
        "§9.9：合并结果变了就必须是新修订，否则历史被覆盖"
    );
    assert_eq!(
        crate::artifact_import::canonical::sha256_hex(&fs::read(&primary).unwrap()),
        primary_hash,
        "主 SARIF 必须一字未动"
    );
    let merged = &current_envelopes(&connection, RecordKind::FindingCandidate)[0];
    assert!(
        scalar_text(merged, "fingerprints").contains("abc123"),
        "当前投影必须是合并后的最新修订：{merged}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn conflicting_field_values_keep_every_original() {
    let root = sandbox("imp006");
    let bundle = source_dir(&root).join("bundle");
    // Both reports name the same class *and* the same endpoint: two corroborating
    // dimensions is what §9.7 requires before two rows may be treated as one
    // finding (a lone shared endpoint stays two findings, see §IMP-004).
    write_json(
        &bundle,
        "primary.sarif",
        &sarif_document(vec![sarif_result("v1", "warning", "同一问题", &[], Some(serde_json::json!({"severity":"high","cwe":"CWE-89","endpoint":"/api/x","method":"GET"})))]),
    );
    write_json(
        &bundle,
        "findings.sarif",
        // A preceding coverage row makes the supplemental finding pointer later.
        &sarif_document(vec![sarif_result("COVERAGE", "none", "coverage", &[], None), sarif_result(
            "RULE-9",
            "warning",
            "同一问题",
            &[("src/x.py", 1)],
            Some(serde_json::json!({"cwe": "CWE-89", "endpoint": "/api/x"})),
        )]),
    );
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    import_at(&connection, &root);
    let envelopes = current_envelopes(&connection, RecordKind::FindingCandidate);
    let merged = envelope_where(&envelopes, "endpoint", "/api/x")
        .cloned()
        .expect("合并记录");
    assert_eq!(
        scalar_text(&merged, "severity"),
        "high",
        "同一适配器按记录位置确定优先级"
    );
    let conflicts = merged
        .get("conflicts")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(
        conflicts.iter().any(|row| {
            row.get("field").and_then(serde_json::Value::as_str) == Some("severity")
                && row.get("candidate").and_then(serde_json::Value::as_str) == Some("medium")
        }),
        "被覆盖的原值必须留在冲突列表里：{conflicts:?}"
    );
    let _ = fs::remove_dir_all(root);
}
