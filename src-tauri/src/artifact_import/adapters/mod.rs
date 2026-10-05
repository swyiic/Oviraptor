//! Adapters parse one historical format each and never fall back to another format
//! when they come up empty (§9.7). Every adapter returns canonical records plus
//! diagnostics for what it could not read.

pub mod frontend_recon;
mod json_records;
pub mod model_audit;
pub mod sarif;
pub(crate) mod sentinel_bundle;
pub mod sentinel_stages;
pub(crate) mod source_report;

use super::canonical::{
    finding_fingerprint, identity_values, CanonicalRecord, Producer, RecordInput, RecordKind,
};
use super::diagnostics::Diagnostic;
use super::limits::Limits;
use super::manifest::Manifest;
use serde_json::{Map, Value as JsonValue};

pub struct ParseContext<'a> {
    pub bundle_id: &'a str,
    pub manifest: &'a Manifest,
    pub payloads: &'a [(String, Vec<u8>)],
    pub limits: &'a Limits,
}

/// Payload fields an adapter may keep; anything else is preserved as an extension.
pub const FINDING_FIELDS: &[&str] = &[
    "id",
    "title",
    "severity",
    "cvss",
    "cwe",
    "target",
    "endpoint",
    "method",
    "parameter",
    "symbol",
    "path",
    "region",
    "locations",
    "code_locations",
    "technical_analysis",
    "poc_description",
    "poc_script_code",
    "remediation_steps",
    "evidence",
    "confidence",
    "confidence_rationale",
    "counterevidence",
    "severity_change_conditions",
    "fix_verification",
    "update_history",
    "status",
    "tags",
    "rule_id",
    "producer_note",
    "source",
    "session_id",
    "locations",
    "code_flows",
    "fingerprints",
    "level",
    "kind",
    "message",
    "region",
    "surface",
    "outcome",
    "schema_version",
    "summary",
    "parameter",
    "symbol",
];

pub const COVERAGE_FIELDS: &[&str] = &[
    "surface",
    "outcome",
    "reason",
    "identities",
    "evidence",
    "rule_id",
    "kind",
    "level",
    "schema_version",
    "summary",
    "completeness",
    "entries",
    "gaps",
    "surfaces_reviewed",
    "caveats",
];

pub const RUN_STATE_FIELDS: &[&str] = &[
    "trace_source",
    "run_id",
    "run_name",
    "status",
    "target",
    "created_at",
    "instruction",
];

pub const EVENT_FIELDS: &[&str] = &[
    "trace_source",
    "trace_kind",
    "trace_session_key",
    "type",
    "role",
    "name",
    "call_id",
    "status",
    "arguments",
    "output",
    "content",
    "created_at",
    "line",
    "session_id",
    "raw",
];

pub const EVIDENCE_FIELDS: &[&str] = &[
    "url",
    "status_code",
    "final_url",
    "apis",
    "api_candidates",
    "routes",
    "js_files",
    "identity_runs",
    "runtime_exploration",
    "capture_status",
    "capture_error",
    "stage",
    "observations",
    "validations",
    "opportunities",
    "report",
    "counts",
    "summary",
    "sensitive_info",
    "schema_version",
];

/// §9.7 缺口 8：finding 的逻辑身份是五元组 —— 类别(规则/CWE) + 目标或仓库路径 +
/// 方法 + 参数或符号 + 区域。数组顺序、标题与严重度都不参与，避免重排或改级别时把
/// 同一问题当成两个。传这个标记的条目由 `finding_fingerprint` 归一后取值。
pub const FINDING_FINGERPRINT: &[&str] = &["*fingerprint"];

#[allow(clippy::too_many_arguments)]
pub(crate) fn record(
    context: &ParseContext<'_>,
    kind: RecordKind,
    adapter: &str,
    producer: Producer,
    pointer: String,
    relative_path: &str,
    payload: Map<String, JsonValue>,
    identity: &[&str],
    known: &'static [&'static str],
) -> CanonicalRecord {
    let identity = if identity == FINDING_FINGERPRINT {
        finding_fingerprint(&payload)
    } else {
        identity_values(&payload, identity)
    };
    CanonicalRecord::new(RecordInput {
        kind,
        adapter,
        producer,
        bundle_id: context.bundle_id.to_string(),
        source_artifact_id: context
            .manifest
            .content_hash(relative_path)
            .unwrap_or_default()
            .to_string(),
        pointer,
        identity,
        payload,
        known,
    })
}

/// Dispatches each file to exactly one adapter. Unknown names are reported, not
/// guessed at, and one unreadable file never silences the others.
pub fn parse_bundle(context: &ParseContext<'_>) -> (Vec<CanonicalRecord>, Vec<Diagnostic>) {
    let mut records = Vec::new();
    let mut diagnostics = Vec::new();
    for (relative_path, bytes) in context.payloads {
        let name = std::path::Path::new(relative_path)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let outcome = if name == "llm-hook.jsonl" {
            model_audit::hook_records(context, relative_path, bytes)
        } else if name == "model-prompt-audit.json" {
            model_audit::prompt_records(context, relative_path, bytes)
        } else if std::path::Path::new(&name)
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("sarif"))
        {
            sarif::records(context, relative_path, bytes)
        } else if name == "oviraptor_recon.json" || name == "asset_atlas_recon.json" {
            frontend_recon::records(context, relative_path, bytes)
        } else if name.ends_with(".json") && sentinel_stages::stage_of(name).is_some() {
            sentinel_stages::records(context, relative_path, bytes)
        } else if name == "meta.json" || source_report::is_json_name(name) {
            (Vec::new(), Vec::new())
        } else {
            diagnostics.push(Diagnostic::warning(
                "unrecognized_artifact",
                relative_path,
                format!("未知文件名 {name}，本轮没有解析它"),
            ));
            (Vec::new(), Vec::new())
        };
        let (mut rows, mut notes) = outcome;
        records.append(&mut rows);
        diagnostics.append(&mut notes);
    }
    (records, diagnostics)
}
