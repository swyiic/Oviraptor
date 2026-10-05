//! Model audit artifacts are inert imported records, never a live model source.
use super::{json_records, record, ParseContext, EVENT_FIELDS};
use crate::artifact_import::canonical::{CanonicalRecord, Producer, RecordKind};
use crate::artifact_import::diagnostics::Diagnostic;
use serde_json::Value;

pub fn hook_records(
    context: &ParseContext<'_>,
    path: &str,
    bytes: &[u8],
) -> (Vec<CanonicalRecord>, Vec<Diagnostic>) {
    json_records::event_stream_records(
        context,
        path,
        bytes,
        &Producer::new("model-hook", "", ""),
        "model_audit",
        "model_hook",
    )
}

pub fn prompt_records(
    context: &ParseContext<'_>,
    path: &str,
    bytes: &[u8],
) -> (Vec<CanonicalRecord>, Vec<Diagnostic>) {
    let value = match json_records::parse_json(context, path, bytes) {
        Ok(value) => value,
        Err(notes) => return (Vec::new(), notes),
    };
    let Some(mut payload) = value.as_object().cloned() else {
        return (
            Vec::new(),
            vec![Diagnostic::warning(
                "prompt_audit_shape_unsupported",
                path,
                "提示词审计必须是对象",
            )],
        );
    };
    payload.insert("trace_source".into(), Value::String(path.into()));
    payload.insert("trace_kind".into(), Value::String("prompt_audit".into()));
    (
        vec![record(
            context,
            RecordKind::EventTrace,
            "model_audit",
            Producer::new("prompt-audit", "", ""),
            "/".into(),
            path,
            payload,
            &["trace_source"],
            EVENT_FIELDS,
        )],
        Vec::new(),
    )
}
