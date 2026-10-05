// §12 Stage 4 — how a source-carrying scan (`code`, `greybox` with a repository, and
// `cicd`) runs on the Native pipeline. It freezes a snapshot, runs the configured
// analyzers through the bounded runner, and lets only the canonical importer turn their
// SARIF into records. A missing capability closes the branch with a gap; it never
// starts another engine.

use crate::artifact_import::{import_roots, ImportContext, Limits};
use crate::native_pipeline::analysis_view::{AnalysisManifest, SourceAnalysisView};
use crate::native_pipeline::analyzer::{self, AnalyzerEngine, AnalyzerSpec};
use crate::native_pipeline::ci::{self, CiFreeze, GatePolicy};
use crate::native_pipeline::process::ProcessLimits;
use crate::native_pipeline::snapshot::RepositorySnapshot;
use crate::native_pipeline::NativeSourcePlan;
use serde_json::json;

/// Analyzers this build knows how to drive. Anything else is a coverage gap.
const NATIVE_ANALYZERS: [&str; 2] = ["semgrep", "codeql"];

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("exe") | Some("cmd") | Some("bat")
        )
    }
}

fn executable_on_path(name: &str) -> Option<PathBuf> {
    let candidates = std::env::var_os("PATH")?;
    std::env::split_paths(&candidates)
        .map(|directory| directory.join(name))
        .find(|candidate| is_executable(candidate))
}

fn configured_native_analyzer_runtime(
    settings: &JsonValue,
    engine: &str,
    locate: impl FnOnce(&str) -> Option<PathBuf>,
) -> Result<(PathBuf, Option<String>), String> {
    let image = settings
        .get("nativeAnalyzers")
        .and_then(|analyzers| analyzers.get(engine))
        .and_then(|config| config.get("image"))
        .and_then(JsonValue::as_str)
        .filter(|image| analyzer::image_digest(image).is_some())
        .ok_or_else(|| format!("analyzer_missing:{engine}:pinned_sandbox_image_required"))?;
    let program = locate("docker")
        .ok_or_else(|| format!("analyzer_missing:{engine}:container_runtime_unavailable"))?;
    Ok((program, Some(image.to_string())))
}

/// Where the sealed object store and the analyzer scratch live for one attempt.
fn native_source_paths(app_data_dir: &Path, work_dir: &Path) -> (PathBuf, PathBuf) {
    (
        app_data_dir.join("artifact-import-cas"),
        work_dir.join("source-analysis"),
    )
}

fn native_analyzer_output_dir(scratch_dir: &Path, engine: &str) -> Result<PathBuf, String> {
    if !NATIVE_ANALYZERS.contains(&engine) {
        return Err("analyzer_output_engine_invalid".into());
    }
    let mut path = scratch_dir.canonicalize().map_err(|e| e.to_string())?;
    // Only this leaf is writable in the container. The parent contains sealed
    // provenance and selected source copies and must never be mounted as /out.
    for part in ["analyzer-output", engine] {
        path.push(part);
        match fs::create_dir(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.to_string()),
        }
        let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || path.canonicalize().map_err(|e| e.to_string())? != path
        {
            return Err("analyzer_output_directory_unsafe".into());
        }
    }
    Ok(path)
}

fn native_source_attempt_active(
    connection: &rusqlite::Connection,
    scan_id: &str,
    attempt_number: i64,
) -> bool {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1 AND attempt_count=?2 AND status='scanning')
         AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)",
        params![scan_id, attempt_number], |row| row.get::<_, bool>(0),
    ).unwrap_or(false)
}

/// One frozen source scan: snapshot, analyzers, canonical import, and for CI the gate.
#[allow(clippy::too_many_arguments)]
fn run_native_source_scan(
    db_path: &Path,
    app_data_dir: &Path,
    scan_id: &str,
    attempt_number: i64,
    work_dir: &Path,
    source_path: &str,
    scan_type: &str,
    diff_base: &str,
) -> Result<JsonValue, String> {
    // This is the production entry. Historical task JSON is not a model grant;
    // reconstruct only a candidate runtime and verify its publication receipt.
    // Analyzer-only unit seams below deliberately do not contact a model.
    let connection = crate::db::open(db_path)?;
    verify_source_runtime_contract(&connection, scan_id, attempt_number, work_dir)?;
    drop(connection);
    let mut report = run_native_source_scan_using(
        db_path,
        app_data_dir,
        scan_id,
        attempt_number,
        work_dir,
        source_path,
        scan_type,
        diff_base,
        |connection, engine, snapshot, view, scratch_dir, cancelled| {
            let settings = sentinel_settings(connection);
            let (program, image) =
                configured_native_analyzer_runtime(&settings, engine, executable_on_path)?;
            let rule_pack = enabled_rule_pack(connection, engine)
                .ok_or_else(|| format!("rule_pack_missing:{engine}"))?;
            let spec = AnalyzerSpec {
                engine: AnalyzerEngine::parse(engine)
                    .ok_or_else(|| format!("unsupported_analyzer:{engine}"))?,
                program,
                image,
                rule_pack: PathBuf::from(rule_pack),
                languages: snapshot.languages(),
                repository: view.repository().to_path_buf(),
                scratch_dir: scratch_dir.to_path_buf(),
                scan_id: scan_id.to_string(),
                attempt_number,
                limits: ProcessLimits::default(),
            };
            analyzer::run_logged(connection, &spec, cancelled)
        },
    )?;
    match run_native_source_assessments(db_path, scan_id, attempt_number, work_dir) {
        Ok(assessments) => merge_native_source_assessments(&mut report, assessments),
        Err(error) => {
            let reason = crate::agent_runtime::secrets::redact_text_with(&error, None);
            report["sourceMultiAgent"] =
                json!({"status":"incomplete","reason":reason,"independentReviewCompleted":false});
            if let Some(gaps) = report["gaps"].as_array_mut() {
                gaps.push(json!("source_multi_agent_execution_incomplete"));
            }
        }
    }
    Ok(report)
}

