//! Overall coverage verdicts bind the receipt-backed scope index, not the
//! candidate verdict namespace. Admission alone is never proof of delivery.
use super::source_coverage::CoveragePreparation;
use crate::agent_runtime::secrets::redact_json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub(crate) struct CoverageReviewContract {
    root: String,
    digest: String,
    authors: BTreeSet<String>,
    evidence: BTreeSet<String>,
    required_gaps: BTreeSet<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CoverageResponse {
    schema_version: u32,
    subject: String,
    material_digest: String,
    coverage_sufficient: bool,
    rationale: String,
    reason_codes: Vec<String>,
    evidence_refs: Vec<String>,
    outstanding_gaps: Vec<String>,
}

impl CoverageReviewContract {
    pub(crate) fn from_preparation(preparation: &CoveragePreparation) -> Result<Self, String> {
        let value = preparation.as_json();
        let material = &value["material"];
        let text = |value: &Value| -> Result<String, String> {
            value
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .map(str::to_string)
                .ok_or_else(|| "source_coverage_contract_material_invalid".into())
        };
        let root = text(&material["rootRunId"])?;
        let digest = text(&value["digest"])?;
        let status = material["candidateReview"]["status"].as_str();
        if !matches!(status, Some("completed" | "not_applicable_no_candidates")) {
            return Err("source_coverage_candidate_review_not_ready".into());
        }
        let mut authors = BTreeSet::new();
        let mut evidence = BTreeSet::new();
        for phase in material["phaseReceipts"]
            .as_array()
            .ok_or("source_coverage_phase_refs_missing")?
        {
            authors.insert(text(&phase["runId"])?);
            evidence.insert(format!("phase:{}", text(&phase["payloadHash"])?));
        }
        for tool in material["toolEvidenceRefs"]
            .as_array()
            .ok_or("source_coverage_tool_refs_missing")?
        {
            authors.insert(text(&tool["authorRunId"])?);
            evidence.insert(format!("tool:{}", text(&tool["sha256"])?));
        }
        if status == Some("completed") {
            authors.insert(text(&material["candidateReview"]["runId"])?);
            evidence.insert(format!(
                "candidate-review:{}",
                text(&material["candidateReview"]["payloadSha256"])?
            ));
        }
        // These are phase placeholders only, not deterministic scope/tool gaps.
        // This removal is permitted only after candidate prerequisite audit.
        let required_gaps = preparation
            .gaps
            .iter()
            .filter(|gap| {
                !matches!(
                    gap.as_str(),
                    "source_coverage_review"
                        | "source_independent_review"
                        | "source_review_not_completed"
                )
            })
            .cloned()
            .collect();
        Ok(Self {
            root,
            digest,
            authors,
            evidence,
            required_gaps,
        })
    }

    pub(crate) fn as_json(&self) -> Value {
        json!({"schemaVersion":1,"subject":"source_coverage","rootRunId":self.root,
            "materialDigest":self.digest,"requiredGaps":self.required_gaps,"evidenceRefs":self.evidence,
            "responseFields":["schemaVersion","subject","materialDigest","coverageSufficient",
                "rationale","reasonCodes","evidenceRefs","outstandingGaps"],
            "rules":["Preserve every required gap; additional evidence gaps may be added.",
                "coverageSufficient requires no outstanding gaps and real referenced evidence.",
                "A selected file or analyzer run is not proof of line-level coverage.",
                "This verdict cannot grant tools, network, host access or change candidate verdicts."]})
    }

    pub(crate) fn bind_response(&self, reviewer: &str, text: &str) -> Result<Value, String> {
        if reviewer.trim().is_empty() || reviewer == self.root || self.authors.contains(reviewer) {
            return Err("source_coverage_reviewer_not_independent".into());
        }
        if text.len() > 1_048_576 {
            return Err("source_coverage_response_too_large".into());
        }
        // Direct typed decoding rejects duplicate keys before Value can erase them.
        let mut response: CoverageResponse =
            serde_json::from_str(text).map_err(|_| "source_coverage_response_schema_invalid")?;
        if response.schema_version != 1
            || response.subject != "source_coverage"
            || response.material_digest != self.digest
        {
            return Err("source_coverage_response_material_mismatch".into());
        }
        let valid = |values: &[String], max_count: usize, max_len: usize| {
            values.len() <= max_count
                && values
                    .iter()
                    .all(|s| !s.trim().is_empty() && s.len() <= max_len)
                && values.iter().collect::<BTreeSet<_>>().len() == values.len()
        };
        if response.rationale.trim().is_empty()
            || response.rationale.len() > 8192
            || response.reason_codes.is_empty()
            || !valid(&response.reason_codes, 64, 256)
            || !valid(&response.outstanding_gaps, 4096, 2048)
            || !valid(&response.evidence_refs, 256, 128)
        {
            return Err("source_coverage_response_fields_invalid".into());
        }
        let gaps = response
            .outstanding_gaps
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        if !self.required_gaps.is_subset(&gaps) {
            return Err("source_coverage_deterministic_gap_removed".into());
        }
        if response
            .evidence_refs
            .iter()
            .any(|r| !self.evidence.contains(r))
        {
            return Err("source_coverage_evidence_reference_invalid".into());
        }
        if response.coverage_sufficient {
            if !gaps.is_empty() || response.evidence_refs.is_empty() {
                return Err("source_coverage_sufficient_without_evidence".into());
            }
        } else if gaps.is_empty() {
            return Err("source_coverage_insufficient_gap_missing".into());
        }
        response.outstanding_gaps.sort();
        response.evidence_refs.sort();
        response.reason_codes.sort();
        let result = serde_json::to_value(response)
            .map_err(|_| "source_coverage_response_encoding_failed")?;
        if redact_json(&result) != result {
            return Err("source_coverage_response_contains_secrets".into());
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract(gaps: &[&str]) -> CoverageReviewContract {
        CoverageReviewContract {
            root: "root".into(),
            digest: "frozen-digest".into(),
            authors: ["mapper", "analyst", "candidate-reviewer"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            evidence: ["phase:receipt"].into_iter().map(str::to_string).collect(),
            required_gaps: gaps.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn response(gaps: &[&str]) -> Value {
        json!({"schemaVersion":1,"subject":"source_coverage","materialDigest":"frozen-digest",
            "coverageSufficient":gaps.is_empty(),"rationale":"Evidence checked against selected scope",
            "reasonCodes":["scope_checked"],"evidenceRefs":["phase:receipt"],"outstandingGaps":gaps})
    }

    #[test]
    fn source_coverage_contract_requires_independent_identity_and_real_evidence() {
        let contract = contract(&[]);
        for author in ["", " ", "root", "mapper", "analyst", "candidate-reviewer"] {
            assert_eq!(
                contract
                    .bind_response(author, &response(&[]).to_string())
                    .unwrap_err(),
                "source_coverage_reviewer_not_independent"
            );
        }
        assert_eq!(
            contract
                .bind_response("coverage-reviewer", &response(&[]).to_string())
                .unwrap()["coverageSufficient"],
            true
        );
        for refs in [
            json!([]),
            json!(["tool:fabricated"]),
            json!(["phase:receipt", "phase:receipt"]),
        ] {
            let mut reply = response(&[]);
            reply["evidenceRefs"] = refs;
            assert!(contract
                .bind_response("reviewer", &reply.to_string())
                .is_err());
        }
    }

    #[test]
    fn source_coverage_contract_preserves_required_gaps_and_separates_completion_from_sufficiency()
    {
        let bound = contract(&["missing_analyzer"]);
        assert_eq!(
            bound
                .bind_response("reviewer", &response(&[]).to_string())
                .unwrap_err(),
            "source_coverage_deterministic_gap_removed"
        );
        let valid = bound
            .bind_response(
                "reviewer",
                &response(&["missing_analyzer", "additional_gap"]).to_string(),
            )
            .unwrap();
        assert_eq!(valid["coverageSufficient"], false);
        assert_eq!(
            valid["outstandingGaps"],
            json!(["additional_gap", "missing_analyzer"])
        );
        let mut inconsistent = response(&["missing_analyzer"]);
        inconsistent["coverageSufficient"] = json!(true);
        assert!(bound
            .bind_response("reviewer", &inconsistent.to_string())
            .is_err());
        let mut empty = response(&[]);
        empty["coverageSufficient"] = json!(false);
        assert!(contract(&[])
            .bind_response("reviewer", &empty.to_string())
            .is_err());
    }

    #[test]
    fn source_coverage_contract_rejects_duplicate_unknown_and_unbound_fields() {
        let contract = contract(&[]);
        let valid = response(&[]).to_string();
        let duplicate = valid.replacen('{', "{\"coverageSufficient\":false,", 1);
        assert_eq!(
            contract.bind_response("reviewer", &duplicate).unwrap_err(),
            "source_coverage_response_schema_invalid"
        );
        for (key, value) in [
            ("extra", json!(true)),
            ("materialDigest", json!("foreign")),
            ("subject", json!("source_candidates")),
            ("schemaVersion", json!(2)),
            ("reasonCodes", json!([])),
        ] {
            let mut reply = response(&[]);
            reply[key] = value;
            assert!(
                contract
                    .bind_response("reviewer", &reply.to_string())
                    .is_err(),
                "{key}"
            );
        }
    }
}
