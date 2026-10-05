//! Bounded user-visible semantics. Never store raw assistant messages/reasoning.
use crate::agent_runtime::{model::gateway::ModelResponse, secrets};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

mod safe_marker;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DecisionSummary {
    schema_version: u8,
    observed: Vec<String>,
    missing: Vec<String>,
    suggestions: Vec<String>,
    cost_notes: Vec<String>,
    risks: Vec<String>,
}
impl DecisionSummary {
    pub(crate) fn schema() -> Value {
        json!({"schemaVersion":1,"observed":["evidence-backed observation"],
            "missing":["missing fact"],"suggestions":["advisory next step; no authorization"],
            "costNotes":["bounded cost caveat; actual cost comes from receipts"],
            "risks":["risk supported by the frozen facts"]})
    }
    pub(crate) fn parse_response(response: &ModelResponse) -> Result<Self, String> {
        if !response.tool_calls.is_empty()
            || response.finish_reason != "stop"
            || response.text.len() > 48 * 1024
        {
            return Err("root_tick_decision_invalid".into());
        }
        let result: Self =
            serde_json::from_str(&response.text).map_err(|_| "root_tick_decision_invalid")?;
        result.validate()?;
        // Redact semantic strings before persistence. Financial responseHash
        // still identifies the original response bytes without retaining them.
        let mut safe = result;
        for list in [
            &mut safe.observed,
            &mut safe.missing,
            &mut safe.suggestions,
            &mut safe.cost_notes,
            &mut safe.risks,
        ] {
            for text in list {
                *text = secrets::redact_text_with(text, None);
            }
        }
        safe.validate()?;
        Ok(safe)
    }
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err("root_tick_decision_invalid".into());
        }
        for list in [
            &self.observed,
            &self.missing,
            &self.suggestions,
            &self.cost_notes,
            &self.risks,
        ] {
            if list.len() > 16
                || list.iter().any(|text| {
                    text.trim().is_empty()
                        || text.chars().count() > 512
                        || text.chars().any(|c| c.is_control())
                })
            {
                return Err("root_tick_decision_invalid".into());
            }
        }
        Ok(())
    }
    pub(crate) fn as_json(&self) -> Value {
        serde_json::to_value(self).expect("bounded strings")
    }
    pub(crate) fn from_json(value: &Value) -> Result<Self, String> {
        let summary: Self =
            serde_json::from_value(value.clone()).map_err(|_| "root_tick_decision_invalid")?;
        summary.validate()?;
        for list in [
            &summary.observed,
            &summary.missing,
            &summary.suggestions,
            &summary.cost_notes,
            &summary.risks,
        ] {
            if list.iter().any(|text| !safe_marker::verified_safe(text)) {
                return Err("root_tick_decision_unredacted".into());
            }
        }
        if summary.as_json() != *value {
            return Err("root_tick_decision_invalid".into());
        }
        Ok(summary)
    }
}
