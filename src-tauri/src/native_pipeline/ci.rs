//! §10.4 — CI on top of the same Native code pipeline. The freeze records what was
//! actually scanned and the gate reads only stored reviewer decisions. The product
//! JSON/bundle export belongs to commands::native_source_findings; this module only
//! provides the standalone SARIF view and gate evaluation.

use super::analyzer::AnalyzerOutcome;
use super::snapshot::{DiffBaseState, RepositorySnapshot};
#[cfg(test)]
use crate::agent_runtime::review::list_review_decisions;
use crate::agent_runtime::review::{ReviewDecision, ReviewVerdict};
use serde_json::{json, Value as JsonValue};

/// What this CI run covered. `Diff` only exists when the base was proven usable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CiScope {
    Diff,
    Full,
    Unavailable,
}

impl CiScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Diff => "diff",
            Self::Full => "full",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Everything a rerun must be able to reproduce. There is no setter: a new freeze is a
/// new derivation from a snapshot and a set of analyzer outcomes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CiFreeze {
    pub head_sha: String,
    pub base_sha: String,
    pub diff_base_state: String,
    pub tree_hash: String,
    pub rule_pack_digest: String,
    pub analyzer_versions: Vec<(String, String)>,
    pub scope: CiScope,
    pub file_count: usize,
    pub analysis_manifest_digest: Option<String>,
}

impl CiFreeze {
    pub fn of(
        snapshot: &RepositorySnapshot,
        outcomes: &[AnalyzerOutcome],
        requested_scope: CiScope,
    ) -> Self {
        // A diff scope that cannot be proven is downgraded here, at the only place that
        // is allowed to decide the scope, and the downgrade is visible in the freeze.
        let scope = match requested_scope {
            CiScope::Diff if snapshot.diff_base == DiffBaseState::Valid => CiScope::Diff,
            _ => CiScope::Full,
        };
        let mut versions: Vec<(String, String)> = outcomes
            .iter()
            .map(|outcome| {
                (
                    outcome.engine.as_str().to_string(),
                    if outcome.version.is_empty() {
                        "unknown".to_string()
                    } else {
                        outcome.version.clone()
                    },
                )
            })
            .collect();
        versions.sort();
        let mut rule_packs = outcomes
            .iter()
            .map(|outcome| (outcome.engine.as_str(), outcome.rule_pack_digest.as_str()))
            .collect::<Vec<_>>();
        rule_packs.sort();
        // Preserve the historical single/shared-pack identity, but never discard
        // another engine's distinct rules. Engine order cannot change the freeze.
        let rule_pack_digest = match rule_packs.first() {
            None => String::new(),
            Some((_, digest)) if rule_packs.iter().all(|(_, value)| value == digest) => {
                (*digest).to_string()
            }
            Some(_) => crate::artifact_import::canonical::sha256_hex(
                json!(rule_packs).to_string().as_bytes(),
            ),
        };
        Self {
            head_sha: snapshot.commit_sha.clone(),
            base_sha: snapshot.base_sha.clone(),
            diff_base_state: snapshot.diff_base.as_str().to_string(),
            tree_hash: snapshot.tree_hash.clone(),
            rule_pack_digest,
            analyzer_versions: versions,
            scope,
            file_count: snapshot.file_count(),
            analysis_manifest_digest: None,
        }
    }

    /// Production freezes describe the independently materialized analyzer input,
    /// while tree_hash continues to identify the complete provenance snapshot.
    pub fn of_analysis(
        snapshot: &RepositorySnapshot,
        outcomes: &[AnalyzerOutcome],
        manifest: &super::analysis_view::AnalysisManifest,
    ) -> Self {
        let mut freeze = Self::of(snapshot, outcomes, CiScope::Full);
        freeze.scope = match manifest.scope.as_str() {
            "full" => CiScope::Full,
            "diff" => CiScope::Diff,
            _ => CiScope::Unavailable,
        };
        freeze.file_count = manifest.files.len();
        freeze.analysis_manifest_digest = Some(manifest.digest());
        freeze
    }

