//! Independently owned, explicitly adopted business
//! transport and analyzer settings; no Host OAuth and no rule-source mutation.
use crate::configuration_policies::{self as policies, PolicyReference};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

const TABLES:&str="'configuration_runtime_versions','configuration_runtime_bindings','configuration_proxy_credentials'";
const SCHEMA: &str = r#"
CREATE TABLE configuration_runtime_versions(owner_id TEXT NOT NULL,revision INTEGER NOT NULL CHECK(revision>0),kind TEXT NOT NULL CHECK(kind IN ('network','securityRules')),descriptor_json TEXT NOT NULL CHECK(json_valid(descriptor_json)),digest TEXT NOT NULL,PRIMARY KEY(owner_id,revision));
CREATE TABLE configuration_runtime_bindings(profile_id INTEGER NOT NULL REFERENCES config_profiles(id),section TEXT NOT NULL CHECK(section IN ('network','securityRules')),owner_id TEXT NOT NULL,revision INTEGER NOT NULL,PRIMARY KEY(profile_id,section),FOREIGN KEY(owner_id,revision) REFERENCES configuration_runtime_versions(owner_id,revision));
CREATE TABLE configuration_proxy_credentials(credential_id TEXT PRIMARY KEY,owner TEXT NOT NULL CHECK(owner='business-transport'),endpoint TEXT NOT NULL,proxy_url TEXT NOT NULL);
CREATE TRIGGER configuration_runtime_versions_no_update BEFORE UPDATE ON configuration_runtime_versions BEGIN SELECT RAISE(ABORT,'configuration_runtime_immutable'); END;
CREATE TRIGGER configuration_runtime_versions_no_delete BEFORE DELETE ON configuration_runtime_versions BEGIN SELECT RAISE(ABORT,'configuration_runtime_immutable'); END;
CREATE TRIGGER configuration_runtime_versions_no_replace BEFORE INSERT ON configuration_runtime_versions WHEN EXISTS(SELECT 1 FROM configuration_runtime_versions WHERE owner_id=NEW.owner_id AND revision=NEW.revision) BEGIN SELECT RAISE(ABORT,'configuration_runtime_immutable'); END;
CREATE TRIGGER configuration_proxy_credentials_no_update BEFORE UPDATE ON configuration_proxy_credentials BEGIN SELECT RAISE(ABORT,'configuration_proxy_credential_immutable'); END;
CREATE TRIGGER configuration_proxy_credentials_no_delete BEFORE DELETE ON configuration_proxy_credentials BEGIN SELECT RAISE(ABORT,'configuration_proxy_credential_immutable'); END;
CREATE TRIGGER configuration_proxy_credentials_no_replace BEFORE INSERT ON configuration_proxy_credentials WHEN EXISTS(SELECT 1 FROM configuration_proxy_credentials WHERE credential_id=NEW.credential_id) BEGIN SELECT RAISE(ABORT,'configuration_proxy_credential_immutable'); END;
"#;

