use rusqlite::{params, Connection, OpenFlags};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

const DEFAULT_BUSINESS_FRONTEND_SKILL: &str =
    include_str!("../resources/skills/business_frontend_deep_analysis.md");

pub fn open(path: &Path) -> Result<Connection, String> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .map_err(|error| error.to_string())?;
    connection
        .busy_timeout(Duration::from_secs(10))
        .map_err(|error| error.to_string())?;
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(|error| error.to_string())?;
    // journal_mode is persistent and is enabled once during initialize().
    // Re-applying it for every UI command asks SQLite for a write lock. While a
    // collector is writing this made otherwise read-only page changes wait for
    // the full busy timeout and looked like the application had frozen.
    connection
        .pragma_update(None, "synchronous", "NORMAL")
        .map_err(|error| error.to_string())?;
    Ok(connection)
}

/// §8.1: one-time promotion of the Strix-named model settings to neutral keys.
///
/// `modelProfiles` / `activeModelProfileId` plus the flat `modelDeployment`,
/// `modelApiBase`, `modelApiKey` and `localApiKey` become the only written shape.
/// The `strixLlm*` keys are read here as the migration source and left in place so
/// an older build can still open the same profile; nothing writes them again.
fn migrate_neutral_model_settings(connection: &rusqlite::Connection) {
    let rows: Vec<(i64, String)> = connection
        .prepare(
            "SELECT id, settings_json FROM config_profiles WHERE json_valid(settings_json) AND json_type(settings_json,'$.modelProfiles') IS NULL",
        )
        .and_then(|mut statement| {
            statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .map(|rows| rows.filter_map(Result::ok).collect::<Vec<_>>())
        })
        .unwrap_or_default();
    for (id, raw) in rows {
        let Ok(mut settings) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        let mut profiles = settings
            .get("strixLlmProfiles")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        if profiles.is_empty() {
            // The pre-profile shape: one flat model, key and base URL.
            let text = |key: &str| {
                settings
                    .get(key)
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string()
            };
            if !text("strixLlm").is_empty() || !text("strixApiBase").is_empty() {
                profiles.push(serde_json::json!({
                    "id": "legacy-default",
                    "name": "默认模型",
                    "llm": text("strixLlm"),
                    "apiBase": text("strixApiBase"),
                    "apiKey": text("strixApiKey"),
                    "localApiKey": "",
                    "deployment": "cloud",
                }));
            }
        }
        let text = |key: &str| {
            settings
                .get(key)
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string)
        };
        let active_id = text("strixActiveLlmProfileId").or_else(|| {
            profiles
                .first()
                .and_then(|profile| profile.get("id"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        });
        let active_id = active_id.unwrap_or_default();
        let active = profiles
            .iter()
            .find(|profile| {
                profile.get("id").and_then(serde_json::Value::as_str) == Some(active_id.as_str())
            })
            .or(profiles.first())
            .cloned();
        let field = |key: &str| {
            active
                .as_ref()
                .and_then(|profile| profile.get(key))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string()
        };
        let deployment = if field("deployment") == "local" {
            "local"
        } else {
            "cloud"
        };
        let updated = serde_json::json!({
            "modelProfiles": profiles,
            "activeModelProfileId": active_id,
            "modelDeployment": deployment,
            "modelApiBase": field("apiBase"),
            "modelApiKey": field("apiKey"),
            "localApiKey": field("localApiKey"),
        });
        let Some(object) = settings.as_object_mut() else {
            continue;
        };
        if let Some(source) = updated.as_object() {
            for (key, value) in source {
                object.insert(key.clone(), value.clone());
            }
        }
        let _ = connection.execute(
            "UPDATE config_profiles SET settings_json=?1 WHERE id=?2",
            rusqlite::params![settings.to_string(), id],
        );
    }
}

pub fn initialize(app_data_dir: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(app_data_dir).map_err(|error| error.to_string())?;
    let path = app_data_dir.join("oviraptor.sqlite3");
    let mut connection = open(&path)?;
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    connection
        .execute_batch(SCHEMA)
        .map_err(|error| error.to_string())?;
    migrate_legacy_agent_and_retry_columns(&mut connection)?;
    verify_locked_platform_schema(&mut connection)?;
    migrate_agent_run_orchestration_columns(&mut connection)?;
    migrate_fuse_entry_status(&mut connection)?;
    migrate_builtin_prompts_and_recon_routes(&mut connection)?;
    migrate_targets_and_opportunities(&mut connection)?;
    reconcile_investigation_source_keys(&mut connection)?;
    repair_asset_duplicates_and_probe_labels(&mut connection)?;
    migrate_budget_defaults(&mut connection)?;
    Ok(path)
}

// §12: the migration helpers, the schema text and the tests live in their own
// files. These are textual includes of the same module, so nothing had to be made
// public or duplicated.
include!("db_migrate.rs");
include!("db_schema.rs");
include!("db_initialize.rs");

#[cfg(test)]
mod tests {
    // §12: the migration tests live in their own file; this is a textual include, so
    // they still reach the private helpers of `db` through `super::*`.
    include!("db_tests.rs");
}
