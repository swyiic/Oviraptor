use crate::agent_runtime::role_config::{
    self, CapabilityBundle, RoleConfigDraft, DRAFT_STATUS_LABEL,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveRoleConfigDraftInput {
    pub id: String,
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    pub role: String,
    #[serde(default)]
    pub objective: String,
    pub capability_bundle_ids: Vec<String>,
}

#[tauri::command]
pub fn list_capability_bundles() -> Result<Vec<CapabilityBundle>, String> {
    role_config::load_capability_directory(&role_config::default_capability_directory())
}

#[tauri::command]
pub fn list_role_config_drafts(state: State<'_, AppState>) -> Result<Vec<RoleConfigDraft>, String> {
    let connection = db::open(&state.db_path)?;
    role_config::list_role_config_drafts(&connection)
}

#[tauri::command]
pub fn save_role_config_draft(
    state: State<'_, AppState>,
    input: SaveRoleConfigDraftInput,
) -> Result<RoleConfigDraft, String> {
    let role = role_config::parse_draft_role(&input.role)?;
    let connection = db::open(&state.db_path)?;
    let mut draft = RoleConfigDraft::new(
        input.id,
        input.display_name,
        role,
        input.objective,
        input.capability_bundle_ids,
    );
    draft.description = input.description;
    draft.status_label = DRAFT_STATUS_LABEL.to_string();
    role_config::save_role_config_draft(
        &connection,
        &draft,
        &role_config::default_capability_directory(),
    )
}

#[tauri::command]
pub fn role_config_draft_status_label() -> String {
    DRAFT_STATUS_LABEL.to_string()
}
