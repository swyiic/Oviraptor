//! Birth-only contract; no protection receipt, target authority, or execution grant.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ProtectionSupervision {
    schema_version: u8,
    source: String,
    closure: String,
    deduplication: String,
    permitted_step: String,
    execution_grant: bool,
}
impl ProtectionSupervision {
    pub(super) fn creation_contract() -> Self {
        Self {
            schema_version: 1,
            source: "original_signed_protection_response_receipt".into(),
            closure: "original_cost_settled_saved_and_acknowledged".into(),
            deduplication: "original_response_physical_fact".into(),
            permitted_step: "assess:protection_changed".into(),
            execution_grant: false,
        }
    }
    pub(super) fn validate(&self) -> Result<(), String> {
        if *self != Self::creation_contract() {
            return Err("root_protection_supervision_contract_invalid".into());
        }
        Ok(())
    }
    pub(super) fn as_json(&self) -> Value {
        json!(self)
    }
}

