//! Frozen source phase admission. Budgets are consequences of the versioned
//! phase contract, not a proxy for permission to execute a Reviewer.
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SourcePhaseContract {
    pub(crate) model_request_limit: i64,
    pub(crate) candidate_review: bool,
    pub(crate) candidate_decisions: bool,
    pub(crate) coverage_review: bool,
}

impl SourcePhaseContract {
    pub(crate) fn from_plan(plan: &Value) -> Result<Self, String> {
        // Old versions never inherit a phase or its additional model budget.
        if plan["schemaVersion"] != 4
            && (plan.get("sourceCoveragePhaseVersion").is_some()
                || plan.get("sourceCoverageDecisionPhaseVersion").is_some())
        {
            return Err("source_surface_frozen_plan_invalid".into());
        }
        let contract = match (
            plan["schemaVersion"].as_i64(),
            plan.get("sourceReviewPhaseVersion"),
            plan.get("sourceDecisionPhaseVersion"),
        ) {
            (Some(1), None, None) => Self {
                model_request_limit: 8,
                candidate_review: false,
                candidate_decisions: false,
                coverage_review: false,
            },
            (Some(2), Some(version), None) if *version == 1 => Self {
                model_request_limit: 9,
                candidate_review: true,
                candidate_decisions: false,
                coverage_review: false,
            },
            (Some(3), Some(review), Some(decision)) if *review == 1 && *decision == 1 => Self {
                model_request_limit: 9,
                candidate_review: true,
                candidate_decisions: true,
                coverage_review: false,
            },
            (Some(4), Some(review), Some(decision))
                if *review == 1
                    && *decision == 1
                    && plan["sourceCoveragePhaseVersion"] == 1
                    && plan["sourceCoverageDecisionPhaseVersion"] == 1 =>
            {
                Self {
                    model_request_limit: 10,
                    candidate_review: true,
                    candidate_decisions: true,
                    coverage_review: true,
                }
            }
            _ => return Err("source_surface_frozen_plan_invalid".into()),
        };
        if plan["sourceToolsPhaseVersion"] != 1
            || plan["modelRequestLimit"] != contract.model_request_limit
        {
            return Err("source_surface_frozen_plan_invalid".into());
        }
        Ok(contract)
    }
}

