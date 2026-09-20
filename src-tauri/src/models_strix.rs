// Strix compatibility surface: skills, rule packs, trace and learning records, and the
// workbench input. Included from models.rs.

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StrixSkill {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub instructions: String,
    pub builtin: bool,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrixSkillInput {
    pub id: Option<i64>,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub instructions: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityRulePack {
    pub id: i64,
    pub key: String,
    pub name: String,
    pub engine: String,
    pub repository: String,
    pub reference: String,
    pub local_path: String,
    pub previous_version: String,
    pub version: String,
    pub enabled: bool,
    pub builtin: bool,
    pub status: String,
    pub last_sync_at: String,
    pub error: String,
    pub added_count: i64,
    pub modified_count: i64,
    pub deleted_count: i64,
    pub change_summary: Value,
    pub progress: i64,
    pub progress_stage: String,
    pub progress_message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityRulePackInput {
    pub key: String,
    pub name: String,
    pub engine: String,
    pub repository: String,
    #[serde(default)]
    pub reference: String,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StrixTraceToolStat {
    pub name: String,
    pub calls: i64,
    pub results: i64,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StrixTraceSummary {
    pub scan_id: String,
    pub task_name: String,
    pub project_name: String,
    pub status: String,
    pub scan_type: String,
    pub model: String,
    pub run_count: i64,
    pub agent_count: i64,
    pub message_count: i64,
    pub reasoning_count: i64,
    pub tool_call_count: i64,
    pub tool_result_count: i64,
    pub llm_requests: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_tokens: i64,
    pub total_tokens: i64,
    pub hooked_request_count: i64,
    pub exact_request_capture: bool,
    pub usage_entry_count: i64,
    pub usage_agent_count: i64,
    pub token_usage_estimated: bool,
    pub instruction_hash: String,
    pub tools: Vec<StrixTraceToolStat>,
    pub knowledge_id: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StrixTraceEvent {
    pub id: String,
    pub session_id: String,
    pub call_id: String,
    pub target_url: String,
    pub event_type: String,
    pub role: String,
    pub name: String,
    pub status: String,
    pub detail: String,
    pub detail_size: i64,
    pub detail_truncated: bool,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StrixTraceDetail {
    pub summary: StrixTraceSummary,
    pub events: Vec<StrixTraceEvent>,
    pub prompt_audit: Option<StrixPromptAudit>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StrixPromptAudit {
    pub capture_mode: String,
    pub source: String,
    pub capture_level: String,
    pub exact_model_request: bool,
    pub model: String,
    pub deployment: String,
    pub full_power: bool,
    pub recorded_at: String,
    pub instruction_sha256: String,
    pub instruction_chars: i64,
    pub instruction: Option<String>,
    pub notice: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StrixKnowledgeEntry {
    pub id: i64,
    pub scan_id: String,
    pub project_id: Option<i64>,
    pub title: String,
    pub summary: String,
    pub patterns: Value,
    pub source_hash: String,
    pub skill_id: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StrixLearningCandidate {
    pub id: i64,
    pub scan_id: String,
    pub project_id: Option<i64>,
    pub scan_type: String,
    pub title: String,
    pub summary: String,
    pub candidate: Value,
    pub status: String,
    pub target_skill_id: Option<i64>,
    pub source_hash: String,
    pub created_at: String,
    pub reviewed_at: String,
    pub updated_at: String,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrixWorkbenchInput {
    pub project_id: i64,
    pub task_name: String,
    pub scan_type: String,
    #[serde(default)]
    pub urls: Vec<String>,
    #[serde(default)]
    pub source_path: String,
    #[serde(default)]
    pub skill_ids: Vec<i64>,
    #[serde(default)]
    pub instruction: String,
    #[serde(default)]
    pub scan_mode: String,
    #[serde(default)]
    pub scope_mode: String,
    #[serde(default)]
    pub diff_base: String,
    pub max_budget_usd: Option<f64>,
    #[serde(default)]
    pub environment: String,
    #[serde(default)]
    pub auth_profile_name: String,
    #[serde(default)]
    pub auth_type: String,
    #[serde(default)]
    pub auth_header_name: String,
    #[serde(default)]
    pub auth_value: String,
    #[serde(default)]
    pub auth_session_id: String,
    #[serde(default)]
    pub auth_session_ids: Vec<String>,
    #[serde(default)]
    pub auth_session_scope_id: String,
    #[serde(default)]
    pub ci_provider: String,
    #[serde(default)]
    pub repository_url: String,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub commit_sha: String,
    #[serde(default)]
    pub build_id: String,
    #[serde(default)]
    pub max_critical: i64,
    #[serde(default = "default_max_high")]
    pub max_high: i64,
    #[serde(default)]
    pub block_release: bool,
}

fn default_max_high() -> i64 {
    5
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrixLlmTestInput {
    pub llm: String,
    pub deployment: String,
    pub api_base: String,
    #[serde(default)]
    pub api_key: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StrixLlmTestResult {
    pub ok: bool,
    pub status: String,
    pub message: String,
    pub model: String,
    pub deployment: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FofaApiTestInput {
    pub key: String,
    #[serde(default)]
    pub proxy_url: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FofaApiTestResult {
    pub ok: bool,
    pub status: String,
    pub message: String,
    pub account: String,
    pub plan: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StrixUpdateStatus {
    pub installed: bool,
    pub executable: String,
    pub current_version: String,
    pub latest_version: String,
    pub update_available: bool,
    pub checked_at: String,
    pub release_url: String,
    pub check_error: String,
}
