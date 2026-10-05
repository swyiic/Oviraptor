//! §9.5 — the canonical record envelope. Adapters produce these; nothing below is
//! allowed to invent a claim: every record from this module carries the historical,
//! unreviewed, read-only claim of §9.8.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as JsonValue};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const CANONICAL_SCHEMA: &str = "oviraptor.artifact.v1";
pub const ADAPTER_VERSION: u32 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordKind {
    FindingCandidate,
    Coverage,
    RunState,
    Usage,
    EventTrace,
    EvidenceNote,
}

impl RecordKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FindingCandidate => "finding_candidate",
            Self::Coverage => "coverage",
            Self::RunState => "run_state",
            Self::Usage => "usage",
            Self::EventTrace => "event_trace",
            Self::EvidenceNote => "evidence_note",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Producer {
    pub name: String,
    pub version_raw: String,
    pub format_schema_raw: String,
}

impl Producer {
    pub fn new(name: &str, version_raw: &str, format_schema_raw: &str) -> Self {
        Self {
            name: name.to_string(),
            version_raw: version_raw.to_string(),
            format_schema_raw: format_schema_raw.to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provenance {
    pub bundle_id: String,
    pub source_artifact_id: String,
    pub source_record_pointer: String,
    pub import_adapter: String,
    pub adapter_version: u32,
}

/// Historical claims are never executable and never a confirmation (§9.8, IMP-018).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Claim {
    pub authority: String,
    pub review_state: String,
    pub execution_eligible: bool,
    pub read_only: bool,
}

impl Claim {
    pub fn historical_external() -> Self {
        Self {
            authority: "historical_external".to_string(),
            review_state: "unreviewed".to_string(),
            execution_eligible: false,
            read_only: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalRecord {
    pub canonical_schema: String,
    pub record_kind: RecordKind,
    pub logical_key: String,
    pub revision_hash: String,
    pub producer: Producer,
    pub provenance: Provenance,
    pub claim: Claim,
    pub payload: JsonValue,
    pub extensions: JsonValue,
    pub field_origins: BTreeMap<String, String>,
    pub conflicts: Vec<JsonValue>,
    /// §9.7 normalized identity dimensions. Kept out of the envelope: it is merge
    /// bookkeeping, not historical content.
    #[serde(skip, default)]
    pub fingerprint: Vec<String>,
    /// Current parse/merge lineage for the commit receipt, not historical
    /// envelope content. Keep it out of serialization and revision identity.
    #[serde(skip, default)]
    pub contributing_artifacts: BTreeSet<String>,
}

/// What an adapter knows about a record before it becomes an envelope: `identity`
/// fields decide the logical key, `known` fields stay in the payload, everything
/// else is preserved as an extension instead of being dropped.
pub struct RecordInput<'a> {
    pub kind: RecordKind,
    pub adapter: &'a str,
    pub producer: Producer,
    pub bundle_id: String,
    pub source_artifact_id: String,
    pub pointer: String,
    pub identity: Vec<String>,
    pub payload: Map<String, JsonValue>,
    pub known: &'static [&'static str],
}

impl CanonicalRecord {
    pub fn new(input: RecordInput<'_>) -> Self {
        let origin = format!("{}:{}", input.adapter, input.pointer);
        let mut payload = Map::new();
        let mut extensions = Map::new();
        let mut field_origins = BTreeMap::new();
        for (key, value) in input.payload {
            if input.known.contains(&key.as_str()) {
                field_origins.insert(key.clone(), origin.clone());
                payload.insert(key, value);
            } else {
                extensions.insert(key, value);
            }
        }
        // Rows that carry no identity field at all must not collide into a single
        // logical key, so they fall back to where they came from.
        let anonymous = input.identity.iter().all(|value| value.is_empty());
        let identity = if anonymous {
            format!("{}|{}", input.adapter, input.pointer)
        } else {
            input.identity.join("|")
        };
        let logical_key = Self::key(&[input.kind.as_str(), &identity]);
        let contributing_artifacts = BTreeSet::from([input.source_artifact_id.clone()]);
        let mut record = Self {
            canonical_schema: CANONICAL_SCHEMA.to_string(),
            record_kind: input.kind,
            logical_key,
            revision_hash: String::new(),
            producer: input.producer,
            provenance: Provenance {
                bundle_id: input.bundle_id,
                source_artifact_id: input.source_artifact_id,
                source_record_pointer: input.pointer,
                import_adapter: input.adapter.to_string(),
                adapter_version: ADAPTER_VERSION,
            },
            claim: Claim::historical_external(),
            payload: JsonValue::Object(payload),
            extensions: JsonValue::Object(extensions),
            field_origins,
            conflicts: Vec::new(),
            fingerprint: input.identity,
            contributing_artifacts,
        };
        record.finalize_revision_hash();
        record
    }

    /// §9.7/§9.9 缺口 3: the revision hash describes the *final* merged record, so a
    /// supplement that arrives from another format produces a new revision instead of
    /// silently rewriting the old one. `field_origins` is deliberately excluded: an
    /// origin string carries the position the row came from, and §IDM-005 requires that
    /// reordering a source array never invents a revision.
    pub fn finalize_revision_hash(&mut self) {
        self.revision_hash = Self::key(&[
            &canonical_json(&self.payload),
            &canonical_json(&self.extensions),
            &canonical_json(&JsonValue::Array(self.conflicts.clone())),
        ]);
    }

    fn key(parts: &[&str]) -> String {
        let mut digest = Sha256::new();
        for part in parts {
            digest.update(part.as_bytes());
            digest.update([0u8]);
        }
        format!("sha256:{}", hex(&digest.finalize()))
    }

    /// §9.7 — cross-format merge. Equal fields agree, new fields are taken, and a
    /// disagreement keeps both values plus who said what; nothing is overwritten
    /// silently and the adopted source stays recorded.
    pub fn merge_from(&mut self, other: &CanonicalRecord) {
        let Some(target) = self.payload.as_object_mut() else {
            return;
        };
        for (key, value) in other.payload.as_object().cloned().unwrap_or_default() {
            match target.get(&key) {
                None => {
                    let origin = other
                        .field_origins
                        .get(&key)
                        .cloned()
                        .unwrap_or_else(|| other.provenance.import_adapter.clone());
                    target.insert(key.clone(), value);
                    self.field_origins.insert(key, origin);
                }
                Some(existing) if *existing == value => {}
                Some(existing) => self.conflicts.push(serde_json::json!({
                    "field": key,
                    "adopted": existing,
                    "adoptedFrom": self.field_origins.get(&key),
                    "candidate": value,
                    "candidateFrom": other.field_origins.get(&key),
                    "candidateAdapter": other.provenance.import_adapter,
                })),
            }
        }
        let target_extensions = self
            .extensions
            .as_object_mut()
            .expect("extensions are always an object");
        for (key, value) in other.extensions.as_object().cloned().unwrap_or_default() {
            match target_extensions.get(&key) {
                None => {
                    target_extensions.insert(key, value);
                }
                Some(existing) if *existing == value => {}
                Some(existing) => self.conflicts.push(serde_json::json!({
                    "field": key,
                    "adopted": existing,
                    "candidate": value,
                    "candidateAdapter": other.provenance.import_adapter,
                })),
            }
        }
        self.contributing_artifacts
            .insert(self.provenance.source_artifact_id.clone());
        self.contributing_artifacts
            .insert(other.provenance.source_artifact_id.clone());
        self.contributing_artifacts
            .extend(other.contributing_artifacts.iter().cloned());
    }
}

/// Stable serialization: `serde_json::Map` is a sorted map, so key order is already
/// normalized; array order is kept because flows and steps are meaningful.
pub fn canonical_json(value: &JsonValue) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// Pulls the identity fields a logical key is built from (§9.7).
pub fn identity_values(payload: &Map<String, JsonValue>, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .map(|key| {
            payload
                .get(*key)
                .map(|value| match value {
                    JsonValue::String(text) => text.trim().to_string(),
                    JsonValue::Null => String::new(),
                    other => other.to_string(),
                })
                .unwrap_or_default()
        })
        .collect()
}

/// §9.7 缺口 8 — a finding's logical identity is the five-tuple
/// `class/rule/CWE + target/repo path + method + parameter/symbol + region`.
/// Each dimension names the payload fields that may supply it, in priority order.
const FINDING_CLASS_FIELDS: &[&str] = &["cwe", "rule_id", "kind"];
const FINDING_TARGET_FIELDS: &[&str] =
    &["target", "repo_path", "repository_path", "endpoint", "url"];
const FINDING_PARAMETER_FIELDS: &[&str] = &["parameter", "symbol"];

/// The normalized identity dimensions, in a fixed order. An empty entry means this
/// format did not say anything about that dimension, which is different from saying
/// something else. The §9.7 "target/repo path" is carried as a host part and a path
/// part, because one format writes `https://host/path` in a single field while
/// another splits a base target and an endpoint, and those must still compare equal.
pub fn finding_fingerprint(payload: &Map<String, JsonValue>) -> Vec<String> {
    let (host, path) = target_parts(payload);
    vec![
        first_text(payload, FINDING_CLASS_FIELDS)
            .map_or(String::new(), |text| normalize_class(&text)),
        host,
        path,
        first_text(payload, &["method"]).map_or(String::new(), |text| text.to_uppercase()),
        first_text(payload, FINDING_PARAMETER_FIELDS)
            .map_or(String::new(), |text| normalize_word(&text)),
        first_text(payload, &["region"]).map_or(String::new(), |text| normalize_word(&text)),
    ]
}

fn first_text(payload: &Map<String, JsonValue>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        payload
            .get(*key)
            .map(|value| match value {
                JsonValue::String(text) => collapse_spaces(text),
                JsonValue::Null => String::new(),
                other => collapse_spaces(&other.to_string()),
            })
            .filter(|text| !text.is_empty())
    })
}

fn collapse_spaces(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_word(text: &str) -> String {
    collapse_spaces(text).to_lowercase().replace('\\', "/")
}

/// The §9.7 five-tuple: class, target/repo path (host + path), method,
/// parameter/symbol, region.
/// `CWE-639`, `cwe 639` and `CWE:639` are the same class; everything else keeps its
/// own rule name, lower-cased.
fn normalize_class(text: &str) -> String {
    let lowered = normalize_word(text);
    if let Some(tail) = lowered.strip_prefix("cwe") {
        let digits: String = tail.chars().filter(char::is_ascii_alphanumeric).collect();
        if !digits.is_empty() {
            return format!("cwe-{digits}");
        }
    }
    lowered
}

/// A target may be written as one full URL, or split into a host and a path across
/// two fields, or be a repository path. Those spellings describe one location, so the
/// dimension is rebuilt from whichever side supplies host and whichever supplies path.
fn target_parts(payload: &Map<String, JsonValue>) -> (String, String) {
    let mut host = String::new();
    let mut path = String::new();
    for key in FINDING_TARGET_FIELDS {
        let Some(text) = first_text(payload, std::slice::from_ref(key)) else {
            continue;
        };
        let (part_host, part_path) = split_target(&text);
        if host.is_empty() {
            host = part_host;
        }
        if path.is_empty() {
            path = part_path;
        }
    }
    (host, path)
}

fn split_target(text: &str) -> (String, String) {
    let lowered = normalize_word(text);
    let without_scheme = lowered
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let (head, tail) = match without_scheme.find('/') {
        Some(at) => (&without_scheme[..at], &without_scheme[at..]),
        None => (without_scheme, ""),
    };
    (
        head.trim_end_matches('/').to_string(),
        tail.trim_end_matches('/').to_string(),
    )
}

/// §9.7 缺口 8: two fingerprints describe the same finding when nothing they both
/// state disagrees *and* at least two dimensions are stated by both sides. One shared
/// field is a coincidence — two records that merely name the same
/// endpoint are two findings until something else corroborates it, and merging them
/// would drop a unique finding (§IMP-004). Records that only fail to contradict each
/// other stay apart, which is also what keeps a title-only note from
/// swallowing a different finding.
pub fn fingerprints_merge(left: &[String], right: &[String]) -> bool {
    if left.iter().all(String::is_empty) || right.iter().all(String::is_empty) {
        return false;
    }
    let mut shared = 0usize;
    for (mine, theirs) in left.iter().zip(right.iter()) {
        if mine.is_empty() || theirs.is_empty() {
            continue;
        }
        if mine != theirs {
            return false;
        }
        shared += 1;
    }
    shared > 1
}