// Presentation only, after the coordinator's audited projection transaction.
// This never publishes decisions or turns a report into executable authority.
fn merge_native_source_assessments(report: &mut JsonValue, assessments: JsonValue) {
    if !assessments["sourceCoverageDecision"].is_null() {
        report["analyzerGaps"] = report["gaps"].clone();
    }
    if !assessments["sourceDecisionProjection"].is_null() {
        let projection = &assessments["sourceDecisionProjection"];
        report["sourceDecisionProjection"] = projection.clone();
        report["sourceFindings"] = projection["findings"].clone();
        report["analyzerGaps"] = report["gaps"].clone();
        if let Some(gaps) = report["gaps"].as_array_mut() {
            gaps.retain(|gap| gap != "source_review_not_completed");
            for gap in [
                "source_coverage_review",
                "source_candidate_evidence_incomplete",
            ] {
                if gap == "source_candidate_evidence_incomplete"
                    && projection["counts"]["insufficient"].as_u64() == Some(0)
                {
                    continue;
                }
                if !gaps.contains(&json!(gap)) {
                    gaps.push(json!(gap));
                }
            }
        }
    }
    if !assessments["gate"].is_null() {
        // Keep the analyzer-stage history, including its original review gap.
        report["analyzerGate"] = report["gate"].clone();
        report["gate"] = assessments["gate"].clone();
        report["gaps"] = assessments["gate"]["gaps"].clone();
    }
    // A delivered coverage decision preserves all deterministic gaps while
    // replacing only review-stage placeholders. Keep the analyzer history;
    // presentation never confers execution or publication authority.
    if !assessments["sourceCoverageDecision"].is_null() {
        report["sourceCoverageDecision"] = assessments["sourceCoverageDecision"].clone();
        report["independentReviewCompleted"] = assessments["independentReviewCompleted"].clone();
        if assessments["gate"].is_null() {
            report["gaps"] =
                assessments["sourceCoverageDecision"]["decision"]["outstandingGaps"].clone();
        }
    } else if let Some(extra) =
        assessments["coverageReviewPreparation"]["outstandingGaps"].as_array()
    {
        if let Some(gaps) = report["gaps"].as_array_mut() {
            for gap in extra {
                if !gaps.contains(gap) {
                    gaps.push(gap.clone());
                }
            }
        }
    }
    report["sourceMultiAgent"] = assessments;
}

#[cfg(test)]
fn finish_native_source_attempt(
    db_path: &Path,
    scan_id: &str,
    attempt_number: i64,
    status: &str,
    checkpoint: &str,
) -> Result<bool, String> {
    if !matches!(status, "completed" | "completed_with_gaps" | "failed") {
        return Err("invalid_source_terminal_status".into());
    }
    let mut connection = db::open(db_path)?;
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let changed = transaction.execute(
        "UPDATE sentinel_scans SET status=?1,current_checkpoint=?2,updated_at=datetime('now','localtime')
         WHERE id=?3 AND attempt_count=?4 AND status='scanning'
         AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?3)",
        params![status, checkpoint, scan_id, attempt_number],
    ).map_err(|error| error.to_string())?;
    if changed == 1 {
        sync_sentinel_attempt(&transaction, scan_id);
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(changed == 1)
}

#[allow(clippy::too_many_arguments)]
fn launch_native_source_pipeline(
    db_path: PathBuf,
    app_data_dir: PathBuf,
    scan_id: String,
    attempt_number: i64,
    work_dir: PathBuf,
    source_path: String,
    scan_type: String,
    diff_base: String,
) -> Result<(), String> {
    std::thread::Builder::new().name("native-source-pipeline".into()).spawn(move || {
        let _branch_guard = match claim_workbench_pipeline_branch(&db_path, &scan_id, attempt_number, "source") {
            Ok(guard) => guard,
            Err(error) => {
                append_runner_log(&work_dir.join("oviraptor-runner.log"), &format!("源码分支未取得执行权，不启动；未领取的工作台分支按准入结果登记，其他状态不改写：{error}"));
                return;
            }
        };
        let log_path = work_dir.join("oviraptor-runner.log");
        append_runner_log(
            &log_path,
            &format!("原生源码流水线开始：{source_path}（第 {attempt_number} 次尝试）"),
        );
        let outcome = run_native_source_scan(
            &db_path,
            &app_data_dir,
            &scan_id,
            attempt_number,
            &work_dir,
            &source_path,
            &scan_type,
            &diff_base,
        );
        match outcome {
            Ok(report) => {
                append_runner_log(&log_path, &report.to_string());
                if let Err(error) = finish_native_source_branch(&db_path, &scan_id, attempt_number, &report) {
                    append_runner_log(&log_path, &format!("源码终态未写入：{error}"));
                }
            }
            Err(error) => {
                append_runner_log(&log_path, &format!("原生源码流水线未完成：{error}"));
                if let Err(write_error) = finish_native_source_branch(&db_path, &scan_id, attempt_number, &json!({"error":error})) {
                    append_runner_log(&log_path, &format!("源码终态未写入：{write_error}"));
                }
            }
        }
    }).map(|_| ()).map_err(|error| format!("native_source_thread_spawn:{error}"))
}

include!("native_source_branch_completion.rs");
