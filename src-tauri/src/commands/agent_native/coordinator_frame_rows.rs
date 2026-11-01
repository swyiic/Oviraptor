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

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeCoordinatorMapperFact {
    summary: String,
    priority_contracts: Vec<String>,
    risks: Vec<String>,
}
// This parses the model's own already-receipt-verified evidence summary (the
// caller matched `text` against the stored payload before calling us), so the
// only question here is whether the model happened to shape its JSON exactly
// how the schema expected. It rarely does: an extra field, a missing list, a
// numeric item, an over-long string or a 17th entry would each hard-fail the
// whole run at bootstrap on `root_frame_mapper_schema_invalid` — the "change
// one URL and it dies" failure. Since this fact is advisory (it never feeds the
// budget receipts or the physical-fact hashes in `closed_rows`), salvage it
// instead of rejecting it: keep the real `summary`, coerce any list item to
// text, drop empties, strip control chars, truncate over-long values and cap
// the list length. Only genuinely unparseable output (not even a JSON object)
// still fails, because then there is nothing to salvage.
const MAPPER_TEXT_MAX: usize = 2048;
const MAPPER_LIST_MAX: usize = 16;
// Keep a single clean bounded line out of whatever the model produced: strip
// control characters, collapse whitespace runs, trim and truncate by chars so a
// multibyte value is never split.
fn sanitize_mapper_text(raw: &str) -> String {
    let mut out = String::new();
    let mut last_space = false;
    for c in raw.chars() {
        // Whitespace first: a tab/newline is also a control char, but it
        // separates words and must collapse to a single space, not vanish and
        // glue the words together.
        if c.is_whitespace() {
            if !out.is_empty() && !last_space {
                out.push(' ');
                last_space = true;
            }
            continue;
        }
        if c.is_control() || c == '\u{feff}' {
            continue;
        }
        out.push(c);
        last_space = false;
        if out.chars().count() >= MAPPER_TEXT_MAX {
            break;
        }
    }
    out.trim().to_string()
}
// A list item may arrive as a string, a structured object/array (keep it as
// compact JSON so the model's structure survives as readable text) or a bare
// scalar; null and anything that sanitizes to empty is dropped rather than
// failing the run.
fn coerce_mapper_item(value: &JsonValue) -> Option<String> {
    let text = match value {
        JsonValue::Null => return None,
        JsonValue::String(text) => text.clone(),
        JsonValue::Object(_) | JsonValue::Array(_) => value.to_string(),
        other => other.to_string(),
    };
    let clean = sanitize_mapper_text(&text);
    (!clean.is_empty()).then_some(clean)
}
// Accept an absent field, a single value or a list; salvage each entry and cap
// the result so a flood of items cannot bloat the stored fact.
fn coerce_mapper_list(value: Option<&JsonValue>) -> Vec<String> {
    let items: Vec<String> = match value {
        None | Some(JsonValue::Null) => Vec::new(),
        Some(JsonValue::Array(items)) => items.iter().filter_map(coerce_mapper_item).collect(),
        Some(single) => coerce_mapper_item(single).into_iter().collect(),
    };
    items.into_iter().take(MAPPER_LIST_MAX).collect()
}
fn native_coordinator_mapper_semantic(text: &str) -> Result<JsonValue, String> {
    let raw: JsonValue =
        serde_json::from_str(text).map_err(|_| "root_frame_mapper_schema_invalid")?;
    // Nothing to salvage from a non-object payload; keep that guard.
    if !raw.is_object() {
        return Err("root_frame_mapper_schema_invalid".into());
    }
    // An empty summary still proceeds with a bounded placeholder: a mapper that
    // produced no headline is not a reason to abort the whole scan at bootstrap.
    let summary = raw.get("summary").and_then(JsonValue::as_str).unwrap_or("");
    let summary = sanitize_mapper_text(summary);
    let summary = if summary.is_empty() {
        "（映射未产出摘要）".to_string()
    } else {
        summary
    };
    let v = NativeCoordinatorMapperFact {
        summary,
        priority_contracts: coerce_mapper_list(raw.get("priorityContracts")),
        risks: coerce_mapper_list(raw.get("risks")),
    };
    serde_json::to_value(v)
        .map(|v| crate::agent_runtime::secrets::redact_json(&v))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod mapper_salvage_tests {
    use super::JsonValue;
    // These shapes all used to hard-fail the whole scan at bootstrap on
    // `root_frame_mapper_schema_invalid`; they must now be salvaged so a single
    // target's imperfect mapper output no longer aborts an autonomous run.
    fn run(text: &str) -> Result<JsonValue, String> {
        super::native_coordinator_mapper_semantic(text)
    }

    #[test]
    fn clean_input_is_preserved() {
        let out = run(r#"{"summary":"登录入口可达","priorityContracts":["/pc/login"],"risks":["弱口令"]}"#).unwrap();
        assert_eq!(out["summary"], "登录入口可达");
        assert_eq!(out["priorityContracts"][0], "/pc/login");
        assert_eq!(out["risks"][0], "弱口令");
    }

    #[test]
    fn unknown_field_no_longer_fails() {
        // deny_unknown_fields used to reject this outright.
        let out = run(r#"{"summary":"ok","priorityContracts":[],"risks":[],"extra":42}"#).unwrap();
        assert_eq!(out["summary"], "ok");
    }

    #[test]
    fn missing_lists_default_to_empty() {
        let out = run(r#"{"summary":"仅摘要"}"#).unwrap();
        assert_eq!(out["summary"], "仅摘要");
        assert_eq!(out["priorityContracts"].as_array().unwrap().len(), 0);
        assert_eq!(out["risks"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn scalar_and_null_items_are_coerced_or_dropped() {
        let out = run(r#"{"summary":"s","priorityContracts":[1,true,null,"keep"],"risks":[]}"#).unwrap();
        let list = out["priorityContracts"].as_array().unwrap();
        // null dropped; 1/true/"keep" kept as text.
        assert_eq!(list.len(), 3);
        assert_eq!(list[0], "1");
        assert_eq!(list[1], "true");
        assert_eq!(list[2], "keep");
    }

    #[test]
    fn object_item_becomes_compact_json() {
        let out = run(r#"{"summary":"s","priorityContracts":[{"path":"/a"}],"risks":[]}"#).unwrap();
        assert_eq!(out["priorityContracts"][0], "{\"path\":\"/a\"}");
    }

    #[test]
    fn single_value_list_is_wrapped() {
        let out = run(r#"{"summary":"s","priorityContracts":"/only","risks":[]}"#).unwrap();
        assert_eq!(out["priorityContracts"][0], "/only");
    }

    #[test]
    fn oversize_summary_is_truncated_not_rejected() {
        let big = "甲".repeat(5000);
        let out = run(&format!(r#"{{"summary":"{big}","priorityContracts":[],"risks":[]}}"#)).unwrap();
        assert_eq!(out["summary"].as_str().unwrap().chars().count(), super::MAPPER_TEXT_MAX);
    }

    #[test]
    fn long_list_is_capped() {
        let items: Vec<String> = (0..40).map(|i| format!("\"c{i}\"")).collect();
        let out = run(&format!(
            r#"{{"summary":"s","priorityContracts":[{}],"risks":[]}}"#,
            items.join(",")
        ))
        .unwrap();
        assert_eq!(out["priorityContracts"].as_array().unwrap().len(), super::MAPPER_LIST_MAX);
    }

    #[test]
    fn empty_summary_gets_placeholder() {
        let out = run(r#"{"summary":"   ","priorityContracts":[],"risks":[]}"#).unwrap();
        assert!(!out["summary"].as_str().unwrap().is_empty());
    }

    #[test]
    fn control_chars_are_stripped() {
        let out = run("{\"summary\":\"a\\u0007b\\tc\",\"priorityContracts\":[],\"risks\":[]}").unwrap();
        // bell removed, tab collapsed to one space.
        assert_eq!(out["summary"], "ab c");
    }

    #[test]
    fn non_object_still_fails() {
        assert!(run(r#"["not","an","object"]"#).is_err());
        assert!(run("not json at all").is_err());
        assert!(run(r#""just a string""#).is_err());
    }
}
