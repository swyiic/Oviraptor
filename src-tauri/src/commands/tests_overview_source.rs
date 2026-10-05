#[test]
fn overview_source_counts_audited_current_decisions_without_subtracting_them_from_legacy_rows() {
    let (root,connection,lease,result)=source_reviewer_execution_fixture("all_confirmed",None);
    let report=result.unwrap();
    let confirmed=report["sourceDecisionProjection"]["counts"]["confirmed"].as_i64().unwrap();
    assert!(confirmed>1,"fixture exercises whole-set counting, not a single page");
    let before=bundle_native_snapshot(&connection);
    let stats=sentinel_overview_stats_in(&connection,Some(1)).unwrap();
    assert_eq!(stats.reviewer_confirmed_count,confirmed);
    assert_eq!(stats.reviewer_high_risk_count,confirmed);
    assert_eq!(stats.other_vulnerability_count,stats.vulnerability_count,"source decisions are not rows in the legacy finding table");
    let stats=serde_json::to_value(stats).unwrap();
    assert_eq!(stats["sourceReviewerConfirmedCount"],confirmed);
    assert_eq!(stats["sourceReviewAuditedTaskCount"],1);
    assert_eq!(stats["sourceReviewUnavailableTaskCount"],0);
    assert_eq!(stats["sourceReviewUnverifiedTaskCount"],0);
    assert_eq!(bundle_native_snapshot(&connection),before);
    let other=serde_json::to_value(sentinel_overview_stats_in(&connection,Some(2)).unwrap()).unwrap();
    assert_eq!(other["reviewerConfirmedCount"],0);
    assert_eq!(other["sourceReviewAuditedTaskCount"],0);
    assert_eq!(serde_json::to_value(sentinel_overview_stats_in(&connection,None).unwrap()).unwrap(),stats);
    connection.execute("INSERT INTO sentinel_deleted_scans(scan_id) VALUES(?1)",[&lease.scan_id]).unwrap();
    let before=bundle_native_snapshot(&connection);
    let hidden=sentinel_overview_stats_in(&connection,None).unwrap();
    assert_eq!(hidden.reviewer_confirmed_count,0);assert_eq!(hidden.task_count,0);
    assert_eq!(bundle_native_snapshot(&connection),before);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn overview_source_does_not_borrow_old_attempt_confirmation_or_treat_unreviewed_as_clean() {
    let (root,connection,lease,result)=source_reviewer_execution_fixture("confirmed",None);
    result.unwrap();
    assert_eq!(sentinel_overview_stats_in(&connection,None).unwrap().reviewer_confirmed_count,1);
    connection.execute("UPDATE sentinel_scans SET attempt_count=2,status='scanning' WHERE id=?1",[&lease.scan_id]).unwrap();
    connection.execute("INSERT INTO sentinel_scan_attempts(scan_id,attempt_number,status) VALUES(?1,2,'scanning')",[&lease.scan_id]).unwrap();
    let before=bundle_native_snapshot(&connection);
    let stats=serde_json::to_value(sentinel_overview_stats_in(&connection,None).unwrap()).unwrap();
    assert_eq!(stats["reviewerConfirmedCount"],0);
    assert_eq!(stats["sourceReviewAuditedTaskCount"],0);
    assert_eq!(stats["sourceReviewUnavailableTaskCount"],1);
    assert_eq!(bundle_native_snapshot(&connection),before);
    assert_eq!(native_source_findings(&connection,&lease.scan_id,1,0,50).unwrap()["counts"]["confirmed"],1,"old attempt remains readable in its own scope");
    connection.execute("UPDATE sentinel_scans SET attempt_count=1 WHERE id=?1",[&lease.scan_id]).unwrap();
    let stats=serde_json::to_value(sentinel_overview_stats_in(&connection,None).unwrap()).unwrap();
    assert_eq!(stats["reviewerConfirmedCount"],0,"contradictory current attempt must not choose an old review");
    assert_eq!(stats["sourceReviewUnverifiedTaskCount"],1);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn overview_source_reaudits_entire_delivery_and_keeps_corrupt_tasks_visible_as_unverified() {
    let (root,connection,lease,result)=source_reviewer_execution_fixture("all_confirmed",None);
    let expected=result.unwrap()["sourceDecisionProjection"]["counts"]["confirmed"].as_i64().unwrap();
    for sql in [
        "UPDATE agent_source_review_decisions SET record_json='{}' WHERE id=(SELECT id FROM agent_source_review_decisions ORDER BY id DESC LIMIT 1)",
        "UPDATE agent_messages SET acknowledged_at='' WHERE kind='source_review_result'",
        "UPDATE agent_specialist_calls SET response_json='{}' WHERE role='evidence_reviewer'",
        "DELETE FROM agent_coordinator_leases",
    ] {
        connection.execute_batch("SAVEPOINT overview_corruption; DROP TRIGGER source_decision_no_update; DROP TRIGGER agent_specialist_call_immutable").unwrap();
        connection.execute_batch(sql).unwrap();
        let before=bundle_native_snapshot(&connection);
        let stats=serde_json::to_value(sentinel_overview_stats_in(&connection,None).unwrap()).unwrap();
        assert_eq!(stats["reviewerConfirmedCount"],0,"{sql}");
        assert_eq!(stats["sourceReviewAuditedTaskCount"],0,"{sql}");
        assert_eq!(stats["sourceReviewUnverifiedTaskCount"],1,"{sql}");
        assert_eq!(bundle_native_snapshot(&connection),before);
        connection.execute_batch("ROLLBACK TO overview_corruption; RELEASE overview_corruption").unwrap();
    }
    assert_eq!(sentinel_overview_stats_in(&connection,None).unwrap().reviewer_confirmed_count,expected);
    let (view,_,_)=crate::agent_runtime::multi_agent::source::historical_materials(&connection,&lease.scan_id,1).unwrap();
    fs::rename(view.repository().join("app.py"),root.join("retained-overview-source")).unwrap();
    let stats=serde_json::to_value(sentinel_overview_stats_in(&connection,None).unwrap()).unwrap();
    assert_eq!(stats["reviewerConfirmedCount"],0);
    assert_eq!(stats["sourceReviewUnverifiedTaskCount"],1,"missing frozen bytes cannot use cached confirmation");
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn overview_source_legacy_claims_and_tombstoned_rows_never_qualify() {
    let (root,state,connection)=bundle_import_fixture();
    connection.execute_batch("UPDATE sentinel_scans SET scan_type='code' WHERE id='live';
        INSERT INTO sentinel_findings(scan_id,stage,kind,record_key,severity,record_json) VALUES
        ('live','source-review','vulnerability','pretend','critical','{\"reviewState\":\"confirmed\",\"sourceDecisionId\":\"fake\"}');").unwrap();
    let stats=serde_json::to_value(sentinel_overview_stats_in(&connection,None).unwrap()).unwrap();
    assert_eq!(stats["reviewerConfirmedCount"],0);
    assert_eq!(stats["otherVulnerabilityCount"],1);
    assert_eq!(stats["sourceReviewUnavailableTaskCount"],1);
    connection.execute_batch("INSERT INTO sentinel_targets(project_id,scan_id,url) VALUES(1,'live','https://example.test');
        INSERT INTO sentinel_fuse_zone(project_id,url,normalized_url,source_scan_id) VALUES(1,'https://example.test','https://example.test','live');
        INSERT INTO sentinel_opportunities(project_id,scan_id,opportunity_key,status,score) VALUES(1,'live','one','ready',90);
        INSERT INTO sentinel_validations(scan_id,url,finding_key,finding_kind,verdict) VALUES('live','','source-review:vulnerability:pretend','vulnerability','confirmed_issue');
        INSERT INTO sentinel_findings(scan_id,stage,kind,record_key) VALUES('live','native','fingerprint','fp'),('live','native','api','api'),('live','native','endpoint','endpoint');").unwrap();
    let before=sentinel_overview_stats_in(&connection,None).unwrap();
    assert_eq!((before.url_count,before.active_fuse_count,before.opportunity_count,before.ready_opportunity_count),(1,1,1,1));
    connection.execute_batch("INSERT INTO sentinel_deleted_scans(scan_id) VALUES('live')").unwrap();
    let native_before=bundle_native_snapshot(&connection);
    let stats=sentinel_overview_stats_in(&connection,None).unwrap();
    assert_eq!(stats.vulnerability_count,0);assert_eq!(stats.high_risk_count,0);
    assert_eq!(stats.other_vulnerability_count,0);assert_eq!(stats.task_count,0);
    assert_eq!(stats.pending_vulnerability_count,0);
    assert_eq!((stats.url_count,stats.active_fuse_count,stats.opportunity_count,stats.ready_opportunity_count),(0,0,0,0));
    assert_eq!((stats.validated_count,stats.fingerprint_count,stats.api_count,stats.endpoint_count),(0,0,0,0));
    assert_eq!(bundle_native_snapshot(&connection),native_before);
    drop(state);drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn overview_source_audited_zero_is_not_unavailable_and_readonly_callers_keep_their_snapshot() {
    let (root,connection,lease,result)=source_reviewer_execution_fixture("valid",None);
    assert_eq!(result.unwrap()["sourceDecisionProjection"]["counts"]["confirmed"],0);
    let reader=rusqlite::Connection::open_with_flags(connection.path().unwrap(),rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let before=bundle_native_snapshot(&connection);
    let stats=serde_json::to_value(sentinel_overview_stats_in(&reader,None).unwrap()).unwrap();
    assert_eq!(stats["reviewerConfirmedCount"],0);
    assert_eq!(stats["sourceReviewAuditedTaskCount"],1);
    assert_eq!(stats["sourceReviewUnavailableTaskCount"],0);
    assert_eq!(stats["sourceReviewUnverifiedTaskCount"],0);
    reader.execute_batch("BEGIN").unwrap();
    assert_eq!(serde_json::to_value(sentinel_overview_stats_in(&reader,None).unwrap()).unwrap(),stats);
    assert!(!reader.is_autocommit(),"do not commit or rollback the caller's read transaction");
    reader.execute_batch("ROLLBACK").unwrap();
    assert_eq!(bundle_native_snapshot(&connection),before);
    // An advanced scan pointer without a corresponding attempt is unavailable,
    // not permission to reuse the last audited zero or call the new scan clean.
    connection.execute("UPDATE sentinel_scans SET attempt_count=2 WHERE id=?1",[&lease.scan_id]).unwrap();
    let missing=serde_json::to_value(sentinel_overview_stats_in(&reader,None).unwrap()).unwrap();
    assert_eq!(missing["sourceReviewAuditedTaskCount"],0);
    assert_eq!(missing["sourceReviewUnavailableTaskCount"],1);
    drop(reader);drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn overview_source_and_web_confirmations_add_without_reclassifying_historical_rows() {
    let (root,connection,_,result)=source_reviewer_execution_fixture("all_confirmed",None);
    let source_count=result.unwrap()["sourceDecisionProjection"]["counts"]["confirmed"].as_i64().unwrap();
    connection.execute_batch("INSERT INTO sentinel_scans(id,project_id,status,attempt_count) VALUES('overview-web',1,'completed',1);
        INSERT INTO sentinel_scan_attempts(scan_id,attempt_number,status) VALUES('overview-web',1,'completed');
        INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,severity) VALUES
            ('overview-web','https://example.test','native','vulnerability','confirmed','critical'),
            ('overview-web','https://example.test','historical','vulnerability','legacy','high');
        INSERT INTO agent_runs(id,scan_id,role,parent_run_id,root_run_id,lane,status,terminal_state) VALUES
            ('overview-root','overview-web','coordinator',NULL,'overview-root','','terminal','completed'),
            ('overview-reviewer','overview-web','evidence_reviewer','overview-root','overview-root','review','terminal','completed');
        INSERT INTO agent_assignments(id,coordinator_run_id,child_run_id,role,lane,state,dedup_key)
            VALUES('overview-assignment','overview-root','overview-reviewer','evidence_reviewer','review','completed','overview-once');
        INSERT INTO agent_finding_candidates(id,root_run_id,scan_id,target_url,stage,kind,record_key,severity,status,candidate_revision,reviewer_run_id,published_at)
            VALUES('overview-candidate','overview-root','overview-web','https://example.test','native','vulnerability','confirmed','critical','published',1,'overview-reviewer','2026-09-27');
        INSERT INTO agent_review_decisions(root_run_id,candidate_id,candidate_revision,reviewer_run_id,verdict)
            VALUES('overview-root','overview-candidate',1,'overview-reviewer','confirmed');
        INSERT INTO agent_review_requests(id,root_run_id,assignment_id,reviewer_run_id,candidate_id,candidate_revision,status,decision_id,lease_epoch,fencing_token)
            SELECT 'overview-request','overview-root','overview-assignment','overview-reviewer','overview-candidate',1,'confirmed',id,1,'fixture'
            FROM agent_review_decisions WHERE candidate_id='overview-candidate';").unwrap();
    let before=bundle_native_snapshot(&connection);
    let stats=sentinel_overview_stats_in(&connection,None).unwrap();
    assert_eq!(stats.reviewer_confirmed_count,source_count+1);
    assert_eq!(stats.reviewer_high_risk_count,source_count+1);
    assert_eq!(stats.source_reviewer_confirmed_count,source_count);
    assert_eq!(stats.other_vulnerability_count,stats.vulnerability_count-1);
    assert!(stats.other_vulnerability_count>=1);
    assert_eq!(bundle_native_snapshot(&connection),before);
    drop(connection);fs::remove_dir_all(root).unwrap();
}
