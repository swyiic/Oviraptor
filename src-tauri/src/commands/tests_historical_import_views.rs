// Reuse the real importer and native-state snapshot fixture, not a parallel schema.
#[test]
fn historical_import_views_retirement_uses_only_exact_task_and_deletion_ids() {
    let (root, state, connection) = bundle_import_fixture();
    let content = json!({"format":"oviraptor-sentinel-v1", "scan":{"id":"live"},
        "findings":[{"recordKey":"view-record", "title":"Current import"}]}).to_string();
    import_sentinel_results_content(&state.db_path, &state.app_data_dir, &content).unwrap();
    let history = historical_import_runs(&connection, None).unwrap().remove(0);
    connection.execute_batch(
        "UPDATE sentinel_scans SET task_path=(SELECT source_path FROM import_projection_memberships LIMIT 1) WHERE id='live';
         INSERT INTO sentinel_scans(id,task_path) SELECT 'strix-live',task_path FROM sentinel_scans WHERE id='live';
         INSERT INTO sentinel_deleted_scans(scan_id) VALUES('strix-live');"
    ).unwrap();
    let before = bundle_native_snapshot(&connection);
    assert_eq!(historical_import_previews(&connection, "live").unwrap().len(), 2);
    assert_eq!(historical_import_runs(&connection, None).unwrap().len(), 1);
    assert_eq!(historical_bundle_previews(&connection, &history.bundle_id, history.row_id).unwrap().len(), 2);
    assert_eq!(bundle_native_snapshot(&connection), before);
    connection.execute("DELETE FROM sentinel_deleted_scans WHERE scan_id='strix-live'", []).unwrap();
    let before = bundle_native_snapshot(&connection);
    assert!(historical_import_previews(&connection, "strix-live").unwrap().is_empty(),
        "same source directory cannot establish an alias identity");
    assert_eq!(bundle_native_snapshot(&connection), before);
    connection.execute("INSERT INTO sentinel_deleted_scans(scan_id) VALUES('live')", []).unwrap();
    let before = bundle_native_snapshot(&connection);
    assert!(historical_import_previews(&connection, "live").unwrap().is_empty());
    assert!(historical_import_runs(&connection, None).unwrap().is_empty());
    assert!(historical_bundle_previews(&connection, &history.bundle_id, history.row_id).unwrap().is_empty());
    assert_eq!(bundle_native_snapshot(&connection), before);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn historical_import_views_exclude_invalid_candidates_without_hiding_valid_run_records() {
    let (root, state, connection) = bundle_import_fixture();
    let content = json!({"format":"oviraptor-sentinel-v1", "scan":{"id":"live"},
        "findings":[{"recordKey":"view-record", "title":"Unsupported candidate"}]})
    .to_string();
    import_sentinel_results_content(&state.db_path, &state.app_data_dir, &content).unwrap();
    connection.execute_batch(
        "INSERT INTO import_record_revisions(record_key,logical_key,record_kind,revision_hash,adapter,envelope_json) \
         SELECT record_key,logical_key,record_kind,'view-mixed',adapter, \
             json_set(envelope_json,'$.canonicalSchema','unsupported-schema') \
         FROM import_record_revisions WHERE record_kind='finding_candidate'; \
         UPDATE import_projection_memberships SET revision_id=(SELECT MAX(r.id) \
             FROM import_record_revisions r WHERE r.record_key=import_projection_memberships.record_key);"
    ).unwrap();
    let histories = historical_import_runs(&connection, None).unwrap();
    assert_eq!(histories.len(), 1);
    let history = &histories[0];
    assert_eq!(history.finding_candidates, 0);
    for rows in [
        historical_import_previews(&connection, "live").unwrap(),
        historical_bundle_previews(&connection, &history.bundle_id, history.row_id).unwrap(),
    ] {
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, "run_state");
    }
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn historical_import_views_reject_unsupported_schema_and_nonboolean_claims() {
    let (root, state, connection) = bundle_import_fixture();
    let content = json!({"format":"oviraptor-sentinel-v1", "scan":{"id":"live"},
        "findings":[{"recordKey":"view-record", "title":"Public history"}]})
    .to_string();
    import_sentinel_results_content(&state.db_path, &state.app_data_dir, &content).unwrap();
    let history = historical_import_runs(&connection, None).unwrap().remove(0);
    assert_eq!(
        historical_import_previews(&connection, "live")
            .unwrap()
            .len(),
        2
    );
    let before = bundle_native_snapshot(&connection);
    let mut failures = Vec::new();
    for change in [
        "'$.canonicalSchema','unsupported-schema'",
        "'$.canonicalSchema',NULL",
        "'$.claim.readOnly',1",
        "'$.claim.executionEligible',0",
        "'$.claim.readOnly','true'",
        "'$.claim.executionEligible','false'",
        "'$.claim.authority','native_ledger'",
        "'$.claim.reviewState','confirmed'",
        "'$.claim.readOnly',json('false')",
        "'$.claim.executionEligible',json('true')",
    ] {
        connection
            .execute_batch("SAVEPOINT historical_view_claim")
            .unwrap();
        // Never rewrite immutable history. Point the test projection at a new,
        // deliberately unsupported revision to exercise each actual SQL reader.
        connection.execute_batch(&format!(
            "INSERT INTO import_record_revisions(record_key,logical_key,record_kind,revision_hash,adapter,envelope_json) \
             SELECT record_key,logical_key,record_kind,'view-invalid',adapter,json_set(envelope_json,{change}) \
             FROM import_record_revisions WHERE id IN (SELECT revision_id FROM import_projection_memberships); \
             UPDATE import_projection_memberships SET revision_id=(SELECT MAX(r.id) \
                 FROM import_record_revisions r WHERE r.record_key=import_projection_memberships.record_key);"
        )).unwrap();
        for (reader, count) in [
            (
                "task",
                historical_import_previews(&connection, "live")
                    .unwrap()
                    .len(),
            ),
            (
                "catalog",
                historical_import_runs(&connection, None).unwrap().len(),
            ),
            (
                "bundle",
                historical_bundle_previews(&connection, &history.bundle_id, history.row_id)
                    .unwrap()
                    .len(),
            ),
        ] {
            if count != 0 {
                failures.push(format!("{reader} accepted {change}"));
            }
        }
        connection
            .execute_batch("ROLLBACK TO historical_view_claim; RELEASE historical_view_claim")
            .unwrap();
    }
    assert_eq!(bundle_native_snapshot(&connection), before);
    assert_eq!(
        historical_import_previews(&connection, "live")
            .unwrap()
            .len(),
        2
    );
    drop(connection);
    fs::remove_dir_all(root).unwrap();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn historical_import_views_valid_claims_are_read_only_on_every_surface() {
    let (root, state, connection) = bundle_import_fixture();
    let content = json!({"format":"oviraptor-sentinel-v1", "scan":{"id":"live"},
        "findings":[{"recordKey":"view-record", "title":"Public history"}]})
    .to_string();
    import_sentinel_results_content(&state.db_path, &state.app_data_dir, &content).unwrap();
    let before = bundle_native_snapshot(&connection);
    let revision_snapshot = || {
        connection
            .prepare("SELECT envelope_json FROM import_record_revisions ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    let originals = revision_snapshot();
    let readonly = rusqlite::Connection::open_with_flags(
        &state.db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let history = historical_import_runs(&readonly, None).unwrap().remove(0);
    let task = historical_import_previews(&readonly, "live").unwrap();
    let bundle = historical_bundle_previews(&readonly, &history.bundle_id, history.row_id).unwrap();
    for rows in [&task, &bundle] {
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|row| row.title == "Public history"));
        assert!(rows.iter().all(|row| row.read_only
            && !row.execution_eligible
            && row.review_state == "unreviewed"));
    }
    assert_eq!(bundle_native_snapshot(&connection), before);
    assert_eq!(revision_snapshot(), originals);
    drop(readonly);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}
