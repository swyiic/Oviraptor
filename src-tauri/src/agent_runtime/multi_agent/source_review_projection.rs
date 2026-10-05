//! Shared, receipt-audited qualification for findings, CI and exports. Candidate
//! review and overall coverage remain separate; zero candidates never creates
//! an empty candidate Reviewer or a synthetic SourceDecisionSet.
use super::{
    lease::CoordinatorLease,
    source_coverage::{self, CoveragePreparation},
    source_coverage_decisions::{self, CoverageDecision},
    source_coverage_reviewer,
    source_decisions::{self, SourceDecisionSet},
    source_review_contract::CandidateDecision,
};
use rusqlite::Connection;
use serde_json::{json, Value};

#[derive(Debug, PartialEq)]
pub(crate) struct SourceReviewProjection {
    candidates: Option<SourceDecisionSet>,
    coverage: Option<CoverageDecision>,
    preparation: CoveragePreparation,
}

impl SourceReviewProjection {
    pub(crate) fn candidates(&self) -> Option<&SourceDecisionSet> {
        self.candidates.as_ref()
    }

    pub(crate) fn coverage(&self) -> Option<&CoverageDecision> {
        self.coverage.as_ref()
    }

    pub(crate) fn decisions(&self) -> impl Iterator<Item = &CandidateDecision> {
        self.candidates.iter().flat_map(|set| set.decisions())
    }

    pub(crate) fn confirmed_count(&self) -> usize {
        self.candidates
            .as_ref()
            .map_or(0, SourceDecisionSet::confirmed_count)
    }

    pub(crate) fn coverage_summary(&self) -> Value {
        let mut summary = self.preparation.summary();
        if let Some(coverage) = &self.coverage {
            let record = coverage.as_json();
            summary["status"] = json!("reviewed");
            summary["independentReviewCompleted"] = json!(true);
            summary["coverageSufficient"] = record["decision"]["coverageSufficient"].clone();
            summary["outstandingGaps"] = record["decision"]["outstandingGaps"].clone();
            summary["sourceCoverageDecision"] = record;
        }
        summary
    }

    pub(crate) fn gaps(&self) -> Result<Vec<String>, String> {
        serde_json::from_value(self.coverage_summary()["outstandingGaps"].clone())
            .map_err(|_| "source_review_projection_gaps_invalid".into())
    }

    pub(crate) fn as_json(&self) -> Value {
        let material = self.preparation.as_json();
        let mut projection = self.candidates.as_ref().map_or_else(
            || {
                json!({"schemaVersion":1,"surface":"source",
                "scanId":material["material"]["scanId"],
                "attemptNumber":material["material"]["attemptNumber"],
                "rootRunId":material["material"]["rootRunId"],
                "materialDigest":material["digest"],
                "independentCandidateReviewCompleted":false,
                "counts":{"decisions":0,"confirmed":0,"rejected":0,"insufficient":0},
                "findings":[]})
            },
            SourceDecisionSet::as_json,
        );
        projection["candidateReviewStatus"] =
            material["material"]["candidateReview"]["status"].clone();
        projection["materialSubject"] = json!(if self.candidates.is_some() {
            "source_candidates"
        } else {
            "source_coverage"
        });
        projection["independentReviewCompleted"] = json!(self.coverage.is_some());
        projection["coverage"] = self.coverage_summary();
        projection
    }
}

/// Historical, read-only audit. The caller retains the transaction; no lease is
/// acquired, no missing receipt is repaired, and display/import JSON is ignored.
pub(crate) fn read_audited(
    db: &Connection,
    lease: &CoordinatorLease,
) -> Result<SourceReviewProjection, String> {
    if db.is_autocommit() {
        return Err("source_review_projection_transaction_required".into());
    }
    let preparation = source_coverage::audit(db, lease)?;
    let coverage = if source_coverage_reviewer::enabled(db, lease)? {
        Some(source_coverage_decisions::read_audited(db, lease)?)
    } else {
        let count: i64 = db
            .query_row(
                "SELECT count(*) FROM agent_source_coverage_decisions WHERE root_run_id=?1",
                [&lease.root_run_id],
                |r| r.get(0),
            )
            .map_err(|_| "source_review_projection_coverage_inventory_failed")?;
        if count != 0 {
            return Err("source_review_projection_unplanned_coverage_decisions".into());
        }
        None
    };
    let candidates = match preparation.as_json()["material"]["candidateReview"]["status"].as_str() {
        Some("completed") => Some(source_decisions::read_audited(db, lease)?),
        Some("not_applicable_no_candidates") if coverage.is_some() => {
            // A zero-candidate coverage receipt does not license stray candidate
            // publication rows. Do not silently hide them behind zero counts.
            let count: i64 = db
                .query_row(
                    "SELECT count(*) FROM agent_source_review_decisions WHERE root_run_id=?1",
                    [&lease.root_run_id],
                    |r| r.get(0),
                )
                .map_err(|_| "source_review_projection_candidate_inventory_failed")?;
            if count != 0 {
                return Err("source_review_projection_unplanned_candidate_decisions".into());
            }
            None
        }
        _ => return Err("source_review_projection_unqualified".into()),
    };
    Ok(SourceReviewProjection {
        candidates,
        coverage,
        preparation,
    })
}
