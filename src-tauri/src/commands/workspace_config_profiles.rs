//! Configuration persistence. Preserve stored settings using the database policy;
//! profile storage does not resolve credentials or start business execution.
use crate::{db, models::{ConfigProfile, ConfigProfileInput}, AppState};
use rusqlite::{params, OptionalExtension};
use serde_json::Value as JsonValue;
use tauri::State;
#[path = "workspace_config_profile_sections.rs"]
pub(crate) mod configuration_sections;
#[path = "workspace_config_categories.rs"]
pub(crate) mod categories;
#[tauri::command]
pub fn list_config_profiles(state: State<AppState>) -> Result<Vec<ConfigProfile>, String> {
    let connection = db::open(&state.db_path)?;
    read_config_profiles(&connection)
}

pub(crate) fn read_config_profiles(connection: &rusqlite::Connection) -> Result<Vec<ConfigProfile>, String> {
    if connection.is_autocommit() {
        let tx=connection.unchecked_transaction().map_err(|error|error.to_string())?;
        let result=read_config_profiles(&tx);
        tx.rollback().map_err(|error|error.to_string())?;
        return result;
    }
    let mut statement = connection.prepare(
        "SELECT id,name,description,is_default,settings_json,created_at,updated_at FROM config_profiles ORDER BY is_default DESC,id"
    ).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok(ConfigProfile {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                is_default: row.get::<_, i64>(3)? != 0,
                settings: serde_json::from_str(&row.get::<_, String>(4)?).map_err(|error| rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(error)))?,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })
        .map_err(|error| error.to_string())?;
    let mut profiles = rows.collect::<Result<Vec<_>, _>>().map_err(|error| error.to_string())?;
    for profile in &mut profiles {
        profile.settings = crate::configuration_policies::resolve(connection, profile.id, &profile.settings)?;
    }
    Ok(profiles)
}

#[tauri::command]
pub fn save_config_profile(
    state: State<'_, AppState>,
    input: ConfigProfileInput,
) -> Result<i64, String> {
    let mut connection = db::open(&state.db_path)?;
    persist_config_profile(&mut connection, input)
}

pub(crate) fn persist_config_profile(
    connection: &mut rusqlite::Connection,
    input: ConfigProfileInput,
) -> Result<i64, String> {
    if input.name.trim().is_empty() {
        return Err("配置名称不能为空".into());
    }
    if !input.settings.is_object() {
        return Err("配置 settings 必须是 JSON 对象".into());
    }
    let mut settings = db::normalize_settings(&input.settings);
    if let Some(object)=settings.as_object_mut() { object.remove("_configurationRefs"); }
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    if input.is_default {
        transaction
            .execute("UPDATE config_profiles SET is_default=0", [])
            .map_err(|error| error.to_string())?;
    }
    let id = if let Some(id) = input.id {
        let raw:String=transaction.query_row("SELECT settings_json FROM config_profiles WHERE id=?1",[id],|row|row.get(0))
            .optional().map_err(|error|error.to_string())?.ok_or("配置不存在，未保存任何更改")?;
        let stored:JsonValue=serde_json::from_str(&raw).map_err(|_|"已保存的配置无法解析，不能隐式替换")?;
        let settings=crate::configuration_policies::legacy_update(&transaction,id,&stored,&input.settings)?;
        let changed = transaction.execute(
            "UPDATE config_profiles SET name=?1,description=?2,is_default=?3,settings_json=?4,updated_at=datetime('now','localtime') WHERE id=?5",
            params![input.name.trim(), input.description.trim(), input.is_default as i64, settings.to_string(), id],
        ).map_err(|error| error.to_string())?;
        if changed != 1 {
            return Err("配置不存在，未保存任何更改".into());
        }
        id
    } else {
        transaction.execute(
            "INSERT INTO config_profiles(name,description,is_default,settings_json) VALUES(?1,?2,?3,?4)",
            params![input.name.trim(), input.description.trim(), input.is_default as i64, settings.to_string()],
        ).map_err(|error| error.to_string())?;
        transaction.last_insert_rowid()
    };
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(id)
}

#[tauri::command]
pub fn delete_config_profile(state: State<AppState>, profile_id: i64) -> Result<(), String> {
    let connection = db::open(&state.db_path)?;
    delete_config_profile_inner(&connection,profile_id)
}

