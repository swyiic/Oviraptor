//! One category per mutation. No legacy profile writer or implicit adoption.
use crate::configuration_policies::PolicyReference;
use crate::{configuration_catalog as catalog, db, AppState};
use tauri::State;

fn write_error(error: String) -> String {
    if error == "configuration_category_commit_unconfirmed_retry_same_request" {
        error
    } else {
        format!("configuration_category_rejected:{error}")
    }
}

#[tauri::command]
pub fn get_configuration_category(
    state: State<'_, AppState>,
    section: String,
    reference: Option<PolicyReference>,
) -> Result<catalog::CategoryView, String> {
    catalog::read_category(&db::open(&state.db_path)?, &section, reference.as_ref())
}
#[tauri::command]
pub fn save_configuration_category(
    state: State<'_, AppState>,
    input: catalog::SaveCategoryInput,
) -> Result<catalog::CategoryWriteResult, String> {
    catalog::save(&mut db::open(&state.db_path)?, input).map_err(write_error)
}
#[tauri::command]
pub fn set_configuration_category_default(
    state: State<'_, AppState>,
    input: catalog::SelectCategoryInput,
) -> Result<catalog::CategoryWriteResult, String> {
    catalog::select(&mut db::open(&state.db_path)?, input).map_err(write_error)
}
#[tauri::command]
pub fn get_configuration_category_receipt(
    state: State<'_, AppState>,
    request_id: String,
) -> Result<Option<catalog::CategoryWriteResult>, String> {
    catalog::read_receipt(&db::open(&state.db_path)?, &request_id)
}
