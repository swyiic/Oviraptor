// Projects, browser auth sessions, targets, config profiles and the job/run bookkeeping
// the shell reports with. Included from models.rs.

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub status: String,
    pub asset_count: i64,
    pub pending_count: i64,
    pub target_count: i64,
    pub asset_run_count: i64,
    pub scan_count: i64,
    pub vulnerability_count: i64,
    pub validation_count: i64,
    pub active_fuse_count: i64,
    pub last_run_at: Option<String>,
    pub last_scan_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectImpact {
    pub asset_count: i64,
    pub asset_event_count: i64,
    pub target_count: i64,
    pub asset_run_count: i64,
    pub saved_view_count: i64,
    pub sentinel_scan_count: i64,
    pub sentinel_target_count: i64,
    pub finding_count: i64,
    pub validation_count: i64,
    pub opportunity_count: i64,
    pub fuse_count: i64,
    pub appsec_vulnerability_count: i64,
    pub knowledge_count: i64,
    pub learning_candidate_count: i64,
    pub browser_auth_session_count: i64,
    pub other_record_count: i64,
    pub total_records: i64,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BrowserAuthSession {
    pub id: String,
    pub project_id: i64,
    pub owner_scan_id: String,
    pub draft_scope_id: String,
    pub name: String,
    pub entry_url: String,
    pub final_url: String,
    pub status: String,
    pub scope_hosts: Vec<String>,
    pub cookie_count: i64,
    pub header_count: i64,
    pub storage_count: i64,
    pub captured_request_count: i64,
    pub last_validated_at: String,
    pub expires_at: String,
    pub last_error: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserAuthSessionInput {
    pub id: Option<String>,
    pub project_id: i64,
    pub name: String,
    pub entry_url: String,
    #[serde(default)]
    pub draft_scope_id: String,
    #[serde(default)]
    pub scan_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInput {
    pub id: Option<i64>,
    pub name: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigProfile {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub is_default: bool,
    pub settings: Value,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigProfileInput {
    pub id: Option<i64>,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub is_default: bool,
    pub settings: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub id: i64,
    pub project_id: i64,
    pub target_type: String,
    pub value: String,
    pub enabled: bool,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetImportInput {
    pub project_id: i64,
    pub target_type: String,
    pub values: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardStats {
    pub project_count: i64,
    pub asset_count: i64,
    pub alive_count: i64,
    pub pending_count: i64,
    pub new_count: i64,
    pub changed_count: i64,
    pub blocked_count: i64,
    pub running_jobs: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobRun {
    pub id: i64,
    pub project_id: i64,
    pub profile_id: Option<i64>,
    pub project_name: String,
    pub name: String,
    pub pipeline: String,
    pub status: String,
    pub stage: String,
    pub progress: f64,
    pub processed: i64,
    pub total: i64,
    pub output_dir: String,
    pub error: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub reminder_days: i64,
    pub custom_icon: bool,
    pub deduplicated_assets: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettingsInput {
    pub reminder_days: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StaleProject {
    pub project_id: i64,
    pub project_name: String,
    pub days_since_update: Option<i64>,
    pub last_run_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InterruptedJob {
    pub run_id: i64,
    pub project_id: i64,
    pub project_name: String,
    pub profile_id: Option<i64>,
    pub name: String,
    pub pipeline: String,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupStatus {
    pub reminder_days: i64,
    pub stale_projects: Vec<StaleProject>,
    pub interrupted_jobs: Vec<InterruptedJob>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartJobInput {
    pub project_id: i64,
    pub profile_id: i64,
    pub name: String,
    #[serde(default = "default_pipeline")]
    pub pipeline: String,
}

fn default_pipeline() -> String {
    "collect".to_string()
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct JobProgressEvent {
    pub run_id: i64,
    pub status: String,
    pub stage: String,
    pub progress: f64,
    pub message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub id: i64,
    pub run_id: Option<i64>,
    pub level: String,
    pub stage: String,
    pub message: String,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetEvent {
    pub id: i64,
    pub project_id: i64,
    pub asset_id: i64,
    pub asset_key: String,
    pub company: String,
    pub host: String,
    pub event_type: String,
    pub summary: String,
    pub run_id: Option<i64>,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    pub query: AssetQuery,
    pub fields: Vec<String>,
    #[serde(default)]
    pub chinese_headers: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub path: String,
    pub rows: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub inserted: i64,
    pub updated: i64,
    pub invalid: i64,
}
