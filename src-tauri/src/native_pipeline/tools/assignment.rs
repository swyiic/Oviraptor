//! Candidate evidence and assignment completion; review remains independent.

use super::{source_candidate_identity, BrokerDenial, SourceBroker};
use crate::agent_runtime::evidence_graph::contract::{
    EvidenceNode, EvidenceNodeKind, EvidenceProvenance,
};
use crate::agent_runtime::evidence_graph::store::{evidence_natural_key, insert_evidence_node};
use crate::agent_runtime::store::stable_hash;
use rusqlite::{params, Connection};
use serde_json::{json, Value as JsonValue};

impl SourceBroker {
    pub(super) fn submit_candidate(
        &mut self,
        connection: &Connection,
        map: &serde_json::Map<String, JsonValue>,
    ) -> Result<JsonValue, BrokerDenial> {
        // A model may name a suspect, never close one: confirmation belongs to the
        // reviewer decision, so the very field is refused.
        if map.contains_key("status")
            || map.contains_key("confirmed")
            || map.contains_key("verdict")
        {
            return Err(BrokerDenial::new(
                "confirmation_not_model_side",
                "模型只能提交候选，confirmed/rejected 由评审决定写入",
            ));
        }
        let title = map
            .get("title")
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                BrokerDenial::new("invalid_arguments", "evidence.submit_candidate 需要 title")
            })?
            .to_string();
        let rationale = map
            .get("rationale")
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                BrokerDenial::new("invalid_arguments", "候选必须给出理由，否则无法复核")
            })?
            .to_string();
        let path = map
            .get("path")
            .and_then(JsonValue::as_str)
            .map(Self::normalized)
            .unwrap_or_default();
        if !path.is_empty() && self.selected_hash(&path).is_none() {
            return Err(BrokerDenial::new(
                "path_not_in_snapshot",
                format!("候选引用的 {path} 不在冻结快照里"),
            ));
        }
        let line = map.get("line").and_then(JsonValue::as_u64).unwrap_or(0);
        if self.analysis_view.is_some() {
            if path.is_empty() || line == 0 {
                return Err(BrokerDenial::new(
                    "source_location_required",
                    "源码候选必须绑定获准文件与正整数行号",
                ));
            }
            let text = self.read_frozen(&path)?;
            if line > text.lines().count() as u64 {
                return Err(BrokerDenial::new(
                    "source_line_out_of_bounds",
                    "候选行号超出冻结文件内容",
                ));
            }
        }
        let mut identity = format!(
            "{}|{}|{}",
            self.scan_id,
            self.attempt_number,
            title.to_lowercase()
        );
        if let Some(view) = &self.analysis_view {
            // Same title at different locations is not the same candidate. Bind
            // the actual input and supporting claim, not a mutable display label.
            identity = source_candidate_identity(
                &self.root_run_id,
                &self.scan_id,
                self.attempt_number,
                &view.manifest.digest(),
                map,
            );
        }
        let hash = stable_hash(&identity);
        let node = EvidenceNode {
            id: format!("cand-{}", &hash[..16]),
            root_run_id: self.root_run_id.clone(),
            revision: self.revision,
            kind: EvidenceNodeKind::CandidateFinding,
            provenance: EvidenceProvenance::SourceDerived,
            natural_key_hash: evidence_natural_key(
                &self.root_run_id,
                EvidenceNodeKind::CandidateFinding,
                &identity,
            ),
            payload: json!({
                "title": title,
                "rationale": rationale,
                "severity": map.get("severity").and_then(JsonValue::as_str).unwrap_or("informational"),
                "rule": map.get("rule").and_then(JsonValue::as_str).unwrap_or("model-suspicion"),
                "cwe": map.get("cwe").and_then(JsonValue::as_str).unwrap_or(""),
                "path": path,
                "line": line,
                "contentHash": self.selected_hash(&path),
                "analysisManifestDigest": self.analysis_view.as_ref().map(|view|view.manifest.digest()),
                "scanId": self.scan_id,
                "attemptNumber": self.attempt_number,
                "sourceAnalyzer": map.get("sourceAnalyzer").and_then(JsonValue::as_str).unwrap_or("model"),
                "treeHash": self.snapshot.tree_hash,
                "reviewState": "candidate",
            }),
            artifact_refs: vec![self.snapshot.tree_hash.clone()],
            created_by_run_id: self.run_id.clone(),
            supersedes_id: String::new(),
            created_at: String::new(),
        };
        let id = match insert_evidence_node(connection, &node)
            .map_err(|error| BrokerDenial::new("internal_error", error))?
        {
            crate::agent_runtime::evidence_graph::store::EvidenceInsert::Inserted(id) => id,
            crate::agent_runtime::evidence_graph::store::EvidenceInsert::Existing(id) => id,
        };
        self.submitted += 1;
        Ok(json!({
            "id": id,
            "reviewState": "candidate",
            "confirmable": false,
            "message": "已登记为候选，等待评审决定",
        }))
    }

    pub(super) fn finish(
        &mut self,
        connection: &Connection,
        map: &serde_json::Map<String, JsonValue>,
    ) -> Result<JsonValue, BrokerDenial> {
        if self.finished.is_none() {
            let summary = map
                .get("summary")
                .and_then(JsonValue::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    BrokerDenial::new("invalid_arguments", "assignment.finish 需要 summary")
                })?
                .to_string();
            self.finished = Some(summary);
        }
        let mut gaps = self.known_gaps();
        // A model finishing a source assignment is not an independent review.
        // Never let empty analyzer gaps manufacture a conclusive source result.
        if self.analysis_view.is_some()
            && !gaps.iter().any(|gap| gap == "source_review_not_completed")
        {
            gaps.push("source_review_not_completed".into());
        }
        if let Some(rows) = map.get("gaps").and_then(JsonValue::as_array) {
            for row in rows.iter().filter_map(JsonValue::as_str) {
                let text = row.trim().to_string();
                if !text.is_empty() && !gaps.contains(&text) {
                    gaps.push(text);
                }
            }
        }
        // The count comes from the graph, not from this call, because a resumed run
        // rebuilds the broker from the frozen snapshot.
        let candidates: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM agent_evidence_nodes
                 WHERE root_run_id=?1 AND revision=?2 AND kind='candidate_finding'",
                params![self.root_run_id, self.revision],
                |row| row.get(0),
            )
            .map_err(|error| {
                BrokerDenial::new("internal_error", format!("无法统计候选：{error}"))
            })?;
        Ok(json!({
            "status": "finished",
            "summary": self.finished.clone().unwrap_or_default(),
            "candidates": candidates,
            "gaps": gaps,
            "conclusive": gaps.is_empty(),
        }))
    }
}
