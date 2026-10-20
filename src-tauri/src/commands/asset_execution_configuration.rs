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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_returns_only_plan_flags_and_identifiers_without_materialized_credentials() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE config_profiles(id INTEGER PRIMARY KEY,is_default INTEGER,settings_json TEXT);").unwrap();
        let settings = json!({"fofaKey":"synthetic-private-key","fofaEmail":"fixture@example.test","proxyUrl":"http://user:password@localhost:8999","runProbe":false,"runRefine":true});
        db.execute("INSERT INTO config_profiles VALUES(1,1,?1)", [settings.to_string()]).unwrap();
        let view = read_on(&db).unwrap();
        assert_eq!(view["runProbe"], false);
        assert_eq!(view["runRefine"], true);
        assert_eq!(view["configurationSource"], "current-defaults");
        assert_eq!(view.as_object().unwrap().len(), 4);
        let text = view.to_string();
        for secret in ["synthetic-private-key", "fixture@example.test", "password", "proxyUrl", "fofaKey"] {
            assert!(!text.contains(secret));
        }
    }
}
