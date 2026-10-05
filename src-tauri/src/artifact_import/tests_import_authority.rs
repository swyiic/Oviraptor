// §9.2/§9.8 — Historical review claims and coverage never grant authority.

#[test]
fn review_fields_and_history_are_not_dropped() {
    let root = sandbox("imp007");
    let bundle = source_dir(&root).join("review-report");
    let properties = serde_json::json!({
        "counterevidence":"contradicting evidence", "confidence_rationale":"reported rationale",
        "severity_change_conditions":"reported conditions", "fix_verification":"reported verification",
        "update_history":[{"severity":"medium"},{"severity":"high"}],
        "executionEligible":true, "reviewState":"confirmed"
    });
    write_json(&bundle, "findings.sarif", &sarif_document(vec![sarif_result(
        "REVIEW", "warning", "reported review", &[], Some(properties.clone()),
    )]));
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    import_at(&connection, &root);
    let envelopes = current_envelopes(&connection, RecordKind::FindingCandidate);
    let record = envelope_where(&envelopes, "rule_id", "REVIEW")
        .cloned()
        .expect("reported SARIF finding");
    let payload = &record["extensions"]["sarif_result"]["properties"];
    assert_eq!(payload, &properties, "reported properties are preserved as inert data");
    assert_eq!(record["claim"]["executionEligible"], false);
    assert_eq!(record["claim"]["reviewState"], "unreviewed");
    for field in [
        "counterevidence",
        "confidence_rationale",
        "severity_change_conditions",
        "fix_verification",
        "update_history",
    ] {
        assert!(
            payload.get(field).is_some(),
            "§IMP-007 要求 {field} 不丢失：{payload}"
        );
    }
    assert_eq!(
        payload
            .get("update_history")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(2)
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn pass_open_not_applicable_and_coverage_never_become_findings() {
    let root = sandbox("imp010");
    let bundle = source_dir(&root).join("bundle");
    write_json(
        &bundle,
        "findings.sarif",
        &sarif_document([
            ("RULE-P", "warning", "pass"),
            ("RULE-N", "warning", "notApplicable"),
            ("RULE-O", "warning", "open"),
            ("RULE-COV", "none", "fail"),
            ("RULE-GAP", "note", "informational"),
        ].into_iter().map(|(rule, level, kind)| {
            let mut result = sarif_result(rule, level, "标准非发现记录", &[], None);
            result["kind"] = serde_json::json!(kind);
            result
        }).collect()),
    );
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    import_at(&connection, &root);
    assert_eq!(
        current_envelopes(&connection, RecordKind::FindingCandidate).len(),
        0,
        "§9.8：pass/open/notApplicable/level=none/coverage 结果都不能成为漏洞"
    );
    let coverage = current_envelopes(&connection, RecordKind::Coverage);
    assert_eq!(coverage.len(), 5, "每一条都应作为覆盖记录保留");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn historical_claims_are_unreviewed_read_only_and_inert() {
    let root = sandbox("imp018");
    let bundle = copy_fixture_bundle(&root, "producer_1_5_3");
    write_file(&bundle, "llm-hook.jsonl", "{\"kind\":\"model_call_started\",\"requestId\":\"inert-audit\"}\n");
    let mut current_report = sarif_document(vec![]);
    current_report["runs"][0]["properties"] = serde_json::json!({"reportLabel":"current run report"});
    write_json(&bundle, "current.sarif", &current_report);
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    import_at(&connection, &root);
    let kinds = [
        RecordKind::FindingCandidate,
        RecordKind::Coverage,
        RecordKind::RunState,
        RecordKind::EventTrace,
        RecordKind::EvidenceNote,
    ];
    assert!(current_envelopes(&connection, RecordKind::Usage).is_empty());
    for kind in kinds {
        let envelopes = current_envelopes(&connection, kind);
        assert!(!envelopes.is_empty(), "每一类记录都应被检查：{kind:?}");
        for envelope in envelopes {
            let claim = envelope.get("claim").cloned().unwrap_or_default();
            assert_eq!(
                claim.get("authority").and_then(serde_json::Value::as_str),
                Some("historical_external")
            );
            assert_eq!(
                claim.get("reviewState").and_then(serde_json::Value::as_str),
                Some("unreviewed")
            );
            assert_eq!(
                claim
                    .get("executionEligible")
                    .and_then(serde_json::Value::as_bool),
                Some(false)
            );
            assert_eq!(
                claim.get("readOnly").and_then(serde_json::Value::as_bool),
                Some(true)
            );
        }
    }
    assert_eq!(
        table_count(&connection, "sentinel_findings"),
        0,
        "§2.1：历史记录不得成为活动结果"
    );
    assert_eq!(
        table_count(&connection, "agent_runs"),
        0,
        "§2.1：历史记录不得注册成 Agent run"
    );
    let _ = fs::remove_dir_all(root);
}
