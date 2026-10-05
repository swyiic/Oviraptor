//! Bounded JSON and JSONL parsing shared by inert artifact readers.
//! No producer-specific format detection or backend selection belongs here.

use super::{record, ParseContext, EVENT_FIELDS};
use crate::artifact_import::canonical::{CanonicalRecord, Producer, RecordKind};
use crate::artifact_import::diagnostics::{Diagnostic, Severity};
use serde_json::{Map, Value as JsonValue};

pub(super) fn parse_json(
    context: &ParseContext<'_>,
    relative_path: &str,
    bytes: &[u8],
) -> Result<JsonValue, Vec<Diagnostic>> {
    if !context.limits.json_within_depth(bytes) {
        return Err(vec![Diagnostic::build(
            "json_depth_limit",
            Severity::Error,
            relative_path,
            String::new(),
            format!("嵌套深度超过 {}", context.limits.json_depth),
        )]);
    }
    serde_json::from_slice(bytes).map_err(|error| {
        vec![Diagnostic::build(
            "json_invalid",
            Severity::Error,
            relative_path,
            String::new(),
            error.to_string(),
        )]
    })
}

pub(super) fn event_stream_records(
    context: &ParseContext<'_>,
    relative_path: &str,
    bytes: &[u8],
    producer: &Producer,
    adapter: &str,
    trace_kind: &str,
) -> (Vec<CanonicalRecord>, Vec<Diagnostic>) {
    let text = String::from_utf8_lossy(bytes);
    let mut records = Vec::new();
    let mut diagnostics = Vec::new();
    let mut lines = text.lines().enumerate().peekable();
    while let Some((index, line)) = lines.next() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.len() > context.limits.json_line_bytes {
            diagnostics.push(Diagnostic::warning(
                "line_length_limit",
                relative_path,
                format!(
                    "第 {} 行超过 {} 字节，已跳过",
                    index + 1,
                    context.limits.json_line_bytes
                ),
            ));
            continue;
        }
        if !context.limits.json_within_depth(trimmed.as_bytes()) {
            diagnostics.push(Diagnostic::warning(
                "json_depth_limit",
                relative_path,
                format!("第 {} 行嵌套过深，已跳过", index + 1),
            ));
            continue;
        }
        match serde_json::from_str::<JsonValue>(trimmed) {
            Ok(parsed) => {
                let mut payload = parsed.as_object().cloned().unwrap_or_else(|| {
                    let mut map = Map::new();
                    map.insert("raw".to_string(), parsed.clone());
                    map
                });
                payload.insert("line".to_string(), JsonValue::from(index as i64 + 1));
                payload.insert(
                    "trace_source".into(),
                    JsonValue::String(relative_path.into()),
                );
                payload.insert("trace_kind".into(), JsonValue::String(trace_kind.into()));
                if let Some(session) = payload.get("session_id").and_then(JsonValue::as_str) {
                    let key = crate::artifact_import::canonical::sha256_hex(
                        format!("{relative_path}\0{session}").as_bytes(),
                    );
                    payload.insert("trace_session_key".into(), JsonValue::String(key));
                }
                records.push(record(
                    context,
                    RecordKind::EventTrace,
                    adapter,
                    producer.clone(),
                    format!("/{}", index + 1),
                    relative_path,
                    payload,
                    &["trace_source", "line"],
                    EVENT_FIELDS,
                ));
            }
            Err(error) => {
                let last = lines.peek().is_none();
                diagnostics.push(Diagnostic::at_pointer(
                    if last {
                        "jsonl_tail_truncated"
                    } else {
                        "jsonl_bad_line"
                    },
                    if last {
                        Severity::Info
                    } else {
                        Severity::Warning
                    },
                    relative_path,
                    format!("/{}", index + 1),
                    error.to_string(),
                ));
            }
        }
    }
    (records, diagnostics)
}
