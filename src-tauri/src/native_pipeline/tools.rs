//! §10.2 items 6, 10-11 — the only door a model has to the repository. Every call is
//! answered from the frozen snapshot, so "what the model saw" is always reproducible
//! from the tree hash, and anything that could run code, reach the network or edit the
//! source is refused here rather than trusted upstream.

use super::analysis_view::SourceAnalysisView;
use super::snapshot::{RepositorySnapshot, SnapshotFile};
use rusqlite::Connection;
use serde_json::{json, Value as JsonValue};

mod analyzer_results;
mod assignment;
mod dependencies;
mod policy;
mod repository;
use policy::{allowed_keys, forbidden_key, path_like};

/// §10.2 item 6: this list is the whole surface. There is no shell tool, no file-write
/// tool and no way to name a binary from a model call.
pub const SOURCE_TOOLS: &[&str] = &[
    "repo.inventory",
    "repo.search",
    "repo.read_slice",
    "git.changed_files",
    "analyzer.list_results",
    "analyzer.get_result",
    "callgraph.get_slice",
    "dependency.get_record",
    "evidence.submit_candidate",
    "assignment.finish",
];

/// Stable v1 identity shared by submission and receipt-based review. Keep this
/// byte-compatible with existing graph nodes; payload hashes are checked
/// separately and must not silently become a new component of this identity.
pub(crate) fn source_candidate_identity(
    root_run_id: &str,
    scan_id: &str,
    attempt_number: i64,
    analysis_digest: &str,
    arguments: &serde_json::Map<String, JsonValue>,
) -> String {
    json!([
        root_run_id,
        scan_id,
        attempt_number,
        analysis_digest,
        arguments
            .get("path")
            .and_then(JsonValue::as_str)
            .unwrap_or(""),
        arguments
            .get("line")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0),
        arguments
            .get("title")
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .trim(),
        arguments
            .get("rationale")
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .trim(),
        arguments.get("rule"),
        arguments.get("severity"),
        arguments.get("cwe"),
        arguments.get("sourceAnalyzer"),
    ])
    .to_string()
}

/// A refusal that a model can act on: a stable code, and a message that says what the
/// allowed shape was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrokerDenial {
    pub code: &'static str,
    pub message: String,
}

impl BrokerDenial {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn as_json(&self) -> JsonValue {
        json!({"error": self.message, "code": self.code})
    }
}

#[derive(Clone, Debug)]
pub struct SourceBroker {
    pub snapshot: RepositorySnapshot,
    analysis_view: Option<SourceAnalysisView>,
    pub scan_id: String,
    pub attempt_number: i64,
    /// Scope key the analyzer SARIF was imported under — the broker reads results back
    /// from the canonical importer's own tables, never from analyzer stdout (§item 4).
    pub result_scope_key: String,
    pub root_run_id: String,
    pub run_id: String,
    pub revision: i64,
    submitted: usize,
    finished: Option<String>,
}

impl SourceBroker {
    // Historical unit fixtures exercise the isolated broker without publication.
    // Production has no constructor that can grant the full provenance snapshot.
    #[cfg(test)]
    pub fn new(
        snapshot: RepositorySnapshot,
        scan_id: &str,
        attempt_number: i64,
        root_run_id: &str,
        run_id: &str,
        revision: i64,
    ) -> Self {
        Self::build(
            snapshot,
            scan_id,
            attempt_number,
            root_run_id,
            run_id,
            revision,
        )
    }

    fn build(
        snapshot: RepositorySnapshot,
        scan_id: &str,
        attempt_number: i64,
        root_run_id: &str,
        run_id: &str,
        revision: i64,
    ) -> Self {
        Self {
            result_scope_key: format!("scan={scan_id}"),
            snapshot,
            analysis_view: None,
            scan_id: scan_id.to_string(),
            attempt_number,
            root_run_id: root_run_id.to_string(),
            run_id: run_id.to_string(),
            revision,
            submitted: 0,
            finished: None,
        }
    }

    pub fn scoped(
        snapshot: RepositorySnapshot,
        view: SourceAnalysisView,
        root_run_id: &str,
        run_id: &str,
        revision: i64,
    ) -> Result<Self, String> {
        view.verify(&snapshot)?;
        if view.manifest.scope == "unavailable" {
            return Err("source_analysis_scope_unavailable".into());
        }
        let mut broker = Self::build(
            snapshot,
            &view.manifest.scan_id,
            view.manifest.attempt_number,
            root_run_id,
            run_id,
            revision,
        );
        broker.analysis_view = Some(view);
        Ok(broker)
    }

    fn selected_files(&self) -> &[SnapshotFile] {
        self.analysis_view
            .as_ref()
            .map_or(&self.snapshot.files, |view| &view.manifest.files)
    }

    fn selected_hash(&self, path: &str) -> Option<&str> {
        self.selected_files()
            .iter()
            .find(|file| file.path == path)
            .map(|file| file.content_hash.as_str())
    }

