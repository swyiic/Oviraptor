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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn plan(version: i64) -> Value {
        let mut value = json!({"schemaVersion":version,"sourceToolsPhaseVersion":1,
            "modelRequestLimit":if version == 1 {8} else {9}});
        if version >= 2 {
            value["sourceReviewPhaseVersion"] = json!(1);
        }
        if version >= 3 {
            value["sourceDecisionPhaseVersion"] = json!(1);
        }
        value
    }

    #[test]
    fn source_phase_contract_preserves_each_historical_version() {
        for (version, requests, review, decisions) in [
            (1, 8, false, false),
            (2, 9, true, false),
            (3, 9, true, true),
        ] {
            let frozen = plan(version);
            let original = frozen.to_string();
            assert_eq!(
                SourcePhaseContract::from_plan(&frozen).unwrap(),
                SourcePhaseContract {
                    model_request_limit: requests,
                    candidate_review: review,
                    candidate_decisions: decisions,
                    coverage_review: false,
                }
            );
            assert_eq!(
                frozen.to_string(),
                original,
                "reading never migrates a plan"
            );
        }
    }

    #[test]
    fn source_phase_contract_budget_never_grants_a_phase() {
        for version in 1..=3 {
            for limit in [-1, 0, 8, 9, 10, i64::MAX] {
                let mut frozen = plan(version);
                frozen["modelRequestLimit"] = json!(limit);
                assert_eq!(
                    SourcePhaseContract::from_plan(&frozen).is_ok(),
                    limit == if version == 1 { 8 } else { 9 },
                    "version {version}, limit {limit}"
                );
            }
        }
    }

    #[test]
    fn source_phase_contract_v4_requires_explicit_coverage_and_publication_versions() {
        let mut frozen = plan(3);
        frozen["schemaVersion"] = json!(4);
        frozen["modelRequestLimit"] = json!(10);
        frozen["sourceCoveragePhaseVersion"] = json!(1);
        frozen["sourceCoverageDecisionPhaseVersion"] = json!(1);
        assert_eq!(
            SourcePhaseContract::from_plan(&frozen).unwrap(),
            SourcePhaseContract {
                model_request_limit: 10,
                candidate_review: true,
                candidate_decisions: true,
                coverage_review: true,
            }
        );
        for key in [
            "sourceCoveragePhaseVersion",
            "sourceCoverageDecisionPhaseVersion",
            "sourceReviewPhaseVersion",
            "sourceDecisionPhaseVersion",
        ] {
            for bad in [Value::Null, json!(false), json!("1"), json!(0), json!(2)] {
                let mut changed = frozen.clone();
                changed[key] = bad;
                assert!(SourcePhaseContract::from_plan(&changed).is_err(), "{key}");
            }
            let mut missing = frozen.clone();
            missing.as_object_mut().unwrap().remove(key);
            assert!(SourcePhaseContract::from_plan(&missing).is_err());
        }
        for budget in [8, 9, 11] {
            let mut changed = frozen.clone();
            changed["modelRequestLimit"] = json!(budget);
            assert!(SourcePhaseContract::from_plan(&changed).is_err());
        }
    }

    #[test]
    fn source_phase_contract_rejects_unknown_or_mixed_versions() {
        for version in [0, 4, 99] {
            assert!(SourcePhaseContract::from_plan(&plan(version)).is_err());
        }
        for version in 1..=3 {
            for key in [
                "schemaVersion",
                "sourceToolsPhaseVersion",
                "sourceReviewPhaseVersion",
                "sourceDecisionPhaseVersion",
                "modelRequestLimit",
            ] {
                for invalid in [Value::Null, json!(false), json!("1"), json!(-1), json!(99)] {
                    let mut frozen = plan(version);
                    frozen[key] = invalid;
                    assert!(
                        SourcePhaseContract::from_plan(&frozen).is_err(),
                        "{version}:{key}"
                    );
                }
            }
        }
        let mut first = plan(1);
        first["sourceReviewPhaseVersion"] = json!(1);
        assert!(SourcePhaseContract::from_plan(&first).is_err());
        let mut second = plan(2);
        second["sourceDecisionPhaseVersion"] = json!(1);
        assert!(SourcePhaseContract::from_plan(&second).is_err());
        for key in ["sourceReviewPhaseVersion", "sourceDecisionPhaseVersion"] {
            let mut third = plan(3);
            third.as_object_mut().unwrap().remove(key);
            assert!(SourcePhaseContract::from_plan(&third).is_err());
        }
    }

    #[test]
    fn source_phase_contract_rejects_coverage_on_historical_versions_even_without_extra_budget() {
        for version in 1..=3 {
            for key in [
                "sourceCoveragePhaseVersion",
                "sourceCoverageDecisionPhaseVersion",
            ] {
                for value in [Value::Null, json!(false), json!(0), json!(1), json!("1")] {
                    let mut frozen = plan(version);
                    frozen[key] = value;
                    assert_eq!(
                        SourcePhaseContract::from_plan(&frozen).unwrap_err(),
                        "source_surface_frozen_plan_invalid",
                        "{version}:{key}"
                    );
                }
            }
        }
    }
}
