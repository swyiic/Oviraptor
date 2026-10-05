//! Creation-only local Root deliberation. This grants no target capability.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LocalDeliberation {
    schema_version: u8,
    model_round_limit: u8,
    local_call_limit: u8,
    context_byte_limit: u32,
    reviewer_token_floor: i64,
    reviewer_request_floor: i64,
    tools: Vec<String>,
}
impl LocalDeliberation {
    pub(crate) fn creation_contract() -> Self {
        Self {
            schema_version: 1,
            model_round_limit: 3,
            local_call_limit: 4,
            context_byte_limit: 96 * 1024,
            reviewer_token_floor: 15_000,
            reviewer_request_floor: 1,
            tools: [
                "snapshot.read",
                "evidence.read",
                "capability_budget.read",
                "plan.propose",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
        }
    }
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self != &Self::creation_contract() {
            return Err("root_local_deliberation_contract_invalid".into());
        }
        Ok(())
    }
    pub(crate) fn as_json(&self) -> Value {
        json!(self)
    }
}
