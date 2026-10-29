//! User-facing toggle for storing model credentials in the OS keychain
//! (docs/KEYCHAIN_MIGRATION_PLAN.md). Persisted in app_settings and mirrored into
//! the in-memory flag the secret store reads. Default OFF.
use crate::{db, secret_store, AppState};
use tauri::State;

const SETTING_KEY: &str = "keychain_enabled";

/// Read the persisted setting (absent → off). Used at startup to seed the flag.
pub(crate) fn load_persisted(connection: &rusqlite::Connection) -> bool {
    connection
        .query_row(
            "SELECT value FROM app_settings WHERE key=?1",
            [SETTING_KEY],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .map(|value| value == "true")
        .unwrap_or(false)
}

#[tauri::command]
pub fn get_credential_keychain_enabled(state: State<AppState>) -> Result<bool, String> {
    let connection = db::open(&state.db_path)?;
    Ok(load_persisted(&connection))
}

#[tauri::command]
pub fn set_credential_keychain_enabled(
    state: State<AppState>,
    enabled: bool,
) -> Result<bool, String> {
    let connection = db::open(&state.db_path)?;
    connection
        .execute(
            "INSERT INTO app_settings(key,value) VALUES(?1,?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            rusqlite::params![SETTING_KEY, if enabled { "true" } else { "false" }],
        )
        .map_err(|error| error.to_string())?;
    secret_store::set_keychain_setting(enabled);
    Ok(enabled)
}
