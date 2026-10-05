#[test]
fn legacy_claims_remain_visible_but_do_not_become_native_reviewer_confirmations() {
    let root = std::env::temp_dir().join(format!("oviraptor-overview-provenance-{}", Uuid::new_v4()));
    let db_path = db::initialize(&root).unwrap();
    let connection = db::open(&db_path).unwrap();
    let index: String = connection.query_row(
        "SELECT name FROM sqlite_master WHERE type='index' AND name='idx_agent_finding_candidates_projection'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(index, "idx_agent_finding_candidates_projection");
    connection.execute("INSERT INTO projects(id,name) VALUES(1,'reviewed'),(2,'other')", []).unwrap();
    connection.execute("INSERT INTO sentinel_scans(id,project_id,status,attempt_count) VALUES('provenance',1,'completed',1),('outside',2,'completed',1)", []).unwrap();
    connection.execute("INSERT INTO sentinel_scan_attempts(scan_id,attempt_number,status) VALUES('provenance',1,'completed'),('outside',1,'completed')", []).unwrap();
    connection.execute("INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,severity) VALUES \
        ('provenance','https://example.test','strix','vulnerability','legacy','critical'), \
        ('provenance','https://example.test','agent','vulnerability','native','high'), \
        ('provenance','https://example.test','agent','vulnerability','unreviewed','critical'), \
        ('outside','https://other.test','strix','vulnerability','outside','high')", []).unwrap();
    // A manual verdict and a candidate's status alone are not Native Reviewer decisions.
    connection.execute("INSERT INTO sentinel_validations(scan_id,url,finding_key,finding_kind,verdict,severity) VALUES \
        ('provenance','https://example.test','strix:vulnerability:legacy','vulnerability','confirmed_issue','critical')", []).unwrap();
    connection.execute("INSERT INTO agent_runs(id,scan_id,role,status,terminal_state,root_run_id,lane,parent_run_id) VALUES \
        ('root-provenance','provenance','coordinator','terminal','completed','','',NULL), \
        ('reviewer-provenance','provenance','evidence_reviewer','terminal','completed','root-provenance','review','root-provenance')", []).unwrap();
    connection.execute("INSERT INTO agent_assignments(id,coordinator_run_id,child_run_id,role,lane,state,dedup_key) VALUES \
        ('review-assignment','root-provenance','reviewer-provenance','evidence_reviewer','review','completed','review-once')", []).unwrap();
    connection.execute("INSERT INTO agent_finding_candidates(id,root_run_id,scan_id,target_url,stage,kind,record_key,severity,status,candidate_revision,reviewer_run_id,published_at) VALUES \
        ('native-candidate','root-provenance','provenance','https://example.test','agent','vulnerability','native','high','published',1,'reviewer-provenance','2026-09-26'), \
        ('fake-candidate','root-provenance','provenance','https://example.test','agent','vulnerability','unreviewed','critical','published',1,'reviewer-provenance','2026-09-26')", []).unwrap();
    connection.execute("INSERT INTO agent_review_decisions(root_run_id,candidate_id,candidate_revision,reviewer_run_id,verdict) VALUES \
        ('root-provenance','native-candidate',1,'reviewer-provenance','confirmed')", []).unwrap();
    connection.execute("INSERT INTO agent_review_requests(id,root_run_id,assignment_id,reviewer_run_id,candidate_id,candidate_revision,status,decision_id,lease_epoch,fencing_token) \
        SELECT 'review-request','root-provenance','review-assignment','reviewer-provenance','native-candidate',1,'confirmed',d.id,1,'test-token' \
        FROM agent_review_decisions d WHERE d.candidate_id='native-candidate'", []).unwrap();
    connection.execute("INSERT INTO agent_review_decisions(root_run_id,candidate_id,candidate_revision,reviewer_run_id,verdict) VALUES \
        ('root-provenance','fake-candidate',1,'reviewer-provenance','rejected')", []).unwrap();
    connection.execute("INSERT INTO agent_review_requests(id,root_run_id,assignment_id,reviewer_run_id,candidate_id,candidate_revision,status,decision_id,lease_epoch,fencing_token) \
        SELECT 'rejected-request','root-provenance','review-assignment','reviewer-provenance','fake-candidate',1,'rejected',d.id,1,'test-token' \
        FROM agent_review_decisions d WHERE d.candidate_id='fake-candidate'", []).unwrap();

    let stats = sentinel_overview_stats_in(&connection, Some(1)).unwrap();
    assert_eq!(stats.vulnerability_count, 3, "historical records remain queryable");
    assert_eq!(stats.high_risk_count, 3, "legacy total remains backwards-compatible");
    assert_eq!(stats.reviewer_confirmed_count, 1);
    assert_eq!(stats.reviewer_high_risk_count, 1);
    assert_eq!(stats.other_vulnerability_count, 2);
    assert_eq!(sentinel_overview_stats_in(&connection, None).unwrap().other_vulnerability_count, 3);
    for fault in [
        "UPDATE sentinel_scans SET attempt_count=2 WHERE id='provenance'",
        "INSERT INTO sentinel_scan_attempts(scan_id,attempt_number,status) VALUES('provenance',2,'scanning')",
        "DELETE FROM sentinel_scan_attempts WHERE scan_id='provenance'",
        "UPDATE agent_runs SET attempt_number=2 WHERE id='root-provenance'",
        "UPDATE agent_runs SET attempt_number=2 WHERE id='reviewer-provenance'",
        "UPDATE agent_runs SET scan_id='outside' WHERE id='reviewer-provenance'",
        "UPDATE agent_runs SET parent_run_id=NULL WHERE id='reviewer-provenance'",
        "INSERT INTO sentinel_deleted_scans(scan_id) VALUES('provenance')",
    ] {
        connection.execute_batch("SAVEPOINT overview_web_fault").unwrap();
        connection.execute_batch(fault).unwrap();
        assert_eq!(sentinel_overview_stats_in(&connection, Some(1)).unwrap().reviewer_confirmed_count,0,"{fault}");
        connection.execute_batch("ROLLBACK TO overview_web_fault; RELEASE overview_web_fault").unwrap();
    }
    connection.execute("UPDATE sentinel_findings SET record_json='{\"changed\":true}' WHERE scan_id='provenance' AND record_key='native'", []).unwrap();
    assert_eq!(sentinel_overview_stats_in(&connection, Some(1)).unwrap().reviewer_confirmed_count, 0,
        "a changed projection must not borrow an older confirmation");
    let historical: i64 = connection.query_row("SELECT COUNT(*) FROM sentinel_findings WHERE scan_id='provenance' AND stage='strix' AND record_key='legacy'", [], |row| row.get(0)).unwrap();
    assert_eq!(historical, 1);
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}
