//! SARIF results use top-level kind/level for classification. Vendor properties
//! and rule names are opaque evidence, never review or execution authority.

use super::{record, COVERAGE_FIELDS, FINDING_FIELDS, FINDING_FINGERPRINT, RUN_STATE_FIELDS};
use crate::artifact_import::adapters::ParseContext;
use crate::artifact_import::canonical::{CanonicalRecord, Producer, RecordKind};
use crate::artifact_import::diagnostics::Diagnostic;
use serde_json::{Map, Value as JsonValue};

const FINDING_LEVELS: &[&str] = &["error", "warning"];

pub fn records(
    context: &ParseContext<'_>,
    relative_path: &str,
    bytes: &[u8],
) -> (Vec<CanonicalRecord>, Vec<Diagnostic>) {
    let Ok(value) = serde_json::from_slice::<JsonValue>(bytes) else {
        return (
            Vec::new(),
            vec![Diagnostic::error(
                "sarif_invalid",
                relative_path,
                "SARIF 不是有效 JSON".to_string(),
            )],
        );
    };
    let runs = value
        .get("runs")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    if runs.is_empty() {
        return (
            Vec::new(),
            vec![Diagnostic::warning(
                "sarif_no_runs",
                relative_path,
                "没有 runs 条目".to_string(),
            )],
        );
    }
    let mut records = Vec::new();
    for (run_index, run) in runs.iter().enumerate() {
        if super::source_report::is_native(&run["properties"]["oviraptorSourceReview"]) {
            continue; // Parsed with its report scope by the shared native adapter.
        }
        let tool = run
            .pointer("/tool/driver")
            .cloned()
            .unwrap_or(JsonValue::Null);
        let producer = Producer::new(
            tool.get("name")
                .and_then(JsonValue::as_str)
                .unwrap_or("sarif-tool"),
            tool.get("version")
                .and_then(JsonValue::as_str)
                .unwrap_or_default(),
            value
                .get("version")
                .and_then(JsonValue::as_str)
                .unwrap_or_default(),
        );
        // Preserve a run-level report once, including audited zero-result
        // exports. Copying properties/tool rules into every finding would have
        // quadratic memory cost. This is historical data, never a native run.
        if let Some(properties) = run.get("properties") {
            let payload = serde_json::json!({
                "run_id":format!("{relative_path}#/runs/{run_index}"),
                "run_name":tool.get("name").and_then(JsonValue::as_str).unwrap_or("sarif-tool"),
                "status":"imported", "sarif_run_properties":properties, "sarif_tool":tool
            })
            .as_object()
            .unwrap()
            .clone();
            records.push(record(
                context,
                RecordKind::RunState,
                "sarif",
                producer.clone(),
                format!("/runs/{run_index}"),
                relative_path,
                payload,
                &["run_id"],
                RUN_STATE_FIELDS,
            ));
        }
        let results = run
            .get("results")
            .and_then(JsonValue::as_array)
            .cloned()
            .unwrap_or_default();
        for (index, result) in results.iter().enumerate() {
            let pointer = format!("/runs/{run_index}/results/{index}");
            if !result.is_object() {
                continue;
            }
            let level = result
                .get("level")
                .and_then(JsonValue::as_str)
                .unwrap_or("warning")
                .to_ascii_lowercase();
            let kind = result
                .get("kind")
                .map(|value| value.as_str().unwrap_or("invalid"))
                .unwrap_or("fail")
                .to_ascii_lowercase();
            let rule_id = result
                .get("ruleId")
                .and_then(JsonValue::as_str)
                .unwrap_or_default()
                .to_string();
            let locations = locations_of(result);
            let mut payload = Map::new();
            // Keep the complete result as historical extension data. Neither
            // properties nor fingerprints confer native review authority.
            payload.insert("sarif_result".to_string(), result.clone());
            payload.insert("rule_id".to_string(), JsonValue::String(rule_id.clone()));
            payload.insert("level".to_string(), JsonValue::String(level.clone()));
            payload.insert("kind".to_string(), JsonValue::String(kind.clone()));
            payload.insert(
                "message".to_string(),
                result
                    .pointer("/message/text")
                    .cloned()
                    .unwrap_or(JsonValue::Null),
            );
            payload.insert(
                "title".to_string(),
                result
                    .pointer("/message/text")
                    .cloned()
                    .unwrap_or(JsonValue::Null),
            );
            payload.insert("locations".to_string(), JsonValue::Array(locations.clone()));
            if let Some(first) = locations.first() {
                payload.insert(
                    "path".to_string(),
                    first.get("path").cloned().unwrap_or(JsonValue::Null),
                );
                payload.insert(
                    "region".to_string(),
                    first.get("region").cloned().unwrap_or(JsonValue::Null),
                );
            }
            payload.insert(
                "code_flows".to_string(),
                result
                    .get("codeFlows")
                    .or_else(|| result.get("codeFlow"))
                    .cloned()
                    .unwrap_or(JsonValue::Null),
            );
            payload.insert(
                "fingerprints".to_string(),
                result
                    .get("partialFingerprints")
                    .cloned()
                    .unwrap_or(JsonValue::Null),
            );
            let is_coverage = level == "none" || kind != "fail";
            if is_coverage {
                payload.insert(
                    "surface".to_string(),
                    payload
                        .get("path")
                        .cloned()
                        .unwrap_or(JsonValue::String(rule_id.clone())),
                );
                payload.insert("outcome".to_string(), JsonValue::String(kind.clone()));
                records.push(record(
                    context,
                    RecordKind::Coverage,
                    "sarif",
                    producer.clone(),
                    pointer,
                    relative_path,
                    payload,
                    &["surface", "rule_id"],
                    COVERAGE_FIELDS,
                ));
                continue;
            }
            if !FINDING_LEVELS.contains(&level.as_str()) {
                records.push(record(
                    context,
                    RecordKind::Coverage,
                    "sarif",
                    producer.clone(),
                    pointer,
                    relative_path,
                    payload,
                    &["surface", "rule_id"],
                    COVERAGE_FIELDS,
                ));
                continue;
            }
            payload.insert(
                "severity".to_string(),
                JsonValue::String(
                    result
                        .pointer("/properties/severity")
                        .and_then(JsonValue::as_str)
                        .filter(|value| {
                            matches!(
                                *value,
                                "critical" | "high" | "medium" | "low" | "informational"
                            )
                        })
                        .unwrap_or(if level == "error" { "high" } else { "medium" })
                        .to_string(),
                ),
            );
            payload.insert(
                "cwe".to_string(),
                result
                    .pointer("/properties/cwe")
                    .cloned()
                    .unwrap_or(JsonValue::Null),
            );
            // `rule_id` already carries the class dimension; inventing a symbol out of
            // it would make every SARIF result look like a different finding (§9.7).
            // A SARIF result can name the same HTTP identity as the JSON export; without
            // that no cross-format merge would ever be possible (§9.7).
            if payload.get("endpoint").is_none() {
                let named = result
                    .pointer("/properties/endpoint")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
                    .or_else(|| {
                        locations.first().and_then(|row| {
                            row.get("path")
                                .and_then(serde_json::Value::as_str)
                                .map(str::to_string)
                        })
                    });
                if let Some(endpoint) = named {
                    payload.insert("endpoint".to_string(), JsonValue::String(endpoint));
                }
            }
            records.push(record(
                context,
                RecordKind::FindingCandidate,
                "sarif",
                producer.clone(),
                pointer,
                relative_path,
                payload,
                FINDING_FINGERPRINT,
                FINDING_FIELDS,
            ));
        }
    }
    (records, Vec::new())
}

/// Zero, one or many locations are all representable (§IMP-008).
fn locations_of(result: &JsonValue) -> Vec<JsonValue> {
    result
        .get("locations")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|location| {
            let physical = location
                .get("physicalLocation")
                .cloned()
                .unwrap_or(JsonValue::Null);
            let mut row = Map::new();
            row.insert(
                "path".to_string(),
                physical
                    .pointer("/artifactLocation/uri")
                    .cloned()
                    .unwrap_or(JsonValue::Null),
            );
            row.insert(
                "region".to_string(),
                physical.get("region").cloned().unwrap_or(JsonValue::Null),
            );
            row.insert(
                "message".to_string(),
                location
                    .get("message")
                    .and_then(|value| value.get("text"))
                    .cloned()
                    .unwrap_or(JsonValue::Null),
            );
            JsonValue::Object(row)
        })
        .collect()
}
