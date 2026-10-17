//! Application-owned presentation state. No assets, queries, paths or credentials.
//! Bootstrap alone initializes this schema after proving the original store and
//! taking its exclusive controller. Ordinary reads never initialize or repair it.
use crate::asset_service::service_instance_on;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

const MAX_REVISION: i64 = 9_007_199_254_740_991;
const TABLE:&str="CREATE TABLE asset_service_preferences(singleton INTEGER PRIMARY KEY CHECK(singleton=1),instance_id TEXT NOT NULL REFERENCES asset_service_store(instance_id),revision INTEGER NOT NULL CHECK(revision>=0 AND revision<=9007199254740991),preferences_json TEXT NOT NULL CHECK(json_valid(preferences_json)))";
const UPDATE_TRIGGER:&str="CREATE TRIGGER asset_service_preferences_update BEFORE UPDATE ON asset_service_preferences WHEN NEW.singleton<>OLD.singleton OR NEW.instance_id<>OLD.instance_id OR NEW.revision<>OLD.revision+1 BEGIN SELECT RAISE(ABORT,'asset preferences identity or revision changed'); END";
const DELETE_TRIGGER:&str="CREATE TRIGGER asset_service_preferences_delete BEFORE DELETE ON asset_service_preferences BEGIN SELECT RAISE(ABORT,'asset preferences singleton immutable'); END";
const COLUMNS: &[&str] = &[
    "projectName",
    "assetKey",
    "company",
    "reviewTier",
    "host",
    "link",
    "ip",
    "port",
    "protocol",
    "domain",
    "title",
    "statusCode",
    "probeOutcome",
    "hasLogin",
    "hasRegister",
    "entrySeen",
    "entryOpenedAt",
    "probeEntryState",
    "contentCategory",
    "score",
    "decision",
    "note",
    "ownershipStatus",
    "ownershipConfidence",
    "authorizationStatus",
    "exposureEligible",
    "ownershipReason",
    "sentinelStatus",
    "sentinelScanCount",
    "sentinelSentAt",
    "firstSeen",
    "lastSeen",
    "lastAlive",
    "projectFirstSeen",
    "projectLastSeen",
    "lastRunId",
    "isDeleted",
    "deletedAt",
];

