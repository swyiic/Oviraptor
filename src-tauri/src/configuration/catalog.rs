//! Category defaults are independent of legacy profile bindings. Reads never
//! create storage or migrate data. A write changes one category in one caller-
//! owned transaction, and its immutable receipt contains no submitted settings.
use crate::{
    configuration_connections as connections, configuration_json,
    configuration_policies::{self as policies, PolicyReference},
    configuration_runtime_owners as runtime,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const SECTIONS: [&str; 8] = [
    "models",
    "assetApis",
    "collection",
    "probe",
    "rules",
    "agent",
    "network",
    "securityRules",
];
const SCHEMA: &str = r#"
CREATE TABLE configuration_category_catalog(kind TEXT NOT NULL,owner_id TEXT NOT NULL,name TEXT NOT NULL,PRIMARY KEY(kind,owner_id));
CREATE TABLE configuration_category_defaults(kind TEXT PRIMARY KEY,selection_revision INTEGER NOT NULL CHECK(selection_revision>0),reference_json TEXT NOT NULL CHECK(json_valid(reference_json)));
CREATE TABLE configuration_category_receipts(request_id TEXT PRIMARY KEY,arguments_digest TEXT NOT NULL,receipt_json TEXT NOT NULL CHECK(json_valid(receipt_json)));
CREATE TRIGGER configuration_category_catalog_no_update BEFORE UPDATE ON configuration_category_catalog BEGIN SELECT RAISE(ABORT,'configuration_category_catalog_immutable'); END;
CREATE TRIGGER configuration_category_catalog_no_delete BEFORE DELETE ON configuration_category_catalog BEGIN SELECT RAISE(ABORT,'configuration_category_catalog_immutable'); END;
CREATE TRIGGER configuration_category_catalog_no_replace BEFORE INSERT ON configuration_category_catalog WHEN EXISTS(SELECT 1 FROM configuration_category_catalog WHERE kind=NEW.kind AND owner_id=NEW.owner_id) BEGIN SELECT RAISE(ABORT,'configuration_category_catalog_immutable'); END;
CREATE TRIGGER configuration_category_receipts_no_update BEFORE UPDATE ON configuration_category_receipts BEGIN SELECT RAISE(ABORT,'configuration_category_receipt_immutable'); END;
CREATE TRIGGER configuration_category_receipts_no_delete BEFORE DELETE ON configuration_category_receipts BEGIN SELECT RAISE(ABORT,'configuration_category_receipt_immutable'); END;
CREATE TRIGGER configuration_category_receipts_no_replace BEFORE INSERT ON configuration_category_receipts WHEN EXISTS(SELECT 1 FROM configuration_category_receipts WHERE request_id=NEW.request_id) BEGIN SELECT RAISE(ABORT,'configuration_category_receipt_immutable'); END;
"#;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DefaultSelection {
    pub revision: i64,
    pub reference: Option<PolicyReference>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SaveMode {
    Create,
    Revise,
    Fork,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveCategoryInput {
    pub request_id: String,
    pub section: String,
    pub mode: SaveMode,
    pub name: String,
    pub settings: Value,
    pub expected_settings: Value,
    pub expected_reference: Option<PolicyReference>,
    pub expected_default: Option<DefaultSelection>,
    pub make_default: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectCategoryInput {
    pub request_id: String,
    pub section: String,
    pub reference: Option<PolicyReference>,
    pub expected_default: Option<DefaultSelection>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CategoryReceipt {
    pub schema_version: u32,
    pub request_id: String,
    pub section: String,
    pub action: String,
    pub reference: Option<PolicyReference>,
    pub default_selection: Option<DefaultSelection>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryWriteResult {
    pub receipt: CategoryReceipt,
    pub current_default: Option<DefaultSelection>,
    /// Whether today's default selection still matches this historical ack.
    pub current_state: String,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEntry {
    pub name: String,
    pub reference: PolicyReference,
    pub versions: Vec<PolicyReference>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryView {
    pub section: String,
    pub source: String,
    pub legacy_profile_id: Option<i64>,
    pub default_selection: Option<DefaultSelection>,
    pub reference: Option<PolicyReference>,
    pub settings: Value,
    pub builtin_settings: Value,
    pub entries: Vec<CatalogEntry>,
    pub legacy_available: bool,
}

fn section(section: &str) -> Result<(), String> {
    if SECTIONS.contains(&section) {
        Ok(())
    } else {
        Err("configuration_category_kind_invalid".into())
    }
}
fn storage(db: &Connection) -> Result<bool, String> {
    let count:i64=db.query_row("SELECT count(*) FROM sqlite_schema WHERE type='table' AND name IN ('configuration_category_catalog','configuration_category_defaults','configuration_category_receipts')",[],|r|r.get(0)).map_err(|_|"configuration_category_storage_unavailable")?;
    if count == 0 {
        return Ok(false);
    }
    if count != 3 {
        return Err("configuration_category_storage_incomplete".into());
    }
    let guards:i64=db.query_row("SELECT count(*) FROM sqlite_schema WHERE type='trigger' AND name IN ('configuration_category_catalog_no_update','configuration_category_catalog_no_delete','configuration_category_catalog_no_replace','configuration_category_receipts_no_update','configuration_category_receipts_no_delete','configuration_category_receipts_no_replace')",[],|r|r.get(0)).map_err(|_|"configuration_category_storage_unavailable")?;
    if guards != 6 {
        return Err("configuration_category_storage_incomplete".into());
    }
    Ok(true)
}
fn ensure(db: &Connection) -> Result<(), String> {
    if !storage(db)? {
        db.execute_batch(SCHEMA)
            .map_err(|_| "configuration_category_storage_unavailable")?;
    }
    Ok(())
}
fn request_id(id: &str) -> Result<(), String> {
    if uuid::Uuid::parse_str(id).is_ok_and(|u| u.to_string() == id) {
        Ok(())
    } else {
        Err("configuration_category_request_invalid".into())
    }
}
pub fn keys(kind: &str) -> Result<&'static [&'static str], String> {
    section(kind)?;
    match kind {
        "models" | "assetApis" => connections::keys(kind),
        "network" | "securityRules" => runtime::keys(kind),
        _ => policies::policy_keys(kind),
    }
}
pub fn builtin_settings(kind: &str) -> Result<Value, String> {
    section(kind)?;
    Ok(match kind {
        "models" => {
            json!({"modelProfiles":[],"activeModelProfileId":"","modelName":"","modelDeployment":"cloud","modelApiBase":"","modelApiKey":"","localApiKey":""})
        }
        "assetApis" => json!({"fofaEmail":"","fofaKey":"","configPath":""}),
        "collection" => {
            json!({"fofaProfile":"professional","pageSize":500,"maxPages":0,"interval":6,"collectionTimeout":45,"fullHistory":false,"enableCidr24":false,"collectionCache":true,"precisionFirst":true})
        }
        "probe" => {
            json!({"runRefine":true,"runProbe":true,"includeOther":true,"includeWeak":false,"priorityRate":20,"otherRate":10,"workers":96,"probeTimeout":6,"probeTotalTimeout":8,"probeRetries":0,"maxBodyBytes":524288,"allowPrivate":false,"strictTls":false,"schemeFallback":true,"perHostInterval":1.5})
        }
        "rules" => {
            json!({"gamblingKeywords":["在线赌博","博彩平台","真人视讯","体育投注"],"pornKeywords":["色情网站","成人网站","成人视频","情色直播"],"negativeKeywords":["打击赌博","扫黄打非","反诈","公安","法院"],"contentThreshold":12,"replaceDefaultContentRules":false})
        }
        "agent" => {
            json!({"agentLocalFullPower":false,"agentPromptAuditMode":"off","agentFrontendPacketMode":"balanced","agentFrontendPacketBudgetKb":24,"agentQuickScore":30,"agentStandardScore":55,"agentDeepScore":80,"agentQuickTimeout":300,"agentStandardTimeout":480,"agentDeepTimeout":900,"agentQuickTokenLimit":200000,"agentStandardTokenLimit":400000,"agentDeepTokenLimit":800000,"agentQuickRequestLimit":8,"agentStandardRequestLimit":12,"agentDeepRequestLimit":16,"agentNoToolTurnLimit":6})
        }
        "network" => {
            json!({"proxyUrl":"","noProxy":"127.0.0.1,localhost","agentProxyEnabled":false,"authorizedProxyPool":[]})
        }
        "securityRules" => {
            json!({"nativeAnalyzers":{"semgrep":{"image":""},"codeql":{"image":""}}})
        }
        _ => unreachable!(),
    })
}
fn normalize(kind: &str, settings: &Value) -> Result<Value, String> {
    let input = settings
        .as_object()
        .ok_or("configuration_category_settings_invalid")?;
    let allowed = keys(kind)?;
    if input.keys().any(|k| !allowed.contains(&k.as_str())) {
        return Err("configuration_category_field_denied".into());
    }
    let mut value = builtin_settings(kind)?;
    value.as_object_mut().unwrap().extend(input.clone());
    Ok(value)
}
fn references(db: &Connection, kind: &str) -> Result<Vec<PolicyReference>, String> {
    match kind {
        "models" | "assetApis" => connections::independent_references(db, kind),
        "network" | "securityRules" => runtime::independent_references(db, kind),
        _ => policies::independent_references(db, kind),
    }
}
pub fn read_reference(db: &Connection, reference: &PolicyReference) -> Result<Value, String> {
    section(&reference.kind)?;
    let value = match reference.kind.as_str() {
        "models" | "assetApis" => connections::independent_read(db, reference),
        "network" | "securityRules" => runtime::independent_read(db, reference),
        _ => policies::independent_read(db, reference),
    }?;
    normalize(&reference.kind, &value)
}
fn selection(db: &Connection, kind: &str) -> Result<Option<DefaultSelection>, String> {
    section(kind)?;
    if !storage(db)? {
        return Ok(None);
    }
    let row:Option<(i64,String)>=db.query_row("SELECT selection_revision,reference_json FROM configuration_category_defaults WHERE kind=?1",[kind],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|"configuration_category_default_unavailable")?;
    row.map(|(revision, text)| {
        let reference: Option<PolicyReference> =
            serde_json::from_str(&text).map_err(|_| "configuration_category_default_invalid")?;
        if revision <= 0
            || reference
                .as_ref()
                .is_some_and(|r| r.kind != kind || r.revision <= 0 || r.policy_id.is_empty())
        {
            return Err("configuration_category_default_invalid".into());
        }
        Ok(DefaultSelection {
            revision,
            reference,
        })
    })
    .transpose()
}
fn legacy_id(db: &Connection) -> Result<Option<i64>, String> {
    let exists:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='config_profiles')",[],|r|r.get(0)).map_err(|_|"configuration_category_legacy_unavailable")?;
    if !exists {
        return Ok(None);
    }
    let id: Option<i64> = db
        .query_row(
            "SELECT id FROM config_profiles ORDER BY is_default DESC,id LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| "configuration_category_legacy_unavailable")?;
    Ok(id)
}
fn legacy_reference(
    db: &Connection,
    id: i64,
    kind: &str,
) -> Result<Option<PolicyReference>, String> {
    section(kind)?;
    let (table, column) = match kind {
        "models" | "assetApis" => ("configuration_connection_bindings", "owner_id"),
        "network" | "securityRules" => ("configuration_runtime_bindings", "owner_id"),
        _ => ("configuration_policy_bindings", "policy_id"),
    };
    let exists: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?1)",
            [table],
            |r| r.get(0),
        )
        .map_err(|_| "configuration_category_legacy_unavailable")?;
    if exists {
        let reference: Option<PolicyReference> = db
            .query_row(
                &format!(
                    "SELECT {column},revision FROM {table} WHERE profile_id=?1 AND section=?2"
                ),
                params![id, kind],
                |r| {
                    Ok(PolicyReference {
                        policy_id: r.get(0)?,
                        revision: r.get(1)?,
                        kind: kind.into(),
                    })
                },
            )
            .optional()
            .map_err(|_| "configuration_category_legacy_unavailable")?;
        Ok(reference)
    } else {
        Ok(None)
    }
}

fn legacy_category(db: &Connection, id: i64, kind: &str) -> Result<Value, String> {
    let raw: String = db
        .query_row(
            "SELECT settings_json FROM config_profiles WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .map_err(|_| "configuration_category_legacy_unavailable")?;
    let raw: Value =
        serde_json::from_str(&raw).map_err(|_| "configuration_category_legacy_invalid")?;
    if let Some(reference) = legacy_reference(db, id, kind)? {
        let mut value = read_reference(db, &reference)?;
        // A previously adopted policy may predate the six optional fields.
        // Preserve that category's historical values just as resolve does.
        let mut result = fallback_category(kind, &crate::db::normalize_settings(&raw))?;
        let original = match kind {
            "models" | "assetApis" => connections::independent_read(db, &reference),
            "network" | "securityRules" => runtime::independent_read(db, &reference),
            _ => policies::independent_read(db, &reference),
        }?;
        result.as_object_mut().unwrap().extend(
            original
                .as_object()
                .ok_or("configuration_category_legacy_invalid")?
                .clone(),
        );
        if kind == "assetApis" && original.get("fofaEmail").is_none() {
            value["fofaEmail"] = result.get("fofaEmail").cloned().unwrap_or(json!(""));
        }
        // Connections/runtime codecs return complete DTOs; use their
        // canonical defaults and aliases rather than stale raw caches.
        return Ok(
            if ["models", "assetApis", "network", "securityRules"].contains(&kind) {
                value
            } else {
                result
            },
        );
    }
    fallback_category(kind, &crate::db::normalize_settings(&raw))
}
fn fallback_category(kind: &str, fallback: &Value) -> Result<Value, String> {
    let mut value = builtin_settings(kind)?;
    for key in keys(kind)? {
        if let Some(item) = fallback.get(*key) {
            value[*key] = item.clone();
        }
    }
    Ok(value)
}
fn entries(db: &Connection, kind: &str) -> Result<Vec<CatalogEntry>, String> {
    let mut groups: BTreeMap<String, Vec<PolicyReference>> = BTreeMap::new();
    for reference in references(db, kind)? {
        groups
            .entry(reference.policy_id.clone())
            .or_default()
            .push(reference);
    }
    let has_storage = storage(db)?;
    groups
        .into_iter()
        .map(|(id, mut versions)| {
            versions.sort_by(|a, b| b.revision.cmp(&a.revision));
            let name: Option<String> = if has_storage {
                db.query_row(
                    "SELECT name FROM configuration_category_catalog WHERE kind=?1 AND owner_id=?2",
                    params![kind, id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|_| "configuration_category_catalog_unavailable")?
            } else {
                None
            };
            Ok(CatalogEntry {
                name: name.unwrap_or_else(|| {
                    format!("{kind} {}", id.chars().take(8).collect::<String>())
                }),
                reference: versions[0].clone(),
                versions,
            })
        })
        .collect()
}
pub fn read_category(
    db: &Connection,
    kind: &str,
    requested: Option<&PolicyReference>,
) -> Result<CategoryView, String> {
    if db.is_autocommit() {
        let tx = db
            .unchecked_transaction()
            .map_err(|_| "configuration_category_read_unavailable")?;
        let result = read_category(&tx, kind, requested);
        tx.rollback()
            .map_err(|_| "configuration_category_read_unavailable")?;
        return result;
    }
    section(kind)?;
    let default_selection = selection(db, kind)?;
    let legacy_id = legacy_id(db)?;
    let reference = requested
        .cloned()
        .or_else(|| default_selection.as_ref().and_then(|s| s.reference.clone()));
    if reference.as_ref().is_some_and(|r| r.kind != kind) {
        return Err("configuration_category_reference_invalid".into());
    }
    let (settings, source, legacy_profile_id) = if let Some(reference) = &reference {
        (read_reference(db, reference)?, "owner", None)
    } else if default_selection.is_none() && requested.is_none() && legacy_id.is_some() {
        (
            legacy_category(db, legacy_id.unwrap(), kind)?,
            "legacy-fallback",
            legacy_id,
        )
    } else {
        (builtin_settings(kind)?, "builtin", None)
    };
    Ok(CategoryView {
        section: kind.into(),
        source: source.into(),
        legacy_profile_id,
        default_selection,
        reference,
        settings,
        builtin_settings: builtin_settings(kind)?,
        entries: entries(db, kind)?,
        legacy_available: legacy_id.is_some(),
    })
}

/// Resolve the actual independent defaults. Only categories that have never
/// selected a global default consult their corresponding legacy binding. A
/// stale/broken legacy model therefore cannot block an independent model owner.
/// All reads share one snapshot and never create or migrate storage.
pub fn read_current_defaults(db: &Connection) -> Result<Value, String> {
    if db.is_autocommit() {
        let tx = db
            .unchecked_transaction()
            .map_err(|_| "configuration_category_read_unavailable")?;
        let result = read_current_defaults(&tx);
        tx.rollback()
            .map_err(|_| "configuration_category_read_unavailable")?;
        return result;
    }
    let selected = SECTIONS
        .into_iter()
        .map(|kind| Ok((kind, selection(db, kind)?)))
        .collect::<Result<Vec<_>, String>>()?;
    let legacy_id = if selected.iter().any(|(_, selected)| selected.is_none()) {
        legacy_id(db)?
    } else {
        None
    };
    // Preserve historical extras only while there are legacy fallback classes.
    // Trusted authority/provenance fields are reconstructed, never borrowed.
    let mut object = if let Some(id) = legacy_id {
        let raw: String = db
            .query_row(
                "SELECT settings_json FROM config_profiles WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(|_| "configuration_category_legacy_unavailable")?;
        let raw: Value =
            serde_json::from_str(&raw).map_err(|_| "configuration_category_legacy_invalid")?;
        crate::db::normalize_settings(&raw)
            .as_object()
            .cloned()
            .ok_or("configuration_category_legacy_invalid")?
    } else {
        Map::new()
    };
    for key in [
        "_configurationRefs",
        "_configurationCategorySelections",
        "_configurationLegacyFallbacks",
        "_configurationError",
        "_businessTransport",
    ] {
        object.remove(key);
    }
    let mut refs = Map::new();
    let mut selections = Map::new();
    let mut fallbacks = Map::new();
    for (kind, selected) in selected {
        let (settings, reference) = if let Some(selected) = selected {
            let settings = match &selected.reference {
                Some(reference) => read_reference(db, reference)?,
                None => builtin_settings(kind)?,
            };
            selections.insert(
                kind.into(),
                serde_json::to_value(&selected)
                    .map_err(|_| "configuration_category_default_invalid")?,
            );
            (settings, selected.reference)
        } else if let Some(id) = legacy_id {
            fallbacks.insert(kind.into(), json!(id));
            (
                legacy_category(db, id, kind)?,
                legacy_reference(db, id, kind)?,
            )
        } else {
            (builtin_settings(kind)?, None)
        };
        for key in keys(kind)? {
            object.remove(*key);
        }
        object.extend(
            settings
                .as_object()
                .ok_or("configuration_category_settings_invalid")?
                .clone(),
        );
        if let Some(reference) = reference {
            refs.insert(
                kind.into(),
                serde_json::to_value(reference)
                    .map_err(|_| "configuration_category_default_invalid")?,
            );
        }
    }
    if !refs.is_empty() {
        object.insert("_configurationRefs".into(), Value::Object(refs));
    }
    if !selections.is_empty() {
        object.insert(
            "_configurationCategorySelections".into(),
            Value::Object(selections),
        );
    }
    if !fallbacks.is_empty() {
        object.insert(
            "_configurationLegacyFallbacks".into(),
            Value::Object(fallbacks),
        );
    }
    Ok(Value::Object(object))
}

/// A current consumer opts in to precisely the classes it actually uses.
/// Unrelated owners and historical unknown fields never enter this runtime.
pub fn read_current_defaults_for(db: &Connection, kinds: &[&str]) -> Result<Value, String> {
    if db.is_autocommit() {
        let tx = db
            .unchecked_transaction()
            .map_err(|_| "configuration_category_read_unavailable")?;
        let result = read_current_defaults_for(&tx, kinds);
        tx.rollback()
            .map_err(|_| "configuration_category_read_unavailable")?;
        return result;
    }
    let mut object = Map::new();
    let mut refs = Map::new();
    let mut selections = Map::new();
    let mut fallbacks = Map::new();
    let mut fallback_id = None;
    let mut looked_up_legacy = false;
    for &kind in kinds {
        section(kind)?;
        let (settings, reference) = if let Some(selected) = selection(db, kind)? {
            let settings = match &selected.reference {
                Some(reference) => read_reference(db, reference)?,
                None => builtin_settings(kind)?,
            };
            selections.insert(
                kind.into(),
                serde_json::to_value(&selected)
                    .map_err(|_| "configuration_category_default_invalid")?,
            );
            (settings, selected.reference)
        } else {
            if !looked_up_legacy {
                fallback_id = legacy_id(db)?;
                looked_up_legacy = true;
            }
            if let Some(id) = fallback_id {
                fallbacks.insert(kind.into(), json!(id));
                (
                    legacy_category(db, id, kind)?,
                    legacy_reference(db, id, kind)?,
                )
            } else {
                (builtin_settings(kind)?, None)
            }
        };
        object.extend(
            settings
                .as_object()
                .ok_or("configuration_category_settings_invalid")?
                .clone(),
        );
        if let Some(reference) = reference {
            refs.insert(
                kind.into(),
                serde_json::to_value(reference)
                    .map_err(|_| "configuration_category_default_invalid")?,
            );
        }
    }
    if !refs.is_empty() {
        object.insert("_configurationRefs".into(), Value::Object(refs));
    }
    if !selections.is_empty() {
        object.insert(
            "_configurationCategorySelections".into(),
            Value::Object(selections),
        );
    }
    if !fallbacks.is_empty() {
        object.insert(
            "_configurationLegacyFallbacks".into(),
            Value::Object(fallbacks),
        );
    }
    Ok(Value::Object(object))
}

/// Compatibility defaults are supplied explicitly by the consumer. Selected
/// category references replace only their own fields and reference, regardless
/// of later legacy global-profile changes. No read creates storage.
pub fn read_defaults(db: &Connection, legacy: &Value) -> Result<Value, String> {
    if db.is_autocommit() {
        let tx = db
            .unchecked_transaction()
            .map_err(|_| "configuration_category_read_unavailable")?;
        let result = read_defaults(&tx, legacy);
        tx.rollback()
            .map_err(|_| "configuration_category_read_unavailable")?;
        return result;
    }
    let mut object = Map::new();
    for kind in SECTIONS {
        object.extend(builtin_settings(kind)?.as_object().unwrap().clone());
    }
    if let Some(legacy) = legacy.as_object() {
        object.extend(legacy.clone());
    }
    object.remove("_configurationCategorySelections");
    let mut refs = object
        .remove("_configurationRefs")
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    let mut selections = Map::new();
    for kind in SECTIONS {
        if let Some(selected) = selection(db, kind)? {
            let settings = match &selected.reference {
                Some(reference) => read_reference(db, reference)?,
                None => builtin_settings(kind)?,
            };
            for key in keys(kind)? {
                object.remove(*key);
            }
            object.extend(settings.as_object().unwrap().clone());
            refs.remove(kind);
            if let Some(reference) = &selected.reference {
                refs.insert(
                    kind.into(),
                    serde_json::to_value(reference)
                        .map_err(|_| "configuration_category_default_invalid")?,
                );
            }
            selections.insert(
                kind.into(),
                serde_json::to_value(selected)
                    .map_err(|_| "configuration_category_default_invalid")?,
            );
        }
    }
    if !refs.is_empty() {
        object.insert("_configurationRefs".into(), Value::Object(refs));
    }
    if !selections.is_empty() {
        object.insert(
            "_configurationCategorySelections".into(),
            Value::Object(selections),
        );
    }
    Ok(Value::Object(object))
}

fn result(db: &Connection, receipt: CategoryReceipt) -> Result<CategoryWriteResult, String> {
    let current_default = selection(db, &receipt.section)?;
    let current_state = if current_default == receipt.default_selection {
        "unchanged"
    } else {
        "modified"
    }
    .into();
    Ok(CategoryWriteResult {
        receipt,
        current_default,
        current_state,
    })
}
fn lookup(db: &Connection, id: &str) -> Result<Option<(String, CategoryReceipt)>, String> {
    if !storage(db)? {
        return Ok(None);
    }
    let row:Option<(String,String)>=db.query_row("SELECT arguments_digest,receipt_json FROM configuration_category_receipts WHERE request_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|"configuration_category_receipt_unavailable")?;
    row.map(|(digest, text)| {
        let receipt: CategoryReceipt =
            serde_json::from_str(&text).map_err(|_| "configuration_category_receipt_invalid")?;
        if receipt.schema_version != 1
            || receipt.request_id != id
            || !SECTIONS.contains(&receipt.section.as_str())
            || !["create", "revise", "fork", "select"].contains(&receipt.action.as_str())
            || receipt
                .reference
                .as_ref()
                .is_some_and(|r| r.kind != receipt.section)
        {
            return Err("configuration_category_receipt_invalid".into());
        }
        if let Some(reference) = &receipt.reference {
            read_reference(db, reference)?;
        }
        Ok((digest, receipt))
    })
    .transpose()
}
pub fn read_receipt(db: &Connection, id: &str) -> Result<Option<CategoryWriteResult>, String> {
    request_id(id)?;
    if db.is_autocommit() {
        let tx = db
            .unchecked_transaction()
            .map_err(|_| "configuration_category_read_unavailable")?;
        let value = read_receipt(&tx, id);
        tx.rollback()
            .map_err(|_| "configuration_category_read_unavailable")?;
        return value;
    }
    lookup(db, id)?
        .map(|(_, receipt)| result(db, receipt))
        .transpose()
}
fn publish_default(
    db: &Connection,
    kind: &str,
    reference: Option<&PolicyReference>,
    expected: Option<&DefaultSelection>,
) -> Result<DefaultSelection, String> {
    if selection(db, kind)?.as_ref() != expected {
        return Err("configuration_category_default_conflict".into());
    }
    let revision = match expected {
        Some(s) => s
            .revision
            .checked_add(1)
            .ok_or("configuration_category_revision_exhausted")?,
        None => 1,
    };
    let text =
        serde_json::to_string(&reference).map_err(|_| "configuration_category_default_invalid")?;
    let count=if let Some(expected)=expected {db.execute("UPDATE configuration_category_defaults SET selection_revision=?1,reference_json=?2 WHERE kind=?3 AND selection_revision=?4",params![revision,text,kind,expected.revision])}else{db.execute("INSERT INTO configuration_category_defaults(kind,selection_revision,reference_json) VALUES(?1,?2,?3)",params![kind,revision,text])}.map_err(|_|"configuration_category_default_write_failed")?;
    let observed = DefaultSelection {
        revision,
        reference: reference.cloned(),
    };
    if count != 1 || selection(db, kind)?.as_ref() != Some(&observed) {
        return Err("configuration_category_default_postcondition".into());
    }
    Ok(observed)
}
fn record(
    db: &Connection,
    digest: &str,
    receipt: CategoryReceipt,
) -> Result<CategoryWriteResult, String> {
    let text =
        serde_json::to_string(&receipt).map_err(|_| "configuration_category_receipt_invalid")?;
    if db.execute("INSERT INTO configuration_category_receipts(request_id,arguments_digest,receipt_json) VALUES(?1,?2,?3)",params![receipt.request_id,digest,text]).map_err(|_|"configuration_category_receipt_write_failed")?!=1 || lookup(db,&receipt.request_id)?.map(|(_,r)|r)!=Some(receipt.clone()) {return Err("configuration_category_receipt_postcondition".into());}
    result(db, receipt)
}
fn replay(db: &Connection, id: &str, digest: &str) -> Result<Option<CategoryWriteResult>, String> {
    lookup(db, id)?
        .map(|(previous, receipt)| {
            if previous != digest {
                return Err("configuration_category_request_conflict".into());
            }
            result(db, receipt)
        })
        .transpose()
}
pub fn save_on(db: &Connection, input: &SaveCategoryInput) -> Result<CategoryWriteResult, String> {
    if db.is_autocommit() {
        return Err("configuration_category_transaction_required".into());
    }
    section(&input.section)?;
    request_id(&input.request_id)?;
    if input.name.len() > 512
        || input.name.chars().any(char::is_control)
        || serde_json::to_vec(input)
            .map_err(|_| "configuration_category_input_invalid")?
            .len()
            > 2 * 1024 * 1024
    {
        return Err("configuration_category_input_invalid".into());
    }
    let digest = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&json!({"action":"save","input":input}))
                .map_err(|_| "configuration_category_input_invalid")?
        )
    );
    if let Some(result) = replay(db, &input.request_id, &digest)? {
        return Ok(result);
    }
    if selection(db, &input.section)? != input.expected_default {
        return Err("configuration_category_default_conflict".into());
    }
    let (base, previous, action) = match input.mode {
        SaveMode::Create => {
            if input.expected_reference.is_some() {
                return Err("configuration_category_reference_invalid".into());
            }
            (
                read_category(db, &input.section, None)?.settings,
                None,
                "create",
            )
        }
        SaveMode::Fork | SaveMode::Revise => {
            let previous = input
                .expected_reference
                .as_ref()
                .ok_or("configuration_category_reference_required")?;
            if previous.kind != input.section {
                return Err("configuration_category_reference_invalid".into());
            }
            (
                read_reference(db, previous)?,
                if matches!(input.mode, SaveMode::Revise) {
                    Some(previous)
                } else {
                    None
                },
                if matches!(input.mode, SaveMode::Revise) {
                    "revise"
                } else {
                    "fork"
                },
            )
        }
    };
    if !configuration_json::equivalent(&base, &input.expected_settings) {
        return Err("configuration_category_settings_conflict".into());
    }
    let settings = normalize(&input.section, &input.settings)?;
    ensure(db)?;
    let reference = match input.section.as_str() {
        "models" | "assetApis" => {
            connections::independent_append(db, &input.section, &settings, previous)
        }
        "network" | "securityRules" => {
            runtime::independent_append(db, &input.section, &settings, previous)
        }
        _ => policies::independent_append(db, &input.section, &settings, previous),
    }?;
    let existing: Option<String> = db
        .query_row(
            "SELECT name FROM configuration_category_catalog WHERE kind=?1 AND owner_id=?2",
            params![input.section, reference.policy_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| "configuration_category_catalog_unavailable")?;
    if existing.is_none() {
        let name = if input.name.trim().is_empty() {
            input.section.as_str()
        } else {
            input.name.trim()
        };
        if db
            .execute(
                "INSERT INTO configuration_category_catalog(kind,owner_id,name) VALUES(?1,?2,?3)",
                params![input.section, reference.policy_id, name],
            )
            .map_err(|_| "configuration_category_catalog_write_failed")?
            != 1
        {
            return Err("configuration_category_catalog_write_failed".into());
        }
    } else if !input.name.trim().is_empty() && existing.as_deref() != Some(input.name.trim()) {
        return Err("configuration_category_name_immutable_fork_instead".into());
    }
    let default_selection = if input.make_default {
        Some(publish_default(
            db,
            &input.section,
            Some(&reference),
            input.expected_default.as_ref(),
        )?)
    } else {
        selection(db, &input.section)?
    };
    record(
        db,
        &digest,
        CategoryReceipt {
            schema_version: 1,
            request_id: input.request_id.clone(),
            section: input.section.clone(),
            action: action.into(),
            reference: Some(reference),
            default_selection,
        },
    )
}
pub fn save(db: &mut Connection, input: SaveCategoryInput) -> Result<CategoryWriteResult, String> {
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| "configuration_category_transaction_unavailable")?;
    let result = save_on(&tx, &input)?;
    tx.commit()
        .map_err(|_| "configuration_category_commit_unconfirmed_retry_same_request")?;
    Ok(result)
}
pub fn select_on(
    db: &Connection,
    input: &SelectCategoryInput,
) -> Result<CategoryWriteResult, String> {
    if db.is_autocommit() {
        return Err("configuration_category_transaction_required".into());
    }
    section(&input.section)?;
    request_id(&input.request_id)?;
    let digest = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&json!({"action":"select","input":input}))
                .map_err(|_| "configuration_category_input_invalid")?
        )
    );
    if let Some(result) = replay(db, &input.request_id, &digest)? {
        return Ok(result);
    }
    if let Some(reference) = &input.reference {
        if reference.kind != input.section {
            return Err("configuration_category_reference_invalid".into());
        }
        read_reference(db, reference)?;
    }
    ensure(db)?;
    let default_selection = Some(publish_default(
        db,
        &input.section,
        input.reference.as_ref(),
        input.expected_default.as_ref(),
    )?);
    record(
        db,
        &digest,
        CategoryReceipt {
            schema_version: 1,
            request_id: input.request_id.clone(),
            section: input.section.clone(),
            action: "select".into(),
            reference: input.reference.clone(),
            default_selection,
        },
    )
}
pub fn select(
    db: &mut Connection,
    input: SelectCategoryInput,
) -> Result<CategoryWriteResult, String> {
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| "configuration_category_transaction_unavailable")?;
    let result = select_on(&tx, &input)?;
    tx.commit()
        .map_err(|_| "configuration_category_commit_unconfirmed_retry_same_request")?;
    Ok(result)
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;
