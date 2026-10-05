// The investigation graph and the validation/hypothesis records that hang off it.
// Included from models.rs.

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationValidationInput {
    pub scan_id: String,
    pub target_url: String,
    pub opportunity_id: Option<i64>,
    pub hypothesis_id: Option<i64>,
    pub api_key: Option<String>,
    pub identity_id: Option<String>,
    pub method: String,
    pub request_url: String,
    #[serde(default)]
    pub request_headers: Value,
    #[serde(default)]
    pub request_body: String,
    #[serde(default)]
    pub response_status: i64,
    #[serde(default)]
    pub response_status_text: String,
    #[serde(default)]
    pub response_headers: Value,
    #[serde(default)]
    pub response_body: String,
    #[serde(default)]
    pub decoded_body: String,
    pub verdict: String,
    #[serde(default)]
    pub severity: String,
    #[serde(default)]
    pub confidence: String,
    #[serde(default)]
    pub ai_assessment: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub next_action: String,
    #[serde(default)]
    pub evidence_refs: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationValidation {
    pub id: i64,
    pub scan_id: String,
    pub target_url: String,
    pub opportunity_id: Option<i64>,
    pub hypothesis_id: Option<i64>,
    pub api_key: String,
    pub identity_id: String,
    pub method: String,
    pub request_url: String,
    pub request_headers: Value,
    pub request_body: String,
    pub response_status: i64,
    pub response_status_text: String,
    pub response_headers: Value,
    pub response_body: String,
    pub decoded_body: String,
    pub verdict: String,
    pub severity: String,
    pub confidence: String,
    pub ai_assessment: String,
    pub note: String,
    pub next_action: String,
    pub evidence_refs: Value,
    pub created_at: String,
    pub updated_at: String,
}

fn default_validation_key() -> String {
    "url-summary".to_string()
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationNode {
    pub id: i64,
    pub scan_id: String,
    pub target_url: String,
    pub node_key: String,
    pub node_type: String,
    pub label: String,
    pub confidence: String,
    pub value_score: i64,
    pub status: String,
    pub payload: Value,
    pub first_seen: String,
    pub last_seen: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationEdge {
    pub id: i64,
    pub scan_id: String,
    pub target_url: String,
    pub source_key: String,
    pub relation: String,
    pub target_key: String,
    pub confidence: String,
    pub evidence: Value,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationAction {
    pub id: i64,
    pub scan_id: String,
    pub target_url: String,
    pub action_key: String,
    pub state_key: String,
    pub action_type: String,
    pub label: String,
    pub outcome: String,
    pub value_score: i64,
    pub protocol: Value,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationApiModel {
    pub id: i64,
    pub scan_id: String,
    pub target_url: String,
    pub api_key: String,
    pub method: String,
    pub url: String,
    pub normalized_path: String,
    pub source: String,
    pub confidence: String,
    pub auth_scope: String,
    pub parameters: Value,
    pub request_schema: Value,
    pub response_schema: Value,
    pub state_keys: Value,
    pub action_keys: Value,
    pub identity_keys: Value,
    pub observed_count: i64,
    pub baseline_status: String,
    pub payload: Value,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationHypothesis {
    pub id: i64,
    pub project_id: Option<i64>,
    pub scan_id: String,
    pub target_url: String,
    pub hypothesis_key: String,
    pub category: String,
    pub title: String,
    pub status: String,
    pub score: i64,
    pub confidence: String,
    pub contract: Value,
    pub evidence: Value,
    pub decision: Value,
    pub mutation_approval: Value,
    pub source_opportunity_key: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationIdentityDiff {
    pub id: i64,
    pub scan_id: String,
    pub target_url: String,
    pub api_key: String,
    pub left_identity_key: String,
    pub right_identity_key: String,
    pub difference_type: String,
    pub risk_score: i64,
    pub status: String,
    pub matrix: Value,
    pub created_at: String,
}

#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationMetrics {
    pub scan_id: String,
    pub target_url: String,
    pub node_count: i64,
    pub edge_count: i64,
    pub state_count: i64,
    pub action_count: i64,
    pub api_count: i64,
    pub parameter_count: i64,
    pub hypothesis_count: i64,
    pub added_count: i64,
    pub changed_count: i64,
    pub removed_count: i64,
    pub duplicate_count: i64,
    pub information_gain: i64,
    pub token_worthy: bool,
    pub stop_reason: String,
    pub decision: Value,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationGraph {
    pub scan_id: String,
    pub target_url: String,
    pub nodes: Vec<InvestigationNode>,
    pub edges: Vec<InvestigationEdge>,
    pub actions: Vec<InvestigationAction>,
    pub apis: Vec<InvestigationApiModel>,
    pub related_services: Vec<Value>,
    pub hypotheses: Vec<InvestigationHypothesis>,
    pub identity_diffs: Vec<InvestigationIdentityDiff>,
    pub metrics: Option<InvestigationMetrics>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationHypothesisUpdateInput {
    pub hypothesis_id: i64,
    pub status: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationMutationApprovalInput {
    pub hypothesis_id: i64,
    pub approved: bool,
    pub max_attempts: Option<i64>,
    pub expires_minutes: Option<i64>,
    pub note: Option<String>,
}

#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationOverview {
    pub target_count: i64,
    pub node_count: i64,
    pub edge_count: i64,
    pub api_count: i64,
    pub parameter_count: i64,
    pub hypothesis_count: i64,
    pub ready_hypothesis_count: i64,
    pub identity_diff_count: i64,
    pub token_worthy_count: i64,
    pub average_information_gain: i64,
    pub fact_count: i64,
    pub promoted_strategy_count: i64,
}
