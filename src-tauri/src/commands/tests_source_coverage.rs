#[test]
fn source_coverage_preparation_rejects_injected_phase_without_mutating_historical_receipts() {
    use crate::agent_runtime::multi_agent::source_coverage;
    let (root,connection,lease,result)=source_reviewer_execution_fixture("all_confirmed",None);
    let report=result.unwrap();
    let original:String=connection.query_row("SELECT plan_json FROM agent_runs WHERE id=?1",
        [&lease.root_run_id],|r|r.get(0)).unwrap();
    let calls:i64=connection.query_row("SELECT count(*) FROM agent_specialist_calls",[],|r|r.get(0)).unwrap();
    for key in ["sourceCoveragePhaseVersion","sourceCoverageDecisionPhaseVersion"] {
        let tx=rusqlite::Transaction::new_unchecked(&connection,rusqlite::TransactionBehavior::Immediate).unwrap();
        tx.execute("UPDATE agent_runs SET plan_json=json_set(plan_json,?2,1) WHERE id=?1",
            params![lease.root_run_id,format!("$.{key}")]).unwrap();
        assert_eq!(source_coverage::audit(&tx,&lease).unwrap_err(),"source_surface_frozen_plan_invalid");
        tx.rollback().unwrap();
        let tx=rusqlite::Transaction::new_unchecked(&connection,rusqlite::TransactionBehavior::Deferred).unwrap();
        assert_eq!(source_coverage::audit(&tx,&lease).unwrap().as_json(),report["coverageReviewPreparation"]);
        tx.rollback().unwrap();
    }
    assert_eq!(connection.query_row("SELECT plan_json FROM agent_runs WHERE id=?1",[&lease.root_run_id],|r|r.get::<_,String>(0)).unwrap(),original);
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_specialist_calls",[],|r|r.get::<_,i64>(0)).unwrap(),calls);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_coverage_preparation_binds_scope_receipts_without_claiming_review() {
    let (root,connection,lease,result)=source_reviewer_execution_fixture("all_confirmed",None);
    let report=result.unwrap();
    let preparation=&report["coverageReviewPreparation"];
    assert_eq!(preparation["status"],"prepared_not_reviewed");
    assert_eq!(preparation["executionEligible"],false);
    assert_eq!(preparation["independentReviewCompleted"],false);
    let material=&preparation["material"];
    assert_eq!(material["subject"],"source_coverage");
    assert_eq!(material["rootRunId"],lease.root_run_id);
    assert_eq!(material["scanId"],lease.scan_id);
    assert_eq!(material["attemptNumber"],1);
    assert_eq!(preparation["digest"],crate::artifact_import::canonical::sha256_hex(material.to_string().as_bytes()));
    let (view,results,plan_hash)=crate::agent_runtime::multi_agent::source::historical_materials(&connection,&lease.scan_id,1).unwrap();
    assert_eq!(material["sourcePlanHash"],plan_hash);
    assert_eq!(material["analysisDigest"],view.manifest.digest());
    assert_eq!(material["analysisResultsDigest"],results.digest());
    assert_eq!(material["scope"]["selectedFiles"],json!(view.manifest.files));
    assert_eq!(material["scope"]["effective"],view.manifest.scope);
    assert_eq!(material["scope"]["changedPathsWithoutContent"],json!(view.manifest.changed_paths_without_content));
    assert_eq!(material["analyzerRuns"],json!(results.runs));
    assert_eq!(material["phaseReceipts"].as_array().unwrap().len(),4);
    assert_eq!(material["candidateReview"]["status"],"completed");
    assert_eq!(material["targetRequestsGranted"],0);
    assert_eq!(material["hostActionsGranted"],0);
    assert_eq!(material["toolsGranted"],json!([]));
    assert!(preparation["outstandingGaps"].as_array().unwrap().contains(&json!("source_coverage_review")));
    assert!(!preparation["outstandingGaps"].as_array().unwrap().contains(&json!("source_review_not_completed")));
    assert_eq!(report["independentReviewCompleted"],false);
    assert_ne!(report["gate"]["status"],"passed");
    let page=native_source_findings(&connection,&lease.scan_id,1,0,50).unwrap();
    assert_eq!(page["coverage"]["materialDigest"],preparation["digest"]);
    assert_eq!(page["coverage"]["outstandingGaps"],preparation["outstandingGaps"]);
    assert_eq!(page["coverage"]["status"],"prepared_not_reviewed");
    assert_eq!(page["coverage"]["selectedFileCount"],view.manifest.files.len());
    let closure:String=connection.query_row("SELECT payload_json FROM agent_events WHERE run_id=?1 AND event_type='terminal_reduced'",
        [&lease.root_run_id],|r|r.get(0)).unwrap();
    let closure:JsonValue=serde_json::from_str(&closure).unwrap();
    assert_eq!(&closure["coverageReviewPreparation"],preparation);
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_assignments WHERE coordinator_run_id=?1",
        [&lease.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),5,"preparing material must not create a sixth assignment");
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_coverage_preparation_preserves_tool_gaps_in_closure_ci_and_exports() {
    let (root,connection,lease,result)=source_reviewer_execution_fixture("coverage_gap",None);
    let report=result.unwrap();
    let preparation=&report["coverageReviewPreparation"];
    for gaps in [&preparation["outstandingGaps"],&report["gate"]["gaps"]] {
        let gaps=gaps.as_array().unwrap();
        assert!(gaps.contains(&json!("dependency_graph_incomplete")));
        assert!(gaps.contains(&json!("source_candidate_evidence_incomplete")));
        assert!(gaps.contains(&json!("source_coverage_review")));
    }
    let reports=preparation["material"]["reportedGaps"].as_array().unwrap();
    let limitations=reports.iter().filter(|g|g["code"]=="dependency_graph_incomplete").collect::<Vec<_>>();
    assert_eq!(limitations.len(),2,"each tool author must remain attributable after gap code deduplication");
    assert!(limitations.iter().all(|g|g["origin"]=="tool_assignment" && g["assignmentId"].is_string()));
    let tx=rusqlite::Transaction::new_unchecked(&connection,rusqlite::TransactionBehavior::Deferred).unwrap();
    let bases=crate::agent_runtime::multi_agent::source::completion_task_slices(&tx,&lease).unwrap();
    let proof=source_assessment_completion(&tx,&lease,&bases).unwrap();
    assert!(proof.completion.uncovered_families.contains(&"dependency_graph_incomplete".into()));
    tx.rollback().unwrap();
    // Non-CI display must also carry actual source-tool limitations.
    let mut non_ci=report.clone();non_ci["gate"]=JsonValue::Null;
    let mut presentation=json!({"gaps":[],"sourceClaims":[]});
    merge_native_source_assessments(&mut presentation,non_ci);
    assert!(presentation["gaps"].as_array().unwrap().contains(&json!("dependency_graph_incomplete")));
    for format in [NativeSourceExportFormat::Json,NativeSourceExportFormat::Sarif,NativeSourceExportFormat::Bundle] {
        let path=write_native_source_findings_export_as(&connection,&root.join("coverage-export"),&lease.scan_id,1,format).unwrap();
        let exported:JsonValue=serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        let review=match format {
            NativeSourceExportFormat::Json=>&exported["review"],
            NativeSourceExportFormat::Sarif=>&exported["runs"][0]["properties"]["oviraptorSourceReview"]["review"],
            NativeSourceExportFormat::Bundle=>&exported["documents"]["json"]["review"],
        };
        assert!(review["ciGate"]["gaps"].as_array().unwrap().contains(&json!("dependency_graph_incomplete")));
        assert_eq!(review["independentReviewCompleted"],false);
        assert_eq!(review["coverage"]["materialDigest"],preparation["digest"]);
        assert_eq!(review["coverage"]["outstandingGaps"],preparation["outstandingGaps"]);
    }
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_coverage_preparation_distinguishes_selected_diff_from_full_snapshot() {
    let (root,connection,lease,result)=source_reviewer_execution_fixture_configured("all_confirmed",None,3,None,7,Some("diff"),|root,_,record,_| {
        run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001"))
    });
    let report=result.unwrap();
    let material=&report["coverageReviewPreparation"]["material"];
    assert_eq!(material["scope"]["requested"],"diff");
    assert_eq!(material["scope"]["effective"],"diff");
    let selected=material["scope"]["selectedFiles"].as_array().unwrap();
    assert_eq!(selected.len(),1);
    assert_eq!(selected[0]["path"],"app.py");
    assert!(material["snapshot"]["frozenFileCount"].as_u64().unwrap()>selected.len() as u64);
    assert_eq!(material["scope"]["selectedFilesAreCoverageProof"],false);
    assert_eq!(material["analyzerRunsArePerFileCoverageProof"],false);
    assert_eq!(material["rootRunId"],lease.root_run_id);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_coverage_preparation_zero_candidates_is_not_a_clean_bill_or_extra_call() {
    let (root,connection,lease,result)=source_reviewer_execution_fixture_using_calls("no_candidates",None,3,None,6,|root,_,record,_| {
        run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001"))
    });
    let report=result.unwrap();
    let preparation=&report["coverageReviewPreparation"];
    assert_eq!(preparation["status"],"prepared_not_reviewed");
    assert_eq!(preparation["material"]["candidateReview"]["status"],"not_applicable_no_candidates");
    assert_eq!(report["confirmedFindings"],0);
    assert_eq!(report["independentCandidateReviewCompleted"],false);
    assert_eq!(report["independentReviewCompleted"],false);
    assert!(preparation["outstandingGaps"].as_array().unwrap().contains(&json!("source_coverage_review")));
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_assignments WHERE coordinator_run_id=?1",
        [&lease.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),4);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_coverage_preparation_historical_read_is_stable_readonly_and_ignores_display_json() {
    use crate::agent_runtime::multi_agent::source_coverage;
    let (root,connection,lease,result)=source_reviewer_execution_fixture("valid",None);
    let expected=result.unwrap()["coverageReviewPreparation"].clone();
    let original:PathBuf=connection.query_row("SELECT source_path FROM source_scope_contracts WHERE scan_id=?1 AND attempt_number=1",
        [&lease.scan_id],|r|r.get::<_,String>(0)).unwrap().into();
    assert!(original.starts_with(&root) && original!=root);
    fs::rename(&original,root.join("moved-original-repository")).unwrap();
    connection.execute("UPDATE sentinel_scans SET status='completed',attempt_count=2 WHERE id=?1",[&lease.scan_id]).unwrap();
    connection.execute("UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01'",[]).unwrap();
    // A forged prepared display must never become coverage authority.
    connection.execute("UPDATE agent_events SET payload_json=json_set(payload_json,'$.coverageReviewPreparation.status','completed','$.coverageReviewPreparation.independentReviewCompleted',json('true'),'$.coverageReviewPreparation.outstandingGaps',json('[]')) WHERE event_type='terminal_reduced'",[]).unwrap();
    let before=connection.total_changes();
    assert_eq!(source_coverage::audit(&connection,&lease).unwrap_err(),"source_coverage_transaction_required");
    let tx=rusqlite::Transaction::new_unchecked(&connection,rusqlite::TransactionBehavior::Deferred).unwrap();
    assert_eq!(source_coverage::audit(&tx,&lease).unwrap().as_json(),expected);
    let mut wrong=lease.clone();wrong.attempt_number=2;
    assert!(source_coverage::audit(&tx,&wrong).is_err());
    tx.rollback().unwrap();
    assert_eq!(connection.total_changes(),before);
    let page=native_source_findings(&connection,&lease.scan_id,1,0,50).unwrap();
    assert_eq!(page["coverage"]["materialDigest"],expected["digest"]);
    assert_eq!(page["coverage"]["outstandingGaps"],expected["outstandingGaps"]);
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_capability_leases WHERE revoked_at=''",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_coverage_preparation_refuses_pending_candidate_delivery_and_never_replays() {
    use crate::agent_runtime::multi_agent::source_coverage;
    let fault="CREATE TRIGGER coverage_pending BEFORE INSERT ON agent_messages WHEN NEW.kind='source_review_result' BEGIN SELECT RAISE(ABORT,'hold candidate delivery'); END;";
    let (root,connection,lease,result)=source_reviewer_execution_fixture("valid",Some(fault));
    assert!(result.is_err());
    let before=connection.total_changes();
    let tx=rusqlite::Transaction::new_unchecked(&connection,rusqlite::TransactionBehavior::Deferred).unwrap();
    assert!(source_coverage::audit(&tx,&lease).is_err());
    tx.rollback().unwrap();
    assert_eq!(connection.total_changes(),before);
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_events WHERE run_id=?1 AND event_type='terminal_reduced'",
        [&lease.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_coverage_preparation_rejects_missing_phase_ack_tool_receipt_and_foreign_material() {
    use crate::agent_runtime::multi_agent::source_coverage;
    let (root,connection,lease,result)=source_reviewer_execution_fixture("all_confirmed",None);
    assert!(result.is_ok(),"{result:?}");
    for mutation in [
        "UPDATE agent_messages SET acknowledged_at='' WHERE kind='source_tool_result';",
        "UPDATE agent_messages SET acknowledged_at='' WHERE kind='source_review_result';",
        "DELETE FROM agent_source_tool_receipts WHERE tool_name='repo.read_slice';",
        "UPDATE agent_assignments SET task_slice_json=json_set(task_slice_json,'$.reviewMaterial.digest','changed') WHERE role='evidence_reviewer';",
        "UPDATE agent_runs SET used_requests=99 WHERE role='repo_mapper';",
        "UPDATE native_scan_plans SET plan_hash='foreign';",
    ] {
        let tx=rusqlite::Transaction::new_unchecked(&connection,rusqlite::TransactionBehavior::Immediate).unwrap();
        tx.execute_batch(mutation).unwrap();
        assert!(source_coverage::audit(&tx,&lease).is_err(),"accepted {mutation}");
        tx.rollback().unwrap();
    }
    drop(connection);fs::remove_dir_all(root).unwrap();
}
