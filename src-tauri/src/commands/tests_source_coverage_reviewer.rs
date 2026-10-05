// Pin this fixture to V4 so later defaults cannot silently change the contract
// under recovery and historical-consumer acceptance tests.
fn source_coverage_fixture_response(input:&JsonValue)->JsonValue {
    assert_eq!(input["phase"],"source_coverage_review");
    assert_eq!(input["toolsGranted"],json!([]));
    assert_eq!(input["targetRequestsGranted"],0);
    assert_eq!(input["hostActionsGranted"],0);
    let contract=&input["reviewMaterial"]["decisionContract"];
    let gaps=contract["requiredGaps"].as_array().unwrap();
    json!({"schemaVersion":1,"subject":"source_coverage","materialDigest":contract["materialDigest"],
        "coverageSufficient":gaps.is_empty(),"rationale":"Fixture coverage review against actual phase receipts",
        "reasonCodes":["fixture_coverage_review"],"evidenceRefs":[contract["evidenceRefs"][0]],"outstandingGaps":gaps})
}

fn source_coverage_execution_fixture(mode: &'static str, fault: Option<&str>) -> (
    PathBuf, rusqlite::Connection, crate::agent_runtime::multi_agent::lease::CoordinatorLease, Result<JsonValue,String>,
) {
    source_reviewer_execution_fixture_using_calls(mode,fault,4,None,if mode=="no_candidates" {7} else {8},|root,_,record,_| {
        run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001"))
    })
}

