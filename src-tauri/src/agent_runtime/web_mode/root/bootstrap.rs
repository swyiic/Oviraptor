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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    supervision_request_allocation_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    budget_consumption_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    http_budget_consumption_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    web_lease_expiry_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    capability_transition_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    authorization_output_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    authorization_output_request_capacity_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    external_surface_response_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    external_surface_output_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    external_surface_output_request_capacity_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    business_logic_input_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    business_logic_output_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    business_logic_output_request_capacity_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    business_logic_dispatch_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mapper_loop_rule: Option<String>,
}
impl BootstrapDispatch {
    pub(super) fn creation_contract() -> Self {
        // New Roots are born without the web-call budget-consumption advisory:
        // that post-hoc settlement step kept deferring (empty root suggestions)
        // and hard-stopping otherwise-successful scans, while real token billing
        // is already recorded through the receipt/entry path. Dropping only this
        // one advisory rule makes the consumption hook take its symmetric
        // rule-absent skip (never publishing and never verifying), so nothing
        // downstream is left half-settled. Historical Roots that were sealed with
        // the rule keep enforcing it (validate() still accepts that shape).
        Self::web_expiry_no_consumption_contract()
    }
    pub(super) fn original_v7_contract() -> Self {
        Self {
            schema_version: 7,
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
            supervision_request_allocation_rule: Some(
                crate::agent_runtime::multi_agent::budget::supervision_capacity::ALLOCATION_RULE
                    .into(),
            ),
            budget_consumption_rule: Some("original_settled_web_call_advisory_v1".into()),
            http_budget_consumption_rule: None,
            web_lease_expiry_rule: None,
            capability_transition_rule: None,
            authorization_output_rule: None,
            authorization_output_request_capacity_rule: None,
            external_surface_response_rule: None,
            external_surface_output_rule: None,
            external_surface_output_request_capacity_rule: None,
            business_logic_input_rule: None,
            business_logic_output_rule: None,
            business_logic_output_request_capacity_rule: None,
            business_logic_dispatch_rule: None,
            mapper_loop_rule: None,
        }
    }
    pub(super) fn http_creation_contract() -> Self {
        let mut value = Self::original_v7_contract();
        value.schema_version = 8;
        value.http_budget_consumption_rule = Some(crate::agent_runtime::web_mode::http_budget_receipt::RULE.into());
        value
    }
    pub(super) fn web_expiry_creation_contract() -> Self {
        let mut value = Self::http_creation_contract();
        value.schema_version = 9;
        value.web_lease_expiry_rule = Some("original_expired_web_fee_delivery_advisory_v1".into());
        value
    }
    // Same sealed v9 shape as web_expiry_creation_contract, but with the web-call
    // budget-consumption advisory rule removed. schema_version stays 9 so every
    // other v9 code path (and the `== 15` mapper-loop checks elsewhere) is
    // unaffected; only the consumption hook changes behaviour, taking its
    // rule-absent skip. This is the contract new Roots are created with.
    pub(super) fn web_expiry_no_consumption_contract() -> Self {
        let mut value = Self::web_expiry_creation_contract();
        value.budget_consumption_rule = None;
        value
    }
    pub(super) fn capability_creation_contract() -> Self {
        let mut value = Self::web_expiry_creation_contract();
        value.schema_version = 10;
        value.capability_transition_rule = Some("original_closed_web_capability_transition_advisory_v1".into());
        value
    }
    pub(super) fn authorization_output_creation_contract() -> Self {
        let mut value = Self::capability_creation_contract();
        value.schema_version = 11;
        value.authorization_output_rule = Some("original_closed_three_side_authorization_output_advisory_v1".into());
        value.authorization_output_request_capacity_rule = Some("retain_one_original_authorization_output_assessment_per_worker_v1".into());
        value
    }
    pub(super) fn external_surface_output_creation_contract() -> Self {
        let mut value = Self::authorization_output_creation_contract();
        value.schema_version = 12;
        value.external_surface_response_rule = Some("original_received_anonymous_public_surface_response_v1".into());
        value.external_surface_output_rule = Some("original_closed_anonymous_entry_surface_output_advisory_v1".into());
        value.external_surface_output_request_capacity_rule = Some("retain_one_original_surface_output_assessment_per_worker_v1".into());
        value
    }
    pub(super) fn business_logic_output_creation_contract() -> Self {
        let mut value = Self::external_surface_output_creation_contract();
        value.schema_version = 13;
        value.business_logic_input_rule = Some("original_frozen_business_logic_input_v1".into());
        value.business_logic_output_rule = Some("original_closed_readonly_business_logic_output_advisory_v1".into());
        value.business_logic_output_request_capacity_rule = Some("retain_one_original_business_logic_output_assessment_per_worker_v1".into());
        value
    }
    pub(super) fn business_logic_dispatch_creation_contract() -> Self {
        let mut value = Self::business_logic_output_creation_contract();
        value.schema_version = 14;
        value.business_logic_dispatch_rule = Some("original_paid_frozen_business_logic_dispatch_v1".into());
        value
    }
    pub(super) fn mapper_loop_creation_contract() -> Self {
        let mut value = Self::business_logic_dispatch_creation_contract();
        value.schema_version = 15;
        value.reserved_model_requests = 2;
        value.mapper_loop_rule = Some("original_frozen_mapper_two_round_one_lookup_v1".into());
        value
    }
    pub(super) fn validate(&self) -> Result<(), String> {
        let original_v6 = Self::original_v6_contract();
        let original_v5 = Self::original_v5_contract();
        let mut original_v4 = original_v5.clone();
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
            && self != &Self::web_expiry_no_consumption_contract()
            && self != &Self::original_v7_contract()
            && self != &Self::http_creation_contract()
            && self != &Self::web_expiry_creation_contract()
            && self != &Self::capability_creation_contract()
            && self != &Self::authorization_output_creation_contract()
            && self != &Self::external_surface_output_creation_contract()
            && self != &Self::business_logic_output_creation_contract()
            && self != &Self::business_logic_dispatch_creation_contract()
            && self != &Self::mapper_loop_creation_contract()
            && self != &original_v6
            && self != &original_v5
            && self != &original_v4
            && self != &original_v3
            && self != &original_v2
            && self != &original_native
        {
            return Err("root_bootstrap_dispatch_contract_invalid".into());
        }
        Ok(())
    }
    pub(super) fn original_v5_contract() -> Self {
        let mut value = Self::original_v6_contract();
        value.schema_version = 5;
        value.supervision_request_allocation_rule = None;
        value
    }
    pub(super) fn original_v6_contract() -> Self {
        let mut value = Self::original_v7_contract();
        value.schema_version = 6;
        value.budget_consumption_rule = None;
        value
    }
    pub(super) fn as_json(&self) -> Value {
        json!(self)
    }
}

