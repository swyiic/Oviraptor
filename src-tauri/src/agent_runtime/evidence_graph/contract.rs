//! Shared evidence graph contract (Stage 1A §4.5, §5.3–§5.4).
//!
//! Facts, inferences and conclusions are three different node kinds and nothing in
//! this module turns one into another: promotion needs a review decision, which is
//! written elsewhere by the reviewer role only.

// Stage 1A declares the contract and its storage only; the scheduler, the child
// runs and the review gate that consume them land in the next stages. Every item
// here is exercised by the Stage 1A tests, so the reachability warning is expected.
#![allow(dead_code)]
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceNodeKind {
    #[default]
    Target,
    Identity,
    PageState,
    Endpoint,
    RequestRecord,
    ResponseShape,
    BusinessObject,
    Hypothesis,
    Contract,
    ToolInvocation,
    CandidateFinding,
    Finding,
}

impl EvidenceNodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Target => "target",
            Self::Identity => "identity",
            Self::PageState => "page_state",
            Self::Endpoint => "endpoint",
            Self::RequestRecord => "request_record",
            Self::ResponseShape => "response_shape",
            Self::BusinessObject => "business_object",
            Self::Hypothesis => "hypothesis",
            Self::Contract => "contract",
            Self::ToolInvocation => "tool_invocation",
            Self::CandidateFinding => "candidate_finding",
            Self::Finding => "finding",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "identity" => Self::Identity,
            "page_state" => Self::PageState,
            "endpoint" => Self::Endpoint,
            "request_record" => Self::RequestRecord,
            "response_shape" => Self::ResponseShape,
            "business_object" => Self::BusinessObject,
            "hypothesis" => Self::Hypothesis,
            "contract" => Self::Contract,
            "tool_invocation" => Self::ToolInvocation,
            "candidate_finding" => Self::CandidateFinding,
            "finding" => Self::Finding,
            _ => Self::Target,
        }
    }
}

/// How the runtime came to know this. `inferred` is never evidence of a vulnerability.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceProvenance {
    #[default]
    Observed,
    SourceDerived,
    Inferred,
}

impl EvidenceProvenance {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Observed => "observed",
            Self::SourceDerived => "source_derived",
            Self::Inferred => "inferred",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "source_derived" => Self::SourceDerived,
            "inferred" => Self::Inferred,
            _ => Self::Observed,
        }
    }

    pub fn is_observed(self) -> bool {
        matches!(self, Self::Observed)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceEdgeKind {
    #[default]
    DerivedFrom,
    ObservedBy,
    Supports,
    Contradicts,
    Targets,
    UsesIdentity,
    Controls,
    Tests,
    Supersedes,
}

impl EvidenceEdgeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DerivedFrom => "derived_from",
            Self::ObservedBy => "observed_by",
            Self::Supports => "supports",
            Self::Contradicts => "contradicts",
            Self::Targets => "targets",
            Self::UsesIdentity => "uses_identity",
            Self::Controls => "controls",
            Self::Tests => "tests",
            Self::Supersedes => "supersedes",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "observed_by" => Self::ObservedBy,
            "supports" => Self::Supports,
            "contradicts" => Self::Contradicts,
            "targets" => Self::Targets,
            "uses_identity" => Self::UsesIdentity,
            "controls" => Self::Controls,
            "tests" => Self::Tests,
            "supersedes" => Self::Supersedes,
            _ => Self::DerivedFrom,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EvidenceNode {
    pub id: String,
    pub root_run_id: String,
    pub revision: i64,
    pub kind: EvidenceNodeKind,
    pub provenance: EvidenceProvenance,
    pub natural_key_hash: String,
    pub payload: JsonValue,
    pub artifact_refs: Vec<String>,
    pub created_by_run_id: String,
    pub supersedes_id: String,
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EvidenceEdge {
    pub id: i64,
    pub root_run_id: String,
    pub revision: i64,
    pub from_node_id: String,
    pub to_node_id: String,
    pub kind: EvidenceEdgeKind,
    pub payload: JsonValue,
    pub created_by_run_id: String,
    pub created_at: String,
}
