//! Explicitly adopted scan connections and asset accounts. Descriptors contain
//! credential references only; secrets never enter policy/version payloads.
//! Reads do not create tables or migrate historical profile JSON.
use crate::configuration_policies::PolicyReference;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

const TABLES: &str = "'configuration_connection_versions','configuration_connection_bindings','configuration_model_connections','configuration_service_accounts'";
const SCHEMA: &str = r#"
CREATE TABLE configuration_connection_versions (
 owner_id TEXT NOT NULL, revision INTEGER NOT NULL CHECK(revision>0),
 kind TEXT NOT NULL CHECK(kind IN ('models','assetApis')),
 descriptor_json TEXT NOT NULL CHECK(json_valid(descriptor_json)),
 descriptor_digest TEXT NOT NULL, source_profile_id INTEGER NOT NULL,
 source_digest TEXT NOT NULL, PRIMARY KEY(owner_id,revision)
);
CREATE TABLE configuration_connection_bindings (
 profile_id INTEGER NOT NULL REFERENCES config_profiles(id),
 section TEXT NOT NULL CHECK(section IN ('models','assetApis')),
 owner_id TEXT NOT NULL, revision INTEGER NOT NULL,
 PRIMARY KEY(profile_id,section),
 FOREIGN KEY(owner_id,revision) REFERENCES configuration_connection_versions(owner_id,revision)
);
CREATE TABLE configuration_service_accounts (
 account_id TEXT PRIMARY KEY, owner TEXT NOT NULL CHECK(owner='rust-scanning'),
 provider TEXT NOT NULL CHECK(provider IN ('openai-compatible','fofa')),
 auth_type TEXT NOT NULL CHECK(auth_type IN ('cloud-api-key','local-api-key','fofa-api-key')),
 endpoint TEXT NOT NULL, credential TEXT NOT NULL
);
CREATE TABLE configuration_model_connections (
 connection_id TEXT PRIMARY KEY, model TEXT NOT NULL,
 deployment TEXT NOT NULL CHECK(deployment IN ('cloud','local')),
 endpoint TEXT NOT NULL, account_id TEXT NOT NULL REFERENCES configuration_service_accounts(account_id)
);
CREATE TRIGGER configuration_connection_versions_update BEFORE UPDATE ON configuration_connection_versions BEGIN SELECT RAISE(ABORT,'configuration_connection_immutable'); END;
CREATE TRIGGER configuration_connection_versions_delete BEFORE DELETE ON configuration_connection_versions BEGIN SELECT RAISE(ABORT,'configuration_connection_immutable'); END;
CREATE TRIGGER configuration_model_connections_update BEFORE UPDATE ON configuration_model_connections BEGIN SELECT RAISE(ABORT,'configuration_connection_immutable'); END;
CREATE TRIGGER configuration_model_connections_delete BEFORE DELETE ON configuration_model_connections BEGIN SELECT RAISE(ABORT,'configuration_connection_immutable'); END;
CREATE TRIGGER configuration_service_accounts_update BEFORE UPDATE ON configuration_service_accounts BEGIN SELECT RAISE(ABORT,'configuration_account_immutable'); END;
CREATE TRIGGER configuration_service_accounts_delete BEFORE DELETE ON configuration_service_accounts BEGIN SELECT RAISE(ABORT,'configuration_account_immutable'); END;
"#;

pub fn keys(section: &str) -> Result<&'static [&'static str], String> {
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
        _ => Err("configuration_connection_kind_invalid".into()),
    }
}

pub fn project(settings: &Value, section: &str) -> Result<Value, String> {
    Ok(Value::Object(
        keys(section)?
            .iter()
            .filter_map(|key| settings.get(*key).map(|v| ((*key).to_string(), v.clone())))
            .collect(),
    ))
}

fn exists(db: &Connection) -> Result<bool, String> {
    let count: i64 = db
        .query_row(
            &format!(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ({TABLES})"
            ),
            [],
            |r| r.get(0),
        )
        .map_err(|_| "configuration_connection_storage_unavailable")?;
    match count {
        0 => Ok(false),
        4 => Ok(true),
        _ => Err("configuration_connection_storage_incomplete".into()),
    }
}

fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("configuration_connection_field_invalid:{key}"))
}

fn endpoint(value: &str, deployment: &str) -> Result<String, String> {
    // Default is explicit and stable, never borrowed from OPENAI_BASE_URL or
    // the Host's OAuth settings. Local gateways must always name their address.
    let value = if value.trim().is_empty() && deployment == "cloud" {
        "https://api.openai.com/v1"
    } else {
        value.trim().trim_end_matches('/')
    };
    let url = reqwest::Url::parse(value).map_err(|_| "configuration_model_endpoint_invalid")?;
    if !["http", "https"].contains(&url.scheme())
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || value.chars().any(char::is_control)
    {
        return Err("configuration_model_endpoint_invalid".into());
    }
    Ok(value.to_string())
}

fn secret(
    db: &Connection,
    provider: &str,
    auth: &str,
    endpoint: &str,
    credential: &str,
) -> Result<String, String> {
    if credential.chars().any(char::is_control) || credential == "__CONFIGURED__" {
        return Err("configuration_account_credential_invalid".into());
    }
    if let Some(id)=db.query_row("SELECT account_id FROM configuration_service_accounts WHERE owner='rust-scanning' AND provider=?1 AND auth_type=?2 AND endpoint=?3 AND credential=?4 LIMIT 1",params![provider,auth,endpoint,credential],|r|r.get::<_,String>(0)).optional().map_err(|_|"configuration_account_read_failed")? {return Ok(id);}
    let id = uuid::Uuid::new_v4().to_string();
    db.execute("INSERT INTO configuration_service_accounts(account_id,owner,provider,auth_type,endpoint,credential) VALUES(?1,'rust-scanning',?2,?3,?4,?5)",params![id,provider,auth,endpoint,credential]).map_err(|_|"configuration_account_write_failed")?;
    Ok(id)
}

