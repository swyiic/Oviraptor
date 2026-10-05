//! Deterministic local tools over original captured facts; no SQL/IO dispatcher.
use super::decision::DecisionSummary;
use crate::agent_runtime::{
    model::{gateway::ModelResponse, ToolSchema},
    secrets,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LocalStep {
    calls: Vec<LocalResult>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalResult {
    id: String,
    name: String,
    arguments: Value,
    result: Value,
}
pub(crate) fn schemas() -> Vec<ToolSchema> {
    let empty = json!({"type":"object","properties":{},"additionalProperties":false});
    vec![
        ToolSchema::new("snapshot.read", "Read the captured Root snapshot. Does not refresh or execute a target.",empty.clone()),
        ToolSchema::new("evidence.read", "Read the captured, redacted evidence only. No file paths, URLs or arbitrary queries.",empty.clone()),
        ToolSchema::new("capability_budget.read", "Read the original hard ceilings and coordination capability. These ceilings are not live remaining balances or grants; Rust checks the current ledger on every admission.",empty),
        ToolSchema::new("plan.propose", "Propose the offered step or defer. This is advisory and never dispatches an assignment.",json!({"type":"object","properties":{"step":{"type":"string","maxLength":64}},"required":["step"],"additionalProperties":false})),
    ]
}
impl LocalStep {
    pub(crate) fn parse(response: &ModelResponse, basis: &Value) -> Result<Option<Self>, String> {
        if response.tool_calls.is_empty() {
            return Ok(None);
        }
        let remaining = 4usize
            .checked_sub(
                basis["localCallsUsed"]
                    .as_u64()
                    .ok_or("root_local_call_budget_invalid")? as usize,
            )
            .ok_or("root_local_call_limit")?;
        if response.finish_reason != "tool_calls"
            || response.tool_calls.len() > remaining
            || response.tool_calls.len() > 4
        {
            return Err("root_local_call_limit_or_response_invalid".into());
        }
        let mut calls = Vec::new();
        for call in &response.tool_calls {
            if call.id.is_empty()
                || call.id.len() > 64
                || !call
                    .id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
                || secrets::redact_text_with(&call.id, None) != call.id
                || calls.iter().any(|c: &LocalResult| c.id == call.id)
                || basis["localHistory"].as_array().is_some_and(|h| {
                    h.iter().any(|s| {
                        s["calls"]
                            .as_array()
                            .is_some_and(|cs| cs.iter().any(|c| c["id"] == call.id))
                    })
                })
            {
                return Err("root_local_tool_id_invalid".into());
            }
            let result = execute(&call.name, &call.arguments, basis)?;
            calls.push(LocalResult {
                id: call.id.clone(),
                name: call.name.clone(),
                arguments: call.arguments.clone(),
                result,
            });
        }
        Ok(Some(Self { calls }))
    }
    pub(crate) fn verify(&self, basis: &Value) -> Result<(), String> {
        let response = ModelResponse {
            tool_calls: self
                .calls
                .iter()
                .map(|c| crate::agent_runtime::model::gateway::ToolCall {
                    id: c.id.clone(),
                    name: c.name.clone(),
                    arguments: c.arguments.clone(),
                })
                .collect(),
            finish_reason: "tool_calls".into(),
            ..Default::default()
        };
        if Self::parse(&response, basis)?.as_ref() != Some(self) {
            return Err("root_local_result_changed".into());
        }
        Ok(())
    }
    pub(crate) fn summary(&self) -> Result<DecisionSummary, String> {
        DecisionSummary::from_json(&json!({"schemaVersion":1,
            "observed":["Requested bounded local facts or an advisory plan; no target action was executed."],
            "missing":[],"suggestions":[],"costNotes":[],"risks":[]}))
    }
    pub(crate) fn tool_names(&self) -> Vec<&str> {
        self.calls.iter().map(|call| call.name.as_str()).collect()
    }
    pub(crate) fn count(&self) -> usize {
        self.calls.len()
    }
    pub(crate) fn as_json(&self) -> Value {
        json!(self)
    }
    pub(crate) fn from_json(value: &Value, basis: &Value) -> Result<Self, String> {
        let result: Self =
            serde_json::from_value(value.clone()).map_err(|_| "root_local_result_invalid")?;
        result.verify(basis)?;
        if result.as_json() != *value {
            return Err("root_local_result_invalid".into());
        }
        Ok(result)
    }
    pub(crate) fn messages(&self) -> Vec<Value> {
        let mut result = vec![
            json!({"role":"assistant","tool_calls":self.calls.iter().map(|c|json!({
            "id":c.id,"type":"function","function":{"name":c.name,"arguments":c.arguments.to_string()}})).collect::<Vec<_>>()}),
        ];
        result.extend(
            self.calls
                .iter()
                .map(|c| json!({"role":"tool","tool_call_id":c.id,"content":c.result.to_string()})),
        );
        result
    }
}
fn execute(name: &str, args: &Value, basis: &Value) -> Result<Value, String> {
    let object = args
        .as_object()
        .ok_or("root_local_tool_arguments_invalid")?;
    let result = match name {
        "snapshot.read" if object.is_empty() => {
            let mut snapshot = basis.clone();
            let object = snapshot
                .as_object_mut()
                .ok_or("root_local_snapshot_invalid")?;
            object.remove("localHistory");
            object.remove("localParents"); // Internal physical proofs are not model context.
            snapshot
        }
        "evidence.read" if object.is_empty() => {
            json!({"frozenEvidence":basis["frozenEvidence"],"changedFact":basis["changedFact"]})
        }
        "capability_budget.read" if object.is_empty() => {
            let mut value=json!({"capabilityClass":basis["capabilityClass"],
            "originalHardLimits":basis["originalHardLimits"],"deliberationContract":basis["localDeliberation"],
            "remainingBalance":"not represented by hard ceilings; current ledger admission is enforced by Rust", "targetGrant":false});
            if let Some(snapshot)=basis.get("budgetSnapshot") {value["budgetSnapshot"]=snapshot.clone();}
            value
        }
        "plan.propose" if object.len() == 1 && object.contains_key("step") => {
            let step = args["step"]
                .as_str()
                .ok_or("root_local_tool_arguments_invalid")?;
            if step != "defer" && Some(step) != basis["permittedSuggestion"].as_str() {
                return Err("root_local_proposal_not_offered".into());
            }
            json!({"step":step,"advisoryOnly":true,"dispatched":false})
        }
        _ => return Err("root_local_tool_denied".into()),
    };
    let safe = secrets::redact_json(&result);
    if safe.to_string().len() > 96 * 1024 {
        return Err("root_local_result_too_large".into());
    }
    Ok(safe)
}
