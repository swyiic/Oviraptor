//! Receipt-backed coverage preparation, not a coverage verdict or a new phase
//! grant. Keep this separate from the historical candidate material contract:
//! changing that material would invalidate already delivered v1-v3 receipts.
use super::{lease::CoordinatorLease, source, source_phases, source_reviewer};
use crate::{
    agent_runtime::secrets::redact_json,
    artifact_import::canonical::sha256_hex,
    native_pipeline::{snapshot::RepositorySnapshot, NativeSourcePlan},
};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::collections::BTreeSet;

#[derive(Debug, PartialEq)]
pub(crate) struct CoveragePreparation {
    value: Value,
    pub(crate) gaps: Vec<String>,
}

impl CoveragePreparation {
    pub(crate) fn as_json(&self) -> Value {
        self.value.clone()
    }

    /// Bounded-detail UI/report projection. Keep bulk source evidence in its
    /// original receipt store rather than repeating it on every findings page.
    pub(crate) fn summary(&self) -> Value {
        let material = &self.value["material"];
        json!({"schemaVersion":1,"status":"prepared_not_reviewed",
            "scanId":material["scanId"],"attemptNumber":material["attemptNumber"],
            "rootRunId":material["rootRunId"],"materialDigest":self.value["digest"],
            "executionEligible":false,"independentReviewCompleted":false,
            "requestedScope":material["scope"]["requested"],
            "effectiveScope":material["scope"]["effective"],
            "selectedFileCount":material["scope"]["selectedFiles"].as_array().map(Vec::len),
            "changedPathsWithoutContentCount":material["scope"]["changedPathsWithoutContent"].as_array().map(Vec::len),
            "outstandingGaps":self.gaps})
    }
}

