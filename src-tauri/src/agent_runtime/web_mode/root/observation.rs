//! New creation-only observations; frozen snapshots convey no execution grant.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct LiveBudgetObservation {
    schema_version: u8,
    snapshot_timing: String,
    execution_grant: bool,
}
impl LiveBudgetObservation {
    pub(super) fn creation_contract() -> Self {
        Self {
            schema_version: 1,
            snapshot_timing: "before_each_original_model_call".into(),
            execution_grant: false,
        }
    }
    pub(super) fn validate(&self) -> Result<(), String> {
        if *self != Self::creation_contract() {
            return Err("root_budget_observation_contract_invalid".into());
        }
        Ok(())
    }
    pub(super) fn as_json(&self) -> Value {
        json!(self)
    }
}
