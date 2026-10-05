// Original per-child SDK proof lives with the financial runtime.
use crate::agent_runtime::multi_agent::budget::web_lifetime::WEB_MODEL_INVOCATION_KIND;
fn native_web_model_idle_original(
    db: &rusqlite::Connection,
    scope: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<Vec<crate::agent_runtime::execution_owner::NativeInvocationOwner>, String> {
    crate::agent_runtime::multi_agent::budget::web_lifetime::require_idle_original(db, scope)
}