#[test]
fn source_coverage_reviewer_real_execution_has_distinct_identity_receipt_and_publication() {
    use crate::agent_runtime::multi_agent::{source_coverage_decisions,source_coverage_reviewer,source_reviewer};
    let (root,connection,lease,result)=source_coverage_execution_fixture("valid",None);
    let report=result.unwrap();
    assert_eq!(report["independentReviewCompleted"],true);
    assert_eq!(report["independentCandidateReviewCompleted"],true);
    assert_eq!(report["modelRequests"],8);
    assert_eq!(report["totalTokens"],160);
    assert_eq!(report["sourceCoverageDecision"]["decision"]["coverageSufficient"],false);
    assert_ne!(report["gate"]["status"],"passed");
    let tx=rusqlite::Transaction::new_unchecked(&connection,rusqlite::TransactionBehavior::Deferred).unwrap();
    let candidate=source_reviewer::audit_delivery(&tx,&lease).unwrap();
    let coverage=source_coverage_reviewer::audit_delivery(&tx,&lease).unwrap();
    assert_ne!(candidate.child.run_id,coverage.child.run_id);
    assert_ne!(candidate.child.assignment_id,coverage.child.assignment_id);
    assert_ne!(candidate.message_id,coverage.message_id);
    assert_eq!(coverage.decision_ids.len(),1);
    assert_eq!(source_coverage_decisions::read_audited(&tx,&lease).unwrap().as_json(),report["sourceCoverageDecision"]);
    assert_eq!(coverage.payload["result"]["materialDigest"],report["coverageReviewPreparation"]["digest"]);
    assert_eq!(tx.query_row("SELECT count(*) FROM agent_assignments WHERE coordinator_run_id=?1",[&lease.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),6);
    tx.rollback().unwrap();
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_coverage_reviewer_zero_candidates_still_reviews_coverage_without_fake_candidate_child() {
    let (root,connection,lease,result)=source_coverage_execution_fixture("no_candidates",None);
    let report=result.unwrap();
    assert_eq!(report["independentReviewCompleted"],true);
    assert_eq!(report["independentCandidateReviewCompleted"],false);
    assert_eq!(report["modelRequests"],7);
    assert!(report["gate"].is_object(),"zero candidates still require a real coverage-backed CI gate");
    assert_eq!(report["gate"]["independentCandidateReviewCompleted"],false);
    assert_eq!(report["gate"]["independentReviewCompleted"],true);
    assert_ne!(report["gate"]["status"],"passed");
    assert_eq!(report["coverageReviewPreparation"]["material"]["candidateReview"]["status"],"not_applicable_no_candidates");
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_assignments WHERE coordinator_run_id=?1 AND role='evidence_reviewer'",[&lease.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_source_review_decisions",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_coverage_consumers_share_real_decision_in_findings_ci_export_and_history() {
    for mode in ["valid","no_candidates"] {
        let (root,connection,lease,result)=source_coverage_execution_fixture(mode,None);
        let report=result.unwrap();
        let before=source_reviewer_reentry_state(&connection);
        let page=native_source_findings(&connection,&lease.scan_id,1,0,1).unwrap();
        assert_eq!(page["status"],"audited");
        assert_eq!(page["independentReviewCompleted"],true);
        assert_eq!(page["independentCandidateReviewCompleted"],mode!="no_candidates");
        assert_eq!(page["coverage"]["status"],"reviewed");
        assert_eq!(page["coverage"]["coverageSufficient"],false);
        assert_eq!(page["coverage"]["sourceCoverageDecision"],report["sourceCoverageDecision"]);
        assert_eq!(report["gate"]["coverage"],page["coverage"]);
        assert_eq!(report["gate"]["gaps"],page["coverage"]["outstandingGaps"]);
        assert!(!report["gate"]["gaps"].as_array().unwrap().contains(&json!("source_coverage_review")));
        if mode=="no_candidates" {
            assert_eq!(page["candidateReviewStatus"],"not_applicable_no_candidates");
            assert_eq!(page["materialSubject"],"source_coverage");
            assert_eq!(page["counts"],json!({"decisions":0,"confirmed":0,"rejected":0,"insufficient":0}));
            assert!(report["gate"]["sourceDecisionProjection"].is_null());
        }
        let exported=native_source_findings_export_snapshot(&connection,&lease.scan_id,1).unwrap();
        assert_eq!(exported["coverage"],page["coverage"]);
        assert_eq!(exported["ciGate"]["coverage"],page["coverage"]);
        assert_eq!(exported["ciGate"]["independentReviewCompleted"],true);
        let tx=connection.unchecked_transaction().unwrap();
        let counts=source_overview_counts(&tx,None).unwrap();
        assert_eq!(counts.audited_tasks,1);
        assert_eq!(counts.confirmed,exported["counts"]["confirmed"].as_i64().unwrap());
        tx.rollback().unwrap();
        assert_eq!(source_reviewer_reentry_state(&connection),before,"consumption must not write or renew authority");
        // A later attempt cannot overwrite or supply this attempt's coverage.
        connection.execute("INSERT INTO sentinel_scan_attempts(scan_id,attempt_number,status) VALUES(?1,2,'scanning')",[&lease.scan_id]).unwrap();
        assert_eq!(native_source_findings_export_snapshot(&connection,&lease.scan_id,1).unwrap(),exported);
        assert!(native_source_findings_export_snapshot(&connection,&lease.scan_id,2).is_err());
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_coverage_consumers_reject_corruption_and_stale_projection_without_repair() {
    use crate::agent_runtime::multi_agent::source_review_projection;
    for mode in ["valid","no_candidates"] {
        let (root,connection,lease,result)=source_coverage_execution_fixture(mode,None);
        result.unwrap();
        for sql in [
            "UPDATE agent_messages SET acknowledged_at='' WHERE kind='source_coverage_review_result'",
            "DROP TRIGGER source_coverage_decision_no_update; UPDATE agent_source_coverage_decisions SET material_digest=printf('%064d',0)",
            "DROP TRIGGER source_coverage_decision_no_delete; DELETE FROM agent_source_coverage_decisions",
            "UPDATE agent_assignments SET trigger_code='source_candidates_ready' WHERE trigger_code='source_coverage_ready'",
        ] {
            let tx=connection.unchecked_transaction().unwrap();
            let prior=source_review_projection::read_audited(&tx,&lease).unwrap();
            tx.execute_batch(sql).unwrap();
            let before=source_reviewer_reentry_state(&tx);
            assert!(source_review_projection::read_audited(&tx,&lease).is_err(),"{mode}: {sql}");
            assert!(crate::native_pipeline::source_ci::evaluate(&tx,&lease,&prior,crate::native_pipeline::ci::GatePolicy::default()).is_err());
            let page=native_source_findings(&tx,&lease.scan_id,1,0,1).unwrap();
            assert_eq!(page["status"],"unverified");
            assert!(page["counts"].is_null());
            assert!(page["coverage"].is_null());
            assert!(native_source_findings_export_snapshot(&tx,&lease.scan_id,1).is_err());
            assert_eq!(source_overview_counts(&tx,None).unwrap().unverified_tasks,1);
            assert_eq!(source_reviewer_reentry_state(&tx),before);
            tx.rollback().unwrap();
        }
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_coverage_consumers_pending_review_is_not_published_or_replayed() {
    use crate::agent_runtime::multi_agent::source_review_projection;
    for mode in ["valid","no_candidates"] {
        let calls=if mode=="no_candidates" {6} else {7};
        let (root,connection,_,result)=source_reviewer_execution_fixture_using_calls(mode,None,4,None,calls,|root,connection,record,lease| {
            source_coverage_checkpoint_fixture(root,connection,record,lease,"before_coverage");
            let before=source_reviewer_reentry_state(connection);
            let tx=connection.unchecked_transaction().unwrap();
            assert!(source_review_projection::read_audited(&tx,lease).is_err());
            let page=native_source_findings(&tx,&lease.scan_id,1,0,20).unwrap();
            assert_eq!(page["status"],"unverified");
            assert!(page["counts"].is_null());
            assert!(page["coverage"].is_null());
            assert!(native_source_findings_export_snapshot(&tx,&lease.scan_id,1).is_err());
            assert_eq!(source_overview_counts(&tx,None).unwrap().unverified_tasks,1);
            tx.rollback().unwrap();
            assert_eq!(source_reviewer_reentry_state(connection),before,"readers must not dispatch the missing coverage review");
            Err("fixture_intentionally_pending_coverage".into())
        });
        assert_eq!(result.unwrap_err(),"fixture_intentionally_pending_coverage");
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_coverage_consumers_reject_unplanned_publications_without_repair() {
    use crate::agent_runtime::multi_agent::source_review_projection;
    for schema in [3,4] {
        let mode=if schema==4 {"no_candidates"} else {"valid"};
        let (root,connection,lease,result)=source_reviewer_execution_fixture_using_calls(mode,None,schema,None,7,|root,_,record,_| {
            run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001"))
        });
        result.unwrap();
        let tx=connection.unchecked_transaction().unwrap();
        let prior=source_review_projection::read_audited(&tx,&lease).unwrap();
        let (sql,expected)=if schema==4 {
            ("INSERT INTO agent_source_review_decisions(id,root_run_id,scan_id,attempt_number,candidate_identity_json,material_digest,record_json,record_digest)
                VALUES('unplanned-candidate',?1,?2,1,'{}',?3,'{}',?3)","source_review_projection_unplanned_candidate_decisions")
        } else {
            ("INSERT INTO agent_source_coverage_decisions(id,root_run_id,scan_id,attempt_number,material_digest,record_json,record_digest)
                VALUES('unplanned-coverage',?1,?2,1,?3,'{}',?3)","source_review_projection_unplanned_coverage_decisions")
        };
        tx.execute(sql,params![lease.root_run_id,lease.scan_id,"0".repeat(64)]).unwrap();
        let before=source_reviewer_reentry_state(&tx);
        assert_eq!(source_review_projection::read_audited(&tx,&lease).unwrap_err(),expected);
        assert!(crate::native_pipeline::source_ci::evaluate(&tx,&lease,&prior,crate::native_pipeline::ci::GatePolicy::default()).is_err());
        assert_eq!(native_source_findings(&tx,&lease.scan_id,1,0,20).unwrap()["status"],"unverified");
        assert!(native_source_findings_export_snapshot(&tx,&lease.scan_id,1).is_err());
        assert_eq!(source_reviewer_reentry_state(&tx),before);
        tx.rollback().unwrap();
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_coverage_consumers_survive_reinitialize_and_allow_scan_cascade() {
    use crate::agent_runtime::multi_agent::source_review_projection;
    for mode in ["valid","no_candidates"] {
        let (root,connection,lease,result)=source_coverage_execution_fixture(mode,None);
        result.unwrap();
        let tx=connection.unchecked_transaction().unwrap();
        let before=source_review_projection::read_audited(&tx,&lease).unwrap();
        tx.rollback().unwrap();
        drop(connection);
        let path=crate::db::initialize(&root).unwrap();
        let connection=crate::db::open(&path).unwrap();
        let tx=connection.unchecked_transaction().unwrap();
        assert_eq!(source_review_projection::read_audited(&tx,&lease).unwrap(),before);
        tx.execute("DELETE FROM sentinel_scans WHERE id=?1",[&lease.scan_id]).unwrap();
        for sql in ["SELECT count(*) FROM agent_source_coverage_decisions","SELECT count(*) FROM agent_source_review_decisions",
            "SELECT count(*) FROM agent_runs","SELECT count(*) FROM agent_assignments","SELECT count(*) FROM agent_messages"] {
            assert_eq!(tx.query_row(sql,[],|r|r.get::<_,i64>(0)).unwrap(),0,"{mode}: {sql}");
        }
        assert!(native_source_findings_export_snapshot(&tx,&lease.scan_id,1).is_err());
        tx.rollback().unwrap();
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_coverage_consumers_sufficient_frozen_scope_can_pass_without_fake_candidates() {
    let (root,connection,lease,result)=source_reviewer_execution_fixture_using_calls("no_candidates_separate_git",None,4,None,7,|root,_,record,_| {
        run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001"))
    });
    let report=result.unwrap();
    assert_eq!(report["sourceCoverageDecision"]["decision"]["outstandingGaps"],json!([]),"{report}");
    assert_eq!(report["sourceCoverageDecision"]["decision"]["coverageSufficient"],true);
    assert_eq!(report["independentReviewCompleted"],true);
    assert_eq!(report["independentCandidateReviewCompleted"],false);
    assert_eq!(report["gate"]["status"],"passed");
    assert!(report["gate"]["sourceDecisionProjection"].is_null());
    let page=native_source_findings(&connection,&lease.scan_id,1,0,20).unwrap();
    assert_eq!(page["coverage"],report["gate"]["coverage"]);
    assert_eq!(page["candidateReviewStatus"],"not_applicable_no_candidates");
    let export=native_source_findings_export_snapshot(&connection,&lease.scan_id,1).unwrap();
    assert_eq!(export["coverage"],page["coverage"]);
    assert_eq!(export["ciGate"]["status"],"passed");
    assert_eq!(export["ciGate"]["coverage"],page["coverage"]);
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_source_review_decisions",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_coverage_exports_roundtrip_as_history_without_runtime_authority() {
    for mode in ["valid","no_candidates"] {
        let (root,connection,lease,result)=source_coverage_execution_fixture(mode,None);
        let report=result.unwrap();
        let directory=root.join("coverage-review-export");
        let path=write_native_source_findings_export_as(&connection,&directory,&lease.scan_id,1,NativeSourceExportFormat::Bundle).unwrap();
        let bytes=fs::read(&path).unwrap();
        let bundle:JsonValue=serde_json::from_slice(&bytes).unwrap();
        let json=&bundle["documents"]["json"];
        assert_eq!(json["coverageReviewCompleted"],true);
        assert_eq!(json["executionEligible"],false);
        assert_eq!(json["review"]["coverage"]["sourceCoverageDecision"],report["sourceCoverageDecision"]);
        let mut summary=json.clone();summary["review"].as_object_mut().unwrap().remove("findings");
        assert_eq!(bundle["documents"]["sarif"]["runs"][0]["properties"]["oviraptorSourceReview"],summary);
        let destination=root.join("imported-coverage-history");
        let database=db::initialize(&destination).unwrap();
        let imported=db::open(&database).unwrap();
        let before=bundle_native_snapshot(&imported);
        assert!(import_sentinel_project_path(&database,&destination,&path).unwrap()>0);
        let runs=historical_import_runs(&imported,None).unwrap();
        assert_eq!(runs.len(),1);
        assert_eq!(runs[0].finding_candidates,json["review"]["counts"]["confirmed"].as_i64().unwrap());
        assert!(historical_import_previews(&imported,&lease.scan_id).unwrap().iter()
            .all(|row|row.read_only && !row.execution_eligible && row.review_state=="unreviewed"));
        assert_eq!(bundle_native_snapshot(&imported),before);
        assert_eq!(imported.query_row("SELECT count(*) FROM agent_source_coverage_decisions",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert_eq!(fs::read(&path).unwrap(),bytes);
        drop(imported);drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_coverage_reviewer_rejects_gap_erasure_foreign_material_and_fabricated_refs() {
    for mode in ["coverage_drop_gap","coverage_foreign","coverage_fabricated"] {
        let (root,connection,lease,result)=source_coverage_execution_fixture(mode,None);
        assert!(result.is_err(),"{mode}: {result:?}");
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_source_coverage_decisions",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_messages WHERE kind='source_coverage_review_result'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_specialist_calls WHERE root_run_id=?1 AND state='received'",[&lease.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),4);
        let (spent,reserved):(i64,i64)=connection.query_row("SELECT spent_requests,reserved_requests FROM agent_budget_ledger WHERE root_run_id=?1",[&lease.root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!((spent,reserved),(7,1),"invalid received response must not be refunded");
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

fn source_coverage_checkpoint_fixture(root:&Path,connection:&rusqlite::Connection,record:&WorkbenchStartRecord,
    lease:&crate::agent_runtime::multi_agent::lease::CoordinatorLease,checkpoint:&str) {
    use crate::agent_runtime::multi_agent::source_coverage_reviewer;
    source_reviewer_checkpoint_fixture(root,connection,record,lease,"before_review");
    let database=root.join("oviraptor.sqlite3");
    let work=root.join("attempt-0001");
    let (model,runtime,_)=verify_source_runtime_contract(connection,&record.scan_id,1,&work).unwrap();
    let proxy=source_runtime_proxy(&runtime).unwrap();
    let profile=agent_model_profile(&model,proxy).unwrap();
    let context=SpecialistTransportContext { supervision: None,db_path:&database,scan_id:&record.scan_id,attempt_number:1,
        target_key:&lease.target_key,run_id:&lease.root_run_id,environment:&model,proxy,usage_dir:&work,
        deadline:Some(std::time::Instant::now()+Duration::from_secs(180))};
    run_source_candidate_review(connection,&context,lease).unwrap();
    if checkpoint=="before_coverage" {return;}
    let tx=connection.unchecked_transaction().unwrap();
    let slice=source_coverage_reviewer::task_slice(&tx,lease).unwrap().unwrap();
    tx.rollback().unwrap();
    let (tokens,_)=source_assessment_budget(&source_assessment_messages(source_coverage_reviewer::SYSTEM,&slice),&profile).unwrap();
    let child=source_coverage_reviewer::prepare(connection,lease,&slice,tokens).unwrap();
    if checkpoint=="undispatched" {return;}
    if checkpoint=="dispatch_revoked" {
        for mutation in [
            "UPDATE agent_capability_leases SET revoked_at='revoked' WHERE assignment_id=NEW.assignment_id AND capability='review.write';",
            "UPDATE agent_capability_leases SET revoked_at='revoked' WHERE assignment_id=NEW.assignment_id AND capability='evidence.read';",
            "UPDATE agent_capability_leases SET fencing_token='stale-fence' WHERE assignment_id=NEW.assignment_id AND capability='review.write';",
            "INSERT INTO agent_capability_leases(id,root_run_id,assignment_id,child_run_id,capability,lease_epoch,fencing_token,lease_expires_at)
                SELECT 'unexpected-coverage-http',root_run_id,assignment_id,child_run_id,'http.request',lease_epoch,fencing_token,lease_expires_at
                FROM agent_capability_leases WHERE assignment_id=NEW.assignment_id AND capability='review.write';",
        ] {
            connection.execute_batch(&format!("CREATE TRIGGER coverage_dispatch_failure AFTER INSERT ON agent_specialist_calls
                WHEN NEW.role='evidence_reviewer' BEGIN {mutation} END;")).unwrap();
            let before=source_reviewer_reentry_state(connection);
            let error=specialist_round_transport(&context,lease,&child,source_coverage_reviewer::SYSTEM,slice.clone()).unwrap_err();
            assert!(error.contains("source_coverage_review_dispatch_capabilities_invalid"),"{error}");
            assert_eq!(source_reviewer_reentry_state(connection),before,"coverage claim must roll back permission drift before HTTP");
            connection.execute_batch("DROP TRIGGER coverage_dispatch_failure").unwrap();
        }
        return;
    }
    if checkpoint=="unknown" {
        connection.execute_batch("CREATE TRIGGER coverage_checkpoint_failure BEFORE UPDATE OF state ON agent_specialist_calls
            WHEN NEW.state='received' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
        let error=specialist_round_transport(&context,lease,&child,source_coverage_reviewer::SYSTEM,slice).unwrap_err();
        assert!(error.contains("specialist_response_persist_missing"),"{error}");
        connection.execute_batch("DROP TRIGGER coverage_checkpoint_failure").unwrap();
        return;
    }
    specialist_round_transport(&context,lease,&child,source_coverage_reviewer::SYSTEM,slice).unwrap();
    match checkpoint {
        "received"=>{},
        "paused_received"=>stop_failed_child_preserving_usage(connection,lease,&child,"saved coverage response interrupted").unwrap(),
        "delivered"=>{deliver_source_coverage_review(connection,&context,lease,&child).unwrap();},
        _=>panic!("unknown checkpoint"),
    }
}

#[test]
fn source_coverage_reviewer_publication_and_mailbox_faults_preserve_saved_usage() {
    for fault in [
        "CREATE TRIGGER coverage_fault BEFORE INSERT ON agent_source_coverage_decisions BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER coverage_fault AFTER INSERT ON agent_source_coverage_decisions BEGIN UPDATE agent_messages SET acknowledged_at='' WHERE kind='source_coverage_review_result'; END;",
        "CREATE TRIGGER coverage_fault BEFORE INSERT ON agent_collaboration_events WHEN NEW.event_type='mailbox_message' AND json_extract(NEW.payload_json,'$.kind')='source_coverage_review_result' BEGIN SELECT RAISE(IGNORE); END;",
    ] {
        let (root,connection,lease,result)=source_coverage_execution_fixture("valid",Some(fault));
        assert!(result.is_err(),"{fault}: {result:?}");
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_source_coverage_decisions",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_messages WHERE kind='source_coverage_review_result'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert_eq!(connection.query_row("SELECT spent_requests,reserved_requests FROM agent_budget_ledger WHERE root_run_id=?1",
            [&lease.root_run_id],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?))).unwrap(),(7,1));
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_coverage_reviewer_immutable_publication_and_audit_reject_damage_without_repair() {
    use crate::agent_runtime::multi_agent::source_coverage_decisions;
    let (root,connection,lease,result)=source_coverage_execution_fixture("valid",None);
    let report=result.unwrap();
    for sql in [
        "UPDATE agent_source_coverage_decisions SET record_json='{}'",
        "DELETE FROM agent_source_coverage_decisions",
        "INSERT OR REPLACE INTO agent_source_coverage_decisions SELECT * FROM agent_source_coverage_decisions",
    ] {assert!(connection.execute_batch(sql).is_err(),"{sql}");}
    for sql in [
        "UPDATE agent_messages SET acknowledged_at='' WHERE kind='source_coverage_review_result'",
        "UPDATE agent_messages SET payload_json='{}' WHERE kind='source_coverage_review_result'",
        "UPDATE agent_runs SET used_requests=0 WHERE id IN (SELECT child_run_id FROM agent_assignments WHERE trigger_code='source_coverage_ready')",
        "UPDATE agent_assignments SET trigger_code='unplanned_review' WHERE trigger_code='source_coverage_ready'",
        "DROP TRIGGER source_coverage_decision_no_update; UPDATE agent_source_coverage_decisions SET record_json='{}'",
        "DROP TRIGGER source_coverage_decision_no_update; UPDATE agent_source_coverage_decisions SET record_digest=printf('%064d',0)",
        "DROP TRIGGER source_coverage_decision_no_delete; DELETE FROM agent_source_coverage_decisions",
    ] {
        let tx=connection.unchecked_transaction().unwrap();
        tx.execute_batch(sql).unwrap();
        let before=source_reviewer_reentry_state(&tx);
        assert!(source_coverage_decisions::read_audited(&tx,&lease).is_err(),"{sql}");
        assert!(source_coverage_decisions::publish(&tx,&lease).is_err(),"terminal history must not be repaired: {sql}");
        assert_eq!(source_reviewer_reentry_state(&tx),before);
        tx.rollback().unwrap();
        let tx=connection.unchecked_transaction().unwrap();
        assert_eq!(source_coverage_decisions::read_audited(&tx,&lease).unwrap().as_json(),report["sourceCoverageDecision"]);
        tx.rollback().unwrap();
    }
    drop(connection);fs::remove_dir_all(root).unwrap();
}