pub(crate) fn delete_config_profile_inner(connection: &rusqlite::Connection,profile_id:i64)->Result<(),String> {
    let tx=rusqlite::Transaction::new_unchecked(connection,rusqlite::TransactionBehavior::Immediate)
        .map_err(|error|error.to_string())?;
    let (is_default,raw):(i64,String)=tx.query_row("SELECT is_default,settings_json FROM config_profiles WHERE id=?1",[profile_id],
        |row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(|error|error.to_string())?.ok_or("配置方案不存在")?;
    if is_default!=0 {return Err("系统默认配置不能删除".into());}
    let stored:JsonValue=serde_json::from_str(&raw).map_err(|_|"已保存的配置无法解析，不能隐式删除")?;
    let retained=db::settings_for_update(&stored,&serde_json::json!({}))?;
    if retained.as_object().is_none_or(|fields|!fields.is_empty()) {
        return Err("retired_settings_require_confirmed_cleanup".into());
    }
    crate::configuration_policies::release_profile_bindings(&tx,profile_id)?;
    let changed=tx.execute("DELETE FROM config_profiles WHERE id=?1",[profile_id]).map_err(|error|error.to_string())?;
    let remains:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM config_profiles WHERE id=?1)",[profile_id],|row|row.get(0)).map_err(|error|error.to_string())?;
    if changed!=1 || remains {return Err("配置删除未确认，已回滚".into());}
    tx.commit().map_err(|error|error.to_string())
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::configuration_models::{ConfigProfile, ConfigProfileInput};

    pub(crate) fn read_config_profiles(
        connection: &rusqlite::Connection,
    ) -> Result<Vec<ConfigProfile>, String> {
        super::read_config_profiles(connection)
    }

    pub(crate) fn persist_config_profile(
        connection: &mut rusqlite::Connection,
        input: ConfigProfileInput,
    ) -> Result<i64, String> {
        super::persist_config_profile(connection, input)
    }

    pub(crate) fn delete_config_profile_inner(
        connection: &rusqlite::Connection,
        profile_id: i64,
    ) -> Result<(), String> {
        super::delete_config_profile_inner(connection, profile_id)
    }
}

#[cfg(test)]
mod selection_tests {
    use super::*;

    fn fixture() -> rusqlite::Connection {
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE config_profiles(id INTEGER PRIMARY KEY,name TEXT,description TEXT,is_default INTEGER,settings_json TEXT,created_at TEXT,updated_at TEXT);
            INSERT INTO config_profiles VALUES(9,'newest','',0,'{\"fixture\":9}','2000-01-01','2099-01-01'),(3,'oldest id','',0,'{\"fixture\":3}','2000-01-01','2000-01-01'),(7,'middle','',0,'{\"fixture\":7}','2000-01-01','2050-01-01');").unwrap();
        connection
    }

    #[test]
    fn configuration_profile_no_default_fallback_matches_model_resolver() {
        let connection = fixture();
        let profiles = read_config_profiles(&connection).unwrap();
        assert_eq!(profiles.iter().map(|profile| profile.id).collect::<Vec<_>>(), vec![3, 7, 9]);
        // Category defaults add builtins and provenance to the legacy snapshot.
        // The selected fallback identity must agree, not the entire JSON shape.
        let defaults = crate::model_configuration::default_settings(&connection);
        assert_eq!(profiles[0].settings["fixture"], defaults["fixture"]);
        for kind in ["models", "assetApis", "collection", "probe", "rules", "agent", "network", "securityRules"] {
            assert_eq!(defaults["_configurationLegacyFallbacks"][kind], profiles[0].id);
        }
        // Editing a newer profile must not silently change a no-default fallback.
        connection.execute("UPDATE config_profiles SET updated_at='2199-01-01' WHERE id=9", []).unwrap();
        assert_eq!(read_config_profiles(&connection).unwrap()[0].id, 3);
    }

    #[test]
    fn configuration_profile_explicit_defaults_sort_before_fallback_by_stable_id() {
        let connection = fixture();
        connection.execute("UPDATE config_profiles SET is_default=1 WHERE id IN (7,9)", []).unwrap();
        let profiles = read_config_profiles(&connection).unwrap();
        assert_eq!(profiles.iter().map(|profile| profile.id).collect::<Vec<_>>(), vec![7, 9, 3]);
        // Category defaults add builtins and provenance to the legacy snapshot.
        // The selected fallback identity must agree, not the entire JSON shape.
        let defaults = crate::model_configuration::default_settings(&connection);
        assert_eq!(profiles[0].settings["fixture"], defaults["fixture"]);
        for kind in ["models", "assetApis", "collection", "probe", "rules", "agent", "network", "securityRules"] {
            assert_eq!(defaults["_configurationLegacyFallbacks"][kind], profiles[0].id);
        }
    }
}
