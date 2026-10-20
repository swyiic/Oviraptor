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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[cfg(target_os = "macos")]
    #[test]
    fn shipped_default_probe_survives_javascript_number_roundtrip() {
        use std::io::Write;
        // Use the shipped default profile, including its original 20.0/10.0.
        let source=include_str!("../db_initialize_budgets.rs");
        let body=source.split("let defaults = json!(").nth(1).unwrap()
            .split("\n        });").next().unwrap();
        let raw:Value=serde_json::from_str(&format!("{body}}}")).unwrap();
        assert!(raw["priorityRate"].is_f64());assert!(raw["otherRate"].is_f64());
        let mut node=std::process::Command::new("/opt/homebrew/bin/node")
            .args(["--input-type=module","-e","let s='';for await(const c of process.stdin)s+=c;process.stdout.write(JSON.stringify(JSON.parse(s)))"])
            .env_clear().stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().unwrap();
        node.stdin.take().unwrap().write_all(raw.to_string().as_bytes()).unwrap();
        let output=node.wait_with_output().unwrap();assert!(output.status.success());
        let wire:Value=serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(wire["priorityRate"].as_i64(),Some(20));
        let mut connection=fixture();
        connection.execute("UPDATE config_profiles SET settings_json=?1 WHERE id=1",[raw.to_string()]).unwrap();
        let expected=section_view(&wire,section_keys("probe").unwrap());
        let mut incoming=expected.clone();incoming["workers"]=json!(4);
        persist_category(&mut connection,input("probe",incoming,expected),true).unwrap();
        let current=crate::configuration_policies::read_profile(&connection,1).unwrap();
        assert_eq!(current["workers"],4);
        let mut stale=section_view(&current,section_keys("probe").unwrap());stale["priorityRate"]=json!(21);
        let mut bad=input("probe",section_view(&current,section_keys("probe").unwrap()),stale);
        bad.expected_reference=crate::configuration_policies::reference(&current,"probe").unwrap();
        assert!(persist_section(&mut connection,bad).unwrap_err().contains("config_section_conflict"));
    }

    #[test]
    fn runtime_category_adoption_reaches_real_network_and_analyzer_consumers() {
        let mut connection=fixture();
        let network=json!({"proxyUrl":"http://synthetic-user:synthetic-password@127.0.0.1:8181","noProxy":"localhost,.fixture.invalid","agentProxyEnabled":true,"authorizedProxyPool":["eu|socks5h://pool-user:pool-password@[::1]:1080"]});
        let image=format!("fixture.invalid/analyzer@sha256:{}","a".repeat(64));
        let analyzers=json!({"nativeAnalyzers":{"semgrep":{"image":image},"codeql":{"image":""}}});
        persist_category(&mut connection,input("network",network.clone(),json!({})),true).unwrap();
        persist_category(&mut connection,input("securityRules",analyzers,json!({})),true).unwrap();
        let effective=crate::model_configuration::default_settings(&connection);
        let proxies=crate::scanning::ownership_test_support::approved_agent_proxies(&effective);
        assert_eq!(proxies,vec![("EU".into(),"socks5h://pool-user:pool-password@[::1]:1080".into())]);
        assert_eq!(effective["proxyUrl"],network["proxyUrl"]);
        assert_eq!(effective["noProxy"],network["noProxy"]);
        let (program,selected)=crate::scanning::ownership_test_support::configured_native_analyzer_runtime(&effective,"semgrep",|name|{assert_eq!(name,"docker");Some(std::path::PathBuf::from("synthetic-docker-not-executed"))}).unwrap();
        assert_eq!(program,std::path::PathBuf::from("synthetic-docker-not-executed"));assert_eq!(selected,Some(image));
        assert!(crate::scanning::ownership_test_support::configured_native_analyzer_runtime(&effective,"codeql",|_|panic!("empty image must not locate any program")).is_err());
        let raw=read(&connection);assert!(raw.get("proxyUrl").is_none());assert!(raw.get("nativeAnalyzers").is_none());
        for section in ["network","securityRules"] {assert_eq!(crate::configuration_policies::reference(&effective,section).unwrap().unwrap().revision,1);}
    }

    fn fixture() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE config_profiles(id INTEGER PRIMARY KEY,name TEXT,description TEXT,is_default INTEGER,settings_json TEXT,updated_at TEXT); INSERT INTO config_profiles VALUES(1,'kept','description',1,'{}','before');").unwrap();
        let settings = json!({"modelProfiles":[],"fofaKey":"synthetic-key","workers":6,"pageSize":100,"historicalCustom":{"untouched":true},"pythonExecutable":"retained-inert"});
        connection
            .execute(
                "UPDATE config_profiles SET settings_json=?1",
                [settings.to_string()],
            )
            .unwrap();
        connection
    }
    fn input(
        section: &str,
        settings: Value,
        expected_settings: Value,
    ) -> ConfigProfileSectionInput {
        ConfigProfileSectionInput {
            profile_id: 1,
            section: section.into(),
            settings,
            expected_settings,
            expected_reference: None,
        }
    }
    fn read(connection: &Connection) -> Value {
        let raw: String = connection
            .query_row("SELECT settings_json FROM config_profiles", [], |row| {
                row.get(0)
            })
            .unwrap();
        serde_json::from_str(&raw).unwrap()
    }
    #[test]
    fn configuration_sections_preserve_other_categories_and_historical_fields() {
        let mut connection = fixture();
        persist_section(
            &mut connection,
            input("probe", json!({"workers":12}), json!({"workers":6})),
        )
        .unwrap();
        persist_section(
            &mut connection,
            input(
                "collection",
                json!({"pageSize":200}),
                json!({"pageSize":100}),
            ),
        )
        .unwrap();
        let saved = read(&connection);
        assert_eq!(saved["workers"], 12);
        assert_eq!(saved["pageSize"], 200);
        assert_eq!(saved["fofaKey"], "synthetic-key");
        assert_eq!(saved["historicalCustom"]["untouched"], true);
        assert_eq!(saved["pythonExecutable"], "retained-inert");
        let metadata: (String, i64) = connection
            .query_row("SELECT name,is_default FROM config_profiles", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(metadata, ("kept".into(), 1));
    }
    #[test]
    fn configuration_sections_reject_stale_same_category_and_foreign_keys_atomically() {
        let mut connection = fixture();
        persist_section(
            &mut connection,
            input("probe", json!({"workers":12}), json!({"workers":6})),
        )
        .unwrap();
        let before = read(&connection);
        assert!(persist_section(
            &mut connection,
            input("probe", json!({"workers":20}), json!({"workers":6}))
        )
        .unwrap_err()
        .contains("config_section_conflict"));
        assert!(persist_section(
            &mut connection,
            input(
                "probe",
                json!({"fofaKey":"forbidden"}),
                json!({"workers":12})
            )
        )
        .is_err());
        assert_eq!(read(&connection), before);
        assert!(persist_section(
            &mut connection,
            input(
                "runtime",
                json!({"windowsRuntimeDirectory":"ignored"}),
                json!({})
            )
        )
        .is_err());
    }
    #[test]
    fn configuration_sections_metadata_cannot_overwrite_settings_and_conflicts_roll_back() {
        let mut connection = fixture();
        let before = read(&connection);
        let input = || ConfigProfileMetadataInput {
            profile_id: 1,
            name: "renamed".into(),
            description: "new description".into(),
            is_default: true,
            expected: ProfileMetadata {
                name: "kept".into(),
                description: "description".into(),
                is_default: true,
            },
        };
        persist_metadata(&mut connection, input()).unwrap();
        assert_eq!(read(&connection), before);
        assert!(persist_metadata(&mut connection, input())
            .unwrap_err()
            .contains("config_metadata_conflict"));
        let mut missing = input();
        missing.profile_id = 2;
        assert!(persist_metadata(&mut connection, missing).is_err());
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM config_profiles WHERE is_default=1",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
    }
    #[test]
    fn configuration_sections_default_switch_requires_a_replacement() {
        let mut connection = fixture();
        let before = read(&connection);
        let unset = ConfigProfileMetadataInput {
            profile_id: 1,
            name: "kept".into(),
            description: "description".into(),
            is_default: false,
            expected: ProfileMetadata {
                name: "kept".into(),
                description: "description".into(),
                is_default: true,
            },
        };
        assert!(persist_metadata(&mut connection, unset)
            .unwrap_err()
            .contains("config_default_required"));
        assert_eq!(read(&connection), before);
        connection
            .execute(
                "INSERT INTO config_profiles VALUES(2,'replacement','',0,'{}','before')",
                [],
            )
            .unwrap();
        let replacement = ConfigProfileMetadataInput {
            profile_id: 2,
            name: "replacement".into(),
            description: "".into(),
            is_default: true,
            expected: ProfileMetadata {
                name: "replacement".into(),
                description: "".into(),
                is_default: false,
            },
        };
        persist_metadata(&mut connection, replacement).unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT id FROM config_profiles WHERE is_default=1",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            2
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT settings_json FROM config_profiles WHERE id=1",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            before.to_string()
        );
    }
    #[test]
    fn configuration_sections_adoption_and_revisioned_save_do_not_write_legacy_json() {
        let mut connection = fixture();
        let before = read(&connection);
        persist_category(
            &mut connection,
            input(
                "collection",
                json!({"pageSize":200}),
                json!({"pageSize":100}),
            ),
            true,
        )
        .unwrap();
        assert_eq!(read(&connection), before);
        let effective = crate::configuration_policies::read_profile(&connection, 1).unwrap();
        let source = crate::configuration_policies::reference(&effective, "collection")
            .unwrap()
            .unwrap();
        let mut update = input(
            "collection",
            json!({"pageSize":300}),
            section_view(&effective, section_keys("collection").unwrap()),
        );
        update.expected_reference = Some(source.clone());
        persist_section(&mut connection, update).unwrap();
        assert_eq!(read(&connection), before);
        let effective = crate::model_configuration::default_settings(&connection);
        assert_eq!(effective["pageSize"], 300);
        assert_eq!(
            crate::configuration_policies::reference(&effective, "collection")
                .unwrap()
                .unwrap()
                .revision,
            2
        );
        let missing_reference = input(
            "collection",
            json!({"pageSize":400}),
            section_view(&crate::configuration_policies::read_profile(&connection, 1).unwrap(), section_keys("collection").unwrap()),
        );
        assert!(persist_section(&mut connection, missing_reference)
            .unwrap_err()
            .contains("configuration_policy_conflict"));
        assert_eq!(
            crate::configuration_policies::read_profile(&connection, 1).unwrap()["pageSize"],
            300
        );
    }
}