fn read_secret(
    db: &Connection,
    id: &str,
    provider: &str,
    auth: &str,
    endpoint: &str,
) -> Result<String, String> {
    let (owner,p,a,e,key):(String,String,String,String,String)=db.query_row("SELECT owner,provider,auth_type,endpoint,credential FROM configuration_service_accounts WHERE account_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(|_|"configuration_account_reference_invalid")?;
    if owner != "rust-scanning"
        || p != provider
        || a != auth
        || e != endpoint
        || key.chars().any(char::is_control)
        || key == "__CONFIGURED__"
    {
        return Err("configuration_account_scope_mismatch".into());
    }
    Ok(key)
}

fn write_descriptor(db: &Connection, section: &str, settings: &Value) -> Result<Value, String> {
    let object = settings
        .as_object()
        .ok_or("configuration_connection_settings_invalid")?;
    if object
        .keys()
        .any(|k| !keys(section).is_ok_and(|keys| keys.contains(&k.as_str())))
    {
        return Err("configuration_connection_field_denied".into());
    }
    if section == "assetApis" {
        let email = settings.get("fofaEmail").map(|_| string(settings,"fofaEmail"))
            .transpose()?.unwrap_or("").trim();
        if email.len()>320 || email.chars().any(char::is_control) {
            return Err("configuration_account_identity_invalid".into());
        }
        let key = settings
            .get("fofaKey")
            .map(|_| string(settings, "fofaKey"))
            .transpose()?
            .unwrap_or("")
            .trim();
        let path = settings
            .get("configPath")
            .map(|_| string(settings, "configPath"))
            .transpose()?
            .unwrap_or("")
            .trim();
        if !key.is_empty() && !path.is_empty() {
            return Err("FOFA API Key 与配置文件请选择一种，避免账号来源冲突".into());
        }
        if path.chars().any(char::is_control) {
            return Err("configuration_account_path_invalid".into());
        }
        let id = secret(db, "fofa", "fofa-api-key", "https://fofa.info", key)?;
        // A path is a declared source, not permission to read it during adoption.
        return Ok(json!({"accountId":id,"fofaEmail":email,"configPath":path}));
    }
    let profiles = settings
        .get("modelProfiles")
        .and_then(Value::as_array)
        .ok_or("请先保存模型列表，再预览拆分")?;
    let active = string(settings, "activeModelProfileId")?;
    let mut seen = std::collections::HashSet::new();
    let mut entries = Vec::new();
    for p in profiles {
        let id = string(p, "id")?;
        let model = string(p, "llm")?.trim();
        let deployment = string(p, "deployment")?;
        if id.is_empty()
            || !seen.insert(id)
            || model.is_empty()
            || !["local", "cloud"].contains(&deployment)
        {
            return Err("configuration_model_identity_invalid".into());
        }
        let base = endpoint(string(p, "apiBase")?, deployment)?;
        let auth = if deployment == "local" {
            "local-api-key"
        } else {
            "cloud-api-key"
        };
        let key = string(
            p,
            if deployment == "local" {
                "localApiKey"
            } else {
                "apiKey"
            },
        )?
        .trim();
        if deployment == "cloud" && key.is_empty() {
            return Err("独立云模型必须填写明确的 API Key；不会使用 Host OAuth 或环境变量".into());
        }
        let account = secret(db, "openai-compatible", auth, &base, key)?;
        let prior=db.query_row("SELECT connection_id FROM configuration_model_connections WHERE model=?1 AND deployment=?2 AND endpoint=?3 AND account_id=?4 LIMIT 1",params![model,deployment,base,account],|r|r.get::<_,String>(0)).optional().map_err(|_|"configuration_model_read_failed")?;
        let connection = if let Some(id) = prior {
            id
        } else {
            let id = uuid::Uuid::new_v4().to_string();
            db.execute("INSERT INTO configuration_model_connections(connection_id,model,deployment,endpoint,account_id) VALUES(?1,?2,?3,?4,?5)",params![id,model,deployment,base,account]).map_err(|_|"configuration_model_write_failed")?;
            id
        };
        entries.push(json!({"id":id,"name":p.get("name").and_then(Value::as_str).unwrap_or(""),"connectionId":connection}));
    }
    if (profiles.is_empty() && !active.is_empty())
        || (!profiles.is_empty() && !seen.contains(active))
    {
        return Err("configuration_model_active_id_invalid".into());
    }
    Ok(json!({"activeModelProfileId":active,"entries":entries}))
}

fn materialize(db: &Connection, section: &str, descriptor: &Value) -> Result<Value, String> {
    if section == "assetApis" {
        let key = read_secret(
            db,
            string(descriptor, "accountId")?,
            "fofa",
            "fofa-api-key",
            "https://fofa.info",
        )?;
        let path = string(descriptor, "configPath")?;
        if !key.is_empty() && !path.is_empty() {
            return Err("configuration_account_source_conflict".into());
        }
        let mut value=json!({"fofaKey":key,"configPath":path});
        // Old immutable descriptors did not record identity. Keep them readable
        // and let a legacy profile retain its declared email until explicit save.
        if descriptor.get("fofaEmail").is_some() {
            let email=string(descriptor,"fofaEmail")?;
            if email.len()>320 || email.chars().any(char::is_control) {return Err("configuration_account_identity_invalid".into());}
            value["fofaEmail"]=json!(email);
        }
        return Ok(value);
    }
    let active = string(descriptor, "activeModelProfileId")?;
    let entries = descriptor
        .get("entries")
        .and_then(Value::as_array)
        .ok_or("configuration_connection_payload_invalid")?;
    let mut models = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for entry in entries {
        let id = string(entry, "id")?;
        if id.is_empty() || !seen.insert(id) {
            return Err("configuration_model_identity_invalid".into());
        }
        let (model,deployment,base,account):(String,String,String,String)=db.query_row("SELECT model,deployment,endpoint,account_id FROM configuration_model_connections WHERE connection_id=?1",[string(entry,"connectionId")?],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|_|"configuration_model_reference_invalid")?;
        if model.trim().is_empty()
            || !["cloud", "local"].contains(&deployment.as_str())
            || endpoint(&base, &deployment)? != base
        {
            return Err("configuration_model_reference_invalid".into());
        }
        let key = read_secret(
            db,
            &account,
            "openai-compatible",
            if deployment == "local" {
                "local-api-key"
            } else {
                "cloud-api-key"
            },
            &base,
        )?;
        if deployment == "cloud" && key.is_empty() {
            return Err("configuration_account_credential_invalid".into());
        }
        models.push(json!({"id":id,"name":string(entry,"name")?,"llm":model,"deployment":deployment,"apiBase":base,"apiKey":if deployment=="cloud" {key.as_str()} else {""},"localApiKey":if deployment=="local" {key.as_str()} else {""}}));
    }
    if (models.is_empty() && !active.is_empty()) || (!models.is_empty() && !seen.contains(active)) {
        return Err("configuration_model_active_id_invalid".into());
    }
    let selected = models.iter().find(|m| m["id"] == active);
    Ok(json!({"modelProfiles":models,"activeModelProfileId":active,
        "modelName":selected.and_then(|m|m.get("llm")).cloned().unwrap_or(json!("")),
        "modelDeployment":selected.and_then(|m|m.get("deployment")).cloned().unwrap_or(json!("cloud")),
        "modelApiBase":selected.and_then(|m|m.get("apiBase")).cloned().unwrap_or(json!("")),
        "modelApiKey":selected.and_then(|m|m.get("apiKey")).cloned().unwrap_or(json!("")),
        "localApiKey":selected.and_then(|m|m.get("localApiKey")).cloned().unwrap_or(json!(""))}))
}

/// Merge independently owned descriptors into the compatibility runtime view.
pub fn overlay(
    db: &Connection,
    profile: i64,
    object: &mut Map<String, Value>,
    references: &mut Map<String, Value>,
) -> Result<(), String> {
    if !exists(db)? {
        return Ok(());
    }
    let mut statement=db.prepare("SELECT b.section,b.owner_id,b.revision,v.kind,v.descriptor_json,v.descriptor_digest FROM configuration_connection_bindings b LEFT JOIN configuration_connection_versions v ON v.owner_id=b.owner_id AND v.revision=b.revision WHERE b.profile_id=?1 ORDER BY b.section").map_err(|_|"configuration_connection_storage_unavailable")?;
    let rows = statement
        .query_map([profile], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
            ))
        })
        .map_err(|_| "configuration_connection_storage_unavailable")?;
    for row in rows {
        let (section, id, revision, kind, payload, digest) =
            row.map_err(|_| "configuration_connection_reference_invalid")?;
        if revision <= 0 || kind.as_deref() != Some(section.as_str()) {
            return Err("configuration_connection_reference_invalid".into());
        }
        let payload = payload.ok_or("configuration_connection_reference_invalid")?;
        if digest.as_deref() != Some(format!("{:x}", Sha256::digest(payload.as_bytes())).as_str()) {
            return Err("configuration_connection_digest_mismatch".into());
        }
        let value: Value = serde_json::from_str(&payload)
            .map_err(|_| "configuration_connection_payload_invalid")?;
        let effective = materialize(db, &section, &value)?;
        for key in keys(&section)? {
            if *key!="fofaEmail" || effective.get(*key).is_some() {object.remove(*key);}
        }
        object.extend(effective.as_object().unwrap().clone());
        references.insert(
            section.clone(),
            serde_json::to_value(PolicyReference {
                policy_id: id,
                revision,
                kind: section,
            })
            .map_err(|_| "configuration_connection_reference_invalid")?,
        );
    }
    Ok(())
}

