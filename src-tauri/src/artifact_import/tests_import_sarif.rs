// §13.2 — SARIF locations, code flows and historical result semantics.

#[test]
fn sarif_handles_zero_one_and_many_locations() {
    let root = sandbox("imp008");
    let bundle = source_dir(&root).join("bundle");
    write_json(
        &bundle,
        "findings.sarif",
        &sarif_document(vec![
            sarif_result("ZERO", "error", "无位置", &[], None),
            sarif_result("ONE", "error", "一个位置", &[("a.py", 1)], None),
            sarif_result(
                "MANY",
                "error",
                "多个位置",
                &[("b.py", 2), ("c.py", 7), ("d.py", 9)],
                None,
            ),
        ]),
    );
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    import_at(&connection, &root);
    let envelopes = current_envelopes(&connection, RecordKind::FindingCandidate);
    for (rule, expected) in [("ZERO", 0), ("ONE", 1), ("MANY", 3)] {
        let record = envelope_where(&envelopes, "rule_id", rule)
            .cloned()
            .unwrap_or_else(|| panic!("{rule} 没有生成记录"));
        assert_eq!(
            payload_of(&record)
                .get("locations")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len),
            Some(expected),
            "{rule} 的位置数量不对"
        );
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn sarif_standard_result_kinds_do_not_promote_non_failures() {
    let root = sandbox("sarif-standard-kinds");
    let bundle = source_dir(&root).join("bundle");
    let mut results = Vec::new();
    for (index, kind) in ["pass", "notApplicable", "open", "review", "informational", "fail"].iter().enumerate() {
        let mut row = sarif_result(&format!("KIND-{index}"), "warning", kind, &[],
            Some(serde_json::json!({"kind":"fail"})));
        row["kind"] = serde_json::json!(kind);
        results.push(row);
    }
    for (index,kind) in [serde_json::Value::Null,serde_json::json!(7),serde_json::json!("unknown")].into_iter().enumerate() {
        let mut row=sarif_result(&format!("INVALID-{index}"),"warning","invalid kind",&[],None);
        row["kind"]=kind;
        results.push(row);
    }
    write_json(&bundle, "findings.sarif", &sarif_document(results));
    let connection = open_connection(&initialize_db(&root));
    import_at(&connection, &root);
    let findings = current_envelopes(&connection, RecordKind::FindingCandidate);
    assert_eq!(findings.len(), 1, "only kind=fail is a finding, even with a conflicting legacy extension");
    assert_eq!(scalar_text(&findings[0], "rule_id"), "KIND-5");
    assert_eq!(current_envelopes(&connection, RecordKind::Coverage).len(), 8);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn sarif_standard_code_flows_preserve_all_paths_and_extensions() {
    let root = sandbox("sarif-standard-flows");
    let bundle = source_dir(&root).join("bundle");
    let flows = serde_json::json!([
        {"message":{"text":"first"},"threadFlows":[{"locations":[{"location":{"physicalLocation":{"artifactLocation":{"uri":"a.rs"},"region":{"startLine":1}}}}]}]},
        {"message":{"text":"second"},"threadFlows":[{"locations":[{"location":{"physicalLocation":{"artifactLocation":{"uri":"b.rs"},"region":{"startLine":2}}}}]}]}
    ]);
    let mut row = sarif_result("FLOW", "error", "standard paths", &[("a.rs", 1), ("b.rs", 2)], None);
    row["codeFlows"] = flows.clone();
    row["codeFlow"] = serde_json::json!({"legacy":true});
    row["properties"] = serde_json::json!({"vendor":{"retained":true}});
    row["relatedLocations"] = serde_json::json!([{"id":7,"message":{"text":"context"}}]);
    write_json(&bundle, "findings.sarif", &sarif_document(vec![row.clone()]));
    let connection = open_connection(&initialize_db(&root));
    import_at(&connection, &root);
    let findings = current_envelopes(&connection, RecordKind::FindingCandidate);
    assert_eq!(findings.len(), 1);
    let payload = payload_of(&findings[0]);
    assert_eq!(payload["code_flows"], flows, "standard plural codeFlows must win over legacy spelling");
    assert_eq!(findings[0].pointer("/extensions/sarif_result"), Some(&row), "lossless historical result, not an execution grant");
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn sarif_standard_severity_and_empty_run_metadata_remain_historical() {
    let root=sandbox("sarif-standard-severity");
    let bundle=source_dir(&root).join("bundle");
    let severities=["critical","high","medium","low","informational","unknown"];
    let rows=severities.iter().enumerate().map(|(index,severity)|sarif_result(
        &format!("SEV-{index}"),"warning","reported finding",&[],Some(serde_json::json!({"severity":severity}))))
        .collect::<Vec<_>>();
    let mut document=sarif_document(rows);
    let properties=serde_json::json!({"oviraptorSourceReview":{"executionEligible":true,"coverageReviewCompleted":true,"review":{"status":"audited","counts":{"confirmed":0}}}});
    document["runs"].as_array_mut().unwrap().push(serde_json::json!({"tool":{"driver":{"name":"empty-export"}},"properties":properties,"results":[]}));
    write_json(&bundle,"findings.sarif",&document);
    let connection=open_connection(&initialize_db(&root));
    import_at(&connection,&root);
    let findings=current_envelopes(&connection,RecordKind::FindingCandidate);
    assert_eq!(findings.len(),6);
    for (index,severity) in severities.iter().enumerate() {
        let row=envelope_where(&findings,"rule_id",&format!("SEV-{index}")).unwrap();
        assert_eq!(scalar_text(row,"severity"),if *severity=="unknown" {"medium"} else {severity});
        assert_eq!(scalar_text(row,"title"),"reported finding");
    }
    let runs=current_envelopes(&connection,RecordKind::RunState);
    assert_eq!(runs.len(),1,"zero-result report remains inspectable");
    assert_eq!(runs[0]["extensions"]["sarif_run_properties"],properties);
    assert_eq!(scalar_text(&runs[0],"status"),"imported");
    assert_eq!(table_count(&connection,"agent_runs"),0);
    assert_eq!(table_count(&connection,"agent_source_review_decisions"),0);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn sarif_code_flow_and_thread_flow_are_preserved() {
    let root = sandbox("imp009");
    copy_fixture_bundle(&root, "producer_1_5_3");
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    import_at(&connection, &root);
    let envelopes = current_envelopes(&connection, RecordKind::FindingCandidate);
    let record = envelope_where(&envelopes, "rule_id", "RULE-1")
        .cloned()
        .expect("SARIF 的 RULE-1 记录");
    let flows = payload_of(&record)
        .get("code_flows")
        .and_then(|value| value.get("threadFlows"))
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert_eq!(flows.len(), 1, "codeFlow/threadFlow 必须原样保留");
    assert_eq!(
        flows[0]
            .get("locations")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(2)
    );
    assert_eq!(
        payload_of(&record)
            .get("locations")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(2),
        "多 location 不能只留第一个"
    );
    let _ = fs::remove_dir_all(root);
}
