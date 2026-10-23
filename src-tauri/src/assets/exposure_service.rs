//! Native Assets operator adapter. Reuses the original domain owner bodies.
//! Database/export paths are trusted bootstrap capabilities, never RPC fields.
use crate::commands::exposure;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

fn input<T: DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|_| "asset_service_arguments_invalid".into())
}
fn output(value: impl Serialize) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|_| "asset_service_result_invalid".into())
}
fn same_project(expected: i64, provided: i64) -> Result<(), String> {
    if expected <= 0 || expected > 9_007_199_254_740_991 || expected != provided {
        return Err("asset_service_project_scope_mismatch".into());
    }
    Ok(())
}
macro_rules! args {
    ($name:ident {$($field:ident : $kind:ty),* $(,)?}) => {
        #[derive(Deserialize)]
        #[serde(rename_all="camelCase",deny_unknown_fields)]
        struct $name { $($field:$kind),* }
    };
}

args!(PageExposureFindingsArgs {
    input: exposure::contract::ExposureFindingPageInput
});
args!(PageExposureRunsArgs {
    project_id: i64,
    page_size: i64,
    cursor: String
});
args!(ListExposureSourceResultsForProjectArgs {
    run_id: i64,
    expected_project_id: i64
});
args!(ReadExposureFindingArgs {
    finding_id: i64,
    expected_project_id: i64
});
args!(ExposureScopeSummaryArgs { project_id: i64 });
args!(ExposureKnownLimitsArgs {});
args!(ExposureSummaryArgs { project_id: i64 });
args!(ListExposureFindingsArgs {
    project_id: i64,
    status: String,
    search: String
});
args!(SaveExposureFindingArgs {
    input: crate::asset_models::ExposureFindingInput
});
args!(ListExposureRunsArgs { project_id: i64 });
args!(ListExposureSourceResultsArgs {
    run_id: i64,
    expected_project_id: i64
});
args!(CancelExposureScanArgs {
    project_id: i64,
    run_id: i64
});
args!(ExportExposureFindingsArgs { project_id: i64, kind: Option<String> });
args!(PageExposureIncidentsArgs { project_id: i64, status: Option<String>, review_needed_only: Option<bool>, page_size: i64, cursor: Option<String> });
args!(ReadExposureIncidentArgs {
    project_id: i64,
    incident_id: i64
});
args!(PageExposureIncidentLocationsArgs { project_id: i64, incident_id: i64, page_size: i64, cursor: Option<String> });
args!(ExposureCoverageDrilldownArgs { project_id: i64, source_key: String, window_days: i64, unit_entity_id: Option<i64>, gaps_only: bool, page_size: i64, cursor: Option<String> });
args!(ActOnExposureIncidentArgs {
    project_id: i64,
    action: Value
});
args!(ProposeExposureIncidentsArgs { project_id: i64 });
args!(VerifyExposureExportArgs { path: String });
args!(ReviewExposureFindingScopedArgs { project_id: i64, finding_id: i64, expected_revision: i64, status: String, actor: String, reason: String, references: Vec<i64>, expected_review_event: i64 });
args!(ListExposureReviewTrailArgs {
    project_id: i64,
    finding_id: i64
});
args!(SaveExposureRemediationArgs { project_id: i64, finding_id: i64, observation_id: Option<i64>, run_id: i64, actor: String, note: String, status: String });
args!(PageExposureRemediationsArgs {
    project_id: i64,
    status: String,
    page_size: i64,
    cursor: String,
    finding_id: Option<i64>
});
args!(StartExposureRetestArgs {
    project_id: i64,
    remediation_id: i64
});
args!(ReadExposureRetestOutcomeArgs {
    project_id: i64,
    remediation_id: i64
});
args!(MarkExposureRemediationArgs {
    project_id: i64,
    remediation_id: i64,
    status: String,
    actor: String,
    reason: String
});
args!(ExposureReviewTaxonomyArgs { project_id: i64 });
args!(StartExposureScanArgs { project_id: i64 });
args!(ResumeExposureScanArgs {
    run_id: i64,
    expected_project_id: i64
});
args!(GetExposureSettingsArgs { project_id: i64 });
args!(SaveExposureSettingsArgs {
    project_id: i64,
    expected_version: i64,
    settings: Value
});
args!(ListExposureSettingVersionsArgs {
    project_id: i64,
    limit: i64
});
args!(PageExposurePendingDiscoveriesArgs {
    project_id: i64,
    status: String,
    page_size: i64,
    cursor: String
});
args!(PageExposureClustersArgs {
    project_id: i64,
    kind: String,
    search: String,
    page_size: i64,
    cursor: String
});
args!(PageExposureClusterMembersArgs {
    project_id: i64,
    cluster_id: i64,
    page_size: i64,
    cursor: String
});
args!(ReadExposureEvidenceArgs {
    finding_id: i64,
    expected_project_id: i64
});
args!(ImportExposureDocumentsArgs {
    project_id: i64,
    request: Value
});
args!(ReclassifyExposureFindingArgs {
    project_id: i64,
    request: Value
});
args!(GradeExposureIntelArgs {
    project_id: i64,
    request: Value
});