/// Caller holds IMMEDIATE. Atomic append/switch, old JSON and old references
/// stay unchanged. No provider test, OAuth, file read or environment mutation.
pub fn save(
    db: &Connection,
    profile: i64,
    section: &str,
    settings: &Value,
    expected: &Value,
    expected_ref: Option<&PolicyReference>,
    adopt: bool,
) -> Result<PolicyReference, String> {
    if db.is_autocommit() {
        return Err("configuration_connection_transaction_required".into());
    }
    keys(section)?;
    let current = crate::configuration_policies::read_profile(db, profile)?;
    let reference = crate::configuration_policies::reference(&current, section)?;
    if !crate::configuration_json::equivalent(&project(&current, section)?, expected) || reference.as_ref() != expected_ref {
        return Err("configuration_connection_conflict".into());
    }
    if adopt != reference.is_none() {
        return Err("configuration_connection_adoption_conflict".into());
    }
    if !exists(db)? {
        db.execute_batch(SCHEMA)
            .map_err(|_| "configuration_connection_storage_unavailable")?;
    }
    let mut settings=settings.clone();
    if section=="assetApis" && settings.get("fofaEmail").is_none() {
        settings["fofaEmail"]=current.get("fofaEmail").cloned().unwrap_or(json!(""));
    }
    let descriptor = write_descriptor(db, section, &settings)?.to_string();
    let (id, revision) = match reference {
        Some(r) => (
            r.policy_id,
            r.revision
                .checked_add(1)
                .ok_or("configuration_connection_revision_exhausted")?,
        ),
        None => (uuid::Uuid::new_v4().to_string(), 1),
    };
    db.execute("INSERT INTO configuration_connection_versions(owner_id,revision,kind,descriptor_json,descriptor_digest,source_profile_id,source_digest) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![id,revision,section,descriptor,format!("{:x}",Sha256::digest(descriptor.as_bytes())),profile,format!("{:x}",Sha256::digest(expected.to_string().as_bytes()))]).map_err(|_|"configuration_connection_write_failed")?;
    db.execute("INSERT INTO configuration_connection_bindings(profile_id,section,owner_id,revision) VALUES(?1,?2,?3,?4) ON CONFLICT(profile_id,section) DO UPDATE SET owner_id=excluded.owner_id,revision=excluded.revision",params![profile,section,id,revision]).map_err(|_|"configuration_connection_write_failed")?;
    Ok(PolicyReference {
        policy_id: id,
        revision,
        kind: section.into(),
    })
}

pub fn release(db: &Connection, profile: i64) -> Result<(), String> {
    if exists(db)? {
        db.execute(
            "DELETE FROM configuration_connection_bindings WHERE profile_id=?1",
            [profile],
        )
        .map_err(|_| "configuration_connection_write_failed")?;
    }
    Ok(())
}

pub fn independent_references(db: &Connection, section: &str) -> Result<Vec<PolicyReference>, String> {
    keys(section)?;
    if !exists(db)? {return Ok(vec![]);}
    let mut statement=db.prepare("SELECT owner_id,revision FROM configuration_connection_versions WHERE kind=?1 ORDER BY owner_id,revision DESC").map_err(|_|"configuration_connection_read_failed")?;
    let result = statement.query_map([section],|r|Ok(PolicyReference{policy_id:r.get(0)?,revision:r.get(1)?,kind:section.into()})).map_err(|_|"configuration_connection_read_failed")?.collect::<Result<Vec<_>,_>>().map_err(|_|"configuration_connection_read_failed".into());
    result
}

pub fn independent_read(db: &Connection, reference: &PolicyReference) -> Result<Value, String> {
    keys(&reference.kind)?;
    if !exists(db)? || reference.revision<=0 || reference.policy_id.is_empty() {return Err("configuration_connection_reference_invalid".into());}
    let (kind,text,digest):(String,String,String)=db.query_row("SELECT kind,descriptor_json,descriptor_digest FROM configuration_connection_versions WHERE owner_id=?1 AND revision=?2",params![reference.policy_id,reference.revision],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|"configuration_connection_reference_invalid")?;
    if kind!=reference.kind || digest!=format!("{:x}",Sha256::digest(text.as_bytes())) {return Err("configuration_connection_reference_invalid".into());}
    let descriptor=serde_json::from_str(&text).map_err(|_|"configuration_connection_reference_invalid")?;
    materialize(db,&reference.kind,&descriptor)
}

/// A catalog append has no profile binding. The caller owns an IMMEDIATE
/// transaction and category/default authority; older profile references remain.
pub fn independent_append(db: &Connection, section: &str, settings: &Value, previous: Option<&PolicyReference>) -> Result<PolicyReference,String> {
    keys(section)?;
    if db.is_autocommit() {return Err("configuration_connection_transaction_required".into());}
    if let Some(previous)=previous {
        if previous.kind!=section {return Err("configuration_connection_reference_invalid".into());}
        independent_read(db,previous)?;
        let head:i64=db.query_row("SELECT max(revision) FROM configuration_connection_versions WHERE owner_id=?1",[&previous.policy_id],|r|r.get(0)).map_err(|_|"configuration_connection_read_failed")?;
        if head!=previous.revision {return Err("configuration_category_head_conflict".into());}
    }
    if !exists(db)? {db.execute_batch(SCHEMA).map_err(|_|"configuration_connection_storage_unavailable")?;}
    let descriptor=write_descriptor(db,section,settings)?.to_string();
    let reference=PolicyReference{policy_id:previous.map(|r|r.policy_id.clone()).unwrap_or_else(||uuid::Uuid::new_v4().to_string()),revision:match previous {Some(r)=>r.revision.checked_add(1).ok_or("configuration_connection_revision_exhausted")?,None=>1},kind:section.into()};
    let count=db.execute("INSERT INTO configuration_connection_versions(owner_id,revision,kind,descriptor_json,descriptor_digest,source_profile_id,source_digest) VALUES(?1,?2,?3,?4,?5,0,?6)",params![reference.policy_id,reference.revision,section,descriptor,format!("{:x}",Sha256::digest(descriptor.as_bytes())),format!("{:x}",Sha256::digest(settings.to_string().as_bytes()))]).map_err(|_|"configuration_connection_write_failed")?;
    if count!=1 {return Err("configuration_connection_write_failed".into());}
    independent_read(db,&reference)?;
    Ok(reference)
}

/// Asset jobs do not consume scan models. Persist their references, never an
/// unrelated scan key copied from a legacy/effective business profile.
pub fn redact_asset_snapshot(settings: &mut Value) {
    if let Some(object) = settings.as_object_mut() {
        for key in ["modelApiKey", "localApiKey"] {
            object.remove(key);
        }
        if let Some(profiles) = object
            .get_mut("modelProfiles")
            .and_then(Value::as_array_mut)
        {
            for profile in profiles {
                if let Some(p) = profile.as_object_mut() {
                    p.remove("apiKey");
                    p.remove("localApiKey");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE config_profiles(id INTEGER PRIMARY KEY,settings_json TEXT); INSERT INTO config_profiles VALUES(1,'{}'),(2,'{}');").unwrap();
        let raw = json!({"modelProfiles":[{"id":"one","name":"Synthetic","llm":"grok-test","deployment":"cloud","apiBase":"https://synthetic.invalid/v1","apiKey":"synthetic-cloud-key","localApiKey":"unselected-legacy-key","futureField":"preserved-in-source"}],"activeModelProfileId":"one","fofaKey":"synthetic-fofa-key","configPath":"","workers":6,"historical":{"kept":true}});
        db.execute(
            "UPDATE config_profiles SET settings_json=?1",
            [raw.to_string()],
        )
        .unwrap();
        db
    }
    fn raw(db: &Connection) -> String {
        db.query_row(
            "SELECT settings_json FROM config_profiles WHERE id=1",
            [],
            |r| r.get(0),
        )
        .unwrap()
    }
    fn adopt(db: &mut Connection, section: &str) -> PolicyReference {
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        let settings = crate::configuration_policies::read_profile(&tx, 1).unwrap();
        let payload = project(&settings, section).unwrap();
        let reference = save(&tx, 1, section, &payload, &payload, None, true).unwrap();
        tx.commit().unwrap();
        reference
    }
    #[test]
    fn connection_reads_are_lazy_and_explicit_adoption_keeps_legacy_bytes() {
        let mut db = fixture();
        let original = raw(&db);
        let changes = db.total_changes();
        let settings = crate::configuration_policies::read_profile(&db, 1).unwrap();
        assert_eq!(settings["workers"], 6);
        assert!(!exists(&db).unwrap());
        assert_eq!(db.total_changes(), changes);
        let r = adopt(&mut db, "models");
        assert_eq!(raw(&db), original);
        let effective = crate::configuration_policies::read_profile(&db, 1).unwrap();
        assert_eq!(
            crate::configuration_policies::reference(&effective, "models").unwrap(),
            Some(r)
        );
        assert_eq!(
            effective["modelProfiles"][0]["apiKey"],
            "synthetic-cloud-key"
        );
        assert_eq!(effective["modelProfiles"][0]["localApiKey"], "");
        assert_eq!(effective["fofaKey"], "synthetic-fofa-key");
        let descriptors: String = db
            .query_row(
                "SELECT group_concat(descriptor_json) FROM configuration_connection_versions",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!descriptors.contains("synthetic-cloud-key"));
        assert!(!descriptors.contains("unselected-legacy-key"));
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM configuration_service_accounts",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        // Other profiles remain legacy and are not silently adopted.
        assert!(crate::configuration_policies::reference(
            &crate::configuration_policies::read_profile(&db, 2).unwrap(),
            "models"
        )
        .unwrap()
        .is_none());
    }
    #[test]
    fn connection_rename_reuses_credentials_but_endpoint_rotation_cannot_retarget_old_revision() {
        let mut db = fixture();
        let first = adopt(&mut db, "models");
        let current = crate::configuration_policies::read_profile(&db, 1).unwrap();
        let expected = project(&current, "models").unwrap();
        let mut next = expected.clone();
        next["modelProfiles"][0]["name"] = json!("Renamed");
        let tx = db.transaction().unwrap();
        let second = save(&tx, 1, "models", &next, &expected, Some(&first), false).unwrap();
        tx.commit().unwrap();
        assert_eq!(second.revision, 2);
        assert_eq!(first.policy_id, second.policy_id);
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM configuration_service_accounts",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        let current = crate::configuration_policies::read_profile(&db, 1).unwrap();
        let expected = project(&current, "models").unwrap();
        let mut next = expected.clone();
        next["modelProfiles"][0]["apiBase"] = json!("https://new.invalid/v1");
        next["modelProfiles"][0]["apiKey"] = json!("synthetic-new-key");
        let tx = db.transaction().unwrap();
        save(&tx, 1, "models", &next, &expected, Some(&second), false).unwrap();
        tx.commit().unwrap();
        let original:String=db.query_row("SELECT descriptor_json FROM configuration_connection_versions WHERE owner_id=?1 AND revision=1",[&first.policy_id],|r|r.get(0)).unwrap();
        let frozen = materialize(&db, "models", &serde_json::from_str(&original).unwrap()).unwrap();
        assert_eq!(
            frozen["modelProfiles"][0]["apiBase"],
            "https://synthetic.invalid/v1"
        );
        assert_eq!(frozen["modelProfiles"][0]["apiKey"], "synthetic-cloud-key");
        let tx = db.transaction().unwrap();
        assert_eq!(
            save(&tx, 1, "models", &next, &expected, Some(&second), false).unwrap_err(),
            "configuration_connection_conflict"
        );
    }
    #[test]
    fn connection_owner_rejects_implicit_environment_keys_duplicate_models_and_unknown_active_id() {
        for change in ["empty-key", "duplicate", "active", "oauth-endpoint"] {
            let mut db = fixture();
            let original = raw(&db);
            let tx = db.transaction().unwrap();
            let current = crate::configuration_policies::read_profile(&tx, 1).unwrap();
            let expected = project(&current, "models").unwrap();
            let mut next = expected.clone();
            match change {
                "empty-key" => next["modelProfiles"][0]["apiKey"] = json!(""),
                "duplicate" => {
                    let p = next["modelProfiles"][0].clone();
                    next["modelProfiles"].as_array_mut().unwrap().push(p);
                }
                "active" => next["activeModelProfileId"] = json!("missing"),
                _ => {
                    next["modelProfiles"][0]["apiBase"] =
                        json!("https://user:token@synthetic.invalid/v1")
                }
            }
            assert!(save(&tx, 1, "models", &next, &expected, None, true).is_err());
            tx.rollback().unwrap();
            assert!(!exists(&db).unwrap());
            assert_eq!(raw(&db), original);
        }
    }
    #[test]
    fn local_connection_never_uses_cloud_key_and_owned_models_reject_runtime_environment_fallback()
    {
        let mut db = fixture();
        let tx = db.transaction().unwrap();
        let current = crate::configuration_policies::read_profile(&tx, 1).unwrap();
        let expected = project(&current, "models").unwrap();
        let mut local = expected.clone();
        local["modelProfiles"][0]["deployment"] = json!("local");
        local["modelProfiles"][0]["apiBase"] = json!("http://127.0.0.1:9000/v1");
        local["modelProfiles"][0]["localApiKey"] = json!("");
        save(&tx, 1, "models", &local, &expected, None, true).unwrap();
        tx.commit().unwrap();
        let settings = crate::configuration_policies::read_profile(&db, 1).unwrap();
        let c = crate::model_configuration::model_connection(
            &crate::model_configuration::NormalizedModelSettings::new(&settings),
        )
        .unwrap();
        assert_eq!(c.api_key, "local");
        assert_eq!(c.deployment, "local");
        let accounts: String = db
            .query_row(
                "SELECT group_concat(credential) FROM configuration_service_accounts",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!accounts.contains("synthetic-cloud-key"));
        let mut broken = settings;
        broken["modelProfiles"][0]["deployment"] = json!("cloud");
        assert!(crate::model_configuration::model_connection(
            &crate::model_configuration::NormalizedModelSettings::new(&broken)
        )
        .is_err());
    }
    #[test]
    fn asset_account_is_independent_and_declared_file_is_not_read_during_adoption() {
        let mut db = fixture();
        let original = raw(&db);
        let r = adopt(&mut db, "assetApis");
        let current = crate::configuration_policies::read_profile(&db, 1).unwrap();
        let expected = project(&current, "assetApis").unwrap();
        let tx = db.transaction().unwrap();
        let next = json!({"fofaKey":"","configPath":"/missing/not-read/synthetic.yaml"});
        save(&tx, 1, "assetApis", &next, &expected, Some(&r), false).unwrap();
        tx.commit().unwrap();
        let current = crate::configuration_policies::read_profile(&db, 1).unwrap();
        assert_eq!(current["configPath"], "/missing/not-read/synthetic.yaml");
        assert_eq!(raw(&db), original);
        assert!(crate::configuration_policies::reference(&current, "models")
            .unwrap()
            .is_none());
        let tx = db.transaction().unwrap();
        let expected = project(&current, "assetApis").unwrap();
        let r = crate::configuration_policies::reference(&current, "assetApis")
            .unwrap()
            .unwrap();
        assert!(save(
            &tx,
            1,
            "assetApis",
            &json!({"fofaKey":"ambiguous","configPath":"/missing"}),
            &expected,
            Some(&r),
            false
        )
        .is_err());
    }
    #[test]
    fn connection_versions_are_immutable_and_broken_bindings_fail_closed() {
        let mut db = fixture();
        adopt(&mut db, "models");
        assert!(db
            .execute(
                "UPDATE configuration_service_accounts SET endpoint='https://other.invalid'",
                []
            )
            .is_err());
        assert!(db
            .execute("DELETE FROM configuration_model_connections", [])
            .is_err());
        db.execute_batch(
            "PRAGMA foreign_keys=OFF; UPDATE configuration_connection_bindings SET revision=999;",
        )
        .unwrap();
        assert!(crate::configuration_policies::read_profile(&db, 1).is_err());
    }
    #[test]
    fn legacy_full_save_cannot_overwrite_or_duplicate_adopted_credentials() {
        let mut db = fixture();
        let source: Value = serde_json::from_str(&raw(&db)).unwrap();
        adopt(&mut db, "models");
        let current = crate::configuration_policies::read_profile(&db, 1).unwrap();
        let mut changed = current.clone();
        changed["modelProfiles"][0]["apiKey"] = json!("new-synthetic-key");
        assert!(crate::configuration_policies::legacy_update(&db, 1, &source, &changed).is_err());
        let merged =
            crate::configuration_policies::legacy_update(&db, 1, &source, &current).unwrap();
        assert_eq!(merged["modelProfiles"], source["modelProfiles"]);
        assert!(merged.get("_configurationRefs").is_none());
        let mut snapshot = current;
        redact_asset_snapshot(&mut snapshot);
        let serialized = snapshot.to_string();
        assert!(!serialized.contains("synthetic-cloud-key"));
        assert!(!serialized.contains("unselected-legacy-key"));
        assert!(snapshot["_configurationRefs"]["models"].is_object());
    }
}
