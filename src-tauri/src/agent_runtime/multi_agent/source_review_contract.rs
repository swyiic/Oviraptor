//! Exact source-candidate review admission. This is a data contract, not proof
//! of a model call, a review-lane lease, canonical publication or CI eligibility.
//! The eventual delivery transaction must establish those independent facts.
use crate::{agent_runtime::secrets::redact_json, artifact_import::canonical::sha256_hex};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum CandidateIdentity {
    AnalyzerRevision {
        #[serde(rename = "logicalKey")]
        logical_key: String,
        #[serde(rename = "revisionHash")]
        revision_hash: String,
        #[serde(rename = "revisionId")]
        revision_id: i64,
    },
    GraphCandidate {
        #[serde(rename = "nodeId")]
        node_id: String,
        revision: i64,
    },
}

impl CandidateIdentity {
    fn valid(&self) -> bool {
        match self {
            Self::AnalyzerRevision {
                logical_key,
                revision_hash,
                revision_id,
            } => {
                // Imported revisions retain CanonicalRecord's namespaced hash;
                // envelope/material digests use bare hexadecimal separately.
                !logical_key.trim().is_empty()
                    && revision_hash.strip_prefix("sha256:").is_some_and(is_sha256)
                    && *revision_id > 0
            }
            Self::GraphCandidate { node_id, revision } => {
                !node_id.trim().is_empty() && *revision > 0
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct CandidateRequirement {
    identity: CandidateIdentity,
    candidate_digest: String,
    /// The submitted suspicion itself is never independent supporting evidence.
    evidence_refs: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceReviewContract {
    schema_version: u32,
    subject: &'static str,
    root_run_id: String,
    material_digest: String,
    candidates: Vec<CandidateRequirement>,
    #[serde(skip)]
    authors: BTreeSet<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SourceVerdict {
    Confirmed,
    Rejected,
    InsufficientEvidence,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CandidateDecision {
    pub(crate) identity: CandidateIdentity,
    pub(crate) candidate_digest: String,
    pub(crate) verdict: SourceVerdict,
    pub(crate) rationale: String,
    pub(crate) reason_codes: Vec<String>,
    pub(crate) evidence_refs: Vec<String>,
    pub(crate) counter_evidence_refs: Vec<String>,
    pub(crate) missing_evidence: Vec<String>,
    pub(crate) confidence: f64,
    pub(crate) severity: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SourceReviewResponse {
    schema_version: u32,
    subject: String,
    material_digest: String,
    decisions: Vec<CandidateDecision>,
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

impl SourceReviewContract {
    /// `material` comes from the receipt audit, not from a model. The caller is
    /// still responsible for proving the four completed source phases/ACKs.
    /// No candidates means no candidate Reviewer job, not an empty clean bill.
    pub(crate) fn from_material(
        material: &Value,
        expected_root: &str,
    ) -> Result<Option<Self>, String> {
        if material["schemaVersion"] != 1
            || material["surface"] != "source"
            || material["rootRunId"] != expected_root
            || expected_root.trim().is_empty()
            || material["independentReviewCompleted"] != false
            || material["targetRequestsGranted"] != 0
            || material["hostActionsGranted"] != 0
            || redact_json(material) != *material
        {
            return Err("source_review_contract_material_invalid".into());
        }
        let candidates = material["candidates"]
            .as_array()
            .ok_or("source_review_contract_candidates_invalid")?;
        let tools = material["toolEvidence"]
            .as_array()
            .ok_or("source_review_contract_tools_invalid")?;
        let mut authors = BTreeSet::new();
        let mut tool_refs = BTreeMap::new();
        for tool in tools {
            let author = tool["authorRunId"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .ok_or("source_review_contract_author_invalid")?;
            if tool["rootRunId"] != expected_root {
                return Err("source_review_contract_tool_root_invalid".into());
            }
            authors.insert(author.to_string());
            // Keep only actual read results as supporting references. A
            // submission, inventory or finish does not prove its own claim.
            if matches!(
                tool["call"]["name"].as_str(),
                Some(
                    "repo.read_slice"
                        | "repo.search"
                        | "analyzer.get_result"
                        | "callgraph.get_slice"
                        | "dependency.get_record"
                )
            ) {
                tool_refs.insert(
                    format!("tool:{}", sha256_hex(tool.to_string().as_bytes())),
                    tool,
                );
            }
        }
        let mut identities = BTreeSet::new();
        let mut requirements = Vec::new();
        for candidate in candidates {
            let identity: CandidateIdentity = serde_json::from_value(candidate["identity"].clone())
                .map_err(|_| "source_review_contract_identity_invalid")?;
            if !identity.valid() || !identities.insert(identity.clone()) {
                return Err("source_review_contract_identity_invalid".into());
            }
            let mut evidence_refs = Vec::new();
            match &identity {
                CandidateIdentity::GraphCandidate { .. } => {
                    let author = candidate["authorRunId"]
                        .as_str()
                        .filter(|s| !s.trim().is_empty())
                        .ok_or("source_review_contract_author_invalid")?;
                    authors.insert(author.to_string());
                }
                CandidateIdentity::AnalyzerRevision { .. } => {
                    let envelope_hash = candidate["envelopeSha256"]
                        .as_str()
                        .filter(|s| is_sha256(s))
                        .ok_or("source_review_contract_envelope_invalid")?;
                    evidence_refs.push(format!("analyzer:{envelope_hash}"));
                }
            }
            // All these results belong to this exact frozen root and digest.
            // Semantic relevance still belongs to independent review, not this
            // reference-existence check. No arbitrary path/URL is a reference.
            evidence_refs.extend(tool_refs.keys().cloned());
            requirements.push(CandidateRequirement {
                identity,
                candidate_digest: sha256_hex(candidate.to_string().as_bytes()),
                evidence_refs,
            });
        }
        if requirements.is_empty() {
            return Ok(None);
        }
        requirements.sort_by(|a, b| a.identity.cmp(&b.identity));
        Ok(Some(Self {
            schema_version: 1,
            subject: "source_candidates",
            root_run_id: expected_root.into(),
            material_digest: sha256_hex(material.to_string().as_bytes()),
            candidates: requirements,
            authors,
        }))
    }

    pub(crate) fn as_json(&self) -> Value {
        json!({"requirements":self,"responseContract":{
            "schemaVersion":1,"subject":"source_candidates","materialDigest":self.material_digest,
            "decisionFields":["identity","candidateDigest","verdict","rationale","reasonCodes",
                "evidenceRefs","counterEvidenceRefs","missingEvidence","confidence","severity"],
            "verdicts":["confirmed","rejected","insufficient_evidence"],
            "severityValues":["critical","high","medium","low","informational"],
            "exactlyOneDecisionPerCandidate":true,"confidenceMinimum":0,"confidenceMaximum":1,
            "additionalFieldsAllowed":false,"toolsGranted":[],
            "receiptAndIndependentAssignmentRequired":true,
        }})
    }

    /// Bind untrusted model output to the exact server-owned material. The
    /// result is NOT a persisted canonical decision or permission to publish.
    /// It must later be admitted under the actual model receipt and review lane.
    pub(crate) fn bind_response(&self, reviewer_run_id: &str, text: &str) -> Result<Value, String> {
        if reviewer_run_id.trim().is_empty()
            || reviewer_run_id == self.root_run_id
            || self.authors.contains(reviewer_run_id)
        {
            return Err("source_review_author_not_independent".into());
        }
        if text.len() > 1_048_576 {
            return Err("source_review_response_too_large".into());
        }
        // Deserialize directly into structs: parsing a Value first would erase
        // duplicate JSON fields before the strict decoder could reject them.
        let mut response: SourceReviewResponse =
            serde_json::from_str(text).map_err(|_| "source_review_response_schema_invalid")?;
        if response.schema_version != 1
            || response.subject != self.subject
            || response.material_digest != self.material_digest
        {
            return Err("source_review_response_material_mismatch".into());
        }
        if response.decisions.len() != self.candidates.len() {
            return Err("source_review_candidate_set_mismatch".into());
        }
        let mut seen = BTreeSet::new();
        for decision in &response.decisions {
            let required = self
                .candidates
                .iter()
                .find(|c| c.identity == decision.identity)
                .ok_or("source_review_candidate_set_mismatch")?;
            if !seen.insert(&decision.identity)
                || decision.candidate_digest != required.candidate_digest
            {
                return Err("source_review_candidate_set_mismatch".into());
            }
            let strings_valid = |values: &[String], max: usize| {
                values.len() <= 64
                    && values
                        .iter()
                        .all(|s| !s.trim().is_empty() && s.len() <= max)
                    && values.iter().collect::<BTreeSet<_>>().len() == values.len()
            };
            if decision.rationale.trim().is_empty()
                || decision.rationale.len() > 8192
                || decision.reason_codes.is_empty()
                || !strings_valid(&decision.reason_codes, 256)
                || !strings_valid(&decision.missing_evidence, 2048)
                || !decision.confidence.is_finite()
                || !(0.0..=1.0).contains(&decision.confidence)
                || !matches!(
                    decision.severity.as_str(),
                    "critical" | "high" | "medium" | "low" | "informational"
                )
            {
                return Err("source_review_decision_fields_invalid".into());
            }
            for refs in [&decision.evidence_refs, &decision.counter_evidence_refs] {
                if !strings_valid(refs, 128)
                    || refs.iter().any(|r| !required.evidence_refs.contains(r))
                {
                    return Err("source_review_evidence_reference_invalid".into());
                }
            }
            if decision
                .evidence_refs
                .iter()
                .any(|r| decision.counter_evidence_refs.contains(r))
            {
                return Err("source_review_evidence_reference_contradictory".into());
            }
            match decision.verdict {
                SourceVerdict::Confirmed
                    if decision.evidence_refs.is_empty()
                        || !decision.missing_evidence.is_empty() =>
                {
                    return Err("source_review_confirmed_evidence_missing".into())
                }
                SourceVerdict::Rejected
                    if decision.evidence_refs.is_empty()
                        && decision.counter_evidence_refs.is_empty() =>
                {
                    return Err("source_review_rejection_evidence_missing".into())
                }
                SourceVerdict::InsufficientEvidence if decision.missing_evidence.is_empty() => {
                    return Err("source_review_insufficient_gap_missing".into())
                }
                _ => {}
            }
        }
        response
            .decisions
            .sort_by(|a, b| a.identity.cmp(&b.identity));
        let result =
            serde_json::to_value(response).map_err(|_| "source_review_response_encoding_failed")?;
        if redact_json(&result) != result {
            return Err("source_review_response_contains_secrets".into());
        }
        Ok(result)
    }
}
