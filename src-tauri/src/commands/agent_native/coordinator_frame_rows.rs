// Only hashes of frozen physical facts enter Root's persisted request.
#[derive(Clone)]
struct NativeCoordinatorFrozenRows {
    sql: String,
    keys: Vec<String>,
    hash: String,
    allow_empty: bool,
}
impl NativeCoordinatorFrozenRows {
    fn capture(db: &rusqlite::Connection, sql: &str, keys: &[&str]) -> Result<Self, String> {
        let keys = keys.iter().map(|v| v.to_string()).collect();
        let mut rows = Self {
            sql: sql.into(),
            keys,
            hash: String::new(),
            allow_empty: false,
        };
        rows.hash = rows.read_hash(db)?;
        Ok(rows)
    }
    // Normal paid calls use budget receipts/entries and their event/snapshot.
    // Late-result facts are an additional immutable financial path; capture
    // their exact empty/nonempty state without requiring a fabricated fact.
    fn capture_optional(
        db: &rusqlite::Connection,
        sql: &str,
        keys: &[&str],
    ) -> Result<Self, String> {
        let mut rows = Self {
            sql: sql.into(),
            keys: keys.iter().map(|v| v.to_string()).collect(),
            hash: String::new(),
            allow_empty: true,
        };
        rows.hash = rows.read_hash(db)?;
        Ok(rows)
    }
    fn read_hash(&self, db: &rusqlite::Connection) -> Result<String, String> {
        let mut q = db.prepare(&self.sql).map_err(|e| e.to_string())?;
        let count = q.column_count();
        let rows = q
            .query_map(rusqlite::params_from_iter(&self.keys), |r| {
                (0..count)
                    .map(|i| r.get(i))
                    .collect::<rusqlite::Result<Vec<rusqlite::types::Value>>>()
            })
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        if rows.is_empty() && !self.allow_empty {
            return Err("root_frame_physical_fact_missing".into());
        }
        // Raw SQL bytes, model messages and credentials remain only in memory.
        Ok(crate::agent_runtime::store::stable_hash(&format!(
            "{rows:?}"
        )))
    }
    fn verify(&self, db: &rusqlite::Connection) -> Result<(), String> {
        if self.read_hash(db)? != self.hash {
            return Err("root_frame_original_fact_changed".into());
        }
        Ok(())
    }
    fn fact(&self) -> JsonValue {
        json!({"query":self.sql,"keys":self.keys,"physicalHash":self.hash})
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NativeCoordinatorMapperFact {
    summary: String,
    priority_contracts: Vec<String>,
    risks: Vec<String>,
}
fn native_coordinator_mapper_semantic(text: &str) -> Result<JsonValue, String> {
    let v: NativeCoordinatorMapperFact =
        serde_json::from_str(text).map_err(|_| "root_frame_mapper_schema_invalid")?;
    if v.summary.trim().is_empty()
        || v.summary.chars().count() > 2048
        || v.summary.chars().any(|c| c.is_control())
        || [&v.priority_contracts, &v.risks].iter().any(|a| {
            a.len() > 16
                || a.iter().any(|s| {
                    s.trim().is_empty()
                        || s.chars().count() > 512
                        || s.chars().any(|c| c.is_control())
                })
        })
    {
        return Err("root_frame_mapper_schema_invalid".into());
    }
    serde_json::to_value(v)
        .map(|v| crate::agent_runtime::secrets::redact_json(&v))
        .map_err(|e| e.to_string())
}
