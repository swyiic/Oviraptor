//! Category-scoped updates preserve the other owners in the legacy JSON store.
//! No automatic credential migration, environment action or global rule write.
use crate::{db, AppState};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::Deserialize;
use serde_json::{Map, Value};
use tauri::State;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigProfileSectionInput {
    pub(crate) profile_id: i64,
    pub(crate) section: String,
    pub(crate) settings: Value,
    pub(crate) expected_settings: Value,
    pub(crate) expected_reference: Option<crate::configuration_policies::PolicyReference>,
}

#[derive(Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileMetadata {
    name: String,
    description: String,
    is_default: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigProfileMetadataInput {
    profile_id: i64,
    name: String,
    description: String,
    is_default: bool,
    expected: ProfileMetadata,
}

#[tauri::command]
pub fn save_config_profile_metadata(
    state: State<'_, AppState>,
    input: ConfigProfileMetadataInput,
) -> Result<i64, String> {
    let mut connection = db::open(&state.db_path)?;
    persist_metadata(&mut connection, input)
}

fn persist_metadata(
    connection: &mut Connection,
    input: ConfigProfileMetadataInput,
) -> Result<i64, String> {
    if input.name.trim().is_empty() {
        return Err("配置名称不能为空".into());
    }
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let current: ProfileMetadata = transaction
        .query_row(
            "SELECT name,description,is_default FROM config_profiles WHERE id=?1",
            [input.profile_id],
            |row| {
                Ok(ProfileMetadata {
                    name: row.get(0)?,
                    description: row.get(1)?,
                    is_default: row.get::<_, i64>(2)? != 0,
                })
            },
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or("配置不存在，未保存任何更改")?;
    if current != input.expected {
        return Err(
            "方案信息已被其他窗口修改，请重新打开后保存（config_metadata_conflict）".into(),
        );
    }
    if current.is_default && !input.is_default {
        return Err(
            "请将其他方案设为默认后再切换，不能直接取消当前默认方案（config_default_required）"
                .into(),
        );
    }
    if input.is_default {
        transaction
            .execute(
                "UPDATE config_profiles SET is_default=0 WHERE id<>?1",
                [input.profile_id],
            )
            .map_err(|error| error.to_string())?;
    }
    let changed = transaction.execute("UPDATE config_profiles SET name=?1,description=?2,is_default=?3,updated_at=datetime('now','localtime') WHERE id=?4", params![input.name.trim(),input.description.trim(),i64::from(input.is_default),input.profile_id]).map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err("方案信息未保存，已回滚".into());
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(input.profile_id)
}

fn section_keys(section: &str) -> Result<&'static [&'static str], String> {
    match section {
        "models" => Ok(&[
            "modelProfiles",
            "activeModelProfileId",
            "modelName",
            "modelDeployment",
            "modelApiBase",
            "modelApiKey",
            "localApiKey",
        ]),
        "assetApis" => Ok(&["fofaEmail", "fofaKey", "configPath"]),
        "network" => Ok(&[
            "proxyUrl",
            "noProxy",
            "agentProxyEnabled",
            "authorizedProxyPool",
        ]),
        "agent" => Ok(&[
            "agentLocalFullPower",
            "agentPromptAuditMode",
            "agentFrontendPacketMode",
            "agentFrontendPacketBudgetKb",
            "agentQuickScore",
            "agentStandardScore",
            "agentDeepScore",
            "agentQuickTimeout",
            "agentStandardTimeout",
            "agentDeepTimeout",
            "agentQuickTokenLimit",
            "agentStandardTokenLimit",
            "agentDeepTokenLimit",
            "agentQuickRequestLimit",
            "agentStandardRequestLimit",
            "agentDeepRequestLimit",
            "agentNoToolTurnLimit",
        ]),
        "collection" => Ok(&[
            "fofaProfile",
            "pageSize",
            "maxPages",
            "interval",
            "collectionTimeout",
            "fullHistory",
            "enableCidr24",
            "collectionCache",
        ]),
        "probe" => Ok(&[
            "runRefine",
            "runProbe",
            "includeOther",
            "includeWeak",
            "priorityRate",
            "otherRate",
            "workers",
            "probeTimeout",
            "probeTotalTimeout",
            "probeRetries",
            "maxBodyBytes",
            "allowPrivate",
            "strictTls",
            "schemeFallback",
            "perHostInterval",
        ]),
        "rules" => Ok(&[
            "gamblingKeywords",
            "pornKeywords",
            "negativeKeywords",
            "contentThreshold",
            "replaceDefaultContentRules",
        ]),
        "securityRules" => Ok(&["nativeAnalyzers"]),
        _ => Err("config_section_invalid".into()),
    }
}