    pub fn as_json(&self) -> JsonValue {
        json!({
            "schemaVersion": 1,
            "headSha": self.head_sha,
            "baseSha": self.base_sha,
            "diffBase": self.diff_base_state,
            "treeHash": self.tree_hash,
            "rulePackDigest": self.rule_pack_digest,
            "analyzers": self.analyzer_versions.iter().map(|(engine, version)| json!({
                "engine": engine, "version": version
            })).collect::<Vec<_>>(),
            "scope": self.scope.as_str(),
            "fileCount": self.file_count,
            "analysisManifestDigest": self.analysis_manifest_digest,
        })
    }

    /// Written into the exported bundle so a later import can be tied to the freeze.
    pub fn stable_text(&self) -> String {
        self.as_json().to_string()
    }
}

/// Where the gate landed. `exit_code` is the CI contract; the distinct statuses say
/// *why*, which a bare number cannot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateStatus {
    Passed,
    Warning,
    Blocked,
    CoverageIncomplete,
    Inconclusive,
    InfraFailed,
}

impl GateStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Warning => "warning",
            Self::Blocked => "blocked",
            Self::CoverageIncomplete => "coverage_incomplete",
            Self::Inconclusive => "inconclusive",
            Self::InfraFailed => "infra_failed",
        }
    }

    /// 0 = ship, 2 = a confirmed finding over the threshold, 3 = the answer is not
    /// known, 4 = the run itself could not be trusted (§10.4).
    pub fn exit_code(self) -> i32 {
        match self {
            Self::Passed | Self::Warning => 0,
            Self::Blocked => 2,
            Self::CoverageIncomplete | Self::Inconclusive => 3,
            Self::InfraFailed => 4,
        }
    }

    /// Only these two may ever be read as "the build is green".
    pub fn is_green(self) -> bool {
        self.exit_code() == 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseLimits {
    pub max_critical: usize,
    pub max_high: usize,
    pub block_release: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GatePolicy {
    /// Workbench thresholds are independent severity limits, not one summed quota.
    /// None retains the legacy standalone gate API; task execution must use Some.
    pub release_limits: Option<ReleaseLimits>,
    /// Severities that stop a release once a reviewer confirmed them.
    pub block_severities: Vec<String>,
    /// How many blocking findings are tolerated before the gate fails.
    pub allowed_blocking: usize,
    pub warning_severities: Vec<String>,
    /// Explicit opt-in to scan the whole tree when the diff base is unusable.
    pub full_scan_on_bad_base: bool,
}

impl Default for GatePolicy {
    fn default() -> Self {
        Self {
            release_limits: None,
            block_severities: vec!["critical".to_string(), "high".to_string()],
            allowed_blocking: 0,
            warning_severities: vec!["medium".to_string()],
            full_scan_on_bad_base: false,
        }
    }
}

impl GatePolicy {
    pub fn from_workbench(value: &JsonValue) -> Result<Self, String> {
        let limit = |key| {
            value
                .get(key)
                .and_then(JsonValue::as_u64)
                .filter(|v| *v <= 10_000)
                .map(|v| v as usize)
                .ok_or_else(|| "workbench_ci_policy_invalid".to_string())
        };
        Ok(Self {
            release_limits: Some(ReleaseLimits {
                max_critical: limit("maxCritical")?,
                max_high: limit("maxHigh")?,
                block_release: value
                    .get("blockRelease")
                    .and_then(JsonValue::as_bool)
                    .ok_or("workbench_ci_policy_invalid")?,
            }),
            ..Self::default()
        })
    }

    pub fn from_json(value: &JsonValue) -> Self {
        let strings = |keys: &[&str]| -> Vec<String> {
            keys.iter()
                .filter_map(|key| value.get(*key))
                .find_map(JsonValue::as_array)
                .map(|rows| {
                    rows.iter()
                        .filter_map(JsonValue::as_str)
                        .map(|row| row.trim().to_ascii_lowercase())
                        .filter(|row| !row.is_empty())
                        .collect()
                })
                .unwrap_or_default()
        };
        let fallback = Self::default();
        Self {
            release_limits: None,
            block_severities: if value.get("blockSeverities").is_some() {
                strings(&["blockSeverities"])
            } else {
                fallback.block_severities
            },
            warning_severities: if value.get("warningSeverities").is_some() {
                strings(&["warningSeverities"])
            } else {
                fallback.warning_severities
            },
            allowed_blocking: value
                .get("allowedBlocking")
                .and_then(JsonValue::as_u64)
                .unwrap_or(0) as usize,
            full_scan_on_bad_base: value
                .get("fullScanOnBadBase")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false),
        }
    }

    pub fn as_json(&self) -> JsonValue {
        json!({
            "releaseLimits": self.release_limits,
            "policySource": if self.release_limits.is_some() { "workbench_task" } else { "standalone" },
            "blockSeverities": self.block_severities,
            "warningSeverities": self.warning_severities,
            "allowedBlocking": self.allowed_blocking,
            "fullScanOnBadBase": self.full_scan_on_bad_base,
        })
    }
}

#[derive(Clone, Debug)]
pub struct GateReport {
    pub status: GateStatus,
    pub freeze: CiFreeze,
    pub policy: GatePolicy,
    pub reasons: Vec<String>,
    pub gaps: Vec<String>,
    pub decisions: Vec<ReviewDecision>,
    pub blocking: Vec<ReviewDecision>,
}

impl GateReport {
    pub fn exit_code(&self) -> i32 {
        self.status.exit_code()
    }

    /// The rows a CI log and the importer both consume: candidates and confirmed
    /// findings, never a bare count.
    pub fn findings(&self) -> Vec<JsonValue> {
        let mut rows = Vec::new();
        for decision in &self.decisions {
            if decision.verdict != ReviewVerdict::Confirmed {
                continue;
            }
            let (path, line) = evidence_location(&decision.evidence_refs);
            rows.push(json!({
                "id": decision.candidate_id,
                "title": decision.reason_codes.first().cloned().unwrap_or_else(|| "reviewer-confirmed".to_string()),
                "severity": decision.severity,
                "cwe": "",
                "endpoint": "",
                "path": path,
                "line": line,
                "reviewState": "confirmed",
                "decisionRevision": decision.candidate_revision,
                "reviewerRunId": decision.reviewer_run_id,
            }));
        }
        rows
    }
}

/// The gate's only input surface: stored, immutable reviewer decisions for this root.
/// Findings imported from another scan's history, and candidates nobody reviewed, are
/// not here — so they cannot block (§10.4, CI-004).
// Threshold fixtures may inject review decisions. Production source CI must
// not use this unqualified root-wide reader as proof of review eligibility.
#[cfg(test)]
pub fn evaluate(
    connection: &rusqlite::Connection,
    root_run_id: &str,
    freeze: &CiFreeze,
    coverage_gaps: &[String],
    infra_failure: Option<&str>,
    policy: GatePolicy,
) -> Result<GateReport, String> {
    let decisions = list_review_decisions(connection, root_run_id)?;
    evaluate_decisions(decisions, freeze, coverage_gaps, infra_failure, policy)
}

/// Analyzer completion is not independent review. Until the source review
/// delivery is bound to this attempt, no historical decision is eligible and
/// even a nonempty, otherwise fully covered scan cannot be declared passed.
pub fn evaluate_unreviewed(
    freeze: &CiFreeze,
    coverage_gaps: &[String],
    infra_failure: Option<&str>,
    policy: GatePolicy,
) -> Result<GateReport, String> {
    let mut gaps = coverage_gaps.to_vec();
    if !gaps.iter().any(|gap| gap == "source_review_not_completed") {
        gaps.push("source_review_not_completed".into());
    }
    evaluate_decisions(Vec::new(), freeze, &gaps, infra_failure, policy)
}

fn evaluate_decisions(
    decisions: Vec<ReviewDecision>,
    freeze: &CiFreeze,
    coverage_gaps: &[String],
    infra_failure: Option<&str>,
    policy: GatePolicy,
) -> Result<GateReport, String> {
    let blocking: Vec<ReviewDecision> = decisions
        .iter()
        .filter(|decision| {
            decision.verdict == ReviewVerdict::Confirmed
                && policy
                    .block_severities
                    .iter()
                    .any(|severity| severity == &decision.severity.to_ascii_lowercase())
        })
        .cloned()
        .collect();
    let warning_only: Vec<ReviewDecision> = decisions
        .iter()
        .filter(|decision| {
            decision.verdict == ReviewVerdict::Confirmed
                && policy
                    .warning_severities
                    .iter()
                    .any(|severity| severity == &decision.severity.to_ascii_lowercase())
        })
        .cloned()
        .collect();
    let critical = blocking
        .iter()
        .filter(|row| row.severity.eq_ignore_ascii_case("critical"))
        .count();
    let high = blocking
        .iter()
        .filter(|row| row.severity.eq_ignore_ascii_case("high"))
        .count();
    let (status, reasons, gaps) = evaluate_risk_counts(
        freeze,
        coverage_gaps,
        infra_failure,
        &policy,
        RiskCounts {
            blocking: blocking.len(),
            critical,
            high,
            warnings: warning_only.len(),
        },
    );
    Ok(GateReport {
        status,
        freeze: freeze.clone(),
        policy,
        reasons,
        gaps,
        decisions,
        blocking,
    })
}

/// Source and Web keep different decision identities, sharing only threshold
/// arithmetic and status precedence. This function confers no review authority.
pub(super) struct RiskCounts {
    pub blocking: usize,
    pub critical: usize,
    pub high: usize,
    pub warnings: usize,
}

pub(super) fn evaluate_risk_counts(
    freeze: &CiFreeze,
    coverage_gaps: &[String],
    infra_failure: Option<&str>,
    policy: &GatePolicy,
    counts: RiskCounts,
) -> (GateStatus, Vec<String>, Vec<String>) {
    let mut reasons = Vec::new();
    let mut gaps = coverage_gaps.to_vec();
    let critical_count = counts.critical;
    let high_count = counts.high;
    let (threshold_exceeded, block_release, threshold_reason) = match &policy.release_limits {
        Some(limits) => (
            critical_count > limits.max_critical || high_count > limits.max_high,
            limits.block_release,
            format!(
                "已确认 Critical {critical_count} 条（允许 {}），High {high_count} 条（允许 {}）",
                limits.max_critical, limits.max_high
            ),
        ),
        None => (
            counts.blocking > policy.allowed_blocking,
            true,
            format!(
                "已确认的高危发现 {} 条，允许 {} 条",
                counts.blocking, policy.allowed_blocking
            ),
        ),
    };
    let status = {
        if let Some(detail) = infra_failure {
            reasons.push(format!("配置或基础设施失败，本轮结论不可用：{detail}"));
            GateStatus::InfraFailed
        } else if threshold_exceeded && block_release {
            reasons.push(format!("{threshold_reason}；超过阈值，已阻断发布"));
            GateStatus::Blocked
        } else if freeze.scope == CiScope::Unavailable {
            reasons.push(format!(
                "diff base 或变更清单不可用（{}），没有执行整仓回退",
                freeze.diff_base_state
            ));
            GateStatus::Inconclusive
        } else if !freeze.base_sha.is_empty()
            && freeze.diff_base_state != DiffBaseState::Valid.as_str()
            && !policy.full_scan_on_bad_base
        {
            reasons.push(format!(
                "diff base 状态为 {}，增量范围不可信；未启用整仓策略，结论不确定",
                freeze.diff_base_state
            ));
            GateStatus::Inconclusive
        } else if freeze.file_count == 0 {
            reasons.push("本轮实际扫描为空集，不能报通过".to_string());
            GateStatus::Inconclusive
        } else if !gaps.is_empty() {
            reasons.push(format!("覆盖不完整：{}", gaps.join(", ")));
            GateStatus::CoverageIncomplete
        } else if policy.release_limits.is_some() && counts.blocking > 0 {
            reasons.push(format!(
                "{threshold_reason}；{}",
                if threshold_exceeded {
                    "本任务未启用超阈值阻断，保留风险警告"
                } else {
                    "未超过本任务阈值，保留风险警告"
                }
            ));
            GateStatus::Warning
        } else if counts.warnings > 0 {
            reasons.push(format!(
                "已确认的中危发现 {} 条，未达阻断阈值",
                counts.warnings
            ));
            GateStatus::Warning
        } else {
            reasons.push("没有阻断项，覆盖完整".to_string());
            GateStatus::Passed
        }
    };
    if policy.full_scan_on_bad_base
        && !freeze.base_sha.is_empty()
        && freeze.diff_base_state != DiffBaseState::Valid.as_str()
    {
        gaps.push(format!(
            "diff_base_unusable_fallback_full:{}",
            freeze.diff_base_state
        ));
    }
    (status, reasons, gaps)
}

impl GateReport {
    pub fn as_json(&self) -> JsonValue {
        json!({
            "schemaVersion": 1,
            "product": "oviraptor-nest",
            "status": self.status.as_str(),
            "exitCode": self.exit_code(),
            "freeze": self.freeze.as_json(),
            "policy": self.policy.as_json(),
            "reasons": self.reasons,
            "gaps": self.gaps,
            "counts": {
                "decisions": self.decisions.len(),
                "confirmed": self.decisions.iter().filter(|row| row.verdict == ReviewVerdict::Confirmed).count(),
                "rejected": self.decisions.iter().filter(|row| row.verdict == ReviewVerdict::Rejected).count(),
                "insufficient": self.decisions.iter().filter(|row| row.verdict == ReviewVerdict::InsufficientEvidence).count(),
                "blocking": self.blocking.len(),
            },
            "findings": self.findings(),
        })
    }

    /// SARIF 2.1 with the same rows, so a CI artifact can be consumed by either side.
    pub fn as_sarif(&self) -> JsonValue {
        let results: Vec<JsonValue> = self
            .findings()
            .iter()
            .map(|row| {
                let severity = row
                    .get("severity")
                    .and_then(JsonValue::as_str)
                    .unwrap_or("informational");
                json!({
                    "ruleId": row.get("id").and_then(JsonValue::as_str).unwrap_or_default(),
                    "level": if matches!(severity, "critical" | "high") { "error" } else { "warning" },
                    "message": {"text": row.get("title").and_then(JsonValue::as_str).unwrap_or_default()},
                    "locations": [{
                        "physicalLocation": {
                            "artifactLocation": {"uri": row.get("path").and_then(JsonValue::as_str).unwrap_or_default()},
                            "region": location_region(row),
                        }
                    }],
                    "partialFingerprints": {
                        "oviraptorFindingId": row.get("id").and_then(JsonValue::as_str).unwrap_or_default(),
                        "reviewState": "confirmed",
                        "severity": severity,
                    }
                })
            })
            .collect();
        let rules: Vec<JsonValue> = results
            .iter()
            .map(|row| {
                json!({
                    "id": row.get("ruleId").and_then(JsonValue::as_str).unwrap_or_default(),
                    "name": row.get("ruleId").and_then(JsonValue::as_str).unwrap_or_default(),
                    "shortDescription": {"text": row.pointer("/message/text").and_then(JsonValue::as_str).unwrap_or_default()}
                })
            })
            .collect();
        json!({
            "version": "2.1.0",
            "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
            "runs": [{
                "tool": {
                    "driver": {
                        "name": "oviraptor-nest",
                        "version": self.freeze.analyzer_versions.iter().map(|(_, value)| value.as_str()).collect::<Vec<_>>().join("+"),
                        "rules": rules,
                        "properties": {
                            "treeHash": self.freeze.tree_hash,
                            "headSha": self.freeze.head_sha,
                            "rulePackDigest": self.freeze.rule_pack_digest,
                            "gateStatus": self.status.as_str(),
                        }
                    }
                },
                "results": results,
            }]
        })
    }
}

/// A reviewer cites evidence as `path/to/file.ext:line`; any other reference carries no
/// source location, and an export without one must not invent one.
fn evidence_location(references: &[String]) -> (String, u64) {
    for reference in references {
        let Some((head, tail)) = reference.rsplit_once(':') else {
            continue;
        };
        let Ok(line) = tail.trim().parse::<u64>() else {
            continue;
        };
        if line == 0 || !(head.contains('/') || head.contains('.')) {
            continue;
        }
        return (head.trim().to_string(), line);
    }
    (String::new(), 0)
}

/// The region slot both export formats must agree on, or the merge sees two findings.
fn location_region(row: &JsonValue) -> JsonValue {
    match row.get("line").and_then(JsonValue::as_u64) {
        Some(line) if line > 0 => json!({"startLine": line}),
        _ => JsonValue::Null,
    }
}