pub fn keys(section: &str) -> Result<&'static [&'static str], String> {
    match section {
        "network" => Ok(&[
            "proxyUrl",
            "noProxy",
            "agentProxyEnabled",
            "authorizedProxyPool",
        ]),
        "securityRules" => Ok(&["nativeAnalyzers"]),
        _ => Err("configuration_runtime_kind_invalid".into()),
    }
}
pub fn project(settings: &Value, section: &str) -> Result<Value, String> {
    Ok(Value::Object(
        keys(section)?
            .iter()
            .filter_map(|key| {
                settings
                    .get(*key)
                    .map(|value| ((*key).into(), value.clone()))
            })
            .collect(),
    ))
}
fn exists(db: &Connection) -> Result<bool, String> {
    let count: i64 = db
        .query_row(
            &format!(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ({TABLES})"
            ),
            [],
            |r| r.get(0),
        )
        .map_err(|_| "configuration_runtime_storage_unavailable")?;
    match count {
        0 => Ok(false),
        3 => {
            let guards: i64 = db.query_row("SELECT count(*) FROM sqlite_schema WHERE type='trigger' AND name IN ('configuration_runtime_versions_no_update','configuration_runtime_versions_no_delete','configuration_runtime_versions_no_replace','configuration_proxy_credentials_no_update','configuration_proxy_credentials_no_delete','configuration_proxy_credentials_no_replace')", [], |r| r.get(0)).map_err(|_| "configuration_runtime_storage_unavailable")?;
            if guards != 6 {
                return Err("configuration_runtime_storage_incomplete".into());
            }
            Ok(true)
        }
        _ => Err("configuration_runtime_storage_incomplete".into()),
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Transport {
    proxy_url: String,
    no_proxy: String,
    agent_proxy_enabled: bool,
    authorized_proxy_pool: Vec<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AnalyzerImage {
    image: String,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AnalyzerSettings {
    semgrep: AnalyzerImage,
    codeql: AnalyzerImage,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Analyzers {
    native_analyzers: AnalyzerSettings,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PoolEntry {
    tag: String,
    proxy: Value,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TransportDescriptor {
    proxy: Value,
    no_proxy: String,
    agent_proxy_enabled: bool,
    pool: Vec<PoolEntry>,
}

fn image(value: &str) -> bool {
    if value.is_empty() {
        return true;
    }
    let Some((repository, digest)) = value.rsplit_once("@sha256:") else {
        return false;
    };
    !repository.is_empty()
        && !repository.contains('@')
        && !repository.chars().any(char::is_whitespace)
        && digest.len() == 64
        && digest.bytes().all(|b| b.is_ascii_hexdigit())
}
fn proxy_endpoint(value: &str) -> Result<String, String> {
    if value.len() > 16 * 1024 || value.chars().any(char::is_control) {
        return Err("configuration_proxy_invalid".into());
    }
    let mut url = reqwest::Url::parse(value).map_err(|_| "configuration_proxy_invalid")?;
    if !["http", "https", "socks5", "socks5h"].contains(&url.scheme())
        || url.host_str().is_none()
        || url.fragment().is_some()
        || url.query().is_some()
    {
        return Err("configuration_proxy_invalid".into());
    }
    url.set_username("")
        .map_err(|_| "configuration_proxy_invalid")?;
    url.set_password(None)
        .map_err(|_| "configuration_proxy_invalid")?;
    // Only the endpoint belongs in descriptors; a URL path is not needed to
    // identify a proxy and may itself contain private connection information.
    url.set_path("");
    Ok(url.to_string())
}

fn save_proxy(db: &Connection, raw: &str) -> Result<Value, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(Value::Null);
    }
    let endpoint = proxy_endpoint(raw)?;
    let existing:Option<String>=db.query_row("SELECT credential_id FROM configuration_proxy_credentials WHERE owner='business-transport' AND endpoint=?1 AND proxy_url=?2 LIMIT 1",params![endpoint,raw],|r|r.get(0)).optional().map_err(|_|"configuration_proxy_read_failed")?;
    let id = match existing {
        Some(id) => id,
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            db.execute("INSERT INTO configuration_proxy_credentials(credential_id,owner,endpoint,proxy_url) VALUES(?1,'business-transport',?2,?3)",params![id,endpoint,raw]).map_err(|_|"configuration_proxy_write_failed")?;
            id
        }
    };
    Ok(json!({"endpoint":endpoint,"credentialId":id}))
}
fn materialize_proxy(db: &Connection, value: &Value) -> Result<String, String> {
    if value.is_null() {
        return Ok(String::new());
    }
    let object = value
        .as_object()
        .ok_or("configuration_proxy_reference_invalid")?;
    if object.len() != 2 {
        return Err("configuration_proxy_reference_invalid".into());
    }
    let id = value
        .get("credentialId")
        .and_then(Value::as_str)
        .ok_or("configuration_proxy_reference_invalid")?;
    let endpoint = value
        .get("endpoint")
        .and_then(Value::as_str)
        .ok_or("configuration_proxy_reference_invalid")?;
    let (owner,stored,raw):(String,String,String)=db.query_row("SELECT owner,endpoint,proxy_url FROM configuration_proxy_credentials WHERE credential_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|"configuration_proxy_reference_invalid")?;
    if owner != "business-transport" || stored != endpoint || proxy_endpoint(&raw)? != endpoint {
        return Err("configuration_proxy_scope_mismatch".into());
    }
    Ok(raw)
}

fn descriptor(db: &Connection, section: &str, settings: &Value) -> Result<Value, String> {
    if section == "securityRules" {
        let typed: Analyzers = serde_json::from_value(settings.clone())
            .map_err(|_| "configuration_analyzer_settings_invalid")?;
        if !image(&typed.native_analyzers.semgrep.image)
            || !image(&typed.native_analyzers.codeql.image)
        {
            return Err("configuration_analyzer_pinned_image_required".into());
        }
        return serde_json::to_value(typed)
            .map_err(|_| "configuration_analyzer_settings_invalid".into());
    }
    let typed: Transport = serde_json::from_value(settings.clone())
        .map_err(|_| "configuration_transport_settings_invalid")?;
    if typed.no_proxy.len() > 16 * 1024
        || typed.no_proxy.chars().any(char::is_control)
        || typed.authorized_proxy_pool.len() > 128
    {
        return Err("configuration_transport_settings_invalid".into());
    }
    let mut pool = Vec::new();
    for entry in typed.authorized_proxy_pool {
        let (tag, raw) = entry.split_once('|').unwrap_or(("ALL", &entry));
        if tag.trim().is_empty()
            || tag.len() > 128
            || tag.chars().any(char::is_control)
            || raw.trim().is_empty()
        {
            return Err("configuration_proxy_tag_invalid".into());
        }
        pool.push(json!({"tag":tag.trim(),"proxy":save_proxy(db,raw)?}));
    }
    Ok(
        json!({"proxy":save_proxy(db,&typed.proxy_url)?,"noProxy":typed.no_proxy,"agentProxyEnabled":typed.agent_proxy_enabled,"pool":pool}),
    )
}
fn materialize(db: &Connection, section: &str, descriptor: &Value) -> Result<Value, String> {
    if section == "securityRules" {
        return self::descriptor(db, section, descriptor);
    }
    let typed: TransportDescriptor = serde_json::from_value(descriptor.clone())
        .map_err(|_| "configuration_transport_descriptor_invalid")?;
    if typed.no_proxy.len() > 16 * 1024
        || typed.no_proxy.chars().any(char::is_control)
        || typed.pool.len() > 128
    {
        return Err("configuration_transport_descriptor_invalid".into());
    }
    let mut pool = Vec::new();
    for item in typed.pool {
        if item.tag.is_empty() || item.tag.chars().any(char::is_control) || item.proxy.is_null() {
            return Err("configuration_transport_descriptor_invalid".into());
        }
        pool.push(format!(
            "{}|{}",
            item.tag,
            materialize_proxy(db, &item.proxy)?
        ));
    }
    Ok(
        json!({"proxyUrl":materialize_proxy(db,&typed.proxy)?,"noProxy":typed.no_proxy,"agentProxyEnabled":typed.agent_proxy_enabled,"authorizedProxyPool":pool}),
    )
}

/// Resolve an immutable version independently of today's profile binding.
/// This is a data lookup, never an execution or credential grant.
fn version_descriptor(db: &Connection, reference: &PolicyReference) -> Result<Value, String> {
    keys(&reference.kind)?;
    if reference.policy_id.is_empty() || reference.revision <= 0 || !exists(db)? {
        return Err("configuration_runtime_reference_invalid".into());
    }
    let (kind, text, digest): (String, String, String) = db.query_row(
        "SELECT kind,descriptor_json,digest FROM configuration_runtime_versions WHERE owner_id=?1 AND revision=?2",
        params![reference.policy_id, reference.revision], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).map_err(|_| "configuration_runtime_reference_invalid")?;
    if kind != reference.kind || digest != format!("{:x}", Sha256::digest(text.as_bytes())) {
        return Err("configuration_runtime_reference_invalid".into());
    }
    serde_json::from_str(&text).map_err(|_| "configuration_runtime_reference_invalid".into())
}

fn endpoint_or_empty(value: &Value) -> String {
    value
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .and_then(|s| proxy_endpoint(s).ok())
        .unwrap_or_default()
}

/// Asset snapshots must never persist proxy URL userinfo, including values
/// from a legacy profile that has not been explicitly adopted. No DDL or
/// credential migration happens here. Live requests retain their in-memory
/// transport, while an adopted snapshot records its exact immutable reference.
pub fn redact_asset_snapshot(db: &Connection, settings: &mut Value) -> Result<(), String> {
    let reference = policies::reference(settings, "network")?;
    let descriptor = if let Some(reference) = &reference {
        if reference.kind != "network" {
            return Err("configuration_runtime_reference_invalid".into());
        }
        let descriptor = version_descriptor(db, reference)?;
        // Validate credential references as well, rather than storing a broken
        // snapshot which could later resurrect legacy JSON.
        materialize(db, "network", &descriptor)?;
        descriptor
    } else {
        Value::Null
    };
    let proxy = endpoint_or_empty(settings.get("proxyUrl").unwrap_or(&Value::Null));
    let pool: Vec<Value> = settings
        .get("authorizedProxyPool")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let raw = entry.as_str()?;
            let (tag, url) = raw.split_once('|').unwrap_or(("ALL", raw));
            let endpoint = proxy_endpoint(url.trim()).ok()?;
            Some(Value::String(format!("{}|{}", tag.trim(), endpoint)))
        })
        .collect();
    let object = settings
        .as_object_mut()
        .ok_or("configuration_profile_settings_invalid")?;
    object.insert("proxyUrl".into(), Value::String(proxy));
    object.insert("authorizedProxyPool".into(), Value::Array(pool));
    object.insert(
        "_businessTransport".into(),
        json!({
            "schemaVersion":1,"source":if reference.is_some(){"owner"}else{"legacy"},
            "reference":reference,"descriptor":descriptor
        }),
    );
    Ok(())
}

/// An explicitly resumed asset task repairs transport from the current profile,
/// as before. Record that current reference instead of retaining the old one.
/// This does not change frozen seeds, probe/collection/content policy or models.
pub fn repair_asset_transport(settings: &mut Value, current: &Value) -> Result<(), String> {
    let reference = policies::reference(current, "network")?;
    let object = settings
        .as_object_mut()
        .ok_or("configuration_profile_settings_invalid")?;
    for key in keys("network")? {
        object.remove(*key);
        if let Some(value) = current.get(*key) {
            object.insert((*key).into(), value.clone());
        }
    }
    object.remove("_businessTransport");
    let refs = object
        .entry("_configurationRefs")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("configuration_policy_reference_invalid")?;
    refs.remove("network");
    if let Some(reference) = reference {
        refs.insert(
            "network".into(),
            serde_json::to_value(reference)
                .map_err(|_| "configuration_policy_reference_invalid")?,
        );
    }
    for field in [
        "_configurationCategorySelections",
        "_configurationLegacyFallbacks",
    ] {
        if !object.contains_key(field) && current.get(field).is_none() {
            continue;
        }
        let map = object
            .entry(field)
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or("configuration_policy_reference_invalid")?;
        map.remove("network");
        if let Some(value) = current
            .get(field)
            .and_then(Value::as_object)
            .and_then(|map| map.get("network"))
        {
            map.insert("network".into(), value.clone());
        }
    }
    Ok(())
}

/// Materialize only an adopted immutable transport snapshot in memory. Legacy
/// snapshots require the explicit current-profile repair above; sanitized URLs
/// are never mistaken for a usable saved credential.
pub fn materialize_asset_transport(db: &Connection, settings: &mut Value) -> Result<(), String> {
    let marker = settings
        .get("_businessTransport")
        .ok_or("configuration_transport_snapshot_missing")?;
    if marker.get("schemaVersion").and_then(Value::as_u64) != Some(1)
        || marker.get("source").and_then(Value::as_str) != Some("owner")
    {
        return Err("configuration_transport_snapshot_requires_current_profile".into());
    }
    let reference: PolicyReference = serde_json::from_value(
        marker
            .get("reference")
            .cloned()
            .ok_or("configuration_runtime_reference_invalid")?,
    )
    .map_err(|_| "configuration_runtime_reference_invalid")?;
    if reference.kind != "network"
        || policies::reference(settings, "network")?.as_ref() != Some(&reference)
    {
        return Err("configuration_runtime_reference_invalid".into());
    }
    let descriptor = version_descriptor(db, &reference)?;
    if marker.get("descriptor") != Some(&descriptor) {
        return Err("configuration_runtime_reference_invalid".into());
    }
    let values = materialize(db, "network", &descriptor)?;
    let object = settings
        .as_object_mut()
        .ok_or("configuration_profile_settings_invalid")?;
    for key in keys("network")? {
        object.remove(*key);
    }
    object.extend(
        values
            .as_object()
            .ok_or("configuration_runtime_reference_invalid")?
            .clone(),
    );
    Ok(())
}

pub fn overlay(
    db: &Connection,
    profile: i64,
    settings: &mut Map<String, Value>,
    refs: &mut Map<String, Value>,
) -> Result<(), String> {
    if !exists(db)? {
        return Ok(());
    }
    let mut statement=db.prepare("SELECT b.section,b.owner_id,b.revision,v.kind,v.descriptor_json,v.digest FROM configuration_runtime_bindings b LEFT JOIN configuration_runtime_versions v ON v.owner_id=b.owner_id AND v.revision=b.revision WHERE b.profile_id=?1 ORDER BY b.section").map_err(|_|"configuration_runtime_read_failed")?;
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
        .map_err(|_| "configuration_runtime_read_failed")?;
    for row in rows {
        let (section, id, revision, kind, text, digest) =
            row.map_err(|_| "configuration_runtime_read_failed")?;
        let text = text.ok_or("configuration_runtime_reference_invalid")?;
        if kind.as_deref() != Some(&section)
            || revision <= 0
            || digest != Some(format!("{:x}", Sha256::digest(text.as_bytes())))
        {
            return Err("configuration_runtime_reference_invalid".into());
        }
        let descriptor: Value =
            serde_json::from_str(&text).map_err(|_| "configuration_runtime_reference_invalid")?;
        let values = materialize(db, &section, &descriptor)?;
        for key in keys(&section)? {
            settings.remove(*key);
        }
        settings.extend(
            values
                .as_object()
                .ok_or("configuration_runtime_reference_invalid")?
                .clone(),
        );
        refs.insert(
            section.clone(),
            serde_json::to_value(PolicyReference {
                kind: section,
                policy_id: id,
                revision,
            })
            .map_err(|_| "configuration_runtime_reference_invalid")?,
        );
    }
    Ok(())
}

/// Caller owns the IMMEDIATE transaction; schema and credentials roll back too.
pub fn save(
    db: &Connection,
    profile: i64,
    section: &str,
    incoming: &Value,
    expected: &Value,
    reference: Option<&PolicyReference>,
    adopt: bool,
) -> Result<PolicyReference, String> {
    keys(section)?;
    if db.is_autocommit() {
        return Err("configuration_runtime_transaction_required".into());
    }
    let current = policies::read_profile(db, profile)?;
    let previous = policies::reference(&current, section)?;
    if !crate::configuration_json::equivalent(&project(&current, section)?, expected)
        || previous.as_ref() != reference
        || adopt != previous.is_none()
    {
        return Err("configuration_runtime_conflict".into());
    }
    if !exists(db)? {
        db.execute_batch(SCHEMA)
            .map_err(|_| "configuration_runtime_storage_unavailable")?;
    }
    let descriptor = descriptor(db, section, incoming)?.to_string();
    let result = PolicyReference {
        policy_id: previous
            .as_ref()
            .map(|r| r.policy_id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        revision: match previous.as_ref() {
            Some(r) => r
                .revision
                .checked_add(1)
                .ok_or("configuration_runtime_revision_exhausted")?,
            None => 1,
        },
        kind: section.into(),
    };
    db.execute("INSERT INTO configuration_runtime_versions(owner_id,revision,kind,descriptor_json,digest) VALUES(?1,?2,?3,?4,?5)",params![result.policy_id,result.revision,section,descriptor,format!("{:x}",Sha256::digest(descriptor.as_bytes()))]).map_err(|_|"configuration_runtime_write_failed")?;
    if let Some(old) = previous {
        if db.execute("UPDATE configuration_runtime_bindings SET revision=?1 WHERE profile_id=?2 AND section=?3 AND owner_id=?4 AND revision=?5",params![result.revision,profile,section,old.policy_id,old.revision]).map_err(|_|"configuration_runtime_write_failed")?!=1 {return Err("configuration_runtime_conflict".into());}
    } else {
        db.execute("INSERT INTO configuration_runtime_bindings(profile_id,section,owner_id,revision) VALUES(?1,?2,?3,?4)",params![profile,section,result.policy_id,result.revision]).map_err(|_|"configuration_runtime_write_failed")?;
    }
    let observed = policies::read_profile(db, profile)?;
    if policies::reference(&observed, section)?.as_ref() != Some(&result) {
        return Err("configuration_runtime_postcondition".into());
    }
    Ok(result)
}
pub fn release(db: &Connection, profile: i64) -> Result<(), String> {
    if exists(db)? {
        db.execute(
            "DELETE FROM configuration_runtime_bindings WHERE profile_id=?1",
            [profile],
        )
        .map_err(|_| "configuration_runtime_write_failed")?;
    }
    Ok(())
}

pub fn independent_references(
    db: &Connection,
    section: &str,
) -> Result<Vec<PolicyReference>, String> {
    keys(section)?;
    if !exists(db)? {
        return Ok(vec![]);
    }
    let mut statement=db.prepare("SELECT owner_id,revision FROM configuration_runtime_versions WHERE kind=?1 ORDER BY owner_id,revision DESC").map_err(|_|"configuration_runtime_read_failed")?;
    let result = statement
        .query_map([section], |r| {
            Ok(PolicyReference {
                policy_id: r.get(0)?,
                revision: r.get(1)?,
                kind: section.into(),
            })
        })
        .map_err(|_| "configuration_runtime_read_failed")?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "configuration_runtime_read_failed".into());
    result
}

pub fn independent_read(db: &Connection, reference: &PolicyReference) -> Result<Value, String> {
    let descriptor = version_descriptor(db, reference)?;
    materialize(db, &reference.kind, &descriptor)
}

pub fn independent_append(
    db: &Connection,
    section: &str,
    settings: &Value,
    previous: Option<&PolicyReference>,
) -> Result<PolicyReference, String> {
    keys(section)?;
    if db.is_autocommit() {
        return Err("configuration_runtime_transaction_required".into());
    }
    if let Some(previous) = previous {
        if previous.kind != section {
            return Err("configuration_runtime_reference_invalid".into());
        }
        independent_read(db, previous)?;
        let head: i64 = db
            .query_row(
                "SELECT max(revision) FROM configuration_runtime_versions WHERE owner_id=?1",
                [&previous.policy_id],
                |r| r.get(0),
            )
            .map_err(|_| "configuration_runtime_read_failed")?;
        if head != previous.revision {
            return Err("configuration_category_head_conflict".into());
        }
    }
    if !exists(db)? {
        db.execute_batch(SCHEMA)
            .map_err(|_| "configuration_runtime_storage_unavailable")?;
    }
    let text = descriptor(db, section, settings)?.to_string();
    let reference = PolicyReference {
        policy_id: previous
            .map(|r| r.policy_id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        revision: match previous {
            Some(r) => r
                .revision
                .checked_add(1)
                .ok_or("configuration_runtime_revision_exhausted")?,
            None => 1,
        },
        kind: section.into(),
    };
    if db.execute("INSERT INTO configuration_runtime_versions(owner_id,revision,kind,descriptor_json,digest) VALUES(?1,?2,?3,?4,?5)",params![reference.policy_id,reference.revision,section,text,format!("{:x}",Sha256::digest(text.as_bytes()))]).map_err(|_|"configuration_runtime_write_failed")?!=1 {return Err("configuration_runtime_write_failed".into());}
    independent_read(db, &reference)?;
    Ok(reference)
}

#[cfg(test)]
#[path = "runtime_owners_tests.rs"]
mod tests;
