//! Four-layer context assembly and deterministic compaction (§5).
//!
//! Raw HTTP bodies, JS and screenshots never enter a prompt: they live in the
//! artifact store and reach the model only as ids plus a structural summary.
use super::gateway::truncate;
use serde_json::Value as JsonValue;

pub const CONTEXT_KEEP_RECENT_ROUNDS: usize = 2;

pub struct ContextLayers {
    pub system_rules: String,
    pub schema_hash: String,
    /// Layer 2: this agent's role, budget lease and task slice.
    pub agent_slice: JsonValue,
    /// Layer 3: confirmed, excluded, pending and coverage gaps.
    pub facts: JsonValue,
    /// Layer 4: the last rounds of tool input/output summaries.
    pub recent_rounds: Vec<JsonValue>,
    /// Opaque artifact ids the model may query through tools.
    pub artifact_refs: Vec<String>,
}

impl ContextLayers {
    // Phase 2/3 consumes this; the rules around it are covered by tests now.
    #[allow(dead_code)]
    pub fn messages(&self) -> Vec<JsonValue> {
        let mut messages = vec![
            serde_json::json!({
                "role": "system",
                "content": format!(
                    "{}\n\n工具契约版本：{}",
                    self.system_rules.trim(),
                    &self.schema_hash[..16.min(self.schema_hash.len())]
                ),
                "oviraptorKeep": true,
            }),
            serde_json::json!({
                "role": "user",
                "content": serde_json::json!({
                    "agentSlice": self.agent_slice,
                    "facts": self.facts,
                    "artifactRefs": self.artifact_refs,
                })
                .to_string(),
                "oviraptorKeep": true,
            }),
        ];
        messages.extend(self.recent_rounds.iter().cloned());
        messages
    }
}

/// Shrink a message list to a byte budget without dropping the rules or the
/// live state block. Returns the new messages and whether anything changed.
pub fn compact_messages(messages: &[JsonValue], budget_bytes: usize) -> (Vec<JsonValue>, bool) {
    fn total(rows: &[JsonValue]) -> usize {
        rows.iter().map(|value| value.to_string().len()).sum()
    }
    let mut rendered: Vec<JsonValue> = messages.to_vec();
    if total(&rendered) <= budget_bytes {
        return (rendered, false);
    }
    let tool_positions: Vec<usize> = rendered
        .iter()
        .enumerate()
        .filter(|(_, value)| value.get("role").and_then(JsonValue::as_str) == Some("tool"))
        .map(|(index, _)| index)
        .collect();
    if tool_positions.len() > CONTEXT_KEEP_RECENT_ROUNDS {
        for index in &tool_positions[..tool_positions.len() - CONTEXT_KEEP_RECENT_ROUNDS] {
            let digest = rendered[*index]
                .get("content")
                .and_then(JsonValue::as_str)
                .map(|text| format!("[已收口证据摘要] {}", digest_text(text, 320)))
                .unwrap_or_default();
            rendered[*index]["content"] = JsonValue::String(digest);
        }
    }
    if total(&rendered) <= budget_bytes {
        return (rendered, true);
    }
    // Older assistant narration is next. Kept blocks (rules, live state) stay.
    let droppable: Vec<usize> = rendered
        .iter()
        .enumerate()
        .filter(|(_, value)| {
            value.get("oviraptorKeep").and_then(JsonValue::as_bool) != Some(true)
                && value.get("role").and_then(JsonValue::as_str) == Some("assistant")
        })
        .map(|(index, _)| index)
        .collect();
    let keep_from = droppable.len().saturating_sub(CONTEXT_KEEP_RECENT_ROUNDS);
    for index in &droppable[..keep_from] {
        rendered[*index] = serde_json::json!({
            "role": "assistant",
            "content": "[已完成分支，细节已折叠到覆盖账本]",
        });
    }
    if total(&rendered) > budget_bytes {
        let per_field = (budget_bytes / 8).max(512);
        for value in rendered.iter_mut() {
            if value.get("oviraptorKeep").and_then(JsonValue::as_bool) == Some(true) {
                continue;
            }
            if let Some(text) = value.get("content").and_then(JsonValue::as_str) {
                if text.chars().count() > per_field {
                    value["content"] = JsonValue::String(digest_text(text, per_field));
                }
            }
        }
    }
    (rendered, true)
}

/// A deterministic summary; the model is never asked to spend a call
/// summarising its own history (§5).
pub fn digest_text(text: &str, chars: usize) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&text.len(), &mut hasher);
    std::hash::Hash::hash(text, &mut hasher);
    format!(
        "{}…[sha {:016x}]",
        truncate(text, chars),
        std::hash::Hasher::finish(&hasher)
    )
}