pub(crate) fn project_commands() -> &'static [&'static str] {
    &[
        "page_exposure_findings",
        "page_exposure_runs",
        "list_exposure_source_results_for_project",
        "read_exposure_finding",
        "exposure_scope_summary",
        "exposure_known_limits",
        "exposure_summary",
        "list_exposure_findings",
        "save_exposure_finding",
        "list_exposure_runs",
        "list_exposure_source_results",
        "cancel_exposure_scan",
        "export_exposure_findings",
        "page_exposure_incidents",
        "read_exposure_incident",
        "page_exposure_incident_locations",
        "exposure_coverage_drilldown",
        "act_on_exposure_incident",
        "propose_exposure_incidents",
        "verify_exposure_export",
        "review_exposure_finding_scoped",
        "list_exposure_review_trail",
        "save_exposure_remediation",
        "page_exposure_remediations",
        "start_exposure_retest",
        "read_exposure_retest_outcome",
        "mark_exposure_remediation",
        "exposure_review_taxonomy",
        "start_exposure_scan",
        "resume_exposure_scan",
        "get_exposure_settings",
        "save_exposure_settings",
        "list_exposure_setting_versions",
        "page_exposure_pending_discoveries",
        "page_exposure_clusters",
        "page_exposure_cluster_members",
        "read_exposure_evidence",
        "import_exposure_documents",
        "reclassify_exposure_finding",
        "grade_exposure_intel",
    ]
}
pub(crate) fn application_commands() -> &'static [&'static str] {
    &[]
}

/// Project-bound manual UI actions. Never registered as model tools.
pub(crate) fn dispatch_project(
    db_path: &Path,
    export_dir: &Path,
    project: i64,
    command: &str,
    arguments: Value,
) -> Result<Value, String> {
    exposure::with_background_scope(db_path, || {
        dispatch_project_scoped(db_path, export_dir, project, command, arguments)
    })
}

