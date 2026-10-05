//! Frontend recon results (§IMP-016): observations, endpoints and capture status are
//! evidence, never conclusions. No record produced here is ever a finding candidate.

use super::{record, EVIDENCE_FIELDS};
use crate::artifact_import::adapters::ParseContext;
use crate::artifact_import::canonical::{CanonicalRecord, Producer, RecordKind};
use crate::artifact_import::diagnostics::Diagnostic;
use serde_json::Value as JsonValue;

pub fn records(
    context: &ParseContext<'_>,
    relative_path: &str,
    bytes: &[u8],
) -> (Vec<CanonicalRecord>, Vec<Diagnostic>) {
    let Ok(value) = serde_json::from_slice::<JsonValue>(bytes) else {
        return (
            Vec::new(),
            vec![Diagnostic::error(
                "recon_invalid",
                relative_path,
                "recon JSON 无效".to_string(),
            )],
        );
    };
    let schema = value
        .get("schemaVersion")
        .map(|item| item.to_string())
        .unwrap_or_default();
    let producer = Producer::new("frontend-recon", "", &schema);
    let targets = value
        .get("targets")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let mut records = Vec::new();
    for (index, target) in targets.iter().enumerate() {
        let mut payload = target.as_object().cloned().unwrap_or_default();
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
        payload.insert(
            "final_url".to_string(),
            payload.get("finalUrl").cloned().unwrap_or(JsonValue::Null),
        );
        payload.insert(
            "identity_runs".to_string(),
            payload
                .get("identityRuns")
                .cloned()
                .unwrap_or_else(|| JsonValue::Array(Vec::new())),
        );
        payload.insert(
            "runtime_exploration".to_string(),
            payload
                .get("runtimeExploration")
                .cloned()
                .unwrap_or(JsonValue::Null),
        );
        payload.insert(
            "apis".to_string(),
            payload
                .get("apis")
                .cloned()
                .unwrap_or(JsonValue::Array(Vec::new())),
        );
        payload.insert(
            "routes".to_string(),
            payload
                .get("routes")
                .cloned()
                .unwrap_or(JsonValue::Array(Vec::new())),
        );
        payload.insert(
            "capture_status".to_string(),
            target
                .pointer("/runtimeExploration/captureStatus")
                .cloned()
                .unwrap_or(JsonValue::Null),
        );
        payload.insert(
            "capture_error".to_string(),
            target
                .pointer("/runtimeExploration/captureError")
                .cloned()
                .unwrap_or(JsonValue::Null),
        );
        records.push(record(
            context,
            RecordKind::EvidenceNote,
            "frontend_recon",
            producer.clone(),
            format!("/targets/{index}"),
            relative_path,
            payload,
            &["url", "final_url"],
            EVIDENCE_FIELDS,
        ));
    }
    if records.is_empty() {
        return (
            records,
            vec![Diagnostic::warning(
                "recon_without_targets",
                relative_path,
                "recon 文件没有 targets 条目".to_string(),
            )],
        );
    }
    (records, Vec::new())
}
