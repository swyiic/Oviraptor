fn analysis_view_git(repo: &Path, args: &[&str]) -> String {
    let output=std::process::Command::new("git").current_dir(repo)
        .args(["-c","user.name=Analysis Fixture","-c","user.email=analysis@invalid","-c","commit.gpgsign=false"])
        .args(args).output().unwrap();
    assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap().trim().into()
}

fn analysis_view_fixture(mode: &str, valid_base: bool) -> (PathBuf,rusqlite::Connection,WorkbenchStartRecord) {
    analysis_view_fixture_configure(mode,valid_base,|_,_|{})
}

fn analysis_view_fixture_configure(mode: &str, valid_base: bool, configure: impl FnOnce(&rusqlite::Connection,&mut WorkbenchStartRecord)) -> (PathBuf,rusqlite::Connection,WorkbenchStartRecord) {
    let (root,connection,mut record,plan,draft)=source_ci_publication_fixture();
    let repo=Path::new(&record.source_path);
    fs::write(repo.join("untouched.py"),"print('not selected')\n").unwrap();
    analysis_view_git(repo,&["init","-q"]);
    analysis_view_git(repo,&["add","."]);
    analysis_view_git(repo,&["commit","-qm","base"]);
    let base=analysis_view_git(repo,&["rev-parse","HEAD"]);
    fs::write(repo.join("app.py"),"print('changed')\n").unwrap();
    record.scope_mode=mode.into();
    record.diff_base=if valid_base {base} else {"missing-ref".into()};
    configure(&connection,&mut record);
    publish_workbench_fixture(&root,&connection,&record,&plan,&draft,false).unwrap();
    (root,connection,record)
}

