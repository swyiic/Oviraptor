include!("tests_source_broker_historical_context.rs");

fn source_result_sarif(engine: &str, path: &str, title: &str) -> String {
    json!({"version":"2.1.0","runs":[{"tool":{"driver":{"name":engine}},"results":[{
        "ruleId":title,"level":"warning","message":{"text":title},
        "locations":[{"physicalLocation":{"artifactLocation":{"uri":path},"region":{"startLine":1}}}]
    }]}]}).to_string()
}

fn source_result_outcome(engine:&str,scratch:&Path,path:&str,title:&str)->analyzer::AnalyzerOutcome {
    let mut outcome=source_regression_outcome(engine,scratch);
    let bytes=source_result_sarif(engine,path,title);
    fs::write(outcome.sarif_path.as_ref().unwrap(),&bytes).unwrap();
    outcome.sarif_sha256=crate::artifact_import::canonical::sha256_hex(bytes.as_bytes());outcome
}

fn source_result_identical_engine_fixture() -> (PathBuf,rusqlite::Connection,WorkbenchStartRecord,JsonValue) {
    let (root,connection,record)=analysis_view_fixture("full",true);
    let mut calls=Vec::new();
    let report=analysis_view_run(&root,&record,|_,engine,_,_,scratch,_| {
        calls.push(engine.to_string());
        let mut outcome=source_regression_outcome(engine,scratch);
        // Two independently invoked analyzers may emit byte-identical reports.
        // Their accepted filenames/engine receipts, not the embedded driver, own attribution.
        let bytes=source_result_sarif("shared-driver","app.py","shared-candidate");
        fs::write(outcome.sarif_path.as_ref().unwrap(),&bytes).unwrap();
        outcome.sarif_sha256=crate::artifact_import::canonical::sha256_hex(bytes.as_bytes());
        Ok(outcome)
    }).unwrap();
    assert_eq!(calls,vec!["semgrep","codeql"]);
    (root,connection,record,report)
}

fn source_result_reimport_selected_engines(root:&Path,connection:&rusqlite::Connection,record:&WorkbenchStartRecord,engines:&[&str])->crate::artifact_import::ImportSummary {
    let directory=root.join(format!("provenance-import-{}",Uuid::new_v4()));
    fs::create_dir(&directory).unwrap();
    for engine in engines {
        let original=root.join("attempt-0001/source-analysis/analyzer-output").join(engine).join(format!("{engine}.sarif"));
        fs::copy(original,directory.join(format!("{engine}.sarif"))).unwrap();
    }
    fs::write(directory.join(".oviraptor-scan-id"),&record.scan_id).unwrap();
    let roots=vec![directory];let limits=crate::artifact_import::Limits::default();
    let summary=crate::artifact_import::import_roots(&crate::artifact_import::ImportContext {
        connection,cas_dir:&root.join("provenance-cas"),key_path:&root.join("provenance-key"),roots:&roots,limits:&limits,
    });
    assert!(!summary.outcomes.is_empty());
    assert!(summary.outcomes.iter().all(|b|b.committed_records.is_some() && b.status!=crate::artifact_import::BundleStatus::Failed));
    summary
}


