//! Creation-only protocol. Its presence conveys neither C nor target authority.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct NewEvidenceSupervision {
    schema_version: u8,
    source: String,
    deduplication: String,
    permitted_step: String,
    execution_grant: bool,
}
impl NewEvidenceSupervision {
    pub(super) fn creation_contract() -> Self {
        Self {
            schema_version: 1,
            source: "closed_web_executor_original_http_receipt".into(),
            deduplication: "original_node_physical_fact".into(),
            permitted_step: "assess:new_evidence".into(),
            execution_grant: false,
        }
    }
    pub(super) fn validate(&self) -> Result<(), String> {
        if *self != Self::creation_contract() {
            return Err("root_new_evidence_supervision_contract_invalid".into());
        }
        Ok(())
    }
    pub(super) fn as_json(&self) -> Value {
        json!(self)
    }
}