fn error(code: &str) -> String {
    format!("asset_preferences_{code}")
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Preferences {
    pub(super) columns: Option<Vec<String>>,
    pub(super) column_widths: Option<Vec<ColumnWidth>>,
    pub(super) portal_columns_v1: Option<bool>,
    pub(super) agent_scan_mode: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ColumnWidth {
    pub(super) column: String,
    pub(super) width: f64,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct View {
    schema_version: u32,
    revision: i64,
    preferences: Preferences,
}
impl Preferences {
    fn validate(&self) -> Result<(), String> {
        if let Some(columns) = &self.columns {
            let mut seen = HashSet::new();
            if columns.len() > COLUMNS.len()
                || columns
                    .iter()
                    .any(|name| !COLUMNS.contains(&name.as_str()) || !seen.insert(name))
            {
                return Err(error("columns_invalid"));
            }
        }
        if let Some(widths) = &self.column_widths {
            let mut seen = HashSet::new();
            if widths.len() > COLUMNS.len()
                || widths.iter().any(|row| {
                    !COLUMNS.contains(&row.column.as_str())
                        || !seen.insert(&row.column)
                        || !row.width.is_finite()
                        || !(68.0..=720.0).contains(&row.width)
                })
            {
                return Err(error("column_widths_invalid"));
            }
        }
        if self
            .agent_scan_mode
            .as_deref()
            .is_some_and(|mode| !matches!(mode, "quick" | "standard" | "deep"))
        {
            return Err(error("scan_mode_invalid"));
        }
        Ok(())
    }
}
fn decode(value: Value) -> Result<Preferences, String> {
    let object = value.as_object().ok_or_else(|| error("value_invalid"))?;
    const KEYS: [&str; 4] = [
        "columns",
        "columnWidths",
        "portalColumnsV1",
        "agentScanMode",
    ];
    if object.len() != KEYS.len() || KEYS.iter().any(|key| !object.contains_key(*key)) {
        return Err(error("value_invalid"));
    }
    let preferences: Preferences =
        serde_json::from_value(value).map_err(|_| error("value_invalid"))?;
    preferences.validate()?;
    Ok(preferences)
}
fn schema_on(connection: &Connection) -> Result<(), String> {
    for (kind, name, expected) in [
        ("table", "asset_service_preferences", TABLE),
        (
            "trigger",
            "asset_service_preferences_update",
            UPDATE_TRIGGER,
        ),
        (
            "trigger",
            "asset_service_preferences_delete",
            DELETE_TRIGGER,
        ),
    ] {
        let sql: Option<String> = connection
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type=?1 AND name=?2",
                [kind, name],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| error("schema_unavailable"))?;
        if sql.as_deref().map(|v| v.trim_end_matches(';')) != Some(expected) {
            return Err(error("schema_unavailable"));
        }
    }
    Ok(())
}
pub(super) fn read_on(connection: &Connection) -> Result<View, String> {
    schema_on(connection)?;
    let instance = service_instance_on(connection)?;
    let rows: Vec<(i64, String, i64, String)> = connection
        .prepare(
            "SELECT singleton,instance_id,revision,preferences_json FROM asset_service_preferences",
        )
        .and_then(|mut q| {
            q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect()
        })
        .map_err(|_| error("read_unavailable"))?;
    if rows.len() != 1
        || rows[0].0 != 1
        || rows[0].1 != instance
        || !(0..=MAX_REVISION).contains(&rows[0].2)
    {
        return Err(error("original_owner_mismatch"));
    }
    let value: Value =
        serde_json::from_str(&rows[0].3).map_err(|_| error("stored_value_invalid"))?;
    Ok(View {
        schema_version: 1,
        revision: rows[0].2,
        preferences: decode(value).map_err(|_| error("stored_value_invalid"))?,
    })
}
/// Only original-store bootstrap calls this while its exclusive controller is
/// held. Existing malformed state is refused, never reset to a default layout.
pub(super) fn initialize_on(connection: &Connection) -> Result<(), String> {
    let tx = rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
        .map_err(|_| error("initialization_unavailable"))?;
    let exists: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='asset_service_preferences')",
            [],
            |r| r.get(0),
        )
        .map_err(|_| error("schema_unavailable"))?;
    if !exists {
        let instance = service_instance_on(&tx)?;
        tx.execute_batch(&format!("{TABLE};{UPDATE_TRIGGER};{DELETE_TRIGGER};"))
            .map_err(|_| error("schema_unavailable"))?;
        tx.execute(
            "INSERT INTO asset_service_preferences VALUES(1,?1,0,?2)",
            params![
                instance,
                serde_json::to_string(&Preferences::default())
                    .map_err(|_| error("value_invalid"))?
            ],
        )
        .map_err(|_| error("initialization_unavailable"))?;
    }
    read_on(&tx)?;
    tx.commit().map_err(|_| error("initialization_unconfirmed"))
}
pub(super) fn save_on(
    connection: &mut Connection,
    expected_revision: i64,
    preferences: Preferences,
) -> Result<View, String> {
    if !(0..MAX_REVISION).contains(&expected_revision) {
        return Err(error("revision_invalid"));
    }
    preferences.validate()?;
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| error("write_unavailable"))?;
    let current = read_on(&tx)?;
    if current.revision != expected_revision {
        return Err(error("revision_conflict"));
    }
    let bytes = serde_json::to_string(&preferences).map_err(|_| error("value_invalid"))?;
    let changed=tx.execute("UPDATE asset_service_preferences SET revision=revision+1,preferences_json=?1 WHERE singleton=1 AND revision=?2",params![bytes,expected_revision]).map_err(|_|error("write_unavailable"))?;
    if changed != 1 {
        return Err(error("revision_conflict"));
    }
    let saved = read_on(&tx)?;
    if saved.revision != expected_revision + 1 || saved.preferences != preferences {
        return Err(error("write_unconfirmed"));
    }
    tx.commit().map_err(|_| error("write_unconfirmed"))?;
    Ok(saved)
}
pub(super) fn dispatch_application(
    connection: &mut Connection,
    name: &str,
    args: Value,
) -> Result<Value, String> {
    let value = match name {
        "get_asset_preferences" => {
            if args.as_object().is_none_or(|o| !o.is_empty()) {
                return Err(error("arguments_invalid"));
            }
            read_on(connection)?
        }
        "save_asset_preferences" => {
            let object = args.as_object().ok_or_else(|| error("arguments_invalid"))?;
            if object.len() != 2
                || !object.contains_key("expectedRevision")
                || !object.contains_key("preferences")
            {
                return Err(error("arguments_invalid"));
            }
            let expected = object["expectedRevision"]
                .as_i64()
                .ok_or_else(|| error("revision_invalid"))?;
            save_on(connection, expected, decode(object["preferences"].clone())?)?
        }
        _ => return Err(error("method_unavailable")),
    };
    serde_json::to_value(value).map_err(|_| error("result_invalid"))
}
#[cfg(test)]
#[path = "preferences_service_tests.rs"]
mod tests;