#[test]
fn source_result_same_hash_never_accepts_an_unimported_engine() {
    let (root,connection,record)=analysis_view_fixture("diff",true);
    analysis_view_run(&root,&record,|_,engine,_,_,scratch,_|Ok(source_result_outcome(engine,scratch,"app.py","candidate"))).unwrap();
    let view=source_result_view(&connection,&record);
    let receipt=crate::native_pipeline::results::AnalysisResults::load(&connection,&view).unwrap();
    let summary=source_result_reimport_selected_engines(&root,&connection,&record,&["semgrep"]);
    let mut artifacts=receipt.artifacts.clone();
    let mut unimported=artifacts[0].clone();unimported.engine="codeql".into();
    // Hash equality cannot prove that the second engine's file was accepted.
    artifacts.insert(0,unimported);
    let captured=crate::native_pipeline::results::AnalysisResults::capture(&connection,&view,artifacts,&summary,&[],&[]).unwrap();
    assert!(captured.gaps.contains(&"analysis_result_artifact_not_accepted:codeql".into()));
    assert!(captured.records.iter().all(|r|r.engine=="semgrep"));
    assert!(!captured.records.is_empty());
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_result_same_bytes_attribution_does_not_depend_on_artifact_order() {
    let (root,connection,record,_)=source_result_identical_engine_fixture();
    let view=source_result_view(&connection,&record);
    let receipt=crate::native_pipeline::results::AnalysisResults::load(&connection,&view).unwrap();
    let summary=source_result_reimport_selected_engines(&root,&connection,&record,&["semgrep","codeql"]);
    let first=crate::native_pipeline::results::AnalysisResults::capture(&connection,&view,receipt.artifacts.clone(),&summary,&[],&[]).unwrap();
    let mut reversed=receipt.artifacts.clone();reversed.reverse();
    let second=crate::native_pipeline::results::AnalysisResults::capture(&connection,&view,reversed,&summary,&[],&[]).unwrap();
    assert_eq!(first.records,second.records,"first matching hash must not decide attribution");
    assert_eq!(first.gaps,second.gaps);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_result_legacy_single_origin_is_not_inferred_from_matching_hashes() {
    let (root,connection,record,_)=source_result_identical_engine_fixture();
    let view=source_result_view(&connection,&record);
    let mut legacy=crate::native_pipeline::results::AnalysisResults::load(&connection,&view).unwrap();
    // v1 receipts created before this fix may contain just one engine's origin
    // even though both artifact hashes match. Reading must not add evidence.
    legacy.records.retain(|r|r.engine=="semgrep");
    assert_eq!(legacy.artifacts.len(),2);
    let original_json=serde_json::to_string(&legacy).unwrap();
    let original_digest=legacy.digest();
    let decoded:crate::native_pipeline::results::AnalysisResults=serde_json::from_str(&original_json).unwrap();
    let candidates=decoded.candidates(&connection).unwrap();
    assert_eq!(candidates.len(),1);
    assert_eq!(candidates[0].sources.len(),1);
    assert_eq!(candidates[0].receipt.engine,"semgrep");
    assert_eq!(serde_json::to_string(&decoded).unwrap(),original_json);
    assert_eq!(decoded.digest(),original_digest,"new projection fields are not written into the immutable v1 receipt");
    drop(connection);fs::remove_dir_all(root).unwrap();
}


#[test]
fn source_result_grouping_keeps_different_revisions_separate() {
    let (root,connection,record)=analysis_view_fixture("diff",true);
    analysis_view_run(&root,&record,|_,engine,_,_,scratch,_|Ok(source_result_outcome(engine,scratch,"app.py","same-rule"))).unwrap();
    let view=source_result_view(&connection,&record);
    let mut receipt=crate::native_pipeline::results::AnalysisResults::load(&connection,&view).unwrap();
    let first=receipt.records.iter().find(|r|r.record.record_kind=="finding_candidate").unwrap().clone();
    // A separate real import accepts a changed revision of the same logical
    // candidate. Grouping must never disguise these two revisions as one.
    let output=root.join("attempt-0001/source-analysis/analyzer-output/semgrep/semgrep.sarif");
    let mut report:JsonValue=serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    report["runs"][0]["results"][0]["level"]=json!("error");
    fs::write(&output,report.to_string()).unwrap();
    let summary=source_result_reimport_selected_engines(&root,&connection,&record,&["semgrep"]);
    let bundle=summary.outcomes.iter().find(|b|b.committed_records.as_ref().unwrap().iter().any(|r|r.record_kind=="finding_candidate")).unwrap();
    let newer=bundle.committed_records.as_ref().unwrap().iter().find(|r|r.record_kind=="finding_candidate").unwrap().clone();
    assert_eq!(newer.logical_key,first.record.logical_key);
    assert_ne!(newer.revision_hash,first.record.revision_hash);
    receipt.records.push(crate::native_pipeline::results::AcceptedResult {bundle_id:bundle.bundle_id.clone(),engine:"semgrep".into(),record:newer});
    let candidates=receipt.candidates(&connection).unwrap();
    assert_eq!(candidates.len(),2);
    assert!(candidates.iter().all(|c|c.sources.len()==1));
    assert_ne!(candidates[0].receipt.record.revision_hash,candidates[1].receipt.record.revision_hash);
    // The live immutable receipt was not amended by historical import or by
    // this read projection; its candidate remains the actually frozen one.
    let frozen=crate::native_pipeline::results::AnalysisResults::load(&connection,&view).unwrap();
    assert_eq!(frozen.candidates(&connection).unwrap().len(),1);
    assert_eq!(frozen.candidates(&connection).unwrap()[0].receipt.record,first.record);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

fn source_result_import_history(root: &Path, connection: &rusqlite::Connection, scan_id: &str) -> String {
    use crate::artifact_import::{ImportContext,Limits,import_roots};
    let history=root.join("late-history");fs::create_dir(&history).unwrap();
    fs::write(history.join("results.sarif"),source_result_sarif("historical","untouched.py","late-history")).unwrap();
    fs::write(history.join(".oviraptor-scan-id"),scan_id).unwrap();
    let roots=vec![history];let limits=Limits::default();
    let summary=import_roots(&ImportContext{connection,cas_dir:&root.join("history-cas"),key_path:&root.join("history-key"),roots:&roots,limits:&limits});
    assert!(!summary.outcomes.is_empty());
    connection.query_row("SELECT r.logical_key FROM import_projection_memberships m JOIN import_record_revisions r ON r.id=m.revision_id WHERE m.scope_key=?1 AND m.current=1 AND m.tombstone=0 AND r.record_kind='finding_candidate'",[format!("scan={scan_id}")],|r|r.get(0)).unwrap()
}


fn source_result_view(connection:&rusqlite::Connection,record:&WorkbenchStartRecord)->SourceAnalysisView {
    let scope=load_workbench_source_scope(connection,&record.scan_id,1).unwrap();
    let snapshot=RepositorySnapshot::restore(connection,&record.scan_id,1).unwrap().unwrap();
    let manifest=AnalysisManifest::select(&snapshot,&record.scan_id,1,&json!(scope)).unwrap();
    SourceAnalysisView::restore(connection,&manifest,&snapshot).unwrap()
}



#[test]
fn source_result_receipt_faults_roll_back_projection_and_status() {
    for action in ["SELECT RAISE(IGNORE)","SELECT RAISE(ABORT,'fault')","UPDATE sentinel_scans SET status='paused'"] {
        let (root,connection,record)=analysis_view_fixture("diff",true);
        connection.execute_batch(&format!("CREATE TRIGGER receipt_fault BEFORE INSERT ON source_analysis_results BEGIN {action}; END;")).unwrap();
        let before:String=connection.query_row("SELECT gate_status FROM sentinel_scan_contexts WHERE scan_id=?1",[&record.scan_id],|r|r.get(0)).unwrap();
        let result=analysis_view_run(&root,&record,|_,engine,_,_,scratch,_|Ok(source_regression_outcome(engine,scratch)));
        assert!(result.is_err(),"{action}");
        assert_eq!(connection.query_row("SELECT count(*) FROM source_analysis_results",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert!(NativeSourcePlan::load(&connection,&record.scan_id,1).unwrap().is_none());
        assert_eq!(connection.query_row("SELECT status FROM sentinel_scans WHERE id=?1",[&record.scan_id],|r|r.get::<_,String>(0)).unwrap(),"scanning");
        assert_eq!(connection.query_row("SELECT gate_status FROM sentinel_scan_contexts WHERE scan_id=?1",[&record.scan_id],|r|r.get::<_,String>(0)).unwrap(),before);
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_result_receipt_is_immutable_and_cascades_with_its_parent() {
    let (root,connection,record)=analysis_view_fixture("full",true);
    analysis_view_run(&root,&record,|_,engine,_,_,scratch,_|Ok(source_regression_outcome(engine,scratch))).unwrap();
    for sql in ["UPDATE source_analysis_results SET receipt_digest=printf('%064d',0)","DELETE FROM source_analysis_results",
        "INSERT OR REPLACE INTO source_analysis_results SELECT * FROM source_analysis_results"] {
        assert!(connection.execute(sql,[]).is_err());
    }
    connection.execute("DELETE FROM sentinel_scans WHERE id=?1",[&record.scan_id]).unwrap();
    assert_eq!(connection.query_row("SELECT count(*) FROM source_analysis_results",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_result_receipt_reuses_exact_revision_but_cannot_accept_a_new_analysis() {
    let (root,connection,record)=analysis_view_fixture("diff",true);
    let run=|title:&str|analysis_view_run(&root,&record,|_,engine,_,_,scratch,_|{
        let mut outcome=source_regression_outcome(engine,scratch);
        let bytes=source_result_sarif(engine,"app.py",title);
        fs::write(outcome.sarif_path.as_ref().unwrap(),&bytes).unwrap();
        outcome.sarif_sha256=crate::artifact_import::canonical::sha256_hex(bytes.as_bytes());Ok(outcome)
    });
    let first=run("same").unwrap();
    let count=connection.query_row("SELECT count(*) FROM import_record_revisions",[],|r|r.get::<_,i64>(0)).unwrap();
    let second=run("same").unwrap();
    assert_eq!(first["analysisResultsDigest"],second["analysisResultsDigest"]);
    assert_eq!(connection.query_row("SELECT count(*) FROM import_record_revisions",[],|r|r.get::<_,i64>(0)).unwrap(),count);
    assert!(matches!(run("different"),Err(error) if error=="analysis_results_already_frozen"));
    assert_eq!(source_claims_for_analysis(&connection,&source_result_view(&connection,&record)).unwrap(),first["sourceClaims"].as_array().unwrap().clone());
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_result_receipt_imported_artifact_hash_must_match_accepted_bytes() {
    let (root,connection,record)=analysis_view_fixture("diff",true);
    connection.execute_batch("CREATE TRIGGER change_imported_hash AFTER INSERT ON import_bundle_files BEGIN UPDATE import_bundle_files SET content_hash=printf('%064d',0) WHERE rowid=NEW.rowid; END;").unwrap();
    let result=analysis_view_run(&root,&record,|_,engine,_,_,scratch,_|Ok(source_regression_outcome(engine,scratch)));
    assert!(matches!(result,Err(error) if error=="analysis_result_import_artifact_mismatch"));
    assert_eq!(connection.query_row("SELECT count(*) FROM source_analysis_results",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert!(NativeSourcePlan::load(&connection,&record.scan_id,1).unwrap().is_none());
    drop(connection);fs::remove_dir_all(root).unwrap();
}


include!("tests_source_broker_historical_negatives.rs");
include!("tests_source_broker_original_fixture.rs");
include!("tests_source_broker_original_reads.rs");
include!("tests_source_broker_original_candidates.rs");
include!("tests_source_result_original_reads.rs");
include!("tests_source_result_original_history.rs");
include!("tests_source_result_original_material_failures.rs");
include!("tests_source_result_original_retry.rs");
