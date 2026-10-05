#[test]
fn source_top_level_reentry_reuses_verified_initial_assessment_prefix() {
    for (checkpoint,completed) in [("after_first_assessment",1),("after_initial_assessments",2)] {
        let (root,connection,lease,result)=source_reviewer_execution_fixture_using(
            "valid",None,3,None,|root,connection,record,lease| {
                source_reviewer_checkpoint_fixture(root,connection,record,lease,checkpoint);
                assert_eq!(connection.query_row("SELECT count(*) FROM agent_assignments WHERE coordinator_run_id=?1 AND state='completed'",
                    [&lease.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),completed);
                run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001"))
            });
        let report=result.expect("verified initial phases must be reused without another model request");
        assert_eq!(report["modelRequests"],7);
        assert_eq!(report["assessments"].as_array().unwrap().len(),4);
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_assignments WHERE coordinator_run_id=?1",
            [&lease.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),5);
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_initial_received_response_reentry_delivers_locally_without_replaying_model() {
    for checkpoint in ["first_assessment_received","second_assessment_paused_received"] {
        let (root,connection,lease,result)=source_reviewer_execution_fixture_using(
            "valid",None,3,None,|root,connection,record,lease| {
                source_reviewer_checkpoint_fixture(root,connection,record,lease,checkpoint);
                run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001"))
            });
        let report=result.unwrap_or_else(|error|panic!("{checkpoint}: {error}"));
        assert_eq!(report["modelRequests"],7);
        assert_eq!(report["assessments"].as_array().unwrap().len(),4);
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_messages WHERE kind='evidence_summary' AND acknowledged_at<>''",[],|r|r.get::<_,i64>(0)).unwrap(),2);
        assert_eq!(connection.query_row("SELECT spent_requests,reserved_requests FROM agent_budget_ledger WHERE root_run_id=?1",[&lease.root_run_id],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?))).unwrap(),(7,0));
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_initial_unknown_response_reentry_does_not_replay_or_change_state() {
    let (root,connection,_lease,result)=source_reviewer_execution_fixture_using_calls(
        "valid",None,3,None,1,|root,connection,record,lease| {
            source_reviewer_checkpoint_fixture(root,connection,record,lease,"first_assessment_unknown");
            let before=source_exit_snapshot(connection);
            let result=run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001"));
            assert!(result.is_err());
            source_exit_assert_snapshot(connection, lease, &before);
            result
        });
    assert!(result.is_err());
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_initial_received_reentry_rejects_corrupt_checkpoint_without_writes() {
    for (checkpoint,expected_calls,mutation) in [
        ("first_assessment_received",1,"UPDATE agent_specialist_calls SET response_hash='changed' WHERE state='received'"),
        ("first_assessment_received",1,"UPDATE agent_budget_ledger SET spent_tokens=spent_tokens+1"),
        ("first_assessment_received",1,"UPDATE agent_assignments SET task_slice_json='{}' WHERE state='running'"),
        ("first_assessment_received",1,"UPDATE agent_specialist_calls SET request_json='{}' WHERE state='received'"),
        ("first_assessment_received",1,"UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00'"),
        ("second_assessment_paused_received",2,"UPDATE agent_messages SET acknowledged_at='' WHERE kind='evidence_summary'"),
        ("second_assessment_paused_received",2,"UPDATE agent_budget_ledger SET reserved_tokens=reserved_tokens+1"),
        ("second_assessment_paused_received",2,"INSERT INTO agent_capability_leases(id,root_run_id,assignment_id,child_run_id,capability,lease_epoch,fencing_token,lease_expires_at) SELECT 'unexpected-pending-http',coordinator_run_id,id,child_run_id,'http.request',lease_epoch,fencing_token,datetime('now','+600 seconds','localtime') FROM agent_assignments WHERE state='paused'"),
    ] {
        let (root,connection,_lease,result)=source_reviewer_execution_fixture_using_calls(
            "valid",None,3,None,expected_calls,|root,connection,record,lease| {
                source_reviewer_checkpoint_fixture(root,connection,record,lease,checkpoint);
                if mutation.starts_with("UPDATE agent_specialist_calls") {
                    connection.execute_batch("DROP TRIGGER agent_specialist_call_immutable").unwrap();
                }
                connection.execute_batch(mutation).unwrap();
                let before=source_exit_snapshot(connection);
                let result=run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001"));
                assert!(result.is_err(),"{checkpoint}: {mutation}");
                source_exit_assert_snapshot(connection, lease, &before);
                result
            });
        assert!(result.is_err(),"{checkpoint}: {mutation}");
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_initial_prefix_reentry_rejects_corrupt_receipts_before_activation() {
    for (checkpoint,expected_calls,mutation) in [
        ("after_first_assessment",1,"UPDATE agent_budget_ledger SET spent_tokens=spent_tokens+1"),
        ("after_first_assessment",1,"INSERT INTO tool_invocations(run_id,tool_name,status) SELECT id,'unreconciled-source-tool','running' FROM agent_runs WHERE role='coordinator'"),
        ("after_initial_assessments",2,"UPDATE agent_messages SET acknowledged_at='' WHERE kind='evidence_summary'"),
        ("after_initial_assessments",2,"UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00'"),
    ] {
        let (root,connection,_lease,result)=source_reviewer_execution_fixture_using_calls(
            "valid",None,3,None,expected_calls,|root,connection,record,lease| {
                source_reviewer_checkpoint_fixture(root,connection,record,lease,checkpoint);
                connection.execute_batch(mutation).unwrap();
                let before=source_exit_snapshot(connection);
                let result=run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001"));
                assert!(result.is_err(),"{checkpoint}: {mutation}");
                source_exit_assert_snapshot(connection, lease, &before);
                result
            });
        assert!(result.is_err(),"{checkpoint}: {mutation}");
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_top_level_reentry_reuses_completed_tool_phase() {
    let (root,connection,_lease,result)=source_reviewer_execution_fixture_using(
        "valid",None,3,None,|root,connection,record,lease| {
            source_reviewer_checkpoint_fixture(root,connection,record,lease,"after_first_tool_phase");
            assert_eq!(connection.query_row("SELECT count(*) FROM agent_assignments WHERE coordinator_run_id=?1 AND state='completed'",
                [&lease.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),3);
            run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001"))
        });
    let report=result.expect("completed source tool phase must be replayed from its verified receipt");
    assert_eq!(report["modelRequests"],7);
    assert_eq!(report["assessments"].as_array().unwrap().len(),4);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_partial_tool_reentry_rejects_changed_receipts_without_writes() {
    for mutation in [
        "UPDATE agent_budget_ledger SET spent_tokens=spent_tokens+1",
        "UPDATE agent_messages SET acknowledged_at='' WHERE kind='source_tool_result'",
        "UPDATE agent_assignments SET task_slice_json='{}' WHERE trigger_code='source_tools_ready'",
        "UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00'",
    ] {
        let (root,connection,_lease,result)=source_reviewer_execution_fixture_using_calls(
            "valid",None,3,None,4,|root,connection,record,lease| {
                source_reviewer_checkpoint_fixture(root,connection,record,lease,"after_first_tool_phase");
                connection.execute_batch(mutation).unwrap();
                let before=source_exit_snapshot(connection);
                let result=run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001"));
                assert!(result.is_err(),"{mutation}");
                source_exit_assert_snapshot(connection, lease, &before);
                result
            });
        assert!(result.is_err(),"{mutation}");
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_partial_tool_unknown_result_never_redispatches_or_terminalizes_root() {
    let (root,connection,_lease,result)=source_reviewer_execution_fixture_using_calls(
        "valid",None,3,None,3,|root,connection,record,lease| {
            source_reviewer_checkpoint_fixture(root,connection,record,lease,"first_tool_unknown");
            let before=source_exit_snapshot(connection);
            let result=run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001"));
            assert!(result.is_err());
            source_exit_assert_snapshot(connection, lease, &before);
            result
        });
    assert!(result.is_err());
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_received_tool_reentry_delivers_local_result_without_replaying_model() {
    let (root,connection,lease,result)=source_reviewer_execution_fixture_using(
        "valid",None,3,None,|root,connection,record,lease| {
            source_reviewer_checkpoint_fixture(root,connection,record,lease,"first_tool_received");
            assert_eq!(connection.query_row("SELECT count(*) FROM agent_source_model_rounds WHERE root_run_id=?1 AND state='received'",
                [&lease.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),1);
            run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001"))
        });
    let report=result.expect("the saved round must deliver its pending local tool before continuing");
    assert_eq!(report["modelRequests"],7);
    assert_eq!(report["assessments"].as_array().unwrap().len(),4);
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_source_model_rounds WHERE root_run_id=?1 AND round_number=1",
        [&lease.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),2);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_second_received_tool_reentry_keeps_first_tool_phase_completed() {
    let (root,connection,_lease,result)=source_reviewer_execution_fixture_using(
        "valid",None,3,None,|root,connection,record,lease| {
            source_reviewer_checkpoint_fixture(root,connection,record,lease,"second_tool_received");
            let count:i64=connection.query_row("SELECT count(*) FROM agent_source_model_rounds WHERE root_run_id=?1 AND state='received'",
                [&lease.root_run_id],|r|r.get(0)).unwrap();
            assert_eq!(count,3);
            run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001"))
        });
    let report=result.expect("second source-tools child must resume its own saved round");
    assert_eq!(report["modelRequests"],7);
    assert_eq!(report["assessments"].as_array().unwrap().len(),4);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_received_finish_round_reentry_reuses_prior_tool_and_model_rounds() {
    let (root,connection,_lease,result)=source_reviewer_execution_fixture_using(
        "valid",None,3,None,|root,connection,record,lease| {
            source_reviewer_checkpoint_fixture(root,connection,record,lease,"first_tool_finish_received");
            run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001"))
        });
    let report=result.expect("a received finish round must settle without another model request");
    assert_eq!(report["modelRequests"],7);
    assert_eq!(report["assessments"].as_array().unwrap().len(),4);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_received_tool_reentry_rejects_corrupt_receipt_without_writes() {
    for mutation in [
        "UPDATE agent_budget_ledger SET reserved_tokens=reserved_tokens+1",
        "DROP TRIGGER agent_source_round_immutable; UPDATE agent_source_model_rounds SET response_hash='corrupt'",
        "UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00'",
    ] {
        let (root,connection,_lease,result)=source_reviewer_execution_fixture_using_calls(
            "valid",None,3,None,3,|root,connection,record,lease| {
                source_reviewer_checkpoint_fixture(root,connection,record,lease,"first_tool_received");
                connection.execute_batch(mutation).unwrap();
                let before=source_exit_snapshot(connection);
                let result=run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001"));
                assert!(result.is_err(),"{mutation}");
                source_exit_assert_snapshot(connection, lease, &before);
                result
            });
        assert!(result.is_err());
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_reviewer_top_level_reentry_consumes_saved_response_without_replaying_completed_phases() {
    let (root,connection,lease,result)=source_reviewer_execution_fixture_using("confirmed",None,3,None,|root,connection,record,lease| {
        source_reviewer_checkpoint_fixture(root,connection,record,lease,"received");
        let before:(i64,i64)=connection.query_row("SELECT spent_requests,reserved_requests FROM agent_budget_ledger WHERE root_run_id=?1",
            [&lease.root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(before,(6,1));
        run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001"))
    });
    let result=result.expect("saved Reviewer response must resume from the real top-level entry");
    assert_eq!(result["modelRequests"],7);
    assert_eq!(result["totalTokens"],140);
    assert_eq!(result["confirmedFindings"],1);
    assert_eq!(result["assessments"].as_array().unwrap().len(),4);
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_assignments WHERE coordinator_run_id=?1",[&lease.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),5);
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_messages WHERE kind='source_review_result' AND acknowledged_at<>''",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_events WHERE run_id=?1 AND event_type='terminal_reduced'",[&lease.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),1);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

// Snapshot persisted rows rather than total_changes(): the production entry
// opens another connection, and a rejected admission must not mutate any row.
fn source_reviewer_reentry_state(connection:&rusqlite::Connection)->Vec<(String,Vec<String>)> {
    let names=connection.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap().query_map([],|r|r.get::<_,String>(0)).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    names.into_iter().map(|name| {
        let mut query=connection.prepare(&format!("SELECT * FROM \"{}\"",name.replace('"',"\"\""))).unwrap();
        let columns=query.column_count();
        let mut rows=query.query_map([],|r| {
            (0..columns).map(|i|r.get::<_,rusqlite::types::Value>(i)).collect::<Result<Vec<_>,_>>()
                .map(|values|format!("{values:?}"))
        }).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
        rows.sort();(name,rows)
    }).collect()
}

#[test]
fn source_reviewer_top_level_reentry_finishes_each_committed_review_boundary_once() {
    for checkpoint in ["before_review","undispatched","paused_received","delivered"] {
        let (root,connection,lease,result)=source_reviewer_execution_fixture_using("confirmed",None,3,None,|root,connection,record,lease| {
            source_reviewer_checkpoint_fixture(root,connection,record,lease,checkpoint);
            run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001"))
        });
        let result=result.unwrap_or_else(|error|panic!("{checkpoint}: {error}"));
        assert_eq!((result["modelRequests"].as_i64(),result["totalTokens"].as_i64()),(Some(7),Some(140)),"{checkpoint}");
        assert_eq!(result["assessments"].as_array().unwrap().len(),4);
        assert_eq!(result["confirmedFindings"],1);
        let budget:(i64,i64,i64,i64)=connection.query_row("SELECT spent_requests,spent_tokens,reserved_requests,reserved_tokens FROM agent_budget_ledger WHERE root_run_id=?1",
            [&lease.root_run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(budget,(7,140,0,0),"{checkpoint}");
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_assignments WHERE coordinator_run_id=?1",[&lease.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),5);
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_messages WHERE kind='source_review_result' AND acknowledged_at<>''",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_events WHERE run_id=?1 AND event_type='terminal_reduced'",[&lease.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),1);
        let before=source_reviewer_reentry_state(&connection);
        assert!(run_native_source_assessments(&root.join("oviraptor.sqlite3"),&lease.scan_id,1,&root.join("attempt-0001")).is_err(),"terminal roots must not resurrect");
        assert_eq!(source_reviewer_reentry_state(&connection),before,"terminal reentry changed persisted rows");
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_reviewer_top_level_reentry_preserves_unknown_outcome_without_redispatch_or_refund() {
    let (root,connection,lease,result)=source_reviewer_execution_fixture_using("confirmed",None,3,None,|root,connection,record,lease| {
        source_reviewer_checkpoint_fixture(root,connection,record,lease,"unknown");
        let before=source_exit_snapshot(connection);
        let result=run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001"));
        assert!(result.as_ref().unwrap_err().contains("source_review_outcome_unknown_requires_reconciliation"),"{result:?}");
        source_exit_assert_snapshot(connection, lease, &before);
        result
    });
    assert!(result.is_err());
    let budget:(i64,i64)=connection.query_row("SELECT spent_requests,reserved_requests FROM agent_budget_ledger WHERE root_run_id=?1",
        [&lease.root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(budget,(6,1));
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_reviewer_top_level_reentry_rejects_invalid_checkpoints_without_writes() {
    for (checkpoint,mutation) in [
        ("received","UPDATE agent_messages SET acknowledged_at='' WHERE kind='source_tool_result'"),
        ("received","INSERT INTO tool_invocations(run_id,tool_name,status) SELECT id,'unreconciled-source-tool','running' FROM agent_runs WHERE role='coordinator'"),
        ("received","UPDATE agent_assignments SET task_slice_json='{}' WHERE role='repo_mapper'"),
        ("received","UPDATE agent_runs SET cancel_requested_at='revoked' WHERE role='evidence_reviewer'"),
        ("received","UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00'"),
        ("received","UPDATE agent_runs SET attempt_number=attempt_number+1 WHERE role='evidence_reviewer'"),
        ("received","UPDATE sentinel_scans SET status='cancelled'"),
        ("received","UPDATE config_profiles SET settings_json=json_set(settings_json,'$.modelProfiles[0].apiKey','changed')"),
        ("undispatched","UPDATE agent_runs SET used_requests=1 WHERE role='evidence_reviewer'"),
        ("undispatched","UPDATE agent_capability_leases SET revoked_at='revoked' WHERE child_run_id IN (SELECT id FROM agent_runs WHERE role='evidence_reviewer')"),
        ("undispatched","UPDATE agent_capability_leases SET revoked_at='revoked' WHERE capability='review.write'"),
        ("undispatched","UPDATE agent_capability_leases SET lease_expires_at='2000-01-01 00:00:00' WHERE capability='review.write'"),
        ("undispatched","UPDATE agent_capability_leases SET fencing_token='different-fence' WHERE capability='review.write'"),
        ("undispatched","UPDATE agent_capability_leases SET capability='http.request' WHERE capability='review.write'"),
    ] {
        let calls=if checkpoint=="undispatched" {6} else {7};
        let (root,connection,_,result)=source_reviewer_execution_fixture_using_calls("confirmed",None,3,None,calls,|root,connection,record,lease| {
            source_reviewer_checkpoint_fixture(root,connection,record,lease,checkpoint);
            connection.execute_batch(mutation).unwrap();
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
fn source_reviewer_top_level_reentry_rejects_damaged_accounting_before_delivery() {
    for checkpoint in ["before_review","undispatched","received","paused_received","delivered"] {
        let calls=if matches!(checkpoint,"before_review"|"undispatched") {6} else {7};
        let (root,connection,_,result)=source_reviewer_execution_fixture_using_calls("confirmed",None,3,None,calls,|root,connection,record,lease| {
            source_reviewer_checkpoint_fixture(root,connection,record,lease,checkpoint);
            connection.execute("UPDATE agent_budget_ledger SET spent_tokens=spent_tokens+1 WHERE root_run_id=?1",[&lease.root_run_id]).unwrap();
            let before=source_exit_snapshot(connection);
            let result=run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001"));
            assert!(result.is_err(),"accepted damaged accounting: {checkpoint}");
            source_exit_assert_snapshot(connection, lease, &before);
            result
        });
        assert!(result.is_err());
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

// First-dispatch admission belongs to source recovery, not the independent
// Reviewer behavior suite. Share only its real-HTTP fixture and DB snapshot.
#[test]
fn source_empty_boundary_rejects_orphaned_work_and_accounting_before_dispatch() {
    for mutation in [
        "UPDATE agent_runs SET used_tokens=1 WHERE role='coordinator'",
        "UPDATE agent_runs SET reserved_requests=1 WHERE role='coordinator'",
        "INSERT INTO agent_budget_ledger(root_run_id,total_tokens,total_requests,lease_epoch,fencing_token) SELECT root_run_id,hard_token_budget,hard_request_budget,1,'orphaned' FROM agent_runs WHERE role='coordinator'",
        "INSERT INTO tool_invocations(run_id,tool_name,status) SELECT id,'unreconciled-source-tool','running' FROM agent_runs WHERE role='coordinator'",
    ] {
        let (root,connection,_,result)=source_reviewer_execution_fixture_using_calls(
            "valid",None,3,None,0,|root,connection,record,lease| {
                connection.execute_batch(mutation).unwrap();
                let before=source_exit_snapshot(connection);
                let result=run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001"));
                assert!(result.is_err(),"{mutation}");
                source_exit_assert_snapshot(connection, lease, &before);
                result
            });
        assert!(result.is_err(),"{mutation}");
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}