fn dispatch_project_scoped(
    db_path: &Path,
    export_dir: &Path,
    project: i64,
    command: &str,
    arguments: Value,
) -> Result<Value, String> {
    let _admission = super::owned_work::admission()?;
    same_project(project, project)?;
    match command {
        "page_exposure_findings" => {
            let a: PageExposureFindingsArgs = input(arguments)?;
            same_project(project, a.input.project_id)?;
            output(exposure::page_exposure_findings_at(db_path, a.input)?)
        }
        "page_exposure_runs" => {
            let a: PageExposureRunsArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            output(exposure::page_exposure_runs_at(
                db_path,
                a.project_id,
                a.page_size,
                a.cursor,
            )?)
        }
        "list_exposure_source_results_for_project" => {
            let a: ListExposureSourceResultsForProjectArgs = input(arguments)?;
            same_project(project, a.expected_project_id)?;
            output(exposure::list_exposure_source_results_for_project_at(
                db_path,
                a.run_id,
                a.expected_project_id,
            )?)
        }
        "read_exposure_finding" => {
            let a: ReadExposureFindingArgs = input(arguments)?;
            same_project(project, a.expected_project_id)?;
            output(exposure::read_exposure_finding_at(
                db_path,
                a.finding_id,
                a.expected_project_id,
            )?)
        }
        "exposure_scope_summary" => {
            let a: ExposureScopeSummaryArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            output(exposure::exposure_scope_summary_at(db_path, a.project_id)?)
        }
        "exposure_known_limits" => {
            let _: ExposureKnownLimitsArgs = input(arguments)?;
            output(exposure::exposure_known_limits()?)
        }
        "exposure_summary" => {
            let a: ExposureSummaryArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            output(exposure::exposure_summary_at(db_path, a.project_id)?)
        }
        "list_exposure_findings" => {
            let a: ListExposureFindingsArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            output(exposure::list_exposure_findings_at(
                db_path,
                a.project_id,
                a.status,
                a.search,
            )?)
        }
        "save_exposure_finding" => {
            let a: SaveExposureFindingArgs = input(arguments)?;
            same_project(project, a.input.project_id)?;
            output(exposure::save_exposure_finding_at(db_path, a.input)?)
        }
        "list_exposure_runs" => {
            let a: ListExposureRunsArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            output(exposure::list_exposure_runs_at(db_path, a.project_id)?)
        }
        "list_exposure_source_results" => {
            let a: ListExposureSourceResultsArgs = input(arguments)?;
            same_project(project, a.expected_project_id)?;
            output(exposure::list_exposure_source_results_at(
                db_path,
                a.run_id,
                a.expected_project_id,
            )?)
        }
        "cancel_exposure_scan" => {
            let a: CancelExposureScanArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            output(exposure::cancel_exposure_scan_at(
                db_path,
                a.project_id,
                a.run_id,
            )?)
        }
        "export_exposure_findings" => {
            let a: ExportExposureFindingsArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            // Current-project receipt registration wraps the original CSV producer.
            let directory = super::export_artifacts::project_directory(export_dir, project)?;
            let exported =
                exposure::export_exposure_findings_at(db_path, &directory, a.project_id, a.kind)?;
            let receipt = super::export_artifacts::register(
                db_path,
                export_dir,
                project,
                Path::new(&exported.path),
                exported.rows,
                "exposure",
            )?;
            output(
                serde_json::json!({"path":receipt.reference,"rows":exported.rows,"artifactId":receipt.artifact_id,"sha256":receipt.sha256,"totalBytes":receipt.total_bytes,"expiresAt":receipt.expires_at}),
            )
        }
        "page_exposure_incidents" => {
            let a: PageExposureIncidentsArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            output(exposure::page_exposure_incidents_at(
                db_path,
                a.project_id,
                a.status,
                a.review_needed_only,
                a.page_size,
                a.cursor,
            )?)
        }
        "read_exposure_incident" => {
            let a: ReadExposureIncidentArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            output(exposure::read_exposure_incident_at(
                db_path,
                a.project_id,
                a.incident_id,
            )?)
        }
        "page_exposure_incident_locations" => {
            let a: PageExposureIncidentLocationsArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            output(exposure::page_exposure_incident_locations_at(
                db_path,
                a.project_id,
                a.incident_id,
                a.page_size,
                a.cursor,
            )?)
        }
        "exposure_coverage_drilldown" => {
            let a: ExposureCoverageDrilldownArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            output(exposure::exposure_coverage_drilldown_at(
                db_path,
                a.project_id,
                a.source_key,
                a.window_days,
                a.unit_entity_id,
                a.gaps_only,
                a.page_size,
                a.cursor,
            )?)
        }
        "act_on_exposure_incident" => {
            let a: ActOnExposureIncidentArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            output(exposure::act_on_exposure_incident_at(
                db_path,
                a.project_id,
                a.action,
            )?)
        }
        "propose_exposure_incidents" => {
            let a: ProposeExposureIncidentsArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            output(exposure::propose_exposure_incidents_at(
                db_path,
                a.project_id,
            )?)
        }
        "verify_exposure_export" => {
            let a: VerifyExposureExportArgs = input(arguments)?;
            // The legacy DTO field is retained, but its value must be an issued receipt.
            let receipt = super::export_artifacts::resolve(db_path, export_dir, project, &a.path)?;
            let mut verified = exposure::verify_exposure_export_at(
                &receipt.directory,
                receipt.path.to_string_lossy().into_owned(),
            )?;
            verified["path"] = Value::String(a.path);
            output(verified)
        }
        "review_exposure_finding_scoped" => {
            let a: ReviewExposureFindingScopedArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            output(exposure::review_exposure_finding_scoped_at(
                db_path,
                a.project_id,
                a.finding_id,
                a.expected_revision,
                a.status,
                a.actor,
                a.reason,
                a.references,
                a.expected_review_event,
            )?)
        }
        "list_exposure_review_trail" => {
            let a: ListExposureReviewTrailArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            output(exposure::list_exposure_review_trail_at(
                db_path,
                a.project_id,
                a.finding_id,
            )?)
        }
        "save_exposure_remediation" => {
            let a: SaveExposureRemediationArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            output(exposure::save_exposure_remediation_at(
                db_path,
                a.project_id,
                a.finding_id,
                a.observation_id,
                a.run_id,
                a.actor,
                a.note,
                a.status,
            )?)
        }
        "page_exposure_remediations" => {
            let a: PageExposureRemediationsArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            output(exposure::page_exposure_remediations_at(
                db_path,
                a.project_id,
                a.status,
                a.page_size,
                a.cursor,
                a.finding_id,
            )?)
        }
        "start_exposure_retest" => {
            let a: StartExposureRetestArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            super::owned_work::guard_recovery(db_path, super::owned_work::Family::Exposure)?;
            let result =
                exposure::start_exposure_retest_at(db_path, a.project_id, a.remediation_id)?;
            super::owned_work::record(
                db_path,
                super::owned_work::Kind::ExposureRun,
                project,
                &result.to_string(),
            )?;
            output(result)
        }
        "read_exposure_retest_outcome" => {
            let a: ReadExposureRetestOutcomeArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            output(exposure::read_exposure_retest_outcome_at(
                db_path,
                a.project_id,
                a.remediation_id,
            )?)
        }
        "mark_exposure_remediation" => {
            let a: MarkExposureRemediationArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            output(exposure::mark_exposure_remediation_at(
                db_path,
                a.project_id,
                a.remediation_id,
                a.status,
                a.actor,
                a.reason,
            )?)
        }
        "exposure_review_taxonomy" => {
            let a: ExposureReviewTaxonomyArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            output(exposure::exposure_review_taxonomy_at(
                db_path,
                a.project_id,
            )?)
        }
        "start_exposure_scan" => {
            let a: StartExposureScanArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            super::owned_work::guard_recovery(db_path, super::owned_work::Family::Exposure)?;
            let result = exposure::start_exposure_scan_at(db_path, a.project_id)?;
            super::owned_work::record(
                db_path,
                super::owned_work::Kind::ExposureRun,
                project,
                &result.to_string(),
            )?;
            output(result)
        }
        "resume_exposure_scan" => {
            let a: ResumeExposureScanArgs = input(arguments)?;
            same_project(project, a.expected_project_id)?;
            super::owned_work::guard_recovery(db_path, super::owned_work::Family::Exposure)?;
            super::owned_work::require_existing_owned(
                db_path,
                super::owned_work::Kind::ExposureRun,
                &a.run_id.to_string(),
            )?;
            let result =
                exposure::resume_exposure_scan_at(db_path, a.run_id, a.expected_project_id)?;
            super::owned_work::record(
                db_path,
                super::owned_work::Kind::ExposureRun,
                project,
                &result.to_string(),
            )?;
            output(result)
        }
        "get_exposure_settings" => {
            let a: GetExposureSettingsArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            output(exposure::get_exposure_settings_at(db_path, a.project_id)?)
        }
        "save_exposure_settings" => {
            let a: SaveExposureSettingsArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            output(exposure::save_exposure_settings_at(
                db_path,
                a.project_id,
                a.expected_version,
                a.settings,
            )?)
        }
        "list_exposure_setting_versions" => {
            let a: ListExposureSettingVersionsArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            output(exposure::list_exposure_setting_versions_at(
                db_path,
                a.project_id,
                a.limit,
            )?)
        }
        "page_exposure_pending_discoveries" => {
            let a: PageExposurePendingDiscoveriesArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            output(exposure::page_exposure_pending_discoveries_at(
                db_path,
                a.project_id,
                a.status,
                a.page_size,
                a.cursor,
            )?)
        }
        "page_exposure_clusters" => {
            let a: PageExposureClustersArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            output(exposure::page_exposure_clusters_at(
                db_path,
                a.project_id,
                a.kind,
                a.search,
                a.page_size,
                a.cursor,
            )?)
        }
        "page_exposure_cluster_members" => {
            let a: PageExposureClusterMembersArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            output(exposure::page_exposure_cluster_members_at(
                db_path,
                a.project_id,
                a.cluster_id,
                a.page_size,
                a.cursor,
            )?)
        }
        "read_exposure_evidence" => {
            let a: ReadExposureEvidenceArgs = input(arguments)?;
            same_project(project, a.expected_project_id)?;
            output(exposure::read_exposure_evidence_at(
                db_path,
                a.finding_id,
                a.expected_project_id,
            )?)
        }
        "import_exposure_documents" => {
            let a: ImportExposureDocumentsArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            output(exposure::import_exposure_documents_at(
                db_path,
                a.project_id,
                a.request,
            )?)
        }
        "reclassify_exposure_finding" => {
            let a: ReclassifyExposureFindingArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            output(exposure::reclassify_exposure_finding_at(
                db_path,
                a.project_id,
                a.request,
            )?)
        }
        "grade_exposure_intel" => {
            let a: GradeExposureIntelArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            output(exposure::grade_exposure_intel_at(
                db_path,
                a.project_id,
                a.request,
            )?)
        }
        _ => Err("asset_service_exposure_method_unavailable".into()),
    }
}

/// Includes actual owner threads even after the domain row became terminal.
pub(crate) fn status_on(db_path: &Path) -> Result<Value, String> {
    super::owned_work::status_on(
        db_path,
        super::owned_work::Family::Exposure,
        exposure::active_background_workers(db_path)?,
    )
}

#[cfg(test)]
#[path = "exposure_service_tests.rs"]
mod tests;
