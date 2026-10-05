//! Shared evidence graph contract (§4.5, §5.3–§5.4).
//!
//! Facts, inferences and conclusions are three different node kinds and nothing in
//! this module turns one into another: promotion needs a review decision, which is
//! written elsewhere by the reviewer role only.

// The live scheduler, child runs and review gate consume this contract. Some
// repository APIs remain acceptance-test-only, so the reachability warning is expected.
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
        Self::try_parse(value).unwrap_or(Self::Target)
    }

    /// Unknown node kinds must not silently read as `target` (§3.2).
    pub fn try_parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "target" => Some(Self::Target),
            "identity" => Some(Self::Identity),
            "page_state" => Some(Self::PageState),
            "endpoint" => Some(Self::Endpoint),
            "request_record" => Some(Self::RequestRecord),
            "response_shape" => Some(Self::ResponseShape),
            "business_object" => Some(Self::BusinessObject),
            "hypothesis" => Some(Self::Hypothesis),
            "contract" => Some(Self::Contract),
            "tool_invocation" => Some(Self::ToolInvocation),
            "candidate_finding" => Some(Self::CandidateFinding),
            "finding" => Some(Self::Finding),
            _ => None,
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
        Self::try_parse(value).unwrap_or(Self::Inferred)
    }

    /// Never upgrade: an unreadable provenance is not `observed`, because observed is
    /// the strongest claim the graph can make (§3.2). An unknown word reads as the
    /// weakest provenance the caller still has to review.
    pub fn try_parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "observed" => Some(Self::Observed),
            "source_derived" => Some(Self::SourceDerived),
            "inferred" => Some(Self::Inferred),
            _ => None,
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
        Self::try_parse(value).unwrap_or(Self::DerivedFrom)
    }

    /// Unknown edge kinds are corruption, not a `derived_from` relation (§3.2).
    pub fn try_parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "derived_from" => Some(Self::DerivedFrom),
            "observed_by" => Some(Self::ObservedBy),
            "supports" => Some(Self::Supports),
            "contradicts" => Some(Self::Contradicts),
            "targets" => Some(Self::Targets),
            "uses_identity" => Some(Self::UsesIdentity),
            "controls" => Some(Self::Controls),
            "tests" => Some(Self::Tests),
            "supersedes" => Some(Self::Supersedes),
            _ => None,
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
