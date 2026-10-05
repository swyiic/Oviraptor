//! Same-attempt source CI projection. No Web revision coercion, root-wide
//! historical decision lookup, or mutation of the original analyzer plan.
use super::{
    ci::{self, CiFreeze, CiScope, GatePolicy, GateStatus, RiskCounts},
    snapshot::RepositorySnapshot,
    NativeSourcePlan,
};
use crate::agent_runtime::multi_agent::{
    lease::CoordinatorLease,
    source,
    source_review_contract::SourceVerdict,
    source_review_projection::{self, SourceReviewProjection},
};
use rusqlite::Connection;
use serde_json::{json, Value};

#[derive(Debug, PartialEq)]
pub(crate) struct SourceGateReport {
    pub(crate) status: GateStatus,
    pub(crate) reasons: Vec<String>,
    value: Value,
}

impl SourceGateReport {
    pub(crate) fn as_json(&self) -> Value {
        self.value.clone()
    }
}

/// Read-only, in the caller's transaction. Policy must come from this exact
/// attempt's immutable publication. Execution callers revalidate after writes;
/// historical exporters do not acquire or renew execution authority.
pub(crate) fn evaluate(
    db: &Connection,
    lease: &CoordinatorLease,
    decisions: &SourceReviewProjection,
    policy: GatePolicy,
) -> Result<SourceGateReport, String> {
    if db.is_autocommit() {
        return Err("source_ci_transaction_required".into());
    }
    if source_review_projection::read_audited(db, lease)? != *decisions {
        return Err("source_ci_decision_set_changed".into());
    }
    let (view, results, plan_hash) =
        source::historical_materials(db, &lease.scan_id, lease.attempt_number)?;
    let snapshot = RepositorySnapshot::restore(db, &lease.scan_id, lease.attempt_number)?
        .ok_or("source_ci_snapshot_missing")?;
    let plan = NativeSourcePlan::load(db, &lease.scan_id, lease.attempt_number)?
        .ok_or("source_ci_plan_missing")?;
    if plan.hash() != plan_hash || plan.scan_type != "cicd" {
        return Err("source_ci_plan_mismatch".into());
    }
    // Reconstruct from sealed analyzer receipts, not the caller's mutable report.
    let mut versions = Vec::new();
    let mut packs = Vec::new();
    for run in &results.runs {
        let text = |key: &str| {
            run[key]
                .as_str()
                .ok_or("source_ci_analyzer_receipt_invalid")
        };
        let engine = text("engine")?;
        let version = text("version")?;
        if text("analyzerVersion")? != version {
            return Err("source_ci_analyzer_version_mismatch".into());
        }
        versions.push((
            engine.to_string(),
            if version.is_empty() {
                "unknown".into()
            } else {
                version.into()
            },
        ));
        packs.push((engine, text("rulePackDigest")?));
    }
    versions.sort();
    packs.sort();
    let rule_pack_digest = match packs.first() {
        None => String::new(),
        Some((_, digest)) if packs.iter().all(|(_, value)| value == digest) => {
            (*digest).to_string()
        }
        Some(_) => {
            crate::artifact_import::canonical::sha256_hex(json!(packs).to_string().as_bytes())
        }
    };
    let freeze = CiFreeze {
        head_sha: snapshot.commit_sha,
        base_sha: snapshot.base_sha,
        diff_base_state: snapshot.diff_base.as_str().into(),
        tree_hash: snapshot.tree_hash,
        rule_pack_digest,
        analyzer_versions: versions,
        scope: match view.manifest.scope.as_str() {
            "full" => CiScope::Full,
            "diff" => CiScope::Diff,
            _ => CiScope::Unavailable,
        },
        file_count: view.manifest.files.len(),
        analysis_manifest_digest: Some(view.manifest.digest()),
    };
    // The original analyzer-stage plan is not the whole coverage ledger:
    // actual source-tool rounds may discover additional limitations. Reaudit
    // those receipts instead of losing them when publishing the reviewed gate.
    let gaps = decisions.gaps()?;
    let confirmed = decisions
        .decisions()
        .filter(|d| d.verdict == SourceVerdict::Confirmed)
        .collect::<Vec<_>>();
    let blocking = confirmed
        .iter()
        .filter(|d| {
            policy
                .block_severities
                .contains(&d.severity.to_ascii_lowercase())
        })
        .collect::<Vec<_>>();
    let counts = RiskCounts {
        blocking: blocking.len(),
        critical: blocking.iter().filter(|d| d.severity == "critical").count(),
        high: blocking.iter().filter(|d| d.severity == "high").count(),
        warnings: confirmed
            .iter()
            .filter(|d| {
                policy
                    .warning_severities
                    .contains(&d.severity.to_ascii_lowercase())
            })
            .count(),
    };
    let (status, reasons, gaps) = ci::evaluate_risk_counts(&freeze, &gaps, None, &policy, counts);
    let projection = decisions.as_json();
    let mut summary = projection["counts"].clone();
    summary["blocking"] = json!(blocking.len());
    let value = json!({"schemaVersion":2,"product":"oviraptor-nest","surface":"source",
        "scanId":lease.scan_id,"attemptNumber":lease.attempt_number,"rootRunId":lease.root_run_id,
        "status":status.as_str(),"exitCode":status.exit_code(),"freeze":freeze.as_json(),
        "policy":policy.as_json(),"reasons":reasons,"gaps":gaps,"counts":summary,
        "sourceDecisionProjection":decisions.candidates().map(|set|set.as_json()),
        "coverage":decisions.coverage_summary(),"findings":projection["findings"],
        "independentCandidateReviewCompleted":decisions.candidates().is_some(),
        "independentReviewCompleted":decisions.coverage().is_some()});
    Ok(SourceGateReport {
        status,
        reasons,
        value,
    })
}
