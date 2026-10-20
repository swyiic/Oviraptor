//! Versioned business policies. Legacy JSON is a read-only compatibility source
//! until an operator explicitly adopts a category; credentials never enter here.
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS configuration_policy_versions (
    policy_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision > 0),
    kind TEXT NOT NULL CHECK(kind IN ('collection','probe','rules','agent')),
    settings_json TEXT NOT NULL CHECK(json_valid(settings_json)),
    payload_digest TEXT NOT NULL,
    source_profile_id INTEGER NOT NULL,
    source_digest TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    PRIMARY KEY(policy_id,revision)
);
CREATE TABLE IF NOT EXISTS configuration_policy_bindings (
    profile_id INTEGER NOT NULL REFERENCES config_profiles(id),
    section TEXT NOT NULL CHECK(section IN ('collection','probe','rules','agent')),
    policy_id TEXT NOT NULL,
    revision INTEGER NOT NULL,
    PRIMARY KEY(profile_id,section),
    FOREIGN KEY(policy_id,revision) REFERENCES configuration_policy_versions(policy_id,revision)
);
CREATE TRIGGER IF NOT EXISTS configuration_policy_versions_immutable_update
BEFORE UPDATE ON configuration_policy_versions BEGIN SELECT RAISE(ABORT,'configuration_policy_version_immutable'); END;
CREATE TRIGGER IF NOT EXISTS configuration_policy_versions_immutable_delete
BEFORE DELETE ON configuration_policy_versions BEGIN SELECT RAISE(ABORT,'configuration_policy_version_immutable'); END;
"#;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PolicyReference {
    pub policy_id: String,
    pub revision: i64,
    pub kind: String,
}