    fn selected_manifests(&self) -> Vec<String> {
        self.snapshot
            .manifests
            .iter()
            .filter(|path| self.selected_hash(path).is_some())
            .cloned()
            .collect()
    }

    fn verify_scope(&self, connection: &Connection) -> Result<(), BrokerDenial> {
        self.snapshot
            .verify_frozen()
            .map_err(|error| BrokerDenial::new("snapshot_integrity", error))?;
        if let Some(view) = &self.analysis_view {
            view.verify(&self.snapshot)
                .and_then(|()| view.verify_receipt(connection))
                .and_then(|()| view.verify_source_receipt(connection))
                .map_err(|error| BrokerDenial::new("source_analysis_integrity", error))?;
        }
        Ok(())
    }

    pub fn submitted_candidates(&self) -> usize {
        self.submitted
    }

    pub fn is_finished(&self) -> bool {
        self.finished.is_some()
    }

    pub fn known_gaps(&self) -> Vec<String> {
        self.analysis_view.as_ref().map_or_else(
            || self.snapshot.gaps.clone(),
            |view| view.manifest.gaps.clone(),
        )
    }

    /// One entry point for every model tool call.
    pub fn call(
        &mut self,
        connection: &Connection,
        name: &str,
        arguments: &JsonValue,
    ) -> Result<JsonValue, BrokerDenial> {
        if !SOURCE_TOOLS.contains(&name) {
            return Err(self.unknown_tool(name));
        }
        let map = arguments
            .as_object()
            .ok_or_else(|| BrokerDenial::new("invalid_arguments", "工具参数必须是 JSON 对象"))?;
        for key in map.keys() {
            // A model that asks to confirm its own suspect gets the specific answer,
            // not a generic schema complaint (§10.2 item 10).
            if name == "evidence.submit_candidate"
                && matches!(key.as_str(), "status" | "confirmed" | "verdict")
            {
                return Err(BrokerDenial::new(
                    "confirmation_not_model_side",
                    "模型只能提交候选，confirmed/rejected 由评审决定写入",
                ));
            }
            if let Some((code, why)) = forbidden_key(key) {
                return Err(BrokerDenial::new(
                    code,
                    format!("{why}：字段 {key} 不在允许范围内"),
                ));
            }
            if !allowed_keys(name).contains(&key.as_str()) {
                return Err(BrokerDenial::new(
                    "unexpected_argument",
                    format!("{name} 不接受字段 {key}；允许的是 {:?}", allowed_keys(name)),
                ));
            }
        }
        for key in allowed_keys(name).iter().filter(|key| path_like(key)) {
            if let Some(value) = map.get(*key).and_then(JsonValue::as_str) {
                self.check_relative(value)?;
            }
        }
        self.verify_scope(connection)?;
        let result = match name {
            "repo.inventory" => self.inventory(map),
            "repo.search" => self.search(map),
            "repo.read_slice" => self.read_slice(map),
            "git.changed_files" => self.changed_files(),
            "analyzer.list_results" => self.list_results(connection, map),
            "analyzer.get_result" => self.get_result(connection, map),
            "callgraph.get_slice" => Err(BrokerDenial::new(
                "unsupported_capability",
                "本版本没有调用图分析器；这是一条覆盖缺口，不是没有发现",
            )),
            "dependency.get_record" => self.dependency_record(map),
            "evidence.submit_candidate" => self.submit_candidate(connection, map),
            "assignment.finish" => self.finish(connection, map),
            other => Err(BrokerDenial::new(
                "unknown_tool",
                format!("未注册的工具：{other}"),
            )),
        };
        // A failed handler is not an escape hatch around the post-read check.
        self.verify_scope(connection)?;
        result
    }

    fn unknown_tool(&self, name: &str) -> BrokerDenial {
        let lowered = name.to_ascii_lowercase();
        if [
            "shell",
            "exec",
            "bash",
            "run",
            "subprocess",
            "system",
            "eval",
        ]
        .iter()
        .any(|needle| lowered.contains(needle))
        {
            return BrokerDenial::new(
                "arbitrary_shell",
                format!("模型不能执行命令或自选二进制：{name} 不在源码工具白名单里"),
            );
        }
        if [
            "write", "patch", "delete", "remove", "edit", "append", "mkdir", "chmod",
        ]
        .iter()
        .any(|needle| lowered.contains(needle))
        {
            return BrokerDenial::new(
                "source_modification",
                format!("源码只读：{name} 不在源码工具白名单里"),
            );
        }
        if lowered.contains("download") || lowered.contains("fetch") || lowered.contains("http") {
            return BrokerDenial::new(
                "unapproved_network",
                format!("源码工具不联网：{name} 不在源码工具白名单里"),
            );
        }
        BrokerDenial::new(
            "unknown_tool",
            format!("未知工具 {name}；源码侧只允许 {}", SOURCE_TOOLS.join(", ")),
        )
    }
}