#[test]
fn source_analysis_view_diff_passes_only_selected_bytes_to_the_analyzer() {
    let (root,connection,record)=analysis_view_fixture("diff",true);
    let mut calls=0;
    let report=run_native_source_scan_using(
        &root.join("oviraptor.sqlite3"),&root,&record.scan_id,1,&root.join("attempt-0001"),
        &record.source_path,"cicd",&record.diff_base,
        |_,engine,snapshot,view,scratch,_| {
            calls+=1;
            assert!(!view.repository().join("untouched.py").exists(),"explicit diff must not mount unselected source");
            assert_eq!(fs::read_to_string(view.repository().join("app.py")).unwrap(),"print('changed')\n");
            assert!(snapshot.frozen_root.join("untouched.py").exists(),"complete provenance must remain intact");
            Ok(source_regression_outcome(engine,scratch))
        }).unwrap();
    assert_eq!(calls,1,"CodeQL cannot silently broaden a selected-files diff");
    assert_eq!(report["gate"]["freeze"]["scope"],"diff");
    assert_eq!(report["gate"]["freeze"]["fileCount"],1);
    drop(connection); fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_analysis_view_explicit_bad_diff_never_runs_full_analyzers() {
    let (root,connection,record)=analysis_view_fixture("diff",false);
    let report=run_native_source_scan_using(
        &root.join("oviraptor.sqlite3"),&root,&record.scan_id,1,&root.join("attempt-0001"),
        &record.source_path,"cicd",&record.diff_base,
        |_,_,_,_,_,_| panic!("unusable explicit diff must not execute an analyzer on the full tree"),
    ).unwrap();
    assert_eq!(report["gate"]["status"],"inconclusive");
    assert_eq!(report["fileCount"],0);
    drop(connection); fs::remove_dir_all(root).unwrap();
}

fn analysis_view_run(root: &Path, record: &WorkbenchStartRecord,
    execute: impl FnMut(&rusqlite::Connection,&str,&RepositorySnapshot,&SourceAnalysisView,&Path,&dyn Fn()->bool)
        -> Result<analyzer::AnalyzerOutcome,String>) -> Result<JsonValue,String> {
    run_native_source_scan_using(&root.join("oviraptor.sqlite3"),root,&record.scan_id,1,&root.join("attempt-0001"),
        &record.source_path,"cicd",&record.diff_base,execute)
}

#[test]
fn source_analysis_view_full_and_auto_bind_actual_input_and_ci_digest() {
    for (mode,valid,actual,count) in [("full",true,"full",2),("auto",true,"diff",1),("auto",false,"full",2)] {
        let (root,connection,record)=analysis_view_fixture(mode,valid);
        let report=analysis_view_run(&root,&record,|_,engine,snapshot,view,scratch,_| {
            assert_eq!(view.manifest.scope,actual);
            assert_eq!(view.manifest.files.len(),count);
            assert_eq!(view.repository().join("untouched.py").exists(),actual=="full");
            assert_eq!(snapshot.files.len(),2,"never filter provenance in place");
            Ok(source_regression_outcome(engine,scratch))
        }).unwrap();
        assert_eq!(report["fileCount"],count);
        assert_eq!(report["sourceFileCount"],2);
        assert_eq!(report["gate"]["freeze"]["scope"],actual);
        assert_eq!(report["gate"]["freeze"]["fileCount"],count);
        assert_eq!(report["gate"]["freeze"]["analysisManifestDigest"],report["analysisView"]["digest"]);
        assert_eq!(report["analysisView"]["manifest"]["fallbackReason"].is_string(),mode=="auto"&&!valid);
        assert_eq!(report["analysisView"]["manifest"]["request"]["analysisPolicyVersion"],1);
        if mode=="full" { assert_eq!(report["baseSha"],""); }
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_analysis_view_empty_or_deleted_diff_is_not_clean_or_widened() {
    for deleted in [false,true] {
        let (root,connection,record)=analysis_view_fixture("diff",true);
        let repo=Path::new(&record.source_path);
        if deleted { fs::remove_file(repo.join("app.py")).unwrap(); }
        else { fs::write(repo.join("app.py"),"print('ci fixture')\n").unwrap(); }
        let report=analysis_view_run(&root,&record,|_,_,_,_,_,_|panic!("empty selected input must not run analyzers")).unwrap();
        assert_eq!(report["fileCount"],0);
        assert_eq!(report["gate"]["status"],"inconclusive");
        assert_eq!(report["gate"]["freeze"]["scope"],"diff");
        assert_eq!(report["analysisView"]["manifest"]["changedPathsWithoutContent"],if deleted {json!(["app.py"])} else {json!([])});
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_analysis_view_faulted_publication_rolls_back_snapshot_without_execution() {
    for fault in ["RAISE(ABORT,'injected')","RAISE(IGNORE)"] {
        let (root,connection,record)=analysis_view_fixture("full",true);
        connection.execute_batch(&format!("CREATE TRIGGER analysis_fault BEFORE INSERT ON source_analysis_views BEGIN SELECT {fault}; END;")).unwrap();
        let result=analysis_view_run(&root,&record,|_,_,_,_,_,_|panic!("unpublished view must never execute"));
        assert!(result.is_err());
        for table in ["source_snapshots","source_analysis_views","analyzer_runs"] {
            assert_eq!(connection.query_row(&format!("SELECT count(*) FROM {table}"),[],|r|r.get::<_,i64>(0)).unwrap(),0);
        }
        assert!(NativeSourcePlan::load(&connection,&record.scan_id,1).unwrap().is_none());
        assert!(!root.join("artifact-import-cas").exists());
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_analysis_view_mutation_is_rejected_even_when_analyzer_reports_a_gap() {
    for mutation in ["content","extra","original"] {
        let (root,connection,record)=analysis_view_fixture("diff",true);
        let mut calls=0;
        let result=analysis_view_run(&root,&record,|_,_,snapshot,view,_,_| {
            calls+=1;
            match mutation {
                "extra"=>fs::write(view.repository().join("extra.py"),"not selected").unwrap(),
                "original"=>fs::write(snapshot.root.join("untouched.py"),"changed original").unwrap(),
                _=>{
                    let file=view.repository().join("app.py");
                    fs::remove_file(&file).unwrap();
                    fs::write(file,"tampered").unwrap();
                }
            }
            Err("fixture_analyzer_failure".into())
        });
        assert!(result.is_err(),"{mutation}: a failing analyzer must not bypass integrity checks");
        assert_eq!(calls,1);
        assert!(NativeSourcePlan::load(&connection,&record.scan_id,1).unwrap().is_none());
        assert!(!root.join("artifact-import-cas").exists());
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_analysis_view_receipt_is_immutable_and_parent_deletion_cascades() {
    let (root,connection,record)=analysis_view_fixture("full",true);
    analysis_view_run(&root,&record,|_,engine,_,_,scratch,_|Ok(source_regression_outcome(engine,scratch))).unwrap();
    for sql in ["UPDATE source_analysis_views SET manifest_digest=printf('%064d',0)","DELETE FROM source_analysis_views",
        "INSERT OR REPLACE INTO source_analysis_views SELECT * FROM source_analysis_views"] {
        assert!(connection.execute(sql,[]).is_err());
    }
    connection.execute("DELETE FROM sentinel_scans WHERE id='wb-start'",[]).unwrap();
    assert_eq!(connection.query_row("SELECT count(*) FROM source_analysis_views",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_analysis_view_restart_never_rebuilds_a_missing_or_mismatched_receipt() {
    for change in ["receipt","snapshot","directory","digest"] {
        let (root,connection,record)=analysis_view_fixture("diff",true);
        let first=analysis_view_run(&root,&record,|_,engine,_,_,scratch,_|Ok(source_regression_outcome(engine,scratch))).unwrap();
        let path:String=connection.query_row("SELECT view_root FROM source_analysis_views",[],|r|r.get(0)).unwrap();
        match change {
            "receipt"=>connection.execute_batch("DROP TRIGGER analysis_view_no_delete; DELETE FROM source_analysis_views;").unwrap(),
            "snapshot"=>{connection.execute("UPDATE source_snapshots SET changed_files_json='[\"app.py\",\"untouched.py\"]'",[]).unwrap();},
            "directory"=>fs::remove_dir_all(&path).unwrap(),
            _=>connection.execute_batch("DROP TRIGGER analysis_view_no_update; UPDATE source_analysis_views SET manifest_digest=printf('%064d',0);").unwrap(),
        }
        let result=analysis_view_run(&root,&record,|_,_,_,_,_,_|panic!("invalid restored view must not run"));
        assert!(result.is_err(),"{change}: {first}");
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_analysis_view_restart_keeps_frozen_resolved_base_and_directory() {
    let (root,connection,record)=analysis_view_fixture("diff",true);
    let first=analysis_view_run(&root,&record,|_,engine,_,_,scratch,_|Ok(source_regression_outcome(engine,scratch))).unwrap();
    let old_root:String=connection.query_row("SELECT view_root FROM source_analysis_views",[],|r|r.get(0)).unwrap();
    analysis_view_git(Path::new(&record.source_path),&["commit","--allow-empty","-qm","metadata-only change"]);
    let second=analysis_view_run(&root,&record,|_,engine,_,view,scratch,_| {
        assert_eq!(view.repository(),Path::new(&old_root));
        Ok(source_regression_outcome(engine,scratch))
    }).unwrap();
    assert_eq!(first["analysisView"],second["analysisView"]);
    assert_eq!(first["commitSha"],second["commitSha"]);
    assert_eq!(connection.query_row("SELECT count(*) FROM source_analysis_views",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_analysis_view_withdrawn_receipt_stops_in_flight_before_import() {
    let (root,connection,record)=analysis_view_fixture("full",true);
    let mut calls=0;
    let result=analysis_view_run(&root,&record,|connection,engine,_,_,scratch,cancelled| {
        calls+=1;assert!(!cancelled());
        connection.execute_batch("DROP TRIGGER analysis_view_no_delete; DELETE FROM source_analysis_views;").unwrap();
        assert!(cancelled());
        Ok(source_regression_outcome(engine,scratch))
    });
    assert!(result.is_err());assert_eq!(calls,1);
    assert!(!root.join("artifact-import-cas").exists());
    assert!(NativeSourcePlan::load(&connection,&record.scan_id,1).unwrap().is_none());
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_analysis_view_old_scope_semantics_cannot_gain_auto_fallback_permission() {
    let (root,connection,record)=analysis_view_fixture("auto",false);
    connection.execute_batch("DROP TRIGGER source_scope_no_update; UPDATE source_scope_contracts SET analysis_policy_version=0;").unwrap();
    assert!(analysis_view_run(&root,&record,|_,_,_,_,_,_|panic!("old receipt must not gain new semantics")).is_err());
    assert_eq!(connection.query_row("SELECT count(*) FROM source_snapshots",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_analysis_view_output_mount_cannot_expose_provenance_or_other_engines() {
    let (root,connection,record)=analysis_view_fixture("full",true);
    let mut outputs=Vec::new();
    analysis_view_run(&root,&record,|_,engine,snapshot,view,scratch,_| {
        assert!(!snapshot.frozen_root.starts_with(scratch),"/out must not expose the full provenance snapshot");
        assert!(!view.repository().starts_with(scratch),"/out must not provide a writable alias of /src");
        for prior in &outputs {assert!(!scratch.starts_with(prior),"engines must not share a writable output mount");}
        outputs.push(scratch.to_path_buf());
        Ok(source_regression_outcome(engine,scratch))
    }).unwrap();
    assert_eq!(outputs.len(),2);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn source_analysis_view_links_and_special_files_are_rejected_without_import() {
    for kind in ["symlink","hardlink","fifo","extra-directory"] {
        let (root,connection,record)=analysis_view_fixture("diff",true);
        let result=analysis_view_run(&root,&record,|_,engine,snapshot,view,scratch,_| {
            let file=view.repository().join("app.py");
            if kind=="extra-directory" {fs::create_dir(view.repository().join(".git")).unwrap();}
            else {
                fs::remove_file(&file).unwrap();
                match kind {
                    "symlink"=>std::os::unix::fs::symlink(snapshot.frozen_root.join("app.py"),&file).unwrap(),
                    "hardlink"=>fs::hard_link(snapshot.frozen_root.join("app.py"),&file).unwrap(),
                    _=>{let path=std::ffi::CString::new(file.to_str().unwrap()).unwrap();assert_eq!(unsafe{libc::mkfifo(path.as_ptr(),0o600)},0);},
                }
            }
            Ok(source_regression_outcome(engine,scratch))
        });
        assert!(result.is_err(),"{kind}");
        assert!(!root.join("artifact-import-cas").exists());
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn source_analysis_view_output_directory_alias_is_not_followed() {
    let (root,connection,record)=analysis_view_fixture("diff",true);
    let work=root.join("attempt-0001/source-analysis");
    fs::create_dir_all(&work).unwrap();
    let outside=root.join("do-not-write");fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside,work.join("analyzer-output")).unwrap();
    let result=analysis_view_run(&root,&record,|_,_,_,_,_,_|panic!("unsafe output mount must not execute"));
    assert!(result.unwrap_err().contains("analyzer_output_directory_unsafe"));
    assert_eq!(fs::read_dir(&outside).unwrap().count(),0);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_analysis_view_unknown_changes_require_explicit_auto_fallback() {
    for mode in ["auto","diff"] {
        let (root,connection,record)=analysis_view_fixture(mode,true);
        analysis_view_git(Path::new(&record.source_path),&["update-index","--assume-unchanged","app.py"]);
        let mut calls=0;
        let report=analysis_view_run(&root,&record,|_,engine,_,view,scratch,_| {
            calls+=1;assert_eq!(mode,"auto");assert_eq!(view.manifest.scope,"full");
            assert!(view.repository().join("untouched.py").exists());
            Ok(source_regression_outcome(engine,scratch))
        }).unwrap();
        assert_eq!(calls,if mode=="auto" {2} else {0});
        assert_eq!(report["analysisView"]["manifest"]["fallbackReason"],if mode=="auto" {json!("diff_manifest_unavailable")} else {JsonValue::Null});
        assert_ne!(report["gate"]["status"],"passed");
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_analysis_view_selected_paths_preserve_names_and_omissions() {
    let (root,connection,record)=analysis_view_fixture("diff",true);
    let repo=Path::new(&record.source_path);
    for name in ["nested/space name.py","nested/中文.py","nested/line\nbreak.py"] {
        let path=repo.join(name);fs::create_dir_all(path.parent().unwrap()).unwrap();fs::write(path,"print('selected')").unwrap();
    }
    let oversized=fs::File::create(repo.join("oversized.py")).unwrap();oversized.set_len(5*1024*1024+1).unwrap();drop(oversized);
    let report=analysis_view_run(&root,&record,|_,engine,_,view,scratch,_| {
        assert_eq!(view.manifest.files.len(),4);
        for name in ["nested/space name.py","nested/中文.py","nested/line\nbreak.py"] {
            assert_eq!(fs::read_to_string(view.repository().join(name)).unwrap(),"print('selected')");
        }
        assert!(!view.repository().join("oversized.py").exists());
        Ok(source_regression_outcome(engine,scratch))
    }).unwrap();
    assert_eq!(report["analysisView"]["manifest"]["changedPathsWithoutContent"],json!(["oversized.py"]));
    assert_ne!(report["gate"]["status"],"passed");
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_analysis_view_migration_retains_old_contract_as_non_executable() {
    let (root,connection,record)=analysis_view_fixture("auto",false);
    connection.execute_batch("ALTER TABLE source_scope_contracts DROP COLUMN analysis_policy_version;").unwrap();
    drop(connection);
    let db_path=db::initialize(&root).unwrap();
    assert_eq!(db_path,root.join("oviraptor.sqlite3"));
    let connection=db::open(&db_path).unwrap();
    assert_eq!(connection.query_row("SELECT analysis_policy_version FROM source_scope_contracts",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert!(analysis_view_run(&root,&record,|_,_,_,_,_,_|panic!("migration must not synthesize permission")).is_err());
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_analysis_view_ignored_source_publication_is_atomic() {
    let (root,connection,record)=analysis_view_fixture("full",true);
    connection.execute_batch("CREATE TRIGGER ignore_source BEFORE INSERT ON source_snapshots BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    assert!(analysis_view_run(&root,&record,|_,_,_,_,_,_|panic!("missing provenance cannot execute")).is_err());
    assert_eq!(connection.query_row("SELECT count(*) FROM source_analysis_views",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_analysis_view_source_receipt_changes_stop_before_import() {
    for sql in ["DELETE FROM source_snapshots","UPDATE source_snapshots SET tree_hash='changed'",
        "UPDATE source_snapshots SET changed_files_json='[\"untouched.py\"]'"] {
        let (root,connection,record)=analysis_view_fixture("diff",true);
        let result=analysis_view_run(&root,&record,|connection,engine,_,_,scratch,_| {
            connection.execute(sql,[]).unwrap();Ok(source_regression_outcome(engine,scratch))
        });
        assert!(result.is_err(),"{sql}");
        assert!(!root.join("artifact-import-cas").exists());
        assert!(NativeSourcePlan::load(&connection,&record.scan_id,1).unwrap().is_none());
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_analysis_view_restart_cannot_move_output_mount_to_another_work_directory() {
    let (root,connection,record)=analysis_view_fixture("diff",true);
    analysis_view_run(&root,&record,|_,engine,_,_,scratch,_|Ok(source_regression_outcome(engine,scratch))).unwrap();
    let other=root.join("other-work");fs::create_dir_all(other.join("source-analysis")).unwrap();
    let result=run_native_source_scan_using(&root.join("oviraptor.sqlite3"),&root,&record.scan_id,1,&other,
        &record.source_path,"cicd",&record.diff_base,|_,_,_,_,_,_|panic!("restore cannot relocate its output mount"));
    assert!(result.unwrap_err().contains("snapshot_work_directory_changed"));
    drop(connection);fs::remove_dir_all(root).unwrap();
}