/// Rebuild under the caller's read transaction, including for an old attempt.
/// No execution check, lease renewal, model dispatch, or projection repair.
/// Candidate completion only removes the candidate-review placeholder, never
/// tool-declared limitations or deterministic scope/analyzer gaps.
pub(crate) fn audit(
    db: &Connection,
    lease: &CoordinatorLease,
) -> Result<CoveragePreparation, String> {
    if db.is_autocommit() {
        return Err("source_coverage_transaction_required".into());
    }
    let phases = source_phases::audit(db, lease)?;
    let (view, results, plan_hash) =
        source::historical_materials(db, &lease.scan_id, lease.attempt_number)?;
    let snapshot = RepositorySnapshot::restore(db, &lease.scan_id, lease.attempt_number)?
        .ok_or("source_coverage_snapshot_missing")?;
    let plan = NativeSourcePlan::load(db, &lease.scan_id, lease.attempt_number)?
        .ok_or("source_coverage_plan_missing")?;
    if plan.hash() != plan_hash {
        return Err("source_coverage_plan_mismatch".into());
    }
    let mut reported = BTreeSet::new();
    for (origin, gaps) in [
        ("source_snapshot", &snapshot.gaps),
        ("analysis_scope", &view.manifest.gaps),
        ("source_plan", &plan.gaps),
        ("analyzer_results", &results.gaps),
    ] {
        for gap in gaps {
            reported.insert((origin.to_string(), String::new(), gap.clone()));
        }
    }
    for assessment in &phases.assessments {
        // Only source_tools has a typed, audited finish result. Free-text
        // assessments are not parsed as grants or machine coverage claims.
        if assessment.get("result").is_none() {
            continue;
        }
        let assignment = assessment["assignmentId"]
            .as_str()
            .ok_or("source_coverage_assignment_missing")?;
        let gaps = assessment["result"]["result"]["gaps"]
            .as_array()
            .ok_or("source_coverage_tool_gaps_invalid")?;
        for gap in gaps {
            reported.insert((
                "tool_assignment".to_string(),
                assignment.to_string(),
                gap.as_str()
                    .ok_or("source_coverage_tool_gap_invalid")?
                    .to_string(),
            ));
        }
    }
    let (candidate_review, reviewed, insufficient) = match source_reviewer::progress(db, lease)? {
        source_reviewer::ReviewProgress::NotStarted => (
            json!({"status":if phases.review_material["decisionContract"].is_null() {
                "not_applicable_no_candidates"
            } else {"not_started"}}),
            false,
            false,
        ),
        source_reviewer::ReviewProgress::Delivered(review) => {
            let insufficient = review.payload["result"]["decisions"]
                .as_array()
                .ok_or("source_coverage_decisions_missing")?
                .iter()
                .any(|d| d["verdict"] == "insufficient_evidence");
            (
                json!({"status":"completed","assignmentId":review.child.assignment_id,
                    "runId":review.child.run_id,"messageId":review.message_id,
                    "payloadSha256":sha256_hex(review.payload.to_string().as_bytes()),
                    "sourceDecisionIds":review.decision_ids}),
                true,
                insufficient,
            )
        }
        source_reviewer::ReviewProgress::Undispatched(_)
        | source_reviewer::ReviewProgress::Received(_) => {
            return Err("source_coverage_candidate_review_pending".into());
        }
    };
    let mut gaps = reported
        .iter()
        .filter(|(_, _, code)| !(reviewed && code == "source_review_not_completed"))
        .map(|(_, _, code)| code.clone())
        .collect::<BTreeSet<_>>();
    gaps.insert("source_coverage_review".into());
    if !reviewed {
        gaps.insert("source_independent_review".into());
    }
    if insufficient {
        gaps.insert("source_candidate_evidence_incomplete".into());
    }
    let gaps = gaps.into_iter().collect::<Vec<_>>();
    let tool_refs = phases.review_material["material"]["toolEvidence"]
        .as_array()
        .ok_or("source_coverage_tools_missing")?
        .iter()
        .map(|tool| {
            json!({"sha256":sha256_hex(tool.to_string().as_bytes()),
                "assignmentId":tool["assignmentId"],"authorRunId":tool["authorRunId"],
                "roundNumber":tool["roundNumber"],"callIndex":tool["callIndex"],
                "name":tool["call"]["name"]})
        })
        .collect::<Vec<_>>();
    // This is an evidence index. A selected file or analyzer run is NOT proof
    // that every line/rule was checked. Actual evidence is resolved separately
    // by its audited material/receipt hash; don't duplicate all source output.
    let material = redact_json(&json!({
        "schemaVersion":1,"surface":"source","subject":"source_coverage",
        "scanId":lease.scan_id,"attemptNumber":lease.attempt_number,
        "rootRunId":lease.root_run_id,"target":lease.target_key,
        "sourcePlanHash":plan_hash,"analysisDigest":view.manifest.digest(),
        "analysisResultsDigest":results.digest(),
        "reviewMaterialDigest":phases.review_material["digest"],
        "scope":{"requested":view.manifest.request["scopeMode"],
            "effective":view.manifest.scope,"fallbackReason":view.manifest.fallback_reason,
            "selectedFiles":view.manifest.files,
            "changedPathsWithoutContent":view.manifest.changed_paths_without_content,
            "selectedFilesAreCoverageProof":false},
        "snapshot":{"treeHash":snapshot.tree_hash,"headSha":snapshot.commit_sha,
            "baseSha":snapshot.base_sha,"diffBaseState":snapshot.diff_base.as_str(),
            "frozenFileCount":snapshot.files.len()},
        "plannedCapabilities":{"languages":plan.languages,"analyzers":plan.analyzers,"tools":plan.tools},
        "analyzerRuns":results.runs,"analyzerRunsArePerFileCoverageProof":false,
        "phaseReceipts":phases.manifest,"toolEvidenceRefs":tool_refs,
        "candidateReview":candidate_review,
        "reportedGaps":reported.into_iter().map(|(origin,assignment,code)|
            json!({"origin":origin,"assignmentId":if assignment.is_empty() {None} else {Some(assignment)},"code":code})
        ).collect::<Vec<_>>(),
        "outstandingGaps":gaps,"toolsGranted":[],"targetRequestsGranted":0,"hostActionsGranted":0
    }));
    // Use the redacted projection consistently; raw error text must not reappear
    // through CI gaps or closure summaries after material redaction.
    let gaps = serde_json::from_value::<Vec<String>>(material["outstandingGaps"].clone())
        .map_err(|_| "source_coverage_gap_encoding_failed")?;
    Ok(CoveragePreparation {
        value: json!({"schemaVersion":1,"status":"prepared_not_reviewed",
            "executionEligible":false,"independentReviewCompleted":false,"coverageSufficient":null,
            "digest":sha256_hex(material.to_string().as_bytes()),
            "outstandingGaps":gaps,"material":material}),
        gaps,
    })
}
