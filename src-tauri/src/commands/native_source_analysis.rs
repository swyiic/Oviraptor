// Source-only snapshot, analyzer, canonical import, and fenced projection.
#[allow(clippy::too_many_arguments)]
fn run_native_source_scan_using(
    db_path: &Path,
    app_data_dir: &Path,
    scan_id: &str,
    attempt_number: i64,
    work_dir: &Path,
    source_path: &str,
    scan_type: &str,
    diff_base: &str,
    mut execute: impl FnMut(&rusqlite::Connection, &str, &RepositorySnapshot, &SourceAnalysisView, &Path, &dyn Fn() -> bool)
        -> Result<analyzer::AnalyzerOutcome, String>,
) -> Result<JsonValue, String> {
    let connection = crate::db::open(db_path)?;
    if !native_source_attempt_active(&connection, scan_id, attempt_number) {
        return Err("native_source_attempt_stopped_or_replaced".into());
    }
    let source_scope=load_workbench_source_scope(&connection,scan_id,attempt_number)?;
    source_scope.verify_worker(source_path,scan_type,diff_base)?;
    // Publication freezes CI policy atomically with this attempt. Never read the
    // mutable global settings at completion, or reconstruct historical permission.
    let gate_policy = if scan_type == "cicd" {
        Some(load_workbench_ci_policy(&connection, scan_id, attempt_number)?)
    } else { None };
    let (cas_dir, scratch_dir) = native_source_paths(app_data_dir, work_dir);
    let requested_base = if source_scope.diff_base.is_empty() {
        None
    } else {
        Some(source_scope.diff_base.as_str())
    };
    let restored = RepositorySnapshot::restore(&connection, scan_id, attempt_number)?;
    let was_restored = restored.is_some();
    let snapshot = match restored {
        Some(snapshot) => {
            if Path::new(source_path).canonicalize().map_err(|e| e.to_string())? != snapshot.root {
                return Err("snapshot_source_path_changed".into());
            }
            if scratch_dir.canonicalize().map_err(|_|"snapshot_work_directory_unavailable")?
                != snapshot.scratch_dir.canonicalize().map_err(|e|e.to_string())? {
                return Err("snapshot_work_directory_changed".into());
            }
            snapshot.verify_unchanged()?;
            snapshot
        }
        None => RepositorySnapshot::capture(Path::new(&source_scope.canonical_root), &scratch_dir, requested_base)
            .map_err(|error| format!("源码快照失败：{error}"))?,
    };
    if snapshot.is_empty() && source_scope.scope_mode == "full" {
        return Err("源码快照是空集，不能按‘没有发现’收口".to_string());
    }
    let manifest = AnalysisManifest::select(&snapshot, scan_id, attempt_number, &json!(source_scope))?;
    let view = if was_restored {
        SourceAnalysisView::restore(&connection, &manifest, &snapshot)?
    } else {
        let view = SourceAnalysisView::materialize(&snapshot, manifest)?;
        let publication = rusqlite::Transaction::new_unchecked(&connection, rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        verify_workbench_source_scope(&publication, scan_id, attempt_number, &source_scope)?;
        snapshot.store(&publication, scan_id, attempt_number)?;
        view.store(&publication)?;
        snapshot.verify_unchanged()?;
        view.verify(&snapshot)?;
        verify_workbench_source_scope(&publication, scan_id, attempt_number, &source_scope)?;
        view.verify_receipt(&publication)?;
        view.verify_source_receipt(&publication)?;
        publication.commit().map_err(|e| e.to_string())?;
        view
    };
    if was_restored && NativeSourcePlan::load(&connection,scan_id,attempt_number)?.is_some() {
        // A completed projection must not acquire replacement acceptance evidence
        // merely because its immutable result receipt went missing.
        crate::native_pipeline::results::AnalysisResults::load(&connection,&view)?;
    }
    let mut gaps: Vec<String> = snapshot.gaps.clone();
    gaps.extend(view.manifest.gaps.clone());
    let mut engines: Vec<String> = Vec::new();
    let mut provenance: Vec<JsonValue> = Vec::new();
    let mut outcomes = Vec::new();
    let cancelled = {
        let db_path = db_path.to_path_buf();
        let scan_id = scan_id.to_string();
        let scope = source_scope.clone();
        let view = view.clone();
        move || db::open(&db_path).map(|connection|
            !native_source_attempt_active(&connection, &scan_id, attempt_number)
                || verify_workbench_source_scope(&connection,&scan_id,attempt_number,&scope).is_err()
                || view.verify_receipt(&connection).is_err()).unwrap_or(true)
    };
    for engine in NATIVE_ANALYZERS {
        if cancelled() { return Err("native_source_attempt_stopped_or_replaced".into()); }
        verify_workbench_source_scope(&connection,scan_id,attempt_number,&source_scope)?;
        if let Some(policy) = &gate_policy {
            if &load_workbench_ci_policy(&connection, scan_id, attempt_number)? != policy {
                return Err("workbench_ci_policy_changed".into());
            }
        }
        snapshot.verify_unchanged()?;
        view.verify_receipt(&connection)?;
        view.verify_source_receipt(&connection)?;
        view.verify(&snapshot)?;
        if view.manifest.files.is_empty() { continue; }
        // Project-wide CodeQL extraction needs a separate approved context contract.
        // Never mount the full tree under a selected-files diff authorization.
        if engine == "codeql" && view.manifest.scope != "full" {
            gaps.push("codeql:diff_project_context_not_authorized".into());
            continue;
        }
        let analyzer_output = native_analyzer_output_dir(&scratch_dir,engine)?;
        let execution = execute(&connection, engine, &snapshot, &view, &analyzer_output, &cancelled);
        if cancelled() { return Err("native_source_attempt_stopped_or_replaced".into()); }
        verify_workbench_source_scope(&connection,scan_id,attempt_number,&source_scope)?;
        if let Some(policy) = &gate_policy {
            if &load_workbench_ci_policy(&connection, scan_id, attempt_number)? != policy {
                return Err("workbench_ci_policy_changed".into());
            }
        }
        snapshot.verify_unchanged()?;
        view.verify_receipt(&connection)?;
        view.verify_source_receipt(&connection)?;
        view.verify(&snapshot)?;
        let outcome = match execution {
            Ok(outcome) => outcome,
            Err(gap) => { gaps.push(gap); continue; }
        };
        snapshot.verify_unchanged()?;
        engines.push(engine.to_string());
        if !outcome.repository_read_only {
            // A host binary is not a sandboxed one: the run counts, but so does the fact
            // that the repository was not mounted read-only and the network stayed up.
            gaps.push(format!("analyzer_unsandboxed_host_run:{engine}"));
        }
        if outcome.produced_results() {
            provenance.push(outcome.evidence_json());
        } else {
            gaps.push(format!("{}:{}", engine, outcome.gap_reason()));
            if outcome.truncated {
                gaps.push(format!("{engine}:output_truncated"));
            }
        }
        outcomes.push(outcome);
    }
    if cancelled() { return Err("native_source_attempt_stopped_or_replaced".into()); }
    verify_workbench_source_scope(&connection,scan_id,attempt_number,&source_scope)?;
    snapshot.verify_unchanged()?;
    view.verify_receipt(&connection)?;
    view.verify_source_receipt(&connection)?;
    view.verify(&snapshot)?;

    // Analyzers write files; only the canonical importer turns them into records.
    let key_path = app_data_dir.join("artifact-import.key");
    // Never recursively import the analyzer scratch: failed/old runs and CodeQL
    // working files live there too. Copy only accepted, hash-verified bytes into a
    // fresh importer root. Re-read and hash the SAME bytes that will be imported.
    let import_root = scratch_dir.join(format!("accepted-results-{}", Uuid::new_v4()));
    std::fs::create_dir(&import_root).map_err(|error| error.to_string())?;
    let mut accepted = 0;
    let mut accepted_artifacts = Vec::new();
    for outcome in outcomes.iter().filter(|outcome| outcome.produced_results()) {
        let path = outcome.sarif_path.as_ref().ok_or("accepted_sarif_path_missing")?;
        let bytes = match analyzer::read_sarif(path) {
            Ok(bytes) if crate::artifact_import::canonical::sha256_hex(&bytes) == outcome.sarif_sha256 => bytes,
            _ => { gaps.push(format!("artifact_import:result_integrity:{}", outcome.engine.as_str())); continue; }
        };
        std::fs::write(import_root.join(format!("{}.sarif", outcome.engine.as_str())), bytes)
            .map_err(|error| error.to_string())?;
        accepted_artifacts.push(crate::native_pipeline::results::AcceptedArtifact {
            engine:outcome.engine.as_str().into(),sha256:outcome.sarif_sha256.clone(),
        });
        accepted += 1;
    }
    let marker = import_root.join(".oviraptor-scan-id");
    std::fs::write(&marker, scan_id).map_err(|error| error.to_string())?;
    let roots = if accepted == 0 { vec![] } else { vec![import_root] };
    let limits = Limits::default();
    let context = ImportContext {
        connection: &connection,
        cas_dir: &cas_dir,
        key_path: &key_path,
        roots: &roots,
        limits: &limits,
    };
    if cancelled() { return Err("native_source_attempt_stopped_or_replaced".into()); }
    snapshot.verify_unchanged()?;
    view.verify(&snapshot)?;
    let summary = import_roots(&context);
    let failed_imports = summary.outcomes.iter()
        .filter(|outcome| outcome.status == crate::artifact_import::BundleStatus::Failed).count();
    if failed_imports > 0 {
        gaps.push(format!("artifact_import:failed_bundles:{failed_imports}"));
    }
    for diagnostic in summary.discovery_diagnostics.iter()
        .chain(summary.outcomes.iter().flat_map(|outcome| outcome.diagnostics.iter())) {
        if diagnostic.severity != crate::artifact_import::diagnostics::Severity::Info {
            gaps.push(format!("artifact_import:{}", diagnostic.code));
        }
    }
    if outcomes.iter().any(analyzer::AnalyzerOutcome::produced_results) && summary.outcomes.is_empty() {
        gaps.push("artifact_import:no_bundles_discovered".into());
    }
    // Fence plan/CI projection in the same write transaction. A stop or a new
    // attempt cannot race between the check and these visible result updates.
    let projection = rusqlite::Transaction::new_unchecked(&connection, rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    if !native_source_attempt_active(&projection, scan_id, attempt_number) {
        return Err("native_source_attempt_stopped_or_replaced".into());
    }
    verify_workbench_source_scope(&projection,scan_id,attempt_number,&source_scope)?;
    view.verify_receipt(&projection)?;
    view.verify_source_receipt(&projection)?;
    snapshot.verify_unchanged()?;
    view.verify(&snapshot)?;
    let results=crate::native_pipeline::results::AnalysisResults::capture(&projection,&view,accepted_artifacts,&summary,&outcomes,&gaps)?;
    results.store(&projection,&view)?;
    gaps.extend(results.gaps.iter().cloned());
    // Analysis is real, but this branch has not yet delivered an independent
    // source Reviewer decision. Preserve that distinction in both UI and CI.
    gaps.push("source_review_not_completed".into());
    let plan = NativeSourcePlan::for_snapshot(
        scan_id, attempt_number, scan_type, &snapshot,
        &engines.iter().map(String::as_str).collect::<Vec<_>>(), gaps,
    );
    plan.store(&projection)?;

    let source_claims = source_claims_for_analysis(&projection, &view)?;
    let mut report = json!({
        "backend": "native",
        "requestedSourceScope": source_scope,
        "analysisView": view.as_json(),
        "analysisResultsDigest": results.digest(),
        "planHash": plan.hash(),
        "treeHash": snapshot.tree_hash,
        "commitSha": snapshot.commit_sha,
        "baseSha": snapshot.base_sha,
        "diffBase": snapshot.diff_base.as_str(),
        "fileCount": view.manifest.files.len(),
        "sourceFileCount": snapshot.file_count(),
        "languages": snapshot.languages(),
        "analyzers": engines,
        "analyzerProvenance": provenance,
        "importedBundles": summary.outcomes.len().saturating_sub(failed_imports),
        "failedImportBundles": failed_imports,
        "gaps": plan.gaps,
        "sourceClaims": source_claims,
    });
    if let Some(gate_policy) = gate_policy {
        if load_workbench_ci_policy(&projection, scan_id, attempt_number)? != gate_policy {
            return Err("workbench_ci_policy_changed".into());
        }
        let freeze = CiFreeze::of_analysis(&snapshot, &outcomes, &view.manifest);
        let gate = ci::evaluate_unreviewed(
            &freeze,
            &plan.gaps,
            None,
            gate_policy,
        )?;
        projection
            .execute(
                "UPDATE sentinel_scan_contexts SET gate_status=?2,gate_reason=?3 WHERE scan_id=?1",
                rusqlite::params![scan_id, gate.status.as_str(), gate.reasons.join("; ")],
            )
            .map_err(|error| format!("无法写入发布门禁：{error}"))?;
        if let Some(map) = report.as_object_mut() {
            map.insert("gate".to_string(), gate.as_json());
        }
    }
    if !native_source_attempt_active(&projection,scan_id,attempt_number) {
        return Err("native_source_attempt_stopped_or_replaced".into());
    }
    verify_workbench_source_scope(&projection,scan_id,attempt_number,&source_scope)?;
    view.verify_source_receipt(&projection)?;
    snapshot.verify_unchanged()?;
    view.verify(&snapshot)?;
    if crate::native_pipeline::results::AnalysisResults::load(&projection,&view)?!=results {
        return Err("analysis_result_receipt_changed_during_projection".into());
    }
    projection.commit().map_err(|error| error.to_string())?;
    Ok(report)
}

fn source_claims_for_analysis(connection: &rusqlite::Connection, view: &SourceAnalysisView) -> Result<Vec<JsonValue>, String> {
    let results=crate::native_pipeline::results::AnalysisResults::load(connection,view)?;
    let mut claims = Vec::new();
    for candidate in results.candidates(connection)? {
        let accepted=candidate.receipt;
        let envelope=candidate.envelope;
        let payload = envelope.get("payload").cloned().unwrap_or(JsonValue::Null);
        let path = ["path", "endpoint", "repoPath", "url"]
            .iter()
            .find_map(|key| payload.get(*key).and_then(JsonValue::as_str))
            .unwrap_or("")
            .to_string();
        if path.is_empty() { continue; }
        claims.push(json!({
            "key": accepted.record.logical_key,
            "revisionHash": accepted.record.revision_hash,
            "acceptedSources":candidate.sources,
            "analysisResultsDigest": results.digest(),
            "analysisManifestDigest": view.manifest.digest(),
            "scanId": view.manifest.scan_id,
            "attemptNumber": view.manifest.attempt_number,
            "method": payload.get("method").and_then(JsonValue::as_str).unwrap_or(""),
            "path": path,
            "line": payload.get("line").and_then(JsonValue::as_u64)
                .or_else(|| payload.pointer("/region/startLine").and_then(JsonValue::as_u64))
                .unwrap_or(0),
            "claim": payload.get("title").and_then(JsonValue::as_str)
                .or_else(|| payload.get("rule").and_then(JsonValue::as_str))
                .or_else(|| payload.get("rule_id").and_then(JsonValue::as_str))
                .unwrap_or(""),
        }));
    }
    Ok(claims)
}