pub fn policy_keys(section: &str) -> Result<&'static [&'static str], String> {
    match section {
        "collection" => Ok(&[
            "fofaProfile",
            "pageSize",
            "maxPages",
            "interval",
            "collectionTimeout",
            "fullHistory",
            "enableCidr24",
            "collectionCache",
            "precisionFirst",
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
        _ => Err("configuration_policy_kind_invalid".into()),
    }
}

fn tables_exist(connection: &Connection) -> Result<bool, String> {
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('configuration_policy_versions','configuration_policy_bindings')", [], |row| row.get(0),
    ).map_err(|_| "configuration_policy_storage_unavailable")?;
    match count {
        0 => Ok(false),
        2 => Ok(true),
        _ => Err("configuration_policy_storage_incomplete".into()),
    }
}

pub fn project(settings: &Value, section: &str) -> Result<Value, String> {
    Ok(Value::Object(
        policy_keys(section)?
            .iter()
            .filter_map(|key| {
                settings
                    .get(*key)
                    .map(|value| ((*key).to_string(), value.clone()))
            })
            .collect(),
    ))
}

fn validate(section: &str, settings: &Value) -> Result<(), String> {
    let keys = policy_keys(section)?;
    let object = settings
        .as_object()
        .ok_or("configuration_policy_settings_invalid")?;
    if object.is_empty() || object.keys().any(|key| !keys.contains(&key.as_str())) {
        return Err("configuration_policy_field_denied".into());
    }
    for (key, value) in object {
        let valid = match key.as_str() {
            "fullHistory"
            | "enableCidr24"
            | "collectionCache"
            | "precisionFirst"
            | "runRefine"
            | "runProbe"
            | "includeOther"
            | "includeWeak"
            | "allowPrivate"
            | "strictTls"
            | "schemeFallback"
            | "replaceDefaultContentRules"
            | "agentLocalFullPower" => value.is_boolean(),
            "gamblingKeywords" | "pornKeywords" | "negativeKeywords" => value
                .as_array()
                .is_some_and(|values| values.iter().all(Value::is_string)),
            "fofaProfile" => value.is_string(),
            "maxBodyBytes" => value.as_i64().is_some_and(|v| v >= 0),
            "agentPromptAuditMode" => value
                .as_str()
                .is_some_and(|v| ["off", "metadata", "full"].contains(&v)),
            "agentFrontendPacketMode" => value
                .as_str()
                .is_some_and(|v| ["balanced", "compact", "custom"].contains(&v)),
            _ => value
                .as_f64()
                .is_some_and(|number| number.is_finite() && number >= 0.0),
        };
        if !valid {
            return Err(format!("configuration_policy_value_invalid:{key}"));
        }
    }
    Ok(())
}

/// No DDL or data migration on read. A damaged binding never falls back to an
/// older JSON policy. The caller should hold a read/write transaction if it
/// needs a consistent snapshot across profile metadata and bindings.
pub fn resolve(connection: &Connection, profile_id: i64, raw: &Value) -> Result<Value, String> {
    let mut effective = crate::db::normalize_settings(raw);
    let object = effective
        .as_object_mut()
        .ok_or("configuration_profile_settings_invalid")?;
    // Internal references can only come from the owner tables, not legacy input.
    object.remove("_configurationRefs");
    object.remove("_configurationError");
    let mut references = Map::new();
    if tables_exist(connection)? {
        let mut statement = connection.prepare(
        "SELECT b.section,b.policy_id,b.revision,v.kind,v.settings_json,v.payload_digest FROM configuration_policy_bindings b LEFT JOIN configuration_policy_versions v ON v.policy_id=b.policy_id AND v.revision=b.revision WHERE b.profile_id=?1 ORDER BY b.section",
    ).map_err(|_| "configuration_policy_storage_unavailable")?;
        let rows = statement
            .query_map([profile_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            })
            .map_err(|_| "configuration_policy_storage_unavailable")?;
        for row in rows {
            let (section, policy_id, revision, kind, payload, digest) =
                row.map_err(|_| "configuration_policy_storage_unavailable")?;
            if kind.as_deref() != Some(section.as_str()) || revision <= 0 {
                return Err("configuration_policy_reference_invalid".into());
            }
            let payload = payload.ok_or("configuration_policy_reference_invalid")?;
            if digest.as_deref()
                != Some(format!("{:x}", Sha256::digest(payload.as_bytes())).as_str())
            {
                return Err("configuration_policy_digest_mismatch".into());
            }
            let settings: Value = serde_json::from_str(&payload)
                .map_err(|_| "configuration_policy_payload_invalid")?;
            validate(&section, &settings)?;
            for key in policy_keys(&section)? {
                // Older immutable versions predate these consumer fields.
                // Preserve their legacy value until an explicit scoped save
                // freezes it; a schema extension must not alter a saved job.
                if !extended_defaults(&section).contains_key(*key) || settings.get(*key).is_some() {
                    object.remove(*key);
                }
            }
            object.extend(settings.as_object().unwrap().clone());
            references.insert(
                section.clone(),
                serde_json::to_value(PolicyReference {
                    policy_id,
                    revision,
                    kind: section,
                })
                .map_err(|_| "configuration_policy_reference_invalid")?,
            );
        }
    }
    crate::configuration_connections::overlay(connection, profile_id, object, &mut references)?;
    crate::configuration_runtime_owners::overlay(connection, profile_id, object, &mut references)?;
    if !references.is_empty() {
        object.insert("_configurationRefs".into(), Value::Object(references));
    }
    Ok(effective)
}

pub fn read_profile(connection: &Connection, profile_id: i64) -> Result<Value, String> {
    if connection.is_autocommit() {
        let tx = connection
            .unchecked_transaction()
            .map_err(|_| "configuration_profile_read_failed")?;
        let result = read_profile(&tx, profile_id);
        tx.rollback()
            .map_err(|_| "configuration_profile_read_failed")?;
        return result;
    }
    let raw: String = connection
        .query_row(
            "SELECT settings_json FROM config_profiles WHERE id=?1",
            [profile_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| "configuration_profile_read_failed")?
        .ok_or("配置方案不存在")?;
    let settings: Value =
        serde_json::from_str(&raw).map_err(|_| "configuration_profile_settings_invalid")?;
    resolve(connection, profile_id, &settings)
}

/// Legacy full-form writers may still change non-policy fields, but they cannot
/// silently rewrite an adopted owner or copy its resolved values back to JSON.
pub fn legacy_update(
    connection: &Connection,
    profile_id: i64,
    stored: &Value,
    incoming: &Value,
) -> Result<Value, String> {
    let current = resolve(connection, profile_id, stored)?;
    let mut input = incoming
        .as_object()
        .ok_or("configuration_profile_settings_invalid")?
        .clone();
    input.remove("_configurationRefs");
    input.remove("_configurationError");
    for section in [
        "collection",
        "probe",
        "rules",
        "agent",
        "models",
        "assetApis",
        "network",
        "securityRules",
    ] {
        if reference(&current, section)?.is_none() {
            continue;
        }
        let connection_owner = ["models", "assetApis"].contains(&section);
        let runtime_owner = ["network", "securityRules"].contains(&section);
        let keys = if connection_owner {
            crate::configuration_connections::keys(section)?
        } else if runtime_owner {
            crate::configuration_runtime_owners::keys(section)?
        } else {
            policy_keys(section)?
        };
        let projected = |value: &Value| {
            if connection_owner {
                crate::configuration_connections::project(value, section)
            } else if runtime_owner {
                crate::configuration_runtime_owners::project(value, section)
            } else {
                project(value, section)
            }
        };
        if keys.iter().any(|key| input.contains_key(*key))
            && !crate::configuration_json::equivalent(&projected(&Value::Object(input.clone()))?, &projected(&current)?)
        {
            return Err("此分类已拆分为独立策略，请在设置中按分类保存（configuration_policy_scoped_save_required）".into());
        }
        for key in keys {
            input.remove(*key);
            if let Some(value) = stored.get(*key) {
                input.insert((*key).to_string(), value.clone());
            }
        }
    }
    crate::db::settings_for_update(stored, &Value::Object(input))
}

pub fn release_profile_bindings(connection: &Connection, profile_id: i64) -> Result<(), String> {
    crate::configuration_runtime_owners::release(connection, profile_id)?;
    crate::configuration_connections::release(connection, profile_id)?;
    if tables_exist(connection)? {
        connection
            .execute(
                "DELETE FROM configuration_policy_bindings WHERE profile_id=?1",
                [profile_id],
            )
            .map_err(|_| "configuration_policy_write_failed")?;
    }
    // Versions remain available to frozen task references after explicit removal.
    Ok(())
}

pub fn reference(settings: &Value, section: &str) -> Result<Option<PolicyReference>, String> {
    settings
        .get("_configurationRefs")
        .and_then(|references| references.get(section))
        .map(|value| {
            serde_json::from_value(value.clone())
                .map_err(|_| "configuration_policy_reference_invalid".into())
        })
        .transpose()
}

fn extended_defaults(section: &str) -> Map<String, Value> {
    let value = match section {
        "collection" => serde_json::json!({"collectionCache":true}),
        "probe" => serde_json::json!({"maxBodyBytes":524288,"allowPrivate":false,"strictTls":false,"schemeFallback":true,"perHostInterval":1.5}),
        _ => serde_json::json!({}),
    };
    value.as_object().unwrap().clone()
}

pub fn independent_references(db:&Connection,section:&str)->Result<Vec<PolicyReference>,String> {
    policy_keys(section)?;if !tables_exist(db)? {return Ok(vec![]);}
    let mut statement=db.prepare("SELECT policy_id,revision FROM configuration_policy_versions WHERE kind=?1 ORDER BY policy_id,revision DESC").map_err(|_|"configuration_policy_storage_unavailable")?;
    let result = statement.query_map([section],|r|Ok(PolicyReference{policy_id:r.get(0)?,revision:r.get(1)?,kind:section.into()})).map_err(|_|"configuration_policy_storage_unavailable")?.collect::<Result<Vec<_>,_>>().map_err(|_|"configuration_policy_storage_unavailable".into());
    result
}

pub fn independent_read(db:&Connection,reference:&PolicyReference)->Result<Value,String> {
    policy_keys(&reference.kind)?;if !tables_exist(db)? || reference.revision<=0 || reference.policy_id.is_empty() {return Err("configuration_policy_reference_invalid".into());}
    let (kind,text,digest):(String,String,String)=db.query_row("SELECT kind,settings_json,payload_digest FROM configuration_policy_versions WHERE policy_id=?1 AND revision=?2",params![reference.policy_id,reference.revision],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|"configuration_policy_reference_invalid")?;
    if kind!=reference.kind || digest!=format!("{:x}",Sha256::digest(text.as_bytes())) {return Err("configuration_policy_reference_invalid".into());}
    let value=serde_json::from_str(&text).map_err(|_|"configuration_policy_reference_invalid")?;validate(&reference.kind,&value)?;Ok(value)
}

pub fn independent_append(db:&Connection,section:&str,settings:&Value,previous:Option<&PolicyReference>)->Result<PolicyReference,String> {
    validate(section,settings)?;if db.is_autocommit() {return Err("configuration_policy_transaction_required".into());}
    if let Some(previous)=previous {
        if previous.kind!=section {return Err("configuration_policy_reference_invalid".into());}independent_read(db,previous)?;
        let head:i64=db.query_row("SELECT max(revision) FROM configuration_policy_versions WHERE policy_id=?1",[&previous.policy_id],|r|r.get(0)).map_err(|_|"configuration_policy_storage_unavailable")?;
        if head!=previous.revision {return Err("configuration_category_head_conflict".into());}
    }
    if !tables_exist(db)? {db.execute_batch(SCHEMA).map_err(|_|"configuration_policy_storage_unavailable")?;}
    let reference=PolicyReference{policy_id:previous.map(|r|r.policy_id.clone()).unwrap_or_else(||uuid::Uuid::new_v4().to_string()),revision:match previous {Some(r)=>r.revision.checked_add(1).ok_or("configuration_policy_revision_exhausted")?,None=>1},kind:section.into()};
    let text=settings.to_string();let digest=format!("{:x}",Sha256::digest(text.as_bytes()));
    if db.execute("INSERT INTO configuration_policy_versions(policy_id,revision,kind,settings_json,source_profile_id,source_digest,payload_digest) VALUES(?1,?2,?3,?4,0,?5,?5)",params![reference.policy_id,reference.revision,section,text,digest]).map_err(|_|"configuration_policy_write_failed")?!=1 {return Err("configuration_policy_write_failed".into());}
    independent_read(db,&reference)?;Ok(reference)
}

/// Caller owns an IMMEDIATE transaction; append a new immutable version and
/// switch only this profile's binding. Adoption is an explicit separate action.
pub fn save(
    connection: &Connection,
    profile_id: i64,
    section: &str,
    settings: &Value,
    expected_settings: &Value,
    expected_reference: Option<&PolicyReference>,
    adopt: bool,
) -> Result<PolicyReference, String> {
    validate(section, settings)?;
    let current = read_profile(connection, profile_id)?;
    let existing = reference(&current, section)?;
    if !crate::configuration_json::equivalent(&project(&current, section)?, expected_settings) || existing.as_ref() != expected_reference
    {
        return Err("此策略已被其他窗口修改，请重新打开（configuration_policy_conflict）".into());
    }
    if adopt != existing.is_none() {
        return Err(if adopt {
            "configuration_policy_already_adopted"
        } else {
            "configuration_policy_adoption_required"
        }
        .into());
    }
    let mut settings = settings.clone();
    for (key, default) in extended_defaults(section) {
        settings.as_object_mut().unwrap().entry(key.clone())
            .or_insert_with(|| current.get(&key).cloned().unwrap_or(default));
    }
    validate(section, &settings)?;
    if !tables_exist(connection)? {
        connection
            .execute_batch(SCHEMA)
            .map_err(|_| "configuration_policy_storage_unavailable")?;
    }
    let source_digest = format!(
        "{:x}",
        Sha256::digest(expected_settings.to_string().as_bytes())
    );
    let (policy_id, revision) = if let Some(existing) = existing {
        let revision = existing
            .revision
            .checked_add(1)
            .ok_or("configuration_policy_revision_exhausted")?;
        (existing.policy_id, revision)
    } else {
        (uuid::Uuid::new_v4().to_string(), 1)
    };
    connection.execute(
        "INSERT INTO configuration_policy_versions(policy_id,revision,kind,settings_json,source_profile_id,source_digest,payload_digest) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![policy_id,revision,section,settings.to_string(),profile_id,source_digest,format!("{:x}",Sha256::digest(settings.to_string().as_bytes()))],
    ).map_err(|_| "configuration_policy_write_failed")?;
    connection.execute(
        "INSERT INTO configuration_policy_bindings(profile_id,section,policy_id,revision) VALUES(?1,?2,?3,?4) ON CONFLICT(profile_id,section) DO UPDATE SET policy_id=excluded.policy_id,revision=excluded.revision",
        params![profile_id,section,policy_id,revision],
    ).map_err(|_| "configuration_policy_write_failed")?;
    Ok(PolicyReference {
        policy_id,
        revision,
        kind: section.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE config_profiles(id INTEGER PRIMARY KEY, settings_json TEXT); INSERT INTO config_profiles VALUES(1,'{}'),(2,'{}');").unwrap();
        let raw = json!({"workers":6,"pageSize":100,"fofaKey":"synthetic-kept","agentQuickScore":30,"historical":{"unchanged":true}}).to_string();
        db.execute("UPDATE config_profiles SET settings_json=?1", [&raw])
            .unwrap();
        db
    }
    #[test]
    fn old_owner_versions_preserve_legacy_extended_fields_until_explicit_scoped_save() {
        let mut db=fixture();
        let raw=serde_json::json!({"workers":6,"pageSize":100,"maxBodyBytes":65536,"allowPrivate":true,"strictTls":true,"schemeFallback":false,"perHostInterval":2.5,"collectionCache":false});
        db.execute("UPDATE config_profiles SET settings_json=?1 WHERE id=1",[raw.to_string()]).unwrap();
        db.execute_batch(SCHEMA).unwrap();
        for (section,payload) in [("probe",serde_json::json!({"workers":12})),("collection",serde_json::json!({"pageSize":200}))] {
            let text=payload.to_string();let id=uuid::Uuid::new_v4().to_string();
            db.execute("INSERT INTO configuration_policy_versions(policy_id,revision,kind,settings_json,source_profile_id,source_digest,payload_digest) VALUES(?1,1,?2,?3,1,'fixture',?4)",params![id,section,text,format!("{:x}",Sha256::digest(text.as_bytes()))]).unwrap();
            db.execute("INSERT INTO configuration_policy_bindings VALUES(1,?1,?2,1)",params![section,id]).unwrap();
        }
        let current=read_profile(&db,1).unwrap();
        for key in ["maxBodyBytes","allowPrivate","strictTls","schemeFallback","perHostInterval","collectionCache"] {assert_eq!(current[key],raw[key]);}
        let tx=db.transaction().unwrap();
        for (section,incoming) in [("probe",serde_json::json!({"workers":15})),("collection",serde_json::json!({"pageSize":300}))] {
            let old=reference(&current,section).unwrap().unwrap();
            save(&tx,1,section,&incoming,&project(&current,section).unwrap(),Some(&old),false).unwrap();
        }
        tx.commit().unwrap();
        db.execute("UPDATE config_profiles SET settings_json='{}' WHERE id=1",[]).unwrap();
        let frozen=read_profile(&db,1).unwrap();
        for key in ["maxBodyBytes","allowPrivate","strictTls","schemeFallback","perHostInterval","collectionCache"] {assert_eq!(frozen[key],raw[key]);}
        assert_eq!(frozen["workers"],15);assert_eq!(frozen["pageSize"],300);
        for section in ["probe","collection"] {assert_eq!(reference(&frozen,section).unwrap().unwrap().revision,2);}
    }
    #[test]
    fn policy_reads_never_migrate_legacy_or_copy_credentials() {
        let db = fixture();
        let before = db.total_changes();
        assert_eq!(read_profile(&db, 1).unwrap()["workers"], 6);
        assert!(!tables_exist(&db).unwrap());
        assert_eq!(db.total_changes(), before);
        let tx = db.unchecked_transaction().unwrap();
        save(
            &tx,
            1,
            "probe",
            &json!({"workers":12}),
            &json!({"workers":6}),
            None,
            true,
        )
        .unwrap();
        tx.commit().unwrap();
        assert_eq!(read_profile(&db, 1).unwrap()["workers"], 12);
        assert_eq!(read_profile(&db, 2).unwrap()["workers"], 6);
        let raw: String = db
            .query_row(
                "SELECT settings_json FROM config_profiles WHERE id=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(serde_json::from_str::<Value>(&raw).unwrap()["workers"], 6);
        let version: String = db
            .query_row(
                "SELECT settings_json FROM configuration_policy_versions",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!version.contains("synthetic-kept"));
        assert!(!version.contains("historical"));
    }
    #[test]
    fn policy_versions_are_immutable_and_stale_revisions_cannot_overwrite() {
        let mut db = fixture();
        let tx = db.transaction().unwrap();
        let first = save(
            &tx,
            1,
            "probe",
            &json!({"workers":12}),
            &json!({"workers":6}),
            None,
            true,
        )
        .unwrap();
        tx.commit().unwrap();
        let frozen = read_profile(&db, 1).unwrap();
        let tx = db.transaction().unwrap();
        let next = save(
            &tx,
            1,
            "probe",
            &json!({"workers":18}),
            &project(&frozen, "probe").unwrap(),
            Some(&first),
            false,
        )
        .unwrap();
        tx.commit().unwrap();
        assert_eq!(next.revision, 2);
        assert_eq!(frozen["workers"], 12);
        let current = project(&read_profile(&db, 1).unwrap(), "probe").unwrap();
        let tx = db.transaction().unwrap();
        assert!(save(
            &tx,
            1,
            "probe",
            &json!({"workers":30}),
            &current,
            Some(&first),
            false
        )
        .unwrap_err()
        .contains("conflict"));
        tx.rollback().unwrap();
        let old:String=db.query_row("SELECT settings_json FROM configuration_policy_versions WHERE policy_id=?1 AND revision=1",[first.policy_id],|r|r.get(0)).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&old).unwrap()["workers"], 12);
    }
    #[test]
    fn policy_categories_are_independent_and_unknown_fields_are_rejected() {
        let mut db = fixture();
        let tx = db.transaction().unwrap();
        save(
            &tx,
            1,
            "probe",
            &json!({"workers":12}),
            &json!({"workers":6}),
            None,
            true,
        )
        .unwrap();
        save(
            &tx,
            1,
            "collection",
            &json!({"pageSize":200}),
            &json!({"pageSize":100}),
            None,
            true,
        )
        .unwrap();
        tx.commit().unwrap();
        let read = read_profile(&db, 1).unwrap();
        assert_eq!(read["workers"], 12);
        assert_eq!(read["pageSize"], 200);
        let tx = db.transaction().unwrap();
        assert!(save(
            &tx,
            1,
            "rules",
            &json!({"fofaKey":"forbidden"}),
            &json!({}),
            None,
            true
        )
        .is_err());
        assert!(save(
            &tx,
            1,
            "agent",
            &json!({"agentQuickScore":"invalid"}),
            &json!({"agentQuickScore":30}),
            None,
            true
        )
        .is_err());
        tx.rollback().unwrap();
    }
    #[test]
    fn corrupted_binding_fails_closed_and_cannot_resurrect_legacy_policy() {
        let mut db = fixture();
        let tx = db.transaction().unwrap();
        save(
            &tx,
            1,
            "probe",
            &json!({"workers":12}),
            &json!({"workers":6}),
            None,
            true,
        )
        .unwrap();
        tx.commit().unwrap();
        db.execute_batch("PRAGMA foreign_keys=OFF; DROP TRIGGER configuration_policy_versions_immutable_delete; DELETE FROM configuration_policy_versions;")
            .unwrap();
        assert_eq!(
            read_profile(&db, 1).unwrap_err(),
            "configuration_policy_reference_invalid"
        );
    }
    #[test]
    fn legacy_writers_cannot_overwrite_or_duplicate_an_adopted_owner() {
        let mut db = fixture();
        let tx = db.transaction().unwrap();
        save(
            &tx,
            1,
            "probe",
            &json!({"workers":12}),
            &json!({"workers":6}),
            None,
            true,
        )
        .unwrap();
        tx.commit().unwrap();
        let raw: Value = serde_json::from_str(
            &db.query_row(
                "SELECT settings_json FROM config_profiles WHERE id=1",
                [],
                |r| r.get::<_, String>(0),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(legacy_update(&db, 1, &raw, &json!({"workers":18}))
            .unwrap_err()
            .contains("scoped_save_required"));
        let current = read_profile(&db, 1).unwrap();
        let updated = legacy_update(&db, 1, &raw, &current).unwrap();
        assert_eq!(updated["workers"], 6);
        assert!(updated.get("_configurationRefs").is_none());
        let model_only = legacy_update(&db, 1, &raw, &json!({"modelProfiles":[]})).unwrap();
        assert_eq!(model_only["workers"], 6);
    }
    #[test]
    fn policy_versions_reject_mutation_and_payload_corruption_fails_closed() {
        let mut db = fixture();
        let tx = db.transaction().unwrap();
        save(
            &tx,
            1,
            "probe",
            &json!({"workers":12}),
            &json!({"workers":6}),
            None,
            true,
        )
        .unwrap();
        tx.commit().unwrap();
        assert!(db
            .execute(
                "UPDATE configuration_policy_versions SET settings_json='{}'",
                []
            )
            .is_err());
        assert!(db
            .execute("DELETE FROM configuration_policy_versions", [])
            .is_err());
        db.execute_batch("DROP TRIGGER configuration_policy_versions_immutable_update; UPDATE configuration_policy_versions SET settings_json='{\"workers\":48}';").unwrap();
        assert_eq!(
            read_profile(&db, 1).unwrap_err(),
            "configuration_policy_digest_mismatch"
        );
    }
}
