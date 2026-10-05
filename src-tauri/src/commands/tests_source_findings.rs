#[test]
fn source_findings_bundle_export_is_one_complete_snapshot_and_imports_without_native_authority() {
    use crate::artifact_import::{limits::Limits, import_roots, import_sentinel_snapshot, ImportContext, BundleStatus};
    // Exercise the actual IPC format spelling, not a test-only exporter.
    let format:NativeSourceExportFormat=serde_json::from_value(json!("bundle")).expect("atomic bundle export must be supported");
    let (root,connection,lease,result)=source_reviewer_execution_fixture("all_confirmed",None);
    let expected=result.unwrap()["sourceDecisionProjection"]["findings"].as_array().unwrap().clone();
    let before=bundle_native_snapshot(&connection);
    let directory=root.join("atomic-source-reports");
    let path=write_native_source_findings_export_as(&connection,&directory,&lease.scan_id,1,format).unwrap();
    assert_eq!(fs::read_dir(&directory).unwrap().count(),1,"one publication, not independently visible reports");
    let bytes=fs::read(&path).unwrap();
    let bundle:JsonValue=serde_json::from_slice(&bytes).unwrap();
    assert_eq!(bundle["format"],"oviraptor-source-review-bundle-v1");
    let report=&bundle["documents"]["json"];
    let sarif=&bundle["documents"]["sarif"];
    assert_eq!(report["review"]["findings"],json!(expected));
    let mut summary=report.clone();summary["review"].as_object_mut().unwrap().remove("findings");
    assert_eq!(sarif["runs"][0]["properties"]["oviraptorSourceReview"],summary,"same timestamp, frozen policy and evidence");
    assert_eq!(bundle_native_snapshot(&connection),before);
    let public_root=root.join("public-bundle-import");
    let public_database=db::initialize(&public_root).unwrap();
    let public_connection=db::open(&public_database).unwrap();
    let public_before=bundle_native_snapshot(&public_connection);
    assert!(import_sentinel_project_path(&public_database,&public_root,&path).unwrap()>0);
    assert_eq!(historical_import_runs(&public_connection,None).unwrap()[0].finding_candidates,expected.len() as i64);
    assert_eq!(bundle_native_snapshot(&public_connection),public_before);
    drop(public_connection);
    for direct in [true,false] {
        let base=root.join(if direct {"manual-bundle"} else {"directory-bundle"});
        let imported=db::open(&db::initialize(&base).unwrap()).unwrap();
        let native_before=bundle_native_snapshot(&imported);
        let roots=vec![directory.clone()];let cas=base.join("cas");let key=base.join("key");let limits=Limits::default();
        let context=ImportContext{connection:&imported,cas_dir:&cas,key_path:&key,roots:&roots,limits:&limits};
        let import=|| if direct {import_sentinel_snapshot(&context,&bytes,Some(Path::new(&path))).unwrap()} else {import_roots(&context).outcomes.remove(0)};
        let outcome=import();assert_eq!(outcome.status,BundleStatus::Imported,"{outcome:?}");
        assert_eq!(historical_import_runs(&imported,None).unwrap()[0].finding_candidates,expected.len() as i64);
        let previews=historical_import_previews(&imported,&lease.scan_id).unwrap();
        assert!(previews.iter().all(|row|row.read_only && !row.execution_eligible && row.review_state=="unreviewed"));
        assert_eq!(import().status,BundleStatus::Unchanged);
        assert_eq!(bundle_native_snapshot(&imported),native_before);
    }
    assert_eq!(fs::read(path).unwrap(),bytes);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_findings_joint_json_sarif_import_uses_report_scope_and_one_candidate_per_decision() {
    use crate::artifact_import::{limits::Limits, import_roots, ImportContext, BundleStatus};
    let (root,connection,lease,result)=source_reviewer_execution_fixture("all_confirmed",None);
    let expected=result.unwrap()["sourceDecisionProjection"]["findings"].as_array().unwrap().clone();
    let directory=root.join("mixed-source-reports");
    let json_path=write_native_source_findings_export_as(&connection,&directory,&lease.scan_id,1,NativeSourceExportFormat::Json).unwrap();
    let sarif_path=write_native_source_findings_export_as(&connection,&directory,&lease.scan_id,1,NativeSourceExportFormat::Sarif).unwrap();
    let originals=[fs::read(&json_path).unwrap(),fs::read(&sarif_path).unwrap()];
    let imported=db::open(&db::initialize(&root.join("joint-app")).unwrap()).unwrap();
    let before=bundle_native_snapshot(&imported);
    let roots=vec![directory];let cas=root.join("joint-cas");let key=root.join("joint-key");let limits=Limits::default();
    let context=ImportContext{connection:&imported,cas_dir:&cas,key_path:&key,roots:&roots,limits:&limits};
    let summary=import_roots(&context);
    assert_eq!(summary.outcomes.len(),1,"{summary:?}");
    assert_eq!(summary.outcomes[0].status,BundleStatus::Imported,"{summary:?}");
    assert_eq!(imported.query_row("SELECT count(*) FROM import_bundle_files",[],|r|r.get::<_,i64>(0)).unwrap(),2,"both exported formats must actually be ingested");
    let runs=historical_import_runs(&imported,None).unwrap();
    assert_eq!(runs.len(),1,"one report scope, not a directory-derived task");
    assert_eq!(runs[0].scan_id,lease.scan_id);
    assert_eq!(runs[0].finding_candidates,expected.len() as i64);
    let previews=historical_import_previews(&imported,&lease.scan_id).unwrap();
    assert!(previews.iter().all(|row|row.read_only && !row.execution_eligible && row.review_state=="unreviewed"));
    let envelopes=imported.prepare("SELECT r.envelope_json FROM import_projection_memberships m JOIN import_record_revisions r ON r.id=m.revision_id WHERE m.current=1 AND r.record_kind='finding_candidate'").unwrap()
        .query_map([],|r|r.get::<_,String>(0)).unwrap().map(|r|serde_json::from_str::<JsonValue>(&r.unwrap()).unwrap()).collect::<Vec<_>>();
    assert_eq!(envelopes.len(),expected.len());
    for finding in &expected {
        let row=envelopes.iter().find(|row|row["extensions"]["sourceDecisionId"]==finding["sourceDecisionId"]).unwrap();
        assert_eq!(row["provenance"]["importAdapter"],"source_report");
        assert_eq!(row["producer"]["name"],"oviraptor-snapshot");
        let origins=row["fieldOrigins"].as_object().unwrap();
        assert!(!origins.is_empty());
        assert!(origins.values().all(|value|value.as_str().unwrap().starts_with("source_report:")));
        assert_eq!(row["extensions"]["sarif_result"]["properties"]["oviraptorSourceFinding"],*finding);
    }
    assert_eq!(import_roots(&context).outcomes[0].status,BundleStatus::Unchanged);
    assert_eq!(bundle_native_snapshot(&imported),before);
    assert_eq!(fs::read(json_path).unwrap(),originals[0]);assert_eq!(fs::read(sarif_path).unwrap(),originals[1]);
    drop(imported);drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_findings_ci_export_rebuilds_frozen_gate_for_completed_historical_attempt() {
    let (root,connection,lease,result)=source_reviewer_execution_fixture("all_confirmed",None);
    let expected=result.unwrap()["gate"].clone();
    assert!(expected.is_object());
    // History export must not require a live lease/current attempt, nor read
    // mutable task context as the release policy.
    connection.execute("UPDATE sentinel_scans SET status='completed',attempt_count=2 WHERE id=?1",[&lease.scan_id]).unwrap();
    connection.execute("UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01'",[]).unwrap();
    connection.execute("UPDATE sentinel_scan_contexts SET policy_json='{}'",[]).unwrap();
    let before=bundle_native_snapshot(&connection);
    let directory=root.join("ci-exports");
    for format in [NativeSourceExportFormat::Json,NativeSourceExportFormat::Sarif,NativeSourceExportFormat::Bundle] {
        let path=write_native_source_findings_export_as(&connection,&directory,&lease.scan_id,1,format).unwrap();
        assert!(Path::new(&path).file_name().unwrap().to_str().unwrap().starts_with("source-review-"));
        let report:JsonValue=serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        let bundle=match format {
            NativeSourceExportFormat::Json => &report,
            NativeSourceExportFormat::Sarif => &report["runs"][0]["properties"]["oviraptorSourceReview"],
            NativeSourceExportFormat::Bundle => &report["documents"]["json"],
        };
        assert_eq!(bundle["format"],"oviraptor-source-review-v1");
        assert_eq!(bundle["schemaVersion"],1);
        let gate=&bundle["review"]["ciGate"];
        for field in ["freeze","policy","status","exitCode","reasons","gaps","counts","scanId","attemptNumber","rootRunId"] {
            assert_eq!(gate[field],expected[field],"missing or changed CI field {field}");
        }
        assert_eq!(gate["independentReviewCompleted"],false);
        assert!(gate["gaps"].as_array().unwrap().contains(&json!("source_coverage_review")));
        assert_ne!(gate["status"],"passed");
        assert!(gate.get("findings").is_none(),"summary must not duplicate findings");
        assert!(gate.get("sourceDecisionProjection").is_none());
        assert_eq!(bundle["executionEligible"],false);
    }
    assert_eq!(bundle_native_snapshot(&connection),before,"export is read-only and never renews leases");
    assert!(load_workbench_ci_policy(&connection,&lease.scan_id,1).is_err(),"history support must not authorize execution");
    connection.execute("UPDATE sentinel_scans SET status='scanning' WHERE id=?1",[&lease.scan_id]).unwrap();
    connection.execute("INSERT INTO sentinel_scan_attempts(scan_id,attempt_number,status) VALUES(?1,2,'scanning')",[&lease.scan_id]).unwrap();
    store_workbench_ci_policy(&connection,&lease.scan_id,2,&json!({"maxCritical":9,"maxHigh":99,"blockRelease":true})).unwrap();
    let new_policy=load_workbench_ci_policy(&connection,&lease.scan_id,2).unwrap().as_json();
    assert_ne!(new_policy,expected["policy"]);
    let old=native_source_findings_export_snapshot(&connection,&lease.scan_id,1).unwrap();
    assert_eq!(old["ciGate"]["policy"],expected["policy"],"new attempt policy cannot rewrite history");
    assert!(native_source_findings_export_snapshot(&connection,&lease.scan_id,2).is_err());
    assert_eq!(load_historical_source_ci_policy(&connection,&lease.scan_id,1).unwrap_err(),"source_ci_transaction_required");
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_findings_ci_export_rejects_missing_policy_and_corrupt_freeze_without_publication() {
    let (root,connection,lease,result)=source_reviewer_execution_fixture("all_confirmed",None);
    result.unwrap();
    let directory=root.join("must-not-publish-ci");
    for mutation in [
        "DROP TRIGGER source_ci_policy_no_delete; DELETE FROM source_ci_policies;",
        "UPDATE source_snapshots SET commit_sha='forged-head';",
        "UPDATE source_snapshots SET base_sha='forged-base';",
        "UPDATE source_snapshots SET manifest_json='[]';",
        "DROP TRIGGER source_decision_no_update; UPDATE agent_source_review_decisions SET record_json='{}' WHERE id=(SELECT id FROM agent_source_review_decisions ORDER BY id DESC LIMIT 1);",
        "INSERT INTO sentinel_deleted_scans(scan_id) SELECT id FROM sentinel_scans;",
    ] {
        connection.execute_batch("SAVEPOINT ci_export_fault").unwrap();
        connection.execute_batch(mutation).unwrap();
        let before=source_reviewer_reentry_state(&connection);
        for format in [NativeSourceExportFormat::Json,NativeSourceExportFormat::Sarif,NativeSourceExportFormat::Bundle] {
            assert!(write_native_source_findings_export_as(&connection,&directory,&lease.scan_id,1,format).is_err(),"export accepted {mutation}");
            assert!(!directory.exists(),"failed audit must not publish or create a directory");
        }
        assert_eq!(source_reviewer_reentry_state(&connection),before);
        connection.execute_batch("ROLLBACK TO ci_export_fault; RELEASE ci_export_fault").unwrap();
    }
    assert!(native_source_findings_export_snapshot(&connection,&lease.scan_id,1).is_ok());
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_findings_read_api_uses_audited_decisions_and_preserves_history_without_writes() {
    for mode in ["confirmed","graph_confirmed","rejected","valid"] {
        let (root,connection,lease,result)=source_reviewer_execution_fixture(mode,None);
        let result=result.unwrap();
        let before=source_reviewer_reentry_state(&connection);
        let page=native_source_findings(&connection,&lease.scan_id,1,0,1).unwrap();
        assert_eq!(page["status"],"audited");
        assert_eq!(page["counts"],result["sourceDecisionProjection"]["counts"]);
        assert_eq!(page["findings"],result["sourceDecisionProjection"]["findings"]);
        assert_eq!(page["rootRunId"],lease.root_run_id);
        assert_eq!(page["independentCandidateReviewCompleted"],true);
        assert_eq!(page["independentReviewCompleted"],false);
        assert!(page["nextOffset"].is_null());
        assert!(native_source_findings(&connection,&lease.scan_id,1,99,1).unwrap()["findings"].as_array().unwrap().is_empty());
        for (attempt,offset,limit) in [(0,0,1),(1,-1,1),(1,0,0),(1,0,101)] {
            assert_eq!(native_source_findings(&connection,&lease.scan_id,attempt,offset,limit).unwrap_err(),"invalid_source_findings_page");
        }
        assert_eq!(native_source_findings(&connection,&lease.scan_id,2,0,1).unwrap_err(),"attempt_not_found");
        assert_eq!(native_source_findings(&connection,"foreign-scan",1,0,1).unwrap_err(),"attempt_not_found");
        assert_eq!(source_reviewer_reentry_state(&connection),before);
        connection.execute("UPDATE sentinel_scans SET attempt_count=2,status='scanning' WHERE id=?1",[&lease.scan_id]).unwrap();
        connection.execute("INSERT INTO sentinel_scan_attempts(scan_id,attempt_number,status) VALUES(?1,2,'scanning')",[&lease.scan_id]).unwrap();
        connection.execute("UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01'",[]).unwrap();
        let before=source_reviewer_reentry_state(&connection);
        assert_eq!(native_source_findings(&connection,&lease.scan_id,1,0,1).unwrap(),page,"historical reads do not require live execution grants");
        let current=native_source_findings(&connection,&lease.scan_id,2,0,1).unwrap();
        assert_eq!(current["status"],"not_available");
        assert!(current["counts"].is_null(),"not reviewed is not zero findings");
        assert_eq!(source_reviewer_reentry_state(&connection),before);
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_findings_read_api_rejects_tampered_and_deleted_evidence_without_fallback() {
    let (root,connection,lease,result)=source_reviewer_execution_fixture("confirmed",None);
    result.unwrap();
    for sql in [
        "UPDATE agent_source_review_decisions SET record_json='{}'",
        "DELETE FROM agent_source_review_decisions",
        "UPDATE agent_messages SET acknowledged_at='' WHERE kind='source_review_result'",
        "UPDATE agent_specialist_calls SET response_json='{}' WHERE role='evidence_reviewer'",
        "DELETE FROM agent_coordinator_leases",
        "UPDATE agent_coordinator_leases SET fencing_token='foreign-fence'",
    ] {
        connection.execute_batch("SAVEPOINT source_read_fault").unwrap();
        // Simulate on-disk corruption beyond the normal immutable-write guard;
        // the savepoint rollback restores both data and protective triggers.
        connection.execute_batch("DROP TRIGGER source_decision_no_update; DROP TRIGGER source_decision_no_delete; DROP TRIGGER agent_specialist_call_immutable").unwrap();
        connection.execute_batch(sql).unwrap();
        let before=source_reviewer_reentry_state(&connection);
        let page=native_source_findings(&connection,&lease.scan_id,1,0,1).unwrap();
        assert_eq!(page["status"],"unverified","{sql}");
        assert_eq!(page["findings"],json!([]));
        assert!(page["counts"].is_null());
        assert_eq!(page["independentCandidateReviewCompleted"],false);
        assert_eq!(source_reviewer_reentry_state(&connection),before);
        connection.execute_batch("ROLLBACK TO source_read_fault; RELEASE source_read_fault").unwrap();
    }
    connection.execute("INSERT INTO sentinel_deleted_scans(scan_id) VALUES(?1)",[&lease.scan_id]).unwrap();
    let before=source_reviewer_reentry_state(&connection);
    assert_eq!(native_source_findings(&connection,&lease.scan_id,1,0,1).unwrap_err(),"attempt_not_found");
    assert_eq!(source_reviewer_reentry_state(&connection),before);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_findings_read_api_pages_only_after_auditing_the_entire_set() {
    let (root,connection,lease,result)=source_reviewer_execution_fixture("all_confirmed",None);
    let result=result.unwrap();
    let expected=result["sourceDecisionProjection"]["findings"].as_array().unwrap();
    assert!(expected.len()>1);
    let before=source_reviewer_reentry_state(&connection);
    let first=native_source_findings(&connection,&lease.scan_id,1,0,1).unwrap();
    assert_eq!(first["findings"],json!([expected[0]]));
    assert_eq!(first["nextOffset"],1);
    let second=native_source_findings(&connection,&lease.scan_id,1,1,100).unwrap();
    assert_eq!(second["findings"],json!(expected[1..]));
    assert!(second["nextOffset"].is_null());
    assert_eq!(second["counts"],first["counts"]);
    assert_eq!(source_reviewer_reentry_state(&connection),before);
    connection.execute_batch("DROP TRIGGER source_decision_no_update").unwrap();
    connection.execute("UPDATE agent_source_review_decisions SET record_json='{}' WHERE id=?1",
        [expected[1]["id"].as_str().unwrap()]).unwrap();
    let before=source_reviewer_reentry_state(&connection);
    assert_eq!(native_source_findings(&connection,&lease.scan_id,1,0,1).unwrap()["status"],"unverified","off-page corruption invalidates page zero too");
    assert_eq!(source_reviewer_reentry_state(&connection),before);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_findings_read_api_does_not_upgrade_legacy_review_or_incomplete_delivery() {
    let (root,connection,lease,result)=source_reviewer_execution_fixture_using_calls("confirmed",None,3,None,6,
        |root,connection,record,lease| {
            source_reviewer_checkpoint_fixture(root,connection,record,lease,"before_review");
            Ok(json!({}))
        });
    result.unwrap();
    let before=source_reviewer_reentry_state(&connection);
    let page=native_source_findings(&connection,&lease.scan_id,1,0,50).unwrap();
    assert_eq!(page["status"],"unverified");
    assert_eq!(page["findings"],json!([]));
    assert_eq!(source_reviewer_reentry_state(&connection),before);
    drop(connection);fs::remove_dir_all(root).unwrap();
    let (root,connection,lease,result)=source_reviewer_execution_fixture_schema("valid",None,2);
    result.unwrap();
    let before=source_reviewer_reentry_state(&connection);
    assert_eq!(native_source_findings(&connection,&lease.scan_id,1,0,50).unwrap()["status"],"unverified");
    assert_eq!(source_reviewer_reentry_state(&connection),before);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_findings_read_api_rejects_ambiguous_roots_and_missing_frozen_bytes() {
    let (root,connection,lease,result)=source_reviewer_execution_fixture("confirmed",None);
    result.unwrap();
    let expected=native_source_findings(&connection,&lease.scan_id,1,0,50).unwrap();
    assert_eq!(expected["status"],"audited");
    connection.execute_batch("SAVEPOINT ambiguous_source_root").unwrap();
    connection.execute("INSERT INTO agent_runs(id,scan_id,attempt_number,target_url) VALUES('ambiguous-source-root',?1,1,?2)",
        params![lease.scan_id,lease.target_key]).unwrap();
    let before=source_reviewer_reentry_state(&connection);
    let page=native_source_findings(&connection,&lease.scan_id,1,0,50).unwrap();
    assert_eq!(page["status"],"unverified");
    assert!(page["rootRunId"].is_null(),"never select an arbitrary root");
    assert_eq!(page["findings"],json!([]));
    assert_eq!(source_reviewer_reentry_state(&connection),before);
    connection.execute_batch("ROLLBACK TO ambiguous_source_root; RELEASE ambiguous_source_root").unwrap();
    let (view,_,_)=crate::agent_runtime::multi_agent::source::historical_materials(&connection,&lease.scan_id,1).unwrap();
    let frozen=view.repository().join("app.py");
    let retained=root.join("retained-source-read-file");
    fs::rename(&frozen,&retained).unwrap();
    let before=source_reviewer_reentry_state(&connection);
    let page=native_source_findings(&connection,&lease.scan_id,1,0,50).unwrap();
    assert_eq!(page["status"],"unverified");
    assert_eq!(page["findings"],json!([]));
    assert!(page["counts"].is_null());
    assert_eq!(source_reviewer_reentry_state(&connection),before);
    fs::rename(&retained,&frozen).unwrap();
    assert_eq!(native_source_findings(&connection,&lease.scan_id,1,0,50).unwrap(),expected);
    assert_eq!(source_reviewer_reentry_state(&connection),before);
    drop(connection);fs::remove_dir_all(root).unwrap();
}
#[test]
fn source_findings_sarif_roundtrip_preserves_real_review_without_native_authority() {
    use crate::artifact_import::{limits::Limits, import_roots, ImportContext, BundleStatus};
    let (root,connection,lease,result)=source_reviewer_execution_fixture("all_confirmed",None);
    let execution=result.unwrap();
    let expected=execution["sourceDecisionProjection"]["findings"].as_array().unwrap().clone();
    assert!(expected.len()>1);
    let directory=root.join("sarif-exports");
    let before=bundle_native_snapshot(&connection);
    let path=write_native_source_findings_export_as(&connection,&directory,&lease.scan_id,1,NativeSourceExportFormat::Sarif).unwrap();
    assert!(path.ends_with(".sarif"));
    let bytes=fs::read(&path).unwrap();
    let report:JsonValue=serde_json::from_slice(&bytes).unwrap();
    assert_eq!(report["version"],"2.1.0");
    let results=report["runs"][0]["results"].as_array().unwrap();
    assert_eq!(results.len(),expected.len());
    for (row,finding) in results.iter().zip(&expected) {
        assert_eq!(row["properties"]["oviraptorSourceFinding"],*finding);
        assert_eq!(row["kind"],"fail");
        assert_eq!(row["properties"]["severity"],finding["severity"]);
    }
    let context=&report["runs"][0]["properties"]["oviraptorSourceReview"];
    assert_eq!(context["executionEligible"],false);
    assert_eq!(context["coverageReviewCompleted"],false);
    assert_eq!(context["review"]["scanId"],lease.scan_id);
    assert_eq!(context["review"]["ciGate"]["freeze"],execution["gate"]["freeze"]);
    assert_eq!(context["review"]["ciGate"]["policy"],execution["gate"]["policy"]);
    assert!(context["review"].get("findings").is_none(),"avoid quadratic duplication on import");
    let second=write_native_source_findings_export_as(&connection,&directory,&lease.scan_id,1,NativeSourceExportFormat::Sarif).unwrap();
    assert_ne!(path,second);
    assert_eq!(fs::read(&path).unwrap(),bytes);
    assert_eq!(bundle_native_snapshot(&connection),before);

    let app=root.join("roundtrip-app");
    let imported_path=db::initialize(&app).unwrap();
    let imported=db::open(&imported_path).unwrap();
    let baseline=bundle_native_snapshot(&imported);
    let roots=vec![directory];
    let cas=root.join("roundtrip-cas");let key=root.join("roundtrip-key");
    let limits=Limits::default();
    let import=ImportContext {connection:&imported,cas_dir:&cas,key_path:&key,roots:&roots,limits:&limits};
    let summary=import_roots(&import);
    assert_eq!(summary.outcomes.len(),1,"{summary:?}");
    assert_eq!(summary.outcomes[0].status,BundleStatus::Imported,"{summary:?}");
    let envelopes=imported.prepare("SELECT envelope_json FROM import_record_revisions WHERE record_kind='finding_candidate'").unwrap()
        .query_map([],|r|r.get::<_,String>(0)).unwrap().map(|r|serde_json::from_str::<JsonValue>(&r.unwrap()).unwrap()).collect::<Vec<_>>();
    assert_eq!(envelopes.len(),expected.len(),"two exported views of the same decisions do not duplicate findings");
    for finding in &expected {
        let row=envelopes.iter().find(|row|row["payload"]["rule_id"]==finding["sourceDecisionId"]).unwrap();
        assert_eq!(row["payload"]["severity"],finding["severity"]);
        assert_eq!(row["extensions"]["sarif_result"]["properties"]["oviraptorSourceFinding"],*finding);
    }
    let headers=imported.prepare("SELECT envelope_json FROM import_record_revisions WHERE record_kind='run_state'").unwrap()
        .query_map([],|r|r.get::<_,String>(0)).unwrap().map(|r|serde_json::from_str::<JsonValue>(&r.unwrap()).unwrap()).collect::<Vec<_>>();
    assert_eq!(headers.len(),2,"each exported file retains one run summary, not a copy per finding");
    for header in headers {
        assert_eq!(header["payload"]["status"],"imported");
        assert_eq!(header["extensions"]["sarif_run_properties"]["oviraptorSourceReview"]["review"]["materialDigest"],expected[0]["materialDigest"]);
        assert_eq!(header["extensions"]["sarif_run_properties"]["oviraptorSourceReview"]["review"]["ciGate"],context["review"]["ciGate"]);
    }
    let runs=historical_import_runs(&imported,None).unwrap();
    assert!(!runs.is_empty());
    for run in runs {
        let previews=historical_import_previews(&imported,&run.scan_id).unwrap();
        assert!(previews.iter().all(|row|row.read_only && !row.execution_eligible && row.review_state=="unreviewed"));
    }
    assert_eq!(bundle_native_snapshot(&imported),baseline,"imports cannot create decisions, leases, runs, grants or native findings");
    assert_eq!(import_roots(&import).outcomes[0].status,BundleStatus::Unchanged);
    assert_eq!(fs::read(&path).unwrap(),bytes);
    drop(imported);drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_findings_sarif_reaudits_all_decisions_and_preserves_audited_zero() {
    for mode in ["all_confirmed","rejected"] {
        let (root,connection,lease,result)=source_reviewer_execution_fixture(mode,None);
        result.unwrap();
        let directory=root.join("exports");
        let path=write_native_source_findings_export_as(&connection,&directory,&lease.scan_id,1,NativeSourceExportFormat::Sarif).unwrap();
        let report:JsonValue=serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(report["runs"][0]["results"].as_array().unwrap().is_empty(),mode=="rejected");
        assert_eq!(report["runs"][0]["properties"]["oviraptorSourceReview"]["review"]["status"],"audited");
        let path=write_native_source_findings_export_as(&connection,&directory,&lease.scan_id,1,NativeSourceExportFormat::Bundle).unwrap();
        let container:JsonValue=serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        let (_,paired_sarif)=crate::artifact_import::report_bundle::validate(&container).unwrap();
        assert_eq!(paired_sarif["runs"][0]["results"].as_array().unwrap().is_empty(),mode=="rejected");
        connection.execute_batch("DROP TRIGGER source_decision_no_update; UPDATE agent_source_review_decisions SET record_json='{}' WHERE id=(SELECT id FROM agent_source_review_decisions ORDER BY id DESC LIMIT 1)").unwrap();
        let refused=root.join("must-not-exist");
        assert_eq!(write_native_source_findings_export_as(&connection,&refused,&lease.scan_id,1,NativeSourceExportFormat::Sarif).unwrap_err(),"source_findings_export_unverified");
        assert!(!refused.exists());
        assert_eq!(write_native_source_findings_export_as(&connection,&refused,&lease.scan_id,1,NativeSourceExportFormat::Bundle).unwrap_err(),"source_findings_export_unverified");
        assert!(!refused.exists());
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
    assert!(serde_json::from_str::<NativeSourceExportFormat>("\"shell\"").is_err());
}

#[test]
fn source_findings_sarif_serialization_encodes_paths_and_preserves_severity() {
    let rows=["critical","high","medium","low","informational"].iter().enumerate().map(|(i,severity)|json!({
        "id":format!("d{i}"),"sourceDecisionId":format!("d{i}"),"candidateDigest":"digest",
        "severity":severity,"title":"","path":if i==0 {"目录/a #?.rs"} else {""},"line":0
    })).collect::<Vec<_>>();
    let report=native_source_review_sarif(&json!({"review":{"findings":rows}})).unwrap();
    let results=report["runs"][0]["results"].as_array().unwrap();
    assert_eq!(results[0]["locations"][0]["physicalLocation"]["artifactLocation"]["uri"],"%E7%9B%AE%E5%BD%95/a%20%23%3F.rs");
    assert!(results[0]["locations"][0]["physicalLocation"].get("region").is_none());
    for (row,original) in results.iter().zip(rows) {
        assert_eq!(row["properties"]["oviraptorSourceFinding"],original);
        assert!(!row["message"]["text"].as_str().unwrap().is_empty());
        if original["path"]=="" {assert_eq!(row["locations"],json!([]));}
    }
}

#[test]
fn source_findings_export_contains_full_audited_set_and_no_execution_grant() {
    let (root,connection,lease,result)=source_reviewer_execution_fixture("all_confirmed",None);
    let expected=result.unwrap()["sourceDecisionProjection"]["findings"].clone();
    assert!(expected.as_array().unwrap().len()>1);
    let first=native_source_findings(&connection,&lease.scan_id,1,0,1).unwrap();
    assert!(!first["nextOffset"].is_null());
    let directory=root.join("exports");
    let before=source_reviewer_reentry_state(&connection);
    let path=write_native_source_findings_export(&connection,&directory,&lease.scan_id,1).unwrap();
    let bytes=fs::read(&path).unwrap();
    let bundle:JsonValue=serde_json::from_slice(&bytes).unwrap();
    assert_eq!(bundle["format"],"oviraptor-source-review-v1");
    assert_eq!(bundle["qualification"],"verified_at_export");
    assert_eq!(bundle["executionEligible"],false);
    assert_eq!(bundle["coverageReviewCompleted"],false);
    assert!(bundle.get("scan").is_none(),"not a legacy task import bundle");
    assert_eq!(bundle["review"]["scanId"],lease.scan_id);
    assert_eq!(bundle["review"]["attemptNumber"],1);
    assert_eq!(bundle["review"]["findings"],expected);
    assert_eq!(bundle["review"]["status"],"audited");
    assert_eq!(bundle["review"]["offset"],0);
    assert!(bundle["review"]["nextOffset"].is_null());
    assert_eq!(bundle["review"]["independentReviewCompleted"],false);
    let second=write_native_source_findings_export(&connection,&directory,&lease.scan_id,1).unwrap();
    assert_ne!(path,second,"reports never overwrite each other");
    assert_eq!(fs::read(&path).unwrap(),bytes);
    assert_eq!(fs::read_dir(&directory).unwrap().count(),2);
    assert_eq!(source_reviewer_reentry_state(&connection),before);
    connection.execute("UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01'",[]).unwrap();
    let before=source_reviewer_reentry_state(&connection);
    assert!(write_native_source_findings_export(&connection,&directory,&lease.scan_id,1).is_ok());
    assert_eq!(source_reviewer_reentry_state(&connection),before,"historical export never renews a lease");
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_findings_export_reaudits_off_page_corruption_and_refuses_deleted_or_missing_attempts() {
    let (root,connection,lease,result)=source_reviewer_execution_fixture("all_confirmed",None);
    let expected=result.unwrap()["sourceDecisionProjection"]["findings"].clone();
    assert_eq!(native_source_findings(&connection,&lease.scan_id,1,0,1).unwrap()["status"],"audited");
    let directory=root.join("must-not-be-created");
    connection.execute_batch("SAVEPOINT export_fault; DROP TRIGGER source_decision_no_update").unwrap();
    connection.execute("UPDATE agent_source_review_decisions SET record_json='{}' WHERE id=?1",
        [expected[1]["id"].as_str().unwrap()]).unwrap();
    let before=source_reviewer_reentry_state(&connection);
    assert_eq!(write_native_source_findings_export(&connection,&directory,&lease.scan_id,1).unwrap_err(),"source_findings_export_unverified");
    assert!(!directory.exists());
    assert_eq!(source_reviewer_reentry_state(&connection),before);
    connection.execute_batch("ROLLBACK TO export_fault; RELEASE export_fault").unwrap();
    for (scan,attempt) in [(lease.scan_id.as_str(),0),(lease.scan_id.as_str(),2),("../outside",1)] {
        assert!(write_native_source_findings_export(&connection,&directory,scan,attempt).is_err());
        assert!(!directory.exists());
    }
    connection.execute("INSERT INTO sentinel_deleted_scans(scan_id) VALUES(?1)",[&lease.scan_id]).unwrap();
    let before=source_reviewer_reentry_state(&connection);
    assert_eq!(write_native_source_findings_export(&connection,&directory,&lease.scan_id,1).unwrap_err(),"attempt_not_found");
    assert!(!directory.exists());
    assert_eq!(source_reviewer_reentry_state(&connection),before);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_findings_export_distinguishes_audited_zero_from_legacy_and_handles_io_failure() {
    for (schema,mode) in [(3,"rejected"),(2,"valid")] {
        let (root,connection,lease,result)=source_reviewer_execution_fixture_schema(mode,None,schema);
        result.unwrap();
        let directory=root.join("exports");
        let before=source_reviewer_reentry_state(&connection);
        let exported=write_native_source_findings_export(&connection,&directory,&lease.scan_id,1);
        if schema==2 {
            assert_eq!(exported.unwrap_err(),"source_findings_export_unverified");
            assert!(!directory.exists());
        } else {
            let bundle:JsonValue=serde_json::from_slice(&fs::read(exported.unwrap()).unwrap()).unwrap();
            assert_eq!(bundle["review"]["counts"]["confirmed"],0);
            assert_eq!(bundle["review"]["findings"],json!([]));
            let blocked=root.join("not-a-directory");fs::write(&blocked,"retained user bytes").unwrap();
            assert_eq!(write_native_source_findings_export(&connection,&blocked,&lease.scan_id,1).unwrap_err(),"source_findings_export_directory_failed");
            assert_eq!(fs::read_to_string(blocked).unwrap(),"retained user bytes");
        }
        assert_eq!(source_reviewer_reentry_state(&connection),before);
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}
