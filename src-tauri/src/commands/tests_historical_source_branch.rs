// Loop2 (REM-009): retired source-specific branch must not mint a
// privileged source type. Old `stage == "strix"` rows without code or
// runtime evidence fall back to the neutral scanner type, exactly like any
// other unknown stage. `ai_validation` stays reserved for human repeater
// validations (save_investigation_validation).
#[test]
fn retired_stage_does_not_mint_a_privileged_source_type() {
    let root = std::env::temp_dir().join(format!("oviraptor-historical-branch-{}", Uuid::new_v4()));
    let db_path = db::initialize(&root).unwrap();
    let connection = db::open(&db_path).unwrap();
    connection
        .execute("INSERT INTO projects(id,name) VALUES(77,'branch')", [])
        .unwrap();
    connection
        .execute(
            "INSERT INTO sentinel_scans(id,project_id,scan_type) VALUES('branch-scan',77,'code')",
            [],
        )
        .unwrap();
    // No file, no runtime URL, no engine hint: forces the empty-source
    // fallback branch in appsec_candidates.
    for (key, stage) in [("old-1", "strix"), ("other-1", "nightly")] {
        connection
                .execute(
                    "INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title,severity,record_json) VALUES('branch-scan','/repo',?1,'vulnerability',?2,'Legacy finding','high','{}')",
                    [stage, key],
                )
                .unwrap();
    }
    let candidates = appsec_candidates(&connection, "branch-scan").unwrap();
    assert_eq!(candidates.len(), 2, "both rows must surface as candidates");
    for candidate in &candidates {
        assert_eq!(
            candidate.source_types,
            vec!["scanner".to_string()],
            "retired stage must use the neutral fallback, not a privileged type: {}",
            candidate.source_key
        );
        assert!(
            !candidate
                .source_types
                .contains(&"ai_validation".to_string()),
            "historical rows must not occupy the human-validation source type"
        );
    }
    drop(connection);
    let _ = fs::remove_dir_all(root);
}
