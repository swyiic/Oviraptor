//! `S1`–`S5`, `summary.json` stage files (§IMP-016). They become evidence notes: a
//! historical stage row that says "confirmed" is still an unreviewed, read-only
//! historical claim and never a native conclusion (§9.8, §16).

use super::{record, EVIDENCE_FIELDS};
use crate::artifact_import::adapters::ParseContext;
use crate::artifact_import::canonical::{CanonicalRecord, Producer, RecordKind};
use crate::artifact_import::diagnostics::Diagnostic;
use serde_json::Value as JsonValue;

pub fn stage_of(file_name: &str) -> Option<String> {
    let stem = file_name.trim_end_matches(".json").to_ascii_lowercase();
    if stem == "summary" {
        return Some("summary".to_string());
    }
    ["s1", "s2", "s3", "s4", "s5"]
        .iter()
        .find(|prefix| stem.starts_with(**prefix))
        .map(|prefix| (*prefix).to_string())
}

pub fn records(
    context: &ParseContext<'_>,
    relative_path: &str,
    bytes: &[u8],
) -> (Vec<CanonicalRecord>, Vec<Diagnostic>) {
    let Ok(value) = serde_json::from_slice::<JsonValue>(bytes) else {
        return (
            Vec::new(),
            vec![Diagnostic::error(
                "stage_json_invalid",
                relative_path,
                "阶段结果 JSON 无效".to_string(),
            )],
        );
    };
    let file_name = std::path::Path::new(relative_path)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let stage = stage_of(file_name).unwrap_or_default();
    let mut payload = value.as_object().cloned().unwrap_or_default();
    payload.insert("stage".to_string(), JsonValue::String(stage.clone()));
    payload.insert(
        "url".to_string(),
        payload
            .get("url")
            .cloned()
            .unwrap_or(JsonValue::String(String::new())),
    );
    payload.insert(
        "status_code".to_string(),
        payload
            .get("statusCode")
            .cloned()
            .unwrap_or(JsonValue::Null),
    );
    (
        vec![record(
            context,
            RecordKind::EvidenceNote,
            "sentinel_stages",
            Producer::new("sentinel-stage-export", "", ""),
            format!("/{stage}"),
            relative_path,
            payload,
            &["stage", "url"],
            EVIDENCE_FIELDS,
        )],
        Vec::new(),
    )
}