fn section_view(settings: &Value, keys: &[&str]) -> Value {
    Value::Object(
        keys.iter()
            .filter_map(|key| {
                settings
                    .get(*key)
                    .map(|value| ((*key).to_string(), value.clone()))
            })
            .collect(),
    )
}

#[tauri::command]
pub fn save_config_profile_section(
    state: State<'_, AppState>,
    input: ConfigProfileSectionInput,
) -> Result<i64, String> {
    let mut connection = db::open(&state.db_path)?;
    persist_section(&mut connection, input)
}

#[tauri::command]
pub fn adopt_config_profile_policy(
    state: State<'_, AppState>,
    input: ConfigProfileSectionInput,
) -> Result<i64, String> {
    let mut connection = db::open(&state.db_path)?;
    persist_category(&mut connection, input, true)
}

pub(crate) fn persist_section(
    connection: &mut Connection,
    input: ConfigProfileSectionInput,
) -> Result<i64, String> {
    persist_category(connection, input, false)
}

fn persist_category(
    connection: &mut Connection,
    input: ConfigProfileSectionInput,
    adopt: bool,
) -> Result<i64, String> {
    let keys = section_keys(&input.section)?;
    let incoming = input
        .settings
        .as_object()
        .ok_or("config_section_settings_invalid")?;
    let expected = input
        .expected_settings
        .as_object()
        .ok_or("config_section_settings_invalid")?;
    if incoming.is_empty()
        || incoming
            .keys()
            .chain(expected.keys())
            .any(|key| !keys.contains(&key.as_str()))
    {
        return Err("config_section_field_denied".into());
    }
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let raw: String = transaction
        .query_row(
            "SELECT settings_json FROM config_profiles WHERE id=?1",
            [input.profile_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or("配置不存在，未保存任何更改")?;
    let original: Value =
        serde_json::from_str(&raw).map_err(|_| "已保存的配置无法解析，不能隐式替换")?;
    if !original.is_object() {
        return Err("config_section_settings_invalid".into());
    }
    let view = crate::configuration_policies::resolve(&transaction, input.profile_id, &original)?;
    if !crate::configuration_json::equivalent(&section_view(&view, keys), &Value::Object(expected.clone())) {
        return Err("此分类已被其他窗口修改，请重新打开后保存（config_section_conflict）".into());
    }
    let current_reference = crate::configuration_policies::reference(&view, &input.section)?;
    if adopt || current_reference.is_some() {
        let save = if ["models", "assetApis"].contains(&input.section.as_str()) {
            crate::configuration_connections::save
        } else if ["network", "securityRules"].contains(&input.section.as_str()) {
            crate::configuration_runtime_owners::save
        } else {
            crate::configuration_policies::save
        };
        save(
            &transaction,
            input.profile_id,
            &input.section,
            &input.settings,
            &input.expected_settings,
            input.expected_reference.as_ref(),
            adopt,
        )?;
        transaction
            .execute(
                "UPDATE config_profiles SET updated_at=datetime('now','localtime') WHERE id=?1",
                [input.profile_id],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        return Ok(input.profile_id);
    }
    if input.expected_reference.is_some() {
        return Err("configuration_policy_conflict".into());
    }
    let mut merged: Map<String, Value> = original.as_object().unwrap().clone();
    for (key, value) in incoming {
        merged.insert(key.clone(), value.clone());
    }
    let settings = db::settings_for_update(&original, &Value::Object(merged))?;
    let changed = transaction.execute("UPDATE config_profiles SET settings_json=?1,updated_at=datetime('now','localtime') WHERE id=?2", params![settings.to_string(), input.profile_id]).map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err("配置不存在，未保存任何更改".into());
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(input.profile_id)
}

