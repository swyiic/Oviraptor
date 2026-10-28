//! Non-secret execution-plan preview for ordinary category-default jobs.
use crate::{db, AppState};
use rusqlite::Connection;
use serde_json::{json, Value};
use tauri::State;

pub(crate) fn read_on(connection: &Connection) -> Result<Value, String> {
    let settings = crate::model_configuration::default_settings_for(connection, &["assetApis", "collection", "probe", "rules", "network"]);
    if !settings.is_object() || settings.get("_configurationError").is_some() {
        return Err("asset_execution_configuration_unavailable".into());
    }
    let probe = settings.get("runProbe").and_then(Value::as_bool).unwrap_or(true);
    let refine = settings.get("runRefine").and_then(Value::as_bool).unwrap_or(true) || probe;
    let references = settings.get("_configurationRefs").cloned().unwrap_or(json!({}));
    // Policy references carry identifiers only; never return materialized settings.
    let mut safe = serde_json::Map::new();
    if let Some(entries) = references.as_object() {
        for (kind, reference) in entries {
            if let (Some(id), Some(revision), Some(reference_kind)) = (reference.get("policyId").and_then(Value::as_str), reference.get("revision").and_then(Value::as_i64), reference.get("kind").and_then(Value::as_str)) {
                safe.insert(kind.clone(), json!({"policyId":id,"revision":revision,"kind":reference_kind}));
            }
        }
    }
    Ok(json!({"configurationSource":"current-defaults","runProbe":probe,"runRefine":refine,"policyReferences":safe}))
}

#[tauri::command]
pub fn current_asset_execution_plan(state: State<'_, AppState>) -> Result<Value, String> {
    read_on(&db::open(&state.db_path)?)
}

