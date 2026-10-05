//! Original creation-only Mapper/Executor admission contract, never an existing-root upgrade.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct BootstrapDispatch {
    schema_version: u8,
    permitted_step: String,
    reserved_model_tokens: i64,
    reserved_model_requests: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    allocation_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    executor_allocation_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    root_supervision_token_floor: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    root_supervision_request_floor: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model_cost_rule: Option<String>,
}
impl BootstrapDispatch {
    pub(super) fn creation_contract() -> Self {
        Self {
            schema_version: 5,
            permitted_step: "dispatch:spa_api_mapper".into(),
            reserved_model_tokens: 8_000,
            reserved_model_requests: 1,
            allocation_rule: Some("current_balance_after_reviewer_floor".into()),
            executor_allocation_rule: Some(
                "current_balance_after_reviewer_floor_and_ordered_proposals".into(),
            ),
            root_supervision_token_floor: Some(16_000),
            root_supervision_request_floor: Some(1),
            model_cost_rule: Some("finite_grant_share_native_unlimited_no_scarcity".into()),
        }
    }
    pub(super) fn validate(&self) -> Result<(), String> {
        let mut original_v4 = Self::creation_contract();
        original_v4.schema_version = 4;
        original_v4.model_cost_rule = None;
        let mut original_v3 = original_v4.clone();
        original_v3.schema_version = 3;
        original_v3.root_supervision_token_floor = None;
        original_v3.root_supervision_request_floor = None;
        let mut original_v2 = original_v3.clone();
        original_v2.schema_version = 2;
        original_v2.executor_allocation_rule = None;
        let mut original_native = original_v2.clone();
        original_native.schema_version = 1;
        original_native.allocation_rule = None;
        if self != &Self::creation_contract()
            && self != &original_v4
            && self != &original_v3
            && self != &original_v2
            && self != &original_native
        {
            return Err("root_bootstrap_dispatch_contract_invalid".into());
        }
        Ok(())
    }
    pub(super) fn as_json(&self) -> Value {
        json!(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mapper_creation_contract_keeps_original_native_json_and_rejects_rule_mutations() {
        let raw = r#"{"schemaVersion":1,"permittedStep":"dispatch:spa_api_mapper","reservedModelTokens":8000,"reservedModelRequests":1}"#;
        let original: BootstrapDispatch = serde_json::from_str(raw).unwrap();
        original.validate().unwrap();
        assert_eq!(serde_json::to_string(&original).unwrap(), raw);
        let raw_v2 = r#"{"schemaVersion":2,"permittedStep":"dispatch:spa_api_mapper","reservedModelTokens":8000,"reservedModelRequests":1,"allocationRule":"current_balance_after_reviewer_floor"}"#;
        let original_v2: BootstrapDispatch = serde_json::from_str(raw_v2).unwrap();
        original_v2.validate().unwrap();
        assert_eq!(serde_json::to_string(&original_v2).unwrap(), raw_v2);
        let raw_v3 = r#"{"schemaVersion":3,"permittedStep":"dispatch:spa_api_mapper","reservedModelTokens":8000,"reservedModelRequests":1,"allocationRule":"current_balance_after_reviewer_floor","executorAllocationRule":"current_balance_after_reviewer_floor_and_ordered_proposals"}"#;
        let original_v3: BootstrapDispatch = serde_json::from_str(raw_v3).unwrap();
        original_v3.validate().unwrap();
        assert_eq!(serde_json::to_string(&original_v3).unwrap(), raw_v3);
        let fresh = BootstrapDispatch::creation_contract();
        fresh.validate().unwrap();
        assert_eq!(fresh.as_json()["schemaVersion"], 5);
        let raw_v4 = r#"{"schemaVersion":4,"permittedStep":"dispatch:spa_api_mapper","reservedModelTokens":8000,"reservedModelRequests":1,"allocationRule":"current_balance_after_reviewer_floor","executorAllocationRule":"current_balance_after_reviewer_floor_and_ordered_proposals","rootSupervisionTokenFloor":16000,"rootSupervisionRequestFloor":1}"#;
        let original_v4: BootstrapDispatch = serde_json::from_str(raw_v4).unwrap();
        original_v4.validate().unwrap();
        assert_eq!(serde_json::to_string(&original_v4).unwrap(), raw_v4);
        let mut missing_rule = fresh.as_json();
        missing_rule
            .as_object_mut()
            .unwrap()
            .remove("modelCostRule");
        assert!(serde_json::from_value::<BootstrapDispatch>(missing_rule)
            .unwrap()
            .validate()
            .is_err());
        for old in [&original, &original_v2, &original_v3, &original_v4] {
            let mut injected = old.as_json();
            injected["modelCostRule"] = fresh.as_json()["modelCostRule"].clone();
            assert!(serde_json::from_value::<BootstrapDispatch>(injected)
                .unwrap()
                .validate()
                .is_err());
        }
        for (key, value) in [
            ("schemaVersion", json!(6)),
            ("modelCostRule", json!("free_model_calls")),
            ("allocationRule", json!("hard_ceiling")),
            ("reservedModelTokens", json!(9000)),
            ("reservedModelRequests", json!(2)),
        ] {
            let mut altered = fresh.as_json();
            altered[key] = value;
            assert!(serde_json::from_value::<BootstrapDispatch>(altered)
                .unwrap()
                .validate()
                .is_err());
        }
        for field in ["rootSupervisionTokenFloor", "rootSupervisionRequestFloor"] {
            let mut missing = fresh.as_json();
            missing.as_object_mut().unwrap().remove(field);
            assert!(serde_json::from_value::<BootstrapDispatch>(missing)
                .unwrap()
                .validate()
                .is_err());
            let mut injected = original_v3.as_json();
            injected[field] = fresh.as_json()[field].clone();
            assert!(serde_json::from_value::<BootstrapDispatch>(injected)
                .unwrap()
                .validate()
                .is_err());
        }
        let mut upgraded = original_v3.as_json();
        upgraded["rootSupervisionTokenFloor"] = json!(16_000);
        upgraded["rootSupervisionRequestFloor"] = json!(1);
        assert!(serde_json::from_value::<BootstrapDispatch>(upgraded)
            .unwrap()
            .validate()
            .is_err());
        let mut missing_executor = fresh.as_json();
        missing_executor
            .as_object_mut()
            .unwrap()
            .remove("executorAllocationRule");
        assert!(
            serde_json::from_value::<BootstrapDispatch>(missing_executor)
                .unwrap()
                .validate()
                .is_err()
        );
        let mut old_v2 = original_v2.as_json();
        old_v2["executorAllocationRule"] = fresh.as_json()["executorAllocationRule"].clone();
        assert!(serde_json::from_value::<BootstrapDispatch>(old_v2)
            .unwrap()
            .validate()
            .is_err());
        let mut missing = fresh.as_json();
        missing.as_object_mut().unwrap().remove("allocationRule");
        assert!(serde_json::from_value::<BootstrapDispatch>(missing)
            .unwrap()
            .validate()
            .is_err());
        let mut old = original.as_json();
        old["allocationRule"] = fresh.as_json()["allocationRule"].clone();
        assert!(serde_json::from_value::<BootstrapDispatch>(old)
            .unwrap()
            .validate()
            .is_err());
    }
}
