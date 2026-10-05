#[test]
fn source_coverage_reviewer_reentry_delivers_each_checkpoint_once_including_no_candidates() {
    for (mode,checkpoint) in [("valid","before_coverage"),("valid","undispatched"),("valid","received"),
        ("valid","paused_received"),("valid","delivered"),("no_candidates","received"),("no_candidates","paused_received"),("no_candidates","delivered")] {
        let calls=if mode=="no_candidates" {7} else {8};
        let (root,connection,lease,result)=source_reviewer_execution_fixture_using_calls(mode,None,4,None,calls,|root,connection,record,lease| {
            source_coverage_checkpoint_fixture(root,connection,record,lease,checkpoint);
            if checkpoint=="paused_received" {
                assert_eq!(connection.query_row("SELECT count(*) FROM agent_capability_leases WHERE root_run_id=?1 AND revoked_at=''",[&lease.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),0);
                // Local receipt settlement must succeed without resurrecting
                // dispatch permissions, including when there were no candidates.
                connection.execute_batch("CREATE TRIGGER forbid_recovery_grant_insert BEFORE INSERT ON agent_capability_leases
                    WHEN NEW.revoked_at='' BEGIN SELECT RAISE(ABORT,'local recovery must not grant dispatch'); END;
                    CREATE TRIGGER forbid_recovery_grant_update BEFORE UPDATE ON agent_capability_leases
                    WHEN NEW.revoked_at='' BEGIN SELECT RAISE(ABORT,'local recovery must not renew dispatch'); END;").unwrap();
            }
            run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001"))
        });
        let report=result.unwrap_or_else(|error|panic!("{mode}/{checkpoint}: {error}"));
        assert_eq!(report["independentReviewCompleted"],true);
        let budget:(i64,i64,i64,i64)=connection.query_row("SELECT spent_requests,spent_tokens,reserved_requests,reserved_tokens
            FROM agent_budget_ledger WHERE root_run_id=?1",[&lease.root_run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(budget,(calls as i64,20*calls as i64,0,0));
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_source_coverage_decisions",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_messages WHERE kind='source_coverage_review_result' AND acknowledged_at<>''",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        let before=source_reviewer_reentry_state(&connection);
        assert!(run_native_source_assessments(&root.join("oviraptor.sqlite3"),&lease.scan_id,1,&root.join("attempt-0001")).is_err());
        assert_eq!(source_reviewer_reentry_state(&connection),before);
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_coverage_reviewer_reentry_rejects_invalid_authority_without_writes_or_calls() {
    // Target the coverage child only: damaging the already delivered candidate
    // Reviewer would test the preceding phase instead of this new authority.
    let coverage_child="SELECT child_run_id FROM agent_assignments WHERE trigger_code='source_coverage_ready'";
    for (checkpoint,mutation) in [
        ("received","UPDATE agent_runs SET cancel_requested_at='requested' WHERE id IN ($coverage_child)"),
        ("received","UPDATE agent_runs SET cancel_requested_at='requested' WHERE role='coordinator'"),
        ("received","UPDATE sentinel_scans SET status='cancelled'"),
        ("received","UPDATE sentinel_scans SET attempt_count=2"),
        ("received","UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00'"),
        ("received","UPDATE agent_assignments SET fencing_token='stale-fence' WHERE trigger_code='source_coverage_ready'"),
        ("received","UPDATE agent_runs SET attempt_number=attempt_number+1 WHERE id IN ($coverage_child)"),
        ("received","UPDATE config_profiles SET settings_json=json_set(settings_json,'$.modelProfiles[0].apiKey','changed')"),
        ("received","DELETE FROM agent_lane_leases WHERE assignment_id IN (SELECT id FROM agent_assignments WHERE trigger_code='source_coverage_ready')"),
        ("undispatched","UPDATE agent_capability_leases SET revoked_at='revoked' WHERE capability='review.write' AND child_run_id IN ($coverage_child)"),
        ("undispatched","UPDATE agent_capability_leases SET lease_expires_at='2000-01-01 00:00:00' WHERE capability='review.write' AND child_run_id IN ($coverage_child)"),
        ("undispatched","UPDATE agent_capability_leases SET fencing_token='stale-fence' WHERE capability='review.write' AND child_run_id IN ($coverage_child)"),
        ("undispatched","UPDATE agent_capability_leases SET capability='http.request' WHERE capability='review.write' AND child_run_id IN ($coverage_child)"),
        ("undispatched","UPDATE agent_capability_leases SET revoked_at='revoked' WHERE capability='evidence.read' AND child_run_id IN ($coverage_child)"),
        ("undispatched","DELETE FROM agent_lane_leases WHERE assignment_id IN (SELECT id FROM agent_assignments WHERE trigger_code='source_coverage_ready')"),
    ] {
        let calls=if checkpoint=="undispatched" {7} else {8};
        let (root,connection,_,result)=source_reviewer_execution_fixture_using_calls("valid",None,4,None,calls,|root,connection,record,lease| {
            source_coverage_checkpoint_fixture(root,connection,record,lease,checkpoint);
            assert_eq!(connection.query_row("SELECT count(*) FROM agent_assignments WHERE trigger_code='source_coverage_ready'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
            let mutation=mutation.replace("$coverage_child",coverage_child);
            let changed_before=connection.total_changes();
            connection.execute_batch(&mutation).unwrap();
            assert!(connection.total_changes()>changed_before,"authority mutation was a no-op: {mutation}");
            let before=source_exit_snapshot(connection);
            let result=run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001"));
            assert!(result.is_err(),"accepted {checkpoint}: {mutation}");
            source_exit_assert_snapshot(connection, lease, &before);
            result
        });
        assert!(result.is_err());
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_coverage_reviewer_dispatch_rechecks_permissions_after_claim_without_http() {
    let (root,connection,_,result)=source_reviewer_execution_fixture_using_calls("valid",None,4,None,7,|root,connection,record,lease| {
        source_coverage_checkpoint_fixture(root,connection,record,lease,"dispatch_revoked");
        Err("expected_coverage_dispatch_refusal".into())
    });
    assert_eq!(result.unwrap_err(),"expected_coverage_dispatch_refusal");
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_coverage_reviewer_unknown_dispatch_is_not_replayed_or_closed_on_reentry() {
    let (root,connection,lease,result)=source_reviewer_execution_fixture_using_calls("valid",None,4,None,8,|root,connection,record,lease| {
        source_coverage_checkpoint_fixture(root,connection,record,lease,"unknown");
        let before=source_exit_snapshot(connection);
        let result=run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001"));
        assert!(result.as_ref().unwrap_err().contains("source_coverage_review_outcome_unknown_requires_reconciliation"),"{result:?}");
        source_exit_assert_snapshot(connection, lease, &before);
        result
    });
    assert!(result.is_err());
    assert_eq!(connection.query_row("SELECT spent_requests,reserved_requests FROM agent_budget_ledger WHERE root_run_id=?1",
        [&lease.root_run_id],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?))).unwrap(),(7,1));
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_source_coverage_decisions",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(connection);fs::remove_dir_all(root).unwrap();
}
