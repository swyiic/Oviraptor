// Capacity pressure is not a provider invoice, currency, or execution authority.
const NATIVE_UNLIMITED_MODEL_COST_RULE: &str = "finite_grant_share_native_unlimited_no_scarcity";
#[derive(Clone, Copy, PartialEq, Eq)]
enum NativeCoordinatorModelCostRule {
    OriginalConservative,
    NativeUnlimitedNoScarcity,
}
impl NativeCoordinatorModelCostRule {
    fn from_original(bootstrap: &JsonValue) -> Result<Self, String> {
        match bootstrap.get("modelCostRule") {
            None if bootstrap["schemaVersion"] == 5 => {
                Err("root_model_cost_original_rule_invalid".into())
            }
            None => Ok(Self::OriginalConservative),
            Some(v)
                if bootstrap["schemaVersion"] == 5
                    && v.as_str() == Some(NATIVE_UNLIMITED_MODEL_COST_RULE) =>
            {
                Ok(Self::NativeUnlimitedNoScarcity)
            }
            _ => Err("root_model_cost_original_rule_invalid".into()),
        }
    }
    fn weight(self, amount: i64, ceiling: Option<i64>) -> Result<i64, String> {
        if amount < 0 || ceiling.is_some_and(|v| v < 0) {
            return Err("root_decision_cost_invalid".into());
        }
        // NULL is the verified original Native unbounded ceiling. It has no
        // finite allocation share; SDK estimates/fees/unknown debt still use
        // their original financial producer and fresh-work admission gates.
        if self == Self::NativeUnlimitedNoScarcity && ceiling.is_none() {
            Ok(0)
        } else {
            Ok(native_coordinator_cost_weight(amount, ceiling))
        }
    }
}
