//! Fresh ordinary Root contract; no execution or renewal authority.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct LeaseExpirySupervision {
    schema_version: u8,
    source: String,
    closure: String,
    deduplication: String,
    permitted_step: String,
    execution_grant: bool,
}
impl LeaseExpirySupervision {
    pub(super) fn creation_contract() -> Self {
        Self {
            schema_version: 1,
            source: "original_signed_worker_expiry_receipt".into(),
            closure: "original_cost_settled_saved_and_acknowledged".into(),
            deduplication: "original_receipt_physical_fact".into(),
            permitted_step: "assess:lease_expired".into(),
            execution_grant: false,
        }
    }
    pub(super) fn validate(&self) -> Result<(), String> {
        if *self != Self::creation_contract() {
            return Err("root_lease_expiry_supervision_contract_invalid".into());
        }
        Ok(())
    }
    pub(super) fn as_json(&self) -> Value { json!(self) }
}

