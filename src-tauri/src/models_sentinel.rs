// The Nest sentinel desk: scans, attempts, targets, fuse entries, checkpoints,
// validations, findings and opportunities. Included from models.rs.

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SentinelScan {
    pub id: String,
    pub project_id: Option<i64>,
    pub project_name: String,
    pub status: String,
    pub current_checkpoint: String,
    pub task_path: String,
    pub previous_scan_id: String,
    pub llm_requests: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_tokens: i64,
    pub total_tokens: i64,
    pub scan_type: String,
    pub task_name: String,
    pub source_path: String,
    pub skill_names: String,
    pub attempt_count: i64,
    pub archived_at: String,
    pub created_at: String,
    pub updated_at: String,
    pub requested_scan_mode: String,
    pub llm_model: String,
    pub llm_deployment: String,
    pub llm_full_power: bool,
    pub latest_attempt_number: i64,
    pub latest_attempt_status: String,
    pub latest_attempt_checkpoint: String,
    pub latest_attempt_stop_reason: String,
    #[serde(default)]
    pub administrative_closure_recorded: bool,
    #[serde(default)]
    pub closure_handoff_recorded: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SentinelScanAttempt {
    pub scan_id: String,
    pub attempt_number: i64,
    pub execution_mode: String,
    pub status: String,
    pub stage: String,
    pub checkpoint: String,
    pub stop_reason: String,
    pub work_dir: String,
    pub backend_plan_json: String,
    pub llm_requests: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_tokens: i64,
    pub total_tokens: i64,
    pub started_at: String,
    pub finished_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SentinelTarget {
    pub id: i64,
    pub project_id: i64,
    pub scan_id: Option<String>,
    pub company: String,
    pub url: String,
    pub status: String,
    pub value_score: i64,
    pub scan_mode: String,
    pub routing_reason: String,
    pub last_attempt_number: i64,
    pub created_at: String,
    pub updated_at: String,
    pub scan_count: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SentinelFuseEntry {
    pub id: i64,
    pub project_id: i64,
    pub asset_id: Option<i64>,
    pub company: String,
    pub url: String,
    pub source_scan_id: String,
    pub reason: String,
    pub verdict: String,
    pub note: String,
    pub evidence: String,
    pub archived: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SentinelFuseReviewInput {
    pub id: i64,
    pub verdict: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub evidence: String,
    #[serde(default)]
    pub archived: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SentinelCheckpoint {
    pub scan_id: String,
    pub url: String,
    pub stage: String,
    pub raw_json: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SentinelValidation {
    pub id: i64,
    pub scan_id: String,
    pub url: String,
    pub finding_key: String,
    pub finding_kind: String,
    pub verdict: String,
    pub severity: String,
    pub note: String,
    pub evidence: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SentinelValidationWorkItem {
    pub finding_id: i64,
    pub scan_id: String,
    pub project_id: Option<i64>,
    pub project_name: String,
    pub task_name: String,
    pub url: String,
    pub finding_key: String,
    pub finding_kind: String,
    pub title: String,
    pub original_severity: String,
    pub record_json: String,
    pub validation_id: Option<i64>,
    pub verdict: String,
    pub confirmed_severity: String,
    pub note: String,
    pub evidence: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SentinelValidationInput {
    pub scan_id: String,
    pub url: String,
    #[serde(default = "default_validation_key")]
    pub finding_key: String,
    #[serde(default)]
    pub finding_kind: String,
    pub verdict: String,
    #[serde(default)]
    pub severity: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub evidence: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SentinelFinding {
    pub id: i64,
    pub scan_id: String,
    pub target_url: String,
    pub stage: String,
    pub kind: String,
    pub record_key: String,
    pub title: String,
    pub severity: String,
    pub record_json: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SentinelOpportunity {
    pub id: i64,
    pub project_id: Option<i64>,
    pub scan_id: String,
    pub target_url: String,
    pub opportunity_key: String,
    pub category: String,
    pub title: String,
    pub score: i64,
    pub status: String,
    pub confidence: String,
    pub why: Value,
    pub evidence: Value,
    pub recommended_action: Value,
    pub source: String,
    pub record: Value,
    pub first_seen: String,
    pub last_seen: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SentinelOverviewStats {
    pub task_count: i64,
    pub url_count: i64,
    pub fingerprint_count: i64,
    pub api_count: i64,
    pub endpoint_count: i64,
    pub vulnerability_count: i64,
    pub high_risk_count: i64,
    /// Only current Native publications backed by a confirmed Reviewer decision.
    pub reviewer_confirmed_count: i64,
    pub reviewer_high_risk_count: i64,
    pub source_reviewer_confirmed_count: i64,
    pub source_review_audited_task_count: i64,
    pub source_review_unavailable_task_count: i64,
    pub source_review_unverified_task_count: i64,
    /// Historical/manual/unreviewed records, not Native Reviewer confirmations.
    pub other_vulnerability_count: i64,
    pub validated_count: i64,
    pub pending_vulnerability_count: i64,
    pub vulnerable_url_count: i64,
    pub active_fuse_count: i64,
    pub opportunity_count: i64,
    pub ready_opportunity_count: i64,
}
